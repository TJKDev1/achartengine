# 0001: Plugin ABI

- Status: accepted
- Date: 2026-11-25
- Issue: #7 (spikes: #5 wasmtime, #6 Extism)

## Context

Plugins are sandboxed WASM modules (DESIGN D2). They need a stable, versioned way to:

- exchange control messages with the host,
- pass column data cheaply (DESIGN §5),
- call host services through the permission layer (DESIGN §8).

Plugins should be writable in several languages over time, but v0 may ship a Rust-only SDK.

## Options

1. **WIT / Component Model.** Standard and typed, with a good toolchain, but heavier. Language
   support is still uneven, and passing large columns means copying through canonical ABI lists.
2. **Home-made ABI on `wasmtime`.** A few fixed exports plus one host import, with columns as raw
   arrays in linear memory. Small, fast and fully under our control, but we have to write the SDKs.
3. **Home-made message layer on Extism.** Extism supplies the alloc/call plumbing and PDKs for many
   languages. We define only the messages. Its cost is an extra dependency and abstraction layer.

## Decision

Option 2: a home-made ABI directly on `wasmtime`. We spiked options 2 and 3 (#5, #6), and both met
the criteria except the `wasm32` build. Extism copies every column four times and its call-only
API can't implement ADR 0005's `Runtime` trait (see [Extism](#extism-6) below). We give up its
PDKs for other languages and write our own SDK.

- A plugin exports `alloc`, `free` and `call`, and imports one function, `host_call`.
- Columns are raw little-endian arrays in plugin memory. The host copies a column in once and
  keeps it there across calls, sending only appended rows (DESIGN §6.3).
- Control messages are MessagePack, encoded by hand with the `rmp` crate, without `std` or
  `serde`, so `ace-host` stays `no_std`.
- Fuel limits every plugin. In live use a wall-clock deadline also applies, which wasmtime
  implements with epochs. It caps a plugin whose fuel budget would take too long to burn.
  Replays and backtests run with fuel only ([ADR 0005](0005-split-host-from-runtime.md),
  [DESIGN §7](../DESIGN.md#7-determinism-d8)).

A `host_call` the host can answer from its own state, such as the logical clock or the seeded RNG,
returns the answer directly. One that needs platform I/O (network, storage, signing with a secret)
returns a request id at once. The plugin's `call` then returns, and the host delivers the result
in a message carrying that id on a later `call`. Because the plugin never waits inside
`host_call`, a browser backend needs neither JSPI, Asyncify nor `Atomics.wait` (see ADR 0005,
Async I/O). The SDK turns the request ids into something a plugin author doesn't have to track.

## Spike results

### wasmtime (#5)

The code and the full table are in
[`spikes/wasmtime-abi`](../../spikes/wasmtime-abi/README.md). It meets every criterion except the
`wasm32` build:

- SMA(20) over 1M rows runs in wasm with output bit-identical to the native version.
- A plugin with the `storage` permission can store a value through `host_call`; without it the
  host answers `Denied`.
- A `Noop` round trip costs 350 ns (median of three runs). Copying a 1M-row `f64` column in and
  back out costs 2.2 ms, more than the SMA itself.
- Fuel stops an infinite loop, and a 64 MiB memory cap makes a 128 MiB allocation fail inside the
  plugin without killing it. Fuel and epoch interruption cost the same: either one slows the SMA
  from 0.8 ms to 1.8 ms.
- wasmtime doesn't build for `wasm32-unknown-unknown`, because its runtime needs `mmap`.
  [ADR 0005](0005-split-host-from-runtime.md) answers this by moving the runtime out of
  `ace-host` behind a `Runtime` trait, with `ace-host-wasmtime` as the native backend.

The plugin is 172 KiB, mostly `rmp-serde` and `format!`. It has 15 `unsafe` blocks in 166 lines,
pointer handling that an SDK would hide. wasmtime brings 86 crates and an 11 MB host binary.

### Extism (#6)

The code and a side-by-side table are in
[`spikes/extism-abi`](../../spikes/extism-abi/README.md). It meets the same criteria and also
fails `wasm32`, because Extism runs on wasmtime. The differences:

- It doesn't fit ADR 0005's trait. Extism exports take no arguments, and neither the host nor a
  host function can reach plugin memory: every input and output is copied through the memory of
  Extism's own kernel module. Extism could only sit behind a `call(name, bytes) -> bytes` trait,
  with the ABI in Extism instead of `ace-host`.
- Columns can stay in the plugin between calls, but each one crosses four copies. Moving a 1M-row
  column in and out takes 14 ms, 6 times as long as on wasmtime and 7 times the SMA. A `Noop`
  round trip takes 2.7 µs.
- Its memory cap traps the call instead of failing the allocation, so a plugin can't handle the
  refusal.
- Its PDK makes the plugin ordinary Rust, with 1 `unsafe` block in 154 lines, and Extism has
  PDKs for other languages. Our own SDK would hide the same pointer handling.
- It pins wasmtime 49, two majors behind, and brings 235 crates and a 17 MB host binary.

## Consequences

Plugin authors get one SDK, in Rust, until we write others. MessagePack has implementations in
most languages, so other SDKs stay possible. Their I/O is asynchronous from the first version,
which makes simple plugins harder to write but means a browser backend won't break them.

Fuel and epochs each slow a tight numeric loop by the same amount, from 0.8 ms to 1.8 ms for the
SMA. Neither spike measured both together, which is what live use runs, so the first host
benchmarks should. A plugin that hits the live deadline is stopped and reported, and the journal
records the stop as an event, like a fuel stop (ADR 0005). Replay then repeats the recorded stop
instead of running the plugin longer, so the session still replays exactly (D8).

Hand-written MessagePack is more code than `rmp-serde`, but it keeps `ace-host` `no_std` and the
wire format under our control. `ace-host` must fuzz the decoder (#33), since it reads bytes a
plugin wrote.
