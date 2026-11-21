//! Reading mode's library (ADR 0018): projects (research ones marked `◇`), the open project's
//! research tasks with their report, rounds (brief and findings), quick findings and sources,
//! then the decision log and the repo's docs.
//!
//! Only the open project's content is in the snapshot. Findings and sources come from
//! `LoadResearch { Listing }` and the docs from `ListDocs`, both asked for when reading mode
//! opens. The Tangents section (ADR 0017, M22) lists every tangent from the snapshot; its
//! answers and sources come from `LoadTangent { Listing }`, and Tangents is pinned last among
//! the projects.
//!
//! `<leader>fr`, or `Enter` on another project's row, browses that project instead (ADR 0018,
//! decided 2026-11-11 later): the library re-roots to its research, decisions and docs, loaded
//! read-only with `LibraryCommand::BrowseProject` and not kept up to date. Its documents are
//! [`ReadTarget::Elsewhere`]. Browsing never opens the project: `<leader>pp` does that.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dovetail_api::{
    Snapshot,
    decision::{Decision, DecisionId},
    project::{ProjectId, ProjectKind},
    task::{Task, TaskId},
};
use dovetail_api::{library::BrowsedProject, research::ResearchSummary};
use dovetail_api::{
    library::DocEntry,
    research::{FindingId, FindingSummary, ResearchListing, ResearchWhat, SourceId, SourceKind},
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::reader::ReadTarget;
use super::{Buffer, Effect, ViewCx, clip_spans, footer, pane, scroll_to, step, title};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;
use crate::theme::Theme;

/// A row's identity, so the cursor and folds survive the list changing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LibKey {
    /// A project.
    Project(ProjectId),
    /// A research task.
    Task(TaskId),
    /// Its report.
    Report(TaskId),
    /// One of its rounds.
    Round(TaskId, u32),
    /// A round's brief.
    Brief(TaskId, u32),
    /// A finding.
    Finding(TaskId, FindingId),
    /// The task's quick findings.
    Quick(TaskId),
    /// The task's sources.
    Sources(TaskId),
    /// A source.
    Source(TaskId, SourceId),
    /// Tangents (ADR 0017): every tangent, whatever project is open.
    Tangents,
    /// One tangent, by its task in Tangents.
    Tangent(TaskId),
    /// One of a tangent's findings (its answers, and its subagents' findings).
    TangentFinding(TaskId, FindingId),
    /// A tangent's sources.
    TangentSources(TaskId),
    /// One of a tangent's sources.
    TangentSource(TaskId, SourceId),
    /// The decision log.
    Decisions,
    /// One decision.
    Decision(DecisionId),
    /// The repo's docs.
    Docs,
    /// One doc.
    Doc(PathBuf),
    /// The line saying the open project has no research yet.
    NoResearch,
    /// The line saying a tangent's answers are being listed.
    TangentLoading(TaskId),
    /// The line saying there are no tangents yet.
    NoTangents,
    /// The line saying a task's findings are being listed.
    Loading(TaskId),
    /// The line saying a browsed project is being loaded.
    BrowseLoading,
}

impl LibKey {
    /// Whether a group row starts open.
    fn open_by_default(&self) -> bool {
        matches!(self, LibKey::Task(_) | LibKey::Tangents)
    }
}

struct Row {
    key: LibKey,
    depth: usize,
    /// `Some(open)` for a row that folds.
    fold: Option<bool>,
    spans: Vec<Span<'static>>,
}

/// Another project the library shows instead of the open one.
struct Browsing {
    id: ProjectId,
    name: String,
    /// `None` until the core's answer arrives.
    loaded: Option<Browsed>,
}

/// A browsed project's library, as the core loaded it.
struct Browsed {
    kind: ProjectKind,
    tasks: Vec<Task>,
    decisions: Vec<Decision>,
    research: Vec<ResearchSummary>,
    listings: HashMap<TaskId, ResearchListing>,
    docs: Vec<DocEntry>,
}

/// What the rows are made from: the open project's snapshot and listings, or a browsed
/// project's copy.
struct Source<'a> {
    kind: ProjectKind,
    tasks: &'a [Task],
    decisions: &'a [Decision],
    research: &'a [ResearchSummary],
    listings: &'a HashMap<TaskId, ResearchListing>,
    docs: Option<&'a [DocEntry]>,
    /// The browsed project, when it is not the open one.
    elsewhere: Option<&'a ProjectId>,
}

