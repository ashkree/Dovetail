//! Dovetail's core: sessions, storage, worktrees and the MCP tool handlers.
//!
//! The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does.

mod ask;
mod attachments;
mod clipboard;
mod completion;
pub mod config;
mod context;
pub mod db;
mod docs;
pub mod editor;
pub mod error;
pub mod external;
mod foreground;
mod inbox;
mod journal;
pub mod memory;
mod oom;
pub mod paths;
pub mod profile;
pub mod projects;
pub mod recipe;
mod registration;
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
mod todos;
mod tools;
pub mod upgrade;
mod usage;
pub mod worktree;
mod worktree_info;

pub use service::{COORDINATOR, CoreError, CoreOptions, spawn};
pub use shutdown::CoreHandle;
