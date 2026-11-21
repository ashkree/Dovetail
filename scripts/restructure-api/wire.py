import sys
sys.path.insert(0, '/tmp/restructure')
from spec import CMD, EVT

cf = []
for v, (f, n) in CMD.items():
    if f not in cf:
        cf.append(f)
ef = []
for v, (f, n) in EVT.items():
    if f not in ef:
        ef.append(f)
C = lambda f: f.capitalize()
old = open('wire.rs').read()
body = old.split('use crate::snapshot::Snapshot;\n', 1)[1]
imports = ["use std::sync::mpsc::{Receiver, Sender};", "", "use serde::{Deserialize, Serialize};", "",
           "use crate::snapshot::Snapshot;"]
feats = sorted(set(cf) | set(ef))
for f in feats:
    names = []
    if f in cf:
        names.append(f"{C(f)}Command")
    if f in ef:
        names.append(f"{C(f)}Event")
    imports.append(f"use crate::{f}::{{{', '.join(names)}}};" if len(names) > 1 else f"use crate::{f}::{names[0]};")
out = ("//! What crosses between the core and a client: the stream, the commands, the events.\n\n"
       + "\n".join(imports) + "\n" + body.rstrip("\n") + "\n")
DOC = {
    "task": "the user's tasks", "decision": "decision logs", "agent": "agents and their to-do lists",
    "conversation": "conversations and the chat bar", "inbox": "Needs you", "model": "models",
    "mcp": "the MCP menu", "journal": "the journal", "tangent": "tangents", "terminal": "the terminal",
    "clipboard": "the clipboard", "upgrade": "upgrades", "canvas": "canvases",
    "design": "design previews and variants", "effort": "efforts", "worktree": "worktrees",
    "project": "projects", "map": "maps", "research": "research", "library": "the library",
    "app": "the app", "activity": "the Activity feed", "status": "the status bar",
    "usage": "usage limits"}
out += ("\n/// Something the UI asks the core to do: one variant per feature, each holding that feature's\n"
        "/// commands.\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
        "#[serde(rename_all = \"snake_case\")]\npub enum Command {\n")
for f in cf:
    out += f"    /// A command about {DOC[f]}.\n    {C(f)}({C(f)}Command),\n"
out += "}\n"
out += ("\n/// One change to the client's [`Snapshot`], or an answer to a command: one variant per feature,\n"
        "/// each holding that feature's events.\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
        "#[serde(rename_all = \"snake_case\")]\npub enum Event {\n")
for f in ef:
    out += f"    /// An event about {DOC[f]}.\n    {C(f)}({C(f)}Event),\n"
out += "}\n"
open('wire.rs', 'w').write(out)

NOAPPLY = ('library', 'mcp', 'app', 'clipboard', 'terminal')
feats_apply = [f for f in ef if f not in NOAPPLY]
apply = '''//! The reducer: the only way client state changes.
//!
//! This file checks the sequence number and hands each event to the feature it belongs to
//! (`<feature>/apply.rs`). It also holds the two helpers those reducers share.

use crate::snapshot::Snapshot;
use crate::wire::Event;
use crate::{
'''
apply += "    " + ", ".join(sorted(feats_apply)) + ",\n};\n"
apply += '''
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
'''
for f in ef:
    if f in feats_apply:
        apply += f"        Event::{C(f)}(event) => {f}::apply(state, event),\n"
apply += '''        // Nothing to store: the UI shows these as they pass through.
        Event::Library(_) | Event::Mcp(_) | Event::App(_) | Event::Clipboard(_) | Event::Terminal(_) => {}
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
'''
open('apply.rs', 'w').write(apply)
