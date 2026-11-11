//! Structure map → Mermaid flowchart source (decision 23d).
//!
//! Each node kind has one shape and one `classDef`. merman 0.9.3's PNG path only honours a node's
//! *first* class (S5, `docs/spikes/merman.md`), so the selected node does not get a second class:
//! it gets one combined `<kind>Sel` class (its kind's style with the selection ring) *instead of*
//! its kind's class.

use std::collections::HashMap;
use std::fmt::Write as _;

use dovetail_api::map::{MapContent, Node, NodeId, NodeKind};

/// The selection ring's stroke, appended to the selected node's kind style.
const RING: &str = "stroke:#ffd84a,stroke-width:6px";

/// Mermaid front matter: ELK layout, dark theme (text and edges suit a dark transparent
/// terminal), top to bottom.
const HEADER: &str =
    "---\nconfig:\n  layout: elk\n  theme: dark\n  look: classic\n---\nflowchart TB\n";

/// A kind's colours (design 23d). One table for both renderers: the Mermaid `classDef`s here
/// and the swimlane SVG in `swimlane_render.rs`.
pub(crate) struct KindColors {
    /// Box fill; `None` draws no box.
    pub fill: Option<&'static str>,
    /// Border colour; `None` draws no border.
    pub stroke: Option<&'static str>,
    /// Border width in pixels.
    pub stroke_width: u32,
    /// Dashed border (External).
    pub dashed: bool,
    /// Label colour.
    pub text: &'static str,
    /// Extra CSS for the label, if any.
    pub extra: Option<&'static str>,
}

/// The colours for a kind.
pub(crate) fn kind_colors(kind: NodeKind) -> KindColors {
    let c = |fill, stroke, stroke_width, text| KindColors {
        fill: Some(fill),
        stroke: Some(stroke),
        stroke_width,
        dashed: false,
        text,
        extra: None,
    };
    match kind {
        NodeKind::Part => c("#c9785d", "#f2c2a8", 3, "#1b1b1b"),
        NodeKind::File => c("#f3efe6", "#9a958c", 1, "#1b1b1b"),
        NodeKind::Endpoint => c("#4cc3d9", "#1d7f91", 2, "#0b2a30"),
        NodeKind::Data => c("#e8a93a", "#9b6a12", 2, "#2a1b00"),
        NodeKind::External => KindColors {
            dashed: true,
            ..c("#5c5f66", "#c3c6cc", 2, "#eceff4")
        },
        NodeKind::Actor => c("#f2a1b6", "#b5546f", 2, "#3a0f1c"),
        NodeKind::Outcome => c("#5fbf6a", "#1f6b2a", 5, "#0d2a11"),
        NodeKind::Logic => KindColors {
            extra: Some("font-family:monospace"),
            ..c("#8f9be8", "#4c58a8", 1, "#101433")
        },
        NodeKind::Concept => KindColors {
            fill: None,
            stroke: None,
            stroke_width: 0,
            dashed: false,
            text: "#c8b6f0",
            extra: Some("font-style:italic"),
        },
    }
}

/// The `classDef` body for a kind.
fn style(kind: NodeKind) -> String {
    let c = kind_colors(kind);
    let mut parts = vec![format!("fill:{}", c.fill.unwrap_or("none"))];
    match c.stroke {
        Some(stroke) => {
            parts.push(format!("stroke:{stroke}"));
            parts.push(format!("stroke-width:{}px", c.stroke_width));
        }
        None => parts.push("stroke:none".into()),
    }
    if c.dashed {
        parts.push("stroke-dasharray:6 4".into());
    }
    parts.push(format!("color:{}", c.text));
    parts.extend(c.extra.map(str::to_owned));
    parts.join(",")
}

/// The kind's style with the selection ring in place of its stroke.
fn selected_style(kind: NodeKind) -> String {
    let full = style(kind);
    let kept: Vec<&str> = full
        .split(',')
        .filter(|p| !p.starts_with("stroke:") && !p.starts_with("stroke-width:"))
        .collect();
    format!("{},{RING}", kept.join(","))
}

fn class_name(kind: NodeKind) -> &'static str {
    kind.as_str()
}

fn selected_class_name(kind: NodeKind) -> String {
    format!("{}Sel", kind.as_str())
}

/// Mermaid id for the node at `index`. Agent ids are safe characters but not safe Mermaid ids
/// (`end`, ids starting with `o` or `x`), so Mermaid gets its own.
pub(crate) fn mermaid_id(index: usize) -> String {
    format!("n{index}")
}

