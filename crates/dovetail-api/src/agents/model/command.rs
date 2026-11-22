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
}
