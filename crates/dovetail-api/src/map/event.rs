//! The `map` feature's events.

use crate::map::{Map, MapSummary};
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `map`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapEvent {
    /// A map was drawn (version 1) or redrawn (a later version): its shelf entry now.
    Upserted(MapSummary),
    /// A map version's content, in answer to [`super::MapCommand::Load`]. Changes nothing in the
    /// snapshot; the UI keeps what it shows.
    Loaded(Map),
}
