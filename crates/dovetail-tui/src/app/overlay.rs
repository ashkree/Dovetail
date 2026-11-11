//! Overlays: the pickers and quick capture float over the layout and take every key until they
//! close.

use crossterm::event::KeyEvent;
use dovetail_api::{Command, InboxId, TaskChange, TaskId};

use super::App;
use crate::capture::{Capture, CaptureOutcome, LinkTarget};
use crate::mode::Mode;
use crate::picker::{Picker, PickerKey, PickerOutcome};
use crate::switcher;

/// What choosing a picker row does.
pub(super) enum Purpose {
    /// Open what the row stands for.
    Open,
    /// Relate this task to the chosen one, or unrelate them.
    Relate(TaskId),
    /// File this unfiled finding (an inbox item) under the chosen task.
    FileUnfiled(InboxId),
}

/// What floats over the layout.
pub(super) enum Overlay {
    /// The agent switcher, the everything picker or the relate picker.
    Picker(Picker, Purpose),
    /// Quick capture.
    Capture(Capture),
}

impl App {
    fn open_overlay(&mut self, overlay: Overlay) {
        self.resolver.reset();
        self.overlay = Some(overlay);
        // Typing goes to the overlay, so the status bar reads INSERT.
        self.mode = Mode::Insert;
    }

    fn close_overlay(&mut self) {
        self.overlay = None;
        self.mode = Mode::Normal;
    }

    /// `<leader>ss`: Dovetail's agents only.
    pub(super) fn open_switcher(&mut self) {
        let items = switcher::agent_items(&self.snap, self.now_ms, &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Switch", items), Purpose::Open));
    }

    /// `:skills`: the slash commands and skills of the agent `i` would talk to.
    pub(super) fn open_command_picker(&mut self) {
        let Some(agent) = self.chat_target() else {
            return;
        };
        let commands = super::commands_for(&self.snap, &agent);
        if commands.is_empty() {
            self.note = Some("no commands yet: they arrive when a session starts".into());
            return;
        }
        let items = switcher::command_items(commands, &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Skills", items), Purpose::Open));
    }

    /// `<leader><leader>`: tasks, agents and inbox items.
    pub(super) fn open_finder(&mut self) {
        let items = switcher::all_items(&self.snap, self.now_ms, &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Find", items), Purpose::Open));
    }

    /// `:research`: the tasks that have research; choosing one opens its detail.
    pub(super) fn open_research_list(&mut self) {
        let items = switcher::research_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some("no research yet · :round <task> <brief> opens a round".into());
            return;
        }
        self.open_overlay(Overlay::Picker(
            Picker::new("Research", items),
            Purpose::Open,
        ));
    }

    /// `R` in the task detail: pick a task to relate this one to (or unrelate).
    pub(super) fn open_relate_picker(&mut self, task: TaskId) {
        let items = switcher::relate_items(&self.snap, &self.theme, task);
        self.open_overlay(Overlay::Picker(
            Picker::new("Relate to", items),
            Purpose::Relate(task),
        ));
    }

    /// `y` or Enter on an unfiled finding: pick the task to file it under.
    pub(super) fn open_file_picker(&mut self, item: InboxId) {
        let items = switcher::task_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some("there is no task to file it under: add one first (<leader>t)".into());
            return;
        }
        self.open_overlay(Overlay::Picker(
            Picker::new("File finding under", items),
            Purpose::FileUnfiled(item),
        ));
    }

    /// `<leader>t`: one line that becomes a task, linked to the task in focus.
    pub(super) fn open_capture(&mut self) {
        let link = self.focused_task().and_then(|id| {
            let task = self.snap.tasks.iter().find(|t| t.id == id)?;
            Some(LinkTarget {
                id,
                title: task.title.clone(),
            })
        });
        self.open_overlay(Overlay::Capture(Capture::new(link)));
    }

    pub(super) fn overlay_key(&mut self, event: KeyEvent) {
        let Some(overlay) = self.overlay.take() else {
            return;
        };
        match overlay {
            Overlay::Picker(mut picker, purpose) => match picker.key(event) {
                PickerOutcome::Pending => self.overlay = Some(Overlay::Picker(picker, purpose)),
                PickerOutcome::Closed => self.close_overlay(),
                PickerOutcome::Chosen(key) => {
                    self.close_overlay();
                    match (purpose, key) {
                        (Purpose::Relate(task), PickerKey::Task(other)) => {
                            self.toggle_related(task, other)
                        }
                        (Purpose::FileUnfiled(item), PickerKey::Task(task)) => {
                            self.outbox.push(Command::FileUnfiled { item, task });
                            self.note = Some(format!("filing the finding under #{}", task.0));
                        }
                        (_, key) => self.choose(key),
                    }
                }
            },
            Overlay::Capture(mut capture) => match capture.key(event) {
                CaptureOutcome::Pending => self.overlay = Some(Overlay::Capture(capture)),
                CaptureOutcome::Cancel => self.close_overlay(),
                CaptureOutcome::Save { open_detail } => {
                    self.close_overlay();
                    match capture.build() {
                        Some(task) => {
                            if open_detail {
                                self.open_after_create = Some(task.title.clone());
                            }
                            self.outbox.push(Command::CreateTask(task));
                        }
                        None => self.note = Some("a task needs a title".into()),
                    }
                }
            },
        }
    }

    fn toggle_related(&mut self, task: TaskId, other: TaskId) {
        let linked = self
            .snap
            .tasks
            .iter()
            .find(|t| t.id == task)
            .is_some_and(|t| t.related.contains(&other));
        let change = if linked {
            TaskChange::Unrelate(other)
        } else {
            TaskChange::Relate(other)
        };
        self.outbox.push(Command::UpdateTask {
            id: task,
            patch: vec![change],
        });
    }

    /// Opens what a picker row stands for.
    fn choose(&mut self, key: PickerKey) {
        match key {
            PickerKey::Agent(id) => self.open_agent_in_main(id),
            PickerKey::Task(id) => self.open_task(id, false),
            PickerKey::Inbox(id) => {
                self.focus_inbox();
                self.buffers.inbox.select_item(&self.snap, id);
            }
            PickerKey::Command(name) => {
                if let Some(agent) = self.chat_target() {
                    self.start_chat(agent);
                    self.chat_view_mut().insert_text(&format!("/{name} "));
                }
            }
        }
    }
}
