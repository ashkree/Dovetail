//! The `activity` domain: what needs the user and what happened: the inbox, the activity feed and the status bar.
//!
//! A folder only: each feature below keeps its own `Command` and `Event`.

pub mod feed;
pub mod inbox;
pub mod status;

// The `feed` feature is the domain's namesake, so its items sit at the domain level.
pub use feed::*;
