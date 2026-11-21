//! Inputs of the tools Dovetail serves to sessions over MCP, and the environment the bridge reads.
//!
//! These types are the single definition shared by the bridge (`dovetail mcp`), which parses
//! Claude Code's arguments into them, and the core, which handles them. A schema change is a
//! change here, so both sides fail to compile instead of disagreeing at run time.

use serde::{Deserialize, Serialize};

use crate::research::{Evidence, FindingId, ProposalKind, ResearchWhat, SourceId};
use crate::{Alternative, Bucket, DecisionId, PLevel, ReasoningEffort, TaskId, TaskStatus};

/// Names of the environment variables Dovetail passes to the processes it launches.
pub mod env {
    /// The session's agent id; set for the MCP bridge and the worktree hook.
    pub const AGENT_ID: &str = "DOVETAIL_AGENT_ID";
    /// Path of the app's unix socket; set for the MCP bridge.
    pub const SOCKET: &str = "DOVETAIL_SOCKET";
    /// `coordinator`, `slice` or `side-chat` (see [`super::Role`]); set for the MCP bridge.
    pub const ROLE: &str = "DOVETAIL_ROLE";
    /// The session's suites, comma-separated (see [`super::Suite::join`]); set for the MCP
    /// bridge. Without it the bridge uses the role's defaults.
    pub const SUITES: &str = "DOVETAIL_SUITES";
    /// The project id; set for the worktree hook.
    pub const PROJECT: &str = "DOVETAIL_PROJECT";
    /// A slice's repo name (ADR 0021); set for the worktree hook, which applies the recipe's
    /// `[repos.<name>]` overrides.
    pub const REPO: &str = "DOVETAIL_REPO";
    /// Replaces `$XDG_DATA_HOME/dovetail` (databases, worktrees). With the three below, lets a
    /// dev build in a slice run on its own state without touching Claude Code's environment.
    pub const DATA_DIR: &str = "DOVETAIL_DATA_DIR";
    /// Replaces `$XDG_CONFIG_HOME/dovetail` (config, role prompts, recipes).
    pub const CONFIG_DIR: &str = "DOVETAIL_CONFIG_DIR";
    /// Replaces `$XDG_STATE_HOME/dovetail` (logs, launch files, setup state).
    pub const STATE_DIR: &str = "DOVETAIL_STATE_DIR";
    /// Replaces `$XDG_RUNTIME_DIR/dovetail` (sockets; keep it short, ~108-byte limit).
    pub const RUNTIME_DIR: &str = "DOVETAIL_RUNTIME_DIR";
}

/// Which suites a session gets by default, and which role prompt it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The coordinator: every suite.
    Coordinator,
    /// A slice: Core, Coordination, Maps and Design.
    Slice,
    /// A side chat: Core only (ADR 0011, ADR 0016).
    SideChat,
    /// Reading mode's Ask (ADR 0018): no suites; it reads the research files.
    Ask,
}

impl Role {
    /// The value of [`env::ROLE`].
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Coordinator => "coordinator",
            Role::Slice => "slice",
            Role::SideChat => "side-chat",
            Role::Ask => "ask",
        }
    }

    /// Parses [`env::ROLE`]; anything unknown is a slice, the safer default.
    pub fn from_env_value(value: &str) -> Role {
        match value {
            "coordinator" => Role::Coordinator,
            "side-chat" => Role::SideChat,
            "ask" => Role::Ask,
            _ => Role::Slice,
        }
    }

    /// The suites the role gets unless `config.toml` says otherwise (ADR 0016).
    pub fn default_suites(self) -> &'static [Suite] {
        match self {
            Role::Coordinator => Suite::ALL,
            Role::Slice => &[Suite::Core, Suite::Coordination, Suite::Maps, Suite::Design],
            Role::SideChat => &[Suite::Core],
            Role::Ask => &[],
        }
    }
}

/// A named group of tools; a role's config lists the suites it gets (ADR 0016). Each tool's
/// name starts with its suite's [`Suite::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Suite {
    /// Questions, status and the decision log: every role.
    Core,
    /// The user's task list.
    Tasks,
    /// Slice lifecycle and project registration.
    Project,
    /// What sessions tell each other through Dovetail: a slice's reports (ADR 0022). Claims
    /// and ordering come in v1.1.
    Coordination,
    /// Research rounds, reports and proposals (ADR 0015).
    Research,
    /// Maps (decision 25).
    Maps,
    /// The effort canvas: add a block, add a card (into a block or incoming), link two cards
    /// (ADR 0025, ADR 0027). Coordinator only for now.
    Canvas,
    /// What a slice's previewed page can switch: `design_declare_variants` (ADR 0026).
    Design,
}

