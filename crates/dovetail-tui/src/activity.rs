//! The "alive" indicators: a spinner, a label and an elapsed time while an agent works or a long
//! command runs.
//!
//! Nothing here reads the clock. Callers pass the UI's clock (`now_ms`, Unix milliseconds), so
//! frames and words are the same in every test. The UI animates only while an indicator is
//! visible (see [`next_frame_in`]); an idle app has no deadline at all.

use std::collections::HashMap;
use std::time::Duration;

use dovetail_api::{
    Snapshot,
    agent::{AgentId, AgentStatus, RunningTool},
    conversation::{ConversationEntry, EntryKind},
};

use crate::fmt;

/// Time one spinner frame shows: about 8 fps, slow enough to stay calm and cheap to draw.
pub const FRAME_MS: i64 = 125;

/// The spinner's frames (braille, one cell wide).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What a working agent says it is doing while the model thinks or writes and no tool runs.
/// Dovetail is a joint, so the words are the woodworker's.
pub const WORDS: [&str; 16] = [
    "Dovetailing",
    "Planing",
    "Chiselling",
    "Mortising",
    "Tenoning",
    "Sanding",
    "Scribing",
    "Jointing",
    "Clamping",
    "Gluing up",
    "Rabbeting",
    "Bevelling",
    "Marking out",
    "Fitting",
    "Burnishing",
    "Seasoning",
];

/// Longest command or path shown in a tool label, in cells.
const ARG_WIDTH: usize = 40;

/// The spinner frame to show at `now_ms`.
pub fn spinner(now_ms: i64) -> &'static str {
    SPINNER[frame_index(now_ms) % SPINNER.len()]
}

/// How many frames have passed since the epoch: changes when the picture should.
pub fn frame_index(now_ms: i64) -> usize {
    usize::try_from(now_ms.max(0) / FRAME_MS).unwrap_or(0)
}

/// Time to the next frame boundary, for the loop's deadline. Always `1..=FRAME_MS` ms, so a
/// wake that lands a hair early cannot spin.
pub fn next_frame_in(now_ms: i64) -> Duration {
    let into = now_ms.max(0) % FRAME_MS;
    Duration::from_millis(u64::try_from(FRAME_MS - into).unwrap_or(1))
}

/// `12s`, `1m 05s`, `1h 02m`.
pub fn elapsed(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

/// The woodworking word for the turn that began with `turn_seed` (an entry id). One word per
/// turn: the seed does not change until the next turn begins.
pub fn word(turn_seed: u64) -> &'static str {
    WORDS[usize::try_from(turn_seed % WORDS.len() as u64).unwrap_or(0)]
}

/// The id that starts the turn being shown: the newest message to the agent (the user's, or a
/// notice Dovetail sent in its name). `0` for an empty conversation.
pub fn turn_seed(entries: &[ConversationEntry]) -> u64 {
    entries
        .iter()
        .rev()
        .find(|e| matches!(e.kind, EntryKind::User | EntryKind::Notice))
        .map_or(0, |e| e.id.0)
}

/// What the working line says: the running tool in words ("Reading plan.md") or, with no tool
/// running, the turn's woodworking word and an ellipsis ("Planing…").
pub fn label(tool: Option<&RunningTool>, turn_seed: u64) -> String {
    match tool {
        Some(tool) => tool_label(tool),
        None => format!("{}…", word(turn_seed)),
    }
}

/// A tool call as a short phrase, derived from its name and main argument.
pub fn tool_label(tool: &RunningTool) -> String {
    let arg = tool.arg.trim();
    let file = || fmt::truncate(basename(arg), ARG_WIDTH);
    let text = || fmt::truncate(arg, ARG_WIDTH);
    let with = |verb: &str, what: String| {
        if what.is_empty() {
            format!("{verb}…")
        } else {
            format!("{verb} {what}…")
        }
    };
    match tool.name.as_str() {
        "Read" => with("Reading", file()),
        "Edit" | "MultiEdit" | "NotebookEdit" => with("Editing", file()),
        "Write" => with("Writing", file()),
        "Bash" => with("Running", text()),
        "Grep" => with("Searching for", text()),
        "Glob" | "LS" => with("Finding", text()),
        "WebFetch" => with("Fetching", text()),
        "WebSearch" => with("Searching the web for", text()),
        "Task" | "Agent" | "invoke_subagent" => with("Waiting on subagent", text()),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" => "Updating the to-do list…".into(),
        name => {
            // `mcp__dovetail__core_ask_user` is "ask_user": the server prefix tells nothing.
            let short = name.rsplit("__").next().unwrap_or(name);
            let short = short.strip_prefix("core_").unwrap_or(short);
            with(&format!("Calling {short}"), String::new())
        }
    }
}

/// The last path component, for a file label; a path with no `/` is shown as it is.
fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// A command that takes a while to finish, and that the core reports on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    /// `:eod`, waiting for the coordinator's handoff.
    ClosingDay,
    /// `:handoff`, waiting for the same handoff.
    FreshStart,
    /// `:upgrade`, building in the background.
    Upgrade,
}

