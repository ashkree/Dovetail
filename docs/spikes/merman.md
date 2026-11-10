# S5: merman renders a 30-node map

**Status:** run 2026-11-10 by an agent. The four machine-measured checks pass. **The look in kitty is not judged yet: that is yours** (see "Look in kitty"). Gates M15 slices (c) renderer and (f) canvas.

**Recommendation:** use merman (pin `=0.9.3`), not the elkjs-in-QuickJS fallback, if you are happy with the look. Two findings M15 must build in: give the selected node a single combined class (a second class is dropped in the PNG), and take node positions from merman's layout JSON.

## Procedure

`spikes/merman/` is its own Cargo project, excluded from the workspace. Run it from that directory:

```
cargo run --release                         # all measures, writes out/
./target/release/merman-spike rss 1920x1080 # memory only, one size per process
./target/release/highlight                  # which highlight syntax reaches the PNG
```

- **Map:** 30 nodes covering all nine kinds (Part 4, File 5, Endpoint 5, Data 3, External 3, Actor 2, Outcome 2, Logic 3, Concept 3). Six groups, two levels deep (Core › Storage and Core › MCP bridge, Terminal UI › Input), and 34 labelled and unlabelled connections. The generated Mermaid is `docs/spikes/merman/map.mmd`.
- **Conversion:** groups become `subgraph`s. Each kind gets a shape and a `classDef` from design.md 23d:

  | Kind | Mermaid shape | Style |
  | --- | --- | --- |
  | Part | `[[…]]` subroutine (double side bars) | clay, light 3 px border |
  | File | `[…]` | off-white, thin grey border |
  | Endpoint | `(…)` rounded | cyan |
  | Data | `[(…)]` cylinder | amber |
  | External | `[…]` | grey, dashed border |
  | Actor | `([☺ …])` stadium with glyph | warm pink |
  | Outcome | `[…]` | green, 5 px border; a red variant for fail |
  | Logic | `[ƒ …]`, lines split by `<br/>` | periwinkle, monospace |
  | Concept | `[…]` | no fill, no border, italic lavender text |

  Front matter: `layout: elk`, `theme: dark`, `look: classic`, direction `TB` (top to bottom).
