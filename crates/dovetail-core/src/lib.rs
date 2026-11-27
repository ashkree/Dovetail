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
mod design;
mod docs;
mod efforts;
pub mod error;
pub mod home;
mod inbox;
mod journal;
pub mod mcp;
mod preview;
mod project_admin;
pub mod projects;
mod registration;
mod report;
mod research;
pub mod service;
pub mod sessions;
pub mod settings;
mod shutdown;
mod side_chat;
mod slice;
mod startup;
pub mod storage;
mod tangents;
mod todos;
mod usage;

// Temporary root re-exports for moved domains/infrastructure while the rest of the crate migrates
// (docs/plan/core-by-domain.md, task #59).
pub(crate) use app::clipboard;
pub use app::{editor, external, upgrade};
pub use code::worktree;
pub(crate) use code::worktree_info;
pub use mcp::socket;
pub(crate) use mcp::tools;
pub use sessions as session;
pub use sessions::{drivers, memory, takeover};
pub(crate) use sessions::{foreground, oom, restore};
pub use settings::{config, paths, profile, recipe};
pub use storage as db;

pub use service::{COORDINATOR, CoreError, CoreOptions, spawn};
pub use sessions::drivers::Drivers;
pub use shutdown::CoreHandle;
