//! Spike host for #6: loads `plugin-sma` in Extism, checks the acceptance criteria of ADR 0001
//! and prints measurements.
//!
//! Build the plugin first:
//! `cargo build --release -p plugin-sma --target wasm32-unknown-unknown`, then
//! `cargo run --release -p spike-host`.

use std::collections::BTreeMap;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use extism::{CurrentPlugin, Error, Manifest, PTR, Plugin, PluginBuilder, UserData, Val, Wasm};
use spike_abi::{ColumnId, HostRequest, HostResponse, Request, Response, decode, encode};

/// `anyhow::bail!`, which Extism doesn't re-export.
macro_rules! bail {
    ($($message:tt)*) => {
        return Err(Error::msg(format!($($message)*)))
    };
}

const PLUGIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../target/wasm32-unknown-unknown/release/plugin_sma.wasm"
);
const ROWS: usize = 1_000_000;
const PERIOD: u32 = 20;
/// 64 MiB in 64 KiB wasm pages. Extism applies it to the plugin's and its own memory together.
const MEMORY_PAGES: u32 = 1024;
const FUEL: u64 = 1_000_000_000;

/// Per-plugin state the host function sees through Extism's `UserData`.
#[derive(Default)]
struct State {
    permissions: Vec<&'static str>,
    storage: BTreeMap<String, Vec<u8>>,
}

/// How a plugin is limited, besides the memory cap every plugin gets.
#[derive(Clone, Copy)]
enum Limit {
    Fuel(u64),
    Timeout(Duration),
    None,
}

/// One loaded plugin, and the state its host function sees.
struct SmaPlugin {
    plugin: Plugin,
    state: UserData<State>,
}

fn main() -> Result<(), Error> {
    let wasm = std::fs::read(PLUGIN).map_err(|error| Error::msg(format!("{PLUGIN}: {error}")))?;
    report("plugin size", format!("{} KiB", wasm.len() / 1024));

    let started = Instant::now();
    let mut plugin = SmaPlugin::new(&wasm, vec!["storage"], Limit::Fuel(FUEL))?;
    report("compile and link", time(started.elapsed()));
    let started = Instant::now();
    plugin.send(&Request::Noop)?;
    report("first call (instantiates)", time(started.elapsed()));

    measure_call_overhead(&mut plugin)?;
    let input = random_walk(ROWS);
    measure_sma(&mut plugin, &input)?;
    check_permission_denied(&wasm, &input)?;
    check_fuel_limit(&wasm)?;
    check_timeout(&wasm)?;
    check_memory_limit(&wasm)?;
    measure_repeated_sma(&wasm, &input, Limit::Fuel(FUEL))?;
    measure_repeated_sma(&wasm, &input, Limit::None)?;
    Ok(())
}

/// The one host import: decodes a [`HostRequest`], checks the permission it needs, and
/// returns the [`HostResponse`] through Extism memory.
fn host_call(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    state: UserData<State>,
) -> Result<(), Error> {
    let (Some(input), Some(output)) = (inputs.first(), outputs.first_mut()) else {
        bail!("host_call takes one pointer and returns one");
    };
    let bytes: Vec<u8> = plugin.memory_get_val(input)?;
    let request: HostRequest = decode(&bytes)?;
    let state = state.get()?;
    let response = serve(
        &mut state.lock().unwrap_or_else(PoisonError::into_inner),
        request,
    );
    plugin.memory_set_val(output, encode(&response))
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

impl SmaPlugin {
    fn new(wasm: &[u8], permissions: Vec<&'static str>, limit: Limit) -> Result<Self, Error> {
        let state = UserData::new(State {
            permissions,
            storage: BTreeMap::new(),
        });
        let mut manifest = Manifest::new([Wasm::data(wasm.to_vec())]).with_memory_max(MEMORY_PAGES);
        if let Limit::Timeout(timeout) = limit {
            manifest = manifest.with_timeout(timeout);
        }
        let mut builder = PluginBuilder::new(manifest)
            .with_wasi(false)
            // Extism caches compiled modules on disk by default, which would hide compile time.
            .with_cache_disabled()
            .with_function_in_namespace("ace", "host_call", [PTR], [PTR], state.clone(), host_call);
        if let Limit::Fuel(fuel) = limit {
            builder = builder.with_fuel_limit(fuel);
        }
        Ok(Self {
            plugin: builder.build()?,
            state,
        })
    }

    fn send(&mut self, request: &Request) -> Result<Response, Error> {
        let reply: &[u8] = self.plugin.call("call", encode(request))?;
        Ok(decode(reply)?)
    }

    /// Copies `values` into the plugin, which keeps them as a column.
    fn put_column(&mut self, values: &[f64]) -> Result<ColumnId, Error> {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let id: &[u8] = self.plugin.call("put_column", bytes)?;
        Ok(ColumnId(u32::from_le_bytes(id.try_into()?)))
    }

    /// Copies a column out of the plugin, which drops it.
    fn take_column(&mut self, column: ColumnId) -> Result<Vec<f64>, Error> {
        let bytes: &[u8] = self
            .plugin
            .call("take_column", column.0.to_le_bytes().as_slice())?;
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| {
                let mut value = [0; 8];
                value.copy_from_slice(chunk);
                f64::from_le_bytes(value)
            })
            .collect())
    }

    fn drop_column(&mut self, column: ColumnId) -> Result<(), Error> {
        assert_response(self.send(&Request::Drop { column })?, &Response::Done)
    }

    fn fuel_used(&self) -> u64 {
        self.plugin.fuel_consumed().unwrap_or_default()
    }

    fn storage(&self) -> Result<String, Error> {
        let state = self.state.get()?;
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(format!("{:?}", state.storage))
    }
}

