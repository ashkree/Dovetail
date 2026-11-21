//! Dovetail's core: sessions, storage, worktrees and the MCP tool handlers.
//!
//! The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does, and launches
//! sessions through the registered [`Drivers`] (`ClaudeDriver`, `AgyDriver`, or fakes in tests).

mod ask;
mod attachments;
mod browse;
mod canvas;
mod clipboard;
mod completion;
pub mod config;
mod context;
pub mod db;
mod design;
mod docs;
pub mod drivers;
pub mod editor;
mod efforts;
pub mod error;
pub mod external;
mod foreground;
pub mod home;
mod inbox;
mod journal;
pub mod memory;
mod oom;
pub mod paths;
mod preview;
pub mod profile;
mod project_admin;
pub mod projects;
pub mod recipe;
mod registration;
mod report;
mod research;
mod restore;
pub mod service;
pub mod session;
mod shutdown;
mod side_chat;
mod slice;
pub mod socket;
mod startup;
pub mod takeover;
mod tangents;
mod todos;
mod tools;
pub mod upgrade;
mod usage;
pub mod worktree;
mod worktree_info;

pub use drivers::Drivers;
pub use service::{COORDINATOR, CoreError, CoreOptions, spawn};
pub use shutdown::CoreHandle;
