//! The map outline: one map as text. It is the left-hand companion of the future canvas and the
//! whole view in terminals without kitty graphics.
//!
//! A structure map is a tree: groups nest their children, and `o` folds a group. A swimlane map
//! is a list of its nodes in the order the flow walks them, each with its lane. Below the list
//! sit the selected node in full (description, file, pseudocode, connections) and the current
//! flow's steps; `f` switches flow and `]]`/`[[` walk the flow's steps. The shelf
//! ([`super::MapShelfBuffer`]) opens maps here.

use std::collections::HashSet;
use std::sync::Arc;

use dovetail_api::map::{Flow, Map, MapContent, MapId, MapKind, Node, NodeId, NodeKind};
use dovetail_api::{Command, Event};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Buffer, Effect, ViewCx, footer, pane, scroll_to, step, title};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;
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
    /// A map asked for with [`Command::LoadMap`] that has not arrived.
    loading: Option<MapId>,
    /// The selected node, kept by id so a redrawn version never moves the cursor.
    selected: Option<NodeId>,
    /// Where the selection was last, for when its node goes away.
    last_index: usize,
    offset: usize,
    /// Groups folded shut.
    folded: HashSet<NodeId>,
    /// Index of the flow shown below the list.
    flow: usize,
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
    /// A map was asked for; the next [`Event::MapLoaded`] for it shows the outline.
    pub fn await_map(&mut self, id: MapId) {
        self.loading = Some(id);
    }

    /// The open map's id.
    pub fn map_id(&self) -> Option<&MapId> {
        self.map.as_ref().map(|m| &m.id)
    }

    /// The selected node.
    pub fn selected_node(&self) -> Option<&NodeId> {
        self.selected.as_ref()
    }

    /// Takes in a map event. A newer version of the open map replaces it in place, keeping the
    /// cursor on the same node.
    pub fn on_event(&mut self, event: &Event) -> Option<Reaction> {
        match event {
            Event::MapLoaded(map) => {
                let wanted = self.loading.as_ref() == Some(&map.id);
                if wanted {
                    self.loading = None;
                }
                if wanted || self.map_id() == Some(&map.id) {
                    self.set_map(map.clone());
                }
                wanted.then_some(Reaction::Show)
            }
            Event::MapUpserted(summary) => {
                let open = self.map.as_ref()?;
                (open.id == summary.id && summary.version > open.version).then(|| {
                    Reaction::Request(Command::LoadMap {
                        id: summary.id.clone(),
                        version: None,
                    })
                })
            }
            Event::CommandFailed { .. } => {
                self.loading = None;
                None
            }
            _ => None,
        }
    }

    fn set_map(&mut self, map: Map) {
        if self.map_id() != Some(&map.id) {
            self.selected = None;
            self.last_index = 0;
            self.offset = 0;
            self.folded.clear();
            self.flow = 0;
        }
        self.flow = self.flow.min(map.content.flows.len().saturating_sub(1));
        self.map = Some(Arc::new(map));
    }

    fn resolve(&self, rows: &[Row]) -> Option<usize> {
        if rows.is_empty() {
            return None;
        }
        if let Some(id) = &self.selected
            && let Some(i) = rows.iter().position(|r| &r.node.id == id)
        {
            return Some(i);
        }
        Some(self.last_index.min(rows.len() - 1))
    }

    fn select(&mut self, rows: &[Row], index: usize) {
        self.selected = Some(rows[index].node.id.clone());
        self.last_index = index;
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

    /// `]]`/`[[`: the node of the next or previous step of the current flow.
    fn walk_flow(&mut self, content: &MapContent, forward: bool, effects: &mut Vec<Effect>) {
        let Some(flow) = content.flows.get(self.flow) else {
            effects.push(Effect::Note("this map has no flow".into()));
            return;
        };
        let rows = rows(content, &self.folded);
        let current = self
            .resolve(&rows)
            .map(|i| &rows[i].node.id)
            .and_then(|id| flow.steps.iter().position(|s| &s.node == id));
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
        match target {
            Some(step) => self.goto_node(content, &step.node.clone()),
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
    Line::from(super::clip_spans(spans, width))
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
        let theme = cx.theme;
        let Some(map) = self.map.clone() else {
            let inner = pane(frame, area, title("map", focused, theme), focused, theme);
            let text = "No map open. Pick one from the shelf (␣M).";
            frame.render_widget(Paragraph::new(Span::styled(text, theme.dim())), inner);
            return;
        };
        let content = &map.content;
        let heading = format!(
            "{} · {} · v{}",
            content.title,
            content.kind.as_str(),
            map.version
        );
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let column = reading_column::column(inner, reading_column::DEFAULT_WIDTH);
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
        // The footer takes the last row and a blank above it, when there is room.
        let usable =
            usize::from(
                column
                    .height
                    .saturating_sub(if column.height >= 5 { 2 } else { 0 }),
            );
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
            self.offset = scroll_to(self.offset, i, i, list_height);
        }
        self.offset = self.offset.min(rows.len().saturating_sub(list_height));
        let lines: Vec<Line<'static>> = rows
            .iter()
            .skip(self.offset)
            .take(list_height)
            .map(|r| row_line(r, theme, width))
            .collect();
        let list_area = Rect {
            height: list_height as u16,
            ..column
        };
        frame.render_widget(Paragraph::new(lines), list_area);
        if focused && let Some(i) = selected {
            let y = list_area.y + (i - self.offset) as u16;
            if y < list_area.y + list_area.height {
                frame.buffer_mut().set_style(
                    Rect {
                        y,
                        height: 1,
                        ..inner
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
        let hints: &[&str] = match content.kind {
            MapKind::Structure => &[
                "o fold · f next flow · ]] [[ walk the flow · / search · Esc shelf",
                "o fold · f flow · ]] [[ steps · Esc shelf",
                "o f ]] [[",
            ],
            MapKind::Swimlane => &[
                "f next flow · ]] [[ walk the flow · / search · Esc shelf",
                "f flow · ]] [[ steps · Esc shelf",
                "f ]] [[",
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
        let current = self.resolve(&rows);
        match action {
            Action::MoveDown | Action::MoveUp => {
                if let Some(i) = current {
                    let delta = if action == Action::MoveDown { 1 } else { -1 };
                    self.select(&rows, step(i, rows.len(), delta));
                }
            }
            Action::GoTop if !rows.is_empty() => self.select(&rows, 0),
            Action::GoBottom if !rows.is_empty() => self.select(&rows, rows.len() - 1),
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