fn measure_call_overhead(plugin: &mut SmaPlugin) -> Result<(), Error> {
    const CALLS: u32 = 100_000;
    let started = Instant::now();
    for _ in 0..CALLS {
        assert_response(plugin.send(&Request::Noop)?, &Response::Done)?;
    }
    report(
        "noop call (encode + Extism call)",
        time(started.elapsed() / CALLS),
    );
    Ok(())
}

fn measure_sma(plugin: &mut SmaPlugin, input: &[f64]) -> Result<(), Error> {
    let started = Instant::now();
    let column = plugin.put_column(input)?;
    report("write 1M f64 (8 MB) into plugin", time(started.elapsed()));

    let started = Instant::now();
    let response = plugin.send(&Request::Sma {
        period: PERIOD,
        input: column,
    })?;
    report("SMA(20) over 1M rows in wasm", time(started.elapsed()));
    report("fuel used by the SMA call", plugin.fuel_used());
    let Response::Column(output) = response else {
        bail!("SMA failed: {response:?}");
    };

    let started = Instant::now();
    let in_wasm = plugin.take_column(output)?;
    report("read 1M f64 back", time(started.elapsed()));
    plugin.drop_column(column)?;

    let started = Instant::now();
    let native = native_sma(input, PERIOD as usize);
    report("SMA(20) over 1M rows natively", time(started.elapsed()));

    let same = bit_identical(&in_wasm, &native);
    report("wasm output bit-identical to native", same);
    report("host storage after SMA", plugin.storage()?);
    if !same {
        bail!("wasm and native SMA differ");
    }
    Ok(())
}

fn check_permission_denied(wasm: &[u8], input: &[f64]) -> Result<(), Error> {
    let mut plugin = SmaPlugin::new(wasm, vec![], Limit::Fuel(FUEL))?;
    let column = plugin.put_column(input)?;
    let response = plugin.send(&Request::Sma {
        period: PERIOD,
        input: column,
    })?;
    // Must match the message built in plugin-sma's `store_row_count`.
    assert_response(response, &Response::Error("host denied `storage`".into()))?;
    report("SMA without `storage` permission", "denied, as expected");
    Ok(())
}

fn check_fuel_limit(wasm: &[u8]) -> Result<(), Error> {
    const SPIN_FUEL: u64 = 10_000_000;
    let mut plugin = SmaPlugin::new(wasm, vec![], Limit::Fuel(SPIN_FUEL))?;
    plugin.send(&Request::Noop)?;
    let started = Instant::now();
    let error = expect_trap(&mut plugin, &Request::Spin, "plugin ran out of fuel")?;
    report(
        &format!("infinite loop with {}M fuel", SPIN_FUEL / 1_000_000),
        format!("`{error}` after {}", time(started.elapsed())),
    );
    Ok(())
}

