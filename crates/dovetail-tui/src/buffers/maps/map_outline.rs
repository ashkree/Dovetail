//! The map view: the canvas (kitty graphics, [`super::map_canvas`]) with the outline beside it,
//! or the outline alone in terminals without kitty graphics.
//!
//! A structure map is a tree: groups nest their children, and `o` folds a group. A swimlane map
//! is a list of its nodes in the order the flow walks them, each with its lane. Below the list
//! sit the selected node in full (description, file, pseudocode, connections) and the current
//! flow's steps; `f` switches flow and `]]`/`[[` walk the flow's steps. The shelf
//! ([`super::map_shelf::MapShelfBuffer`]) opens maps here, in MAP mode (decision 23f): the map takes the
//! whole screen, `hjkl` move between nodes on the canvas, the mouse works, and `Esc` leaves.

use std::collections::HashSet;
use std::sync::Arc;

use dovetail_api::{Command, Event};
use dovetail_api::{
    app::AppEvent,
    visual::map::{
        Flow, Map, MapCommand, MapContent, MapEvent, MapId, MapKind, Node, NodeId, NodeKind,
    },
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::{Buffer, Effect, ListCursor, ViewCx, footer, pane, title};
use super::map_canvas::Canvas;
use crate::canvas::kitty::Scene;
use crate::canvas::view::CellPx;
use crate::canvas::{Direction, Painted};
use crate::fmt;
use crate::keymap::action::Action;
use crate::layout::region::BufferKind;
use crate::render::reading_column;
use crate::theme::Theme;

/// What the app does after a map event reached the outline.
#[derive(Debug, Clone, PartialEq)]
pub enum Reaction {
    /// The map the user asked for has arrived: show the outline.
    Show,
    /// The open map has a newer version: ask for it.
    Request(Command),
}

/// The outline buffer: the open map and the cursor in it.
#[derive(Default)]
pub struct MapOutlineBuffer {
    map: Option<Arc<Map>>,
    /// A map asked for with [`MapCommand::Load`] that has not arrived.
    loading: Option<MapId>,
    /// The selected node and viewport scroll, kept by id so a redrawn version never moves the cursor.
    cursor: ListCursor<NodeId>,
    /// Groups folded shut.
    folded: HashSet<NodeId>,
    /// Index of the flow shown below the list.
    flow: usize,
    /// The flow step the selection was reached by (`n`/`p`, `]]`/`[[`), so a swimlane node on
    /// several steps rings the right box.
    flow_step: Option<usize>,
    /// The picture and the view over it.
    canvas: Canvas,
    /// `v` in MAP mode hid the outline beside the canvas.
    outline_hidden: bool,
    /// MAP mode is on: the hints say so.
    map_mode: bool,
}

/// One visible row of the list.
struct Row<'a> {
    node: &'a Node,
    depth: usize,
    /// Nodes inside it (structure maps).
    children: usize,
    folded: bool,
    /// 1-based position in the flow (swimlane maps).
    number: Option<usize>,
    /// The lane of the node's first step (swimlane maps).
    lane: Option<&'a str>,
}

impl MapOutlineBuffer {
    /// A map was asked for; the next [`MapEvent::Loaded`] for it shows the outline.
    pub fn await_map(&mut self, id: MapId) {
        self.loading = Some(id);
    }

    /// The open map's id.
    pub fn map_id(&self) -> Option<&MapId> {
        self.map.as_ref().map(|m| &m.id)
    }

    /// The selected node.
    pub fn selected_node(&self) -> Option<&NodeId> {
        self.cursor.key()
    }

    /// The open map.
    pub fn map(&self) -> Option<&Arc<Map>> {
        self.map.as_ref()
    }

    /// Graphics on, with the cell size, or off (outline only).
    pub fn set_graphics(&mut self, cell: Option<CellPx>) {
        self.canvas.set_graphics(cell);
    }

