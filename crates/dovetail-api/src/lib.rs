#![doc = include_str!("../ARCHITECTURE.md")]
#![warn(missing_docs)]

pub mod activity;
pub mod agents;
pub mod app;
mod apply;
pub mod code;
pub mod research;
pub mod snapshot;
pub mod tracking;
pub mod visual;
mod wire;

// Short paths: every feature module is also reachable at the crate root, so callers write
// `dovetail_api::task::Task` as well as `dovetail_api::tracking::task::Task`. (The `research`
// and `activity` domains re-export their namesake feature's items instead; see `research/mod.rs`.)
pub use activity::{feed, inbox, status};
pub use agents::{agent, conversation, journal, mcp, model, tangent};
pub use code::worktree;
pub use research::library;
pub use tracking::{decision, effort, project, task};
pub use visual::{canvas, design, map};

pub use apply::{ApplyResult, apply};
pub use snapshot::Snapshot;
pub use wire::{Command, CoreLink, Event, PROTOCOL_VERSION, ServerMsg};
