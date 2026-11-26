//! Native WASM runtime for the AChartEngine plugin host.
//!
//! [`WasmtimeRuntime`] implements `ace-host`'s [`Runtime`] trait on wasmtime (ADR 0005). It
//! meters fuel on every call and applies an epoch deadline in live use (ADR 0001). The epoch
//! advances only when the app calls [`Ticker::tick`], so this crate never reads a clock.

use ace_host::{HostCall, Limits, Plugin, PluginError, Runtime};
use wasmtime::{
    AsContext, AsContextMut, Caller, Config, Engine, Extern, Func, Instance, Linker, Memory,
    Module, Store, StoreLimits, StoreLimitsBuilder, Trap, Val, ValType,
};

/// Deadline set when [`Limits::deadline_ticks`] is `None`. wasmtime adds it to the current
/// epoch, so `u64::MAX` would overflow and fire at once.
const NO_DEADLINE: u64 = u64::MAX / 2;

/// Runs plugins on wasmtime, compiling them with Cranelift.
pub struct WasmtimeRuntime {
    engine: Engine,
}

impl WasmtimeRuntime {
    /// Creates a runtime with fuel metering and epoch interruption turned on.
    ///
    /// # Errors
    ///
    /// [`PluginError::InvalidModule`] if wasmtime can't set up a compiler for this machine.
    pub fn new() -> Result<Self, PluginError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|error| invalid_module(&error))?;
        Ok(Self { engine })
    }

    /// A handle that advances this runtime's epoch, for the app's timer.
    #[must_use]
    pub fn ticker(&self) -> Ticker {
        Ticker {
            engine: self.engine.clone(),
        }
    }
}

impl core::fmt::Debug for WasmtimeRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WasmtimeRuntime").finish_non_exhaustive()
    }
}

impl Runtime for WasmtimeRuntime {
    type Module = Module;
    type Plugin<H: HostCall + 'static> = WasmtimePlugin<H>;

    fn load(&self, wasm: &[u8]) -> Result<Module, PluginError> {
        Module::new(&self.engine, wasm).map_err(|error| invalid_module(&error))
    }

    fn instantiate<H: HostCall + 'static>(
        &self,
        module: &Module,
        limits: Limits,
        host: H,
    ) -> Result<WasmtimePlugin<H>, PluginError> {
        let state = State {
            host: Some(host),
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(limits.fuel).map_err(plugin_error)?;
        store.set_epoch_deadline(limits.deadline_ticks.unwrap_or(NO_DEADLINE));

        let mut linker = Linker::new(&self.engine);
        linker
            .func_wrap("ace", "host_call", host_call::<H>)
            .map_err(|error| invalid_module(&error))?;
        let instance = linker
            .instantiate(&mut store, module)
            .map_err(instantiate_error)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| PluginError::InvalidModule("no memory exported as `memory`".into()))?;
        Ok(WasmtimePlugin {
            store,
            instance,
            memory,
            deadline_ticks: limits.deadline_ticks,
        })
    }
}

/// Advances the epoch of the [`WasmtimeRuntime`] it came from. A call stops with
/// [`PluginError::DeadlineExceeded`] after [`Limits::deadline_ticks`] ticks.
///
/// It can be sent to another thread, so the app can tick it from a timer.
#[derive(Clone)]
pub struct Ticker {
    engine: Engine,
}

impl Ticker {
    /// Advances the epoch by one tick.
    pub fn tick(&self) {
        self.engine.increment_epoch();
    }
}

impl core::fmt::Debug for Ticker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ticker").finish_non_exhaustive()
    }
}

/// A plugin instance running on wasmtime.
pub struct WasmtimePlugin<H: 'static> {
    store: Store<State<H>>,
    instance: Instance,
    memory: Memory,
    deadline_ticks: Option<u64>,
}

impl<H: 'static> core::fmt::Debug for WasmtimePlugin<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WasmtimePlugin")
            .field("deadline_ticks", &self.deadline_ticks)
            .finish_non_exhaustive()
    }
}

impl<H: HostCall + 'static> Plugin for WasmtimePlugin<H> {
    fn call(&mut self, name: &str, args: &[u64]) -> Result<Vec<u64>, PluginError> {
        let func = self
            .instance
            .get_func(&mut self.store, name)
            .ok_or_else(|| PluginError::NoSuchExport(name.into()))?;
        // The deadline counts from the start of each call, not from instantiation.
        if let Some(ticks) = self.deadline_ticks {
            self.store.set_epoch_deadline(ticks);
        }
        call_func(&mut self.store, func, name, args)
    }

    fn read(&mut self, ptr: u32, buffer: &mut [u8]) -> Result<(), PluginError> {
        read_memory(&self.store, self.memory, ptr, buffer)
    }

    fn write(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), PluginError> {
        write_memory(&mut self.store, self.memory, ptr, bytes)
    }

    fn set_fuel(&mut self, fuel: u64) -> Result<(), PluginError> {
        self.store.set_fuel(fuel).map_err(plugin_error)
    }

    fn fuel(&self) -> Result<u64, PluginError> {
        self.store.get_fuel().map_err(plugin_error)
    }
}

/// What a plugin's store holds besides the instance.
struct State<H> {
    /// Taken out while `host_call` runs, so the handler can be given the plugin.
    host: Option<H>,
    limits: StoreLimits,
}

