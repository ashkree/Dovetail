//! The `research` feature's commands.

use crate::inbox::InboxId;
use crate::research::{ResearchKind, ResearchWhat};
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// Research: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchCommand {
    /// Open a research round on a task (`:round <task> <brief>`, `docs/research.md` §5.1):
    /// subagents the coordinator spawns while it is open are filed under it. While another round
    /// is open the new one is queued instead (its `brief.md` is written at once) and opens, in
    /// request order, when the open one closes. Refused for an unknown or archived task, an
    /// empty brief, or a folder that is not a project yet; the refusal comes back as
    /// [`crate::app::AppEvent::CommandFailed`].
    StartRound {
        /// The task (the question).
        task: TaskId,
        /// The user's criteria, written to the round's `brief.md`.
        brief: String,
    },
    /// Close the open research round of a task (`:round close`). The next queued round opens.
    CloseRound {
        /// The task.
        task: TaskId,
    },
    /// Drop a queued research round of a task (`:round drop <task>`): its row and `brief.md` go.
    /// Refused for a round that is open or closed.
    DropRound {
        /// The task.
        task: TaskId,
        /// Which of its queued rounds; `None` drops the one requested last.
        round: Option<u32>,
    },
    /// Set what a research task is for (`:research kind <decision|question>`).
    SetKind {
        /// The task.
        task: TaskId,
        /// The kind.
        kind: ResearchKind,
    },
    /// File an unfiled coordinator finding under a task (`y` on its Needs-you item, then a task
    /// from the picker): the finding and its source files move to the task's research folder.
    /// `d` on the item drops the finding with [`crate::inbox::InboxCommand::Dismiss`].
    FileUnfiled {
        /// The "Unfiled finding" item.
        item: InboxId,
        /// The task to file it under.
        task: TaskId,
    },
    /// Fetch research content. The core answers with [`super::ResearchEvent::Loaded`], or
    /// [`crate::app::AppEvent::CommandFailed`] when there is no such thing.
    Load {
        /// The task.
        task: TaskId,
        /// What to fetch.
        what: ResearchWhat,
    },
    /// Make the Ask session of a research task ready (`i` in reading mode, ADR 0018): a
    /// read-only session grounded in the task's report and the sources it cites, apart from
    /// the coordinator. The core answers with [`crate::agent::AgentEvent::Upserted`] for an
    /// [`crate::agent::AgentKind::Ask`] agent of the task (the one there is, or a new one); questions
    /// then go to it as [`crate::conversation::ConversationCommand::Prompt`]s. Refused for a task with no report.
    StartAsk {
        /// The research task.
        task: TaskId,
    },
}
