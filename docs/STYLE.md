# Style

Write so that someone new to a file understands it on the first read. Clippy's size limits and
the word list in `scripts/check-prose.sh` are enforced. The rest is judgment: if a rule here makes
something harder to read, break it.

## Code

**Write the obvious version first.** A plain function beats a trait with one implementation, and
a struct with public fields beats a builder for three fields. Add a layer only when the simple
version fails a requirement, and say which one in a comment (DESIGN §2, principle 6).

**Name things after what they mean.**

| Instead of | Write |
|------------|-------|
| `data`, `info`, `item`, `val` | `bars`, `visible_range`, `last_close` |
| `process`, `handle`, `do_update` | `append_bar`, `resample`, `clip_to_viewport` |
| `Manager`, `Helper`, `Utils` | `PluginRegistry`, `TickFormatter` |

Short names are fine when the scope is a few lines or the maths is standard (`i`, `x`, `y`, `dt`).
Replace a `bool` parameter with an enum such as `Side::Buy` / `Side::Sell`.

**Keep functions short and flat.** If a function needs a comment to mark where its next part
starts, make that part its own function. Prefer early returns to nesting. Clippy limits length,
arguments and nesting depth (see `clippy.toml`).

**Order a file for the reader:** public items first, the most important at the top, helpers
below their callers, tests at the bottom.

**Copy the exemplars.** People and models copy nearby code, so new code should follow a few
chosen modules. They'll be listed here once picked (#38).

## Comments

A comment says why the code is the way it is, or warns about something the code can't show.

```rust
// Bad: restates the code.
// Increment the index.
index += 1;

// Good: says what the code can't.
// Bars can arrive out of order during a revise, so search instead of assuming the last slot.
let slot = bars.partition_point(|bar| bar.time < time);
```

Delete comments that narrate (`// Now we loop over the series`), mark sections of a function, or
describe a change (`// Fixed the off-by-one` belongs in the commit message).

## Rustdoc

Start with one short sentence saying what the item does. Then add what a caller needs: units,
edge cases, errors, and an example if the use isn't obvious.

```rust
// Bad: repeats the signature and pads.
/// This function is responsible for computing the simple moving average. It takes the
/// input values and a period, and returns the computed values.

// Good:
/// Simple moving average over the last `period` values.
///
/// The first `period - 1` outputs are `None`, because there isn't a full window yet.
```

## Prose

This covers docs, comments and commit messages.

**Use paragraphs for reasoning.** Lists are for steps, options and checklists. When ideas depend
on each other ("because", "so", "unless"), a paragraph keeps those links and a list drops them.
Don't add a heading for three sentences.

**State things once.** Each point goes in one place, without hedging. If something is uncertain,
say what and why.

**Commit messages:** the subject says what changed, in the imperative (`feat(core): add column
store`). The body says why, plus anything a reviewer should know. The diff already shows which
files changed.

### Patterns to avoid

Writers rarely notice these in their own text, so look for them when you reread.

- **Announcements and summaries:** "This section describes…", "In short, …". Start with the
  content and stop when it's done.
- **Contrast framing:** "X, not just Y", "it's not A, it's B". Say X.
- **Lists of three for rhythm:** "fast, safe and simple". Keep the items that carry information.
- **Punchlines:** a short closing sentence that restates the paragraph ("Stop there.", "That's
  the point.").
- **Vague claims:** "rendering is fast". Give the number: "rendering 10,000 bars takes 2 ms".
- **Padding and sales words**, listed below.

### Words to avoid

`scripts/check-prose.sh` flags these in docs, comments and commit messages:

- Sales talk: robust, seamless(ly), powerful, cutting-edge, state-of-the-art, best-in-class,
  world-class, blazing(ly) fast, game-changer, elegant, empower, holistic, synergy
- Inflated verbs: leverage, utilize, facilitate, streamline, delve
- Filler: it's important to note, it is worth noting, needless to say, in order to, basically,
  essentially, simply, a wide range of, plethora, myriad, tapestry
- Stacked connectors: furthermore, moreover, in conclusion, in summary

If a flagged word is correct (an `elegant` theme), add `prose:allow` to the line, as
`<!-- prose:allow -->` in Markdown.

## How this is checked

The clippy limits and `scripts/check-prose.sh` run in CI and the git hooks. Every PR also gets a
readability review against this file by a model that didn't write the change
(`.claude/skills/readability-review`, ADR 0004). At each milestone a person reads one module and
one doc page in full and files an issue if either was unpleasant to read, because the patterns
above get past both the tools and a model checking its own text.
