<img src="assets/icon.svg" width="64" height="64" alt="">

# AChartEngine

A charting engine for anything that produces data over a dimension: stocks, crypto, server
metrics, sensors, odds. It is written in Rust.

- **Headless core.** It outputs data and draw commands; frontends are adapters.
- **Everything is a plugin.** Data feeds, indicators, trading bots, drawing tools, panels and even
  whole UIs are sandboxed WASM plugins that anyone can write.
- **Deterministic.** Backtests and replays are reproducible bit for bit.

> **Status:** early design. Nothing usable yet. See [docs/DESIGN.md](docs/DESIGN.md) for the
> architecture, and the [milestones](https://github.com/TJKDev1/achartengine/milestones) and
> [issues](https://github.com/TJKDev1/achartengine/issues) for current goals.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Short version: `lefthook install`, keep clippy happy,
and use Conventional Commits.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option.

Created in [T3 Code](https://t3.codes).
