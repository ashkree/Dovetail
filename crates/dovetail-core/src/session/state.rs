//! The state of one live session's turn, kept explicitly (plan §4).
//!
//! A turn starts when Dovetail sends a prompt or Claude Code starts one by itself (`system/init`
//! comes first; an inbound message wakes an idle session, spike S2). It is over at a `result`,
//! but the session is only done when no background tasks remain: never on the first `result`
//! alone (stream rule 2).

use dovetail_api::{
    agent::{AgentStatus, PermissionMode},
    conversation::EntryId,
};
use dovetail_driver::PermissionMode as CliMode;

/// Maps the UI's mode to the CLI's.
pub fn to_cli(mode: PermissionMode) -> CliMode {
    match mode {
        PermissionMode::Auto => CliMode::Auto,
        PermissionMode::AcceptEdits => CliMode::AcceptEdits,
        PermissionMode::Manual => CliMode::Manual,
    }
}

/// Maps the CLI's mode name (`auto`, `acceptEdits`, `default`, `plan`, `dontAsk`) to the UI's.
/// Plan mode asks before acting, and `dontAsk` (side chats) runs nothing unapproved, so both
/// show as manual.
pub fn from_cli(name: &str) -> Option<PermissionMode> {
    Some(match CliMode::from_cli(name)? {
        CliMode::Auto => PermissionMode::Auto,
        CliMode::AcceptEdits => PermissionMode::AcceptEdits,
        CliMode::Manual | CliMode::Plan | CliMode::DontAsk => PermissionMode::Manual,
    })
}

/// The `<leader>sm` cycle: auto, acceptEdits, manual, auto (Decision 15).
pub fn next_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Auto => PermissionMode::AcceptEdits,
        PermissionMode::AcceptEdits => PermissionMode::Manual,
        PermissionMode::Manual => PermissionMode::Auto,
    }
}

/// The assistant entry currently receiving text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Streaming {
    /// The API message id the text belongs to.
    pub message_id: Option<String>,
    /// The conversation entry it streams into.
    pub entry: EntryId,
    /// Text confirmed by complete `assistant` lines so far; wins over the deltas.
    pub confirmed: String,
}

/// One turn's tool uses, folded into one summary line (readability rule).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolTally {
    /// The summary entry, once the turn has used a tool.
    pub entry: Option<EntryId>,
    /// Tool names in first-use order, with counts.
    pub counts: Vec<(String, u32)>,
}

impl ToolTally {
    /// Counts one use of `name`.
    pub fn add(&mut self, name: &str) {
        match self.counts.iter_mut().find(|(n, _)| n == name) {
            Some((_, count)) => *count += 1,
            None => self.counts.push((name.to_owned(), 1)),
        }
    }

    /// `Bash ×2 · Read · core_ask_user`.
    pub fn line(&self) -> String {
        self.counts
            .iter()
            .map(|(name, count)| {
                let name = name.strip_prefix("mcp__dovetail__").unwrap_or(name);
                if *count > 1 {
                    format!("{name} ×{count}")
                } else {
                    name.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Where a live session's turn stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    /// A turn is running.
    pub running: bool,
    /// Background tasks still running (from `background_tasks_changed`).
    pub background: usize,
    /// The assistant entry receiving text.
    pub streaming: Option<Streaming>,
    /// This turn's tools.
    pub tools: ToolTally,
    /// The `tool_use` id of the tool shown as running (`Agent::tool`), until its result.
    pub tool_id: Option<String>,
    /// The last assistant text, for the finished-turn preview.
    pub last_text: String,
    /// An interrupt was requested and its `result` has not come yet.
    pub interrupting: bool,
    /// Inputs sent and not yet taken up by a turn. Claude Code echoes each input it takes
    /// (`--replay-user-messages`): inputs sent mid-turn may be folded into the running turn
    /// (echoed before its `result`) or run as a turn of their own afterwards. A `result` ends
    /// the session's work only when every sent input has been echoed (live rehearsal,
    /// 2026-11-06: three inputs, one turn, one `result`).
    pub queued: usize,
    /// This process has echoed an input. Without echoes (a stream without
    /// `--replay-user-messages`), each `result` is taken to answer one input instead.
    pub replays_seen: bool,
}

impl Turn {
    /// Whether the session is busy: in a turn or with background work left.
    pub fn busy(&self) -> bool {
        self.running || self.background > 0
    }

    /// Dovetail sent an input: a turn runs, now or after the current one.
    pub fn input_sent(&mut self) {
        self.running = true;
        self.queued += 1;
    }

    /// Claude Code echoed one of Dovetail's inputs: it belongs to the running turn.
    pub fn input_replayed(&mut self) {
        self.replays_seen = true;
        self.queued = self.queued.saturating_sub(1);
    }

    /// Clears what belongs to one turn when its `result` arrives. The session stays running
    /// while an input it was sent has not been taken up yet (stream rule 2).
    pub fn end(&mut self) {
        if !self.replays_seen {
            self.queued = self.queued.saturating_sub(1);
        }
        self.running = self.queued > 0;
        self.streaming = None;
        self.tools = ToolTally::default();
        self.tool_id = None;
        self.interrupting = false;
    }
}

/// What decides the status a session shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusInputs {
    /// A slice waiting for its worktree.
    pub setting_up: bool,
    /// An inbox item holds it (a permission request).
    pub needs_you: bool,
    /// In a turn, or with background tasks left.
    pub busy: bool,
    /// Its last session failed and nothing has run since.
    pub failed: bool,
}

impl StatusInputs {
    /// The status, most urgent first.
    pub fn status(self) -> AgentStatus {
        if self.setting_up {
            AgentStatus::SettingUp
        } else if self.needs_you {
            AgentStatus::NeedsYou
        } else if self.busy {
            AgentStatus::Working
        } else if self.failed {
            AgentStatus::Error
        } else {
            AgentStatus::Idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_the_cli_names() {
        for mode in [
            PermissionMode::Auto,
            PermissionMode::AcceptEdits,
            PermissionMode::Manual,
        ] {
            assert_eq!(from_cli(to_cli(mode).as_cli()), Some(mode));
        }
        assert_eq!(from_cli("default"), Some(PermissionMode::Manual));
        assert_eq!(from_cli("plan"), Some(PermissionMode::Manual));
        assert_eq!(from_cli("bogus"), None);
    }

    #[test]
    fn the_tally_folds_repeats() {
        let mut tally = ToolTally::default();
        for name in ["Bash", "Read", "Bash", "mcp__dovetail__core_ask_user"] {
            tally.add(name);
        }
        assert_eq!(tally.line(), "Bash ×2 · Read · core_ask_user");
    }
}