    /// Whether MAP mode is on, for the hints.
    pub fn set_map_mode(&mut self, on: bool) {
        self.map_mode = on;
    }

    /// A painting for the event loop to start.
    pub fn take_paint_job(&mut self) -> Option<crate::canvas::PaintJob> {
        self.canvas.take_job()
    }

    /// A painting came back; returns whether the view changed.
    pub fn on_painted(&mut self, painted: Painted) -> bool {
        self.canvas.receive(painted)
    }

    /// What kitty should show after the last frame, or `None` (nothing to show, or the canvas
    /// was not drawn).
    pub fn scene(&self, ring: [u8; 3]) -> Option<Scene> {
        let selected = self.cursor.key().map(|id| (id, self.flow_step));
        self.canvas.scene(selected, ring)
    }

    /// The canvas, for tests and the mouse.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// A mouse event at a screen cell, in MAP mode. Returns whether anything changed.
    pub fn on_mouse(&mut self, kind: MouseKind, col: u16, row: u16) -> bool {
        let on_canvas = self.canvas.contains(col, row);
        match kind {
            MouseKind::Press if on_canvas => {
                if let Some(hit) = self.canvas.node_at(col, row).cloned()
                    && let Some(map) = self.map.clone()
                {
                    self.goto_node(&map.content, &hit.id);
                    self.flow_step = hit.step;
                }
                self.canvas.drag_to(col, row);
                true
            }
            MouseKind::Drag => {
                self.canvas.drag_to(col, row);
                true
            }
            MouseKind::Release => {
                self.canvas.end_drag();
                false
            }
            MouseKind::ScrollUp | MouseKind::ScrollDown if on_canvas => {
                self.canvas
                    .zoom(kind == MouseKind::ScrollUp, Some((col, row)));
                true
            }
            _ => false,
        }
    }

    /// Takes in a map event. A newer version of the open map replaces it in place, keeping the
    /// cursor on the same node.
    pub fn on_event(&mut self, event: &Event) -> Option<Reaction> {
        match event {
            Event::Map(MapEvent::Loaded(map)) => {
                let wanted = self.loading.as_ref() == Some(&map.id);
                if wanted {
                    self.loading = None;
                }
                if wanted || self.map_id() == Some(&map.id) {
                    self.set_map(map.clone());
                }
                wanted.then_some(Reaction::Show)
            }
            Event::Map(MapEvent::Upserted(summary)) => {
                let open = self.map.as_ref()?;
                (open.id == summary.id && summary.version > open.version).then(|| {
                    Reaction::Request(Command::Map(MapCommand::Load {
                        id: summary.id.clone(),
                        version: None,
                    }))
                })
            }
            Event::App(AppEvent::CommandFailed { .. }) => {
                self.loading = None;
                None
            }
            _ => None,
        }
    }

    fn set_map(&mut self, map: Map) {
        if self.map_id() != Some(&map.id) {
            self.canvas.reset();
            self.flow_step = None;
            self.cursor.reset();
            self.folded.clear();
            self.flow = 0;
        }
        self.flow = self.flow.min(map.content.flows.len().saturating_sub(1));
        self.map = Some(Arc::new(map));
    }

    fn resolve(&self, rows: &[Row]) -> Option<usize> {
        self.cursor.resolve(rows, |r, k| &r.node.id == k)
    }

    fn select(&mut self, rows: &[Row], index: usize) {
        self.cursor.select_index(rows, index, |r| r.node.id.clone());
    }

    /// Selects a node by id, opening the groups that hide it.
    fn goto_node(&mut self, content: &MapContent, id: &NodeId) {
        let mut at = id;
        let mut hops = content.groups.len();
        while let Some(group) = content.groups.iter().find(|g| g.children.contains(at)) {
            self.folded.remove(&group.parent);
            at = &group.parent;
            hops -= 1;
            if hops == 0 {
                break;
            }
        }
        let rows = rows(content, &self.folded);
        if let Some(i) = rows.iter().position(|r| &r.node.id == id) {
            self.select(&rows, i);
        }
    }

