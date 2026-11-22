//! Tests for the `visual` domain.

use crate::common::Case;

mod canvas;
mod design;
mod map;

pub fn cases() -> Vec<Case> {
    [canvas::cases(), map::cases(), design::cases()]
        .into_iter()
        .flatten()
        .collect()
}