impl Suite {
    /// Every suite, in the order the ADR lists them.
    pub const ALL: &'static [Suite] = &[
        Suite::Core,
        Suite::Tasks,
        Suite::Project,
        Suite::Coordination,
        Suite::Research,
        Suite::Maps,
        Suite::Canvas,
        Suite::Design,
    ];

    /// The name in `config.toml` and [`env::SUITES`], also the tools' prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            Suite::Core => "core",
            Suite::Tasks => "tasks",
            Suite::Project => "project",
            Suite::Coordination => "coordination",
            Suite::Research => "research",
            Suite::Maps => "maps",
            Suite::Canvas => "canvas",
            Suite::Design => "design",
        }
    }

    /// Parses one suite name.
    pub fn from_name(name: &str) -> Option<Suite> {
        Suite::ALL.iter().copied().find(|s| s.as_str() == name)
    }

    /// The value of [`env::SUITES`]: names joined by commas.
    pub fn join(suites: &[Suite]) -> String {
        suites
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Parses [`env::SUITES`]; unknown names are skipped, so a typo grants nothing.
    pub fn parse_list(value: &str) -> Vec<Suite> {
        value
            .split(',')
            .filter_map(|n| Suite::from_name(n.trim()))
            .collect()
    }
}

/// A row of [`TOOLS`]: a tool's name, its suite, and the name it had before ADR 0016.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    /// The name, without the server prefix.
    pub name: &'static str,
    /// The suite that grants it.
    pub suite: Suite,
    /// The unprefixed v1 name, still answered for one release as a deprecated alias.
    pub old_name: Option<&'static str>,
}

/// `Some(old)` or `None`, for the optional `was` of a `tools!` row.
macro_rules! old_name {
    () => {
        None
    };
    ($old:literal) => {
        Some($old)
    };
}

/// Defines [`Tool`], [`TOOLS`] and [`Tool::name`] from one list, so a tool's variant, wire name,
/// suite and old name are written once.
macro_rules! tools {
    ($( $(#[doc = $doc:literal])* $variant:ident($input:ident) = $name:literal, $suite:ident
        $(, was $old:literal)?; )*) => {
        /// One tool call: the tool and its arguments. On the wire,
        /// `{"tool": "<name>", "input": {..}}`; an old name still parses (ADR 0016).
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[serde(tag = "tool", content = "input")]
        pub enum Tool {
            $( $(#[doc = $doc])* #[serde(rename = $name $(, alias = $old)?)] $variant($input), )*
        }

        /// Every tool, in the order `tools/list` shows them.
        pub const TOOLS: &[ToolSpec] = &[
            $( ToolSpec { name: $name, suite: Suite::$suite, old_name: old_name!($($old)?) }, )*
        ];

        impl Tool {
            /// The tool's name, without the server prefix.
            pub fn name(&self) -> &'static str {
                match self {
                    $( Tool::$variant(_) => $name, )*
                }
            }
        }
    };
}

tools! {
    /// `core_ask_user`.
    AskUser(AskUser) = "core_ask_user", Core, was "ask_user";
    /// `core_report_status`.
    ReportStatus(ReportStatus) = "core_report_status", Core, was "report_status";
    /// `core_log_decision`.
    LogDecision(LogDecision) = "core_log_decision", Core, was "log_decision";
    /// `tasks_create` (ADR 0002).
    CreateTask(CreateTask) = "tasks_create", Tasks, was "create_task";
    /// `tasks_list` (task #20).
    ListTasks(ListTasks) = "tasks_list", Tasks, was "list_tasks";
    /// `tasks_get` (task #20).
    GetTask(GetTask) = "tasks_get", Tasks, was "get_task";
    /// `tasks_update` (task #25).
    UpdateTask(UpdateTask) = "tasks_update", Tasks, was "update_task";
    /// `tasks_archive` (task #9).
    ArchiveTask(ArchiveTask) = "tasks_archive", Tasks, was "archive_task";
    /// `project_start_slice` (ADR 0004).
    StartSlice(StartSlice) = "project_start_slice", Project, was "start_slice";
    /// `project_archive_slice` (task #9).
    ArchiveSlice(ArchiveSlice) = "project_archive_slice", Project, was "archive_slice";
    /// `project_add` (ADR 0005).
    AddProject(AddProject) = "project_add", Project, was "add_project";
    /// `research_start_round` (ADR 0015).
    ResearchStartRound(ResearchStartRound) = "research_start_round", Research;
    /// `research_write_report` (ADR 0015).
    ResearchWriteReport(ResearchWriteReport) = "research_write_report", Research;
    /// `research_propose` (ADR 0015).
    ResearchPropose(ResearchPropose) = "research_propose", Research;
    /// `research_get`: read a stored research document.
    ResearchGet(ResearchGet) = "research_get", Research;
    /// `coordination_report` (ADR 0022): a slice's report to the coordinator.
    Report(Report) = "coordination_report", Coordination;
    /// `coordination_claim_criterion` (decided 2026-11-16): say an acceptance criterion is met,
    /// with evidence; only the user approves it.
    ClaimCriterion(ClaimCriterion) = "coordination_claim_criterion", Coordination;
    /// `maps_draw` (decision 25, M15).
    DrawMap(DrawMap) = "maps_draw", Maps, was "draw_map";
    /// `canvas_add_block` (ADR 0027): a block where the effort's canvas has room.
    CanvasAddBlock(CanvasAddBlock) = "canvas_add_block", Canvas;
    /// `canvas_add_card` (ADR 0025, ADR 0027): a card into a block, or the incoming strip.
    CanvasAddCard(CanvasAddCard) = "canvas_add_card", Canvas;
    /// `canvas_link_cards` (ADR 0025): link two cards.
    CanvasLinkCards(CanvasLinkCards) = "canvas_link_cards", Canvas;
    /// `design_declare_variants` (ADR 0026): the slots the viewer's variant panel shows.
    DeclareVariants(DeclareVariants) = "design_declare_variants", Design;
}

/// The row of [`TOOLS`] named `name`, by its current or its old name.
pub fn tool_spec(name: &str) -> Option<&'static ToolSpec> {
    TOOLS
        .iter()
        .find(|t| t.name == name || t.old_name == Some(name))
}

/// The row whose old name is `name`: a call by a deprecated alias (ADR 0016).
pub fn deprecated_alias(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.old_name == Some(name))
}