impl Busy {
    /// The status bar's wording while it runs.
    pub fn wording(self) -> &'static str {
        match self {
            Busy::ClosingDay => "Closing the day…",
            Busy::FreshStart => "Starting fresh…",
            Busy::Upgrade => "Building the upgrade…",
        }
    }

    /// The notice that shows when it is done.
    pub fn done(self) -> &'static str {
        match self {
            Busy::ClosingDay => "day closed",
            Busy::FreshStart => "fresh start ready",
            Busy::Upgrade => "upgrade build finished; see Needs you",
        }
    }

    /// Which command the snapshot says is running, if any.
    fn of(snap: &Snapshot) -> Option<Busy> {
        match &snap.journal {
            Some(j) if j.closing && j.fresh => Some(Busy::FreshStart),
            Some(j) if j.closing => Some(Busy::ClosingDay),
            _ if snap.upgrade_building => Some(Busy::Upgrade),
            _ => None,
        }
    }
}

/// When each indicator began, kept by the UI because the core sends states, not durations.
#[derive(Debug, Default)]
pub struct Activity {
    /// When each working agent was first seen working.
    working: HashMap<AgentId, i64>,
    /// The running long command and when it was first seen.
    busy: Option<(Busy, i64)>,
}

impl Activity {
    /// Follows the snapshot: starts clocks for agents that began working, drops them for the
    /// ones that stopped. Returns the command that has just finished, if one has.
    pub fn sync(&mut self, snap: &Snapshot, now_ms: i64) -> Option<Busy> {
        self.working.retain(|id, _| {
            snap.agents
                .iter()
                .any(|a| &a.id == id && is_working(a.status))
        });
        for agent in snap.agents.iter().filter(|a| is_working(a.status)) {
            self.working.entry(agent.id.clone()).or_insert(now_ms);
        }
        let now_busy = Busy::of(snap);
        let was = self.busy.map(|(b, _)| b);
        if now_busy == was {
            return None;
        }
        self.busy = now_busy.map(|b| (b, now_ms));
        // A command replaced by another one has not "finished" in a way worth a notice.
        was.filter(|_| now_busy.is_none())
    }

    /// How long `agent` has been working at `now_ms`; `None` if it is not.
    pub fn working_for(&self, agent: &AgentId, now_ms: i64) -> Option<i64> {
        self.working.get(agent).map(|since| (now_ms - since).max(0))
    }

    /// The running long command and how long it has run.
    pub fn busy(&self, now_ms: i64) -> Option<(Busy, i64)> {
        self.busy.map(|(b, since)| (b, (now_ms - since).max(0)))
    }
}

fn is_working(status: AgentStatus) -> bool {
    status == AgentStatus::Working
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, arg: &str) -> RunningTool {
        RunningTool {
            name: name.into(),
            arg: arg.into(),
        }
    }

    #[test]
    fn the_spinner_turns_every_frame_and_wraps() {
        assert_eq!(spinner(0), SPINNER[0]);
        assert_eq!(spinner(FRAME_MS - 1), SPINNER[0]);
        assert_eq!(spinner(FRAME_MS), SPINNER[1]);
        assert_eq!(spinner(FRAME_MS * SPINNER.len() as i64), SPINNER[0]);
    }

    #[test]
    fn the_next_frame_is_never_more_than_one_frame_away() {
        assert_eq!(next_frame_in(0), Duration::from_millis(125));
        assert_eq!(next_frame_in(100), Duration::from_millis(25));
        assert_eq!(next_frame_in(124), Duration::from_millis(1));
        assert_eq!(next_frame_in(125), Duration::from_millis(125));
    }

    #[test]
    fn elapsed_reads_as_seconds_minutes_hours() {
        assert_eq!(elapsed(-5), "0s");
        assert_eq!(elapsed(999), "0s");
        assert_eq!(elapsed(12_400), "12s");
        assert_eq!(elapsed(65_000), "1m 05s");
        assert_eq!(elapsed(3_725_000), "1h 02m");
    }

    #[test]
    fn a_turn_keeps_its_word_and_words_differ_between_turns() {
        assert_eq!(word(7), word(7));
        assert_eq!(word(0), "Dovetailing");
        assert_eq!(word(16), word(0));
        assert_ne!(word(1), word(2));
    }

    #[test]
    fn the_word_list_is_the_one_the_user_chose() {
        assert_eq!(WORDS.len(), 16);
        assert!(WORDS.contains(&"Gluing up") && WORDS.contains(&"Marking out"));
    }

    #[test]
    fn a_running_tool_is_named_by_what_it_does() {
        assert_eq!(
            label(Some(&tool("Read", "/repo/docs/plan.md")), 0),
            "Reading plan.md…"
        );
        assert_eq!(
            label(Some(&tool("Bash", "cargo test")), 0),
            "Running cargo test…"
        );
        assert_eq!(
            label(Some(&tool("Edit", "crates/dovetail-tui/src/keymap.rs")), 0),
            "Editing keymap.rs…"
        );
        assert_eq!(
            label(Some(&tool("mcp__dovetail__core_ask_user", "")), 0),
            "Calling ask_user…"
        );
        assert_eq!(
            label(Some(&tool("invoke_subagent", "Line Counter")), 0),
            "Waiting on subagent Line Counter…"
        );
    }

    #[test]
    fn a_long_command_is_cut_with_an_ellipsis() {
        let long = "cargo test --workspace --all-targets -- --nocapture --test-threads=1";
        let text = label(Some(&tool("Bash", long)), 0);
        assert!(text.starts_with("Running cargo test --workspace"));
        assert!(text.contains("…"));
        assert!(unicode_width::UnicodeWidthStr::width(text.as_str()) < 60);
    }

    #[test]
    fn with_no_tool_the_label_is_the_turns_word() {
        assert_eq!(label(None, 1), "Planing…");
    }
}
