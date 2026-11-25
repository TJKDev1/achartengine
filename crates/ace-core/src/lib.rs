//! Headless core of AChartEngine.
//!
//! The core owns the data model (columns + scales), the dataflow graph and the draw-command
//! output. It never touches a window, a GPU or the wall clock, and it must compile to
//! `wasm32-unknown-unknown`. See `docs/DESIGN.md` for the full architecture.

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
