# gui-text: egui renders a long transcript readably

**Status:** built 2026-11-15 by an agent; builds clean and both renderers start. **Not judged yet: the look and the numbers are yours** (see "What to judge"). Gates the GUI crate (ADR 0020).

## Procedure

`spikes/gui-text/` is its own Cargo project, excluded from the workspace. It shows a fake coordinator transcript (no Claude session) in the mockup's column: 720 px measure, 15 px text, line height 1.65, the Mist colours, older turns dimmed, your turns on a panel, inline code, code blocks and folded tool rows. A virtualised scroll lays out only the visible turns. The top bar shows live RSS, peak RSS, frames painted, the last frame's time, the font and the renderer. On quit (`q`) it prints a summary line to stderr.

```
cargo build --release
./target/release/gui-text-spike                    # glow renderer, 2 000 turns
./target/release/gui-text-spike --renderer wgpu    # compare memory
./target/release/gui-text-spike --turns 20000      # stress the scroll
```

Flags: `--font`, `--bold`, `--mono` (font paths), `--size PX`, `--line-height X`, `--turns N`, `--renderer glow|wgpu`.

Keys: `j`/`k` line, `d`/`u` half page, `Space`/`b` page, `g`/`G` top and bottom, `Tab` opens every tool row, `+`/`-` text size, `l` toggles line height 1.65 ↔ 1.4, `q` quits. The mouse wheel and clicking a tool row work too.

**Fonts.** The mockup uses Inter, which is not installed on this machine, so the spike falls back to Noto Sans (body), Noto Sans Medium (bold) and Noto Sans Mono. For the real comparison, install Inter (`sudo pacman -S inter-font`) and rerun: the spike picks it up from `/usr/share/fonts/inter/`. Inter ships as a variable font, and egui draws its default instance only, so bold then falls back to Noto Sans Medium unless a static Inter SemiBold is passed with `--bold`.

## What to judge

| Check | Pass | Result |
| --- | --- | --- |
| Text quality | Close to the HTML mockup at the same size: crisp, even spacing, no blurry or jumpy glyphs | _your call_ |
| Scroll | Smooth with the wheel and `j`/`k`/`Space` through 2 000 turns; still usable at 20 000 | _your call_ |
| Memory | Peak RSS under ~120 MB with glow (wgpu noted for comparison) | _read off the top bar_ |
| Idle | The frame counter stops when nothing moves or is hovered | _read off the top bar_ |
| Frame time | Last frame under ~4 ms while scrolling | _read off the top bar_ |

If text quality fails, compare one alternative toolkit on the same transcript before writing the GUI crate (ADR 0020).

## Findings so far

- egui 0.37 changed `App::update` to `App::ui(&mut Ui)` and `TopBottomPanel` to `Panel::top`. Docs and examples written for 0.3x before that won't compile. The GUI crate pins `=0.37.0`.
- A per-span `line_height` exists in `TextFormat`, which is what the reading column needs; no markdown crate was used, so the GUI crate will render markdown itself from `pulldown-cmark` (already a dependency).
- Release binary with both renderers: 13 MB stripped.
