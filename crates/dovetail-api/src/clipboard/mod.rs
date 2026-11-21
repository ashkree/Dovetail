//! The `clipboard` feature: the system clipboard: reading it for a chat bar, copying to it.

mod command;
mod event;
pub use command::ClipboardCommand;
pub use event::ClipboardEvent;

use serde::{Deserialize, Serialize};
use crate::conversation::Attachment;

/// What the core found on the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paste {
    /// An accepted image (PNG, JPEG, GIF or WebP, at most [`MAX_ATTACHMENT_BYTES`]),
    /// already saved under Dovetail's `attachments/` folder: the UI adds it as a chip.
    Image(Attachment),
    /// Text, to insert at the cursor.
    Text(String),
    /// Nothing could be pasted; a short notice for the status bar saying why.
    Refused(String),
}
