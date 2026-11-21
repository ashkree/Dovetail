//! The `effort` feature: efforts: goals with acceptance criteria.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::EffortCommand;
pub use event::EffortEvent;

use serde::{Deserialize, Serialize};

/// Identifies an effort. Matches the `efforts.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffortId(pub i64);

impl Default for EffortId {
    /// [`UNSORTED_EFFORT`]: where a task with no effort named goes.
    fn default() -> Self {
        UNSORTED_EFFORT
    }
}

/// Identifies an acceptance criterion. Matches the `criteria.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CriterionId(pub i64);

/// The effort every project has from the start: tasks nobody has sorted yet. It cannot be
/// renamed or closed.
pub const UNSORTED_EFFORT: EffortId = EffortId(1);

/// The name of [`UNSORTED_EFFORT`].
pub const UNSORTED_EFFORT_NAME: &str = "unsorted";

/// One line of work in a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effort {
    /// Effort id.
    pub id: EffortId,
    /// Its name, in lower kebab case ([`effort_name`]) and unique in the project.
    pub name: String,
    /// What sort of work it is.
    pub kind: EffortKind,
    /// The question it answers or the goal it reaches, in one line.
    pub question_or_goal: String,
    /// Open, or closed by the user.
    pub status: EffortStatus,
    /// The "where was I" card, kept up to date by the agent.
    #[serde(default)]
    pub where_was_i: WhereWasI,
    /// Its acceptance criteria, in the order they were added.
    #[serde(default)]
    pub criteria: Vec<Criterion>,
}

/// What sort of work an effort is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortKind {
    /// Something new for the product.
    Feature,
    /// A fault put right.
    Fix,
    /// Tidying with no change in behaviour.
    Cleanup,
    /// A question to answer.
    Research,
    /// A trial whose result may be thrown away.
    Experiment,
}

/// Whether an effort is still being worked on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortStatus {
    /// Being worked on, or waiting to be.
    #[default]
    Open,
    /// Closed by the user. Only the user closes an effort.
    Closed,
}

/// An effort's "where was I" card: what the user reads first on coming back to it. What the
/// effort is comes from [`Effort::question_or_goal`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhereWasI {
    /// The last thing that got done.
    #[serde(default)]
    pub last_done: String,
    /// What is waiting on the user.
    #[serde(default)]
    pub waits_on_you: String,
    /// What comes next.
    #[serde(default)]
    pub next_step: String,
}

/// One acceptance criterion of an effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    /// Criterion id.
    pub id: CriterionId,
    /// The effort it belongs to.
    pub effort: EffortId,
    /// What must be true.
    pub text: String,
    /// Where it stands.
    pub state: CriterionState,
    /// What the claim rests on, as the claimant wrote it (`PR #41`, `finding 41`); `None` until
    /// someone has claimed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// Where an acceptance criterion stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionState {
    /// Not met yet.
    Open,
    /// An agent says it is met and gave evidence; it waits for the user.
    Claimed,
    /// The user approved it. Nothing else sets this.
    Met,
}

/// Fields for a new effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewEffort {
    /// Its name as typed; the core converts it with [`effort_name`].
    pub name: String,
    /// What sort of work it is.
    pub kind: EffortKind,
    /// The question it answers or the goal it reaches.
    #[serde(default)]
    pub question_or_goal: String,
}

/// One field change in [`EffortCommand::Update`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortChange {
    /// New kind.
    Kind(EffortKind),
    /// New question or goal.
    QuestionOrGoal(String),
    /// Close it, or open it again.
    Status(EffortStatus),
}

/// An effort name as stored, from the name as typed: lower case, with each run of anything that
/// is not a letter or a digit turned into one hyphen, and none at either end. `"Guest Checkout!"`
/// becomes `guest-checkout`. Empty when the typed name holds no letter or digit.
pub fn effort_name(typed: &str) -> String {
    let mut name = String::new();
    for c in typed.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            name.push(c);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    if name.ends_with('-') {
        name.pop();
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_lower_kebab_case() {
        assert_eq!(effort_name("Guest Checkout!"), "guest-checkout");
        assert_eq!(effort_name("  which_provider?? "), "which-provider");
        assert_eq!(effort_name("already-kebab"), "already-kebab");
        assert_eq!(effort_name("API v2"), "api-v2");
        assert_eq!(effort_name("Écoute"), "écoute");
        assert_eq!(effort_name(" -- "), "");
    }
}
