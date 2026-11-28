//! Tangents (ADR 0017, amended 2026-11-11): asking one, the tangents list, reading one in
//! reading mode, and sending one to a task or a project.

use dovetail_api::{Command, tracking::task::TaskId};
use dovetail_api::{agents::tangent::TangentCommand, research::ResearchWhat};

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