    /// The node list, the selected node in full and the current flow, in `area`.
    fn draw_outline(
        &mut self,
        cx: &ViewCx,
        frame: &mut Frame,
        area: Rect,
        focused: bool,
        content: &MapContent,
    ) {
        let theme = cx.theme;
        let column = reading_column::column(area, reading_column::DEFAULT_WIDTH);
        let width = usize::from(column.width);
        let rows = rows(content, &self.folded);
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled("This map has no nodes.", theme.dim())),
                Rect {
                    height: 1,
                    ..column
                },
            );
            return;
        }
        let selected = self.resolve(&rows);
        let usable = usize::from(column.height);
        let mut tail = selected
            .map(|i| detail_lines(content, rows[i].node, theme, width))
            .unwrap_or_default();
        if let Some(flow) = content.flows.get(self.flow) {
            let node = selected.map(|i| &rows[i].node.id);
            tail.extend(flow_lines(content, flow, self.flow, node, theme, width));
        }
        // The list keeps at least a few rows; the rest goes to the tail.
        let list_min = rows.len().min(4);
        let tail_height = tail
            .len()
            .min(usable.saturating_sub(list_min))
            .min(usable * 3 / 5);
        let list_height = (usable - tail_height).max(1);
        if let Some(i) = selected {
            self.cursor.scroll_into_view(i, rows.len(), list_height);
        } else {
            self.cursor.clamp_offset(rows.len(), list_height);
        }
        let lines: Vec<Line<'static>> = rows
            .iter()
            .skip(self.cursor.offset())
            .take(list_height)
            .map(|r| row_line(r, theme, width))
            .collect();
        let list_area = Rect {
            height: list_height as u16,
            ..column
        };
        frame.render_widget(Paragraph::new(lines), list_area);
        if focused && let Some(i) = selected {
            let y = list_area.y + (i - self.cursor.offset()) as u16;
            if y < list_area.y + list_area.height {
                frame.buffer_mut().set_style(
                    Rect {
                        y,
                        height: 1,
                        ..area
                    },
                    theme.selected(),
                );
            }
        }
        if tail_height > 0 {
            tail.truncate(tail_height);
            frame.render_widget(
                Paragraph::new(tail),
                Rect {
                    y: list_area.y + list_area.height,
                    height: tail_height as u16,
                    ..column
                },
            );
        }
    }

    /// The selected node's box on the canvas.
    fn selected_box(&self) -> Option<crate::canvas::Placed> {
        let id = self.cursor.key()?;
        self.canvas.picture()?.box_of(id, self.flow_step).cloned()
    }

    /// Keeps the selected node on screen after it moved.
    fn reveal_selected(&mut self) {
        if let Some(b) = self.selected_box() {
            self.canvas.reveal(&b);
        }
    }

    /// MAP mode `hjkl`: the nearest node that way on the canvas. With no picture (outline only)
    /// `j`/`k` move a row and `h`/`l` say why they do nothing.
    fn select_toward(&mut self, content: &MapContent, dir: Direction, effects: &mut Vec<Effect>) {
        if self.canvas.picture().is_none() {
            let rows = rows(content, &self.folded);
            match dir {
                Direction::Down => {
                    self.cursor
                        .step(&rows, 1, |r, k| &r.node.id == k, |r| r.node.id.clone());
                }
                Direction::Up => {
                    self.cursor
                        .step(&rows, -1, |r, k| &r.node.id == k, |r| r.node.id.clone());
                }
                _ => effects.push(Effect::Note(
                    "h and l move on the map picture, which this terminal cannot show".into(),
                )),
            }
            self.flow_step = None;
            return;
        }
        let from = self.selected_box();
        match self.canvas.step(from.as_ref(), dir).cloned() {
            Some(next) => {
                self.goto_node(content, &next.id);
                self.flow_step = next.step;
                self.canvas.reveal(&next);
            }
            None => effects.push(Effect::Note("no node that way".into())),
        }
    }

    /// MAP mode `Tab`/`Shift-Tab`: along the selected node's first connection out (or in).
    fn follow(&mut self, content: &MapContent, forward: bool, effects: &mut Vec<Effect>) {
        let Some(at) = self.cursor.key().cloned() else {
            effects.push(Effect::Note("select a node first (hjkl)".into()));
            return;
        };
        let next = content.connections.iter().find_map(|c| {
            if forward {
                (c.from == at).then(|| c.to.clone())
            } else {
                (c.to == at).then(|| c.from.clone())
            }
        });
        match next {
            Some(id) => {
                self.goto_node(content, &id);
                self.flow_step = None;
                self.reveal_selected();
            }
            None => effects.push(Effect::Note(
                if forward {
                    "no connection out of this node"
                } else {
                    "no connection into this node"
                }
                .into(),
            )),
        }
    }

    /// MAP mode `Enter`: zoom into the selected group. Anything else has nothing inside it to
    /// show; its file, if it has one, goes to the status bar.
    fn drill_in(&mut self, content: &MapContent, effects: &mut Vec<Effect>) {
        let Some(id) = self.cursor.key().cloned() else {
            effects.push(Effect::Note("select a node first (hjkl)".into()));
            return;
        };
        let is_group = content.groups.iter().any(|g| g.parent == id);
        match (is_group, self.selected_box()) {
            (true, Some(b)) => self.canvas.drill_in(&b),
            (true, None) => {
                self.folded.remove(&id);
            }
            (false, _) => {
                let node = content.nodes.iter().find(|n| n.id == id);
                let text = match node.and_then(|n| n.file.as_deref()) {
                    Some(file) => format!("file: {file}"),
                    None => "nothing inside this node to drill into".into(),
                };
                effects.push(Effect::Note(text));
            }
        }
    }

    /// `]]`/`[[`: the node of the next or previous step of the current flow.
    fn walk_flow(&mut self, content: &MapContent, forward: bool, effects: &mut Vec<Effect>) {
        let Some(flow) = content.flows.get(self.flow) else {
            effects.push(Effect::Note("this map has no flow".into()));
            return;
        };
        let rows = rows(content, &self.folded);
        let selected = self.resolve(&rows).map(|i| &rows[i].node.id);
        // The step the selection was reached by, else the node's first step.
        let current = self
            .flow_step
            .filter(|&i| flow.steps.get(i).map(|s| &s.node) == selected)
            .or_else(|| selected.and_then(|id| flow.steps.iter().position(|s| &s.node == id)));
        let target = match (current, forward) {
            (None, true) => flow.steps.first(),
            (None, false) => flow.steps.last(),
            (Some(at), true) => flow.steps[at..]
                .iter()
                .find(|s| s.node != flow.steps[at].node),
            (Some(at), false) => flow.steps[..at]
                .iter()
                .rev()
                .find(|s| s.node != flow.steps[at].node),
        };
        let target = target.map(|t| (flow.steps.iter().position(|s| std::ptr::eq(s, t)), t));
        match target {
            Some((index, step)) => {
                self.goto_node(content, &step.node.clone());
                self.flow_step = index;
                self.reveal_selected();
            }
            None => effects.push(Effect::Note(
                if forward {
                    "last step of the flow"
                } else {
                    "first step of the flow"
                }
                .into(),
            )),
        }
    }
}