/// The library buffer.
#[derive(Default)]
pub struct LibraryBuffer {
    selected: Option<LibKey>,
    /// Rows whose fold differs from their default.
    toggled: HashSet<LibKey>,
    listings: HashMap<TaskId, ResearchListing>,
    /// Tangents' findings and sources, by tangent (their ids are Tangents' own).
    tangent_listings: HashMap<TaskId, ResearchListing>,
    docs: Option<Vec<DocEntry>>,
    browsing: Option<Browsing>,
    offset: usize,
}

impl LibraryBuffer {
    /// Shows `id` (named `name`) instead of the open project, loading until
    /// [`Self::receive_browsed`]; the cursor goes to its row. Folds start over.
    pub fn browse(&mut self, id: ProjectId, name: String) {
        self.toggled.clear();
        self.selected = Some(LibKey::Project(id.clone()));
        self.offset = 0;
        self.browsing = Some(Browsing {
            id,
            name,
            loaded: None,
        });
    }

    /// Back to the open project; the cursor goes to its row when given.
    pub fn stop_browsing(&mut self, open: Option<&ProjectId>) {
        if self.browsing.take().is_some() {
            self.toggled.clear();
            self.offset = 0;
            self.selected = open.map(|id| LibKey::Project(id.clone()));
        }
    }

    /// The project browsed, if not the open one.
    pub fn browsing(&self) -> Option<&ProjectId> {
        self.browsing.as_ref().map(|b| &b.id)
    }

    /// Whether the browsed project is still loading.
    pub fn browse_loading(&self) -> bool {
        self.browsing.as_ref().is_some_and(|b| b.loaded.is_none())
    }

    /// Keeps a browsed project's library, if it is the one browsed.
    pub fn receive_browsed(&mut self, browsed: &BrowsedProject) {
        let Some(b) = self
            .browsing
            .as_mut()
            .filter(|b| b.id == browsed.project.id)
        else {
            return;
        };
        b.name = browsed.project.name.clone();
        b.loaded = Some(Browsed {
            kind: browsed.project.kind,
            tasks: browsed.tasks.clone(),
            decisions: browsed.decisions.clone(),
            research: browsed.research.clone(),
            listings: browsed
                .listings
                .iter()
                .map(|l| (l.task, l.listing.clone()))
                .collect(),
            docs: browsed.docs.clone(),
        });
    }

    /// A browsed project's findings and sources of `task`, if `project` is the one browsed.
    pub fn browsed_listing(&self, project: &ProjectId, task: TaskId) -> Option<&ResearchListing> {
        let b = self.browsing.as_ref().filter(|b| &b.id == project)?;
        b.loaded.as_ref()?.listings.get(&task)
    }

    /// The decision-log entry `id` of the browsed project `project`, as markdown.
    pub fn browsed_decision(
        &self,
        project: &ProjectId,
        id: DecisionId,
        tz: &jiff::tz::TimeZone,
    ) -> Option<String> {
        let b = self.browsing.as_ref().filter(|b| &b.id == project)?;
        let loaded = b.loaded.as_ref()?;
        decision_markdown(&loaded.decisions, &loaded.tasks, id, tz)
    }

    fn source<'a>(&'a self, snap: &'a Snapshot) -> Option<Source<'a>> {
        match &self.browsing {
            Some(b) => {
                let l = b.loaded.as_ref()?;
                Some(Source {
                    kind: l.kind,
                    tasks: &l.tasks,
                    decisions: &l.decisions,
                    research: &l.research,
                    listings: &l.listings,
                    docs: Some(&l.docs),
                    elsewhere: Some(&b.id),
                })
            }
            None => Some(Source {
                kind: snap.project.as_ref().map_or(ProjectKind::Code, |p| p.kind),
                tasks: &snap.tasks,
                decisions: &snap.decisions,
                research: &snap.research,
                listings: &self.listings,
                docs: self.docs.as_deref(),
                elsewhere: None,
            }),
        }
    }

