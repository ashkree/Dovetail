//! Reading mode (ADR 0018, decided 2026-11-11): the second layout preset, with the library,
//! the reading column and Ask in place of Layout B, which comes back as it was.
//!
//! The app asks the core for what the library lists (`LoadResearch { Listing }` per research
//! task, `ListDocs`) when reading mode opens, opens documents in the reading column, resolves
//! `gd`/`gx` against the listings, and starts Ask through `Command::StartAsk`. Opening a web
//! page is the core's job (`Command::OpenExternal`).

use dovetail_api::research::{
    BrowseWhat, Evidence, ResearchContent, ResearchDocument, ResearchWhat, SourceKind,
};
use dovetail_api::{AgentId, AgentKind, Command, Event, ProjectId, ProjectKind, TaskId};

use super::overlay::{Overlay, Purpose};
use super::{App, ChatSite};
use crate::buffers::library::decision_markdown;
use crate::buffers::reader::Opened;
use crate::buffers::{ReadTarget, ask_agent};
use crate::mode::Mode;
use crate::picker::Picker;
use crate::region::{Preset, RegionId};
use crate::switcher;

/// What `i` says while another project is browsed: Ask reads the open project only.
pub(crate) const ASK_OFF_WHILE_BROWSING: &str =
    "Ask works in the open project; switch with <leader>pp to ask about this.";

impl App {
    /// Whether reading mode is on.
    pub fn reading_mode(&self) -> bool {
        self.regions.preset() == Preset::Reading
    }

    /// `<leader>r`.
    pub(super) fn toggle_reading_mode(&mut self) {
        if self.reading_mode() {
            self.leave_reading_mode();
        } else {
            self.enter_reading_mode();
        }
    }

    pub(super) fn enter_reading_mode(&mut self) {
        // The task in focus in Layout B, to open its report straight away.
        let focused = self.focused_task();
        // Browsing is not remembered: reading mode starts on the open project.
        let open = self.snap.project.as_ref().map(|p| p.id.clone());
        self.buffers.library.stop_browsing(open.as_ref());
        self.buffers.reader.forget_elsewhere();
        self.park_chat();
        self.focus_mode = false;
        self.regions.set_preset(Preset::Reading);
        self.mode = Mode::Normal;
        self.request_library();
        if self.buffers.reader.opened().is_none()
            && let Some(task) = focused
            && self
                .snap
                .research
                .iter()
                .any(|r| r.task == task && r.report_ms.is_some())
        {
            self.read(
                ReadTarget::Research {
                    task,
                    what: ResearchWhat::Report,
                },
                Some(task),
                format!("#{} report", task.0),
            );
            self.buffers
                .library
                .select(crate::buffers::library::LibKey::Report(task));
            return;
        }
        if self.buffers.reader.opened().is_none() {
            self.regions.set_focus(RegionId::Library);
        }
    }

    /// `Esc` in Normal mode, or `<leader>r` again: Layout B as it was.
    pub(super) fn leave_reading_mode(&mut self) {
        if self.chat_site == ChatSite::Ask {
            self.park_chat();
        }
        self.mode = Mode::Normal;
        self.regions.set_preset(Preset::LayoutB);
    }

    /// Closes whichever chat bar is open, keeping its draft.
    pub(super) fn park_chat(&mut self) {
        self.chat_view_mut().park_chat();
        if self.mode == Mode::Insert && self.input.is_none() {
            self.mode = Mode::Normal;
        }
        self.chat_site = ChatSite::Main;
    }

    /// What the library lists: every research task's findings and sources, and the repo docs.
    fn request_library(&mut self) {
        let tasks: Vec<TaskId> = self.snap.research.iter().map(|r| r.task).collect();
        for task in tasks {
            self.outbox.push(Command::LoadResearch {
                task,
                what: ResearchWhat::Listing,
            });
        }
        let research = self
            .snap
            .project
            .as_ref()
            .is_some_and(|p| p.kind == ProjectKind::Research);
        if !research && self.snap.project.is_some() {
            self.outbox.push(Command::ListDocs);
        }
        // The Tangents section's answers and sources (ADR 0017).
        let tangents: Vec<TaskId> = self
            .snap
            .tangents
            .iter()
            .filter(|t| t.research.is_some())
            .map(|t| t.task.id)
            .collect();
        for tangent in tangents {
            self.outbox.push(Command::LoadTangent {
                tangent,
                what: ResearchWhat::Listing,
            });
        }
    }

