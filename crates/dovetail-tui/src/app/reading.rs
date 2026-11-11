//! Reading mode (ADR 0018, decided 2026-11-11): the second layout preset, with the library,
//! the reading column and Ask in place of Layout B, which comes back as it was.
//!
//! The app asks the core for what the library lists (`LoadResearch { Listing }` per research
//! task, `ListDocs`) when reading mode opens, opens documents in the reading column, resolves
//! `gd`/`gx` against the listings, and starts Ask through `Command::StartAsk`. Opening a web
//! page is the core's job (`Command::OpenExternal`).

use dovetail_api::research::{
    Evidence, ResearchContent, ResearchDocument, ResearchWhat, SourceKind,
};
use dovetail_api::{AgentId, AgentKind, Command, Event, ProjectKind, TaskId};

use super::{App, ChatSite};
use crate::buffers::library::decision_markdown;
use crate::buffers::reader::Opened;
use crate::buffers::{ReadTarget, ask_agent};
use crate::mode::Mode;
use crate::region::{Preset, RegionId};

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

    fn enter_reading_mode(&mut self) {
        // The task in focus in Layout B, to open its report straight away.
        let focused = self.focused_task();
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
    }

    /// Core answers reading mode uses: listings, docs, documents, and a new Ask session.
    /// Called before the event is applied.
    pub(super) fn reading_event(&mut self, event: &Event) {
        match event {
            Event::ResearchLoaded {
                task,
                content: ResearchContent::Listing(listing),
                ..
            } => self.buffers.library.receive_listing(*task, listing.clone()),
            Event::DocsListed { docs } => self.buffers.library.receive_docs(docs.clone()),
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
            ReadTarget::Doc(path) => {
                self.outbox.push(Command::LoadDoc { path: path.clone() });
                self.buffers.reader.await_target(target, task, title);
                self.note = Some("loading…".into());
            }
            ReadTarget::Decision(id) => {
                let Some(markdown) = decision_markdown(&self.snap, *id, &self.tz) else {
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
        }
    }

    /// `gd` and `gx`: open the cited finding or source, or the web source in the browser.
    pub(super) fn follow_citation(
        &mut self,
        task: Option<TaskId>,
        evidence: Option<Evidence>,
        browse: bool,
        from: ReadTarget,
    ) {
        let Some(task) = task else {
            self.note =
                Some("this document is not research: it cites nothing Dovetail keeps".into());
            return;
        };
        let listing = self.buffers.library.listing(task);
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
                let label = self
                    .buffers
                    .library
                    .finding(task, id)
                    .map_or_else(String::new, |f| format!(" · {}", f.label));
                self.read(
                    ReadTarget::Research {
                        task,
                        what: ResearchWhat::Finding(id),
                    },
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
                    ReadTarget::Research {
                        task,
                        what: ResearchWhat::Source(id),
                    },
                    Some(task),
                    format!("S{}{locator}", id.0),
                );
            }
        }
    }

    /// The core opens the page; the TUI never starts a process.
    fn open_external(&mut self, url: String) {
        self.note = Some(format!("opening {url} in the browser"));
        self.outbox.push(Command::OpenExternal { url });
    }

    /// `i` in the reading column or Ask: type a question about the open report.
    pub(super) fn ask_about_report(&mut self) {
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
            let task = self.buffers.reader.task();
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