    /// Keeps a task's findings and sources, as the core listed them.
    pub fn receive_listing(&mut self, task: TaskId, listing: ResearchListing) {
        self.listings.insert(task, listing);
    }

    /// Keeps a tangent's findings and sources, as the core listed them.
    pub fn receive_tangent_listing(&mut self, tangent: TaskId, listing: ResearchListing) {
        self.tangent_listings.insert(tangent, listing);
    }

    /// A tangent's findings and sources, if listed.
    pub fn tangent_listing(&self, tangent: TaskId) -> Option<&ResearchListing> {
        self.tangent_listings.get(&tangent)
    }

    /// A tangent's latest answer: its newest finding labelled `answer`, else its newest
    /// finding. `None` until its listing arrived, or when it has none.
    pub fn latest_answer(&self, tangent: TaskId) -> Option<&FindingSummary> {
        let findings = &self.tangent_listing(tangent)?.findings;
        findings
            .iter()
            .filter(|f| f.label == ANSWER_LABEL)
            .max_by_key(|f| f.id)
            .or_else(|| findings.iter().max_by_key(|f| f.id))
    }

    /// Keeps the repo's docs, as the core listed them.
    pub fn receive_docs(&mut self, docs: Vec<DocEntry>) {
        self.docs = Some(docs);
    }

    /// A task's findings and sources, if listed.
    pub fn listing(&self, task: TaskId) -> Option<&ResearchListing> {
        self.listings.get(&task)
    }

    /// The finding `id` of `task`, if listed.
    pub fn finding(&self, task: TaskId, id: FindingId) -> Option<&FindingSummary> {
        self.listing(task)?.findings.iter().find(|f| f.id == id)
    }

    /// The row under the cursor.
    pub fn selected(&self) -> Option<&LibKey> {
        self.selected.as_ref()
    }

    /// The research task of the row under the cursor, if it belongs to one.
    pub fn selected_task(&self) -> Option<TaskId> {
        match self.selected.as_ref()? {
            LibKey::Task(t)
            | LibKey::Report(t)
            | LibKey::Round(t, _)
            | LibKey::Brief(t, _)
            | LibKey::Finding(t, _)
            | LibKey::Quick(t)
            | LibKey::Sources(t)
            | LibKey::Source(t, _)
            | LibKey::Loading(t) => Some(*t),
            _ => None,
        }
    }

    /// Puts the cursor on a row (opening its parents is the caller's business).
    pub fn select(&mut self, key: LibKey) {
        self.selected = Some(key);
    }

    fn is_open(&self, key: &LibKey) -> bool {
        key.open_by_default() != self.toggled.contains(key)
    }

    fn toggle(&mut self, key: &LibKey) {
        if !self.toggled.remove(key) {
            self.toggled.insert(key.clone());
        }
    }

