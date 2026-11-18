# Proposal: the `tui-readability` spike

**Status:** proposal, waiting for the user's OK. Not the spike itself. Covers ADR 0030 §5 and §7 step 1, and the `kitty-text-sizing` sub-spike named in its Consequences.

## 1. Goal and what you decide

Render real transcripts in the terminal in two or three variants of ADR 0030 §5, side by side, in kitty, kitty under tmux, and one other terminal. At the end you choose:
- **a variant**, or a mix of them: column width, margins, spacing, heading treatment and backgrounds. It becomes the TUI spec in `docs/ui-session-brief.md` (Readability);
- **whether scaled headings (OSC 66) go in**, given how they behave under tmux;
- **the syntax highlighter crate.** It then gets a row in `docs/implementation-plan.md` §2;
- **the keys for sideways scrolling** of a focused wide block. These are provisional until the design session.

## 2. Inputs: three real transcripts, copied read-only

A read-only scan of `~/.claude/projects/*/*.jsonl` found these. Each covers several of the stresses:

| Fixture | Source (read-only) | Stresses |
| --- | --- | --- |
| `v2-rework.jsonl` | `~/.claude/projects/-home-mave-Documents-projects-Dovetail/d8515de8-3457-4e44-9884-b037996bb1a6.jsonl` (7.5 MB, the v2 rework design) | 84 headings, 15 tables up to 337 cols, 19 code blocks with box-drawn or arrow diagrams |
| `tui-return.jsonl` | `…/-home-mave-Documents-projects-Dovetail/36131e81-c839-48e4-bb7e-cac576c21c6f.jsonl` (1.1 MB, the ADR 0030 grilling) | a 53-line code block, a 316-col table, diagrams |
| `skills-grill.jsonl` | `…/-home-mave-Documents-projects-Dovetail/ecef0f02-3f4c-4d59-8051-50483f8e5ae3.jsonl` (2.1 MB) | the widest table (468 cols), 26-line code, a diagram |

Optional fourth input, a plain markdown document: `docs/research/11-research-where-is-the-codebase-against-the-imple/report.md`. It is already in the repo and exercises reading mode.

**How they're copied.** `spikes/tui-readability/fetch-fixtures.sh` uses `cp` only (no `mv`, no writes under `~/.claude`) into `spikes/tui-readability/fixtures/`. It then trims each copy with `jq` to the user and assistant `text` blocks plus one line per `tool_use` (name and target). Thinking, tool results and attachments are dropped, which also takes the files from MB down to KB.

**Redaction.** `fixtures/` is in the spike's `.gitignore`. Transcripts carry paths, account names and possibly secrets from tool output. Nothing from them is committed unless you look at a trimmed fixture and say so. Screenshots in the results doc show only the rendered text you've seen on screen.

## 3. Variants

All three use the Mist palette from `theme.rs` (`body`, `bold`, `italic`, `code`, `dim`, `line`, `selection`, `speaker_*`), with no new hues. Where a variant needs a tint, it derives it from `selection` (`#2c2e3e`) and `ink` (`#13141a`).

