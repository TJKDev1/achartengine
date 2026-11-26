# AChartEngine — Design

Status: **draft**. Brainstormed 2026-11-25. Implementation has started; see `crates/`.

## 1. Vision

AChartEngine is a charting engine for anything that produces data over a dimension: stocks, crypto, server metrics, sensor data, sports odds, and more. It is written in Rust.

The **engine** and the **tool** built on it are strictly separated, and everything above the core can be extended. Indicators, data feeds, trading bots, drawing tools, UI panels and even whole UIs are plugins that anyone can write. Our own reference app is built mostly from the same plugin API that third parties use.

## 2. Principles

1. **The core is headless.** It knows nothing about windows, GPUs or OSes. It outputs data and draw commands.
2. **First-party code has no special access.** If the reference app needs a hook, the hook goes into the public plugin API.
3. **Nothing assumes markets.** Market concepts (sessions, tick sizes, orders) live in plugins and optional crates, not the core data model.
4. **Deterministic by construction.** The same inputs always produce the same outputs, bit for bit.
5. **Plugins can't harm users.** Plugins are sandboxed and get only the permissions they declare and the user grants.
6. **Clever only where it pays.** Choose the simple solution unless it clearly fails a requirement. Document every non-obvious mechanism and why the simple version wasn't enough.
7. **Don't break published plugins.** The plugin API is versioned, and contracts are designed to grow without breaking (see §6).

## 3. Decisions

| # | Topic | Decision |
|---|-------|----------|
| D1 | Platform | Headless core with frontends as adapters. The reference app is **native desktop only**. Core and plugin host must **still compile to `wasm32`** (checked in CI) so a web or embedded frontend stays possible. The WASM runtime and the real I/O are per platform ([ADR 0005](adr/0005-split-host-from-runtime.md)). |
| D2 | Extensions | **Sandboxed WASM plugins.** |
| D3 | Product | The engine plus a **reference app** that dogfoods the plugin API. |
| D4 | Data model | Generic **columns + scales**. Time series is only the most common case. The x-axis is not always time. |
| D5 | Plugin UI | **A canvas of draw primitives is the base layer.** A declarative **widget library** is an optional layer that draws onto that canvas. Frontends only implement primitives. |
| D6 | Composition | A **typed dataflow graph** for data, plus a **service/event bus** for commands, UI events and plugin-provided services. |
| D7 | Updates | A **streaming-first** contract (`append`, `update_last`, `revise`, `reset`). An SDK helper lets simple plugins do a full recompute instead. |
| D8 | Determinism | Backtests and replay are **bit-for-bit reproducible**. |
| D9 | Security | Plugins **declare permissions**, the user approves them, and the host enforces them. |
| D10 | License | Open source, **MIT OR Apache-2.0** (dual). Plugins may use any license. |
| D11 | Bindings | **Python bindings (PyO3) early.** They also test that the core API is clean. |
| D12 | Scope | Every capability in §9 stays possible. Nothing is cut, but only the v0 test versions (§10) get built at first. |

## 4. Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│ Reference app (native)        Python (PyO3)        CLI / headless    │
├──────────────────────────────────────────────────────────────────────┤
│ Frontend adapter: wgpu + winit — rasterizes draw lists, sends input  │
├──────────────────────────────────────────────────────────────────────┤
│ Plugin host: ABI, permissions, I/O broker, secrets, storage;         │
│ runs WASM through a runtime backend (wasmtime on native)             │
├──────────────────────────────────────────────────────────────────────┤
│ Engine core: data model · dataflow graph · scheduler · logical clock │
│ · layout · scales · draw-list generation · hit-testing · state/undo  │
└──────────────────────────────────────────────────────────────────────┘
        ▲ plugins (WASM): sources, transforms, indicators, strategies,
          brokers, sinks, series renderers, drawing tools, panels, shells
```

### 4.1 Engine core (`ace-core`)
- Column store for series data, the dataflow graph, the scheduler and the logical clock.
- Scales and axes, viewport and layout, and draw-list generation.
- No I/O, no threads required, no OS APIs. This is what keeps D1's `wasm32` guarantee.
- `#![no_std]` + `alloc` ([ADR 0002](adr/0002-no-std-core.md)), so the rule above is a compile error. CI checks it on `wasm32v1-none`, which has no `std`.

