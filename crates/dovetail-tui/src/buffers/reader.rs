//! Reading mode's reading column (ADR 0018): one document at a time, from the library or a
//! followed citation, in a centred column of at most 96 columns of text, padded, with no
//! background.
//!
//! The document is split at its headings into sections (`]]`/`[[` jump between them, `zc`/`zo`
//! fold one), and each section into markdown and image parts. Citation ids (`[F3]`, `[S2]`,
//! each id of `[F1, S2]`) become stops: `]s`/`[s` select one, `gd` opens it, `gx` opens a web
//! source in the browser through the core. HTML is converted to markdown first. Images are
//! placed with kitty graphics when the terminal has them ([`Figure`]), else shown as a line of
//! text.

use std::collections::HashSet;
use std::path::PathBuf;

use dovetail_api::{
    Event, tracking::decision::DecisionId, tracking::project::ProjectId, tracking::task::TaskId,
};
use dovetail_api::{
    agents::tangent::TangentEvent,
    app::AppEvent,
    research::ResearchEvent,
    research::library::{BrowseWhat, BrowsedDoc, LibraryEvent},
};
use dovetail_api::{
    research::library::DocImage,
    research::{Evidence, ResearchDocument, ResearchWhat},
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Buffer, Effect, ViewCx, footer, pane, title};
use crate::action::Action;
use crate::canvas::kitty::Figure;
use crate::canvas::view::CellPx;
use crate::region::BufferKind;
use crate::render::{citations, html, markdown, reading_column};
use crate::theme::Theme;

/// Most documents `Ctrl-o` goes back through.
const BACK_LIMIT: usize = 50;
/// Tallest an image is drawn, in rows.
const MAX_FIGURE_ROWS: usize = 40;
/// kitty image ids of the reading column's figures start here (the map uses 7301 and 7302).
pub const FIGURE_ID_BASE: u32 = 7_400;

/// Something the reading column shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadTarget {
    /// A report, brief, finding or source of a research task.
    Research {
        /// The task.
        task: TaskId,
        /// Which document.
        what: ResearchWhat,
    },
    /// An answer, finding or source of a tangent, in Tangents (ADR 0017). Its citations
    /// resolve against the tangent's own findings and sources.
    Tangent {
        /// The tangent's task in Tangents.
        task: TaskId,
        /// Which document.
        what: ResearchWhat,
    },
    /// A decision-log entry, written out from the snapshot.
    Decision(DecisionId),
    /// A repo doc, relative to `docs/`.
    Doc(PathBuf),
    /// One of the above, of a project browsed with `<leader>fr` rather than the open one
    /// (ADR 0018, decided 2026-11-11 later): fetched with `LibraryCommand::LoadBrowsed`, read-only.
    Elsewhere(ProjectId, Box<ReadTarget>),
}

impl ReadTarget {
    /// The browsed project it belongs to, when it is not the open one's.
    pub fn elsewhere(&self) -> Option<&ProjectId> {
        match self {
            ReadTarget::Elsewhere(project, _) => Some(project),
            _ => None,
        }
    }
}

/// A document open in the reading column.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    /// What it is.
    pub target: ReadTarget,
    /// The research task its citations belong to, if any.
    pub task: Option<TaskId>,
    /// Pane title part: `report`, `F1 · nucleo`, `adr/0018-…`.
    pub title: String,
    /// Its text and images.
    pub document: ResearchDocument,
}

/// A citation id in the laid-out text: a line and a char range of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stop {
    line: usize,
    start: usize,
    end: usize,
    evidence: Evidence,
}

/// Rows kept for an image, under a line of its own.
#[derive(Debug, Clone)]
struct Slot {
    line: usize,
    cols: u16,
    rows: usize,
    path: PathBuf,
}

/// What the last layout made of the document.
#[derive(Default)]
struct Layout {
    width: usize,
    lines: Vec<Line<'static>>,
    sections: Vec<usize>,
    stops: Vec<Stop>,
    slots: Vec<Slot>,
    page: usize,
    /// Where the text was drawn: the column's left edge and first row.
    origin: (u16, u16),
    height: usize,
}

