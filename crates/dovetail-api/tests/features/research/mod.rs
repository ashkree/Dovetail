//! Tests for the `research` domain.

use crate::common::Case;

mod library;
#[allow(clippy::module_inception)]
mod research;

pub fn cases() -> Vec<Case> {
    [research::cases(), library::cases()]
        .into_iter()
        .flatten()
        .collect()
}