/// The line added to the result of a call by an old name, so the model learns the new one.
pub fn deprecation_note(spec: &ToolSpec) -> String {
    format!(
        "Note: `{}` is deprecated and will be removed; call `{}` instead.",
        spec.old_name.unwrap_or(spec.name),
        spec.name
    )
}

/// The rows of [`TOOLS`] the suites grant, in table order.
pub fn tools_in(suites: &[Suite]) -> impl Iterator<Item = &'static ToolSpec> + '_ {
    TOOLS.iter().filter(|t| suites.contains(&t.suite))
}

/// The first word of an `project_archive_slice` / `tasks_archive` answer that did not fail.
pub mod archive_result {
    /// The slice is gone: session stopped, worktree and branch removed, its place freed.
    pub const ARCHIVED: &str = "archived";
    /// The worktree has uncommitted or unpushed work: the user is asked before it goes.
    pub const WAITING_FOR_USER: &str = "waiting_for_user";
}

/// Why a tool call is refused before it reaches a handler. `Display` is the message for the
/// agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// No tool has this name.
    Unknown(String),
    /// The arguments do not fit the tool's schema.
    BadArguments {
        /// The tool.
        tool: String,
        /// What serde said.
        message: String,
    },
    /// A text field is empty.
    Empty(&'static str),
    /// A task id is not positive.
    NotATaskId(&'static str),
    /// `tasks_create` asked for Now (ADR 0002).
    BucketNow,
    /// The coordinator logged a decision without naming its task.
    DecisionNeedsTask,
    /// `tasks_update` asked for `archived`, which is `tasks_archive`'s job.
    UpdateToArchived,
    /// `tasks_update` asked for a status: Dovetail sets task status itself (ADR 0024).
    StatusIsSetByDovetail,
    /// `coordination_report` from a session that is not a slice.
    ReportFromNonSlice,
    /// `coordination_claim_criterion` from a session that is neither a slice nor the
    /// coordinator.
    ClaimFromOtherRole,
    /// A criterion id is not positive.
    NotACriterionId,
    /// `tasks_update` named no field to change.
    NothingToUpdate,
    /// `research_propose` of a decision with no evidence (ADR 0015).
    DecisionNeedsEvidence,
    /// An `evidence` entry is not `F<id>` or `S<id>`.
    BadEvidence(String),
    /// A card id is not positive.
    NotACardId(&'static str),
    /// `canvas_add_card` of an idea or question with no text, or of a reference with no id, or
    /// of an idea or question that also names a reference.
    BadCard(&'static str),
    /// `design_declare_variants` from a session that is not a slice.
    DeclareFromNonSlice,
    /// `design_declare_variants` with a list that cannot be used; the text says what is wrong.
    BadVariants(String),
    /// `project_start_slice` gave an `effort` but no `model`.
    EffortNeedsModel,
    /// `research_get`'s `what` is none of `report`, `round:N`, `finding:N`, `source:N`.
    BadWhat(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Unknown(name) => write!(f, "unknown tool `{name}`"),
            ToolError::BadArguments { tool, message } => {
                write!(f, "bad arguments for `{tool}`: {message}")
            }
            ToolError::Empty(field) => write!(f, "`{field}` must be a non-empty string"),
            ToolError::NotATaskId(field) => {
                write!(f, "`{field}` must be a task id (a positive integer)")
            }
            ToolError::BucketNow => f.write_str(
                "`bucket` must be `next` or `later`: the user decides what is current (ADR 0002)",
            ),
            ToolError::DecisionNeedsTask => {
                f.write_str("the coordinator must name the `task` this decision belongs to")
            }
            ToolError::UpdateToArchived => {
                f.write_str("`status` cannot be set here: archive a task with `tasks_archive`")
            }
            ToolError::StatusIsSetByDovetail => f.write_str(
                "`status` cannot be set: Dovetail moves a task to in progress when its slice \
                 starts, to review when its pull request opens or its slice reports done, and \
                 to done when the pull request is merged. For work with no pull request, ask \
                 the user to mark it done",
            ),
            ToolError::ReportFromNonSlice => {
                f.write_str("`coordination_report` is for slices: it reports to the coordinator")
            }
            ToolError::ClaimFromOtherRole => {
                f.write_str("`coordination_claim_criterion` is for slices and the coordinator")
            }
            ToolError::NotACriterionId => {
                f.write_str("`criterion` must be a criterion id (a positive integer)")
            }
            ToolError::NothingToUpdate => f.write_str(
                "give at least one field to change: `bucket`, `p_level`, `title` or `notes`",
            ),
            ToolError::DecisionNeedsEvidence => f.write_str(
                "a decision needs `evidence`: the finding and source ids it rests on, like \
                 [\"F3\", \"S12\"]. Propose it as an `idea` if nothing backs it yet",
            ),
            ToolError::BadEvidence(text) => write!(
                f,
                "evidence `{text}` is not a finding or source id: write F<id> or S<id>, like                  F3 or S12"
            ),
            ToolError::NotACardId(field) => {
                write!(f, "`{field}` must be a card id (a positive integer)")
            }
            ToolError::BadCard(why) => f.write_str(why),
            ToolError::DeclareFromNonSlice => f.write_str(
                "`design_declare_variants` is for slices: they declare the variants of the page they build",
            ),
            ToolError::BadVariants(why) => write!(f, "bad variants: {why}"),
            ToolError::EffortNeedsModel => {
                f.write_str("`effort` needs a `model` too: name both, like `sonnet` and `medium`")
            }
            ToolError::BadWhat(text) => write!(
                f,
                "`what` must be `report`, `round:N`, `finding:N` or `source:N`, not `{text}`"
            ),
        }
    }
}

impl std::error::Error for ToolError {}

impl Tool {
    /// Parses MCP `tools/call` arguments for the tool `name` (without the server prefix).
    /// An old name (ADR 0016) parses as its new one.
    pub fn parse(name: &str, arguments: serde_json::Value) -> Result<Tool, ToolError> {
        let Some(spec) = tool_spec(name) else {
            return Err(ToolError::Unknown(name.to_owned()));
        };
        let wire = serde_json::json!({ "tool": spec.name, "input": arguments });
        serde_json::from_value(wire).map_err(|e| ToolError::BadArguments {
            tool: name.to_owned(),
            message: e.to_string(),
        })
    }

    /// Whether a session with `suites` may call the tool named `name` (current or old name).
    pub fn allowed(name: &str, suites: &[Suite]) -> bool {
        tool_spec(name).is_some_and(|t| suites.contains(&t.suite))
    }

    /// Checks what serde cannot: non-empty text, positive ids, the bucket rule, and that the
    /// coordinator names the task of a decision.
    pub fn validate(&self, role: Role) -> Result<(), ToolError> {
        fn text(field: &'static str, value: &str) -> Result<(), ToolError> {
            if value.trim().is_empty() {
                Err(ToolError::Empty(field))
            } else {
                Ok(())
            }
        }
        fn id(field: &'static str, value: TaskId) -> Result<(), ToolError> {
            if value.0 > 0 {
                Ok(())
            } else {
                Err(ToolError::NotATaskId(field))
            }
        }
        match self {
            Tool::AskUser(a) => text("question", &a.question),
            Tool::ReportStatus(r) => text("status", &r.status),
            Tool::Report(r) => {
                if role != Role::Slice {
                    return Err(ToolError::ReportFromNonSlice);
                }
                text("summary", &r.summary)
            }
            Tool::ClaimCriterion(c) => {
                if !matches!(role, Role::Slice | Role::Coordinator) {
                    return Err(ToolError::ClaimFromOtherRole);
                }
                if c.criterion.0 <= 0 {
                    return Err(ToolError::NotACriterionId);
                }
                text("evidence", &c.evidence)
            }
            Tool::CanvasAddBlock(b) => {
                if b.block_title().is_empty() {
                    return Err(ToolError::Empty("title"));
                }
                Ok(())
            }
            Tool::CanvasAddCard(c) => {
                if c.under.is_some() && c.block.is_none() {
                    return Err(ToolError::BadCard(
                        "`under` places a card in a block: name the `block` too",
                    ));
                }
                if c.kind.has_own_text() {
                    if c.reference.is_some() {
                        return Err(ToolError::BadCard(
                            "an idea or question card holds its own `text`: leave `reference` out",
                        ));
                    }
                    text("text", &c.text)
                } else {
                    if !c.text.trim().is_empty() {
                        return Err(ToolError::BadCard(
                            "a card that points at something has no `text` of its own: leave it out",
                        ));
                    }
                    match &c.reference {
                        Some(r) if r.trim().is_empty() => Err(ToolError::Empty("reference")),
                        Some(_) => Ok(()),
                        None => Err(ToolError::BadCard(
                            "a finding, source, decision, map or task card needs its `reference`: the id of what it points at",
                        )),
                    }
                }
            }
            Tool::CanvasLinkCards(l) => {
                if l.from.0 <= 0 {
                    return Err(ToolError::NotACardId("from"));
                }
                if l.to.0 <= 0 {
                    return Err(ToolError::NotACardId("to"));
                }
                Ok(())
            }
            Tool::DeclareVariants(d) => {
                if role != Role::Slice {
                    return Err(ToolError::DeclareFromNonSlice);
                }
                d.check().map_err(|p| ToolError::BadVariants(p.join("; ")))
            }
            Tool::CreateTask(c) => {
                text("title", &c.title)?;
                if c.bucket == Some(Bucket::Now) {
                    return Err(ToolError::BucketNow);
                }
                Ok(())
            }
            Tool::LogDecision(d) => {
                match (role, d.task) {
                    // A side chat has no task to default to, like the coordinator.
                    (Role::Coordinator | Role::SideChat | Role::Ask, None) => {
                        return Err(ToolError::DecisionNeedsTask);
                    }
                    (_, Some(task)) => id("task", task)?,
                    (Role::Slice, None) => {}
                }
                text("decision", &d.decision)
            }
            Tool::StartSlice(s) => {
                id("task", s.task)?;
                text("name", &s.name)?;
                match (&s.model, s.effort) {
                    (Some(model), _) => text("model", model)?,
                    (None, Some(_)) => return Err(ToolError::EffortNeedsModel),
                    (None, None) => {}
                }
                text("brief", &s.brief)
            }
            Tool::AddProject(_) => Ok(()),
            Tool::ArchiveSlice(a) => text("name", &a.name),
            Tool::ArchiveTask(a) => id("id", a.id),
            Tool::ListTasks(_) => Ok(()),
            Tool::GetTask(g) => id("id", g.id),
            // Checked whole by `dovetail_maps::validate` in the core, so every problem comes
            // back in one report.
            Tool::DrawMap(_) => Ok(()),
            Tool::ResearchStartRound(r) => {
                id("task", r.task)?;
                text("brief", &r.brief)
            }
            Tool::ResearchWriteReport(r) => {
                id("task", r.task)?;
                text("body", &r.body)
            }
            Tool::ResearchPropose(p) => {
                id("task", p.task)?;
                text("text", &p.text)?;
                p.parsed_evidence()?;
                if p.kind == ProposalKind::Decision && p.evidence.is_empty() {
                    return Err(ToolError::DecisionNeedsEvidence);
                }
                Ok(())
            }
            Tool::ResearchGet(g) => {
                id("task", g.task)?;
                g.target().map(|_| ())
            }
            Tool::UpdateTask(u) => {
                id("id", u.id)?;
                if let Some(title) = &u.title {
                    text("title", title)?;
                }
                match u.status {
                    Some(TaskStatus::Archived) => return Err(ToolError::UpdateToArchived),
                    Some(_) => return Err(ToolError::StatusIsSetByDovetail),
                    None => {}
                }
                if u.bucket.is_none()
                    && u.p_level.is_none()
                    && u.title.is_none()
                    && u.notes.is_none()
                {
                    return Err(ToolError::NothingToUpdate);
                }
                Ok(())
            }
        }
    }
}

/// `core_ask_user`: a question for the user's inbox. It never blocks: the call returns at once and the
/// answer arrives as a user message (ADR 0007). An old caller's `blocking` field is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AskUser {
    /// The question, self-contained.
    pub question: String,
}

/// `core_report_status`: the agent's one-line status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReportStatus {
    /// One line; the core cuts it at 120 characters.
    pub status: String,
}

