//! The reducer: the only way client state changes.
//!
//! This file checks the sequence number and hands each event to the feature it belongs to
//! (`<feature>/apply.rs`). It also holds the two helpers those reducers share.

use crate::snapshot::Snapshot;
use crate::wire::Event;
use crate::{
    activity, agent, canvas, conversation, decision, design, effort, inbox, journal, map, model,
    project, research, status, tangent, task, upgrade, usage, worktree,
};

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
        Event::Task(event) => task::apply(state, event),
        Event::Effort(event) => effort::apply(state, event),
        Event::Design(event) => design::apply(state, event),
        Event::Canvas(event) => canvas::apply(state, event),
        Event::Agent(event) => agent::apply(state, event),
        Event::Inbox(event) => inbox::apply(state, event),
        Event::Worktree(event) => worktree::apply(state, event),
        Event::Activity(event) => activity::apply(state, event),
        Event::Conversation(event) => conversation::apply(state, event),
        Event::Status(event) => status::apply(state, event),
        Event::Usage(event) => usage::apply(state, event),
        Event::Decision(event) => decision::apply(state, event),
        Event::Map(event) => map::apply(state, event),
        Event::Research(event) => research::apply(state, event),
        Event::Tangent(event) => tangent::apply(state, event),
        Event::Model(event) => model::apply(state, event),
        Event::Journal(event) => journal::apply(state, event),
        Event::Upgrade(event) => upgrade::apply(state, event),
        Event::Project(event) => project::apply(state, event),
        // Nothing to store: the UI shows these as they pass through.
        Event::Library(_)
        | Event::Mcp(_)
        | Event::App(_)
        | Event::Clipboard(_)
        | Event::Terminal(_) => {}
    }
    ApplyResult::Applied
}

/// Replaces the item with the same key, or appends it.
pub(crate) fn upsert<T, K: PartialEq>(items: &mut Vec<T>, item: T, key: impl Fn(&T) -> K) {
    let k = key(&item);
    match items.iter_mut().find(|existing| key(existing) == k) {
        Some(existing) => *existing = item,
        None => items.push(item),
    }
}

/// Appends, dropping the oldest items beyond `cap`.
pub(crate) fn push_bounded<T>(items: &mut Vec<T>, item: T, cap: usize) {
    items.push(item);
    if items.len() > cap {
        let excess = items.len() - cap;
        items.drain(..excess);
    }
}
