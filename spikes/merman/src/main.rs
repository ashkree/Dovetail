//! S5 spike: can merman draw Dovetail's structure maps?
//!
//! Builds a 30-node map (nine node kinds, two levels of nested groups), converts it to a
//! Mermaid flowchart, renders it to PNG with merman, and measures:
//! - full render time (cold and warm),
//! - re-render time with the highlight class moved to another node,
//! - peak added RSS,
//! - node positions from merman's layout JSON and SVG, checked against the PNG's pixels.
//!
//! Run: `cargo run --release` from `spikes/merman/`. Writes `out/map.png` (and friends).
//! Throwaway code: no error handling beyond `expect`.

use merman::svg::export::{RasterFitBox, RasterOptions};
use merman::{OperationControl, PngRequest, RenderOutput, RenderRequest, Renderer, SvgRequest};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- the map model

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Part,
    File,
    Endpoint,
    Data,
    External,
    Actor,
    Outcome,
    Logic,
    Concept,
}

impl Kind {
    const ALL: [Kind; 9] = [
        Kind::Part,
        Kind::File,
        Kind::Endpoint,
        Kind::Data,
        Kind::External,
        Kind::Actor,
        Kind::Outcome,
        Kind::Logic,
        Kind::Concept,
    ];

    fn class(self) -> &'static str {
        match self {
            Kind::Part => "part",
            Kind::File => "file",
            Kind::Endpoint => "endpoint",
            Kind::Data => "data",
            Kind::External => "external",
            Kind::Actor => "actor",
            Kind::Outcome => "outcome",
            Kind::Logic => "logic",
            Kind::Concept => "concept",
        }
    }

    /// Mermaid shape per kind (design.md 23d).
    fn shape(self, id: &str, label: &str) -> String {
        match self {
            Kind::Part => format!("{id}[[\"{label}\"]]"), // subroutine: double side bars
            Kind::File => format!("{id}[\"{label}\"]"),
            Kind::Endpoint => format!("{id}(\"{label}\")"), // rounded
            Kind::Data => format!("{id}[(\"{label}\")]"),  // cylinder
            Kind::External => format!("{id}[\"{label}\"]"), // dashed via class
            Kind::Actor => format!("{id}([\"☺ {label}\"])"), // stadium + person glyph
            Kind::Outcome => format!("{id}[\"{label}\"]"), // heavy border via class
            Kind::Logic => format!("{id}[\"ƒ {label}\"]"), // code card via class
            Kind::Concept => format!("{id}[\"{label}\"]"), // no box via class
        }
    }

    /// Style per kind (design.md 23d colours), for a dark terminal.
    fn class_def(self) -> &'static str {
        match self {
            Kind::Part => "fill:#c9785d,stroke:#f2c2a8,stroke-width:3px,color:#1b1b1b",
            Kind::File => "fill:#f3efe6,stroke:#9a958c,stroke-width:1px,color:#1b1b1b",
            Kind::Endpoint => "fill:#4cc3d9,stroke:#1d7f91,stroke-width:2px,color:#0b2a30",
            Kind::Data => "fill:#e8a93a,stroke:#9b6a12,stroke-width:2px,color:#2a1b00",
            Kind::External => {
                "fill:#5c5f66,stroke:#c3c6cc,stroke-width:2px,stroke-dasharray:6 4,color:#eceff4"
            }
            Kind::Actor => "fill:#f2a1b6,stroke:#b5546f,stroke-width:2px,color:#3a0f1c",
            Kind::Outcome => "fill:#5fbf6a,stroke:#1f6b2a,stroke-width:5px,color:#0d2a11",
            Kind::Logic => {
                "fill:#8f9be8,stroke:#4c58a8,stroke-width:1px,color:#101433,font-family:monospace"
            }
            Kind::Concept => "fill:none,stroke:none,color:#c8b6f0,font-style:italic",
        }
    }
}

struct Node {
    id: &'static str,
    kind: Kind,
    label: &'static str,
}

struct Group {
    id: &'static str,
    title: &'static str,
    parent: Option<&'static str>,
    members: &'static [&'static str],
}