/// `coordination_report`: what a slice tells the coordinator about its task (ADR 0022). It
/// goes through Dovetail, never by session name, so it cannot reach another project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Report {
    /// Where the work stands.
    pub state: ReportState,
    /// What happened, in a line or two.
    pub summary: String,
    /// The pull request the report is about, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
}

/// `coordination_claim_criterion`: an agent says an acceptance criterion is met and why. The
/// criterion becomes claimed and waits in the user's inbox; only the user can make it met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClaimCriterion {
    /// The criterion's id.
    pub criterion: crate::CriterionId,
    /// What shows it is met: a pull request, a test, a finding.
    pub evidence: String,
}

/// `canvas_add_block` (ADR 0027): add a block to an effort's canvas. The core places it in empty
/// space; nothing already on the canvas moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CanvasAddBlock {
    /// The effort; `None` means the one the user last tagged (Unsorted when none).
    #[serde(default)]
    pub effort: Option<crate::EffortId>,
    /// `question_tree`, `options_compare` or `free_area`.
    pub layout: crate::Layout,
    /// What the block is about: the root question, the choice being compared.
    pub title: String,
}

impl CanvasAddBlock {
    /// The title, trimmed.
    pub fn block_title(&self) -> &str {
        self.title.trim()
    }
}

