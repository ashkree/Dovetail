//! Dovetail's visual maps (decision 23): the map model and validation of `draw_map` input.
//!
//! An agent sends a [`DrawMapInput`]; [`validate`] checks it and returns either a [`ValidMap`]
//! or every problem it found as [`MapError`]s, each worded so the agent can fix its input.
//! Storage then gives the map its id and version ([`ValidMap::into_map`]).
//!
//! Plain data only: no IO, no async, no rendering. The `MapRenderer`, Mermaid conversion and
//! swimlane layout arrive in later M15 slices.

mod input;
mod model;
mod validate;

pub use input::{
    ConnectionInput, DrawMapInput, FlowInput, GroupInput, LaneInput, MapHeaderInput, NodeInput,
    StepInput,
};
pub use model::{
    Connection, Flow, FlowStep, Group, Lane, LaneId, Map, MapContent, MapId, MapKind, Node, NodeId,
    NodeKind, ValidMap,
};
pub use validate::{MapError, limits, report, validate};
