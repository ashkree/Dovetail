//! The `model` feature's commands.

use crate::agent::AgentId;
use crate::model::ReasoningEffort;
use serde::{Deserialize, Serialize};

/// Models: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCommand {
    /// Switch a session's model and effort (task #39). A running session switches live and the
    /// core answers with [`crate::agent::AgentEvent::Upserted`] once Claude Code confirms, or
    /// [`crate::app::AppEvent::CommandFailed`] with the old model kept. Either way the choice is used from the
    /// session's next start.
    Set {
        /// The session.
        agent: AgentId,
        /// The model, as [`super::ModelOption::name`].
        model: String,
        /// The effort; `None` leaves it to the model's default (a model with no levels).
        effort: Option<ReasoningEffort>,
    },
    /// Move the coordinator to another driver (task #46): a new session on `driver`, started on
    /// `model` at `effort` through the `:handoff` flow (`handoff`) or the `:fresh` one. The old
    /// conversation is kept, not continued. Refused for any other agent, and while a turn runs.
    SwitchDriver {
        /// The session; only the coordinator switches.
        agent: AgentId,
        /// The driver to run on.
        driver: super::DriverKind,
        /// The model, as that driver's [`super::ModelOption::name`].
        model: String,
        /// The effort; `None` for the model's default.
        effort: Option<ReasoningEffort>,
        /// Seed the new session with a handoff (`:handoff`), else start clean (`:fresh`).
        handoff: bool,
    },
}
