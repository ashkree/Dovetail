//! The `status` feature's events.

use crate::status::{Totals, Warning, WarningId};
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusEvent {
    /// Status-bar totals changed.
    TotalsChanged(Totals),
    /// A warning was raised or reworded.
    WarningSet(Warning),
    /// A warning no longer applies.
    WarningCleared(WarningId),
}
