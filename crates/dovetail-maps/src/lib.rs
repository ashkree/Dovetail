//! Dovetail's visual maps (decision 23): validation of `draw_map` input into the map model.
//!
//! An agent sends a [`DrawMapInput`]; [`validate`] checks it and returns either a [`ValidMap`]
//! or every problem it found as [`MapError`]s, each worded so the agent can fix its input.
//! Storage then gives the map its id and version ([`ValidMap::into_map`]).
//!
//! The model types themselves are plain data in `dovetail_api::map`, re-exported here, so the UI
//! can hold map content without depending on this crate (and the renderer it will carry).
//! No IO, no async here; the `MapRenderer`, Mermaid conversion and swimlane layout arrive in
//! later M15 slices.

mod input;
mod validate;

pub use dovetail_api::map::*;
pub use input::{
    ConnectionInput, DrawMapInput, FlowInput, GroupInput, LaneInput, MapHeaderInput, NodeInput,
    StepInput,
};
pub use validate::{MapError, ValidMap, limits, report, validate};