/// Splits the body into the canvas and, unless hidden or too narrow, the outline on its right:
/// about 38 % and at least 40 columns, as the side column (Decision 15).
fn split(body: Rect, outline_hidden: bool) -> (Rect, Option<Rect>) {
    const MIN_OUTLINE: u16 = 40;
    const MIN_CANVAS: u16 = 40;
    let outline = (body.width * 38 / 100).max(MIN_OUTLINE);
    if outline_hidden || body.width < outline + 1 + MIN_CANVAS {
        return (body, None);
    }
    let canvas = Rect {
        width: body.width - outline - 1,
        ..body
    };
    let side = Rect {
        x: body.x + canvas.width + 1,
        width: outline,
        ..body
    };
    (canvas, Some(side))
}

/// A mouse event the map view acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    /// Left button down: select the node under it, start a drag.
    Press,
    /// Moved with the left button down: pan.
    Drag,
    /// Left button up.
    Release,
    /// Wheel up: zoom in around the pointer.
    ScrollUp,
    /// Wheel down: zoom out around the pointer.
    ScrollDown,
}

/// The visible rows: groups nest on structure maps, and a folded group hides its children.
/// Swimlane maps list their nodes in the order of the first flow.
fn rows<'a>(content: &'a MapContent, folded: &HashSet<NodeId>) -> Vec<Row<'a>> {
    let mut out = Vec::new();
    match content.kind {
        MapKind::Structure => {
            let inside: HashSet<&NodeId> = content
                .groups
                .iter()
                .flat_map(|g| g.children.iter())
                .collect();
            let mut seen = HashSet::new();
            for node in content.nodes.iter().filter(|n| !inside.contains(&n.id)) {
                push_tree(content, folded, node, 0, true, &mut seen, &mut out);
            }
            // A group cycle would leave nodes unreached; validation forbids it, but never hide.
            for node in &content.nodes {
                if !seen.contains(&node.id) {
                    push_tree(content, folded, node, 0, true, &mut seen, &mut out);
                }
            }
        }
        MapKind::Swimlane => {
            let steps = content.flows.first().map_or(&[][..], |f| &f.steps[..]);
            let mut order: Vec<&Node> = Vec::new();
            for s in steps {
                if let Some(node) = content.nodes.iter().find(|n| n.id == s.node)
                    && !order.iter().any(|n| n.id == node.id)
                {
                    order.push(node);
                }
            }
            let rest: Vec<&Node> = content
                .nodes
                .iter()
                .filter(|n| !order.iter().any(|o| o.id == n.id))
                .collect();
            order.extend(rest);
            for node in order {
                let first = steps.iter().position(|s| s.node == node.id);
                let lane = first
                    .and_then(|i| steps[i].lane.as_ref())
                    .and_then(|l| content.lanes.iter().find(|x| &x.id == l))
                    .map(|l| l.label.as_str());
                out.push(Row {
                    node,
                    depth: 0,
                    children: 0,
                    folded: false,
                    number: first.map(|i| i + 1),
                    lane,
                });
            }
        }
    }
    out
}

