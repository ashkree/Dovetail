//! The `app` feature: the app itself (resync, quit, open a URL, command failures) and what it
//! hands to the outside: the terminal (editor, native `claude`), the system clipboard and
//! `:upgrade`.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::AppCommand;
pub use event::AppEvent;

use crate::agents::conversation::Attachment;
use serde::{Deserialize, Serialize};

/// What the core found on the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paste {
    /// An accepted image (PNG, JPEG, GIF or WebP, at most
    /// [`crate::agents::conversation::MAX_ATTACHMENT_BYTES`]), already saved under Dovetail's
    /// `attachments/` folder: the UI adds it as a chip.
    Image(Attachment),
    /// Text, to insert at the cursor.
    Text(String),
    /// Nothing could be pasted; a short notice for the status bar saying why.
    Refused(String),
}