    /// `<leader>fr`: every registered project, as `<leader>pp` lists them; the one picked is
    /// browsed (ADR 0018, decided 2026-11-11 later).
    pub(super) fn open_browse_picker(&mut self) {
        let items = switcher::project_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some("no other projects to browse".into());
            return;
        }
        self.open_overlay(Overlay::Picker(
            Picker::new("Browse library", items),
            Purpose::Browse,
        ));
    }

    /// Shows `id`'s library, read-only, loaded now (a project browsed again is loaded again);
    /// the open project's comes back. The open project never changes: `<leader>pp` does that.
    pub(super) fn browse_project(&mut self, id: ProjectId) {
        if self.snap.project.as_ref().is_some_and(|p| p.id == id) {
            self.buffers.library.stop_browsing(Some(&id));
            self.note = None;
        } else {
            let Some(project) = self.snap.projects.iter().find(|p| p.id == id) else {
                return;
            };
            self.note = Some(format!("loading {}…", project.name));
            self.buffers
                .library
                .browse(id.clone(), project.name.clone());
            self.outbox.push(Command::BrowseProject { id });
        }
        self.regions.set_focus(RegionId::Library);
    }

    /// Core answers reading mode uses: listings, docs, documents, and a new Ask session.
    /// Called before the event is applied.
    pub(super) fn reading_event(&mut self, event: &Event) {
        match event {
            Event::ProjectBrowsed(browsed) => {
                self.buffers.library.receive_browsed(browsed);
                if self
                    .note
                    .as_deref()
                    .is_some_and(|n| n.starts_with("loading "))
                {
                    self.note = None;
                }
            }
            // A browse that failed goes back to the open project; the failure is shown.
            Event::CommandFailed { .. } if self.buffers.library.browse_loading() => {
                let open = self.snap.project.as_ref().map(|p| p.id.clone());
                self.buffers.library.stop_browsing(open.as_ref());
            }
            Event::ResearchLoaded {
                task,
                content: ResearchContent::Listing(listing),
                ..
            } => self.buffers.library.receive_listing(*task, listing.clone()),
            Event::DocsListed { docs } => self.buffers.library.receive_docs(docs.clone()),
            Event::TangentLoaded {
                tangent,
                content: ResearchContent::Listing(listing),
                ..
            } => {
                self.buffers
                    .library
                    .receive_tangent_listing(*tangent, listing.clone());
                if self.pending_tangent_read == Some(*tangent) {
                    self.pending_tangent_read = None;
                    self.read_latest_answer(*tangent);
                }
            }
            // A new answer: list it again, so the library and `gd` see it.
            Event::TangentUpserted(t) if self.reading_mode() && t.research.is_some() => {
                self.outbox.push(Command::LoadTangent {
                    tangent: t.task.id,
                    what: ResearchWhat::Listing,
                });
            }
            Event::ResearchUpserted(summary) if self.reading_mode() => {
                self.outbox.push(Command::LoadResearch {
                    task: summary.task,
                    what: ResearchWhat::Listing,
                });
                // A new report replaces the one being read.
                let before = self
                    .snap
                    .research
                    .iter()
                    .find(|r| r.task == summary.task)
                    .and_then(|r| r.report_ms);
                let reading_it = self.buffers.reader.opened().is_some_and(|o| {
                    o.target
                        == ReadTarget::Research {
                            task: summary.task,
                            what: ResearchWhat::Report,
                        }
                });
                if reading_it && summary.report_ms.is_some() && summary.report_ms != before {
                    let title = format!("#{} report", summary.task.0);
                    self.read(
                        ReadTarget::Research {
                            task: summary.task,
                            what: ResearchWhat::Report,
                        },
                        Some(summary.task),
                        title,
                    );
                }
            }
            _ => {}
        }
        if self.buffers.reader.on_event(event) {
            self.note = None;
            // Not out of Ask's chat bar while typing there.
            let typing = self.chat_site == ChatSite::Ask && self.mode == Mode::Insert;
            if self.reading_mode() && !typing {
                self.regions.set_focus(RegionId::Reader);
            }
        }
    }

    /// After an event is applied: a new Ask session that was asked for opens its chat bar.
    pub(super) fn reading_settle(&mut self, event: &Event) {
        if let Event::AgentUpserted(agent) = event
            && agent.kind == AgentKind::Ask
            && agent.task.is_some()
            && self.pending_ask == agent.task
        {
            self.pending_ask = None;
            if self.reading_mode() {
                let id = agent.id.clone();
                self.buffers.ask.follow(agent.task, &self.snap);
                self.start_ask_chat(id);
            }
        }
    }

    /// Opens a document in the reading column; research documents and docs are fetched first.
    pub(super) fn read(&mut self, target: ReadTarget, task: Option<TaskId>, title: String) {
        match &target {
            ReadTarget::Research { task: t, what } => {
                self.outbox.push(Command::LoadResearch {
                    task: *t,
                    what: what.clone(),
                });
                self.buffers.reader.await_target(target, task, title);
                self.note = Some("loading…".into());
            }
            ReadTarget::Tangent { task: t, what } => {
                self.outbox.push(Command::LoadTangent {
                    tangent: *t,
                    what: what.clone(),
                });
                self.buffers.reader.await_target(target, task, title);
                self.note = Some("loading…".into());
            }
            ReadTarget::Doc(path) => {
                self.outbox.push(Command::LoadDoc { path: path.clone() });
                self.buffers.reader.await_target(target, task, title);
                self.note = Some("loading…".into());
            }
            ReadTarget::Decision(id) => {
                let markdown =
                    decision_markdown(&self.snap.decisions, &self.snap.tasks, *id, &self.tz);
                self.show_decision(target, task, title, markdown);
            }
            ReadTarget::Elsewhere(project, inner) => {
                let what = match inner.as_ref() {
                    ReadTarget::Research { task, what } => BrowseWhat::Research {
                        task: *task,
                        what: what.clone(),
                    },
                    ReadTarget::Doc(path) => BrowseWhat::Doc(path.clone()),
                    ReadTarget::Decision(id) => {
                        let markdown = self
                            .buffers
                            .library
                            .browsed_decision(project, *id, &self.tz);
                        self.show_decision(target.clone(), task, title, markdown);
                        return;
                    }
                    // Tangents are never browsed: they show whatever the library shows.
                    ReadTarget::Elsewhere(..) | ReadTarget::Tangent { .. } => return,
                };
                self.outbox.push(Command::LoadBrowsed {
                    project: project.clone(),
                    what,
                });
                self.buffers.reader.await_target(target, task, title);
                self.note = Some("loading…".into());
            }
        }
    }

    /// A decision-log entry, written out here rather than fetched.
    fn show_decision(
        &mut self,
        target: ReadTarget,
        task: Option<TaskId>,
        title: String,
        markdown: Option<String>,
    ) {
        let Some(markdown) = markdown else {
            self.note = Some("that decision is gone".into());
            return;
        };
        self.buffers.reader.open(Opened {
            target,
            task,
            title,
            document: ResearchDocument::text(None, markdown),
        });
        self.regions.set_focus(RegionId::Reader);
    }

    /// `gd` and `gx`: open the cited finding or source, or the web source in the browser.
    pub(super) fn follow_citation(
        &mut self,
        task: Option<TaskId>,
        evidence: Option<Evidence>,
        browse: bool,
        from: ReadTarget,
    ) {
        // A tangent's citations are its own findings and sources (ADR 0017).
        if let ReadTarget::Tangent { task: tangent, .. } = &from {
            let tangent = *tangent;
            return self.follow_tangent_citation(tangent, evidence, browse, from);
        }
        let Some(task) = task else {
            self.note =
                Some("this document is not research: it cites nothing Dovetail keeps".into());
            return;
        };
        // A browsed project's document cites that project's findings and sources.
        let elsewhere = from.elsewhere().cloned();
        let from = match from {
            ReadTarget::Elsewhere(_, inner) => *inner,
            from => from,
        };
        let there = |target: ReadTarget| match &elsewhere {
            Some(project) => ReadTarget::Elsewhere(project.clone(), Box::new(target)),
            None => target,
        };
        let listing = match &elsewhere {
            Some(project) => self.buffers.library.browsed_listing(project, task),
            None => self.buffers.library.listing(task),
        };
        let Some(evidence) = evidence else {
            // `gx` on a web source with no citation in view opens the page itself.
            if browse
                && let ReadTarget::Research {
                    what: ResearchWhat::Source(id),
                    ..
                } = from
                && let Some(source) = listing.and_then(|l| l.sources.iter().find(|s| s.id == id))
                && source.kind == SourceKind::Web
            {
                let url = source.locator.clone();
                self.open_external(url);
                return;
            }
            self.note = Some("no citation in view: ]s selects one".into());
            return;
        };
        match evidence {
            Evidence::Finding(id) => {
                if listing.is_some_and(|l| !l.findings.iter().any(|f| f.id == id)) {
                    self.note = Some(format!("F{} is not one of #{}'s findings", id.0, task.0));
                    return;
                }
                if browse {
                    self.note = Some(format!(
                        "F{} is a finding: gd reads it; gx opens web sources ([S n])",
                        id.0
                    ));
                    return;
                }
                let label = listing
                    .and_then(|l| l.findings.iter().find(|f| f.id == id))
                    .map_or_else(String::new, |f| format!(" · {}", f.label));
                self.read(
                    there(ReadTarget::Research {
                        task,
                        what: ResearchWhat::Finding(id),
                    }),
                    Some(task),
                    format!("F{}{label}", id.0),
                );
            }
            Evidence::Source(id) => {
                let source = listing.and_then(|l| l.sources.iter().find(|s| s.id == id));
                if listing.is_some() && source.is_none() {
                    self.note = Some(format!("S{} is not one of #{}'s sources", id.0, task.0));
                    return;
                }
                if browse {
                    match source {
                        Some(s) if s.kind == SourceKind::Web => {
                            let url = s.locator.clone();
                            self.open_external(url);
                        }
                        Some(s) if s.kind == SourceKind::File => {
                            self.note = Some(format!("S{} is a file: gd reads it", id.0));
                        }
                        Some(_) => {
                            self.note = Some(format!(
                                "S{} is a web search, not a page: gd reads it",
                                id.0
                            ));
                        }
                        None => self.note = Some("the sources are still loading".into()),
                    }
                    return;
                }
                let locator = source.map_or_else(String::new, |s| format!(" · {}", s.locator));
                self.read(
                    there(ReadTarget::Research {
                        task,
                        what: ResearchWhat::Source(id),
                    }),
                    Some(task),
                    format!("S{}{locator}", id.0),
                );
            }
        }
    }

    /// The core opens the page; the TUI never starts a process.
    pub(super) fn open_external(&mut self, url: String) {
        self.note = Some(format!("opening {url} in the browser"));
        self.outbox.push(Command::OpenExternal { url });
    }

    /// `i` in the reading column or Ask: type a question about the open report.
    pub(super) fn ask_about_report(&mut self) {
        let foreign = self.buffers.library.browsing().is_some()
            || self
                .buffers
                .reader
                .opened()
                .is_some_and(|o| o.target.elsewhere().is_some());
        if foreign {
            self.note = Some(ASK_OFF_WHILE_BROWSING.into());
            return;
        }
        let Some(task) = self.buffers.reader.task().or(self.buffers.ask.task()) else {
            self.note = Some("open a report first: ⏎ on one in the library".into());
            return;
        };
        let has_report = self
            .snap
            .research
            .iter()
            .any(|r| r.task == task && r.report_ms.is_some());
        if !has_report {
            self.note = Some(format!(
                "#{} has no report yet, so there is nothing to ask about",
                task.0
            ));
            return;
        }
        match ask_agent(&self.snap, task).map(|a| a.id.clone()) {
            Some(id) => {
                self.buffers.ask.follow(Some(task), &self.snap);
                self.start_ask_chat(id);
            }
            None => {
                self.pending_ask = Some(task);
                self.outbox.push(Command::StartAsk { task });
                self.note = Some("starting Ask…".into());
            }
        }
    }

    /// Types in Ask's chat bar.
    fn start_ask_chat(&mut self, id: AgentId) {
        if self.chat_site != ChatSite::Ask {
            self.chat_view_mut().park_chat();
        }
        self.chat_site = ChatSite::Ask;
        self.buffers.ask.conversation_mut().begin_chat(id);
        self.input = None;
        self.regions.set_focus(RegionId::Ask);
        self.mode = Mode::Insert;
        self.note = None;
    }

    /// Keeps the Ask column on the reading column's task, unless its chat bar is in use.
    pub(super) fn settle_reading(&mut self) {
        let typing = self.chat_site == ChatSite::Ask && self.mode == Mode::Insert;
        if !typing {
            // A browsed project's task is not the open project's task of that number.
            let task = self
                .buffers
                .reader
                .opened()
                .filter(|o| o.target.elsewhere().is_none())
                .and_then(|o| o.task);
            self.buffers.ask.follow(task, &self.snap);
        }
    }

    /// The figures kitty should show after the frame: the reading column's images in view,
    /// when reading mode is on and nothing floats over it.
    pub fn reading_figures(&self) -> Vec<crate::buffers::Figure> {
        if !self.reading_mode() || self.terminal_away || self.overlay.is_some() {
            return Vec::new();
        }
        self.buffers.reader.figures()
    }
}