### 4.2 Draw commands (`ace-draw`)
A small, stable set of retained primitives:
- path / polyline / rect / circle
- **text**: the host does shaping and font handling, so plugins never ship fonts
- image
- clip / transform / layer groups
- batched instanced marks for drawing 1M+ candles or points efficiently

Charts, plugin canvases and the widget library all emit these. Every frontend implements exactly this set.

### 4.3 Widget library (`ace-ui`)
- A Rust crate (and later bindings for other plugin languages) that plugins link in.
- Declarative widget tree → layout → draw commands. Includes theming tokens and keyboard focus.
- Optional. A plugin can skip it and draw directly.
- Accessibility: the library emits a semantic tree beside the draw list so frontends can expose it to screen readers.

### 4.4 Plugin host (`ace-host`)
- Implements the plugin ABI (§12.1), validates modules and enforces permissions.
- Brokers I/O (network, storage, secrets) for plugins. The app or frontend implements the actual I/O.
- Runs plugins through a runtime trait, so `ace-host` has no wasmtime dependency and builds for `wasm32` ([ADR 0005](adr/0005-split-host-from-runtime.md)).
- A runtime backend implements the trait and applies fuel/time and memory limits. `ace-host-wasmtime` is the native one; a browser backend would use the browser's `WebAssembly` API.

### 4.5 Frontend (`ace-frontend-wgpu`)
- Renders draw lists and the widget semantic tree, forwards input, and manages windows.

## 5. Data model

- A **dataset** is a set of named, typed **columns** of equal length: `f64`, `i64`, timestamp, string-dictionary, bool. One column may be the **key** (often time). It must be an `f64`, `i64` or timestamp column in ascending order. Equal neighbours are allowed because ticks can share a timestamp, but NaN is not. A dataset checks this when it is built, so the rest of the engine can rely on it.
  - A timestamp is an `i64` of nanoseconds since the Unix epoch, UTC, which covers 1677–2262. Time zones are a display concern.
  - String columns store each distinct string once, plus a `u32` code per row.
  - Columns have no nulls yet.
- A **scale** maps a column's domain to screen space. Types: linear, log, time, **session-time** (hides non-trading gaps; supplied by a calendar plugin), band/category, percent, indexed-to-100.
- **Bars** are just a dataset with `t, open, high, low, close, volume` columns. Ticks, order book snapshots and events are other datasets. Charts like point & figure, depth and volume profile use non-time key columns.
- Downsampling and level of detail for huge datasets is a **core** responsibility, not a plugin one.

## 6. Plugin model

### 6.1 Roles
A plugin exports one or more **roles**. Each role is a versioned message interface (see §12.1 for the ABI):

| Role | Purpose | Example |
|------|---------|---------|
| `source` | Produces datasets from the outside world | Exchange WebSocket, CSV file |
| `transform` | Datasets in → datasets out | Resample, Heikin-Ashi, spread |
| `compute` | Indicators and analytics | SMA, RSI, pattern detector |
| `strategy` | Signals/orders from data | Bot logic |
| `broker` | Orders → fills/positions | Backtest, paper, live exchange |
| `sink` | Side effects out | Alert to Discord, CSV export |
| `series` | Renders a dataset as marks | Candles, footprint, volume profile |
| `tool` | Interactive drawing with handles and hit-testing | Trendline, Fibonacci |
| `panel` | A UI surface (canvas, optionally `ace-ui`) | Watchlist, DOM ladder |
| `shell` | Arranges panels and charts into a whole UI | "Scalper layout" |
| `service` | An API other plugins call over the bus | Symbol search, calendar |

### 6.2 Graph
Nodes have typed **ports** described by a dataset schema (column names and types, key type). The engine:
- checks connections,
- orders execution topologically,
- caches outputs,
- sends only the changes downstream.

Users or shells wire graphs, and graphs save into workspaces.

### 6.3 Update protocol (D7)
Messages a node receives on each input port:

