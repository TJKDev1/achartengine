# 0005: Split the plugin host from its WASM runtime

- Status: accepted
- Date: 2026-11-26
- Issue: #43

## Context

DESIGN D1 says the core and the plugin host must compile to `wasm32`, so a web or embedded
frontend stays possible. The wasmtime spike (#5) found that wasmtime 51, as the spike configured
it, doesn't build for `wasm32-unknown-unknown`: its runtime needs `mmap`, which that target lacks,
and turning off the Cranelift compiler doesn't help. The spike didn't try wasmtime's `no_std`
mode with a custom platform and its Pulley bytecode interpreter.

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
   where wasmtime compiles it. Nobody has measured wasmi on the spike's SMA plugin (ADR 0001) yet.

## Decision

Option 2. The code that decides what a plugin may do and what its messages mean is shared and
compiles to `wasm32`. Only the part that executes WASM instructions differs per platform.

- `ace-host` is `#![no_std]` + `alloc`, under the same rules as `ace-core` (ADR 0002). CI checks it
  on `wasm32v1-none` and `wasm32-unknown-unknown`. The real I/O (sockets, files, the OS keychain)
  lives outside it, in a native crate that the app, the CLI and the Python bindings share.
- `ace-host-wasmtime` is the native backend, and the reference app uses it.
- No other backend is built now. A browser backend would use `WebAssembly`; wasmi stays a
  candidate for embedded targets that have neither.

The seam between them is a `Runtime` trait that works at the level of exports and memory:

- instantiate a module, giving it the host's `host_call` import as a callback;
- call an export with integer arguments and get integer results, including from inside
  `host_call`, where the host calls the plugin's `alloc` to place its answer;
- copy bytes into and out of plugin memory at a pointer and length;
- set limits.

The ABI itself (`alloc`/`free`, packing pointers, MessagePack, column layout) stays in `ace-host`,
so it's written once. Columns stay resident in plugin memory between calls, addressed by pointer
and length. The trait must not expose wasmtime types, require `Send`, or hand out memory slices:
browser JS handles aren't `Send`, and a browser invalidates views into memory when it grows.

Extism hides linear memory behind its own `call(name, input)`, so it might not fit this trait.
The Extism spike (#6) checks that.

## Consequences

- **Async I/O.** `host_call` is synchronous, but browser network and storage calls are async. A
  WASM import can only wait for them through JavaScript Promise Integration (JSPI), Asyncify
  (which rewrites the module), or a worker blocking on `Atomics.wait` (which needs cross-origin
  isolation). Before a browser backend is possible, I/O host calls must become requests whose
  results arrive on a later call. The ABI decision (#7) should settle this, since changing it
  later breaks plugins.
- **Determinism (D8).** `ace-host` rejects modules that use threads or relaxed SIMD, so no backend
  has to support them. It canonicalizes NaN in decoded messages, in columns read back and in
  anything it hashes or journals, so backends can't disagree on NaN bits. Plugin code that
  branches on NaN bits internally is out of scope.

  Replay re-runs compute plugins, so the limits must stop a plugin at the same point every time.
  Time limits don't, so replays and backtests run with fuel only. A plugin that runs out of fuel
  is a recorded event, and the journal records the backend, its version and the limit settings.
  Fuel counts differ between backends, so a replay that hits a limit is exact only on the backend
  that recorded it.
- **Limits.** wasmtime and wasmi have fuel; browsers don't. A browser backend can inject a fuel
  counter into the module at load time, and `ace-host` can require every module to declare a
  memory maximum within the cap, so the memory limit works the same on every backend.
- **Layering.** `scripts/check-layering.sh` gets entries for both crates. `ace-host` may not depend
  on `ace-host-wasmtime` or on any `wasmtime*`, `wasmi*` or `extism*` crate. The script checks the
  full resolved dependency tree of `ace-host`, because wasmi builds on `wasm32v1-none` and the
  `wasm32` builds wouldn't catch it.