/// The reading column.
#[derive(Default)]
pub struct ReaderBuffer {
    open: Option<Opened>,
    awaiting: Option<(ReadTarget, Option<TaskId>, String)>,
    back: Vec<(Opened, usize)>,
    top: usize,
    cite: Option<usize>,
    folded: HashSet<usize>,
    graphics: Option<CellPx>,
    layout: Layout,
    stale: bool,
}

impl ReaderBuffer {
    /// The document shown.
    pub fn opened(&self) -> Option<&Opened> {
        self.open.as_ref()
    }

    /// The research task of the document shown, for Ask and for citations.
    pub fn task(&self) -> Option<TaskId> {
        self.open.as_ref().and_then(|o| o.task)
    }

    /// Forgets documents of browsed projects, shown or on the `Ctrl-o` stack: reading mode
    /// starts on the open project (ADR 0018, decided 2026-11-11 later).
    pub fn forget_elsewhere(&mut self) {
        self.back.retain(|(o, _)| o.target.elsewhere().is_none());
        if self
            .awaiting
            .as_ref()
            .is_some_and(|(t, _, _)| t.elsewhere().is_some())
        {
            self.awaiting = None;
        }
        if self
            .open
            .as_ref()
            .is_some_and(|o| o.target.elsewhere().is_some())
        {
            self.open = self.back.pop().map(|(o, _)| o);
            self.top = 0;
            self.reset_view();
        }
    }

    /// kitty graphics on (with the cell size) or off.
    pub fn set_graphics(&mut self, cell: Option<CellPx>) {
        if self.graphics != cell {
            self.graphics = cell;
            self.stale = true;
        }
    }

    /// Notes that `target` was asked of the core; its answer opens it.
    pub fn await_target(&mut self, target: ReadTarget, task: Option<TaskId>, title: String) {
        self.awaiting = Some((target, task, title));
    }

    /// Whether a document is on its way.
    pub fn is_loading(&self) -> bool {
        self.awaiting.is_some()
    }

    /// Shows a document; the one shown before goes on the `Ctrl-o` stack.
    pub fn open(&mut self, opened: Opened) {
        if let Some(previous) = self.open.take()
            && previous.target != opened.target
        {
            self.back.push((previous, self.top));
            if self.back.len() > BACK_LIMIT {
                self.back.remove(0);
            }
        }
        self.open = Some(opened);
        self.top = 0;
        self.reset_view();
    }

    fn reset_view(&mut self) {
        self.cite = None;
        self.folded.clear();
        self.stale = true;
    }

    /// Takes in a core answer. Returns whether the awaited document arrived and is now shown.
    pub fn on_event(&mut self, event: &Event) -> bool {
        let (target, document) = match event {
            Event::Research(ResearchEvent::Loaded {
                task,
                what,
                content: dovetail_api::research::ResearchContent::Document(document),
            }) => (
                ReadTarget::Research {
                    task: *task,
                    what: what.clone(),
                },
                document,
            ),
            Event::Tangent(TangentEvent::Loaded {
                tangent,
                what,
                content: dovetail_api::research::ResearchContent::Document(document),
            }) => (
                ReadTarget::Tangent {
                    task: *tangent,
                    what: what.clone(),
                },
                document,
            ),
            Event::Library(LibraryEvent::DocLoaded { path, document }) => {
                (ReadTarget::Doc(path.clone()), document)
            }
            Event::Library(LibraryEvent::BrowsedLoaded {
                doc: BrowsedDoc { project, what },
                document,
            }) => {
                let inner = match what {
                    BrowseWhat::Research { task, what } => ReadTarget::Research {
                        task: *task,
                        what: what.clone(),
                    },
                    BrowseWhat::Doc(path) => ReadTarget::Doc(path.clone()),
                };
                (
                    ReadTarget::Elsewhere(project.clone(), Box::new(inner)),
                    document,
                )
            }
            Event::App(AppEvent::CommandFailed { .. }) => {
                self.awaiting = None;
                return false;
            }
            _ => return false,
        };
        if self.awaiting.as_ref().is_none_or(|(t, _, _)| *t != target) {
            return false;
        }
        let Some((target, task, title)) = self.awaiting.take() else {
            return false;
        };
        self.open(Opened {
            target,
            task,
            title,
            document: document.clone(),
        });
        true
    }