    fn rows(&self, snap: &Snapshot, theme: &Theme) -> Vec<Row> {
        let mut rows = Vec::new();
        let open_id = snap.project.as_ref().map(|p| p.id.clone());
        let browsed = self.browsing.as_ref().map(|b| &b.id);
        // The project shown: the browsed one, else the open one.
        let shown = browsed.or(open_id.as_ref());
        let source = self.source(snap);
        let mut projects: Vec<_> = snap.project.iter().collect();
        projects.extend(
            crate::switcher::projects_in_order(snap)
                .into_iter()
                .filter(|p| Some(&p.id) != open_id.as_ref()),
        );
        for project in projects {
            let is_open = Some(&project.id) == open_id.as_ref();
            let is_shown = Some(&project.id) == shown;
            let key = LibKey::Project(project.id.clone());
            let research = project.kind == ProjectKind::Research;
            let mut spans = Vec::new();
            if crate::switcher::is_tangents(project) {
                let marker = crate::switcher::TANGENT_MARKER;
                spans.push(Span::styled(format!("{marker} "), theme.fg(theme.italic)));
            } else if research {
                spans.push(Span::styled("◇ ", theme.fg(theme.italic)));
            }
            let name_style = match (is_shown, browsed.is_some()) {
                // The browsed project's row is highlighted.
                (true, true) => theme.accent().add_modifier(Modifier::BOLD),
                (true, false) => theme.body().add_modifier(Modifier::BOLD),
                _ => theme.body(),
            };
            spans.push(Span::styled(project.name.clone(), name_style));
            let tag = match (is_open, is_shown) {
                (true, true) => " · open",
                (true, false) => " · open · ⏎ back",
                (false, true) => " · not open · read-only",
                (false, false) => " · ⏎ browses",
            };
            spans.push(Span::styled(tag, theme.dim()));
            // The project shown is unfolded unless folded; the others have nothing to show here.
            let unfolded = is_shown && !self.toggled.contains(&key);
            rows.push(Row {
                key,
                depth: 0,
                fold: is_shown.then_some(unfolded),
                spans,
            });
            if unfolded {
                match &source {
                    Some(source) => self.research_rows(source, theme, &mut rows),
                    None => rows.push(Row {
                        key: LibKey::BrowseLoading,
                        depth: 1,
                        fold: None,
                        spans: vec![Span::styled("(loading…)", theme.dim())],
                    }),
                }
            }
        }
        let Some(source) = source else {
            return rows;
        };

        // Tangents (ADR 0017), whatever project is open; when Tangents is the open project its
        // tangents are its research tasks above.
        let in_tangents = snap
            .projects
            .iter()
            .chain(snap.project.iter())
            .find(|p| Some(&p.id) == shown)
            .is_some_and(crate::switcher::is_tangents);
        if !in_tangents {
            self.tangent_rows(snap, theme, &mut rows);
        }

        let key = LibKey::Decisions;
        let open = self.is_open(&key);
        rows.push(Row {
            key,
            depth: 0,
            fold: Some(open),
            spans: vec![Span::styled(
                format!("Decisions ({})", source.decisions.len()),
                theme.body().add_modifier(Modifier::BOLD),
            )],
        });
        if open {
            let superseded: HashSet<DecisionId> = source
                .decisions
                .iter()
                .filter_map(|d| d.supersedes)
                .collect();
            for d in source.decisions.iter().rev() {
                let mut spans = vec![
                    Span::styled(format!("#{} ", d.task.0), theme.dim()),
                    Span::styled(
                        d.text.clone(),
                        if superseded.contains(&d.id) {
                            theme.dim()
                        } else {
                            theme.body()
                        },
                    ),
                ];
                if !d.evidence.is_empty() {
                    spans.push(Span::styled(
                        format!(" [{}]", d.evidence.len()),
                        theme.fg(theme.cite),
                    ));
                }
                rows.push(Row {
                    key: LibKey::Decision(d.id),
                    depth: 1,
                    fold: None,
                    spans,
                });
            }
        }

        // A research project has no repo, so no docs (decided 2026-11-11).
        if source.kind != ProjectKind::Research {
            let key = LibKey::Docs;
            let open = self.is_open(&key);
            let count = source
                .docs
                .map_or_else(|| "…".to_string(), |d| d.len().to_string());
            rows.push(Row {
                key,
                depth: 0,
                fold: Some(open),
                spans: vec![Span::styled(
                    format!("Docs · ADRs ({count})"),
                    theme.body().add_modifier(Modifier::BOLD),
                )],
            });
            if open {
                for doc in source.docs.into_iter().flatten() {
                    rows.push(Row {
                        key: LibKey::Doc(doc.path.clone()),
                        depth: 1,
                        fold: None,
                        spans: vec![
                            Span::styled(doc.title.clone(), theme.body()),
                            Span::styled(format!("  {}", doc.path.display()), theme.dim()),
                        ],
                    });
                }
            }
        }
        rows
    }