```
reset(snapshot)          // full replace
append(rows)             // new rows at the end
update_last(row)         // last row changed (forming bar)
revise(range, rows)      // late correction anywhere
```

The SDK's `Recompute` adapter turns all four into "here is the full input, give me the full output", so a beginner's indicator is one function.

### 6.4 Bus
- Request/response and pub/sub between plugins and the app.
- Covers commands, keybindings, context-menu entries, selection, crosshair sync and services.
- Uses the same versioned message format as roles.

### 6.5 API evolution
- Role interfaces are semver'd.
- New optional functions and roles are minor versions.
- The host supports at least one previous major version at a time.

## 7. Determinism (D8)

- The engine owns a **logical clock**. Plugins never read wall-clock time; they get the time from the engine.
- Randomness only comes from a **seeded host RNG**.
- Node execution order is deterministic (stable topological order), and there is no plugin-visible concurrency.
- Non-determinism only enters at **`source` and `broker` boundaries**. Their outputs are **journaled** (recorded), so any live session can be replayed exactly. Replays and backtests limit plugins by fuel only, and the journal records the runtime backend ([ADR 0005](adr/0005-split-host-from-runtime.md)). Bar replay and backtests use the same mechanism.
- Floating point: avoid fast-math. WASM float semantics are deterministic apart from NaN bit patterns, which we canonicalize.

## 8. Permissions (D9)

Plugins declare these in their manifest. The user approves them at install time, and the host enforces them.

- `net: [hosts]`: outbound network to the listed hosts only.
- `storage`: a private key-value store and file area.
- `secrets: [names]`: the host **signs or authenticates requests on the plugin's behalf**. Plugins never see raw keys.
- `orders: paper | live`: can submit orders to a broker node. `live` asks for an extra confirmation.
- `read-graph`: can observe other nodes' outputs beyond its wired inputs.
- `notify`: OS notifications and sounds.
- `clipboard`, `files: pick` (a file chosen by the user through a dialog).

Every plugin also gets fuel/time limits per call and memory caps, so a broken plugin can't freeze the app. The runtime backend enforces them. Fuel counts, and so the point where a plugin is stopped, differ between backends ([ADR 0005](adr/0005-split-host-from-runtime.md)).

## 9. Capability catalogue

Everything the architecture must make possible. → names the role or core feature it relies on.

**Data in**
- Exchange/broker feeds, REST/WebSocket, CSV/Parquet, databases → `source`
- Non-market data: metrics, sensors, odds, weather, on-chain, game economies → `source`, generic model
- Ticks, order books, trades → non-bar datasets
- News, earnings, economic calendar → event datasets
- Symbol search and instrument metadata (tick/lot size, multipliers, options chains, futures rollover) → `service`
- Market calendars: sessions, holidays, half-days, timezones → `service` + session-time scale
- Local cache, history backfill, gap fill, live/history merge → core + `source`
- Data licensing flags (no redistribution or export) → dataset metadata

**Transforms**
- Resampling, tick→bar, Heikin-Ashi, Renko, range, Kagi
- Split/dividend adjustment, FX conversion, normalization/indexing
- Synthetic instruments: ratios, spreads, baskets; comparing and overlaying symbols

**Compute**
- Single- and multi-output indicators, indicators built on other indicators, multi-timeframe, multi-symbol
- Pattern detection that outputs annotations
- Screeners and scanners over many symbols (headless batch)
- Alerts (condition → sink)
- Strategies → orders → fills → positions; backtest, paper and live brokers
- Portfolio and risk analytics, correlation
- ML inference inside plugins (e.g. ONNX in WASM)

**Rendering**
- Candles, OHLC, line, area, histogram, baseline, heatmap
- Charts without a time axis: volume/market profile, depth, footprint, point & figure, volatility surface
- Custom marks from plugins; log, percent and custom scales; multiple y-axes; themes
- Millions of points with smooth zoom (level of detail)

**Interaction**
- Drawing tools with handles, snapping and saved state
- Crosshair, tooltips, legends, cross-chart sync
- Commands, keybindings, context menus, command palette
- Bar replay; undo/redo; snapshots
- Touch gestures (future frontends)

