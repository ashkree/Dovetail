//! Tests for the `code` domain.

use crate::common::Case;

mod worktree;

pub fn cases() -> Vec<Case> {
    [worktree::cases()].into_iter().flatten().collect()
}