fn push_tree<'a>(
    content: &'a MapContent,
    folded: &HashSet<NodeId>,
    node: &'a Node,
    depth: usize,
    visible: bool,
    seen: &mut HashSet<NodeId>,
    out: &mut Vec<Row<'a>>,
) {
    if !seen.insert(node.id.clone()) {
        return;
    }
    let kids: Vec<&Node> = content
        .groups
        .iter()
        .filter(|g| g.parent == node.id)
        .flat_map(|g| g.children.iter())
        .filter_map(|id| content.nodes.iter().find(|n| &n.id == id))
        .collect();
    let is_folded = !kids.is_empty() && folded.contains(&node.id);
    if visible {
        out.push(Row {
            node,
            depth,
            children: kids.len(),
            folded: is_folded,
            number: None,
            lane: None,
        });
    }
    // Hidden nodes are still walked, so they count as reached.
    for kid in kids {
        push_tree(
            content,
            folded,
            kid,
            depth + 1,
            visible && !is_folded,
            seen,
            out,
        );
    }
}

/// Glyph and colour for a node kind. The kind's word is always written next to it, so colour
/// is never the only cue.
fn kind_style(kind: NodeKind, theme: &Theme) -> (&'static str, Color) {
    match kind {
        NodeKind::Part => ("▣", theme.bold),
        NodeKind::File => ("▤", theme.code),
        NodeKind::Endpoint => ("◈", theme.working),
        NodeKind::Data => ("▥", theme.italic),
        NodeKind::External => ("◇", theme.needs_you),
        NodeKind::Actor => ("◉", theme.speaker_you),
        NodeKind::Outcome => ("✓", theme.ok),
        NodeKind::Logic => ("ƒ", theme.chip),
        NodeKind::Concept => ("○", theme.dim),
    }
}

