//! Spike host for #5: loads `plugin-sma` in wasmtime, checks the acceptance criteria of
//! ADR 0001 and prints measurements.
//!
//! Build the plugin first:
//! `cargo build --release -p plugin-sma --target wasm32-unknown-unknown`, then
//! `cargo run --release -p spike-host`.

use std::collections::BTreeMap;
use std::time::Instant;

use wasmtime::error::{Context, Result, bail};
use spike_abi::{
    ColumnRef, HostRequest, HostResponse, Request, Response, decode, encode, pack, unpack,
};
use wasmtime::{
    Caller, Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, Trap, TypedFunc,
};

const PLUGIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../target/wasm32-unknown-unknown/release/plugin_sma.wasm"
);
const ROWS: usize = 1_000_000;
const PERIOD: u32 = 20;
const MEMORY_LIMIT: usize = 64 << 20;
const FUEL: u64 = 1_000_000_000;

/// Per-plugin state the host keeps in the wasmtime store.
struct State {
    limits: StoreLimits,
    permissions: Vec<&'static str>,
    storage: BTreeMap<String, Vec<u8>>,
}

struct Plugin {
    store: Store<State>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    free: TypedFunc<(u32, u32), ()>,
    call: TypedFunc<(u32, u32), u64>,
}

fn main() -> Result<()> {
    let wasm = std::fs::read(PLUGIN).with_context(|| format!("reading {PLUGIN}"))?;
    let engine = Engine::new(Config::new().consume_fuel(true))?;

    let started = Instant::now();
    let module = Module::new(&engine, &wasm)?;
    report("plugin size", format!("{} KiB", wasm.len() / 1024));
    report("compile", started.elapsed());

    let linker = linker(&engine)?;
    let started = Instant::now();
    let mut plugin = Plugin::new(&engine, &linker, &module, vec!["storage"])?;
    report("instantiate", started.elapsed());

    measure_call_overhead(&mut plugin)?;
    let input = random_walk(ROWS);
    measure_sma(&mut plugin, &input)?;
    check_permission_denied(&engine, &linker, &module, &input)?;
    check_fuel_limit(&engine, &linker, &module)?;
    check_memory_limit(&mut plugin)?;
    measure_without_fuel(&wasm, &input)?;
    Ok(())
}

fn linker(engine: &Engine) -> Result<Linker<State>> {
    let mut linker = Linker::new(engine);
    linker.func_wrap("ace", "host_call", host_call)?;
    Ok(linker)
}

/// The one host import: decodes a [`HostRequest`], checks the permission it needs, and writes
/// the [`HostResponse`] into a buffer it allocates through the plugin's `alloc`.
fn host_call(mut caller: Caller<'_, State>, ptr: u32, len: u32) -> Result<u64> {
    let memory = export_memory(&mut caller)?;
    let bytes = read(memory.data(&caller), ptr, len)?.to_vec();
    let request: HostRequest = decode(&bytes)?;
    let response = serve(caller.data_mut(), request);

    let reply = encode(&response);
    let reply_len = u32::try_from(reply.len())?;
    let alloc = caller
        .get_export("alloc")
        .and_then(wasmtime::Extern::into_func)
        .context("plugin has no `alloc`")?
        .typed::<u32, u32>(&caller)?;
    let reply_ptr = alloc.call(&mut caller, reply_len)?;
    write(memory.data_mut(&mut caller), reply_ptr, &reply)?;
    Ok(pack(reply_ptr, reply_len))
}

fn serve(state: &mut State, request: HostRequest) -> HostResponse {
    match request {
        HostRequest::StorageSet { key, value } => {
            if !state.permissions.contains(&"storage") {
                return HostResponse::Denied {
                    permission: "storage".into(),
                };
            }
            state.storage.insert(key, value);
            HostResponse::Done
        }
    }
}

