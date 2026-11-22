//! How `conversation` events change the snapshot.

use super::ConversationEvent;
use super::{ConversationEntry, ConversationTail, EntryKind};
use crate::apply::push_bounded;
use crate::snapshot::CONVERSATION_KEEP;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: ConversationEvent) {
    match event {
        ConversationEvent::Appended { agent, entry } => {
            let tail = state.conversations.entry(agent).or_default();
            match tail.entries.iter_mut().find(|e| e.id == entry.id) {
                Some(existing) => *existing = entry,
                None => append_entry(tail, entry),
            }
        }
        ConversationEvent::AssistantDelta { agent, entry, text } => {
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
                        effort: None,
                    },
                ),
            }
        }
        ConversationEvent::Page {
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
        // Nothing to store: the chat bar takes the paths as the event passes through.
        ConversationEvent::PathsListed { .. } => {}
    }
}

/// Appends to a conversation, dropping its oldest entries beyond [`CONVERSATION_KEEP`]. History
/// paged in with `Page` stays until then; snapshots carry only the tail
/// ([`Snapshot::trimmed`](crate::snapshot::Snapshot::trimmed)).
fn append_entry(tail: &mut ConversationTail, entry: ConversationEntry) {
    let before = tail.entries.len();
    push_bounded(&mut tail.entries, entry, CONVERSATION_KEEP);
    if tail.entries.len() == before {
        tail.has_more = true;
    }
}
