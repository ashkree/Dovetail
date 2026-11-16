//! Plain-data protocol between the Dovetail core and its UI.
//!
//! The core sends a [`ServerMsg::Snapshot`], then numbered [`Event`]s. The UI keeps its own copy
//! of the [`Snapshot`] and changes it only through [`apply`]. The UI asks for changes with
//! [`Command`]s. Every type is plain `serde` data, so the core can later run as a daemon behind a
//! unix socket without the UI changing.

#![warn(missing_docs)]

mod apply;
mod canvas;
mod design;
mod effort;
pub mod map;
mod model;
mod protocol;
pub mod research;
pub mod socket;
pub mod tools;

pub use apply::{ApplyResult, apply};
pub use canvas::*;
pub use design::*;
pub use effort::*;
pub use model::*;
pub use protocol::*;