/// `canvas_add_card` (ADR 0025, ADR 0027): put a card into a block, where the core finds room,
/// or into the effort's incoming strip. Nothing already on the canvas moves; there is no tool to
/// move or delete a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CanvasAddCard {
    /// The effort; `None` means the one the user last tagged (Unsorted when none), or the
    /// block's effort when `block` is given.
    #[serde(default)]
    pub effort: Option<crate::EffortId>,
    /// The block to put it in; `None` puts it in the incoming strip.
    #[serde(default)]
    pub block: Option<crate::BlockId>,
    /// A card in that block to put it under and link it from: in a question tree the question
    /// it answers, in an options compare the option it is evidence for.
    #[serde(default)]
    pub under: Option<crate::CardId>,
    /// What the card is: `idea` or `question` with `text`, or `finding`, `source`, `decision`,
    /// `map` or `task` with `reference`.
    pub kind: crate::CardKind,
    /// The words of an idea or a question.
    #[serde(default)]
    pub text: String,
    /// The id of what a finding, source, decision, map or task card points at (a map's id is
    /// text; the others are numbers). It must exist.
    #[serde(default)]
    pub reference: Option<String>,
}

/// `canvas_link_cards` (ADR 0025): link two cards of one effort, such as a question to the
/// finding that answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CanvasLinkCards {
    /// The card the link starts at.
    pub from: crate::CardId,
    /// The card it leads to.
    pub to: crate::CardId,
    /// What the link says, if anything.
    #[serde(default)]
    pub label: String,
}

