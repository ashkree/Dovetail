//! The `usage` feature's events.

use serde::{Deserialize, Serialize};
use crate::usage::UsageLimits;

/// A change to the client's snapshot, or an answer, in the feature `usage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageEvent {
    /// A session reported new usage-limit figures.
    Changed(UsageLimits),
}