impl Plugin {
    fn new(
        engine: &Engine,
        linker: &Linker<State>,
        module: &Module,
        permissions: Vec<&'static str>,
    ) -> Result<Self> {
        let state = State {
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).build(),
            permissions,
            storage: BTreeMap::new(),
        };
        let mut store = Store::new(engine, state);
        store.limiter(|state| &mut state.limits);
        // Fails only on an engine without fuel metering, where there's nothing to set.
        store.set_fuel(FUEL).ok();
        let instance: Instance = linker.instantiate(&mut store, module)?;
        Ok(Self {
            memory: instance
                .get_memory(&mut store, "memory")
                .context("plugin has no memory")?,
            alloc: instance.get_typed_func(&mut store, "alloc")?,
            free: instance.get_typed_func(&mut store, "free")?,
            call: instance.get_typed_func(&mut store, "call")?,
            store,
        })
    }

    fn request(&mut self, request: &Request) -> Result<Response> {
        let bytes = encode(request);
        let len = u32::try_from(bytes.len())?;
        let ptr = self.alloc.call(&mut self.store, len)?;
        write(self.memory.data_mut(&mut self.store), ptr, &bytes)?;
        let packed = self.call.call(&mut self.store, (ptr, len));
        self.free.call(&mut self.store, (ptr, len))?;

        let (reply_ptr, reply_len) = unpack(packed?);
        let response = decode(read(self.memory.data(&self.store), reply_ptr, reply_len)?)?;
        self.free.call(&mut self.store, (reply_ptr, reply_len))?;
        Ok(response)
    }

    /// Copies `values` into plugin memory as little-endian `f64`s.
    fn write_column(&mut self, values: &[f64]) -> Result<ColumnRef> {
        let len = u32::try_from(values.len())?;
        let ptr = self.alloc.call(&mut self.store, len * 8)?;
        if ptr == 0 {
            bail!("plugin is out of memory");
        }
        let start = ptr as usize;
        let target = self
            .memory
            .data_mut(&mut self.store)
            .get_mut(start..start + values.len() * 8)
            .context("column out of bounds")?;
        for (bytes, value) in target.chunks_exact_mut(8).zip(values) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        Ok(ColumnRef { ptr, len })
    }

    /// Copies a column out of plugin memory and frees it.
    fn take_column(&mut self, column: ColumnRef) -> Result<Vec<f64>> {
        let bytes = read(self.memory.data(&self.store), column.ptr, column.len * 8)?;
        let values = bytes
            .chunks_exact(8)
            .map(|chunk| f64::from_le_bytes(chunk.try_into().unwrap_or_default()))
            .collect();
        self.free_column(column)?;
        Ok(values)
    }

    fn free_column(&mut self, column: ColumnRef) -> Result<()> {
        Ok(self
            .free
            .call(&mut self.store, (column.ptr, column.len * 8))?)
    }

    fn fuel_used(&self) -> Result<u64> {
        Ok(FUEL - self.store.get_fuel()?)
    }
}

fn measure_call_overhead(plugin: &mut Plugin) -> Result<()> {
    const CALLS: u32 = 100_000;
    let started = Instant::now();
    for _ in 0..CALLS {
        expect(plugin.request(&Request::Noop)?, &Response::Done)?;
    }
    report(
        "noop call (encode + 4 wasm calls)",
        started.elapsed() / CALLS,
    );
    Ok(())
}

fn measure_sma(plugin: &mut Plugin, input: &[f64]) -> Result<()> {
    let started = Instant::now();
    let column = plugin.write_column(input)?;
    report("write 1M f64 (8 MB) into plugin", started.elapsed());

    plugin.store.set_fuel(FUEL)?;
    let started = Instant::now();
    let response = plugin.request(&Request::Sma {
        period: PERIOD,
        input: column,
    })?;
    report("SMA(20) over 1M rows in wasm", started.elapsed());
    report("  fuel used", format!("{}", plugin.fuel_used()?));
    let Response::Column(output) = response else {
        bail!("SMA failed: {response:?}");
    };

    let started = Instant::now();
    let in_wasm = plugin.take_column(output)?;
    report("read 1M f64 back", started.elapsed());
    plugin.free_column(column)?;

    let started = Instant::now();
    let native = native_sma(input, PERIOD as usize);
    report("SMA(20) over 1M rows natively", started.elapsed());

    let same = in_wasm.len() == native.len()
        && in_wasm
            .iter()
            .zip(&native)
            .all(|(a, b)| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()));
    report("wasm output bit-identical to native", same);
    report(
        "host storage after SMA",
        format!("{:?}", plugin.store.data().storage),
    );
    if !same {
        bail!("wasm and native SMA differ");
    }
    Ok(())
}

