# gui-text: egui renders a long transcript readably

**Status:** run 2026-11-15 by the user. **Passes**, with the memory bar raised by the user (see Results). Gates the GUI crate (ADR 0020).

**Recommendation:** build the GUI crate on egui 0.37 with the glow renderer. Tune scrolling in the GUI crate; it is smooth enough, not yet as smooth as wanted.

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
| Text quality | Close to the HTML mockup at the same size: crisp, even spacing, no blurry or jumpy glyphs | Not objected to; not reported in detail |
| Scroll | Smooth with the wheel and `j`/`k`/`Space` through 2 000 turns; still usable at 20 000 | Smooth enough, not as smooth as wanted |
| Memory | Peak RSS under ~120 MB with glow (wgpu noted for comparison) | glow 216 MB, wgpu 268 MB. Over the bar; **the user accepts it** as small next to Claude Code |
| Idle | The frame counter stops when nothing moves or is hovered | Not reported |
| Frame time | Last frame under ~4 ms while scrolling | Not reported |

If text quality fails, compare one alternative toolkit on the same transcript before writing the GUI crate (ADR 0020).

## Results

- **Renderer:** glow, 52 MB less than wgpu.
- **Memory:** RSS counts the mapped GPU driver and shared libraries, so much of the 216 MB is not egui's own heap. The GUI crate's budget is set from this run: about 220 MB with glow.
- **Scrolling** to tune in the GUI crate: the spike's scroll is egui's default animation on a virtualised list with estimated row heights, so the first pass over unseen turns can jump as heights are measured.

## Findings

- egui 0.37 changed `App::update` to `App::ui(&mut Ui)` and `TopBottomPanel` to `Panel::top`. Docs and examples written for 0.3x before that won't compile. The GUI crate pins `=0.37.0`.
- A per-span `line_height` exists in `TextFormat`, which is what the reading column needs; no markdown crate was used, so the GUI crate will render markdown itself from `pulldown-cmark` (already a dependency).
- Release binary with both renderers: 13 MB stripped.
