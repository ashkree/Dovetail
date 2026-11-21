#![doc = include_str!("../ARCHITECTURE.md")]
#![warn(missing_docs)]

pub mod activity;
pub mod agent;
pub mod app;
mod apply;
pub mod canvas;
pub mod clipboard;
pub mod conversation;
pub mod decision;
pub mod design;
pub mod effort;
pub mod inbox;
pub mod journal;
pub mod library;
pub mod map;
pub mod mcp;
pub mod model;
pub mod project;
pub mod research;
mod snapshot;
pub mod socket;
pub mod status;
pub mod tangent;
pub mod task;
pub mod terminal;
pub mod tools;
pub mod upgrade;
pub mod usage;
mod wire;
pub mod worktree;

pub use apply::{ApplyResult, apply};
pub use snapshot::Snapshot;
pub use wire::{Command, CoreLink, Event, PROTOCOL_VERSION, ServerMsg};