fn check_permission_denied(
    engine: &Engine,
    linker: &Linker<State>,
    module: &Module,
    input: &[f64],
) -> Result<()> {
    let mut plugin = Plugin::new(engine, linker, module, vec![])?;
    let column = plugin.write_column(input)?;
    let response = plugin.request(&Request::Sma {
        period: PERIOD,
        input: column,
    })?;
    expect(response, &Response::Error("host denied `storage`".into()))?;
    report("SMA without `storage` permission", "denied, as expected");
    Ok(())
}

fn check_fuel_limit(engine: &Engine, linker: &Linker<State>, module: &Module) -> Result<()> {
    let mut plugin = Plugin::new(engine, linker, module, vec![])?;
    plugin.store.set_fuel(10_000_000)?;
    let started = Instant::now();
    let error = match plugin.request(&Request::Spin) {
        Ok(response) => bail!("spin returned {response:?}"),
        Err(error) => error,
    };
    let trap = error.downcast_ref::<Trap>();
    report(
        "infinite loop with 10M fuel",
        format!("{trap:?} after {:?}", started.elapsed()),
    );
    if trap != Some(&Trap::OutOfFuel) {
        bail!("expected OutOfFuel, got {error:?}");
    }
    Ok(())
}

fn check_memory_limit(plugin: &mut Plugin) -> Result<()> {
    plugin.store.set_fuel(FUEL)?;
    let response = plugin.request(&Request::Hog { bytes: 128 << 20 })?;
    report(
        "allocate 128 MiB under a 64 MiB cap",
        format!("{response:?}"),
    );
    if !matches!(response, Response::Error(_)) {
        bail!("allocation over the cap succeeded");
    }
    expect(plugin.request(&Request::Noop)?, &Response::Done)?;
    report("plugin still answers after the refusal", true);
    Ok(())
}

/// Runs the SMA on an engine without fuel metering, to see what metering costs.
fn measure_without_fuel(wasm: &[u8], input: &[f64]) -> Result<()> {
    let engine = Engine::new(&Config::new())?;
    let module = Module::new(&engine, wasm)?;
    let mut plugin = Plugin::new(&engine, &linker(&engine)?, &module, vec!["storage"])?;
    let column = plugin.write_column(input)?;
    let started = Instant::now();
    let response = plugin.request(&Request::Sma {
        period: PERIOD,
        input: column,
    })?;
    report("SMA(20) over 1M rows in wasm, no fuel", started.elapsed());
    let Response::Column(output) = response else {
        bail!("SMA failed: {response:?}");
    };
    plugin.free_column(output)?;
    plugin.free_column(column)
}

fn native_sma(values: &[f64], period: usize) -> Vec<f64> {
    let mut sum = 0.0;
    values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            sum += value;
            if i >= period {
                sum -= values[i - period];
            }
            if i + 1 >= period {
                sum / period as f64
            } else {
                f64::NAN
            }
        })
        .collect()
}

/// A deterministic random walk starting at 100, so runs are comparable.
fn random_walk(rows: usize) -> Vec<f64> {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut price = 100.0;
    (0..rows)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            price += (seed % 2001) as f64 / 1000.0 - 1.0;
            price
        })
        .collect()
}

fn export_memory(caller: &mut Caller<'_, State>) -> Result<Memory> {
    caller
        .get_export("memory")
        .and_then(wasmtime::Extern::into_memory)
        .context("plugin has no memory")
}

fn read(memory: &[u8], ptr: u32, len: u32) -> Result<&[u8]> {
    let start = ptr as usize;
    memory
        .get(start..start + len as usize)
        .context("buffer out of bounds")
}

fn write(memory: &mut [u8], ptr: u32, bytes: &[u8]) -> Result<()> {
    let start = ptr as usize;
    memory
        .get_mut(start..start + bytes.len())
        .context("buffer out of bounds")?
        .copy_from_slice(bytes);
    Ok(())
}

fn expect(response: Response, wanted: &Response) -> Result<()> {
    if &response != wanted {
        bail!("expected {wanted:?}, got {response:?}");
    }
    Ok(())
}

fn report(what: &str, value: impl std::fmt::Debug) {
    let value = format!("{value:?}");
    println!("{what:<42} {}", value.trim_matches('"'));
}
