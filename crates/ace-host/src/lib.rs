//! Plugin host of AChartEngine.
//!
//! The host implements the plugin ABI (ADR 0001): the [`Runtime`] trait that backends implement,
//! and [`copy_in`] and [`copy_out`], which move buffers through plugin memory. Permissions will
//! live here too (DESIGN §8). The host doesn't execute WASM itself: a [`Runtime`] backend does,
//! so this crate has no wasmtime dependency and builds for `wasm32` (ADR 0005).
//! `ace-host-wasmtime` is the native backend.
//!
//! The crate is `no_std` + `alloc`, under the same rules as `ace-core` (ADR 0002).

#![no_std]

extern crate alloc;

mod memory;
mod runtime;
#[cfg(test)]
mod test_runtime;

pub use memory::{Region, copy_in, copy_out};
pub use runtime::{HostCall, Limits, Plugin, PluginError, Runtime};
