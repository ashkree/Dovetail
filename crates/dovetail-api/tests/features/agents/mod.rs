//! Tests for the `agents` domain.

use crate::common::Case;

mod agent;
mod conversation;
mod journal;
mod mcp;
mod model;
mod tangent;

pub fn cases() -> Vec<Case> {
    [
        agent::cases(),
        conversation::cases(),
        model::cases(),
        mcp::cases(),
        tangent::cases(),
        journal::cases(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
