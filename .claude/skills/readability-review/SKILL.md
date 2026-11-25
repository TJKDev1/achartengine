---
name: readability-review
description: Review a diff, PR or file for readability against docs/STYLE.md. Use on every PR next to /code-review (ADR 0004), or when asked whether code or docs read well.
---

# Readability review

This review asks whether a newcomer would understand the change on the first read and find it
pleasant. It doesn't look for bugs; `/code-review` does that.

## Who reviews

The reviewer must not be the model or session that wrote the change, because writers don't see
their own habits. If you wrote it, start a fresh agent and give it only this file, the target and
`docs/STYLE.md`, not your reasoning about the change.

## Steps

1. Read `docs/STYLE.md` in full, and any exemplar modules it lists.
2. Get the target: `gh pr diff <n>` for a PR, `git diff origin/main...HEAD` for a branch, or the
   files named. Read the changed files whole, not just the hunks, so you see the change in context.
3. Read the change as a newcomer would, top to bottom, once. Note every place you had to reread,
   jump elsewhere to understand, or skim because it was dull.
4. Then check it against STYLE.md:
   - names, and function size and shape
   - needless layers of abstraction
   - comments that restate the code, and rustdoc that pads
   - lists that should be paragraphs
   - the prose patterns to avoid, in docs, comments and commit messages

## Report

List findings, most important first. For each give the location (`path:line`), what makes it
hard to read, and a concrete rewrite. A finding without a rewrite isn't actionable, so leave it
out.

Report only what a reader would notice. Skip anything clippy, rustfmt or
`scripts/check-prose.sh` already catches. If the change reads well, say so in one line and stop.
