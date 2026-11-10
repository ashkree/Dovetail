//! What "this agent" and "this task" mean right now, and the actions that act on them.

use dovetail_api::{
    AgentId, AgentKind, AgentStatus, Command, InboxId, PermissionMode, SessionLock, TaskId,
};

use super::{App, Confirm, Then};
use crate::action::Action;
use crate::buffers::InputTarget;
use crate::mode::Mode;
use crate::region::{BufferKind, RegionId, Regions};

/// The next mode in the cycle auto, acceptEdits, manual (Decision 15, permission mode).
fn next_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Auto => PermissionMode::AcceptEdits,
        PermissionMode::AcceptEdits => PermissionMode::Manual,
        PermissionMode::Manual => PermissionMode::Auto,
    }
}

fn mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Auto => "auto",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Manual => "manual",
    }
}

impl App {
    /// Focuses the side region showing the inbox (the top one by default, ADR 0008).
    /// Focuses the inbox; when no region shows it, it goes back where the default layout has it.
    pub(super) fn focus_inbox(&mut self) {
        let region = match self.regions.side_showing(BufferKind::Inbox) {
            Some(region) => region,
            None => {
                let Some(region) = Regions::default_side_of(BufferKind::Inbox) else {
                    return;
                };
                self.regions.show(region, BufferKind::Inbox);
                region
            }
        };
        self.regions.set_focus(region);
    }

    pub(super) fn coordinator(&self) -> Option<AgentId> {
        self.snap
            .agents
            .iter()
            .find(|a| a.kind == AgentKind::Coordinator)
            .map(|a| a.id.clone())
    }

    /// Who `i` talks to (plan A11): the agent in the main region when it has focus, the agent
    /// selected in the tree when the tree has focus, otherwise the coordinator.
    pub(super) fn chat_target(&self) -> Option<AgentId> {
        let from_tree = if self.regions.focused_buffer() == BufferKind::Tree {
            self.buffers.tree.selected_agent(&cx!(self))
        } else {
            None
        };
        let from_main = (self.regions.focus() == RegionId::Main
            && self.regions.buffer(RegionId::Main) == BufferKind::Conversation)
            .then(|| self.buffers.conversation.agent().cloned())
            .flatten();
        from_main.or(from_tree).or_else(|| self.coordinator())
    }

    /// The agent `<leader>sm` and `<leader>sx` act on: like [`chat_target`](Self::chat_target),
    /// except that with the inbox focused it is the selected item's agent.
    pub(super) fn focused_agent(&self) -> Option<AgentId> {
        let item = match self.regions.focused_buffer() {
            BufferKind::Inbox => self.buffers.inbox.selected(&self.snap),
            BufferKind::InboxDetail => self.buffers.inbox_detail.current(&self.snap),
            _ => None,
        };
        if let Some(item) = item {
            return Some(item.agent.clone());
        }
        self.chat_target()
    }

    pub(super) fn start_chat(&mut self, agent: AgentId) {
        self.show_agent(agent.clone());
        self.buffers.conversation.begin_chat(agent);
        self.input = None;
        // The input lives in the main pane, so focus goes with it.
        self.regions.set_focus(RegionId::Main);
        self.mode = Mode::Insert;
    }

    /// The task in focus: the one under the tree cursor, the selected inbox item's, the open
    /// conversation's agent's, or the open task detail.
    pub(super) fn focused_task(&self) -> Option<TaskId> {
        let task_of = |agent: &AgentId| self.snap.agents.iter().find(|a| &a.id == agent)?.task;
        match self.regions.focused_buffer() {
            BufferKind::Tree => self.buffers.tree.selected_task(&cx!(self)),
            BufferKind::Inbox => self
                .buffers
                .inbox
                .selected(&self.snap)
                .and_then(|item| task_of(&item.agent)),
            BufferKind::Conversation => self.buffers.conversation.agent().and_then(task_of),
            BufferKind::TaskDetail => self.buffers.task_detail.task(),
            BufferKind::InboxDetail => self
                .buffers
                .inbox_detail
                .current(&self.snap)
                .and_then(|item| task_of(&item.agent)),
            BufferKind::Board => self.buffers.board.selected_task(&self.snap),
            BufferKind::Worktrees => self
                .buffers
                .worktrees
                .selected_agent(&self.snap)
                .and_then(|id| task_of(&id)),
            BufferKind::Activity
            | BufferKind::Messages
            | BufferKind::MapShelf
            | BufferKind::MapOutline => None,
        }
    }