    fn research_rows(&self, source: &Source, theme: &Theme, rows: &mut Vec<Row>) {
        let mut summaries: Vec<_> = source.research.iter().collect();
        summaries.sort_by_key(|s| s.task);
        if summaries.is_empty() {
            rows.push(Row {
                key: LibKey::NoResearch,
                depth: 1,
                fold: None,
                spans: vec![Span::styled(
                    "no research yet: :round <task> <brief>",
                    theme.dim(),
                )],
            });
        }
        for summary in summaries {
            let task = summary.task;
            let name = source
                .tasks
                .iter()
                .find(|t| t.id == task)
                .map_or_else(|| "task".to_string(), |t| t.title.clone());
            let key = LibKey::Task(task);
            let open = self.is_open(&key);
            rows.push(Row {
                key,
                depth: 1,
                fold: Some(open),
                spans: vec![
                    Span::styled(format!("#{} ", task.0), theme.dim()),
                    Span::styled(name, theme.body()),
                ],
            });
            if !open {
                continue;
            }
            rows.push(Row {
                key: LibKey::Report(task),
                depth: 2,
                fold: None,
                spans: match summary.report_ms {
                    Some(_) => vec![
                        Span::styled("● ", theme.fg(theme.cite)),
                        Span::styled("report", theme.body().add_modifier(Modifier::BOLD)),
                    ],
                    None => vec![Span::styled("○ no report yet", theme.dim())],
                },
            });
            let listing = source.listings.get(&task);
            for round in &summary.rounds {
                let key = LibKey::Round(task, round.n);
                let open = self.is_open(&key);
                let state = if round.queued {
                    "queued"
                } else if round.closed_ms.is_some() {
                    "closed"
                } else {
                    "open"
                };
                rows.push(Row {
                    key,
                    depth: 2,
                    fold: Some(open),
                    spans: vec![
                        Span::styled(format!("round {} ", round.n), theme.body()),
                        Span::styled(format!("· {state} · {}", round.findings), theme.dim()),
                    ],
                });
                if !open {
                    continue;
                }
                rows.push(Row {
                    key: LibKey::Brief(task, round.n),
                    depth: 3,
                    fold: None,
                    spans: vec![Span::styled("brief", theme.body())],
                });
                for f in listing
                    .iter()
                    .flat_map(|l| &l.findings)
                    .filter(|f| f.round == Some(round.n))
                {
                    rows.push(finding_row(task, f, 3, theme));
                }
            }
            let Some(listing) = listing else {
                if summary.findings > 0 || summary.sources > 0 {
                    rows.push(Row {
                        key: LibKey::Loading(task),
                        depth: 2,
                        fold: None,
                        spans: vec![Span::styled("(loading findings…)", theme.dim())],
                    });
                }
                continue;
            };
            let quick: Vec<_> = listing
                .findings
                .iter()
                .filter(|f| f.round.is_none())
                .collect();
            if !quick.is_empty() {
                let key = LibKey::Quick(task);
                let open = self.is_open(&key);
                rows.push(Row {
                    key,
                    depth: 2,
                    fold: Some(open),
                    spans: vec![Span::styled(
                        format!("quick findings ({})", quick.len()),
                        theme.body(),
                    )],
                });
                if open {
                    for f in quick {
                        rows.push(finding_row(task, f, 3, theme));
                    }
                }
            }
            if !listing.sources.is_empty() {
                let key = LibKey::Sources(task);
                let open = self.is_open(&key);
                rows.push(Row {
                    key,
                    depth: 2,
                    fold: Some(open),
                    spans: vec![Span::styled(
                        format!("sources ({})", listing.sources.len()),
                        theme.body(),
                    )],
                });
                if open {
                    for s in &listing.sources {
                        let kind = source_kind(s.kind);
                        rows.push(Row {
                            key: LibKey::Source(task, s.id),
                            depth: 3,
                            fold: None,
                            spans: vec![
                                Span::styled(format!("S{} ", s.id.0), theme.fg(theme.cite)),
                                Span::styled(format!("{kind} "), theme.dim()),
                                Span::styled(s.locator.clone(), theme.body()),
                            ],
                        });
                    }
                }
            }
        }
    }

