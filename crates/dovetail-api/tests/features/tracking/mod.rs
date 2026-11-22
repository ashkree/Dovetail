//! Tests for the `tracking` domain.

use crate::common::Case;

mod decision;
mod effort;
mod project;
mod task;

pub fn cases() -> Vec<Case> {
    [
        project::cases(),
        effort::cases(),
        task::cases(),
        decision::cases(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
