//! The `research` domain: research runs and the library they fill.
//!
//! A folder only: each feature below keeps its own `Command` and `Event`.

pub mod library;
// The domain and its namesake feature share a name on purpose (see ARCHITECTURE.md).
#[allow(clippy::module_inception)]
pub mod research;

// The `research` feature is the domain's namesake, so its items sit at the domain level.
pub use research::*;
