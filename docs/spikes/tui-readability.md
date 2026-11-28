# tui-readability: tables, code, diagrams and headings in the TUI (with kitty-text-sizing)

**Status:** decided 2026-11-27 (D49, after user run in kitty, task #36). Gates the TUI design session (ADR 0030 §7 step 1) and the `render/` changes in the v2 views.

**Decision (2026-11-27, D49):**
- Adopt Variant A's quiet layout and non-scaled headings: no OSC 66 scaled headings, 88-col centred prose, 4-col margins, transparent background by default (solid-background toggle per ADR 0030 §5), box-drawn fitted tables (12-col floor, per-block sideways panning).
- Pair with syntax highlighting on code blocks.
- **Rejected:** Variant B (OSC 66 scaled headings); **rejected:** pure Variant A (no syntax highlighting).
- Highlighter crate choice (`syntect` curated set vs `synoptic` or lighter alternative) stays open until adoption, when it will get a row in `docs/implementation-plan.md` §2.

Proposal: `docs/plan/tui-readability-spike.md`. Code: `spikes/tui-readability/` (own Cargo project, empty `[workspace]`, nothing in `crates/` changed).

## Procedure

```
spikes/tui-readability/fetch-fixtures.sh     # cp (read-only) + jq trim into fixtures/ (git-ignored)
spikes/tui-readability/bench.sh              # headless numbers below, out/bench.txt
spikes/tui-readability/run.sh                # the user's run: from kitty, phases 1 and 2
```

`run.sh` runs `--probe` at startup (no prompt; saved to `out/probe-kitty.txt`, one `Detection OK` or `Detection problem` line). Then Part 1 (scaled headings): variant B's scaled headings, then its fallback with `--no-osc66`. Then Part 2 (compare variants): variant A, B and C on every fixture in kitty. Steps are numbered `Step N of 11` with no letters, so they never clash with variant A/B/C. It prints what to look at before each step. Judged in kitty only. **Phase 3 (kitty under tmux) was dropped on 2026-11-21 by the user's decision**: the script no longer has a tmux phase or argument, and the tmux rows below are marked *dropped*. No non-kitty terminal (decision on #36). `--no-osc66` shows the fallback.

Fixtures (trimmed to text blocks and one line per tool call, so 7.7 MB becomes 222 KB): `v2-rework` (419 lines), `tui-return` (46), `skills-grill` (152). The optional plain-markdown report was not in the tree, so there is no fourth. Nothing from `fixtures/`, `out/` or screenshots is committed.

Flags: `--variant a|b|c`, `--fixture PATH`, `--no-osc66`, `--force-osc66` (under tmux this wraps OSC 66 in DCS passthrough), `--no-highlight`, `--probe`, `--bench [--osc on|off]`. Keys: `j k d u g G`, `Tab`/`Shift-Tab` focus a wide block, `h l` pan 8, `0 $`, `1 2 3` variant, `t` transcript, `S` solid, `H` highlight, `O` OSC 66 on/off live, `q`.

## What was built, and where it departs from the proposal

- Variants A, B, C as tabled in the proposal (`src/layout.rs`: 88/88/110 cols, margins, gaps, rules, tint, zebra, solid). Tables are box-drawn with a 12-col floor; code/diagram/table blocks use the pane width, centred, clipped with `▸`/`◂` and panned per block.
- **Fractional H2 changed.** The proposal's `n=3:d=2` is invalid: kitty's protocol allows `n < d` only (fractional scale only shrinks, inside an `s × w` box). B's H2 is `s=2:n=3:d=4:w=2` per two characters (1.5× text in 2×2 boxes). It may look uneven; that is a thing to judge. H1 is `s=2`. Non-ASCII headings and headings wider than the column fall back to normal size.
- Detection follows kitty's documented three-CPR sequence (CR, CPR, `w=2` space, CPR, `s=2` space, CPR), using crossterm's `cursor::position()` (2 s timeout if a terminal answers nothing). No DA1 trick is needed.
- Highlighting uses syntect's `ParseState` and `ScopeStack` directly and maps scopes onto Mist (keyword → bold, string → `code`, comment → `dim`, type → `italic`, number, function name). No theme files, no `highlighting` feature.
- Cargo feature `highlight` (default on) gates syntect, so RSS compares on and off.
- ratatui's backend writer is behind the `unstable-backend-writer` feature; the spike enables it to write the OSC 66 pass and count bytes. The real build needs that, or its own `Backend` wrapper.

## kitty-text-sizing

| Check | Pass | Result |
| --- | --- | --- |
| `--probe` in kitty | detect ON; `w=2` and `s=2` both move the cursor | *pending* |
| Scaled headings draw, scroll, resize, clear | no ghost cells or stripes, no flicker over a long scroll | *pending* |
| Heading cut by the top or bottom edge | drawn at normal size | *pending* (implemented: only headings fully in view are drawn scaled) |
| `--probe` under tmux | detect OFF (tmux answers CPR itself); record the forced probe | tmux without passthrough: detect OFF, forced probe skipped by default. *Dropped 2026-11-21 (user's decision; phase 3 removed).* |
| OSC 66 forced through tmux, passthrough on and off | works cleanly, or breaks (the proposal expects a desync) | *dropped 2026-11-21 (user's decision; phase 3 removed)* |
| Bytes written per frame, OSC 66 on vs off | small | **+36 B per frame** (B on `v2-rework`: p50 2 312 vs 2 276 B with OSC 66 off) |

The heading pass re-emits every fully visible scaled heading after each `terminal.draw` (a `MoveTo`, colour and OSC 66 per heading). It is cheap in bytes; whether kitty and ratatui's diff fight over the multicell cells is what only the user's run shows. If B's scaling cannot be drawn cleanly, phase 2 keeps B without it (`--no-osc66` is already that).

## What to judge

| Check | Pass | Result |
| --- | --- | --- |
| Variant A, B, C on each fixture (prose width, margins, spacing, headings) | a clear pick or mix, per variant × fixture | *pending* |
| Tables up to 468 cols | readable at natural width when it fits, wrapped in cells to the 12-col floor, `▸` clip and `Tab`/`h`/`l` pan when not | *pending* |
| Diagrams (box-drawing code) never wrap, align | glyphs line up | *pending* |
| Tinted code, highlighting on/off (`H`) | highlighting earns its place | *pending* |
| Solid vs transparent (`S`) | legibility gain worth the toggle | *pending* |
| Sideways-scroll keys (`Tab`, `h`/`l`, `0`/`$`) | usable | *pending* |
| Under tmux, OSC 66 off by default | B falls back cleanly | *dropped 2026-11-21 (user's decision; phase 3 removed)* |

## Redraw cost (measured, headless)

A fixed 200×50 viewport over a byte-counting sink, scrolling 3 rows a frame through the whole fixture, release build. Time is `draw` + the OSC 66 pass + flush into the sink, so it is the TUI's own cost and **not** kitty's rendering cost.

| Fixture, variant | Rows | Frames | Frame p50 / p95 | Bytes p50 / mean / p95 |
| --- | --- | --- | --- | --- |
| `v2-rework`, A | 5 425 | 1 801 | 1.09 / 1.24 ms | 2 255 / 2 271 / 2 890 |
| `v2-rework`, B (OSC 66 on, 91 scaled headings) | 5 516 | 1 831 | 1.09 / 1.25 ms | 2 312 / 2 347 / 3 021 |
| `v2-rework`, B (OSC 66 off) | 5 425 | 1 801 | 1.09 / 1.24 ms | 2 276 / 2 300 / 2 923 |
| `v2-rework`, C (solid) | 5 072 | 1 683 | 1.11 / 1.24 ms | 2 597 / 2 748 / 4 160 |

The other two fixtures give the same shape (p50 1.09–1.12 ms; C costs ~400 B more per frame than A). The status line changes every frame (it shows the timing), which is most of the ~2.2 KB; a real status line would not. Highlighting on vs off does not change frame time: highlighting happens once at layout. Layout (markdown + highlight + layout) for `v2-rework`: 9 ms without highlighting, 28–31 ms with, almost all of it compiling the grammars used.

## Memory (measured, `/proc/self/status` VmRSS)

| Build / point | `v2-rework` | `tui-return` | `skills-grill` |
| --- | --- | --- | --- |
| Start | 4.0 MB | 3.9 MB | 4.0 MB |
| Loaded and laid out (A, no highlighting) | 13.3 MB | 7.0 MB | 8.5 MB |
| Loaded and laid out (B, highlighting on) | 26.5 MB | 7.0 MB | 10.2 MB |
| After highlighting every code block | 26.9 MB | 8.1 MB | 10.5 MB |
| Feature `highlight` **off**, after highlighting | 12.9 MB | | |

- **syntect's cost depends on the language first used, +13.5 MB on `v2-rework`** (12.9 → 26.5 MB), whose labelled blocks are mostly `bash`. The bench calls `highlight::warm()` (loads the syntax set) on every run: on `tui-return`, which has nothing it can highlight, that alone is about +1 MB (7.0 → 8.1 MB). `skills-grill` (one `diff` block) is +2.4 MB (8.5 → 10.8 MB). So the set itself is cheap and the first use of a big grammar such as bash is not; I did not profile why (likely deserialising and compiling that language's contexts). Over the proposal's ~8 MB bar for falling back, in the worst case seen.
- **Binary:** 3.39 MB with syntect, 1.15 MB without (+2.2 MB).
- The whole spike is 13 MB with a 5 500-row document and no highlighter, so the TUI's ~100 MB budget is not at stake from rendering; the highlighter would be the biggest single piece of it.

## Width checks

`--probe` compares `unicode-width` with the terminal's cursor after printing each distinct non-ASCII glyph of the fixtures (37 of them: box-drawing, `━`, arrows, `⚙`, `›`, `▸`, `◂`, `▎`, `☑`…). Run under tmux (which has its own width table): **0 mismatches**. Kitty's result: *pending* (`out/probe-kitty.txt`). Emoji in the transcripts are not in this set.

## Findings so far

- **Transcript code fences are almost never labelled.** `v2-rework`: 54 bare fences and closers, 5 `bash`, 1 `ini`; `tui-return`: 1 `text`; `skills-grill`: 1 `diff`, 4 `text`. Only 5 of `v2-rework`'s 30 code blocks and 1 of `skills-grill`'s 9 were highlightable. Highlighting helps a minority of blocks; most are plain text or diagrams. Without a language, guessing one (syntect's first-line detection) would cost more than it gives. This weighs against paying 13 MB for the full syntax set.
- syntect's default syntax set has no TOML or TypeScript; those would be plain.
- Diagrams are detected (box-drawing or arrow glyphs, or `mermaid`) and skip highlighting by default, as the proposal left open.
- Scaled headings add almost nothing in bytes. The risk is visual, not cost.

## Changes the real build should take from the copied renderer

- `render` returns blocks (`Prose`, `Heading`, `Wide`, `Rule`) and layout owns spacing; prose and wide blocks are placed separately (`markdown.rs`, `layout.rs`).
- Box-drawn tables with a 12-col floor and per-block sideways panning (`WideInfo`, `slice_line`).
- Detection and heading pass need the backend writer (see above).
- Reconsider highlighting before it gets a row in `docs/implementation-plan.md` §2: highlighter crate choice (`syntect` curated set vs `synoptic` or lighter alternative) stays open until adoption.
