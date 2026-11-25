# 0003: Pull requests for larger changes

- Status: accepted; ADR 0004 adds a readability review
- Date: 2026-11-25
- Issue: #27

## Context

Everything was committed straight to `main`, so CI only ran after the code had landed. Much of
the code is AI-authored, and the author reviewing its own work misses things a fresh reviewer
catches. Small fixes, though, shouldn't pay for a PR round trip.

## Options

1. **Always direct to `main`.** Fastest, but larger changes land unreviewed and CI catches
   problems only afterwards.
2. **Always PRs, with branch protection.** Every change is checked first, at the cost of
   friction on one-line fixes.
3. **Hybrid.** Small changes go direct; larger changes go through a PR with required CI and an
   independent review.

## Decision

Option 3. A change is "larger" if it adds a crate or dependency, changes the plugin ABI, a
public API or the architecture, or touches more than about 300 lines (lockfiles and snapshots
excluded). Such a PR needs green CI and one review by `/code-review` or a model that didn't
write the change, and is squash-merged with a Conventional Commit title.

We don't turn on branch protection: required checks would block the direct pushes that small
changes still use. The rule is enforced by convention (CONTRIBUTING.md, CLAUDE.md).

## Consequences

- Larger changes take one more step, but problems show up before they reach `main`.
- If larger changes still slip through directly, revisit and switch to option 2.
