//! Dovetail's core: sessions, storage, worktrees and the MCP tool handlers.
//!
//! The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does.

pub mod config;
pub mod db;
mod inbox;
pub mod paths;
pub mod projects;
pub mod recipe;
pub mod service;
pub mod session;
mod slice;
pub mod socket;
mod tools;
pub mod worktree;

pub use service::{COORDINATOR, CoreError, CoreHandle, CoreOptions, spawn};