    /// Shows a task's detail in the main region.
    /// `from_board`: opened from a board card, so `Esc` goes back to the board. Every other
    /// way in (tree, picker, quick capture) goes back to the conversation.
    pub(super) fn open_task(&mut self, id: TaskId, from_board: bool) {
        self.detail_from_board = from_board;
        self.buffers.task_detail.show(id);
        self.regions.show(RegionId::Main, BufferKind::TaskDetail);
        self.regions.set_focus(RegionId::Main);
    }

    /// Shows an inbox item in full in the main region; the inbox keeps it selected.
    pub(super) fn open_inbox_item(&mut self, id: InboxId) {
        self.buffers.inbox.select_item(&self.snap, id);
        self.buffers.inbox_detail.show(id);
        self.regions.show(RegionId::Main, BufferKind::InboxDetail);
        self.regions.set_focus(RegionId::Main);
    }

    /// Shows a buffer in the main region, or goes back to the conversation if it is already
    /// there ("Esc or the same key goes back").
    pub(super) fn toggle_main(&mut self, kind: BufferKind) {
        if self.regions.buffer(RegionId::Main) == kind {
            self.regions.show(RegionId::Main, BufferKind::Conversation);
        } else {
            self.regions.show(RegionId::Main, kind);
            self.regions.set_focus(RegionId::Main);
        }
    }

    /// The session a command about "this agent" applies to. A subagent has no session of its
    /// own, so it is its parent's (plan B20); the flag says it was redirected.
    fn session_of(&self, id: &AgentId) -> Option<(AgentId, bool)> {
        let agent = self.snap.agents.iter().find(|a| &a.id == id)?;
        match (&agent.kind, &agent.parent) {
            (AgentKind::Subagent { .. }, Some(parent)) => Some((parent.clone(), true)),
            _ => Some((agent.id.clone(), false)),
        }
    }

    /// Runs an action that may take text after it (`:spawn name brief`, `:interrupt name`).
    pub(super) fn run_with_arg(&mut self, action: Action, arg: &str) {
        let arg = arg.trim();
        match action {
            Action::NewSlice if arg.is_empty() => self.start_input(InputTarget::SliceName, ""),
            Action::NewSideChat if arg.is_empty() => {
                self.start_input(InputTarget::SideChatQuestion, "")
            }
            Action::NewSideChat => self.new_side_chat(arg.to_string()),
            Action::Upgrade if arg.is_empty() || arg == "--local" => {
                self.outbox.push(Command::Upgrade {
                    local: arg == "--local",
                });
                self.note = Some(if arg.is_empty() {
                    "building the upgrade from origin/main; \"Upgrade ready\" appears in Needs you"
                        .into()
                } else {
                    "building the upgrade from this checkout; \"Upgrade ready\" appears in Needs you".into()
                });
            }
            Action::Upgrade => self.note = Some("usage: :upgrade [--local]".into()),
            Action::NewSlice => {
                let (name, brief) = arg
                    .split_once(char::is_whitespace)
                    .map_or((arg, ""), |(n, b)| (n, b.trim()));
                self.new_slice(name.to_string(), brief.to_string());
            }
            Action::Interrupt => {
                let target = if arg.is_empty() {
                    self.focused_agent()
                } else {
                    self.snap
                        .agents
                        .iter()
                        .find(|a| a.name == arg)
                        .map(|a| a.id.clone())
                };
                let Some(target) = target else {
                    self.note = Some(format!("no agent named {arg}"));
                    return;
                };
                match self.session_of(&target) {
                    // A subagent cannot be stopped alone: ask before stopping its whole session
                    // (decided 2026-11-03).
                    Some((agent, true)) => {
                        let prompt = self.interrupt_prompt(&target, &agent);
                        self.confirm = Some(Confirm {
                            prompt,
                            then: Then::Commands(vec![Command::Interrupt { agent }]),
                            otherwise: None,
                        });
                    }
                    Some((agent, false)) => self.outbox.push(Command::Interrupt { agent }),
                    None => self.note = Some(format!("no agent named {arg}")),
                }
            }
            Action::TaskAddDecision => match self.focused_task() {
                Some(task) if arg.is_empty() => {
                    self.start_input(InputTarget::Decision(task), "");
                }
                Some(task) => self.log_decision(task, arg.to_string()),
                None => self.note = Some("no task in focus to decide on".into()),
            },
            other => self.run(other),
        }
    }

