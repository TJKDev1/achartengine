//! Headless core of AChartEngine.
//!
//! The core owns the data model (columns + scales), the dataflow graph and the draw-command
//! output. It never touches a window, a GPU or the wall clock, and it must compile to
//! `wasm32-unknown-unknown`. See `docs/DESIGN.md` for the full architecture.
//!
//! The crate is `no_std` + `alloc` (ADR 0002), so I/O, threads and OS APIs are compile errors
//! here rather than review comments.

#![no_std]

extern crate alloc;

mod column;
mod dataset;

pub use column::{Column, ColumnType, ColumnValues, StrValues, Timestamp};
pub use dataset::{Dataset, DatasetError};

/// Version of this crate, as set in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_set() {
        assert!(
            !VERSION.is_empty(),
            "crate version must come from Cargo.toml"
        );
    }
}
