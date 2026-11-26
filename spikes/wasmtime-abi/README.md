# Spike: home-made plugin ABI on wasmtime (#5)

Throwaway code for [ADR 0001](../../docs/adr/0001-plugin-abi.md). It has its own Cargo workspace,
so the main workspace's lints and checks skip it. The ABI is described at the top of
[`abi/src/lib.rs`](abi/src/lib.rs): plugin exports `alloc`, `free` and `call`, one host import
`host_call`, MessagePack control messages, and columns passed as raw little-endian `f64` arrays.

```sh
cargo build --release -p plugin-sma --target wasm32-unknown-unknown
cargo run --release -p spike-host
```

## Results

Windows 11, x86-64, Rust 1.97.1, wasmtime 51 with cranelift, release builds. The run is repeatable
to within about 5%.

| Measurement | Result |
|-------------|--------|
| Plugin `.wasm` size | 166 KiB, mostly `rmp-serde` and `format!` |
| Compile the module | 112 ms (once per plugin; can be cached) |
| Instantiate | 57 µs |
| `Noop` round trip (encode, `alloc`, `call`, two `free`s, decode) | 180 ns |
| Copy 1M `f64` (8 MB) into plugin memory | 1.0 ms |
| Copy 1M `f64` back out | 1.1 ms |
| SMA(20) over 1M rows, natively | 1.3 ms |
| SMA(20) over 1M rows in wasm, no fuel | 1.4 ms |
| SMA(20) over 1M rows in wasm, with fuel | 2.4 ms (48M fuel) |

All acceptance criteria from ADR 0001 pass:

- **SMA over 1M rows** runs, and its output is bit-identical to the native version.
- **Permission-checked host call:** the plugin stores its row count through `host_call`. With
  `storage` in its permissions the value lands in host storage; without it the host answers
  `Denied` and the plugin returns an error.
- **Fuel:** an infinite loop with 10M fuel traps with `Trap::OutOfFuel` after 0.15 ms.
- **Memory:** with a 64 MiB cap from `StoreLimits`, a 128 MiB allocation fails inside the plugin
  (`try_reserve` returns an error), and the plugin keeps answering afterwards.
- **`ace-host` on `wasm32`:** fails. wasmtime doesn't build for `wasm32-unknown-unknown`, with or
  without cranelift, because its runtime needs `mmap`.

## Findings

Copying is the main cost of crossing the boundary. Moving a 1M-row column in and out takes 2.1 ms,
about as long as the SMA itself. Most indicators read the same input columns, so the host should
copy each column into a plugin once and keep it there across calls, and send only appended rows
afterwards (DESIGN §6.3). Per-call overhead (180 ns) doesn't matter at this size.

Fuel metering costs about 70% on a tight numeric loop. Fuel is deterministic, which D8 wants for
replay, but epoch interruption is far cheaper and is enough to stop a hung plugin. One option is
epochs for live use and fuel only where the instruction count has to be reproducible; decide in #7.

On the web the host can't be wasmtime. A browser frontend would run plugins on the browser's own
WebAssembly engine, so `ace-host` needs its engine behind a small interface (instantiate, call an
export, read and write memory, set limits) with a wasmtime backend and later a browser one. Fuel
and `StoreLimits` don't exist in browsers, so web limits would need fuel instrumentation applied
to the module before loading and a memory maximum in the module itself.

The ABI was quick to write: 94 lines of shared messages, 144 for the plugin and about 380 for the
host, without help from a framework. Most of the plugin is `unsafe` pointer handling that an SDK
would hide. wasmtime 51 brings 105 crates and an 11 MB host binary; all pass `cargo deny`. It has
its own error type, so the host doesn't need `anyhow`.
