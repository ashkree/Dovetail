//! Tools of the `maps` suite: structure and swimlane maps.

use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::ToolError;

/// Tools in the `maps` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapTool {
    /// `maps_draw` (decision 25, M15).
    Draw(DrawMap),
}

impl MapTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            MapTool::Draw(_) => "maps_draw",
        }
    }

    pub(crate) fn validate(&self, _role: Role) -> Result<(), ToolError> {
        // Checked whole by `dovetail_maps::validate` in the core, so every problem comes back
        // in one report.
        Ok(())
    }
}

/// `maps_draw`: a structure or swimlane map (design 23i). The arguments stay raw JSON here:
/// their shape is `dovetail_maps::DrawMapInput`, which lives with the validation (decision on
/// task #34), and the core parses and validates them there so every problem comes back at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DrawMap(pub serde_json::Value);