**UI**
- Panels: watchlist, order book, DOM ladder, positions, trade journal
- Toolbars, auto-generated settings forms from a plugin's config schema
- Layouts, workspaces, whole UIs (`shell`); multiple windows, chart grids

**Output and platform**
- Export to PNG, SVG or CSV; headless server rendering (chat bots posting charts)
- Sinks: Discord, Telegram, email, webhooks, sounds
- Python bindings and notebooks; a CLI for CI backtests and scanners
- Collaboration (CRDT-shared drawings and layouts)
- AI assistant plugins that read chart state and add drawings
- Plugin dev tools: SDK, test harness, graph inspector/profiler, per-plugin logs, hot reload
- Distribution: plugin signing, versioning and dependencies, a registry later
- Localization, number and date formatting, colorblind-safe themes, screen-reader data tables

## 10. v0 test capabilities (proposal)

Each one is the **smallest version** that proves its extension point works end to end. These are not finished features.

| # | Capability | Proves |
|---|------------|--------|
| T1 | CSV bar `source` + a seeded random-walk `source` | Sources, datasets, the journal |
| T2 | `resample` transform (1m → 5m) | Transforms, the update protocol |
| T3 | SMA (with `Recompute`) and incremental EMA | Both plugin styles; compute chaining (EMA of SMA) |
| T4 | Candle + line `series` | Draw lists, scales, the wgpu frontend |
| T5 | Non-time chart: volume profile | The generic data model (D4) |
| T6 | Trendline `tool` | Hit-testing, handles, saved state, undo |
| T7 | Watchlist `panel` built with `ace-ui` + a raw-canvas panel | Both UI layers (D5) |
| T8 | MA-cross `strategy` + backtest `broker` | Strategy→broker flow, determinism (same run twice gives the same hash) |
| T9 | Alert → log/webhook `sink` with `net` permission | Permissions, the bus |
| T10 | Minimal `shell` that lays out a chart + watchlist | Whole UIs as plugins |
| T11 | Python: load CSV, run SMA, export PNG headlessly | Bindings, headless rendering |
| T12 | Bar replay of T1–T8 | Logical clock, journaling |

## 11. Proposed workspace layout

```
crates/
  ace-core/            data model, graph, scheduler, clock, scales, layout
  ace-draw/            draw-command types (shared by everyone)
  ace-ui/              optional widget library → draw commands
  ace-host/            plugin host: ABI, permissions, I/O broker (no_std, builds for wasm32)
  ace-host-wasmtime/   native WASM runtime backend for ace-host
  ace-frontend-wgpu/   native renderer + input
  ace-render-cpu/      headless rasterizer for PNG export (tiny-skia)
  ace-py/              PyO3 bindings
  ace-sdk/             Rust plugin SDK (Recompute adapter, macros)
abi/                   plugin message schemas (versioned)
plugins/               first-party plugins (T1–T10)
app/                   reference app
```

## 12. Open questions

1. **Plugin ABI: leaning home-made** (over WIT/Component Model). Shape: a few fixed exports (`alloc`, `free`, `call`) and one host import (`host_call`). Control messages are serialized (MessagePack or similar); column data is passed as raw little-endian arrays, with no per-row encoding. Still open: write it directly on `wasmtime`, or build it on **Extism**, which provides plugin SDKs for many languages. Decide with a small spike of both. The runtime sits behind a trait in `ace-host` ([ADR 0005](adr/0005-split-host-from-runtime.md)). Extism hides linear memory, so the Extism spike also checks that it fits the trait.
2. ~~Graph ownership~~ **Resolved:** the engine only needs to support graphs. How they get built is a UI concern. By default, "Add RSI" wires the nodes automatically. A visual node editor for power users can come later as a plugin.
3. **Text input and IME** inside canvas-only plugins: the host probably needs a text-field primitive so IME works.
4. **Workspace/storage format:** a single file (SQLite?) or a folder of JSON plus Parquet.
5. **Plugin languages beyond Rust** for v0: Rust-only SDK first?
6. **Project name:** keep "AChartEngine" / `ace-*` crates?