    /// The selected citation, or else the first one in view.
    fn current_stop(&self) -> Option<Stop> {
        let in_view = |s: &Stop| s.line >= self.top && s.line < self.top + self.layout.height;
        self.cite
            .and_then(|i| self.layout.stops.get(i).copied())
            .filter(in_view)
            .or_else(|| self.layout.stops.iter().copied().find(in_view))
    }

    /// The selected citation, for the status bar: `F1`, and its place among them.
    pub fn citation_label(&self) -> Option<String> {
        let i = self.cite?;
        let stop = self.layout.stops.get(i)?;
        Some(format!(
            "[{}] {}/{}",
            stop.evidence,
            i + 1,
            self.layout.stops.len()
        ))
    }

    /// The images in view, fully, as kitty should place them.
    pub fn figures(&self) -> Vec<Figure> {
        if self.graphics.is_none() {
            return Vec::new();
        }
        let (col, row) = self.layout.origin;
        self.layout
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.line >= self.top && s.line + s.rows <= self.top + self.layout.height)
            .map(|(i, s)| Figure {
                id: FIGURE_ID_BASE + i as u32,
                path: s.path.clone(),
                col,
                row: row + (s.line - self.top) as u16,
                cols: s.cols,
                rows: s.rows as u16,
            })
            .collect()
    }

    fn select(&mut self, forward: bool, effects: &mut Vec<Effect>) {
        let stops = &self.layout.stops;
        if stops.is_empty() {
            effects.push(Effect::Note("no citations in this document".into()));
            return;
        }
        let height = self.layout.height.max(1);
        let in_view = |s: &Stop| s.line >= self.top && s.line < self.top + height;
        let next = match self.cite.filter(|&i| stops.get(i).is_some_and(in_view)) {
            Some(i) if forward => (i + 1).min(stops.len() - 1),
            Some(i) => i.saturating_sub(1),
            None if forward => stops
                .iter()
                .position(|s| s.line >= self.top)
                .unwrap_or(stops.len() - 1),
            None => stops
                .iter()
                .rposition(|s| s.line < self.top + height)
                .unwrap_or(0),
        };
        self.cite = Some(next);
        let line = stops[next].line;
        self.top = super::scroll_to(self.top, line, line, height);
    }

    fn section_of(&self, line: usize) -> Option<usize> {
        self.layout.sections.iter().rposition(|&s| s <= line)
    }

    fn scroll(&mut self, top: usize) {
        let max = self
            .layout
            .lines
            .len()
            .saturating_sub(self.layout.height.max(1));
        self.top = top.min(max);
    }

    /// Lays the document out at `width` (when anything changed since the last time).
    fn build(&mut self, width: usize, theme: &Theme) {
        if !self.stale && self.layout.width == width {
            return;
        }
        let Some(open) = &self.open else {
            self.layout = Layout::default();
            return;
        };
        let text = if html::looks_like_html(&open.document.markdown) {
            html::to_markdown(&open.document.markdown)
        } else {
            open.document.markdown.clone()
        };
        let text = citations::keep_together(&text);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut sections = Vec::new();
        let mut slots = Vec::new();
        for (i, section) in split_sections(&text).iter().enumerate() {
            gap(&mut lines);
            sections.push(lines.len());
            if self.folded.contains(&i) && section.heading {
                let head = section.text.lines().next().unwrap_or("");
                let mut first = markdown::render(head, width, theme);
                if let Some(line) = first.first_mut() {
                    line.spans.push(Span::styled("  … zo", theme.dim()));
                }
                lines.extend(citations::colour(first, theme));
                continue;
            }
            for part in split_parts(&section.text) {
                match part {
                    Part::Markdown(md) => {
                        gap(&mut lines);
                        lines.extend(citations::colour(
                            markdown::render(&md, width, theme),
                            theme,
                        ));
                    }
                    Part::Image { alt, src } => {
                        gap(&mut lines);
                        let image = open.document.images.iter().find(|i| i.src == src);
                        match (image, self.graphics) {
                            (Some(image), Some(cell)) => {
                                let (cols, rows) = figure_size(image, cell, width);
                                slots.push(Slot {
                                    line: lines.len(),
                                    cols,
                                    rows,
                                    path: image.path.clone(),
                                });
                                lines.extend((0..rows).map(|_| Line::default()));
                                lines.push(Line::from(Span::styled(
                                    format!("▣ {alt}"),
                                    theme.dim(),
                                )));
                            }
                            _ => lines.push(Line::from(vec![
                                Span::styled("▣ image: ", theme.dim()),
                                Span::styled(
                                    if alt.is_empty() {
                                        "untitled".into()
                                    } else {
                                        alt
                                    },
                                    theme.fg(theme.italic),
                                ),
                                Span::styled(format!(" · {src}"), theme.dim()),
                            ])),
                        }
                    }
                }
            }
        }
        let stops = stops_in(&lines);
        self.cite = self.cite.filter(|&i| i < stops.len());
        self.layout = Layout {
            width,
            lines,
            sections,
            stops,
            slots,
            ..std::mem::take(&mut self.layout)
        };
        self.stale = false;
    }
}