/// `design_declare_variants` (ADR 0026): replace the list of things the user can switch in the
/// slice's preview. Call it again whenever the list changes, and with `slots: []` when none is
/// left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeclareVariants {
    /// Every slot, in the order the panel shows them.
    pub slots: Vec<crate::VariantSlot>,
}

impl DeclareVariants {
    /// Checks the whole list, so every problem comes back at once: at most
    /// [`crate::MAX_SLOTS`] slots with unique non-empty names, each with 1 to
    /// [`crate::MAX_OPTIONS`] unique non-empty options, and a `kept` that is one of them.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.slots.len() > crate::MAX_SLOTS {
            problems.push(format!("at most {} slots", crate::MAX_SLOTS));
        }
        let mut seen = std::collections::BTreeSet::new();
        for slot in &self.slots {
            let name = slot.name.trim();
            if name.is_empty() {
                problems.push("a slot has an empty name".to_owned());
            } else if !seen.insert(name) {
                problems.push(format!("slot `{name}` appears twice"));
            }
            if slot.options.is_empty() || slot.options.len() > crate::MAX_OPTIONS {
                problems.push(format!(
                    "slot `{name}` needs 1 to {} options",
                    crate::MAX_OPTIONS
                ));
            }
            let mut options = std::collections::BTreeSet::new();
            for o in &slot.options {
                if o.trim().is_empty() {
                    problems.push(format!("slot `{name}` has an empty option"));
                } else if !options.insert(o.as_str()) {
                    problems.push(format!("slot `{name}` has the option `{o}` twice"));
                }
            }
            if let Some(kept) = &slot.kept
                && !slot.options.contains(kept)
            {
                problems.push(format!("slot `{name}`: `kept` is not one of its options"));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// Where a slice says its work stands, in a [`Report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ReportState {
    /// Still working; something worth knowing happened.
    Progress,
    /// The work is ready for review.
    Done,
    /// It cannot go on without the coordinator.
    Blocked,
}

impl ReportState {
    /// The word shown in the report line.
    pub fn as_str(self) -> &'static str {
        match self {
            ReportState::Progress => "progress",
            ReportState::Done => "done",
            ReportState::Blocked => "blocked",
        }
    }
}

