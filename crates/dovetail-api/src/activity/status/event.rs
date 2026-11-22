//! The `status` feature's events.

use super::{DriverUsage, Totals, UsageLimits, Warning, WarningId};
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
    /// A session reported new usage-limit figures.
    UsageChanged(UsageLimits),
    /// Every driver's usage limits, in [`crate::model::DriverKind::ALL`] order, for the usage
    /// menu ([`super::StatusCommand::LoadUsage`], task #46). Changes nothing in the snapshot.
    UsageLoaded(Vec<DriverUsage>),
}
