//! Code domain: git worktree lifecycle and read-only worktree status inspection.

pub mod worktree;
pub(crate) mod worktree_info;

use std::collections::BTreeMap;

use dovetail_api::agents::agent::AgentId;
use dovetail_api::code::worktree::{WorktreeCommand, WorktreeInfo};

use crate::engine::Core;

/// State owned by the code domain.
#[derive(Default)]
pub(crate) struct CodeState {
    /// The worktree view's data, per slice (M12b, `worktree_info`).
    pub(crate) worktrees: BTreeMap<AgentId, WorktreeInfo>,
}

impl Core {
    pub(crate) fn handle_worktree_command(&mut self, command: WorktreeCommand) {
        match command {
            WorktreeCommand::Remove { agent, confirmed } => {
                self.remove_worktree(&agent, confirmed);
            }
            WorktreeCommand::Refresh => self.refresh_worktrees(),
        }
    }
}
