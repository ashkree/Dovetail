//! The `activity` feature's events.

use crate::activity::{ActivityItem, AgentMessage};
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `activity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEvent {
    /// Something happened in the Activity feed.
    Added(ActivityItem),
    /// Agents messaged each other.
    MessageAdded(AgentMessage),
}