/// `tasks_create`: a task filed by the coordinator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateTask {
    /// Short title, 6 words at most.
    pub title: String,
    /// Context, acceptance criteria, links.
    #[serde(default)]
    pub notes: String,
    /// Priority, if given.
    pub p_level: Option<PLevel>,
    /// `next` or `later`; `None` lands in Unsorted. `now` fails [`Tool::validate`].
    pub bucket: Option<Bucket>,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Related tasks.
    #[serde(default)]
    pub related: Vec<TaskId>,
}

/// `core_log_decision`: an entry in a task's decision log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogDecision {
    /// The task; required for the coordinator, a slice's own task by default.
    pub task: Option<TaskId>,
    /// The decision, one line.
    pub decision: String,
    /// The reason.
    pub why: Option<String>,
    /// Options considered and rejected.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    /// The earlier entry it replaces.
    pub supersedes: Option<DecisionId>,
}

/// `project_start_slice`: start a slice on an existing task (ADR 0004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StartSlice {
    /// An existing task.
    pub task: TaskId,
    /// Slice name, used for the branch and worktree.
    pub name: String,
    /// The slice's first prompt.
    pub brief: String,
    /// The repo it works in, by name (ADR 0021). May be left out when the project has one;
    /// required with several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The model the session starts on: an alias (`haiku`, `sonnet`, `opus`) or a full model id.
    /// Left out, the project's default applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The effort it starts at; needs a `model`. Left out, Claude Code's default applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
}

/// `project_add`: register the folder the coordinator runs in as a Dovetail project (ADR 0005).
/// Takes no arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AddProject {}

/// `project_archive_slice`: archive a slice by name (task #9). Clean, pushed work goes at once; unsaved
/// work waits for the user's yes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArchiveSlice {
    /// The slice's name, as given to `project_start_slice`.
    pub name: String,
}

/// `tasks_archive`: archive a task, and first its live slices, with `project_archive_slice`'s rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArchiveTask {
    /// The task.
    pub id: TaskId,
}

/// A bucket as `tasks_list` filters it: the three [`Bucket`]s plus Unsorted (no bucket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum BucketFilter {
    /// [`Bucket::Now`].
    Now,
    /// [`Bucket::Next`].
    Next,
    /// No bucket yet.
    Unsorted,
    /// [`Bucket::Later`].
    Later,
}

impl BucketFilter {
    /// Whether a task with `bucket` is in this one.
    pub fn matches(self, bucket: Option<Bucket>) -> bool {
        match self {
            BucketFilter::Now => bucket == Some(Bucket::Now),
            BucketFilter::Next => bucket == Some(Bucket::Next),
            BucketFilter::Unsorted => bucket.is_none(),
            BucketFilter::Later => bucket == Some(Bucket::Later),
        }
    }
}

/// `tasks_list`: one line per task, Now, Next, Unsorted, Later (task #20). Every field is
/// optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct ListTasks {
    /// Only tasks in this bucket.
    pub bucket: Option<BucketFilter>,
    /// Only tasks with this status; `archived` lists archived tasks without
    /// `include_archived`.
    pub status: Option<TaskStatus>,
    /// Also list archived tasks.
    pub include_archived: bool,
}