fn check_timeout(wasm: &[u8]) -> Result<(), Error> {
    const TIMEOUT: Duration = Duration::from_millis(50);
    let mut plugin = SmaPlugin::new(wasm, vec![], Limit::Timeout(TIMEOUT))?;
    let column = plugin.put_column(&[1.0, 2.0, 3.0])?;
    let started = Instant::now();
    let error = expect_trap(&mut plugin, &Request::Spin, "timeout")?;
    let elapsed = started.elapsed();
    report(
        &format!("infinite loop with a {TIMEOUT:?} timeout"),
        format!("`{error}` after {}", time(elapsed)),
    );
    if elapsed < TIMEOUT || elapsed > TIMEOUT * 4 {
        bail!("timeout fired after {elapsed:?}, expected about {TIMEOUT:?}");
    }
    check_survives_trap(&mut plugin, column)
}

/// Extism's memory cap traps the call instead of failing the allocation, unlike wasmtime's
/// `StoreLimits`, so the plugin can't handle the refusal itself.
fn check_memory_limit(wasm: &[u8]) -> Result<(), Error> {
    const HOG_BYTES: u32 = 128 << 20;
    let mut plugin = SmaPlugin::new(wasm, vec![], Limit::Fuel(FUEL))?;
    let column = plugin.put_column(&[1.0, 2.0, 3.0])?;
    let error = expect_trap(&mut plugin, &Request::Hog { bytes: HOG_BYTES }, "oom")?;
    report(
        &format!(
            "allocate {} MiB under a {} MiB cap",
            HOG_BYTES >> 20,
            MEMORY_PAGES / 16
        ),
        format!("`{error}`"),
    );
    check_survives_trap(&mut plugin, column)
}

/// Sends a request that must fail the call with exactly `message`, and returns the error.
fn expect_trap(plugin: &mut SmaPlugin, request: &Request, message: &str) -> Result<Error, Error> {
    match plugin.send(request) {
        Ok(response) => bail!("{request:?} returned {response:?}"),
        Err(error) if error.to_string() == message => Ok(error),
        Err(error) => bail!("{request:?} failed with `{error}`, expected `{message}`"),
    }
}

/// Checks that a plugin still answers after a trap and that its columns are still there.
fn check_survives_trap(plugin: &mut SmaPlugin, column: ColumnId) -> Result<(), Error> {
    assert_response(plugin.send(&Request::Noop)?, &Response::Done)?;
    let kept = plugin.take_column(column)? == [1.0, 2.0, 3.0];
    report("  plugin answers and keeps its columns", kept);
    if !kept {
        bail!("the plugin lost its column after the trap");
    }
    Ok(())
}

/// Runs the SMA several times on one resident column and reports the fastest call, so memory
/// growth from the first call doesn't count.
fn measure_repeated_sma(wasm: &[u8], input: &[f64], limit: Limit) -> Result<(), Error> {
    const RUNS: usize = 5;
    let metering = match limit {
        Limit::Fuel(_) => "fuel",
        // Extism turns on epoch interruption for every plugin, so a run without fuel still
        // pays for epochs.
        Limit::None => "epochs only",
        Limit::Timeout(_) => bail!("time the SMA with fuel or with no limit"),
    };
    let mut plugin = SmaPlugin::new(wasm, vec!["storage"], limit)?;
    let column = plugin.put_column(input)?;
    let mut fastest = Duration::MAX;
    for _ in 0..RUNS {
        let started = Instant::now();
        let response = plugin.send(&Request::Sma {
            period: PERIOD,
            input: column,
        })?;
        fastest = fastest.min(started.elapsed());
        let Response::Column(output) = response else {
            bail!("SMA failed: {response:?}");
        };
        plugin.drop_column(output)?;
    }
    report(
        &format!("SMA(20), best of {RUNS} later calls, {metering}"),
        time(fastest),
    );
    plugin.drop_column(column)
}

/// True if both columns hold the same bits, treating any two NaNs as equal.
fn bit_identical(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()))
}

/// A copy of the plugin's `rolling_mean`, kept separate on purpose: the check is that the same
/// code gives the same bits natively and in wasm.
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

fn assert_response(response: Response, wanted: &Response) -> Result<(), Error> {
    if &response != wanted {
        bail!("expected {wanted:?}, got {response:?}");
    }
    Ok(())
}

fn time(duration: Duration) -> String {
    format!("{duration:.2?}")
}

fn report(what: &str, value: impl std::fmt::Display) {
    println!("{what:<42} {value}");
}