    /// The Tangents section: every tangent, its answers and findings, and its sources.
    fn tangent_rows(&self, snap: &Snapshot, theme: &Theme, rows: &mut Vec<Row>) {
        let key = LibKey::Tangents;
        let open = self.is_open(&key);
        let marker = crate::switcher::TANGENT_MARKER;
        rows.push(Row {
            key,
            depth: 0,
            fold: Some(open),
            spans: vec![Span::styled(
                format!("{marker} Tangents ({})", snap.tangents.len()),
                theme.body().add_modifier(Modifier::BOLD),
            )],
        });
        if !open {
            return;
        }
        if snap.tangents.is_empty() {
            rows.push(Row {
                key: LibKey::NoTangents,
                depth: 1,
                fold: None,
                spans: vec![Span::styled(
                    "no tangents yet: T in the side panel or :tangent <question>",
                    theme.dim(),
                )],
            });
        }
        for t in &snap.tangents {
            let task = t.task.id;
            let key = LibKey::Tangent(task);
            let open = self.is_open(&key);
            let mut spans = vec![Span::styled(t.task.title.clone(), theme.body())];
            if t.is_new() {
                spans.push(Span::styled(" · new", theme.fg(theme.needs_you)));
            }
            if t.research.is_none() {
                spans.push(Span::styled(" · no answer yet", theme.dim()));
            }
            rows.push(Row {
                key,
                depth: 1,
                fold: t.research.is_some().then_some(open),
                spans,
            });
            if !open || t.research.is_none() {
                continue;
            }
            let Some(listing) = self.tangent_listings.get(&task) else {
                rows.push(Row {
                    key: LibKey::TangentLoading(task),
                    depth: 2,
                    fold: None,
                    spans: vec![Span::styled("(loading answers…)", theme.dim())],
                });
                continue;
            };
            for f in &listing.findings {
                rows.push(Row {
                    key: LibKey::TangentFinding(task, f.id),
                    depth: 2,
                    fold: None,
                    spans: vec![
                        Span::styled(format!("F{} ", f.id.0), theme.fg(theme.cite)),
                        Span::styled(format!("{:02} {}", f.n, f.label), theme.body()),
                    ],
                });
            }
            if !listing.sources.is_empty() {
                let key = LibKey::TangentSources(task);
                let open = self.is_open(&key);
                rows.push(Row {
                    key,
                    depth: 2,
                    fold: Some(open),
                    spans: vec![Span::styled(
                        format!("sources ({})", listing.sources.len()),
                        theme.body(),
                    )],
                });
                if open {
                    for s in &listing.sources {
                        rows.push(Row {
                            key: LibKey::TangentSource(task, s.id),
                            depth: 3,
                            fold: None,
                            spans: vec![
                                Span::styled(format!("S{} ", s.id.0), theme.fg(theme.cite)),
                                Span::styled(format!("{} ", source_kind(s.kind)), theme.dim()),
                                Span::styled(s.locator.clone(), theme.body()),
                            ],
                        });
                    }
                }
            }
        }
    }

    fn index(&self, rows: &[Row]) -> usize {
        self.selected
            .as_ref()
            .and_then(|k| rows.iter().position(|r| &r.key == k))
            .unwrap_or(0)
    }

    /// What `Enter` on `key` opens, with its pane title. A browsed project's document is
    /// [`ReadTarget::Elsewhere`], titled with the project's name.
    fn target(
        &self,
        key: &LibKey,
        snap: &Snapshot,
    ) -> Option<(ReadTarget, Option<TaskId>, String)> {
        // Tangents are the same whatever the library shows: never a browsed project's.
        match key {
            // A tangent's documents carry no project task: their citations resolve against the
            // tangent (`ReadTarget::Tangent`).
            LibKey::TangentFinding(task, id) => {
                let label = self
                    .tangent_listing(*task)
                    .and_then(|l| l.findings.iter().find(|f| f.id == *id))
                    .map_or_else(String::new, |f| format!(" · {}", f.label));
                return Some((
                    ReadTarget::Tangent {
                        task: *task,
                        what: ResearchWhat::Finding(*id),
                    },
                    None,
                    format!("tangent F{}{label}", id.0),
                ));
            }
            LibKey::TangentSource(task, id) => {
                let locator = self
                    .tangent_listing(*task)
                    .and_then(|l| l.sources.iter().find(|s| s.id == *id))
                    .map_or_else(String::new, |s| format!(" · {}", s.locator));
                return Some((
                    ReadTarget::Tangent {
                        task: *task,
                        what: ResearchWhat::Source(*id),
                    },
                    None,
                    format!("tangent S{}{locator}", id.0),
                ));
            }
            _ => {}
        }
        let source = self.source(snap)?;
        let (target, task, title) = Self::target_in(key, &source)?;
        match (source.elsewhere, &self.browsing) {
            (Some(project), Some(b)) => Some((
                ReadTarget::Elsewhere(project.clone(), Box::new(target)),
                task,
                format!("{} · {title}", b.name),
            )),
            _ => Some((target, task, title)),
        }
    }