/// `tasks_get`: one task in full, with its agents and decision log (task #20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GetTask {
    /// The task.
    pub id: TaskId,
}

/// `tasks_update`: change some fields of a task (task #25). Only the fields given change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateTask {
    /// The task.
    pub id: TaskId,
    /// No longer settable: Dovetail sets task status itself (ADR 0024). Still parsed, so a
    /// caller that sends it gets [`Tool::validate`]'s explanation, not a schema error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    /// `now`, `next`, `later` or `unsorted`. A task the coordinator moves into Now is marked
    /// unseen until the user sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<BucketFilter>,
    /// Absent: unchanged; `null`: no priority; a level: that level.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub p_level: Option<Option<PLevel>>,
    /// New title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// New notes, replacing the old.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl UpdateTask {
    /// Where `bucket` moves the task: `Some(None)` is Unsorted, `None` leaves it.
    pub fn new_bucket(&self) -> Option<Option<Bucket>> {
        self.bucket.map(|b| match b {
            BucketFilter::Now => Some(Bucket::Now),
            BucketFilter::Next => Some(Bucket::Next),
            BucketFilter::Unsorted => None,
            BucketFilter::Later => Some(Bucket::Later),
        })
    }
}

/// `maps_draw`: a structure or swimlane map (design 23i). The arguments stay raw JSON here:
/// their shape is `dovetail_maps::DrawMapInput`, which lives with the validation (decision on
/// task #34), and the core parses and validates them there so every problem comes back at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct DrawMap(pub serde_json::Value);

/// `research_start_round`: open the next round of a research task under the user's criteria
/// (`docs/research.md` §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResearchStartRound {
    /// An existing task (the question).
    pub task: TaskId,
    /// The user's criteria for this round, verbatim where possible.
    pub brief: String,
    /// Whether the task asks a `question` or makes a `decision`; sets the task's research kind
    /// (a decision is held to the decision template). Omit to leave it as it is.
    #[serde(default)]
    pub kind: Option<crate::research::ResearchKind>,
}

/// `research_write_report`: store the report of a task, closing its open round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResearchWriteReport {
    /// The research task.
    pub task: TaskId,
    /// The whole report in markdown, citing `[F3]` findings and `[S12]` sources.
    pub body: String,
}

/// `research_propose`: an idea or decision for the user to accept from Needs you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResearchPropose {
    /// The research task.
    pub task: TaskId,
    /// `idea` or `decision`.
    pub kind: ProposalKind,
    /// The idea, or the decision in one line.
    pub text: String,
    /// Finding and source ids, like `F3`, `S12`. Required for a decision.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Why, if any.
    pub why: Option<String>,
    /// Options considered and rejected.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
}

impl ResearchPropose {
    /// The `evidence` strings as ids, without duplicates, in the order given.
    pub fn parsed_evidence(&self) -> Result<Vec<Evidence>, ToolError> {
        let mut out = Vec::new();
        for text in &self.evidence {
            let e = Evidence::parse(text).ok_or_else(|| ToolError::BadEvidence(text.clone()))?;
            if !out.contains(&e) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

/// `research_get`: read one stored research document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResearchGet {
    /// The research task.
    pub task: TaskId,
    /// `report`, `round:N` (its brief), `finding:N` or `source:N` (the `F`/`S` ids).
    pub what: String,
}

impl ResearchGet {
    /// What `what` asks for.
    pub fn target(&self) -> Result<ResearchWhat, ToolError> {
        let bad = || ToolError::BadWhat(self.what.clone());
        let what = self.what.trim();
        if what == "report" {
            return Ok(ResearchWhat::Report);
        }
        let (kind, n) = what.split_once(':').ok_or_else(bad)?;
        let n = n.trim();
        // `finding:F3` and `finding:3` both work: the ids are the ones the report cites.
        let n = n.trim_start_matches(['F', 'S']);
        let n: i64 = n.parse().ok().filter(|n| *n > 0).ok_or_else(bad)?;
        match kind.trim() {
            "round" => Ok(ResearchWhat::Brief {
                round: u32::try_from(n).map_err(|_| bad())?,
            }),
            "finding" => Ok(ResearchWhat::Finding(FindingId(n))),
            "source" => Ok(ResearchWhat::Source(SourceId(n))),
            _ => Err(bad()),
        }
    }
}

/// Deserializes a field that is present, so `null` becomes `Some(None)`; only an absent field
/// (through `#[serde(default)]`) stays `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
