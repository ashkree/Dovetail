//! The `map` feature: drawn diagrams and their versions.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::MapCommand;
pub use event::MapEvent;

use crate::agent::AgentId;
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// A map's id, stable across versions (decision 23h).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MapId(pub String);

/// A node's id, unique within its map.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub String);

/// A lane's id, unique within its map.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LaneId(pub String);

/// The two map types (decision 23e).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapKind {
    /// What exists and how it connects: nested groups and labelled connections.
    Structure,
    /// What happens in order: one flow laid out as lanes × steps.
    Swimlane,
}

impl MapKind {
    /// Both types, in the order the docs list them.
    pub const ALL: [MapKind; 2] = [MapKind::Structure, MapKind::Swimlane];

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            MapKind::Structure => "structure",
            MapKind::Swimlane => "swimlane",
        }
    }

    /// Parses a wire name, ignoring case and surrounding space.
    pub fn parse(value: &str) -> Option<MapKind> {
        let value = value.trim();
        MapKind::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(value))
    }
}

/// The fixed nine node kinds (decision 23d); anything else is a [`NodeKind::Concept`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A service, package or module; drills in.
    Part,
    /// A source file; opens it.
    File,
    /// A route, screen or CLI command.
    Endpoint,
    /// A table, cache, queue or config.
    Data,
    /// Another system: Stripe, SendGrid.
    External,
    /// A user, admin or other role.
    Actor,
    /// A result: pass, fail or a value.
    Outcome,
    /// A few lines of plain pseudocode.
    Logic,
    /// A free-form idea.
    Concept,
}

impl NodeKind {
    /// All nine, in the docs' order (the legend's order too).
    pub const ALL: [NodeKind; 9] = [
        NodeKind::Part,
        NodeKind::File,
        NodeKind::Endpoint,
        NodeKind::Data,
        NodeKind::External,
        NodeKind::Actor,
        NodeKind::Outcome,
        NodeKind::Logic,
        NodeKind::Concept,
    ];

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Part => "part",
            NodeKind::File => "file",
            NodeKind::Endpoint => "endpoint",
            NodeKind::Data => "data",
            NodeKind::External => "external",
            NodeKind::Actor => "actor",
            NodeKind::Outcome => "outcome",
            NodeKind::Logic => "logic",
            NodeKind::Concept => "concept",
        }
    }

    /// Parses a wire name, ignoring case and surrounding space.
    pub fn parse(value: &str) -> Option<NodeKind> {
        let value = value.trim();
        NodeKind::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(value))
    }
}

/// A stored map: one version of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Map {
    /// Stable across versions.
    pub id: MapId,
    /// 1 for a new map, one more on every redraw with the same id.
    pub version: u32,
    /// The map itself.
    pub content: MapContent,
}

/// Everything drawn on a map; what a stored version holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapContent {
    /// Shown on the shelf and in the header.
    pub title: String,
    /// Structure or swimlane.
    pub kind: MapKind,
    /// In input order.
    pub nodes: Vec<Node>,
    /// Empty on swimlane maps.
    pub groups: Vec<Group>,
    /// Empty on swimlane maps.
    pub connections: Vec<Connection>,
    /// Exactly one on swimlane maps.
    pub flows: Vec<Flow>,
    /// Top to bottom; empty on structure maps.
    pub lanes: Vec<Lane>,
}

/// A node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Unique within the map.
    pub id: NodeId,
    /// Decides the shape and colour.
    pub kind: NodeKind,
    /// Drawn in the box.
    pub label: String,
    /// Shown in the side panel; may be empty for imported maps.
    pub description: String,
    /// The file the node stands for.
    pub file: Option<String>,
    /// Shown in the side panel; always set on [`NodeKind::Logic`] nodes.
    pub pseudocode: Option<String>,
}

/// The nodes inside a node. A node sits in at most one group, and groups never form a cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The node you drill into.
    pub parent: NodeId,
    /// The nodes inside it, in input order.
    pub children: Vec<NodeId>,
}

/// A labelled arrow between two nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Source.
    pub from: NodeId,
    /// Target.
    pub to: NodeId,
    /// What it means.
    pub label: String,
}

/// A guided flow: an ordered walk through the map (decision 23g).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    /// Unique within the map.
    pub name: String,
    /// At least one, in order.
    pub steps: Vec<FlowStep>,
}

/// One step of a flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowStep {
    /// Where the step is.
    pub node: NodeId,
    /// Structure maps: index in [`MapContent::connections`] of the connection from the previous
    /// step's node into this one, which the canvas highlights. `None` on the first step and on
    /// swimlane maps.
    pub connection: Option<usize>,
    /// Swimlane maps: label of the arrow into this step from the one before. Always `None` on
    /// structure maps, whose arrows are their connections.
    pub arrow: Option<String>,
    /// What happens here.
    pub note: String,
    /// Set on every step of a swimlane map, never on a structure map.
    pub lane: Option<LaneId>,
}

/// A swimlane participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    /// Unique within the map.
    pub id: LaneId,
    /// Shown at the left of the band.
    pub label: String,
    /// The node this participant is on a structure map, to jump between the two views (23e).
    /// It names a node of another map, so it is checked across maps, not by validation.
    pub node: Option<NodeId>,
}

/// A map as the shelf lists it: its latest version's header, without the content, so the
/// snapshot stays small. The content comes with `MapCommand::Load`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapSummary {
    /// Stable across versions.
    pub id: MapId,
    /// The latest version's title.
    pub title: String,
    /// Structure or swimlane; never changes.
    pub kind: MapKind,
    /// The task of the agent that first drew it; `None` when the coordinator did.
    pub task: Option<TaskId>,
    /// The agent that drew the latest version.
    pub drawn_by: Option<AgentId>,
    /// The latest version: 1 for a new map.
    pub version: u32,
    /// When version 1 was drawn, in Unix milliseconds.
    pub created_ms: i64,
    /// When the latest version was drawn, in Unix milliseconds.
    pub updated_ms: i64,
}
