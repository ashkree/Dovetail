//! The `model` feature's events.

use crate::model::ModelOption;
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEvent {
    /// The models the selector offers changed: config was read, or a session listed what its
    /// models accept.
    Changed(Vec<ModelOption>),
}
