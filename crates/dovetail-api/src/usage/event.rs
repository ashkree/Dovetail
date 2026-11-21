//! The `usage` feature's events.

use crate::usage::UsageLimits;
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `usage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageEvent {
    /// A session reported new usage-limit figures.
    Changed(UsageLimits),
}
