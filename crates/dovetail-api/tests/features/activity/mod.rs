//! Tests for the `activity` domain.

use crate::common::Case;

mod feed;
mod inbox;
mod status;
mod usage;

pub fn cases() -> Vec<Case> {
    [
        inbox::cases(),
        feed::cases(),
        status::cases(),
        usage::cases(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
