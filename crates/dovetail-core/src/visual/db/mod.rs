//! SQLite queries for the visual domain: canvas, design variants, and maps.

pub(crate) mod canvas;
pub(crate) mod design;
pub(crate) mod maps;

pub use canvas::{CanvasState, USER};
pub use maps::{Drawn, NewMapVersion};
