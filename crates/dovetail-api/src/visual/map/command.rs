//! The `map` feature's commands.

use crate::map::MapId;
use serde::{Deserialize, Serialize};

/// Maps: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapCommand {
    /// Fetch a map's content. The core answers with [`super::MapEvent::Loaded`], or
    /// [`crate::app::AppEvent::CommandFailed`] for an unknown map or version.
    Load {
        /// The map.
        id: MapId,
        /// The version; `None` means the latest.
        version: Option<u32>,
    },
}
