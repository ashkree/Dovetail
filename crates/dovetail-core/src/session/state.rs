//! The state of one live session's turn, kept explicitly (plan §4).
//!
//! A turn starts when Dovetail sends a prompt or Claude Code starts one by itself (`system/init`
//! comes first; an inbound message wakes an idle session, spike S2). It is over at a `result`,
//! but the session is only done when no background tasks remain: never on the first `result`
//! alone (stream rule 2).

use dovetail_api::{AgentStatus, EntryId, PermissionMode};
use dovetail_driver::PermissionMode as CliMode;

/// Maps the UI's mode to the CLI's.
pub fn to_cli(mode: PermissionMode) -> CliMode {
    match mode {
        PermissionMode::Auto => CliMode::Auto,
        PermissionMode::AcceptEdits => CliMode::AcceptEdits,
        PermissionMode::Manual => CliMode::Manual,
    }
}

/// Maps the CLI's mode name (`auto`, `acceptEdits`, `default`, `plan`) to the UI's. Plan mode
/// asks before acting, so it shows as manual.
pub fn from_cli(name: &str) -> Option<PermissionMode> {
    Some(match CliMode::from_cli(name)? {
        CliMode::Auto => PermissionMode::Auto,
        CliMode::AcceptEdits => PermissionMode::AcceptEdits,
        CliMode::Manual | CliMode::Plan => PermissionMode::Manual,
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

    /// `Bash ×2 · Read · ask_user`.
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
    /// The last assistant text, for the finished-turn preview.
    pub last_text: String,
    /// An interrupt was requested and its `result` has not come yet.
    pub interrupting: bool,
}

impl Turn {
    /// Whether the session is busy: in a turn or with background work left.
    pub fn busy(&self) -> bool {
        self.running || self.background > 0
    }

    /// Clears what belongs to one turn when it ends.
    pub fn end(&mut self) {
        self.running = false;
        self.streaming = None;
        self.tools = ToolTally::default();
        self.interrupting = false;
    }
}

/// The status a session shows, from what holds it (most urgent first).
pub fn status(setting_up: bool, needs_you: bool, busy: bool, failed: bool) -> AgentStatus {
    if setting_up {
        AgentStatus::SettingUp
    } else if needs_you {
        AgentStatus::NeedsYou
    } else if busy {
        AgentStatus::Working
    } else if failed {
        AgentStatus::Error
    } else {
        AgentStatus::Idle
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
        for name in ["Bash", "Read", "Bash", "mcp__dovetail__ask_user"] {
            tally.add(name);
        }
        assert_eq!(tally.line(), "Bash ×2 · Read · ask_user");
    }
}
