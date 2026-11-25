# 0002: `ace-core` is `no_std` + `alloc`

- Status: accepted
- Date: 2026-11-25
- Issue: #26

## Context

DESIGN §4.1 says the core has no I/O, no threads and no OS APIs, so it keeps D1's `wasm32`
guarantee. Until now that was a convention backed by review and a few `clippy.toml` bans. A lot
of the code will be AI-authored, so rules that only live in prose are easy to break unnoticed.

## Options

1. **Keep `std`.** No extra friction, but nothing stops `std::fs`, `std::thread` or
   `std::time` from creeping in; `wasm32-unknown-unknown` still compiles them as stubs.
2. **`#![no_std]` + `alloc`.** `std` is simply not in scope, so the rule becomes a compile error.
   Costs: imports come from `core`/`alloc`, and dependencies need `default-features = false`
   (or must be `no_std`-capable at all).

## Decision

Option 2. The crate is still nearly empty, so switching now costs almost nothing, and it turns
DESIGN §4.1 into something the compiler checks.

CI (and the pre-push hook) also runs `cargo check -p ace-core --target wasm32v1-none`. That
target ships no `std` at all, so it also catches a dependency that quietly enables `std`.

## Consequences

- Use `core::`/`alloc::` paths (`alloc::vec::Vec`, `alloc::collections::BTreeMap`, ...).
- Every new `ace-core` dependency must work without `std`; check it on `wasm32v1-none`.
- `HashMap` isn't available anyway; the determinism rule (DESIGN §7) already prefers `BTreeMap`.
- Math uses `libm`, which is `no_std` and is what DESIGN §7 requires already.
- Unit tests still run with `std` through the test harness.
