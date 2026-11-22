//! The full client-visible snapshot and its tail caps.

use crate::activity::{ActivityItem, AgentMessage};
use crate::agent::{Agent, AgentId, SlashCommand, TodoItem};
use crate::canvas::{Block, Card, CardLink};
use crate::conversation::ConversationTail;
use crate::decision::Decision;
use crate::design::{Preview, VariantSet};
use crate::effort::Effort;
use crate::inbox::InboxItem;
use crate::journal::JournalState;
use crate::map::MapSummary;
use crate::model::ModelOption;
use crate::project::ProjectSummary;
use crate::research::ResearchSummary;
use crate::status::UsageLimits;
use crate::status::{Totals, Warning};
use crate::tangent::TangentSummary;
use crate::task::Task;
use crate::worktree::WorktreeInfo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The whole client-visible state at one point in the event stream.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Sequence number of the last event folded into this snapshot.
    pub seq: u64,
    /// Core clock when the snapshot was taken, in Unix milliseconds.
    pub now_ms: i64,
    /// The project open in this window, if any.
    pub project: Option<ProjectSummary>,
    /// Every registered project, for the project switcher.
    pub projects: Vec<ProjectSummary>,
    /// The open project's efforts, closed ones included, in id order.
    #[serde(default)]
    pub efforts: Vec<Effort>,
    /// The blocks on the open project's canvases (one canvas per effort), in id order.
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// The open project's cards, in id order: placed ones and the incoming strip's (no block).
    /// Titles only; a card's body comes with [`crate::canvas::CanvasCommand::LoadCard`].
    #[serde(default)]
    pub cards: Vec<Card>,
    /// The links between cards.
    #[serde(default)]
    pub card_links: Vec<CardLink>,
    /// The open project's tasks, archived ones included.
    pub tasks: Vec<Task>,
    /// Every agent in the open project.
    pub agents: Vec<Agent>,
    /// To-do items from Claude's native lists.
    pub todos: Vec<TodoItem>,
    /// Needs you: everything waiting on the user.
    pub inbox: Vec<InboxItem>,
    /// The Activity feed, oldest first, bounded to [`ACTIVITY_TAIL`] items.
    pub activity: Vec<ActivityItem>,
    /// Messages between agents, oldest first, bounded to [`MESSAGE_TAIL`] items.
    pub messages: Vec<AgentMessage>,
    /// The latest entries of each agent's conversation.
    pub conversations: BTreeMap<AgentId, ConversationTail>,
    /// Status-bar totals.
    pub totals: Totals,
    /// Status-bar warnings.
    pub warnings: Vec<Warning>,
    /// Every task's decision log, oldest first.
    #[serde(default)]
    pub decisions: Vec<Decision>,
    /// The open project's Claude usage limits, as last reported by a session of its profile
    /// (limits are per account).
    #[serde(default)]
    pub usage: UsageLimits,
    /// The Claude profile the open project's sessions run under (ADR 0009); `None` when nothing
    /// names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Git and PR state of each slice's worktree, as last gathered (empty until the first
    /// refresh).
    #[serde(default)]
    pub worktrees: Vec<WorktreeInfo>,
    /// The map shelf: every map of the open project, most recently drawn first. Content is
    /// fetched with [`crate::map::MapCommand::Load`].
    #[serde(default)]
    pub maps: Vec<MapSummary>,
    /// Each task's research (rounds, finding and source counts), for tasks that have any.
    /// Content is fetched with [`crate::research::ResearchCommand::Load`].
    #[serde(default)]
    pub research: Vec<ResearchSummary>,
    /// The slash commands and skills each session offers, as Claude Code last listed them.
    /// Sessions of one project share a profile, so one agent's list stands in for another's
    /// until that one reports.
    #[serde(default)]
    pub commands: BTreeMap<AgentId, Vec<SlashCommand>>,
    /// The project's end-of-day journal (ADR 0012); `None` when the open folder is not a
    /// registered project, so it has no journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal: Option<JournalState>,
    /// `:upgrade` is building the next binary in the background (ADR 0014).
    #[serde(default)]
    pub upgrade_building: bool,
    /// The tangents in Tangents (ADR 0017), whatever project is open: every one that is not
    /// archived, oldest first. Content is fetched with [`crate::tangent::TangentCommand::Load`].
    #[serde(default)]
    pub tangents: Vec<TangentSummary>,
    /// The models the model selector offers, in order (task #39).
    #[serde(default)]
    pub models: Vec<ModelOption>,
    /// Each slice's running preview, for slices the viewer was opened on (ADR 0026).
    #[serde(default)]
    pub previews: Vec<Preview>,
    /// The variants slices declared, with the user's picks (ADR 0026).
    #[serde(default)]
    pub variants: Vec<VariantSet>,
    /// The soft limit on running efforts, from `config.toml`'s `max_efforts`: shown against the
    /// running count, never enforced.
    #[serde(default)]
    pub effort_limit: u32,
}

impl Snapshot {
    /// The snapshot as a core sends it: each conversation cut to its newest
    /// [`CONVERSATION_TAIL`] entries, older ones left for [`crate::conversation::ConversationCommand::Load`]. A client keeps
    /// what it pages in beyond that, up to [`CONVERSATION_KEEP`].
    pub fn trimmed(&self) -> Snapshot {
        let mut out = self.clone();
        for tail in out.conversations.values_mut() {
            if tail.entries.len() > CONVERSATION_TAIL {
                let excess = tail.entries.len() - CONVERSATION_TAIL;
                tail.entries.drain(..excess);
                tail.has_more = true;
            }
        }
        out
    }
}

/// Most activity items a client keeps.
pub const ACTIVITY_TAIL: usize = 500;

/// Most agent messages a client keeps.
pub const MESSAGE_TAIL: usize = 500;

/// Most conversation entries a snapshot carries per agent; older ones are paged in on request.
pub const CONVERSATION_TAIL: usize = 200;

/// Most conversation entries a client keeps per agent, paged-in history included, before the
/// oldest are dropped again.
pub const CONVERSATION_KEEP: usize = 2_000;
