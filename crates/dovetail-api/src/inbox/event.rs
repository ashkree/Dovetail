//! The `inbox` feature's events.

use serde::{Deserialize, Serialize};
use crate::inbox::{InboxId, InboxItem};

/// A change to the client's snapshot, or an answer, in the feature `inbox`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxEvent {
    /// An item arrived in Needs you, or an existing one changed.
    Upserted(InboxItem),
    /// An item left Needs you.
    Resolved {
        /// The item.
        id: InboxId,
    },
}
