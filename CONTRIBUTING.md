# Contributing to AChartEngine

Thanks for helping. [docs/DESIGN.md](docs/DESIGN.md) is the source of truth for architecture.
Read it before proposing structural changes. Larger decisions are recorded as ADRs in
[docs/adr/](docs/adr/).

## Setup

```sh
rustup show            # installs the pinned toolchain from rust-toolchain.toml
cargo install --locked lefthook cargo-deny git-cliff cargo-mutants cargo-shear typos-cli taplo-cli zizmor wasmtime-cli
lefthook install       # git hooks: fmt on commit, message check, clippy+tests on push
```

## Everyday commands

| What | Command |
|------|---------|
| Format | `cargo fmt --all` |
| Lint (same as CI) | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Test | `cargo test --workspace --all-features` |
| `wasm32` check | `cargo check -p ace-core --target wasm32-unknown-unknown` |
| `no_std` check | `cargo check -p ace-core --target wasm32v1-none` |
| Tests on WASM | `cargo test -p ace-core --target wasm32-wasip1` (needs `wasmtime` on `PATH`) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` |
| Dependencies | `cargo deny check` |
| Crate layering | `bash scripts/check-layering.sh` |
| Spelling, TOML, workflows | `typos`, `taplo fmt --check`, `zizmor .github/workflows`, `actionlint` |

## Code rules

The lints in the root `Cargo.toml` and `clippy.toml` are **very strict on purpose**, and CI
rejects any warning. In short:

- **No panics in library code.** No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`
  or `unreachable!`; return errors instead. Tests may unwrap.
- **No printing.** No `println!`, `eprintln!` or `dbg!`. Use `tracing` for diagnostics.
- **Every public item has docs.** Functions that can fail need an `# Errors` section.
- **Suppressing a lint** uses `#[expect(lint, reason = "...")]`, never `#[allow]`. `expect` warns
  once the lint stops firing, so suppressions can't go stale.
- **`unsafe` is denied workspace-wide.** A crate that really needs it (for example the WASM host)
  opts in at crate level with a reason. Every block needs a `// SAFETY:` comment and should do
  only one unsafe operation.
- **Modules** use `foo.rs` plus `foo/`, never `foo/mod.rs`.
- **Errors:** libraries use `thiserror` with typed errors; binaries and tests may use `anyhow`.
- **Keep it simple.** Pick the clever solution only when the simple one clearly fails a
  requirement, and leave a comment explaining why.

### Determinism

The engine must give bit-for-bit identical output for identical input (DESIGN §7). Clippy
enforces the following in core code:

- No wall clock (`SystemTime::now`, `Instant::now`), `thread::sleep` or `env::var`. Time comes
  from the engine's logical clock.
- No `std` float transcendentals (`sin`, `exp`, `powf`, `atan2`, `hypot`, …) or `powi`, because
  their results vary by platform. Use `libm`.
- No `std::collections::HashMap`/`HashSet`, because iteration order is random. Use `BTreeMap`,
  `IndexMap`, or a fixed-seed hasher.

Host or frontend code that really needs one of these uses `#[expect(clippy::disallowed_methods,
reason = "...")]`.

## Tests

- Unit tests go in a `#[cfg(test)] mod tests` at the bottom of the file. Integration tests go in
  `tests/`.
- Snapshot tests use `insta` for draw lists and other structured output. Property tests use
  `proptest` for update-protocol invariants such as "streaming equals full recompute".
- Rendering tests compare images against references with a tolerance.
- Determinism tests run a scenario twice and compare output hashes.
- Benchmarks (`criterion`) go in `benches/` and are needed for anything on a hot path.
- CI runs `cargo mutants` on changed code: it mutates the code and fails if no test catches the
  change. If a mutant survives, add the missing assertion, or exclude truly untestable code in
  `.cargo/mutants.toml` (`exclude_re`) with a comment. Run it locally with `cargo mutants --in-diff <(git diff)`.

## Commits and workflow

- **Small changes** go straight to `main`. Every push must pass the pre-push hook and CI.
- **Larger changes** go through a pull request, so CI and a review run *before* the code lands
  ([ADR 0003](docs/adr/0003-pr-flow.md)). A change is larger if it does any of:
  - adds a crate, or a dependency to an existing crate;
  - changes the plugin ABI, a public API or the architecture (anything DESIGN.md or an ADR
    describes);
  - touches more than about 300 lines, not counting lockfiles and snapshots.

  The PR needs green CI and one independent review (`/code-review`, or a second model that did
  not write the change). Fix what the review finds, then squash-merge with a Conventional
  Commit title.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/), and the
  `commit-msg` hook checks them:
  `type(scope)!: summary`, where type is one of `feat fix perf refactor docs test build ci chore
  style revert`. Keep the first line ≤ 72 characters and the scope lowercase (for example
  `feat(core): add column store`).
- Reference issues in commit bodies (`Closes #12`).
- `CHANGELOG.md` is generated from commits with `git cliff -o CHANGELOG.md`.

## Planning

The vision and current goals live on GitHub:

- **Milestones** are phases (for example "M1 Plugin ABI spike").
- **Issues** are units of work, labelled by area (`area:core`, `area:host`, …) and kind.
- The **project board** shows what's in progress.

Open an issue before starting anything non-trivial.

## License

Contributions are dual-licensed under MIT OR Apache-2.0 without additional terms or conditions,
as defined in the Apache-2.0 license.