struct Map {
    nodes: Vec<Node>,
    groups: Vec<Group>,
    edges: Vec<(&'static str, &'static str, &'static str)>,
}

fn sample_map() -> Map {
    use Kind::*;
    let n = |id, kind, label| Node { id, kind, label };
    let nodes = vec![
        n("user", Actor, "You"),
        n("coordagent", Actor, "Coordinator agent"),
        n("tui", Part, "dovetail-tui"),
        n("core", Part, "dovetail-core"),
        n("driver", Part, "dovetail-driver"),
        n("api", Part, "dovetail-api"),
        n("mainrs", File, "src/main.rs"),
        n("apprs", File, "app.rs"),
        n("keymaprs", File, "keymap.rs"),
        n("dbrs", File, "db.rs"),
        n("mcprs", File, "mcp.rs"),
        n("mcpcmd", Endpoint, "dovetail mcp"),
        n("socket", Endpoint, "control socket"),
        n("askuser", Endpoint, "ask_user"),
        n("reportstatus", Endpoint, "report_status"),
        n("statedb", Data, "state.db"),
        n("fts", Data, "FTS5 index"),
        n("recipe", Data, "project recipe"),
        n("claude", External, "Claude Code"),
        n("git", External, "git worktree"),
        n("systemd", External, "systemd-run"),
        n("applyfn", Logic, "apply(delta)<br/>for d in deltas:<br/>  snapshot.merge(d)"),
        n("routefn", Logic, "route(permission)<br/>inbox.push(req)"),
        n("coalesce", Logic, "coalesce(frames)<br/>redraw ≤ 30 fps"),
        n("ready", Outcome, "session ready"),
        n("oom", Outcome, "OOM killed"),
        n("snapshot", Concept, "Snapshot"),
        n("inbox", Concept, "one inbox"),
        n("dirty", Concept, "dirty flag"),
        n("hook", Endpoint, "WorktreeCreate hook"),
    ];
    let groups = vec![
        Group { id: "g_ui", title: "Terminal UI", parent: None, members: &["tui", "apprs", "coalesce", "dirty"] },
        Group { id: "g_input", title: "Input", parent: Some("g_ui"), members: &["keymaprs", "inbox"] },
        Group { id: "g_core", title: "Core", parent: None, members: &["core", "applyfn", "snapshot", "ready", "oom"] },
        Group { id: "g_store", title: "Storage", parent: Some("g_core"), members: &["dbrs", "statedb", "fts"] },
        Group { id: "g_mcp", title: "MCP bridge", parent: Some("g_core"), members: &["mcprs", "mcpcmd", "askuser", "reportstatus"] },
        Group { id: "g_drv", title: "Driver", parent: None, members: &["driver", "routefn", "hook"] },
    ];
    let edges = vec![
        ("user", "tui", "types"),
        ("tui", "apprs", ""),
        ("apprs", "keymaprs", "resolves keys"),
        ("keymaprs", "inbox", ""),
        ("apprs", "coalesce", ""),
        ("coalesce", "dirty", "checks"),
        ("tui", "socket", "commands"),
        ("socket", "core", ""),
        ("core", "applyfn", ""),
        ("applyfn", "snapshot", "updates"),
        ("snapshot", "tui", "deltas"),
        ("core", "dbrs", ""),
        ("dbrs", "statedb", "writes"),
        ("dbrs", "fts", "indexes"),
        ("core", "driver", "spawns"),
        ("driver", "claude", "NDJSON"),
        ("driver", "systemd", "caps"),
        ("driver", "routefn", ""),
        ("routefn", "inbox", "asks"),
        ("driver", "hook", ""),
        ("hook", "git", "creates"),
        ("hook", "recipe", "reads"),
        ("claude", "mcpcmd", "calls tools"),
        ("mcpcmd", "mcprs", ""),
        ("mcprs", "askuser", ""),
        ("mcprs", "reportstatus", ""),
        ("askuser", "inbox", ""),
        ("coordagent", "claude", "runs in"),
        ("core", "ready", ""),
        ("systemd", "oom", "on cap"),
        ("mainrs", "core", "starts"),
        ("mainrs", "tui", "starts"),
        ("api", "core", ""),
        ("api", "tui", ""),
    ];
    Map { nodes, groups, edges }
}

// ---------------------------------------------------------------- Mermaid conversion

/// How node classes are chosen: the real look, or one unique fill per node for the
/// position check.
enum Paint<'a> {
    Kinds { selected: Option<&'a str> },
    Probe,
}

fn probe_colour(i: usize) -> [u8; 3] {
    // Distinct, saturated, and far from the theme's greys and from each other.
    [(40 + i * 7) as u8, (250 - i * 5) as u8, (60 + (i * 97) % 180) as u8]
}

fn to_mermaid(map: &Map, paint: &Paint) -> String {
    let mut s = String::new();
    let _ = write!(s, "---\nconfig:\n  layout: elk\n  theme: dark\n  look: classic\n---\nflowchart {}\n", direction());
    let grouped: HashMap<&str, &str> = map
        .groups
        .iter()
        .flat_map(|g| g.members.iter().map(move |m| (*m, g.id)))
        .collect();
    let node_line = |n: &Node| n.kind.shape(n.id, n.label);

    fn emit_group(s: &mut String, map: &Map, g: &Group, depth: usize, node_line: &dyn Fn(&Node) -> String) {
        let pad = "  ".repeat(depth);
        let _ = writeln!(s, "{pad}subgraph {}[\"{}\"]", g.id, g.title);
        for m in g.members {
            let n = map.nodes.iter().find(|n| n.id == *m).expect("member exists");
            let _ = writeln!(s, "{pad}  {}", node_line(n));
        }
        for child in map.groups.iter().filter(|c| c.parent == Some(g.id)) {
            emit_group(s, map, child, depth + 1, node_line);
        }
        let _ = writeln!(s, "{pad}end");
    }
    for g in map.groups.iter().filter(|g| g.parent.is_none()) {
        emit_group(&mut s, map, g, 1, &node_line);
    }
    for n in map.nodes.iter().filter(|n| !grouped.contains_key(n.id)) {
        let _ = writeln!(s, "  {}", node_line(n));
    }
    for (a, b, label) in &map.edges {
        if label.is_empty() {
            let _ = writeln!(s, "  {a} --> {b}");
        } else {
            let _ = writeln!(s, "  {a} -->|{label}| {b}");
        }
    }
    match paint {
        Paint::Kinds { selected } => {
            for k in Kind::ALL {
                let _ = writeln!(s, "  classDef {} {}", k.class(), k.class_def());
            }
            // Variant for failing outcomes: same kind, red.
            const FAIL: &str = "fill:#e06666,stroke:#7a1414,stroke-width:5px,color:#2a0606";
            let _ = writeln!(s, "  classDef outcomeFail {FAIL}");
            // The selected node gets ONE combined class (its kind's style with a yellow ring)
            // instead of its kind class. merman 0.9.3's PNG path uses only a node's first
            // class: a second `class x sel` or a `style x …` line shows in the SVG but not in
            // the PNG (see src/bin/highlight.rs).
            let base_class = |id: &str| -> (String, String) {
                let n = map.nodes.iter().find(|n| n.id == id).expect("node");
                if id == "oom" { ("outcomeFail".into(), FAIL.into()) } else { (n.kind.class().into(), n.kind.class_def().into()) }
            };
            if let Some(id) = selected {
                let (class, def) = base_class(id);
                let kept: Vec<&str> = def.split(',').filter(|p| !p.starts_with("stroke:") && !p.starts_with("stroke-width:")).collect();
                let _ = writeln!(s, "  classDef {class}Sel {},stroke:#ffd84a,stroke-width:6px", kept.join(","));
            }
            let mut assigned: Vec<(String, Vec<&str>)> = Vec::new();
            for n in &map.nodes {
                let (class, _) = base_class(n.id);
                let class = if Some(n.id) == *selected { format!("{class}Sel") } else { class };
                match assigned.iter_mut().find(|(c, _)| *c == class) {
                    Some((_, ids)) => ids.push(n.id),
                    None => assigned.push((class, vec![n.id])),
                }
            }
            for (class, ids) in assigned {
                let _ = writeln!(s, "  class {} {class}", ids.join(","));
            }
        }
        Paint::Probe => {
            for (i, n) in map.nodes.iter().enumerate() {
                let [r, g, b] = probe_colour(i);
                // Same stroke width as the real kinds would be irrelevant; the check is
                // on centres, and layout does not depend on class styles (checked below).
                let _ = writeln!(s, "  classDef p{i} fill:#{r:02x}{g:02x}{b:02x},stroke:#{r:02x}{g:02x}{b:02x},color:#{r:02x}{g:02x}{b:02x}");
                let _ = writeln!(s, "  class {} p{i}", n.id);
            }
        }
    }
    s
}

/// Flow direction: `DIR=LR` or `DIR=TB` (default TB, which suits a terminal pane better).
fn direction() -> String {
    std::env::var("DIR").unwrap_or_else(|_| "TB".into())
}

// ---------------------------------------------------------------- rendering

const SCALE: f32 = 2.0;

fn svg_request() -> SvgRequest {
    SvgRequest::default()
}

fn render_png(r: &Renderer, src: &str) -> (Vec<u8>, merman::svg::export::RasterPlan) {
    render_png_with(r, src, RasterOptions::default().with_scale(SCALE))
}

fn render_png_with(r: &Renderer, src: &str, options: RasterOptions) -> (Vec<u8>, merman::svg::export::RasterPlan) {
    let out = r
        .render(RenderRequest::png(src, OperationControl::new(), PngRequest { svg: svg_request(), options }))
        .expect("png render");
    let RenderOutput::Png(Some(o)) = out else { panic!("no diagram") };
    (o.bytes, o.plan)
}

fn render_svg(r: &Renderer, src: &str) -> String {
    let out = r.render(RenderRequest::svg(src, OperationControl::new(), svg_request())).expect("svg render");
    let RenderOutput::Svg(Some(o)) = out else { panic!("no diagram") };
    o.svg().to_string()
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
}

/// Node boxes (centre + size) from merman's layout JSON, in SVG user units.
fn layout_nodes(r: &Renderer, src: &str) -> (HashMap<String, Rect>, String) {
    let out = r.render(RenderRequest::layout_json(src, OperationControl::new(), svg_request())).expect("layout");
    let RenderOutput::LayoutJson(Some(l)) = out else { panic!("no diagram") };
    let mut buf = Vec::new();
    l.layout().write_json_pretty(&mut buf).expect("json");
    let text = String::from_utf8(buf).expect("utf8");
    // Tiny hand parser for the "nodes" array: avoids a serde_json dependency in the spike
    // being mistaken for something merman needs. Each node object is flat.
    let mut map = HashMap::new();
    // The semantic section has its own "nodes"; the geometry lives under "FlowchartV2".
    let base = text.find("\"FlowchartV2\"").expect("layout key");
    let nodes_start = base + text[base..].find("\"nodes\"").expect("nodes key");
    let edges_start = base + text[base..].find("\"edges\"").expect("edges key");
    for obj in text[nodes_start..edges_start].split('{').skip(1) {
        let field = |k: &str| -> Option<&str> {
            let i = obj.find(&format!("\"{k}\":"))? + k.len() + 3;
            let rest = obj[i..].trim_start();
            let end = rest.find([',', '\n', '}']).unwrap_or(rest.len());
            Some(rest[..end].trim().trim_matches('"'))
        };
        if field("is_cluster") == Some("true") {
            continue;
        }
        let num = |k| field(k).expect("field").parse::<f64>().expect("number");
        map.insert(
            field("id").expect("id").to_string(),
            Rect { cx: num("x"), cy: num("y"), w: num("width"), h: num("height") },
        );
    }
    (map, text)
}

/// Node centres from the SVG: `<g class="node …" id="<diagram>-flowchart-<id>-<n>" transform="translate(x,y)">`.
fn svg_nodes(svg: &str) -> HashMap<String, (f64, f64)> {
    let mut out = HashMap::new();
    for chunk in svg.split("<g class=\"node ").skip(1) {
        let tag = &chunk[..chunk.find('>').expect("tag end")];
        let id_attr = between(tag, "id=\"", "\"").expect("id");
        let after = &id_attr[id_attr.find("-flowchart-").expect("flowchart id") + "-flowchart-".len()..];
        let id = &after[..after.rfind('-').expect("suffix")];
        let tr = between(tag, "translate(", ")").expect("translate");
        let (x, y) = tr.split_once(',').expect("x,y");
        out.insert(id.to_string(), (x.trim().parse().expect("x"), y.trim().parse().expect("y")));
    }
    out
}

fn view_box(svg: &str) -> [f64; 4] {
    let vb = between(svg, "viewBox=\"", "\"").expect("viewBox");
    let v: Vec<f64> = vb.split_whitespace().map(|t| t.parse().expect("vb")).collect();
    [v[0], v[1], v[2], v[3]]
}

fn between<'a>(s: &'a str, a: &str, b: &str) -> Option<&'a str> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)? + i;
    Some(&s[i..j])
}

