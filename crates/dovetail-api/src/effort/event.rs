//! The `effort` feature's events.

use crate::effort::Effort;
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `effort`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortEvent {
    /// An effort was created or changed: its fields, its "where was I" card or its criteria.
    /// It carries the whole effort. Efforts are never removed.
    Upserted(Effort),
}
