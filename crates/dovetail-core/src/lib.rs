//! Dovetail's core: sessions, storage, worktrees and the MCP tool handlers.
//!
//! The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does.

pub mod config;
pub mod db;
pub mod error;
mod inbox;
pub mod paths;
pub mod projects;
pub mod recipe;
mod registration;
mod restore;
pub mod service;
pub mod session;
mod shutdown;
mod slice;
pub mod socket;
mod todos;
mod tools;
pub mod worktree;

pub use service::{COORDINATOR, CoreError, CoreOptions, spawn};
pub use shutdown::CoreHandle;
