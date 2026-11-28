//! Activity domain: inbox items, activity/message feeds, warnings, totals, and usage limits.

pub(crate) mod inbox;
pub(crate) mod usage;

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;

use dovetail_api::activity::{
    ActivityEvent, ActivityItem, ActivityKind, AgentMessage,
    inbox::{InboxCommand, InboxId, InboxKind},
    status::{DriverUsage, StatusCommand, StatusEvent, Totals, UsageLimits, Warning},
};
use dovetail_api::agents::{agent::AgentId, conversation::EntryKind};
use dovetail_api::snapshot::{ACTIVITY_TAIL, MESSAGE_TAIL};
use dovetail_api::{Event, Snapshot};
use dovetail_tools::{SocketResponseBody, report::AskUser};

use crate::activity::inbox::Waiter;
use crate::engine::{Core, InboxEntry};
use crate::mcp::tools::ok;
use crate::storage::now_ms;

/// Most message ids remembered to drop duplicates.
const MAX_SEEN_MESSAGES: usize = 4096;

/// State owned by the activity domain: Needs you inbox, activity/message feeds, warnings, totals,
/// and per-account / agy usage limits.
pub(crate) struct ActivityState {
    pub(crate) inbox: BTreeMap<InboxId, InboxEntry>,
    pub(crate) next_inbox: u64,
    pub(crate) items: VecDeque<ActivityItem>,
    pub(crate) next_activity: u64,
    pub(crate) messages: VecDeque<AgentMessage>,
    pub(crate) next_message: u64,
    /// `msg_id`s already on the message timeline, so sender and receiver don't both add one.
    pub(crate) seen_messages: HashSet<String>,
    pub(crate) warnings: Vec<Warning>,
    pub(crate) totals: Totals,
    /// Usage limits per Claude home (limits are per account, task #16 and ADR 0009), as last
    /// reported by a session running under it.
    pub(crate) usage: BTreeMap<PathBuf, UsageLimits>,
    /// Cached latest usage report for the agy driver (task #58).
    pub(crate) agy_usage: Option<DriverUsage>,
    /// Whether an agy usage refresh is currently in flight.
    pub(crate) agy_usage_in_flight: bool,
    /// When agy usage last successfully refreshed, for 3-minute throttling.
    pub(crate) agy_usage_last_refreshed: Option<tokio::time::Instant>,
}

impl Default for ActivityState {
    fn default() -> Self {
        Self {
            inbox: BTreeMap::new(),
            next_inbox: 1,
            items: VecDeque::new(),
            next_activity: 1,
            messages: VecDeque::new(),
            next_message: 1,
            seen_messages: HashSet::new(),
            warnings: Vec::new(),
            totals: Totals::default(),
            usage: BTreeMap::new(),
            agy_usage: None,
            agy_usage_in_flight: false,
            agy_usage_last_refreshed: None,
        }
    }
}

pub(crate) fn totals_of(state: &Snapshot) -> Totals {
    Totals {
        memory_mb: state.agents.iter().filter_map(|a| a.memory_mb).sum(),
        // Finished turns are listed but not counted (ADR 0028).
        needs_you: state
            .inbox
            .iter()
            .filter(|i| !matches!(i.kind, InboxKind::FinishedTurn { .. }))
            .count() as u32,
        cost_usd: (state.agents.iter().map(|a| a.cost_usd).sum::<f64>() * 100.0).round() / 100.0,
    }
}

impl Core {
    pub(crate) async fn handle_inbox_command(&mut self, command: InboxCommand) {
        match command {
            InboxCommand::AnswerPermission { item, answer } => {
                self.answer_permission(item, answer).await;
            }
            InboxCommand::AnswerQuestion { item, text } => {
                self.answer_question(item, text).await;
            }
            InboxCommand::Dismiss { item } => self.dismiss(item).await,
        }
    }

    pub(crate) fn handle_status_command(&mut self, command: StatusCommand) {
        match command {
            StatusCommand::LoadUsage => self.load_usage(),
        }
    }

    pub(crate) fn handle_ask_user_tool(
        &mut self,
        agent: &AgentId,
        ask: AskUser,
    ) -> SocketResponseBody {
        let kind = InboxKind::Question {
            text: ask.question.clone(),
        };
        // Shown in the conversation too, where it is easier to read than in Needs you.
        self.add_entry(
            agent,
            EntryKind::Notice,
            format!("Question for you (also in Needs you): {}", ask.question),
        );
        self.raise(
            agent,
            kind,
            Waiter::Question {
                question: ask.question,
            },
        );
        ok(
            "Question sent; the answer will arrive as a new message. Carry on with \
            anything that does not depend on it, or end your turn if nothing is left.",
        )
    }

    pub(crate) fn refresh_totals(&mut self) {
        let snapshot = Snapshot {
            agents: self.agents.rows.values().map(|r| r.agent.clone()).collect(),
            inbox: self
                .activity
                .inbox
                .values()
                .map(|p| p.item.clone())
                .collect(),
            ..Snapshot::default()
        };
        let totals = totals_of(&snapshot);
        if totals != self.activity.totals {
            self.activity.totals = totals.clone();
            self.emit(Event::Status(StatusEvent::TotalsChanged(totals)));
        }
    }

    pub(crate) fn activity(&mut self, kind: ActivityKind, agent: Option<&AgentId>, text: String) {
        let item = ActivityItem {
            id: self.activity.next_activity,
            at_ms: now_ms(),
            agent: agent.cloned(),
            kind,
            text,
        };
        self.activity.next_activity += 1;
        self.activity.items.push_back(item.clone());
        while self.activity.items.len() > ACTIVITY_TAIL {
            self.activity.items.pop_front();
        }
        self.persist("an activity item", self.db.add_activity(item.clone()));
        self.emit(Event::Activity(ActivityEvent::Added(item)));
    }

    pub(crate) fn message(
        &mut self,
        from: AgentId,
        to: AgentId,
        text: String,
        msg_id: Option<String>,
    ) {
        if let Some(id) = &msg_id
            && !self.activity.seen_messages.insert(id.clone())
        {
            return;
        }
        // Old ids do not come back, so forgetting them all now and then is safe.
        if self.activity.seen_messages.len() > MAX_SEEN_MESSAGES {
            self.activity.seen_messages.clear();
        }
        let message = AgentMessage {
            id: self.activity.next_message,
            at_ms: now_ms(),
            from,
            to,
            text,
        };
        self.activity.next_message += 1;
        self.activity.messages.push_back(message.clone());
        while self.activity.messages.len() > MESSAGE_TAIL {
            self.activity.messages.pop_front();
        }
        self.persist("a message", self.db.add_message(message.clone(), msg_id));
        self.emit(Event::Activity(ActivityEvent::MessageAdded(message)));
    }
}
