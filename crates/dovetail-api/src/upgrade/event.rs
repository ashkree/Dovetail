//! The `upgrade` feature's events.

use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `upgrade`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeEvent {
    /// `:upgrade` started building the next binary (`true`) or the build finished, failed or was
    /// cancelled (`false`); the result arrives as a Needs-you item.
    Building(bool),
    /// Every agent is idle and the user said `y` on "Upgrade ready" (ADR 0014): the core is
    /// quitting, and the UI quits too, without asking. The binary then swaps the staged build in
    /// and starts it in the same terminal. Changes nothing in the snapshot.
    Restarting,
}
