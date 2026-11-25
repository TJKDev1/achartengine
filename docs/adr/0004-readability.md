# 0004: Readability checks

- Status: accepted
- Date: 2026-11-25
- Issue: #37

## Context

Most code and docs here are written with AI help. The existing checks (clippy, tests, mutants,
the `/code-review` step from ADR 0003) catch wrong code but don't flag code that is hard or
tiring to read. AI-written text fails in recognisable ways: comments that restate the code,
padding words, lists where reasoning belongs, abstractions with one user, and a style that
drifts from session to session. A model also misses these habits in its own output, even with a
style guide in context: the first draft of `docs/STYLE.md` broke its own rules in its opening
paragraph.

## Options

1. **A style guide only.** Cheap, but nothing checks it, and writers miss their own habits.
2. **Automated checks only** (clippy limits, a word list, readability scores). They catch the
   mechanical part, but most of what makes text unpleasant isn't mechanical, and readability
   scores are noisy and easy to game.
3. **Guide, cheap checks, an independent review and a human read.**

## Decision

Option 3:

- `docs/STYLE.md` says what we want, with examples.
- Clippy limits function length (60 lines), arguments (5) and nesting depth (5). The limits are
  rough and flag functions worth a second look.
- `scripts/check-prose.sh` flags a list of padding and sales words in docs, Rust comments and
  commit messages. It runs in CI and the git hooks. No new tool is needed; Vale could replace it
  if the list outgrows grep.
- Every PR under ADR 0003 also gets a readability review by a model that didn't write it
  (`.claude/skills/readability-review`). Like the code review, it is required by convention.
- At each milestone a person reads one module and one doc page in full.

## Consequences

- PRs take one more review pass.
- An `#[expect]` on a size lint needs a reason, as with any other lint.
- The word list will produce some false hits; `prose:allow` handles those per line.
- If the readability review keeps finding the same problem, add it to STYLE.md or a check.
