//! The `activity` feature's events.

use serde::{Deserialize, Serialize};
use crate::activity::{ActivityItem, AgentMessage};

/// A change to the client's snapshot, or an answer, in the feature `activity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEvent {
    /// Something happened in the Activity feed.
    Added(ActivityItem),
    /// Agents messaged each other.
    MessageAdded(AgentMessage),
}
