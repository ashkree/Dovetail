//! Tangents (ADR 0017, amended 2026-11-11): asking one, the tangents list, reading one in
//! reading mode, and sending one to a task or a project.

use dovetail_api::{Command, tracking::task::TaskId};
use dovetail_api::{
    agents::tangent::TangentCommand,
    research::{Evidence, ResearchWhat, SourceKind},
};

use super::overlay::{Overlay, Purpose};
use super::{App, Confirm, Then};
use crate::buffers::ReadTarget;
use crate::buffers::library::LibKey;
use crate::picker::Picker;
use crate::region::{BufferKind, RegionId};
use crate::switcher;

impl App {
    /// Asks the core for a tangent; its session opens in the side panel when it appears.
    pub(super) fn new_tangent(&mut self, question: String) {
        self.open_side_chat = true;
        self.note = Some("asking a tangent: its answer is kept in Tangents".into());
        self.outbox
            .push(Command::Tangent(TangentCommand::New { question }));
    }

    /// `:tangents`, or the "new tangents" item: the list in the main region, focused.
    pub(super) fn show_tangents(&mut self) {
        if self.reading_mode() {
            self.leave_reading_mode();
        }
        if self.focus_mode {
            self.toggle_focus_mode();
        }
        self.regions.show(RegionId::Main, BufferKind::Tangents);
        self.regions.set_focus(RegionId::Main);
    }

    /// The title of tangent `task`, for prompts.
    fn tangent_title(&self, task: TaskId) -> String {
        self.snap
            .tangents
            .iter()
            .find(|t| t.task.id == task)
            .map_or_else(|| format!("#{}", task.0), |t| t.task.title.clone())
    }

    /// Reads a tangent's latest answer in reading mode, fetching its listing first when the
    /// library does not have it yet.
    pub(super) fn read_tangent(&mut self, tangent: TaskId) {
        if !self.reading_mode() {
            self.enter_reading_mode();
        }
        self.buffers.library.select(LibKey::Tangent(tangent));
        if self.buffers.library.tangent_listing(tangent).is_some() {
            self.read_latest_answer(tangent);
            return;
        }
        self.pending_tangent_read = Some(tangent);
        self.outbox.push(Command::Tangent(TangentCommand::Load {
            tangent,
            what: ResearchWhat::Listing,
        }));
        self.note = Some("loading…".into());
    }

    /// Opens the tangent's latest answer from its listing.
    pub(super) fn read_latest_answer(&mut self, tangent: TaskId) {
        let Some(answer) = self.buffers.library.latest_answer(tangent) else {
            self.note = Some("no answer yet: it shows here when the turn ends".into());
            return;
        };
        let id = answer.id;
        let title = format!("tangent F{} · {}", id.0, self.tangent_title(tangent));
        self.read(
            ReadTarget::Tangent {
                task: tangent,
                what: ResearchWhat::Finding(id),
            },
            None,
            title,
        );
    }

    /// `gd` and `gx` in a tangent's document: its own findings and sources.
    pub(super) fn follow_tangent_citation(
        &mut self,
        tangent: TaskId,
        evidence: Option<Evidence>,
        browse: bool,
        from: ReadTarget,
    ) {
        let listing = self.buffers.library.tangent_listing(tangent);
        let Some(evidence) = evidence else {
            // `gx` on a web source with no citation in view opens the page itself.
            if browse
                && let ReadTarget::Tangent {
                    what: ResearchWhat::Source(id),
                    ..
                } = from
                && let Some(s) = listing.and_then(|l| l.sources.iter().find(|s| s.id == id))
                && s.kind == SourceKind::Web
            {
                let url = s.locator.clone();
                self.open_external(url);
                return;
            }
            self.note = Some("no citation in view: ]s selects one".into());
            return;
        };
        match evidence {
            Evidence::Finding(id) => {
                if browse {
                    self.note = Some(format!(
                        "F{} is a finding: gd reads it; gx opens web sources ([S n])",
                        id.0
                    ));
                    return;
                }
                if listing.is_some_and(|l| !l.findings.iter().any(|f| f.id == id)) {
                    self.note = Some(format!("F{} is not one of this tangent's findings", id.0));
                    return;
                }
                self.read(
                    ReadTarget::Tangent {
                        task: tangent,
                        what: ResearchWhat::Finding(id),
                    },
                    None,
                    format!("tangent F{}", id.0),
                );
            }
            Evidence::Source(id) => {
                let source = listing.and_then(|l| l.sources.iter().find(|s| s.id == id));
                if listing.is_some() && source.is_none() {
                    self.note = Some(format!("S{} is not one of this tangent's sources", id.0));
                    return;
                }
                if browse {
                    match source {
                        Some(s) if s.kind == SourceKind::Web => {
                            let url = s.locator.clone();
                            self.open_external(url);
                        }
                        Some(_) => {
                            self.note = Some(format!("S{} is not a web page: gd reads it", id.0));
                        }
                        None => self.note = Some("the sources are still loading".into()),
                    }
                    return;
                }
                let locator = source.map_or_else(String::new, |s| format!(" · {}", s.locator));
                self.read(
                    ReadTarget::Tangent {
                        task: tangent,
                        what: ResearchWhat::Source(id),
                    },
                    None,
                    format!("tangent S{}{locator}", id.0),
                );
            }
        }
    }

    /// `f` or `s` in the tangents list: pick the project to file it into, or to start a task
    /// from it in.
    pub(super) fn pick_tangent_project(&mut self, tangent: TaskId, start: bool) {
        let items = switcher::tangent_target_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some(
                "no project to send it to: :project add registers this folder, :project new \
                 <name> --research makes one"
                    .into(),
            );
            return;
        }
        let (title, purpose) = if start {
            ("Start a task in", Purpose::StartTangent(tangent))
        } else {
            ("File tangent into", Purpose::FileTangent(tangent))
        };
        self.open_overlay(Overlay::Picker(Picker::new(title, items), purpose));
    }

    /// `a` on a tangent: attach it to the task in focus (the open task detail, else the tree's
    /// selection), after a y/n; with no task in focus, pick one.
    pub(super) fn attach_tangent(&mut self, tangent: TaskId) {
        if self
            .snap
            .project
            .as_ref()
            .is_some_and(switcher::is_tangents)
        {
            self.note = Some("Tangents is open, so the tangent is here already".into());
            return;
        }
        let target = if self.regions.buffer(RegionId::Main) == BufferKind::TaskDetail {
            self.buffers.task_detail.task()
        } else {
            self.buffers.tree.selected_task(&cx!(self))
        };
        let target = target.and_then(|id| self.snap.tasks.iter().find(|t| t.id == id));
        match target {
            Some(task) => {
                self.confirm = Some(Confirm {
                    prompt: format!(
                        "Attach the tangent \"{}\" to #{} {}? Its answers and sources become \
                         quick findings there, and it leaves Tangents.",
                        self.tangent_title(tangent),
                        task.id.0,
                        task.title
                    ),
                    then: Then::Commands(vec![Command::Tangent(TangentCommand::Attach {
                        tangent,
                        task: task.id,
                    })]),
                    otherwise: None,
                });
            }
            None => {
                let items = switcher::task_items(&self.snap, &self.theme);
                if items.is_empty() {
                    self.note =
                        Some("this project has no task to attach it to: ␣t adds one".into());
                    return;
                }
                self.open_overlay(Overlay::Picker(
                    Picker::new("Attach tangent to", items),
                    Purpose::AttachTangent(tangent),
                ));
            }
        }
    }
}
