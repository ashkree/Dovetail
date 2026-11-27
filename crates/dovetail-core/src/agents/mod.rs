//! Agents domain: slices, side chats, tangents, reports, journals, todos, attachments, completion,
//! and agy model discovery.

pub(crate) mod agy_models;
pub(crate) mod attachments;
pub(crate) mod completion;
pub(crate) mod journal;
pub(crate) mod report;
pub(crate) mod side_chat;
pub(crate) mod slice;
pub(crate) mod tangents;
pub(crate) mod todos;

pub use slice::{SetupFailure, SliceError};
