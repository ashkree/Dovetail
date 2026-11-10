//! The `draw_map` tool's input, as an agent sends it (design 23i).
//!
//! Everything [`crate::validate`] judges is kept loose here (kinds and types as strings, text
//! fields defaulting to empty), so a missing field or an unknown kind comes back as one of our
//! specific errors alongside all the others, not as a single serde error. Unknown fields are
//! still refused by serde, so a misspelt field name is never silently dropped.

use serde::{Deserialize, Serialize};

/// The whole `draw_map` input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawMapInput {
    /// Id, title and type.
    #[serde(default)]
    pub map: MapHeaderInput,
    /// Every node on the map.
    #[serde(default)]
    pub nodes: Vec<NodeInput>,
    /// Which nodes sit inside which (structure maps only).
    #[serde(default)]
    pub groups: Vec<GroupInput>,
    /// Labelled arrows between nodes (structure maps only).
    #[serde(default)]
    pub connections: Vec<ConnectionInput>,
    /// Guided flows; a swimlane map has exactly one.
    #[serde(default)]
    pub flows: Vec<FlowInput>,
    /// Ordered participants, top to bottom (swimlane maps only).
    #[serde(default)]
    pub lanes: Vec<LaneInput>,
}

/// The `map` object: what the map is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapHeaderInput {
    /// An existing map's id to update it in place; omitted for a new map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Shown on the map shelf and the canvas header.
    #[serde(default)]
    pub title: String,
    /// `structure` or `swimlane`.
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// One node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeInput {
    /// Unique within the map; referenced by groups, connections and flow steps.
    #[serde(default)]
    pub id: String,
    /// One of the nine kinds (decision 23d), any case.
    #[serde(default)]
    pub kind: String,
    /// Short name drawn in the box.
    #[serde(default)]
    pub label: String,
    /// Plain-English description shown in the side panel.
    #[serde(default)]
    pub description: String,
    /// Path of the file the node stands for, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// A few lines of plain pseudocode; required for `logic` nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pseudocode: Option<String>,
}

/// A group: `parent` is the node you drill into, `children` the nodes inside it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupInput {
    /// The containing node's id.
    #[serde(default)]
    pub parent: String,
    /// Ids of the nodes inside it.
    #[serde(default)]
    pub children: Vec<String>,
}

/// A labelled arrow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInput {
    /// Source node id.
    #[serde(default)]
    pub from: String,
    /// Target node id.
    #[serde(default)]
    pub to: String,
    /// What the arrow means: "calls", "reads from".
    #[serde(default)]
    pub label: String,
}

/// A guided flow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowInput {
    /// Unique within the map.
    #[serde(default)]
    pub name: String,
    /// In order.
    #[serde(default)]
    pub steps: Vec<StepInput>,
}

/// One step of a flow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepInput {
    /// The node this step is at.
    #[serde(default)]
    pub node: String,
    /// Label of the arrow into this step from the one before ("submits", "returns 200").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow: Option<String>,
    /// What happens at this step, shown in the side panel.
    #[serde(default)]
    pub note: String,
    /// The lane this step sits in (swimlane maps only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
}

/// A swimlane participant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneInput {
    /// Unique within the map; named by steps.
    #[serde(default)]
    pub id: String,
    /// Shown at the left of the band.
    #[serde(default)]
    pub label: String,
}