- **merman:** version 0.9.3, the latest non-alpha 0.x (0.10.0-alpha.2 exists, so it wasn't used). Features: `default-features = false` with `diagram-flowchart`, `layout-elk` and `png`.
- **Machine:** the user's box (12 cores), release build with the workspace's profile (`lto = "thin"`, `codegen-units = 1`, `strip`).

## Results

| Measure | Pass | Result | |
| --- | --- | --- | --- |
| Full render | ≤ 1 s | **233 ms** cold in a new process (about 490 ms on the very first run with a cold disk cache). **182 ms** warm (median of 10). Output was 2039×4096 px | PASS |
| Re-render with selection moved | ≤ 300 ms | **184 ms** median and 193 ms max, moving the highlight to each of the 30 nodes in turn, at the 2039×4096 output. 56–71 ms at pane-sized outputs (table below) | PASS without an overlay |
| Peak added RSS | ≤ 150 MB | **86 MiB** at 2039×4096 px (8.4 MP). **24 MiB** at pane-sized outputs (0.3–1.0 MP) | PASS |
| Node positions | all 30, within 5 px | **30 of 30.** Worst centre error was **1.4 PNG px** (1.0 SVG unit) | PASS |
| Look in kitty | your judgement | not judged | **open, yours** |

Time is almost all rasterising. Parse, ELK layout and SVG take **9 ms**. The rest scales with output pixels:

| Output (fit box or scale) | PNG size | Render with a new selection (median) | Peak added RSS |
| --- | --- | --- | --- |
| fit 1280×800 | 399×800 (0.3 MP) | 56 ms | 24 MiB |
| fit 1920×1080 | 538×1080 (0.6 MP) | 62 ms | 24 MiB |
| fit 2560×1440 | 717×1440 (1.0 MP) | 71 ms | 24 MiB |
| scale 1.0 | 1405×2823 (4.0 MP) | 116 ms | — |
| scale 2.0 (capped at 4096 px by merman's default size limit) | 2039×4096 (8.4 MP) | 185 ms | 86 MiB |

RSS was measured in a separate process for each size (`rss` mode), so the spike's own PNG decoding isn't counted. The main run's "peak added 177 MiB" line includes that decoding and is not the measure.

**Overlay option (not needed, but cheap):** a node-sized selection ring built as its own RGBA image and PNG-encoded takes **0.06 ms**. Layout doesn't move when node classes change (0.000 units drift, checked below), so the canvas could place the ring as a second kitty image over a cached base image. That saves the whole re-render if hover or selection ever needs to feel instant.

### Position recovery (the method M15's canvas needs)

1. **Source:** `RenderRequest::layout_json(...)` returns `layout.FlowchartV2.nodes[]` with `id`, `x`, `y` (the **centre**), `width`, `height` and `is_cluster`, in SVG user units. Groups are in the same list with `is_cluster: true`, which is useful for hit-testing "drill into this group". Use `serde_json` to read it (merman already depends on it). Note that `semantic` has its own `nodes` key, so read from `layout`.
2. **Cross-check with the SVG:** every node is a `<g class="node …" id="<diagram>-flowchart-<id>-<n>" transform="translate(x,y)">`. The translate equals the layout JSON's centre exactly (max |Δ| 0.000 over 30 nodes), so either source works. The JSON is cleaner.
3. **To PNG pixels:** `px = (x − vb.min_x) × png_w / vb.w`, and the same for y. `vb` is the SVG `viewBox` (here `4 4 1404.45 2822.88`, which is the layout `bounds` widened by the 8-unit `diagramPadding`). The factor equals `RasterPlan.effective_scale` (1.4509 here).
4. **Verified against pixels:** a probe render gives each node a unique flat fill and stroke colour. Layout was identical to the real styling (0.000 drift in centre and size). The spike finds each colour's pixel bounding box in the PNG and compares its centre with the prediction from step 3. All 30 nodes match. The worst error is 1.4 px (−1.4, +0.2 on `mcp.rs`), and sizes are within 1.4 px too. Per-node figures are in `spikes/merman/out/positions.txt` after a run.

### Highlighting: one class per node

merman 0.9.3's PNG path uses only a node's **first** class. Three things show in the SVG but **not** in the PNG (0 highlight pixels): a second `class applyfn sel` line, a `style applyfn …` line, and a later `class` line assigning a combined class. What works: leave the selected node out of its kind's `class` line and give it one combined class, the kind's style with the ring's stroke (`classDef logicSel fill:#8f9be8,…,stroke:#ffd84a,stroke-width:6px`). That gives 6800 highlight pixels around the node, against 0 unselected. `src/bin/highlight.rs` reproduces this. M15's converter should emit a `<kind>Sel` class per kind. Compare `docs/spikes/merman/crop-plain-applyfn.png` with `crop-selected-applyfn.png`.

## Dependency weight

| | |
| --- | --- |
| Crates in the tree (merman 0.9.3 with the features above, plus `png`) | 157 |
| Of those, not already in the workspace's `Cargo.lock` | 105, including `resvg`/`usvg`/`tiny-skia`, `rustybuzz`/`ttf-parser`/`fontdb`, `html5ever`, `icu_*`, `url`, `lalrpop-util`, `zune-jpeg`, `gif` and the `merman-*` crates |
| Cold build, release profile (thin LTO, 1 codegen unit) | 107 s |
| Cold build, dev profile | 22 s |
| Binary size added (stripped release) | **≈ 14.0 MB** (14.4 MB spike against a 0.4 MB baseline with `png` only) |
| Licences | MIT OR Apache-2.0, except `merman-elk-layered` (EPL-2.0). Its notice must ship |

This is heavy for a lean-dependency project. It is the largest dependency so far, but it's isolated in `dovetail-maps`, as planned (R9). The rendering memory is transient and inside budget.

## Caveats for M15

- **Fonts:** PNG text uses **system fonts**, loaded once per process on the first PNG render. That load accounts for most of the cold-start gap (233 ms against 182 ms). The look depends on the user's installed fonts. Layout uses merman's own text metrics, so labels could fit badly with an unusual default sans font. They fit here. merman-export can take explicit font files (`NativeFontContext`) if this becomes a problem.
- **Retained memory:** after rendering, resident memory stays near the peak (84 MiB at 8.4 MP, 28 MiB at pane size), because the allocator keeps the pages. Render at the pane's size rather than at the size cap. If needed, render in a short-lived worker or call `malloc_trim`.
- **Shape and size:** ELK's layout of this graph is tall in `TB` (1404×2823 units) and wide in `LR` (5:1; the first trial rendered 4096×768 px in about 104 ms). Fitting the whole map into a 1080p screen gives a 538×1080 image with small text, so zoom will matter more than "fit". Panning and zoom should crop a larger cached render through kitty's source rectangle rather than re-rendering. merman's default size limit caps the output at 4096 px on a side (`RasterOptions::with_size_limit` changes it).
- **Edge routing:** long connections between groups take visible detours (see `map.png`). This is ELK's orthogonal routing on a dense graph. Judge it as part of the look.
- **Two calls:** positions come from `layout_json` and pixels from `png`, so layout runs twice (+9 ms). merman may allow one layout feeding both. Not checked.
- **API churn:** merman is pre-1.0 and 0.10 is in alpha. Pin `=0.9.3` and keep everything behind `MapRenderer`.
- **Background:** the PNG is transparent, with `theme: dark` for text and edges, which suits a dark, transparent terminal. A light terminal would need `theme: default`.

## Look in kitty (yours to judge)

Run `bash spikes/merman/view.sh` from the repo root (or `kitten icat` any of the files below). The images are committed, so no build is needed:

```
kitten icat docs/spikes/merman/legend.png       # the nine kinds plus the red Outcome variant, side by side
kitten icat docs/spikes/merman/map.png          # the 30-node map (2039×4096, tall; scroll up to see the top)
kitten icat docs/spikes/merman/crop-selected-applyfn.png   # the selection ring
```

After `cargo run --release` the same files are in `spikes/merman/out/`.

Please judge: are the nine styles visibly distinct, are labels readable at the size kitty shows them, and is the edge routing acceptable? **Result:** _pending your look._

**Fallback if the look fails:** elkjs in QuickJS behind `MapRenderer`. Rendering would still need an SVG rasteriser (resvg), so most of the dependency weight above would stay. What a look failure would change is the styling and layout engine, not the pipeline.
