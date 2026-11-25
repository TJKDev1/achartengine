# AChartEngine: notes for Claude

A Rust charting engine with a headless core, sandboxed WASM plugins and a native reference app.
**[docs/DESIGN.md](docs/DESIGN.md) is the source of truth.** Read the relevant section before
changing architecture, and update it (or add an ADR in `docs/adr/`) when a decision changes.

## Commands

- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Test: `cargo test --workspace --all-features`
- wasm32: `cargo check -p ace-core --target wasm32-unknown-unknown`
- Docs: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- Dependencies: `cargo deny check` and `cargo shear`
- Mutation tests: `cargo mutants --in-diff <(git diff HEAD)`

Hooks: rustfmt runs after every Edit/Write, and a Stop hook runs clippy when `.rs`/`.toml`
files changed. Fix what it reports; never silence it.

## Rules

- The lints are strict and denied (see `CONTRIBUTING.md`). No `unwrap`/`expect`/`panic` in library
  code. Suppress a lint only with `#[expect(lint, reason = "...")]`, never `#[allow]`.
- Determinism: no wall clock, sleep, env vars, std float transcendentals (use `libm`) or std
  `HashMap`/`HashSet` in engine code. clippy.toml enforces this.
- `ace-core` has no I/O, no threads and no OS APIs, and must build for `wasm32-unknown-unknown`.
- No panicking indexing either (`v[i]`, `&s[a..b]`): use `get`, iterators or checked ranges.
  No `_ =>` arms on our own enums; list the variants.
- Tests must fail when the behaviour breaks. CI's `cargo mutants` checks this on every change.
- Don't add a dependency without asking first.
- Simple over clever. Add complexity only when the simple version fails a requirement, with a
  comment explaining why.
- Modules use `foo.rs` + `foo/`, not `mod.rs`.

## Workflow

- Commit directly to `main` with Conventional Commits (`feat(core): ...`); a hook checks them.
- Planning lives on GitHub (TJKDev1/achartengine): milestones = phases, issues = work items, plus
  the project board. Reference issues in commits (`Closes #N`). File new work as issues instead
  of leaving TODOs.
- Ask the user before making architectural decisions that DESIGN.md doesn't settle.
