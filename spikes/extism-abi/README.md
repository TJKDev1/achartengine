# Spike: home-made message layer on Extism (#6)

Throwaway code for [ADR 0001](../../docs/adr/0001-plugin-abi.md), the second of its two spikes.
It does what the [wasmtime spike](../wasmtime-abi/README.md) does, with the same SMA plugin and
host checks, built on Extism 1.41 and its Rust PDK. It has its own Cargo workspace, so the main
workspace's lints and checks skip it. The exports are described at the top of
[`abi/src/lib.rs`](abi/src/lib.rs).

```sh
cargo build --release -p plugin-sma --target wasm32-unknown-unknown
cargo run --release -p spike-host
```

## Results

Both spikes ran alternately, three times each, on the same machine and build settings as the
wasmtime spike. The table gives the median run.

| Measurement | wasmtime | Extism |
|-------------|----------|--------|
| Plugin `.wasm` size | 172 KiB | 251 KiB |
| Compile the module | 121 ms | 40 ms |
| Instantiate | 48 µs | 79 µs (on the first call) |
| `Noop` round trip | 350 ns | 2.7 µs |
| Copy 1M `f64` (8 MB) into the plugin | 1.0 ms | 8.8 ms |
| Copy 1M `f64` back out | 1.2 ms | 5.3 ms |
| SMA(20) over 1M rows natively | 1.4 ms | 1.4 ms |
| SMA(20) in wasm, first call, with fuel | 2.4 ms (48M fuel) | 3.5 ms (67M fuel) |
| SMA(20) in wasm, later calls, with fuel | 1.8 ms | 1.9 ms |
| SMA(20) in wasm, later calls, with epochs | 1.8 ms | 1.9 ms |
| SMA(20) in wasm, later calls, no fuel or epochs | 0.8 ms | not possible |
| Host crates (normal and build) | 86 | 235 |
| Host binary | 11 MB | 17 MB |

The first SMA call grows plugin memory for its 8 MB output; later calls reuse it. Extism turns on
epoch interruption for every plugin, so its "with epochs" row is its run without fuel. Extism's
compile is faster because it enables wasmtime's parallel compilation, which the wasmtime spike
left off. Its fuel count includes the work of its own kernel module.

Every acceptance criterion from ADR 0001 passes except `wasm32`:

- **SMA over 1M rows** runs, and its output is bit-identical to the native version.
- **Permission-checked host call:** `ace::host_call` is an Extism host function. With `storage`
  the row count lands in host storage; without it the host answers `Denied`.
- **Fuel:** an infinite loop with 10M fuel stops with `plugin ran out of fuel` after 0.4 ms.
- **Timeout:** Extism also offers a wall-clock timeout. With 50 ms the loop stops after 50–60 ms.
- **Memory:** with a 64 MiB cap, a 128 MiB allocation traps the call with `oom`. Unlike wasmtime's
  `StoreLimits`, Extism's limiter fails the call instead of failing `memory.grow`, so the plugin
  can't handle the refusal. After the fuel, timeout and memory traps the plugin keeps answering
  and keeps its columns.
- **`ace-host` on `wasm32`:** fails, as with wasmtime, since Extism runs on wasmtime.

## Findings

**Extism doesn't fit the runtime trait of ADR 0005.** That trait calls exports with integer
arguments and copies bytes to and from a pointer in plugin memory. Extism exports take no
arguments: the host passes one byte buffer, which Extism copies into the memory of its own kernel
module, and the plugin copies it from there into its own memory. Neither the host nor a host
function can reach the plugin's memory through Extism's API. So Extism could only sit behind a
trait of the form `call(name, bytes) -> bytes`, and the ABI would be Extism's instead of living
once in `ace-host`.

**Columns cost 4–9 times as much to move.** The plugin can still keep columns between calls,
because Extism keeps the instance alive, so this spike names columns by id instead of pointer.
But every column crosses four copies: host values to bytes, bytes into kernel memory, kernel
memory into the plugin (the PDK reads it 8 bytes at a time, one kernel call each), and bytes to
`f64`s.
Moving a 1M-row column in and out takes 14 ms, against 2.2 ms on wasmtime, and 7 times the SMA
itself. A `Noop` round trip is 8 times as slow, from the same input and output copying.

**Fuel and epochs cost the same.** This corrects the wasmtime spike, which guessed epochs would
be cheaper. On both spikes, a tight loop slows from 0.8 ms to 1.8 ms with either one. So the
choice between them doesn't have to be made on speed, and fuel, which is deterministic, can be
the only limit.

**The PDK is pleasant.** `#[plugin_fn]` and `#[host_fn]` turn the plugin into ordinary Rust: 154
lines with one `unsafe` (calling the host function), against 166 lines with 15 on wasmtime.
An SDK for our own ABI would hide the same pointer handling, so this advantage holds only until
we write one. Extism publishes PDKs for other languages too; this spike tried only Rust.

**Extism brings its own runtime around wasmtime.** It pins wasmtime 49, two major versions
behind the spike's 51, so `ace-host-wasmtime` would follow Extism's upgrades. It starts a timer
thread for timeouts, needs `anyhow`, and still has 235 crates with its HTTP client turned off.
Extism has a JavaScript SDK that runs in browsers; this spike didn't try it as a browser backend.
