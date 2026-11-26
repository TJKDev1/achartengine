use alloc::string::String;
use core::fmt;

/// A WASM engine that can load and run plugins.
///
/// The trait works at the level of exports and memory, so the ABI stays in `ace-host` and each
/// backend only executes instructions (ADR 0005). It doesn't require `Send`, because a browser
/// backend holds JavaScript handles, and it never hands out a slice of plugin memory, because a
/// browser invalidates views into memory when it grows.
pub trait Runtime {
    /// A compiled module, ready to instantiate any number of times.
    type Module;
    /// A running instance, which owns the host's `host_call` handler `H`.
    type Plugin<H: HostCall + 'static>: Plugin;

    /// Compiles a module.
    ///
    /// # Errors
    ///
    /// [`PluginError::InvalidModule`] if `wasm` isn't a valid module.
    fn load(&self, wasm: &[u8]) -> Result<Self::Module, PluginError>;

    /// Instantiates a module under `limits`, with `host` answering its `ace::host_call` import.
    ///
    /// # Errors
    ///
    /// [`PluginError::InvalidModule`] if the module imports anything but `ace::host_call` or
    /// doesn't export its memory as `memory`, or a trap from the module's start function.
    fn instantiate<H: HostCall + 'static>(
        &self,
        module: &Self::Module,
        limits: Limits,
        host: H,
    ) -> Result<Self::Plugin<H>, PluginError>;
}

/// A running plugin, seen from the host.
///
/// Exports take and return integers; anything larger goes through memory (ADR 0001).
/// [`HostCall::host_call`] receives the plugin as `&mut dyn Plugin`, so the host can call the
/// plugin's `alloc` to place its answer.
pub trait Plugin {
    /// Calls the export `name` with integer arguments and returns its results.
    ///
    /// # Errors
    ///
    /// [`PluginError::NoSuchExport`] if there is no such function or it takes the wrong number
    /// of arguments, or the trap or limit that stopped it.
    fn call(&mut self, name: &str, args: &[u64]) -> Result<alloc::vec::Vec<u64>, PluginError>;

    /// Copies plugin memory at `ptr` into `buffer`.
    ///
    /// # Errors
    ///
    /// [`PluginError::OutOfBounds`] if the range isn't inside plugin memory.
    fn read(&mut self, ptr: u32, buffer: &mut [u8]) -> Result<(), PluginError>;

    /// Copies `bytes` into plugin memory at `ptr`.
    ///
    /// # Errors
    ///
    /// [`PluginError::OutOfBounds`] if the range isn't inside plugin memory.
    fn write(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), PluginError>;

    /// Sets the fuel left for the next calls.
    ///
    /// # Errors
    ///
    /// [`PluginError::Trap`] if the backend can't meter fuel.
    fn set_fuel(&mut self, fuel: u64) -> Result<(), PluginError>;

    /// Fuel left, so the host can journal what a call used (DESIGN §7).
    ///
    /// # Errors
    ///
    /// [`PluginError::Trap`] if the backend can't meter fuel.
    fn fuel(&self) -> Result<u64, PluginError>;
}

/// The host's side of the `ace::host_call(ptr: u32, len: u32) -> u64` import.
pub trait HostCall {
    /// Answers a plugin's call. `ptr` and `len` locate the request in plugin memory, and the
    /// result is handed back to the plugin unchanged.
    ///
    /// # Errors
    ///
    /// Any error traps the plugin's current call.
    fn host_call(
        &mut self,
        plugin: &mut dyn Plugin,
        ptr: u32,
        len: u32,
    ) -> Result<u64, PluginError>;
}

/// Limits a backend applies to every call into a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Starting fuel, spent by the start function and every call until [`Plugin::set_fuel`]
    /// refills it. Fuel and `memory_bytes` are deterministic and `deadline_ticks` is not, so
    /// replays rely on the first two (ADR 0001).
    pub fuel: u64,
    /// Most bytes of linear memory the plugin may grow to. Growing past it fails inside the
    /// plugin, which can handle the refusal.
    pub memory_bytes: usize,
    /// Wall-clock deadline for each call in live use, in backend ticks. The host's clock, not
    /// the backend, decides how long a tick is. `None` for replays and backtests.
    pub deadline_ticks: Option<u64>,
}

