//! A [`Runtime`] without WASM, for testing the host. Its plugin is a few Rust functions over a
//! byte vector, standing in for the exports a real plugin has.

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

use crate::{HostCall, Limits, Plugin, PluginError, Region, Runtime};

/// Plugin memory size: one WASM page.
const MEMORY_BYTES: u32 = 64 * 1024;

pub(crate) struct TestRuntime;

impl Runtime for TestRuntime {
    type Module = ();
    type Plugin<H: HostCall + 'static> = TestPlugin<H>;

    fn load(&self, _wasm: &[u8]) -> Result<(), PluginError> {
        Ok(())
    }

    fn instantiate<H: HostCall + 'static>(
        &self,
        _module: &(),
        limits: Limits,
        host: H,
    ) -> Result<TestPlugin<H>, PluginError> {
        Ok(TestPlugin {
            memory: vec![0; MEMORY_BYTES as usize],
            next_free: 8,
            freed: Vec::new(),
            memory_full: false,
            largest_read: 0,
            fuel: limits.fuel,
            host: Some(host),
        })
    }
}

/// Exports `alloc`, `free` and `ask_host`, which passes its `(ptr, len)` to `host_call` and
/// returns the answer. Each call costs one fuel.
pub(crate) struct TestPlugin<H> {
    memory: Vec<u8>,
    /// Bump allocator: `alloc` hands out memory from here, and `free` only records the region.
    next_free: u32,
    pub(crate) freed: Vec<Region>,
    /// Makes `alloc` return 0, as a plugin out of memory does.
    pub(crate) memory_full: bool,
    /// Length of the largest buffer a `read` was asked to fill.
    pub(crate) largest_read: usize,
    fuel: u64,
    /// Taken out while `host_call` runs, so the host can be given `&mut self` as the plugin.
    host: Option<H>,
}

impl TestPlugin<RefuseHostCalls> {
    pub(crate) fn new() -> Self {
        let limits = Limits {
            fuel: 100,
            memory_bytes: MEMORY_BYTES as usize,
            deadline_ticks: None,
        };
        TestRuntime
            .instantiate(&(), limits, RefuseHostCalls)
            .expect("the test runtime always instantiates")
    }
}

impl<H: HostCall> TestPlugin<H> {
    pub(crate) fn host(&self) -> Option<&H> {
        self.host.as_ref()
    }

    fn alloc(&mut self, len: u32) -> u32 {
        match self.next_free.checked_add(len) {
            Some(end) if !self.memory_full && end <= MEMORY_BYTES => {
                let ptr = self.next_free;
                self.next_free = end;
                ptr
            }
            Some(_) | None => 0,
        }
    }

    fn ask_host(&mut self, ptr: u32, len: u32) -> Result<u64, PluginError> {
        let mut host = self
            .host
            .take()
            .ok_or_else(|| PluginError::Trap("`host_call` re-entered".into()))?;
        let answer = host.host_call(self, ptr, len);
        self.host = Some(host);
        answer
    }

    fn range(&self, ptr: u32, len: usize) -> Result<Range<usize>, PluginError> {
        let start = ptr as usize;
        start
            .checked_add(len)
            .filter(|end| *end <= self.memory.len())
            .map(|end| start..end)
            .ok_or(PluginError::OutOfBounds { ptr, len })
    }
}

impl<H: HostCall> Plugin for TestPlugin<H> {
    fn call(&mut self, name: &str, args: &[u64]) -> Result<Vec<u64>, PluginError> {
        self.fuel = self.fuel.checked_sub(1).ok_or(PluginError::OutOfFuel)?;
        let no_such_export = || PluginError::NoSuchExport(name.into());
        let args: Vec<u32> = args
            .iter()
            .map(|arg| u32::try_from(*arg).map_err(|_overflow| no_such_export()))
            .collect::<Result<_, _>>()?;
        match (name, args.as_slice()) {
            ("alloc", [len]) => Ok(vec![u64::from(self.alloc(*len))]),
            ("free", [ptr, len]) => {
                self.freed.push(Region {
                    ptr: *ptr,
                    len: *len,
                });
                Ok(Vec::new())
            }
            ("ask_host", [ptr, len]) => Ok(vec![self.ask_host(*ptr, *len)?]),
            _ => Err(no_such_export()),
        }
    }

    fn read(&mut self, ptr: u32, buffer: &mut [u8]) -> Result<(), PluginError> {
        self.largest_read = self.largest_read.max(buffer.len());
        let range = self.range(ptr, buffer.len())?;
        buffer.copy_from_slice(&self.memory[range]);
        Ok(())
    }

    fn write(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), PluginError> {
        let range = self.range(ptr, bytes.len())?;
        self.memory[range].copy_from_slice(bytes);
        Ok(())
    }

    fn set_fuel(&mut self, fuel: u64) -> Result<(), PluginError> {
        self.fuel = fuel;
        Ok(())
    }

    fn fuel(&self) -> Result<u64, PluginError> {
        Ok(self.fuel)
    }
}

/// A host that refuses every `host_call`.
pub(crate) struct RefuseHostCalls;

impl HostCall for RefuseHostCalls {
    fn host_call(
        &mut self,
        _plugin: &mut dyn Plugin,
        _ptr: u32,
        _len: u32,
    ) -> Result<u64, PluginError> {
        Err(PluginError::Trap("host refused the call".into()))
    }
}
