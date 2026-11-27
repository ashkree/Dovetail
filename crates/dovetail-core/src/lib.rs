//! Dovetail's core: sessions, storage, worktrees and the MCP tool handlers.
//!
//! The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does, and launches
//! sessions through the registered [`Drivers`] (`ClaudeDriver`, `AgyDriver`, or fakes in tests).

pub(crate) mod activity;
pub mod agents;
pub mod app;
pub mod code;
pub mod mcp;
pub(crate) mod research;
pub mod service;
pub mod sessions;
pub mod settings;
mod shutdown;
mod startup;
pub mod storage;
pub mod tracking;
pub(crate) mod visual;

// Temporary root re-exports for moved domains/infrastructure while the rest of the crate migrates
// (docs/plan/core-by-domain.md, task #59).
pub(crate) use activity::inbox;
pub(crate) use agents::{attachments, completion, journal, slice, tangents, todos};
pub(crate) use app::clipboard;
pub use app::{editor, external, upgrade};
pub use code::worktree;
pub(crate) use code::worktree_info;
pub use mcp::socket;
pub(crate) use mcp::tools;
pub(crate) use research::docs;
pub use sessions as session;
pub use sessions::{drivers, memory, takeover};
pub(crate) use sessions::{foreground, oom, restore};
pub use settings::{config, paths, profile, recipe};
pub use storage as db;
pub use tracking::{home, projects};
pub(crate) use tracking::{project_admin, registration};
pub(crate) use visual::preview;

pub use service::{COORDINATOR, CoreError, CoreOptions, spawn};
pub use sessions::drivers::Drivers;
pub use shutdown::CoreHandle;
