//! The `decision` feature's events.

use serde::{Deserialize, Serialize};
use crate::decision::Decision;

/// A change to the client's snapshot, or an answer, in the feature `decision`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEvent {
    /// An entry was added to a task's decision log.
    Added(Decision),
}