/// A blank row before what follows, unless there is one already (or nothing yet).
fn gap(lines: &mut Vec<Line<'static>>) {
    if lines.last().is_some_and(|l| l.width() != 0) {
        lines.push(Line::default());
    }
}

/// A heading and what follows it, up to the next heading.
struct Section {
    heading: bool,
    text: String,
}

/// Splits markdown at its ATX headings (`#` to `######`), outside code fences. Text before the
/// first heading is a section of its own.
fn split_sections(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut fenced = false;
    let mut current = Section {
        heading: false,
        text: String::new(),
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        let heading = !fenced && is_heading(trimmed);
        if heading && (current.heading || !current.text.trim().is_empty()) {
            out.push(std::mem::replace(
                &mut current,
                Section {
                    heading: true,
                    text: String::new(),
                },
            ));
        } else if heading {
            current.heading = true;
        }
        current.text.push_str(line);
        current.text.push('\n');
    }
    if current.heading || !current.text.trim().is_empty() {
        out.push(current);
    }
    out
}

fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with([' ', '\t'])
}

enum Part {
    Markdown(String),
    Image { alt: String, src: String },
}

/// Splits a section at lines that hold one image and nothing else (`![alt](src)`).
fn split_parts(text: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut md = String::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        match (!fenced).then(|| lone_image(trimmed)).flatten() {
            Some((alt, src)) => {
                if !md.trim().is_empty() {
                    out.push(Part::Markdown(std::mem::take(&mut md)));
                }
                md.clear();
                out.push(Part::Image { alt, src });
            }
            None => {
                md.push_str(line);
                md.push('\n');
            }
        }
    }
    if !md.trim().is_empty() {
        out.push(Part::Markdown(md));
    }
    out
}

/// `![alt](src "title")` filling the whole line.
fn lone_image(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let inner = rest.strip_suffix(')')?;
    let src = inner.split_whitespace().next()?.trim_matches(['<', '>']);
    (!src.is_empty() && !alt.contains(']')).then(|| (alt.to_string(), src.to_string()))
}

/// Cells for an image `width` cells wide at most, keeping its shape.
fn figure_size(image: &DocImage, cell: CellPx, width: usize) -> (u16, usize) {
    let cw = f64::from(cell.w.max(1));
    let ch = f64::from(cell.h.max(1));
    let natural = (f64::from(image.width) / cw).ceil() as usize;
    let cols = natural.clamp(1, width.max(1));
    let rows = (cols as f64 * cw * f64::from(image.height) / f64::from(image.width) / ch).ceil();
    let rows = (rows as usize).clamp(1, MAX_FIGURE_ROWS);
    (cols as u16, rows)
}

