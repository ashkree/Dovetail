//! The `inbox` feature: Needs you: everything waiting on the user, and how the user answers.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::{InboxCommand, PermissionAnswer};
pub use event::InboxEvent;

use crate::agent::AgentId;
use crate::decision::Alternative;
use crate::research::{Evidence, ProposalKind};
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// Identifies an inbox item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InboxId(pub u64);

/// An entry in Needs you.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    /// Item id.
    pub id: InboxId,
    /// The agent it came from.
    pub agent: AgentId,
    /// When it arrived, in Unix milliseconds.
    pub at_ms: i64,
    /// What it asks for.
    pub kind: InboxKind,
}

/// What an inbox item asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    /// Allow or deny a tool use.
    Permission {
        /// Tool name.
        tool: String,
        /// Tool input as sent by Claude Code.
        input: serde_json::Value,
        /// Why it was escalated, when given.
        reason: Option<String>,
        /// Rules offered for "always allow".
        suggestions: Vec<RuleSuggestion>,
    },
    /// A `core_ask_user` question.
    Question {
        /// The question.
        text: String,
    },
    /// The agent's turn ended; at most one per agent (ADR 0001).
    FinishedTurn {
        /// Start of its last message.
        preview: String,
    },
    /// A driver error, crash or memory kill.
    Error {
        /// What went wrong.
        text: String,
    },
    /// A research finding of a coordinator subagent that ran with no round open, stored with no
    /// task until the user files it (`y`) or drops it (`d`).
    UnfiledFinding {
        /// The stored finding (its own id sequence until it is filed).
        finding: i64,
        /// Its label, from the subagent's description.
        label: String,
        /// How many sources it fetched or read.
        sources: u32,
    },
    /// Tangents has tangents nobody has triaged yet: raised on the first open of a day (ADR
    /// 0012's morning path). Opening it lists them, to file into a project, start a task from,
    /// or drop (ADR 0017).
    NewTangents {
        /// How many.
        count: u32,
    },
    /// A `research_propose` call: an idea or a decision the coordinator wants the user to accept
    /// (ADR 0015). Nothing is written until `y`.
    Proposal {
        /// The research task (the question).
        task: TaskId,
        /// Idea or decision.
        kind: ProposalKind,
        /// The idea, or the decision in one line.
        text: String,
        /// The findings and sources behind it; never empty for a decision.
        evidence: Vec<Evidence>,
        /// Why, if given.
        #[serde(default)]
        why: Option<String>,
        /// Options considered and not taken (decisions).
        #[serde(default)]
        alternatives: Vec<Alternative>,
    },
}

/// The `tool` of the Permission item Dovetail raises itself for a `project_start_slice` that needs
/// approval (ADR 0004). Its input is `{task, name, brief}`.
pub const START_SLICE_PERMISSION: &str = "start_slice";

/// The `tool` of the Permission item that asks before removing a slice worktree with unsaved
/// work. Its input is `{worktree, branch, uncommitted: [..], unpushed_commits}`.
pub const REMOVE_WORKTREE_PERMISSION: &str = "Remove worktree";

/// The `tool` of the Permission item raised at open when a session's take-over lock was left by
/// a crash and Dovetail cannot tell whether its native `claude` still runs (M14). `y` releases
/// the lock, `d` keeps it. Its input is `{session, reason}`.
pub const RELEASE_TAKEOVER_PERMISSION: &str = "Release take-over lock";

/// The `tool` of the Permission item raised when an upgrade build is staged (ADR 0014): `y`
/// restarts once every agent is idle, `d` leaves the build for the next ordinary quit. Its input
/// is `{source, differing: [..], waiting: [..], restarting}`: where the build came from, the role
/// prompts and recipe whose installed copy differs (never overwritten), the agents `y` still
/// waits for, and whether `y` was pressed.
pub const UPGRADE_READY_PERMISSION: &str = "Upgrade ready";

/// A permission rule Claude Code offers for "always allow".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSuggestion {
    /// Short human-readable form, such as `Bash(cargo test:*)`.
    pub label: String,
    /// The suggestion as Claude Code sent it, to echo back.
    pub raw: serde_json::Value,
}