fn row_line(row: &Row, theme: &Theme, width: usize) -> Line<'static> {
    let (glyph, color) = kind_style(row.node.kind, theme);
    let marker = if row.children == 0 {
        "  "
    } else if row.folded {
        "▸ "
    } else {
        "▾ "
    };
    let mut spans = vec![
        Span::raw("  ".repeat(row.depth)),
        Span::styled(marker, theme.dim()),
    ];
    if let Some(n) = row.number {
        spans.push(Span::styled(format!("{n:>2} "), theme.dim()));
    }
    spans.push(Span::styled(glyph, theme.fg(color)));
    spans.push(Span::styled(format!(" {}", row.node.label), theme.body()));
    let mut facts = vec![row.node.kind.as_str().to_owned()];
    if let Some(lane) = row.lane {
        facts.push(lane.to_owned());
    }
    if row.folded {
        facts.push(format!("{} inside", row.children));
    }
    spans.push(Span::styled(
        format!("  {}", facts.join(" · ")),
        theme.dim(),
    ));
    Line::from(super::super::clip_spans(spans, width))
}

/// The selected node in full, under the list.
fn detail_lines(
    content: &MapContent,
    node: &Node,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let (glyph, color) = kind_style(node.kind, theme);
    let mut lines = vec![
        Line::default(),
        Line::from(Span::styled("╌".repeat(width), theme.fg(theme.line))),
        Line::default(),
        Line::from(vec![
            Span::styled(glyph, theme.fg(color)),
            Span::styled(format!(" {}", node.label), theme.accent()),
            Span::styled(format!("  {}", node.kind.as_str()), theme.dim()),
        ]),
    ];
    let paragraph = |lines: &mut Vec<Line<'static>>, text: &str, style| {
        lines.push(Line::default());
        for l in fmt::wrap(text, width) {
            lines.push(Line::from(Span::styled(l, style)));
        }
    };
    if !node.description.trim().is_empty() {
        paragraph(&mut lines, &node.description, theme.body());
    }
    if let Some(file) = &node.file {
        paragraph(&mut lines, file, theme.fg(theme.code));
    }
    if let Some(code) = &node.pseudocode {
        lines.push(Line::default());
        for l in code.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {}", fmt::truncate(l, width.saturating_sub(2))),
                theme.fg(theme.code),
            )));
        }
    }
    let label_of = |id: &NodeId| {
        content
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .map_or_else(|| id.0.clone(), |n| n.label.clone())
    };
    let mut links = Vec::new();
    for c in &content.connections {
        let (arrow, other) = if c.from == node.id {
            ("→", &c.to)
        } else if c.to == node.id {
            ("←", &c.from)
        } else {
            continue;
        };
        links.push(Line::from(vec![
            Span::styled(format!("{arrow} "), theme.dim()),
            Span::styled(label_of(other), theme.body()),
            Span::styled(format!("  {}", c.label), theme.dim()),
        ]));
    }
    if !links.is_empty() {
        lines.push(Line::default());
        lines.extend(links);
    }
    lines
}

