//! The `effort` feature's commands.

use serde::{Deserialize, Serialize};
use crate::effort::{CriterionId, EffortChange, EffortId, NewEffort, WhereWasI};

/// Efforts: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortCommand {
    /// Create an effort. The core converts the name to lower kebab case
    /// ([`effort_name`]) and refuses an empty one or one already taken. It answers with
    /// [`EffortEvent::Upserted`].
    Create(NewEffort),
    /// Rename an effort; the name is converted as for [`EffortCommand::Create`]. Refused for
    /// Unsorted.
    Rename {
        /// The effort.
        id: EffortId,
        /// The new name, as typed.
        name: String,
    },
    /// Change fields of an effort. Closing Unsorted is refused.
    Update {
        /// The effort.
        id: EffortId,
        /// Changes, applied in order.
        patch: Vec<EffortChange>,
    },
    /// Replace an effort's "where was I" card.
    SetWhereWasI {
        /// The effort.
        id: EffortId,
        /// The card.
        where_was_i: WhereWasI,
    },
    /// Add an acceptance criterion to an effort; it starts open.
    AddCriterion {
        /// The effort.
        effort: EffortId,
        /// What must be true.
        text: String,
    },
    /// Say a criterion is met, with evidence: it becomes claimed and waits for the user.
    /// Refused for one already met.
    ClaimCriterion {
        /// The criterion.
        id: CriterionId,
        /// What the claim rests on.
        evidence: String,
    },
    /// Approve a criterion: it becomes met. Only the user can do this; there is no agent tool
    /// for it.
    ApproveCriterion {
        /// The criterion.
        id: CriterionId,
    },
    /// Send a claimed (or met) criterion back: it becomes open again and keeps its evidence. The
    /// coordinator is told, with `reason`.
    ReturnCriterion {
        /// The criterion.
        id: CriterionId,
        /// Why, for the coordinator.
        reason: Option<String>,
    },
}
