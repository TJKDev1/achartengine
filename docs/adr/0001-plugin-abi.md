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
2. **Home-made ABI on `wasmtime`.** A few fixed exports (`alloc`, `free`, `call`) plus one host
   import (`host_call`). Control messages are MessagePack; column data is raw little-endian arrays
   in linear memory. Small, fast and fully under our control, but we have to write the SDKs.
3. **Home-made message layer on Extism.** Extism supplies the alloc/call plumbing and PDKs for many
   languages. We define only the messages. Its cost is an extra dependency and abstraction layer.

## Decision

Option 2: a home-made ABI directly on `wasmtime`. We first leaned towards a home-made ABI
(option 2 or 3) and spiked both. Each spike implemented a `Recompute` SMA plugin over 1M rows,
called one host service under a permission check, measured call overhead and column-transfer
cost, and checked fuel and memory limits and whether `ace-host` builds for `wasm32`. The results
are below.

Extism copies every column through its own kernel module, which costs more than the SMA the
column feeds, and its API can't implement the `Runtime` trait from
[ADR 0005](0005-split-host-from-runtime.md). We give up its PDKs for other languages and write
our own SDK, which hides the same pointer handling.

The ABI:

- A plugin exports `alloc`, `free` and `call`, and imports one function, `host_call`.
- Columns are raw little-endian arrays in plugin memory. The host copies a column in once and
  keeps it there across calls, sending only appended rows (DESIGN §6.3).
- Control messages are MessagePack. `ace-host` is `no_std`, so it encodes them with the `rmp`
  crate without its `std` feature and writes the encoding by hand, without `serde`. MessagePack
  has implementations in most languages, so SDKs for other languages stay possible.
- A `host_call` the host can answer from its own state, such as the logical clock or the seeded
  RNG, returns the answer directly. One that needs platform I/O (network, storage, signing with
  a secret) returns a request id at once, and the result arrives on a later `call` into the
  plugin. The plugin never waits inside `host_call`, so a browser backend needs neither JSPI,
  Asyncify nor `Atomics.wait`. The SDK hides the request ids.
- Fuel limits every plugin. In live use an epoch deadline also applies, as a wall-clock limit for
  a plugin given a large fuel budget. Replays and backtests run with fuel only, so they stop a
  plugin at the same point every time (ADR 0005, DESIGN §7).

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

Plugin authors get one SDK, in Rust, until we write others. Their I/O is asynchronous from the
first version, which makes simple plugins a little harder to write but means a browser backend
won't break them. The SDK has to make the request ids invisible to a plugin that just wants to
fetch something.

Epochs cost the same as fuel on a tight numeric loop, so the live deadline is close to free. A
plugin that hits it in live use is stopped and reported. Because it depends on wall-clock time,
it can't be replayed, which is why replays leave it off.

Hand-written MessagePack is more code than `rmp-serde`, but it keeps `ace-host` `no_std` and
leaves the wire format under our control. `ace-host` must fuzz the decoder (#33), since it reads
bytes a plugin wrote.
