//! The `journal` feature's events.

use serde::{Deserialize, Serialize};
use crate::journal::JournalState;

/// A change to the client's snapshot, or an answer, in the feature `journal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalEvent {
    /// The end-of-day journal changed: `:eod` started waiting for the handoff, or a day's file
    /// was written (ADR 0012).
    Changed(JournalState),
}