/// The `ace::host_call` import: hands the call to the host's [`HostCall`] handler.
fn host_call<H: HostCall + 'static>(
    mut caller: Caller<'_, State<H>>,
    ptr: u32,
    len: u32,
) -> wasmtime::Result<u64> {
    let mut host =
        caller.data_mut().host.take().ok_or_else(|| {
            wasmtime::Error::new(PluginError::Trap("`host_call` re-entered".into()))
        })?;
    let answer = host.host_call(
        &mut CallerPlugin {
            caller: &mut caller,
        },
        ptr,
        len,
    );
    caller.data_mut().host = Some(host);
    answer.map_err(wasmtime::Error::new)
}

/// The plugin as seen from inside `host_call`, where wasmtime only gives access through the
/// [`Caller`].
struct CallerPlugin<'a, 'b, H: 'static> {
    caller: &'a mut Caller<'b, State<H>>,
}

impl<H: 'static> CallerPlugin<'_, '_, H> {
    fn memory(&mut self) -> Result<Memory, PluginError> {
        self.caller
            .get_export("memory")
            .and_then(Extern::into_memory)
            .ok_or_else(|| PluginError::InvalidModule("no memory exported as `memory`".into()))
    }
}

impl<H: 'static> Plugin for CallerPlugin<'_, '_, H> {
    fn call(&mut self, name: &str, args: &[u64]) -> Result<Vec<u64>, PluginError> {
        let func = self
            .caller
            .get_export(name)
            .and_then(Extern::into_func)
            .ok_or_else(|| PluginError::NoSuchExport(name.into()))?;
        call_func(&mut *self.caller, func, name, args)
    }

    fn read(&mut self, ptr: u32, buffer: &mut [u8]) -> Result<(), PluginError> {
        let memory = self.memory()?;
        read_memory(&*self.caller, memory, ptr, buffer)
    }

    fn write(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), PluginError> {
        let memory = self.memory()?;
        write_memory(&mut *self.caller, memory, ptr, bytes)
    }

    fn set_fuel(&mut self, fuel: u64) -> Result<(), PluginError> {
        self.caller.set_fuel(fuel).map_err(plugin_error)
    }

    fn fuel(&self) -> Result<u64, PluginError> {
        self.caller.get_fuel().map_err(plugin_error)
    }
}

/// Calls `func`, converting the integer arguments and results to and from wasm types.
fn call_func(
    mut store: impl AsContextMut,
    func: Func,
    name: &str,
    args: &[u64],
) -> Result<Vec<u64>, PluginError> {
    let no_such_export = || PluginError::NoSuchExport(name.into());
    let ty = func.ty(&store);
    if ty.params().len() != args.len() {
        return Err(no_such_export());
    }
    let params = ty
        .params()
        .zip(args)
        .map(|(param, arg)| to_val(&param, *arg).ok_or_else(no_such_export))
        .collect::<Result<Vec<_>, _>>()?;
    let mut results = vec![Val::I32(0); ty.results().len()];
    func.call(&mut store, &params, &mut results)
        .map_err(plugin_error)?;
    results.iter().map(from_val).collect()
}

/// An integer argument as the wasm type the export takes. `None` if it doesn't fit.
fn to_val(ty: &ValType, arg: u64) -> Option<Val> {
    match ty {
        ValType::I32 => u32::try_from(arg)
            .ok()
            .map(|arg| Val::I32(arg.cast_signed())),
        ValType::I64 => Some(Val::I64(arg.cast_signed())),
        ValType::F32 | ValType::F64 | ValType::V128 | ValType::Ref(_) => None,
    }
}

/// An export's result as an integer. Plugins only return integers (ADR 0001).
fn from_val(val: &Val) -> Result<u64, PluginError> {
    match val {
        Val::I32(value) => Ok(u64::from(value.cast_unsigned())),
        Val::I64(value) => Ok(value.cast_unsigned()),
        Val::F32(_)
        | Val::F64(_)
        | Val::V128(_)
        | Val::FuncRef(_)
        | Val::ExternRef(_)
        | Val::AnyRef(_)
        | Val::ExnRef(_)
        | Val::ContRef(_) => Err(PluginError::Abi(
            "an export returned a non-integer value".into(),
        )),
    }
}

fn read_memory(
    store: impl AsContext,
    memory: Memory,
    ptr: u32,
    buffer: &mut [u8],
) -> Result<(), PluginError> {
    let len = buffer.len();
    memory
        .read(&store, ptr as usize, buffer)
        .map_err(|_out_of_bounds| PluginError::OutOfBounds { ptr, len })
}

fn write_memory(
    store: impl AsContextMut,
    memory: Memory,
    ptr: u32,
    bytes: &[u8],
) -> Result<(), PluginError> {
    memory
        .write(store, ptr as usize, bytes)
        .map_err(|_out_of_bounds| PluginError::OutOfBounds {
            ptr,
            len: bytes.len(),
        })
}

/// Turns a failed call into a [`PluginError`], keeping one raised by the host's `host_call`.
fn plugin_error(error: wasmtime::Error) -> PluginError {
    let error = match error.downcast::<PluginError>() {
        Ok(error) => return error,
        Err(error) => error,
    };
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => PluginError::OutOfFuel,
        Some(Trap::Interrupt) => PluginError::DeadlineExceeded,
        Some(_) | None => PluginError::Trap(format!("{error:#}")),
    }
}

/// Instantiation fails either because the module doesn't link, or because its start
/// function trapped.
fn instantiate_error(error: wasmtime::Error) -> PluginError {
    if error.downcast_ref::<Trap>().is_some() || error.is::<PluginError>() {
        plugin_error(error)
    } else {
        invalid_module(&error)
    }
}

fn invalid_module(error: &wasmtime::Error) -> PluginError {
    PluginError::InvalidModule(format!("{error:#}"))
}

#[cfg(test)]
mod tests;