/// Every citation id in the lines, in reading order.
fn stops_in(lines: &[Line<'static>]) -> Vec<Stop> {
    let mut out = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let text = crate::fmt::line_plain_text(line);
        let chars: Vec<char> = text.chars().collect();
        for (a, b) in citations::markers(&text) {
            // Inside the brackets: ids separated by commas.
            let mut at = a + 1;
            let inner: String = chars[a + 1..b - 1].iter().collect();
            for part in inner.split(',') {
                let len = part.chars().count();
                let lead = part.chars().take_while(|c| c.is_whitespace()).count();
                let id = part.trim();
                if let Some(evidence) = Evidence::parse(id) {
                    let start = at + lead;
                    out.push(Stop {
                        line: n,
                        start,
                        end: start + id.chars().count(),
                        evidence,
                    });
                }
                at += len + 1;
            }
        }
    }
    out
}

/// `line` with chars `start..end` in `style` on top of their own.
fn highlight(line: &Line<'static>, start: usize, end: usize, style: Style) -> Line<'static> {
    let mut spans = Vec::new();
    let mut pos = 0;
    for span in &line.spans {
        let mut run = String::new();
        let mut run_in = false;
        for ch in span.content.chars() {
            let inside = (start..end).contains(&pos);
            if inside != run_in && !run.is_empty() {
                let s = if run_in {
                    span.style.patch(style)
                } else {
                    span.style
                };
                spans.push(Span::styled(std::mem::take(&mut run), s));
            }
            run_in = inside;
            run.push(ch);
            pos += 1;
        }
        if !run.is_empty() {
            let s = if run_in {
                span.style.patch(style)
            } else {
                span.style
            };
            spans.push(Span::styled(run, s));
        }
    }
    Line {
        spans,
        ..line.clone()
    }
}

impl Buffer for ReaderBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Reader
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let heading = match &self.open {
            Some(open) => format!("reading · {}", open.title),
            None => "reading".to_string(),
        };
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        // Transparent, padded, 96 columns of text (ADR 0018, decided 2026-11-11 later).
        let column = reading_column::reading_text(inner);
        if self.open.is_none() {
            let text = if self.awaiting.is_some() {
                "loading…"
            } else {
                "Pick something in the library: ⏎ opens it here. ]s [s move between citations, \
                 gd opens one, gx opens a web source in the browser, i asks about the report."
            };
            frame.render_widget(
                Paragraph::new(Span::styled(text, theme.dim())).wrap(Wrap { trim: true }),
                column,
            );
            self.layout.slots.clear();
            return;
        }
        let width = usize::from(column.width);
        self.build(width, theme);
        let height = usize::from(column.height.saturating_sub(1)).max(1);
        self.layout.height = height;
        self.layout.page = height.saturating_sub(2).max(1);
        self.layout.origin = (column.x, column.y);
        self.top = self.top.min(self.layout.lines.len().saturating_sub(height));
        let selected = self.cite.and_then(|i| self.layout.stops.get(i).copied());
        for (row, (n, line)) in self
            .layout
            .lines
            .iter()
            .enumerate()
            .skip(self.top)
            .take(height)
            .enumerate()
        {
            let rect = Rect {
                y: column.y + row as u16,
                height: 1,
                ..column
            };
            let line = match selected {
                Some(s) if s.line == n => highlight(line, s.start, s.end, theme.selected()),
                _ => line.clone(),
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        let last = self.layout.lines.len();
        let more = if last > self.top + height {
            format!("{}%", (self.top + height) * 100 / last.max(1))
        } else {
            "end".into()
        };
        footer(
            frame,
            column,
            &[
                &format!("{more} · ]s [s cite · gd open · gx browser · ^o back · i ask · ⎋ leave"),
                &format!("{more} · ]s [s · gd · gx · i ask"),
                &more,
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, _cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let top = self.top;
        let page = self.layout.page.max(1);
        match action {
            Action::MoveDown => self.scroll(top + 1),
            Action::MoveUp => self.scroll(top.saturating_sub(1)),
            Action::PageDown => self.scroll(top + page),
            Action::PageUp => self.scroll(top.saturating_sub(page)),
            Action::GoTop => self.top = 0,
            Action::GoBottom => self.scroll(usize::MAX),
            Action::NextTurn => {
                let next = self.layout.sections.iter().copied().find(|&s| s > top);
                if let Some(line) = next {
                    self.scroll(line);
                }
            }
            Action::PrevTurn => {
                let prev = self.layout.sections.iter().copied().rfind(|&s| s < top);
                self.scroll(prev.unwrap_or(0));
            }
            Action::FoldTurn | Action::UnfoldTurn => {
                let Some(section) = self.section_of(top) else {
                    return true;
                };
                let changed = if action == Action::FoldTurn {
                    self.folded.insert(section)
                } else {
                    self.folded.remove(&section)
                };
                if changed {
                    self.stale = true;
                    self.cite = None;
                    if let Some(&start) = self.layout.sections.get(section) {
                        self.top = start;
                    }
                }
            }
            Action::NextCitation => self.select(true, effects),
            Action::PrevCitation => self.select(false, effects),
            Action::OpenCitation | Action::BrowseCitation => {
                let Some(open) = &self.open else {
                    effects.push(Effect::Note(
                        "nothing open: ⏎ in the library opens a document".into(),
                    ));
                    return true;
                };
                let stop = self.current_stop();
                if let Some(stop) = stop {
                    self.cite = self.layout.stops.iter().position(|s| *s == stop);
                }
                effects.push(Effect::Cite {
                    task: open.task,
                    evidence: stop.map(|s| s.evidence),
                    browse: action == Action::BrowseCitation,
                    from: open.target.clone(),
                });
            }
            Action::ReaderBack => match self.back.pop() {
                Some((previous, top)) => {
                    self.open = Some(previous);
                    self.reset_view();
                    self.top = top;
                }
                None => effects.push(Effect::Note("nothing to go back to".into())),
            },
            _ => return false,
        }
        true
    }

    fn search_rows(&self, _cx: &ViewCx) -> Vec<String> {
        self.layout
            .lines
            .iter()
            .map(crate::fmt::line_plain_text)
            .collect()
    }

    fn current_row(&self, _cx: &ViewCx) -> usize {
        self.top
    }

    fn goto_row(&mut self, row: usize, _cx: &ViewCx, _effects: &mut Vec<Effect>) {
        self.scroll(row.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn sections_split_at_headings_outside_code() {
        let md = "intro\n# One\na\n```\n# not a heading\n```\n## Two\nb\n";
        let s = split_sections(md);
        assert_eq!(s.len(), 3);
        assert!(!s[0].heading && s[1].heading && s[2].heading);
        assert!(s[1].text.contains("# not a heading"));
    }

    #[test]
    fn a_lone_image_is_its_own_part() {
        let parts = split_parts("before\n![A chart](fig.png)\nafter ![inline](x.png)\n");
        assert_eq!(parts.len(), 3);
        assert!(
            matches!(&parts[1], Part::Image { alt, src } if alt == "A chart" && src == "fig.png")
        );
    }

    #[test]
    fn each_id_of_a_marker_is_a_stop() {
        let lines = vec![Line::from(
            "use nucleo [F1][S1] and [F1, S2]; not [3] or [x]",
        )];
        let stops = stops_in(&lines);
        let ids: Vec<String> = stops.iter().map(|s| s.evidence.to_string()).collect();
        assert_eq!(ids, ["F1", "S1", "F1", "S2"]);
        let chars: Vec<char> = text(&lines[0]).chars().collect();
        let last = stops[3];
        assert_eq!(chars[last.start..last.end].iter().collect::<String>(), "S2");
    }

    #[test]
    fn an_image_keeps_its_shape_within_the_width() {
        let image = DocImage {
            src: "a.png".into(),
            path: "/a.png".into(),
            width: 1600,
            height: 800,
        };
        let cell = CellPx { w: 10, h: 20 };
        // 160 cells natural, cut to 100; 100 * 10 * 800 / 1600 / 20 = 25 rows.
        assert_eq!(figure_size(&image, cell, 100), (100, 25));
    }
}
