//! The side panel (decided 2026-11-11): side chats in the side column, in place of the task
//! tree, while the coordinator's conversation stays full height on the left (Layout B,
//! ADR 0006 and ADR 0008). The inbox stays on top, so what needs you is still in view.

use dovetail_api::AgentId;

use super::App;
use crate::buffers::ConversationBuffer;
use crate::mode::Mode;
use crate::region::{BufferKind, RegionId, Regions};

/// The side region the panel takes when it opens: the task tree's (ADR 0008 puts the inbox on
/// top, and it stays).
pub const PANEL_REGION: RegionId = RegionId::SideBottom;

impl App {
    /// The side region showing the panel, if it is open.
    pub(super) fn panel_region(&self) -> Option<RegionId> {
        self.regions.side_showing(BufferKind::SidePanel)
    }

    /// Whether the side panel is open (it is hidden, not closed, in focus mode).
    pub fn side_panel_open(&self) -> bool {
        self.panel_region().is_some()
    }

    /// The side chat the panel shows.
    pub fn panel_agent(&self) -> Option<&AgentId> {
        self.buffers.panel.agent()
    }

    /// `<leader>?`: opens the panel on its side chat and focuses it, or closes it and gives its
    /// region back to the buffer the default layout has there (the tree).
    pub(super) fn toggle_side_panel(&mut self) {
        match self.panel_region() {
            Some(region) => {
                if self.chat_in_panel {
                    self.buffers.panel.conversation_mut().park_chat();
                    self.chat_in_panel = false;
                }
                let back = Regions::default().buffer(region);
                self.regions.show(region, back);
            }
            None => {
                if self.focus_mode {
                    self.toggle_focus_mode();
                }
                self.buffers.panel.settle(&self.snap);
                self.regions.show(PANEL_REGION, BufferKind::SidePanel);
                self.regions.set_focus(PANEL_REGION);
            }
        }
    }

    /// Opens the panel on a side chat and focuses it; the conversation in the main region stays.
    pub(super) fn open_in_panel(&mut self, id: AgentId) {
        if self.focus_mode {
            self.toggle_focus_mode();
        }
        let region = match self.panel_region() {
            Some(region) => region,
            None => {
                self.regions.show(PANEL_REGION, BufferKind::SidePanel);
                PANEL_REGION
            }
        };
        if self.chat_in_panel {
            self.chat_in_panel = false;
            if self.mode == Mode::Insert && self.input.is_none() {
                self.mode = Mode::Normal;
            }
        }
        self.clear_finished_turn(&id);
        self.buffers.panel.show(id);
        self.regions.set_focus(region);
    }

    /// Whether `i` (or Enter in the tree, or a picked command) should type in the panel's chat
    /// bar: the panel has focus and shows `agent`.
    pub(super) fn chats_in_panel(&self, agent: &AgentId) -> bool {
        self.regions.focused_buffer() == BufferKind::SidePanel
            && self.buffers.panel.agent() == Some(agent)
    }

    /// The conversation view whose chat bar is open, or was open last: the panel's or the
    /// main region's.
    pub(super) fn chat_view(&self) -> &ConversationBuffer {
        if self.chat_in_panel {
            self.buffers.panel.conversation()
        } else {
            &self.buffers.conversation
        }
    }

    /// See [`chat_view`](Self::chat_view).
    pub(super) fn chat_view_mut(&mut self) -> &mut ConversationBuffer {
        if self.chat_in_panel {
            self.buffers.panel.conversation_mut()
        } else {
            &mut self.buffers.conversation
        }
    }
}