/// The current flow's steps; the step on the selected node carries a marker.
fn flow_lines(
    content: &MapContent,
    flow: &Flow,
    index: usize,
    selected: Option<&NodeId>,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut heading = vec![
        Span::styled("flow ", theme.dim()),
        Span::styled(flow.name.clone(), theme.accent()),
    ];
    if content.flows.len() > 1 {
        heading.push(Span::styled(
            format!("  {}/{} · f next flow", index + 1, content.flows.len()),
            theme.dim(),
        ));
    }
    let mut lines = vec![Line::default(), Line::from(heading)];
    let mut marked = false;
    for (i, s) in flow.steps.iter().enumerate() {
        let here = !marked && selected == Some(&s.node);
        marked |= here;
        let label = content
            .nodes
            .iter()
            .find(|n| n.id == s.node)
            .map_or(s.node.0.as_str(), |n| n.label.as_str());
        let mut text = format!("{} ", i + 1);
        if let Some(arrow) = &s.arrow {
            text.push_str(&format!("({arrow}) "));
        }
        text.push_str(&format!("{label} — {}", s.note));
        lines.push(Line::default());
        for (k, l) in fmt::wrap(&text, width.saturating_sub(2))
            .into_iter()
            .enumerate()
        {
            let lead = if k > 0 {
                "    "
            } else if here {
                "▸ "
            } else {
                "  "
            };
            let style = if here {
                theme.fg(theme.body).add_modifier(Modifier::BOLD)
            } else {
                theme.body()
            };
            lines.push(Line::from(vec![
                Span::styled(lead, theme.fg(theme.focus)),
                Span::styled(l, style),
            ]));
        }
    }
    lines
}

