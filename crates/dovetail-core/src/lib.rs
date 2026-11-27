//! Dovetail's core: sessions, storage, worktrees and the MCP tool handlers.
//!
//! The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does, and launches
//! sessions through the registered [`Drivers`] (`ClaudeDriver`, `AgyDriver`, or fakes in tests).

mod agy_models;
pub mod app;
mod ask;
mod attachments;
mod browse;
mod canvas;
pub mod code;
mod completion;
mod context;
mod design;
mod docs;
pub mod drivers;
mod efforts;
pub mod error;
mod foreground;
pub mod home;
mod inbox;
mod journal;
pub mod memory;
mod oom;
mod preview;
mod project_admin;
pub mod projects;
mod registration;
mod report;
mod research;
mod restore;
pub mod service;
pub mod session;
pub mod settings;
mod shutdown;
mod side_chat;
mod slice;
pub mod socket;
mod startup;
pub mod storage;
pub mod takeover;
mod tangents;
mod todos;
mod tools;
mod usage;

// Temporary root re-exports for moved leaf domains while the rest of the crate migrates
// (docs/plan/core-by-domain.md, task #59).
pub(crate) use app::clipboard;
pub use app::{editor, external, upgrade};
pub use code::worktree;
pub(crate) use code::worktree_info;
pub use settings::{config, paths, profile, recipe};
pub use storage as db;

pub use drivers::Drivers;
pub use service::{COORDINATOR, CoreError, CoreOptions, spawn};
pub use shutdown::CoreHandle;
