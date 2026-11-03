//! Overlays: the pickers and quick capture float over the layout and take every key until they
//! close.

use crossterm::event::KeyEvent;
use dovetail_api::Command;

use super::App;
use crate::capture::{Capture, CaptureOutcome, LinkTarget};
use crate::mode::Mode;
use crate::picker::{Picker, PickerKey, PickerOutcome};
use crate::region::RegionId;
use crate::switcher;

/// What floats over the layout.
pub(super) enum Overlay {
    /// The agent switcher or the everything picker.
    Picker(Picker),
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
        self.open_overlay(Overlay::Picker(Picker::new("Switch", items)));
    }

    /// `<leader><leader>`: tasks, agents and inbox items.
    pub(super) fn open_finder(&mut self) {
        let items = switcher::all_items(&self.snap, self.now_ms, &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Find", items)));
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
            Overlay::Picker(mut picker) => match picker.key(event) {
                PickerOutcome::Pending => self.overlay = Some(Overlay::Picker(picker)),
                PickerOutcome::Closed => self.close_overlay(),
                PickerOutcome::Chosen(key) => {
                    self.close_overlay();
                    self.choose(key);
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

    /// Opens what a picker row stands for.
    fn choose(&mut self, key: PickerKey) {
        match key {
            PickerKey::Agent(id) => self.open_agent_in_main(id),
            PickerKey::Task(id) => self.open_task(id),
            PickerKey::Inbox(id) => {
                self.regions.set_focus(RegionId::LeftBottom);
                self.buffers.inbox.select_item(&self.snap, id);
            }
        }
    }
}
