#![doc = include_str!("../ARCHITECTURE.md")]

pub(crate) mod activity;
pub mod agents;
pub mod app;
pub mod code;
pub mod engine;
pub mod mcp;
pub(crate) mod research;
pub mod sessions;
pub mod settings;
pub mod storage;
pub mod tracking;
pub(crate) mod visual;

// Root re-exports for moved domains/infrastructure kept for `crates/dovetail` and integration
// tests (docs/plan/core-by-domain.md, task #59).
pub(crate) use activity::inbox;
pub(crate) use agents::{attachments, journal, slice, tangents};
pub(crate) use app::clipboard;
pub use app::{editor, external, upgrade};
pub use code::worktree;
pub(crate) use code::worktree_info;
pub use engine as service;
pub use mcp::socket;
pub(crate) use mcp::tools;
pub(crate) use research::docs;
pub use sessions as session;
pub use sessions::{drivers, memory, takeover};
pub(crate) use sessions::{foreground, oom};
pub use settings::{config, paths, profile, recipe};
pub use storage as db;
pub(crate) use tracking::project_admin;
pub use tracking::{home, projects};
pub(crate) use visual::preview;

pub use engine::{COORDINATOR, CoreError, CoreHandle, CoreOptions, UpgradeOptions, spawn};
pub use sessions::drivers::Drivers;
