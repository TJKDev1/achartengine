# 0001: Plugin ABI

- Status: proposed
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

We lean towards a **home-made ABI** (option 2 or 3). We'll choose between plain `wasmtime` and
Extism after a small spike of each that:

- implements a `Recompute` SMA plugin over 1M rows,
- calls one host service under a permission check,
- measures call overhead and column-transfer cost,
- checks fuel/memory limits and that `ace-host` still builds for `wasm32`.

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

The decision (#7) is still open. The spikes point to option 2, because Extism's copies through
its kernel cost more than the SMA they feed and its API can't implement ADR 0005's trait. Its
PDKs for other languages are the main thing option 2 gives up. These points hold whichever
runtime #7 picks.

Columns stay resident. Copying a column costs as much as a simple indicator, so the host copies
each column into a plugin once, keeps it there across calls and sends only appended rows
(DESIGN §6.3).

Speed doesn't decide between fuel and epochs, because on a tight numeric loop they cost the same.
Fuel is deterministic (D8), and ADR 0005 already requires it for replays. But neither one bounds
the time a plugin spends waiting in `host_call`, so #7 still decides whether live use needs a
wall-clock limit as well.

`host_call` I/O becomes request and reply before any browser backend, because browser I/O is
async (ADR 0005). Changing this after plugins ship would break them, so #7 decides it.
