//! Images attached to a message, and the limits on them.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Most images one message can carry.
pub const MAX_ATTACHMENTS: usize = 10;

/// Bytes in a megabyte, as Dovetail shows sizes (binary units).
const MB: u64 = 1024 * 1024;

/// Largest image file accepted, in bytes (5 MB).
pub const MAX_ATTACHMENT_BYTES: u64 = 5 * MB;

/// A file size as the UI and its notices show it: `412 KB` or `1.2 MB` (binary units, at least
/// `1 KB`). Shared so the core's messages and the TUI's chips agree.
pub fn size_text(size: u64) -> String {
    if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else {
        format!("{} KB", size.div_ceil(1024).max(1))
    }
}

/// [`MAX_ATTACHMENT_BYTES`] for messages: `5 MB`.
pub fn attachment_limit_text() -> String {
    format!("{} MB", MAX_ATTACHMENT_BYTES / MB)
}

/// An image attached to a user message. The UI saves the file under Dovetail's data dir and
/// sends its path; the core reads it when sending, so no raw bytes cross the protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// The saved image file.
    pub path: PathBuf,
    /// Its format.
    pub media_type: ImageType,
    /// File size in bytes.
    pub size: u64,
}

/// An image format Claude accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageType {
    /// `image/png`.
    #[serde(rename = "image/png")]
    Png,
    /// `image/jpeg`.
    #[serde(rename = "image/jpeg")]
    Jpeg,
    /// `image/gif`.
    #[serde(rename = "image/gif")]
    Gif,
    /// `image/webp`.
    #[serde(rename = "image/webp")]
    Webp,
}

impl ImageType {
    /// Every accepted format, in order of preference when a source offers several.
    pub const ALL: [ImageType; 4] = [Self::Png, Self::Jpeg, Self::Webp, Self::Gif];

    /// The MIME type, as Claude's image blocks name it.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// The format a MIME type names (`image/jpg` counts as JPEG).
    pub fn from_mime(mime: &str) -> Option<Self> {
        match mime.trim().to_ascii_lowercase().as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }

    /// File extension for a saved image.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }
}