/// Why a plugin couldn't be loaded or a call into it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginError {
    /// The module isn't valid WebAssembly or doesn't match the ABI.
    InvalidModule(String),
    /// The plugin has no function export by this name with the arguments given.
    NoSuchExport(String),
    /// A range the host tried to read or write isn't inside plugin memory.
    OutOfBounds {
        /// Start of the range.
        ptr: u32,
        /// Length of the range in bytes.
        len: usize,
    },
    /// The plugin broke the ABI, for example by returning a buffer the host can't use.
    Abi(String),
    /// The call used up its fuel.
    OutOfFuel,
    /// The call passed its wall-clock deadline.
    DeadlineExceeded,
    /// The plugin trapped, or the host's `host_call` failed.
    Trap(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModule(reason) => write!(f, "invalid plugin module: {reason}"),
            Self::NoSuchExport(name) => {
                write!(f, "plugin has no export `{name}` with these arguments")
            }
            Self::OutOfBounds { ptr, len } => {
                write!(f, "{len} bytes at {ptr} are outside plugin memory")
            }
            Self::Abi(reason) => write!(f, "plugin broke the ABI: {reason}"),
            Self::OutOfFuel => f.write_str("plugin ran out of fuel"),
            Self::DeadlineExceeded => f.write_str("plugin passed its deadline"),
            Self::Trap(reason) => write!(f, "plugin trapped: {reason}"),
        }
    }
}

impl core::error::Error for PluginError {}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{HostCall, Limits, Plugin, PluginError, Runtime};
    use crate::test_runtime::{TestPlugin, TestRuntime};
    use crate::{Region, copy_in, copy_out};

    /// Answers each request with the request reversed, placed through the plugin's `alloc`.
    struct Reverse {
        requests: Vec<Vec<u8>>,
    }

    impl HostCall for Reverse {
        fn host_call(
            &mut self,
            plugin: &mut dyn Plugin,
            ptr: u32,
            len: u32,
        ) -> Result<u64, PluginError> {
            let request = copy_out(plugin, Region { ptr, len })?;
            let reply: Vec<u8> = request.iter().rev().copied().collect();
            self.requests.push(request);
            Ok(copy_in(plugin, &reply)?.pack())
        }
    }

    fn reverse_plugin(fuel: u64) -> TestPlugin<Reverse> {
        let limits = Limits {
            fuel,
            memory_bytes: 64 * 1024,
            deadline_ticks: None,
        };
        let host = Reverse {
            requests: Vec::new(),
        };
        TestRuntime.instantiate(&(), limits, host).unwrap()
    }

    #[test]
    fn host_call_can_allocate_its_answer_in_the_plugin() {
        let mut plugin = reverse_plugin(100);
        let request = copy_in(&mut plugin, b"abc").unwrap();
        let results = plugin
            .call("ask_host", &[request.ptr.into(), request.len.into()])
            .unwrap();
        let [answer] = results[..] else {
            panic!("`ask_host` returns one value, got {results:?}");
        };
        let reply = copy_out(&mut plugin, Region::unpack(answer)).unwrap();
        assert_eq!(reply, b"cba", "the host's answer reaches the plugin");
        let requests = &plugin.host().unwrap().requests;
        assert_eq!(requests, &[b"abc"], "the host saw the request once");
    }

    #[test]
    fn a_failed_host_call_traps_the_call() {
        let mut plugin = TestPlugin::new();
        assert_eq!(
            plugin.call("ask_host", &[0, 0]),
            Err(PluginError::Trap("host refused the call".into())),
            "the host's error stops the plugin's call"
        );
    }

    #[test]
    fn calls_stop_when_fuel_runs_out() {
        let mut plugin = reverse_plugin(1);
        assert!(
            plugin.call("alloc", &[8]).is_ok(),
            "the first call has fuel"
        );
        assert_eq!(
            plugin.call("alloc", &[8]),
            Err(PluginError::OutOfFuel),
            "the second has none"
        );
        plugin.set_fuel(5).unwrap();
        assert!(
            plugin.call("alloc", &[8]).is_ok(),
            "refuelling lets calls run again"
        );
        assert_eq!(plugin.fuel(), Ok(4), "each call costs one fuel");
    }

    #[test]
    fn unknown_exports_are_reported_by_name() {
        let mut plugin = TestPlugin::new();
        assert_eq!(
            plugin.call("missing", &[]),
            Err(PluginError::NoSuchExport("missing".into())),
            "the error names the export"
        );
    }

    #[test]
    fn errors_read_as_sentences() {
        use alloc::string::ToString;
        assert_eq!(
            PluginError::OutOfBounds { ptr: 8, len: 4 }.to_string(),
            "4 bytes at 8 are outside plugin memory",
            "out of bounds"
        );
        assert_eq!(
            PluginError::NoSuchExport("render".into()).to_string(),
            "plugin has no export `render` with these arguments",
            "missing export"
        );
    }
}