| | **A "quiet"** | **B "kitty"** | **C "dense"** |
| --- | --- | --- | --- |
| Prose column | 88 cols, centred | 88 cols, centred | 110 cols, centred |
| Margins | 4 cols a side, 1 row top | 4 cols a side, 1 row top | 2 cols a side, 0 rows |
| Spacing | 1 blank row between blocks; 2 before an H1/H2 | as A | 1 blank row between blocks; 1 before headings |
| H1 | bold `bold`, full-column `━` rule in `line` below | OSC 66 `s=2` (2× size, 2 rows), `bold`, `━` rule below; fallback as A | bold `bold`, no rule |
| H2 | bold `bold`, `─` rule in `line` below | OSC 66 `n=3:d=2` (1.5×) where kitty has it, else bold; `─` rule | bold `bold` |
| H3+ | bold `italic` colour | bold `italic` colour | bold `italic` colour |
| Tables | box-drawn (`┌─┬─┐`), header in `bold`, fitted to the pane; never wrap | as A | as A, plus zebra rows on `selection` |
| Code | 2-col indent, `code` colour, no highlighting, `lang` label in `dim` | **tinted** block (bg `selection`, full block width) + syntax highlighting | tinted + highlighted |
| Diagrams | like code, never wrap | like code | like code |
| Backgrounds | transparent | transparent; the tint on code blocks only | **solid** `ink` on the content pane (Decision 16's toggle, `S`) |

Wide blocks (tables, code, diagrams) may use the full pane width, not just the prose column (as the brief already allows). If one is still wider than the pane, it is clipped with a `▸` in `dim` at the right edge. When focused, it scrolls sideways.

Table column fitting: give every column its natural width when that fits. Otherwise shrink only the widest columns, down to a 12-col floor, and wrap text inside cells, like today's `fit_columns`. If it still doesn't fit, keep the floors and scroll sideways. The table itself never wraps.

## 4. Program shape

`spikes/tui-readability/` is its own Cargo project with an empty `[workspace]` table, like `spikes/gui-text`. Dependencies: `ratatui` 0.30, `crossterm` 0.29, `pulldown-cmark` 0.13, `serde_json`, `unicode-width`, plus the highlighter under test, behind a cargo feature so RSS can be compared with it on and off.

**It copies the renderer rather than path-depending on `dovetail-tui`.** The spike has to change exactly the code it would depend on: the no-wrap blocks, box tables, the heading pass and the highlight pass. A path dependency would also pull in `dovetail-api` and the rest of the crate. Copied and edited files:
- `crates/dovetail-tui/src/render/markdown.rs` (`render`, `Renderer`, `render_table`, `fit_columns`);
- `render/wrap.rs` (`wrap_spans`, `line_width`);
- `render/reading_column.rs` (`column`, `reading_text`);
- `theme.rs` (`Theme::mist`, `READING_PADDING`).

The diff against the originals is the input to the real build.

The one structural change is that `render` returns blocks, not just lines: `Vec<Block>`, with `Block::Prose(Vec<Line>)`, `Block::Wide { lines, width, kind }` and `Block::Heading { level, text }`. Layout then places prose in the column and wide blocks across the pane, and the draw step knows where headings sit for OSC 66.

**Keys** (provisional):
- `j`/`k`, `d`/`u`, `g`/`G` scroll.
- `Tab`/`Shift-Tab` focus the next or previous wide block in view (a `▎` in `focus` marks it).
- `h`/`l` scroll the focused block 8 cols, and `0`/`$` jump to its start or end.
- `1` `2` `3` switch variant; `t` cycles transcripts; `S` toggles solid backgrounds; `H` toggles highlighting.
- `q` quits.

The status line shows the variant, the transcript, `osc66: on|off|forced-off`, the last frame's time, bytes written and RSS.

**Flags:** `--variant a|b|c`, `--fixture PATH`, `--no-osc66` (force the fallback), `--force-osc66` (skip detection, to see breakage), `--no-highlight`, `--probe` (run detection only, print the result, exit).

**Detection** (this is `kitty-text-sizing`, folded in; see below):
1. Skip it, with OSC 66 off, when `$TMUX` is set (unless `--force-osc66`) or `$TERM` isn't `xterm-kitty`.
2. Otherwise, in raw mode, write `\r` + `ESC]66;w=2; BEL` + `ESC[6n` (CPR) + `ESC[c` (DA1), and read stdin until the DA1 reply, with a 150 ms cap.
3. Supported means a CPR column of 3 (the cursor moved two cells). A terminal that ignores OSC 66 reports 1. DA1 always answers, so there is no hang, and the timeout covers terminals that answer nothing.
4. Clear the probe line afterwards.

**Drawing scaled headings.** ratatui's buffer keeps the heading's rows filled with blank cells. After `terminal.draw`, the spike writes each visible heading itself with `MoveTo` + OSC 66. A heading cut by the top or bottom edge, or narrower than its scaled width, is drawn at normal size.

**Fold in `kitty-text-sizing`, and run it first.** It is the same binary: `--probe` plus variant B is the sub-spike. Phase 1 runs `--probe` and variant B's headings in kitty, then kitty under tmux. If OSC 66 can't be drawn cleanly under ratatui, phase 2 drops B's heading scale and keeps B's other features. The results doc gets a `kitty-text-sizing` section, so the ADR's named spike has an answer.

**Highlighter candidates** (rough added weights, measured in the spike):

| Crate | How | Added weight (approx.) |
| --- | --- | --- |
| `syntect` 5, `default-features = false`, `default-syntaxes` + `regex-fancy` | Sublime grammars, pure Rust (`fancy-regex`), lazy per-syntax regex compile | ~10 crates (`fancy-regex`, `bincode`, `flate2`, `serde`…); +2–3 MB binary; a few MB RSS once used |
| `tree-sitter-highlight` + 5 grammars (rust, toml, bash, json, ts) | Real parsers in C, compiled by `cc` | small Rust deps, but a C toolchain and ~1 MB per grammar; only listed languages |
| `synoptic` 2 | Hand-written regex rules per language | ~2 crates (`regex`, `unicode-width`); <1 MB; coarse, few languages |

**Recommendation: `syntect` with the features above.** Transcripts contain any language. Its scopes map onto the theme's names (keyword → `bold`, string → `code`, comment → `dim`, type → `italic`), so no theme files are shipped. It's pure Rust, with no C build. It's lazy: no cost until a code block is shown. `synoptic` is the fallback if the RSS delta is over ~8 MB.

## 5. What gets recorded in `docs/spikes/tui-readability.md`

Same shape as `docs/spikes/gui-text.md`: status, recommendation, procedure, a "What to judge" table with Pass and Result columns, then findings.
- **Screenshots** of each variant × each fixture, at one scroll position with a wide table and code in view. Taken in kitty 0.50, kitty under tmux, and one non-kitty terminal. Saved in `docs/spikes/tui-readability/`, and only after you've seen them (redaction).
- **Text sizing:** the `--probe` result in each terminal, and whether scaled headings drew, scrolled, resized and cleared cleanly. This includes a heading cut by the edge, and tmux with `allow-passthrough on` and off.
- **Redraw cost:** frame time (median and p95 over a scripted scroll through `v2-rework`), with highlighting on and off, and bytes written per frame with OSC 66 on and off.
- **Memory:** RSS at start, after loading the fixture, and after highlighting every code block (`/proc/self/status` VmRSS). The highlighter's delta is from the feature on vs off, against the TUI's ~100 MB budget.
- **Width checks:** any glyph where `unicode-width` and the terminal disagree (box-drawing, `━`, emoji in transcripts).
- **What failed**, and the changes from the copied renderer that the real build should take.

## 6. Risks and open questions

- **OSC 66 vs ratatui's cell model.** ratatui thinks each cell holds one width-1 symbol. A 2× heading covers 2×N cells that ratatui believes are blank, so its diff may overwrite part of a multicell, and kitty then erases the whole heading. Plan: re-emit visible headings after every draw. This costs bytes per frame, so it is measured. A custom `Buffer` cell marker is the heavier alternative.
- **tmux.** tmux doesn't understand OSC 66. With passthrough the sequence reaches kitty, but tmux's own grid desyncs, so the default under tmux is off. **tmux isn't installed on this machine** (`pacman -S tmux` is needed for the tmux screenshots). Only xterm is available as the non-kitty terminal; foot or alacritty would be more representative.
- **Detection timing.** The probe runs before the alternate screen and has to tolerate input typed during it. Kitty earlier than 0.40 has no OSC 66.
- **Fractional scale (`n=3:d=2`)** may look uneven next to integer cells. If it does, B uses `s=2` for H1 only.
- **Highlighter memory.** syntect's full default syntax set may cost more than expected once many languages have been seen. The fallback is a curated syntax dump of about 15 languages.
- **Wide-block focus vs the design session.** `Tab` and `h`/`l` may clash with the new main layout's focus keys, so the keys stay provisional.
- **Open:** whether the solid background (C) should apply to the prose column only or the whole pane. Whether zebra rows are worth their noise. Whether diagrams (code blocks with box-drawing) should skip highlighting by default.
