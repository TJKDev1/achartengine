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

- SMA(20) over 1M rows takes 1.4 ms in wasm without fuel, against 1.3 ms natively, and its output
  is bit-identical.
- A plugin with the `storage` permission can store a value through `host_call`; without it the
  host answers `Denied`.
- A `Noop` round trip costs 180 ns. Copying a 1M-row `f64` column in and back out costs 2.1 ms,
  as much as the SMA itself.
- Fuel stops an infinite loop, and a 64 MiB memory cap makes a 128 MiB allocation fail inside the
  plugin without killing it. Fuel adds 50–100% to a tight numeric loop. Epoch interruption wasn't
  measured.
- wasmtime doesn't build for `wasm32-unknown-unknown`, because its runtime needs `mmap`.
  [ADR 0005](0005-split-host-from-runtime.md) answers this by moving the runtime out of
  `ace-host` behind a `Runtime` trait, with `ace-host-wasmtime` as the native backend.

The plugin is 172 KiB, mostly `rmp-serde` and `format!`, and most of its 166 lines are `unsafe`
pointer handling that an SDK would hide. wasmtime brings 105 crates and an 11 MB host binary.

### Extism (#6)

Not run yet.

## Consequences

The decision (#7) is still open, but the wasmtime spike settles some points whichever runtime
wins:

- **Columns stay resident.** Since copying costs as much as a simple indicator, the host copies
  each column into a plugin once, keeps it there across calls and sends only appended rows
  (DESIGN §6.3).
- **Fuel only where replay needs it.** Fuel is deterministic (D8) but slow, so #7 should measure
  epoch interruption for live use and keep fuel for replays and backtests, as ADR 0005 requires.
- **The runtime must fit ADR 0005's trait.** Extism hides linear memory behind
  `call(name, input)`, so #6 has to show it can address resident columns by pointer and length.
- **`host_call` I/O becomes request and reply** before any browser backend, because browser I/O
  is async (ADR 0005). Changing this after plugins ship would break them, so #7 decides it.