/// Escapes a label for a double-quoted Mermaid string; newlines become `<br/>`.
fn escape(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for c in label.chars() {
        match c {
            '#' => out.push_str("#35;"),
            '"' => out.push_str("#quot;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            '\r' => {}
            '\n' => out.push_str("<br/>"),
            c => out.push(c),
        }
    }
    out
}

/// A node's declaration: id, shape and label.
fn declaration(id: &str, node: &Node) -> String {
    let label = escape(&node.label);
    match node.kind {
        NodeKind::Part => format!("{id}[[\"{label}\"]]"),
        NodeKind::Endpoint => format!("{id}(\"{label}\")"),
        NodeKind::Data => format!("{id}[(\"{label}\")]"),
        NodeKind::Actor => format!("{id}([\"☺ {label}\"])"),
        NodeKind::Logic => format!("{id}[\"ƒ {label}\"]"),
        NodeKind::File | NodeKind::External | NodeKind::Outcome | NodeKind::Concept => {
            format!("{id}[\"{label}\"]")
        }
    }
}

/// Converts a structure map to Mermaid. `selected` is the highlighted node, if any.
///
/// Group parents become subgraphs titled with the parent's label (the subgraph takes the node's
/// Mermaid id, so connections to the parent still land on it). Subgraphs are not styled per kind.
/// Returns the source and, per node, its Mermaid id (parallel to `content.nodes`).
pub(crate) fn to_mermaid(content: &MapContent, selected: Option<&NodeId>) -> (String, Vec<String>) {
    let ids: Vec<String> = (0..content.nodes.len()).map(mermaid_id).collect();
    let index: HashMap<&NodeId, usize> = content
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.id, i))
        .collect();
    let children: HashMap<&NodeId, &[NodeId]> = content
        .groups
        .iter()
        .map(|g| (&g.parent, g.children.as_slice()))
        .collect();
    let nested: std::collections::HashSet<&NodeId> = content
        .groups
        .iter()
        .flat_map(|g| g.children.iter())
        .collect();

    let out = String::from(HEADER);
    // Kind -> mermaid ids that take the kind's class (leaf nodes; the selected one is left out).
    let mut by_kind: Vec<(NodeKind, Vec<&str>)> = Vec::new();
    let mut selected_kind = None;

    /// Writes nodes and subgraphs, collecting the leaf nodes (the ones that take a class).
    struct Emitter<'a> {
        content: &'a MapContent,
        index: &'a HashMap<&'a NodeId, usize>,
        ids: &'a [String],
        children: &'a HashMap<&'a NodeId, &'a [NodeId]>,
        out: String,
        leaves: Vec<usize>,
    }

    impl Emitter<'_> {
        fn emit(&mut self, depth: usize, node_id: &NodeId) {
            let Some(&i) = self.index.get(node_id) else {
                return;
            };
            let node = &self.content.nodes[i];
            let pad = "  ".repeat(depth);
            match self.children.get(node_id) {
                Some(kids) => {
                    let _ = writeln!(
                        self.out,
                        "{pad}subgraph {}[\"{}\"]",
                        self.ids[i],
                        escape(&node.label)
                    );
                    for kid in *kids {
                        self.emit(depth + 1, kid);
                    }
                    let _ = writeln!(self.out, "{pad}end");
                }
                None => {
                    let _ = writeln!(self.out, "{pad}{}", declaration(&self.ids[i], node));
                    self.leaves.push(i);
                }
            }
        }
    }

    let mut emitter = Emitter {
        content,
        index: &index,
        ids: &ids,
        children: &children,
        out,
        leaves: Vec::new(),
    };
    for node in content.nodes.iter().filter(|n| !nested.contains(&n.id)) {
        emitter.emit(1, &node.id);
    }
    let Emitter {
        out: emitted,
        leaves,
        ..
    } = emitter;
    let mut out = emitted;
    for i in leaves {
        let node = &content.nodes[i];
        if Some(&node.id) == selected {
            selected_kind = Some((node.kind, ids[i].as_str()));
            continue;
        }
        match by_kind.iter_mut().find(|(k, _)| *k == node.kind) {
            Some((_, v)) => v.push(&ids[i]),
            None => by_kind.push((node.kind, vec![&ids[i]])),
        }
    }

    for conn in &content.connections {
        let (Some(&a), Some(&b)) = (index.get(&conn.from), index.get(&conn.to)) else {
            continue;
        };
        if conn.label.is_empty() {
            let _ = writeln!(out, "  {} --> {}", ids[a], ids[b]);
        } else {
            let _ = writeln!(
                out,
                "  {} -->|\"{}\"| {}",
                ids[a],
                escape(&conn.label),
                ids[b]
            );
        }
    }

    for (kind, members) in &by_kind {
        let _ = writeln!(out, "  classDef {} {}", class_name(*kind), style(*kind));
        let _ = writeln!(out, "  class {} {}", members.join(","), class_name(*kind));
    }
    if let Some((kind, id)) = selected_kind {
        let _ = writeln!(
            out,
            "  classDef {} {}",
            selected_class_name(kind),
            selected_style(kind)
        );
        let _ = writeln!(out, "  class {id} {}", selected_class_name(kind));
    }
    (out, ids)
}
