//! Turning conversation text into styled lines: wrapping, markdown, the transcript and the
//! reading column. Nothing here touches the terminal, so each part is tested on its own.

pub mod citations;
pub mod html;
pub mod markdown;
pub mod reading_column;
pub mod transcript;
pub mod working;
pub mod wrap;
