# 0005: Split the plugin host from its WASM runtime

- Status: accepted
- Date: 2026-11-26
- Issue: #43

## Context

DESIGN D1 says the core and the plugin host must compile to `wasm32`, so a web or embedded
frontend stays possible. The wasmtime spike (#5) found that wasmtime 51, as the spike configured
it, doesn't build for `wasm32-unknown-unknown`: its runtime needs `mmap`, which that target lacks,
and turning off the Cranelift compiler doesn't help. The spike didn't try wasmtime's `no_std`
mode with a custom platform and its Pulley interpreter.

A host running inside a browser already has a WASM engine: the browser's `WebAssembly` API, which
compiles plugins to native code.

## Options

1. **Narrow D1 to `ace-core`.** A web frontend writes its own host. This is the least work now, but
   permission checks and message handling get written twice. The copies can drift, and then a
   plugin gives different results on native and web, which breaks D8.
2. **Split the host.** `ace-host` keeps everything that doesn't execute WASM: the ABI messages,
   plugin lifecycle, permission checks, module validation and the I/O broker interface. A runtime
   trait hides the engine, with one backend per platform.
3. **Use one interpreter everywhere**, such as wasmi, which builds for `wasm32`. One backend
   behaves the same on every platform, but every plugin runs interpreted, including on desktop
   where wasmtime compiles it. Nobody has measured wasmi on the spike's SMA yet.

## Decision

Option 2. The code that decides what a plugin may do and what its messages mean is shared and
compiles to `wasm32`. Only the part that executes WASM instructions differs per platform.

- `ace-host` is `#![no_std]` + `alloc`, under the same rules as `ace-core` (ADR 0002). CI checks it
  on `wasm32v1-none`. The real I/O (sockets, files, the OS keychain) lives in the app or frontend,
  which implements the broker interface.
- `ace-host-wasmtime` is the native backend, and the reference app uses it.
- No other backend is built now. A browser backend would use `WebAssembly`; wasmi stays a
  candidate for embedded targets that have neither.

The trait works at message level: instantiate a module with the host's `host_call` import, call
an export with a byte buffer and get bytes back, read and write plugin memory while handling a
`host_call`, and set limits. It must not expose wasmtime types, require `Send`, or hand out
memory slices that outlive a call; browser JS handles aren't `Send`, and a browser invalidates
views into memory when it grows.

Extism hides linear memory behind its own `call(name, input)`, so it might not fit this trait.
The Extism spike (#6) checks that.

## Consequences

- **Async I/O.** `host_call` is synchronous, but browser network and storage calls are async, and
  a WASM import can't wait for them without JSPI or a worker blocking on `Atomics.wait`. Before a
  browser backend is possible, I/O host calls must become requests whose results arrive on a
  later call. The ABI decision (#7) should settle this, since changing it later breaks plugins.
- **Determinism across backends (D8).** `ace-host` rejects modules that use threads or
  relaxed SIMD, and canonicalizes NaN in the values it reads back, so no backend has to provide
  either. Fuel counts and limit errors still differ between backends, so a journal only replays
  exactly on the backend that recorded it, and records which one that was.
- **Limits.** Fuel is a wasmtime feature with no browser equivalent. A browser backend needs
  another way to stop runaway plugins, chosen when it's built.
- **Layering.** `scripts/check-layering.sh` gets entries for both crates. `ace-host` may not
  depend on `ace-host-wasmtime`, wasmtime, wasmi or Extism; the `wasm32v1-none` build catches
  those that arrive through other crates.
