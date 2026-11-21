//! The research part of the task detail (R3, `docs/research.md` §9, §10): the report, rounds,
//! findings and sources of a task as rows, and the read-only document view `Enter` opens.
//!
//! Content is not in the snapshot: the list comes from `LoadResearch { Listing }`, each
//! document from `LoadResearch { <what> }`; the task detail asks and keeps the answers here.

use std::path::PathBuf;

use dovetail_api::research::{
    ResearchDocument, ResearchListing, ResearchSummary, ResearchWhat, SourceKind, ordinal,
    queue_position,
};
use dovetail_api::{
    Snapshot,
    task::{Task, TaskId},
};
use jiff::tz::TimeZone;
use ratatui::text::{Line, Span};

use crate::fmt;
use crate::theme::Theme;

/// One row of the section that `Enter` opens.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// What to ask the core for.
    pub what: ResearchWhat,
    /// A short name for the pane title: `report`, `round 2 brief`, `F7`, `S3`.
    pub title: String,
    /// The file behind it, for `e`; `None` when there is none (a source that is not cached).
    pub path: Option<PathBuf>,
}

/// The section's lines and, for each selectable row, its line number and item.
#[derive(Debug, Default)]
pub struct Section {
    /// The lines, to append to the task detail.
    pub lines: Vec<Line<'static>>,
    /// Selectable rows: index into `lines`, and what they open.
    pub items: Vec<(usize, Item)>,
}

/// The listing of one task, as last answered by the core.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The task it belongs to.
    pub task: TaskId,
    /// Findings and sources.
    pub listing: ResearchListing,
}

/// A document being read.
#[derive(Debug, Clone)]
pub struct Doc {
    /// Pane title part.
    pub title: String,
    /// Its file, absolute, if it has one.
    pub path: Option<PathBuf>,
    /// The markdown.
    pub markdown: String,
}

impl Doc {
    /// A document as the core answered with it.
    pub fn new(title: String, document: ResearchDocument) -> Self {
        Self {
            title,
            path: document.path,
            markdown: document.markdown,
        }
    }
}

