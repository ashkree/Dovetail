//! The `app` feature's events.

use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `app`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEvent {
    /// A command could not be carried out; the UI shows the message.
    CommandFailed {
        /// What went wrong.
        message: String,
    },
}