    /// "Interrupting rebase stops slice-4's turn and its 2 other subagents."
    fn interrupt_prompt(&self, sub: &AgentId, session: &AgentId) -> String {
        let name = |id: &AgentId| {
            self.snap
                .agents
                .iter()
                .find(|a| &a.id == id)
                .map_or_else(|| id.0.clone(), |a| a.name.clone())
        };
        let others = self
            .snap
            .agents
            .iter()
            .filter(|a| {
                a.parent.as_ref() == Some(session)
                    && &a.id != sub
                    && a.status == dovetail_api::AgentStatus::Working
            })
            .count();
        let rest = match others {
            0 => String::new(),
            1 => " and its 1 other running subagent".into(),
            n => format!(" and its {n} other running subagents"),
        };
        format!(
            "Interrupting {} stops {}'s turn{rest}. Go ahead?",
            name(sub),
            name(session)
        )
    }

    pub(super) fn cycle_permission_mode(&mut self) {
        let Some((session, redirected)) = self.focused_agent().and_then(|id| self.session_of(&id))
        else {
            self.note = Some("no agent to change".into());
            return;
        };
        let Some(agent) = self.snap.agents.iter().find(|a| a.id == session) else {
            return;
        };
        let next = next_mode(agent.permission_mode);
        let mut note = format!("{}: permission mode → {}", agent.name, mode_name(next));
        if redirected {
            note.push_str(" (subagents share their session's)");
        }
        self.note = Some(note);
        self.outbox
            .push(Command::CyclePermissionMode { agent: session });
    }

    /// `<leader>st`: lends the terminal to native `claude` on the focused session (a
    /// subagent's: its session's). Asks first when a turn runs, as it is cut off.
    pub(super) fn take_over(&mut self) {
        let Some((session, redirected)) = self.focused_agent().and_then(|id| self.session_of(&id))
        else {
            self.note = Some("no session to take over".into());
            return;
        };
        let Some(agent) = self.snap.agents.iter().find(|a| a.id == session) else {
            return;
        };
        let name = agent.name.clone();
        if agent.lock == SessionLock::TakenOver {
            self.note = Some(format!("{name} is already taken over in a native terminal"));
            return;
        }
        if agent.status == AgentStatus::SettingUp {
            self.note = Some(format!("{name} is still setting up"));
            return;
        }
        let command = Command::TakeOver { agent: session };
        if agent.status == AgentStatus::Working {
            let whose = if redirected {
                format!("{name} (the session this subagent runs in)")
            } else {
                name
            };
            self.confirm = Some(Confirm {
                prompt: format!("{whose} is mid-turn. Take it over anyway? The turn is cut off."),
                then: Then::Handoff(command),
                otherwise: None,
            });
            return;
        }
        self.handoff = Some(command);
    }
}