/// Builds the "Research" section for `task`; empty when the task has no research.
pub fn section(
    snap: &Snapshot,
    task: &Task,
    loaded: Option<&Loaded>,
    theme: &Theme,
    tz: &TimeZone,
    width: usize,
) -> Section {
    let mut out = Section::default();
    let Some(summary) = snap.research.iter().find(|r| r.task == task.id) else {
        return out;
    };
    out.lines.push(Line::default());
    out.lines
        .push(Line::from(Span::styled("Research", theme.accent())));
    out.lines.push(Line::default());
    out.lines
        .push(Line::from(Span::styled(counts(summary), theme.dim())));
    out.lines.push(Line::default());

    match summary.report_ms {
        Some(ms) => {
            let line = vec![
                Span::styled("◆ ", theme.fg(theme.bold)),
                Span::styled(
                    "Report",
                    theme.body().add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Span::styled(format!("  written {}", fmt::when(ms, tz)), theme.dim()),
            ];
            let item = Item {
                what: ResearchWhat::Report,
                title: "report".into(),
                path: Some(summary.dir.join("report.md")),
            };
            out.push_item(line, item);
        }
        None => out
            .lines
            .push(Line::from(Span::styled("◇ no report yet", theme.dim()))),
    }

    let listing = loaded.filter(|l| l.task == task.id).map(|l| &l.listing);
    for round in &summary.rounds {
        out.lines.push(Line::default());
        let state = if round.closed_ms.is_some() {
            "closed".to_owned()
        } else if round.queued {
            match queue_position(&snap.research, task.id, round.n) {
                Some(place) => format!("queued ({} in line)", ordinal(place)),
                None => "queued".to_owned(),
            }
        } else {
            "open".to_owned()
        };
        let line = vec![
            Span::styled(format!("Round {} ", round.n), theme.body()),
            Span::styled(
                format!(
                    "· {state} · {} finding{} · {}",
                    round.findings,
                    plural(round.findings),
                    fmt::when(round.opened_ms, tz)
                ),
                theme.dim(),
            ),
        ];
        let item = Item {
            what: ResearchWhat::Brief { round: round.n },
            title: format!("round {} brief", round.n),
            path: Some(summary.dir.join(&round.dir).join("brief.md")),
        };
        out.push_item(line, item);
        for text in fmt::wrap(&round.brief, width.saturating_sub(4))
            .into_iter()
            .take(2)
        {
            out.lines
                .push(Line::from(Span::styled(format!("    {text}"), theme.dim())));
        }
        if let Some(listing) = listing {
            for f in listing.findings.iter().filter(|f| f.round == Some(round.n)) {
                out.finding(f, theme);
            }
        }
    }

    if let Some(listing) = listing {
        let quick: Vec<_> = listing
            .findings
            .iter()
            .filter(|f| f.round.is_none())
            .collect();
        if !quick.is_empty() {
            out.lines.push(Line::default());
            out.lines
                .push(Line::from(Span::styled("Quick findings", theme.body())));
            for f in quick {
                out.finding(f, theme);
            }
        }
        if !listing.sources.is_empty() {
            out.lines.push(Line::default());
            out.lines.push(Line::from(Span::styled(
                format!("Sources ({})", listing.sources.len()),
                theme.body(),
            )));
            for s in &listing.sources {
                let kind = match s.kind {
                    SourceKind::Web => "web",
                    SourceKind::File => "file",
                    SourceKind::Search => "search",
                };
                let mut line = vec![
                    Span::styled(format!("  S{:<3} ", s.id.0), theme.fg(theme.cite)),
                    Span::styled(format!("{kind:<6} "), theme.dim()),
                    Span::styled(
                        fmt::truncate(&s.locator, width.saturating_sub(14)),
                        if s.is_error {
                            theme.fg(theme.error)
                        } else {
                            theme.body()
                        },
                    ),
                ];
                if s.is_error {
                    line.push(Span::styled(" (failed)", theme.fg(theme.error)));
                }
                let item = Item {
                    what: ResearchWhat::Source(s.id),
                    title: format!("S{}", s.id.0),
                    path: s.path.clone(),
                };
                out.push_item(line, item);
            }
        }
    } else if summary.findings > 0 || summary.sources > 0 {
        out.lines.push(Line::default());
        out.lines
            .push(Line::from(Span::styled("(loading findings…)", theme.dim())));
    }
    out
}

impl Section {
    fn push_item(&mut self, spans: Vec<Span<'static>>, item: Item) {
        self.items.push((self.lines.len(), item));
        self.lines.push(Line::from(spans));
    }

    fn finding(&mut self, f: &dovetail_api::research::FindingSummary, theme: &Theme) {
        let spans = vec![
            Span::styled(format!("  F{:<3} ", f.id.0), theme.fg(theme.cite)),
            Span::styled(format!("{:02} {}", f.n, f.label), theme.body()),
            Span::styled(
                format!(
                    "  · {} source{}",
                    f.sources.len(),
                    plural(f.sources.len() as u32)
                ),
                theme.dim(),
            ),
        ];
        let item = Item {
            what: ResearchWhat::Finding(f.id),
            title: format!("F{}", f.id.0),
            path: Some(f.path.clone()),
        };
        self.push_item(spans, item);
    }
}

fn counts(summary: &ResearchSummary) -> String {
    let rounds = summary.rounds.iter().filter(|r| !r.queued).count();
    let mut out = format!(
        "{rounds} round{} · {} finding{} · {} source{}",
        plural(rounds as u32),
        summary.findings,
        plural(summary.findings),
        summary.sources,
        plural(summary.sources),
    );
    if let Some(kind) = summary.kind {
        out.push_str(&format!(" · {}", kind.as_str()));
    }
    out
}

/// What would be lost by archiving `task`: its open or queued research round, in words
/// ("an open research round"), or `None`.
pub fn blocking_round(snap: &Snapshot, task: TaskId) -> Option<&'static str> {
    let summary = snap.research.iter().find(|r| r.task == task)?;
    if summary.open_round().is_some() {
        Some("an open research round")
    } else if summary.queued_rounds().next().is_some() {
        Some("a queued research round")
    } else {
        None
    }
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}