    fn target_in(key: &LibKey, source: &Source) -> Option<(ReadTarget, Option<TaskId>, String)> {
        let research = |task: TaskId, what: ResearchWhat, title: String| {
            Some((ReadTarget::Research { task, what }, Some(task), title))
        };
        let listing = |task: TaskId| source.listings.get(&task);
        match key {
            LibKey::Report(task) => {
                let has = source
                    .research
                    .iter()
                    .any(|r| r.task == *task && r.report_ms.is_some());
                if !has {
                    return None;
                }
                research(*task, ResearchWhat::Report, format!("#{} report", task.0))
            }
            LibKey::Brief(task, n) => research(
                *task,
                ResearchWhat::Brief { round: *n },
                format!("#{} round {n} brief", task.0),
            ),
            LibKey::Finding(task, id) => {
                let label = listing(*task)
                    .and_then(|l| l.findings.iter().find(|f| f.id == *id))
                    .map_or_else(String::new, |f| format!(" · {}", f.label));
                research(
                    *task,
                    ResearchWhat::Finding(*id),
                    format!("F{}{label}", id.0),
                )
            }
            LibKey::Source(task, id) => {
                let locator = listing(*task)
                    .and_then(|l| l.sources.iter().find(|s| s.id == *id))
                    .map_or_else(String::new, |s| format!(" · {}", s.locator));
                research(
                    *task,
                    ResearchWhat::Source(*id),
                    format!("S{}{locator}", id.0),
                )
            }
            LibKey::Decision(id) => {
                let d = source.decisions.iter().find(|d| d.id == *id)?;
                Some((
                    ReadTarget::Decision(*id),
                    Some(d.task),
                    format!("decision #{}", id.0),
                ))
            }
            LibKey::Doc(path) => Some((
                ReadTarget::Doc(path.clone()),
                None,
                path.display().to_string(),
            )),
            _ => None,
        }
    }
}

/// The label of a tangent's answers (one finding per turn; the core's `TURN_LABEL`).
const ANSWER_LABEL: &str = "answer";

/// A source's kind in a row.
fn source_kind(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Web => "web",
        SourceKind::File => "file",
        SourceKind::Search => "search",
    }
}

fn finding_row(task: TaskId, f: &FindingSummary, depth: usize, theme: &Theme) -> Row {
    Row {
        key: LibKey::Finding(task, f.id),
        depth,
        fold: None,
        spans: vec![
            Span::styled(format!("F{} ", f.id.0), theme.fg(theme.cite)),
            Span::styled(format!("{:02} {}", f.n, f.label), theme.body()),
        ],
    }
}

