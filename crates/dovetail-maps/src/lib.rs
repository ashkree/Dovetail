//! Dovetail's visual maps (decision 23): validation of `draw_map` input into the map model.
//!
//! An agent sends a [`DrawMapInput`]; [`validate`] checks it and returns either a [`ValidMap`]
//! or every problem it found as [`MapError`]s, each worded so the agent can fix its input.
//! Storage then gives the map its id and version ([`ValidMap::into_map`]), after checking lane
//! links against the project's other maps ([`check_lane_links`]). [`input_schema`] is the
//! tool's JSON schema, kept in step with the input types.
//!
//! The model types themselves are plain data in `dovetail_api::map`, re-exported here, so the UI
//! can hold map content without depending on this crate (and the renderer it will carry).
//! [`MapRenderer`] turns a structure map into a PNG and the node rectangles in it (merman, S5).
//! [`swimlane`] lays a swimlane map out on a grid as plain geometry. No async here.

mod input;
mod mermaid;
mod render;
mod schema;
pub mod swimlane;
mod validate;

#[cfg(test)]
mod render_tests;

pub use dovetail_api::map::*;
pub use input::{
    ConnectionInput, DrawMapInput, FlowInput, GroupInput, LaneInput, MapHeaderInput, NodeInput,
    StepInput,
};
pub use render::{MapRenderer, NodePlacement, PixelRect, RenderError, RenderOptions, RenderedMap};
pub use schema::input_schema;
pub use validate::{MapError, ValidMap, check_lane_links, limits, report, validate};
