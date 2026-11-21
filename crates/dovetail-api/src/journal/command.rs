//! The `journal` feature's commands.

use serde::{Deserialize, Serialize};

/// The journal: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalCommand {
    /// Close the day (`:eod`, ADR 0012): ask the coordinator for its handoff, then write today's
    /// journal file with it and Dovetail's facts. The core answers with
    /// [`JournalEvent::Changed`]: `closing` while it waits, then the new `latest`.
    EndOfDay,
    /// A mid-day handoff (`:handoff`, ADR 0012 amendment): ask the coordinator for the same
    /// handoff as [`JournalCommand::EndOfDay`], append it to today's journal file as a "Handoff (fresh
    /// start HH:MM)" section, and start a new coordinator session seeded with it and the open
    /// tasks. Refused with an error while a turn runs. The previous session stays resumable
    /// through a Needs-you item.
    Handoff,
    /// `:fresh`: a new coordinator session with no handoff and no seed message. Refused while a
    /// turn runs. The previous session stays resumable through the same Needs-you item.
    FreshSession,
}
