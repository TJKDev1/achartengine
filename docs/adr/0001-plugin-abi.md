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

## Consequences

To be filled in once the spike is done.