impl Buffer for LibraryBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Library
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let heading = match &self.browsing {
            Some(b) => format!("library · {} · not open", b.name),
            None => "library".to_string(),
        };
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        let rows = self.rows(cx.snap, theme);
        let at = self.index(&rows);
        if self.selected.is_none() {
            self.selected = rows.first().map(|r| r.key.clone());
        }
        let height = usize::from(inner.height.saturating_sub(1)).max(1);
        self.offset = scroll_to(self.offset.min(rows.len()), at, at, height);
        let width = usize::from(inner.width);
        for (n, row) in rows.iter().enumerate().skip(self.offset).take(height) {
            let mut spans = vec![Span::raw("  ".repeat(row.depth))];
            spans.push(match row.fold {
                Some(true) => Span::styled("▾ ", theme.dim()),
                Some(false) => Span::styled("▸ ", theme.dim()),
                None => Span::raw("  "),
            });
            spans.extend(row.spans.iter().cloned());
            let mut line = Line::from(clip_spans(spans, width));
            if n == at && focused {
                line = line.style(theme.selected());
            }
            let rect = Rect {
                y: inner.y + (n - self.offset) as u16,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        footer(
            frame,
            inner,
            &[
                "⏎ read · o fold · <leader>fr browse · ⎋ leave",
                "⏎ read · o fold",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let rows = self.rows(cx.snap, cx.theme);
        let at = self.index(&rows);
        let len = rows.len();
        let go = |delta: isize, this: &mut Self| {
            let i = step(at, len, delta);
            this.selected = rows.get(i).map(|r| r.key.clone());
        };
        match action {
            Action::MoveDown => go(1, self),
            Action::MoveUp => go(-1, self),
            Action::PageDown => go(10, self),
            Action::PageUp => go(-10, self),
            Action::GoTop => go(-(len as isize), self),
            Action::GoBottom => go(len as isize, self),
            Action::LibraryToggleFold => {
                if let Some(row) = rows.get(at)
                    && row.fold.is_some()
                {
                    let key = row.key.clone();
                    self.toggle(&key);
                }
            }
            Action::LibraryOpen => {
                let Some(row) = rows.get(at) else {
                    return true;
                };
                let key = row.key.clone();
                // Another project's row browses it; the open one's, while browsing, comes back
                // (ADR 0018, decided 2026-11-11 later). Only `<leader>pp` switches projects.
                if let LibKey::Project(id) = &key
                    && row.fold.is_none()
                {
                    effects.push(Effect::BrowseProject(id.clone()));
                    return true;
                }
                // A tangent opens its latest answer (fetched first when not listed yet).
                if let LibKey::Tangent(tangent) = &key {
                    let answered = cx
                        .snap
                        .tangents
                        .iter()
                        .any(|t| t.task.id == *tangent && t.research.is_some());
                    effects.push(if answered {
                        Effect::ReadTangent(*tangent)
                    } else {
                        Effect::Note("no answer yet: it shows here when the turn ends".into())
                    });
                    return true;
                }
                // A task opens its report when it has one; other groups fold.
                if let LibKey::Task(task) = &key
                    && let Some((target, task, title)) =
                        self.target(&LibKey::Report(*task), cx.snap)
                {
                    effects.push(Effect::Read {
                        target,
                        task,
                        title,
                    });
                    return true;
                }
                if row.fold.is_some() {
                    self.toggle(&key);
                    return true;
                }
                match self.target(&key, cx.snap) {
                    Some((target, task, title)) => effects.push(Effect::Read {
                        target,
                        task,
                        title,
                    }),
                    None => effects.push(Effect::Note("nothing to read here".into())),
                }
            }
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        self.rows(cx.snap, cx.theme)
            .iter()
            .map(|r| r.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        self.index(&self.rows(cx.snap, cx.theme))
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, _effects: &mut Vec<Effect>) {
        let rows = self.rows(cx.snap, cx.theme);
        if let Some(r) = rows.get(row) {
            self.selected = Some(r.key.clone());
        }
    }
}

/// The markdown of one decision-log entry, for the reading column: what, why, the options not
/// taken, and its evidence as citation markers that `gd` follows.
pub fn decision_markdown(
    decisions: &[Decision],
    tasks: &[Task],
    id: DecisionId,
    tz: &jiff::tz::TimeZone,
) -> Option<String> {
    let d = decisions.iter().find(|d| d.id == id)?;
    let task = tasks
        .iter()
        .find(|t| t.id == d.task)
        .map_or_else(String::new, |t| format!(" {}", t.title));
    let by = match &d.by {
        dovetail_api::decision::DecisionAuthor::User => "you".to_string(),
        dovetail_api::decision::DecisionAuthor::Agent(a) => a.0.clone(),
    };
    let mut md = format!(
        "# {}\n\nTask #{}{task} · {} · by {by}\n",
        d.text,
        d.task.0,
        fmt::when(d.at_ms, tz)
    );
    if let Some(why) = &d.why {
        md.push_str(&format!("\n**Why:** {why}\n"));
    }
    if !d.alternatives.is_empty() {
        md.push_str("\n## Not taken\n\n");
        for a in &d.alternatives {
            match &a.why_not {
                Some(why) => md.push_str(&format!("- {}: {why}\n", a.option)),
                None => md.push_str(&format!("- {}\n", a.option)),
            }
        }
    }
    if !d.evidence.is_empty() {
        md.push_str("\n## Evidence\n\n");
        let marks: Vec<String> = d.evidence.iter().map(|e| format!("[{e}]")).collect();
        md.push_str(&marks.join(" "));
        md.push_str("\n\n`]s` selects one, `gd` opens it.\n");
    }
    if let Some(old) = d.supersedes {
        md.push_str(&format!("\nReplaces decision #{}.\n", old.0));
    }
    Some(md)
}
