//! The reducer: the only way client state changes.

use crate::model::{
    ACTIVITY_TAIL, CONVERSATION_KEEP, ConversationEntry, ConversationTail, EntryKind, MESSAGE_TAIL,
    Snapshot,
};
use crate::protocol::Event;

/// What [`apply`] did with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// The event was folded in.
    Applied,
    /// An event was missed; the state is unchanged and the client should send `Resync`.
    Gap {
        /// The `seq` the client was waiting for.
        expected: u64,
        /// The `seq` that arrived.
        got: u64,
    },
    /// The event is already in the state; it was ignored.
    Stale,
}

/// Folds one event into `state`. Pure, and shared by the UI, `MockCore` and tests.
///
/// Only the event with `seq == state.seq + 1` applies. A later one is a [`ApplyResult::Gap`] and
/// an earlier one is [`ApplyResult::Stale`]; neither changes `state`.
pub fn apply(state: &mut Snapshot, seq: u64, event: Event) -> ApplyResult {
    let expected = state.seq + 1;
    if seq < expected {
        return ApplyResult::Stale;
    }
    if seq > expected {
        return ApplyResult::Gap { expected, got: seq };
    }
    state.seq = seq;

    match event {
        Event::TaskUpserted(task) => upsert(&mut state.tasks, task, |t| t.id),
        Event::TaskRemoved(id) => state.tasks.retain(|t| t.id != id),
        Event::AgentUpserted(agent) => upsert(&mut state.agents, agent, |a| a.id.clone()),
        Event::AgentRemoved(id) => {
            state.agents.retain(|a| a.id != id);
            state.conversations.remove(&id);
            state.worktrees.retain(|w| w.agent != id);
            state.commands.remove(&id);
        }
        Event::WorktreeUpserted(info) => upsert(&mut state.worktrees, info, |w| w.agent.clone()),
        Event::TodosReplaced { list, items } => {
            state.todos.retain(|t| t.list != list);
            state.todos.extend(items);
        }
        Event::InboxUpserted(item) => upsert(&mut state.inbox, item, |i| i.id),
        Event::InboxResolved { id } => state.inbox.retain(|i| i.id != id),
        Event::ActivityAdded(item) => push_bounded(&mut state.activity, item, ACTIVITY_TAIL),
        Event::MessageAdded(message) => push_bounded(&mut state.messages, message, MESSAGE_TAIL),
        Event::ConversationAppended { agent, entry } => {
            let tail = state.conversations.entry(agent).or_default();
            match tail.entries.iter_mut().find(|e| e.id == entry.id) {
                Some(existing) => *existing = entry,
                None => append_entry(tail, entry),
            }
        }
        Event::AssistantDelta { agent, entry, text } => {
            let now_ms = state.now_ms;
            let tail = state.conversations.entry(agent).or_default();
            match tail.entries.iter_mut().find(|e| e.id == entry) {
                Some(existing) => existing.text.push_str(&text),
                // The core always announces an entry before streaming into it, but a client that
                // joined mid-stream keeps the text rather than dropping it.
                None => append_entry(
                    tail,
                    ConversationEntry {
                        id: entry,
                        at_ms: now_ms,
                        kind: EntryKind::Assistant,
                        text,
                        attachments: Vec::new(),
                    },
                ),
            }
        }
        Event::ConversationPage {
            agent,
            entries,
            has_more,
        } => {
            let tail = state.conversations.entry(agent).or_default();
            let mut older: Vec<_> = entries
                .into_iter()
                .filter(|e| !tail.entries.iter().any(|t| t.id == e.id))
                .collect();
            older.append(&mut tail.entries);
            tail.entries = older;
            tail.has_more = has_more;
        }
        Event::DecisionAdded(decision) => upsert(&mut state.decisions, decision, |d| d.id),
        Event::MapUpserted(map) => {
            // Most recently drawn first: a redrawn map moves to the front.
            state.maps.retain(|m| m.id != map.id);
            state.maps.insert(0, map);
        }
        Event::ResearchUpserted(summary) => upsert(&mut state.research, summary, |r| r.task),
        Event::TangentUpserted(tangent) => upsert(&mut state.tangents, tangent, |t| t.task.id),
        Event::TangentRemoved(id) => state.tangents.retain(|t| t.task.id != id),
        Event::TotalsChanged(totals) => state.totals = totals,
        Event::UsageChanged(usage) => state.usage = usage,
        Event::WarningSet(warning) => upsert(&mut state.warnings, warning, |w| w.id.clone()),
        Event::WarningCleared(id) => state.warnings.retain(|w| w.id != id),
        Event::CommandsChanged { agent, commands } => {
            state.commands.insert(agent, commands);
        }
        Event::JournalChanged(journal) => state.journal = Some(journal),
        Event::UpgradeBuilding(on) => state.upgrade_building = on,
        // Nothing to store: the UI shows the message as the event passes through.
        Event::CommandFailed { .. }
        | Event::MapLoaded(_)
        | Event::ResearchLoaded { .. }
        | Event::TangentLoaded { .. }
        | Event::DocsListed { .. }
        | Event::DocLoaded { .. }
        | Event::ProjectBrowsed(_)
        | Event::BrowsedLoaded { .. }
        | Event::ClipboardPasted { .. }
        | Event::PathsListed { .. }
        | Event::TerminalReturned { .. }
        | Event::LendTerminal
        | Event::UpgradeRestarting
        | Event::OpeningProject { .. }
        | Event::RecipeStarted { .. } => {}
    }
    ApplyResult::Applied
}

/// Replaces the item with the same key, or appends it.
fn upsert<T, K: PartialEq>(items: &mut Vec<T>, item: T, key: impl Fn(&T) -> K) {
    let k = key(&item);
    match items.iter_mut().find(|existing| key(existing) == k) {
        Some(existing) => *existing = item,
        None => items.push(item),
    }
}

/// Appends, dropping the oldest items beyond `cap`.
fn push_bounded<T>(items: &mut Vec<T>, item: T, cap: usize) {
    items.push(item);
    if items.len() > cap {
        let excess = items.len() - cap;
        items.drain(..excess);
    }
}

/// Appends to a conversation, dropping its oldest entries beyond [`CONVERSATION_KEEP`]. History
/// paged in with `ConversationPage` stays until then; snapshots carry only the tail
/// ([`Snapshot::trimmed`](crate::Snapshot::trimmed)).
fn append_entry(tail: &mut ConversationTail, entry: ConversationEntry) {
    let before = tail.entries.len();
    push_bounded(&mut tail.entries, entry, CONVERSATION_KEEP);
    if tail.entries.len() == before {
        tail.has_more = true;
    }
}