// ---------------------------------------------------------------- measurement helpers

fn proc_status_kb(key: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").expect("status");
    let line = s.lines().find(|l| l.starts_with(key)).expect("key");
    line.split_whitespace().nth(1).expect("value").parse().expect("kb")
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("cmp"));
    (v[0], v[v.len() / 2], v[v.len() - 1])
}

fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().expect("png info");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("frame");
    assert_eq!(info.color_type, png::ColorType::Rgba, "expected RGBA");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn count_near(px: &[u8], w: u32, (x0, y0, cw, ch): (u32, u32, u32, u32), c: [u8; 3], tol: i32) -> usize {
    let mut n = 0;
    for y in y0..y0 + ch {
        for x in x0..x0 + cw {
            let p = ((y * w + x) * 4) as usize;
            if p + 3 < px.len() && (0..3).all(|k| (px[p + k] as i32 - c[k] as i32).abs() <= tol) && px[p + 3] > 200 {
                n += 1;
            }
        }
    }
    n
}

fn crop(px: &[u8], w: u32, h: u32, (x0, y0, cw, ch): (u32, u32, u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..(y0 + ch).min(h) {
        let a = ((y * w + x0) * 4) as usize;
        let b = ((y * w + (x0 + cw).min(w)) * 4) as usize;
        out.extend_from_slice(&px[a..b]);
    }
    out
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.write_header().expect("header").write_image_data(rgba).expect("data");
    out
}

fn write_png(path: &str, rgba: &[u8], w: u32, h: u32) {
    std::fs::write(path, encode_png(rgba, w, h)).expect("write png");
}

// ---------------------------------------------------------------- main

/// `merman-spike rss <WxH|scale2>`: memory only. One cold render plus a re-render per node
/// at that output size, nothing else in the process (no PNG decoding), then print the peak.
fn rss_mode(size: &str) {
    let rss_start = proc_status_kb("VmRSS:");
    let opts = match size.split_once('x') {
        Some((w, h)) => RasterOptions::default().with_fit_to(RasterFitBox::contain(w.parse().expect("w"), h.parse().expect("h"))),
        None => RasterOptions::default().with_scale(SCALE),
    };
    let map = sample_map();
    let renderer = Renderer::new();
    let (_, plan) = render_png_with(&renderer, &to_mermaid(&map, &Paint::Kinds { selected: None }), opts.clone());
    let after_first = proc_status_kb("VmHWM:");
    for n in &map.nodes {
        render_png_with(&renderer, &to_mermaid(&map, &Paint::Kinds { selected: Some(n.id) }), opts.clone());
    }
    let hwm = proc_status_kb("VmHWM:");
    let rss_end = proc_status_kb("VmRSS:");
    println!(
        "rss {size:>9} -> {}x{} px: start {:.1} MiB; peak added after first render {:.1} MiB, after 31 renders {:.1} MiB; resident at end {:.1} MiB",
        plan.width_px,
        plan.height_px,
        rss_start as f64 / 1024.0,
        (after_first - rss_start) as f64 / 1024.0,
        (hwm - rss_start) as f64 / 1024.0,
        rss_end as f64 / 1024.0
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("rss") {
        rss_mode(args.get(2).map(String::as_str).unwrap_or("scale2"));
        return;
    }
    std::fs::create_dir_all("out").expect("out dir");
    let rss_start = proc_status_kb("VmRSS:");
    let map = sample_map();
    assert_eq!(map.nodes.len(), 30);

    let renderer = Renderer::new();
    let src = to_mermaid(&map, &Paint::Kinds { selected: None });
    std::fs::write("out/map.mmd", &src).expect("write mmd");

    // 1. Full render: cold (first ever call in the process) and warm.
    let t = Instant::now();
    let (png_bytes, plan) = render_png(&renderer, &src);
    let cold = t.elapsed();
    std::fs::write("out/map.png", &png_bytes).expect("write png");
    let mut warm = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        render_png(&renderer, &src);
        warm.push(ms(t.elapsed()));
    }
    let (wmin, wmed, wmax) = stats(warm);
    let mut svg_only = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        render_svg(&renderer, &src);
        svg_only.push(ms(t.elapsed()));
    }
    let (_, smed, _) = stats(svg_only);
    println!("png: {}x{} px (scale {}), {} bytes", plan.width_px, plan.height_px, plan.effective_scale, png_bytes.len());
    println!("full render (parse+layout+svg+png): cold {:.1} ms; warm min/median/max {wmin:.1}/{wmed:.1}/{wmax:.1} ms (n=10)", ms(cold));
    println!("  of which svg only (parse+layout+svg): median {smed:.1} ms; raster ≈ {:.1} ms", wmed - smed);

    // 2. Re-render with the highlight class moved: a new source each time.
    let mut sel = Vec::new();
    for (i, n) in map.nodes.iter().enumerate() {
        let s = to_mermaid(&map, &Paint::Kinds { selected: Some(n.id) });
        let t = Instant::now();
        let (bytes, _) = render_png(&renderer, &s);
        sel.push(ms(t.elapsed()));
        if i == 0 || n.id == "applyfn" {
            std::fs::write(format!("out/map-selected-{}.png", n.id), bytes).expect("write sel");
            std::fs::write(format!("out/map-selected-{}.svg", n.id), render_svg(&renderer, &s)).expect("write sel svg");
            std::fs::write(format!("out/map-selected-{}.mmd", n.id), &s).expect("write sel mmd");
        }
    }
    let (smin, smed2, smax) = stats(sel);
    println!("re-render with selection moved: min/median/max {smin:.1}/{smed2:.1}/{smax:.1} ms (n=30, every node once)");

    // 3. Positions: layout JSON vs SVG, then vs the probe PNG's pixels.
    let (layout, layout_text) = layout_nodes(&renderer, &src);
    std::fs::write("out/layout.json", &layout_text).expect("write layout");
    let svg = render_svg(&renderer, &src);
    std::fs::write("out/map.svg", &svg).expect("write svg");
    let from_svg = svg_nodes(&svg);
    assert_eq!(layout.len(), 30, "layout has 30 non-cluster nodes");
    assert_eq!(from_svg.len(), 30, "svg has 30 node groups");
    let mut svg_vs_layout = 0.0f64;
    for (id, r) in &layout {
        let (x, y) = from_svg[id];
        svg_vs_layout = svg_vs_layout.max((x - r.cx).abs()).max((y - r.cy).abs());
    }
    println!("layout JSON vs SVG translate(): max |Δ| = {svg_vs_layout:.3} user units over 30 nodes");

    // Probe render: same graph, one unique flat colour per node. Layout must not change.
    let probe_src = to_mermaid(&map, &Paint::Probe);
    let (probe_layout, _) = layout_nodes(&renderer, &probe_src);
    let mut layout_drift = 0.0f64;
    for (id, r) in &layout {
        let p = probe_layout[id];
        layout_drift = layout_drift.max((p.cx - r.cx).abs()).max((p.cy - r.cy).abs()).max((p.w - r.w).abs()).max((p.h - r.h).abs());
    }
    println!("layout drift between real and probe styling: {layout_drift:.3} user units");
    let probe_svg = render_svg(&renderer, &probe_src);
    let [vx, vy, vw, vh] = view_box(&probe_svg);
    assert_eq!(view_box(&svg), [vx, vy, vw, vh], "same viewBox");
    let (probe_png, probe_plan) = render_png(&renderer, &probe_src);
    std::fs::write("out/probe.png", &probe_png).expect("write probe");
    let (pw, ph, px) = decode_png(&probe_png);
    let sx = pw as f64 / vw;
    let sy = ph as f64 / vh;
    println!("viewBox {vx} {vy} {vw} {vh}; png {pw}x{ph}; px per unit {sx:.4} x {sy:.4} (plan scale {})", probe_plan.effective_scale);

    let mut worst = 0.0f64;
    let mut report = String::new();
    for (i, n) in map.nodes.iter().enumerate() {
        let c = probe_colour(i);
        let (mut x0, mut y0, mut x1, mut y1, mut count) = (u32::MAX, u32::MAX, 0u32, 0u32, 0u64);
        for y in 0..ph {
            for x in 0..pw {
                let p = ((y * pw + x) * 4) as usize;
                if px[p..p + 3] == c && px[p + 3] == 255 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                    count += 1;
                }
            }
        }
        assert!(count > 0, "node {} not found in probe PNG", n.id);
        // Pixel centre of the painted box, in PNG pixels.
        let pcx = (x0 as f64 + x1 as f64 + 1.0) / 2.0;
        let pcy = (y0 as f64 + y1 as f64 + 1.0) / 2.0;
        let r = layout[n.id];
        // Predicted centre from the layout JSON, mapped through the viewBox.
        let ecx = (r.cx - vx) * sx;
        let ecy = (r.cy - vy) * sy;
        let dx = pcx - ecx;
        let dy = pcy - ecy;
        let dw = (x1 - x0 + 1) as f64 - r.w * sx;
        let dh = (y1 - y0 + 1) as f64 - r.h * sy;
        let d = dx.abs().max(dy.abs());
        worst = worst.max(d);
        let _ = writeln!(
            report,
            "  {:<13} {:<9} centre png ({pcx:7.1},{pcy:7.1}) predicted ({ecx:7.1},{ecy:7.1}) Δ ({dx:+5.1},{dy:+5.1}) px; size Δ ({dw:+5.1},{dh:+5.1}) px; {count} px",
            n.id,
            format!("{:?}", n.kind)
        );
    }
    print!("{report}");
    std::fs::write("out/positions.txt", &report).expect("write positions");
    println!("positions: worst centre error {worst:.2} PNG px at scale {SCALE} ({:.2} SVG units) -> {}", worst / sx, if worst <= 5.0 { "PASS" } else { "FAIL" });

    // Legend: one node per kind in a row, for judging the nine styles side by side.
    let legend = {
        let mut s = String::from("---\nconfig:\n  layout: elk\n  theme: dark\n  look: classic\n---\nflowchart LR\n");
        for k in Kind::ALL {
            let label = match k {
                Kind::Logic => "Logic<br/>if x: y",
                _ => &format!("{k:?}"),
            };
            let _ = writeln!(s, "  {}", k.shape(k.class(), label));
        }
        let _ = writeln!(s, "  classDef outcomeFail fill:#e06666,stroke:#7a1414,stroke-width:5px,color:#2a0606");
        let _ = writeln!(s, "  failed[\"Outcome (fail)\"]");
        let _ = writeln!(s, "  class failed outcomeFail");
        for k in Kind::ALL {
            let _ = writeln!(s, "  classDef {}Def {}", k.class(), k.class_def());
            let _ = writeln!(s, "  class {} {}Def", k.class(), k.class());
        }
        s
    };
    std::fs::write("out/legend.png", render_png(&renderer, &legend).0).expect("write legend");

    // 4. Did the highlight reach the pixels? Count highlight-yellow pixels around `applyfn`
    //    in the plain and the selected render, and save a crop of each for the eye.
    let (bw, bh, base) = decode_png(&png_bytes);
    let sel_png = std::fs::read("out/map-selected-applyfn.png").expect("read sel");
    let (_, _, selpx) = decode_png(&sel_png);
    let r = layout["applyfn"];
    let margin = 12.0;
    let crop_rect = (
        (((r.cx - r.w / 2.0 - vx) * sx) - margin).max(0.0) as u32,
        (((r.cy - r.h / 2.0 - vy) * sy) - margin).max(0.0) as u32,
        ((r.w * sx) + 2.0 * margin) as u32,
        ((r.h * sy) + 2.0 * margin) as u32,
    );
    let yellow = |px: &[u8], w: u32| count_near(px, w, crop_rect, [0xff, 0xd8, 0x4a], 40);
    println!("highlight check around applyfn: yellow px plain {} vs selected {}", yellow(&base, bw), yellow(&selpx, bw));
    write_png("out/crop-plain-applyfn.png", &crop(&base, bw, bh, crop_rect), crop_rect.2, crop_rect.3);
    write_png("out/crop-selected-applyfn.png", &crop(&selpx, bw, bh, crop_rect), crop_rect.2, crop_rect.3);

    // 5. Overlay alternative: draw the selection ring as its own small RGBA image, to be
    //    placed over the cached base image (kitty supports several placements with z-index).
    let mut ov = Vec::new();
    let mut ov_bytes = Vec::new();
    for _ in 0..30 {
        let t = Instant::now();
        let (w, h) = (crop_rect.2, crop_rect.3);
        let mut img = vec![0u8; (w * h * 4) as usize];
        let ring = 6;
        for y in 0..h {
            for x in 0..w {
                let edge = x < ring || y < ring || x >= w - ring || y >= h - ring;
                if edge {
                    let p = ((y * w + x) * 4) as usize;
                    img[p..p + 4].copy_from_slice(&[0xff, 0xd8, 0x4a, 0xff]);
                }
            }
        }
        ov_bytes = encode_png(&img, w, h);
        ov.push(ms(t.elapsed()));
    }
    let (omin, omed, omax) = stats(ov);
    std::fs::write("out/overlay-applyfn.png", &ov_bytes).expect("write overlay");
    println!("overlay ring image ({}x{} px, built + PNG-encoded): min/median/max {omin:.3}/{omed:.3}/{omax:.3} ms", crop_rect.2, crop_rect.3);

    // 6. Size sweep: raster cost follows output pixels. A canvas renders for its pane size.
    for (name, opts) in [
        ("fit 1280x800 (small pane)", RasterOptions::default().with_fit_to(RasterFitBox::contain(1280, 800))),
        ("fit 1920x1080 (full screen 1080p)", RasterOptions::default().with_fit_to(RasterFitBox::contain(1920, 1080))),
        ("fit 2560x1440 (full screen 1440p)", RasterOptions::default().with_fit_to(RasterFitBox::contain(2560, 1440))),
        ("scale 1.0 (1:1 user units)", RasterOptions::default().with_scale(1.0)),
        ("scale 2.0 (capped by size limit)", RasterOptions::default().with_scale(2.0)),
    ] {
        let mut v = Vec::new();
        let mut plan = None;
        for i in 0..6 {
            let s = to_mermaid(&map, &Paint::Kinds { selected: Some(map.nodes[i].id) });
            let t = Instant::now();
            let (_, p) = render_png_with(&renderer, &s, opts.clone());
            v.push(ms(t.elapsed()));
            plan = Some(p);
        }
        let p = plan.expect("plan");
        let (a, m, b) = stats(v);
        println!("  {name:<36} {:>4}x{:<4} ({:4.1} MP): render with new selection min/median/max {a:6.1}/{m:6.1}/{b:6.1} ms", p.width_px, p.height_px, (p.width_px as f64 * p.height_px as f64) / 1e6);
    }

    let rss_end = proc_status_kb("VmRSS:");
    let hwm = proc_status_kb("VmHWM:");
    println!("rss (whole run, includes the spike decoding big PNGs; use `rss` mode for the measure): start {} KiB, end {} KiB, peak {} KiB, peak added {:.1} MiB", rss_start, rss_end, hwm, (hwm - rss_start) as f64 / 1024.0);
}