impl Buffer for MapOutlineBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::MapOutline
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let screen = frame.area();
        self.canvas.set_screen(screen.width, screen.height);
        let theme = cx.theme;
        let Some(map) = self.map.clone() else {
            self.canvas.hide();
            let inner = pane(frame, area, title("map", focused, theme), focused, theme);
            let text = "No map open. Pick one from the shelf (␣M).";
            frame.render_widget(Paragraph::new(Span::styled(text, theme.dim())), inner);
            return;
        };
        let content = &map.content;
        let mut heading = format!(
            "{} · {} · v{}",
            content.title,
            content.kind.as_str(),
            map.version
        );
        let zoom = self.canvas.zoom_level();
        if self.canvas.active() && zoom > 1.0 {
            heading.push_str(&format!(" · {zoom:.1}×"));
        }
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        if inner.height == 0 || inner.width == 0 {
            self.canvas.frame_at(&map, None);
            return;
        }
        // The footer takes the last row and a blank above it, when there is room.
        let body = Rect {
            height: inner
                .height
                .saturating_sub(if inner.height >= 5 { 2 } else { 0 }),
            ..inner
        };
        if self.canvas.active() {
            let (canvas, outline) = split(body, self.outline_hidden);
            self.canvas.frame_at(&map, Some(canvas));
            if self.canvas.picture().is_none() {
                let text = match self.canvas.error() {
                    Some(e) => format!("The map could not be drawn: {e}"),
                    None => "Drawing the map…".into(),
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(text, theme.dim())),
                    Rect {
                        y: canvas.y + canvas.height / 2,
                        height: 1,
                        ..canvas
                    },
                );
            }
            if let Some(outline) = outline {
                self.draw_outline(cx, frame, outline, focused, content);
            }
        } else {
            self.canvas.frame_at(&map, None);
            self.draw_outline(cx, frame, body, focused, content);
        }
        let hints: &[&str] = match (self.map_mode, content.kind) {
            (true, _) => &[
                "hjkl select · ⏎ drill in · ⌫ up · n p steps · f flow · + - 0 zoom · ^hjkl pan · v outline · u redraw · Esc leave",
                "hjkl · ⏎ ⌫ · n p · f · + - 0 · ^hjkl · v · u · Esc",
            ],
            (false, MapKind::Structure) => &[
                "␣v map mode · o fold · f next flow · ]] [[ walk the flow · / search · Esc shelf",
                "␣v map · o fold · f flow · ]] [[ steps · Esc shelf",
                "␣v o f ]] [[",
            ],
            (false, MapKind::Swimlane) => &[
                "␣v map mode · f next flow · ]] [[ walk the flow · / search · Esc shelf",
                "␣v map · f flow · ]] [[ steps · Esc shelf",
                "␣v f ]] [[",
            ],
        };
        footer(frame, inner, hints, theme);
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let _ = cx;
        let Some(map) = self.map.clone() else {
            return false;
        };
        let content = &map.content;
        let rows = rows(content, &self.folded);
        if self
            .cursor
            .handle_nav(action, &rows, |r, k| &r.node.id == k, |r| r.node.id.clone())
        {
            self.flow_step = None;
            return true;
        }
        let current = self.resolve(&rows);
        match action {
            Action::MapSelectLeft => self.select_toward(content, Direction::Left, effects),
            Action::MapSelectDown => self.select_toward(content, Direction::Down, effects),
            Action::MapSelectUp => self.select_toward(content, Direction::Up, effects),
            Action::MapSelectRight => self.select_toward(content, Direction::Right, effects),
            Action::MapFollow => self.follow(content, true, effects),
            Action::MapFollowBack => self.follow(content, false, effects),
            Action::MapDrillIn => self.drill_in(content, effects),
            Action::MapDrillOut => {
                if !self.canvas.drill_out() {
                    effects.push(Effect::Note("already at the top".into()));
                }
            }
            Action::MapZoomIn | Action::MapZoomOut | Action::MapZoomFit
                if self.canvas.picture().is_none() =>
            {
                effects.push(Effect::Note(
                    "zoom works on the map picture, which this terminal cannot show".into(),
                ));
            }
            Action::MapZoomIn => self.canvas.zoom(true, None),
            Action::MapZoomOut => self.canvas.zoom(false, None),
            Action::MapZoomFit => self.canvas.fit(),
            Action::MapPanLeft => self.canvas.pan(Direction::Left),
            Action::MapPanDown => self.canvas.pan(Direction::Down),
            Action::MapPanUp => self.canvas.pan(Direction::Up),
            Action::MapPanRight => self.canvas.pan(Direction::Right),
            Action::MapToggleOutline if self.canvas.active() => {
                self.outline_hidden = !self.outline_hidden;
            }
            Action::MapToggleOutline => {
                effects.push(Effect::Note(
                    "the outline is the whole view in this terminal".into(),
                ));
            }
            Action::MapToggleFold => match current.map(|i| &rows[i]) {
                Some(row) if row.children > 0 => {
                    let id = row.node.id.clone();
                    if !self.folded.remove(&id) {
                        self.folded.insert(id);
                    }
                }
                _ => effects.push(Effect::Note("nothing inside this node to fold".into())),
            },
            Action::MapNextFlow => {
                if content.flows.len() > 1 {
                    self.flow = (self.flow + 1) % content.flows.len();
                    self.flow_step = None;
                } else {
                    effects.push(Effect::Note("this map has one flow".into()));
                }
            }
            Action::MapNextStep => self.walk_flow(content, true, effects),
            Action::MapPrevStep => self.walk_flow(content, false, effects),
            _ => return false,
        }
        true
    }

    fn search_rows(&self, _cx: &ViewCx) -> Vec<String> {
        let Some(map) = &self.map else {
            return Vec::new();
        };
        rows(&map.content, &self.folded)
            .iter()
            .map(|r| {
                format!(
                    "{} {} {}",
                    r.node.label,
                    r.node.kind.as_str(),
                    r.node.description
                )
            })
            .collect()
    }

    fn current_row(&self, _cx: &ViewCx) -> usize {
        let Some(map) = &self.map else {
            return 0;
        };
        self.resolve(&rows(&map.content, &self.folded)).unwrap_or(0)
    }

    fn goto_row(&mut self, row: usize, _cx: &ViewCx, _effects: &mut Vec<Effect>) {
        let Some(map) = self.map.clone() else {
            return;
        };
        let rows = rows(&map.content, &self.folded);
        if row < rows.len() {
            self.select(&rows, row);
        }
    }
}
