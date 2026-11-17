//! Messages between the core and the UI.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use serde::{Deserialize, Serialize};

use crate::effort::{CriterionId, Effort, EffortChange, EffortId, NewEffort, WhereWasI};
use crate::map::{Map, MapId, MapSummary};
use crate::model::{
    ActivityItem, Agent, AgentId, AgentMessage, Alternative, Attachment, Bucket, ConversationEntry,
    Decision, DecisionId, EntryId, InboxId, InboxItem, JournalState, ModelOption, PLevel,
    ProjectId, ProjectSummary, ReasoningEffort, RepoId, SlashCommand, Snapshot, Task, TaskId,
    TaskStatus, TodoItem, TodoListId, Totals, UsageLimits, Warning, WarningId, WorktreeInfo,
};
use crate::research::{
    BrowseWhat, BrowsedDoc, BrowsedProject, DocEntry, ResearchContent, ResearchDocument,
    ResearchSummary, ResearchWhat,
};

/// Version of this protocol, sent in [`ServerMsg::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Everything the core sends: a hello, then a snapshot, then numbered events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// One snapshot per connection or resync against many events: boxing it would only add an
// allocation to every match on the stream.
#[allow(clippy::large_enum_variant)]
#[serde(rename_all = "snake_case")]
pub enum ServerMsg {
    /// First message on a connection.
    Hello {
        /// The core's [`PROTOCOL_VERSION`].
        protocol: u32,
    },
    /// The full state; replaces whatever the client held.
    Snapshot(Snapshot),
    /// One change, to fold in with [`crate::apply`].
    Event {
        /// Must be one more than the client's current `seq`.
        seq: u64,
        /// The change.
        event: Event,
    },
}

/// One change to the client's [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// A task was created or changed.
    TaskUpserted(Task),
    /// A task was deleted.
    TaskRemoved(TaskId),
    /// An effort was created or changed: its fields, its "where was I" card or its criteria.
    /// It carries the whole effort. Efforts are never removed.
    EffortUpserted(Effort),
    /// A slice's preview changed state (ADR 0026).
    PreviewChanged(crate::design::Preview),
    /// A slice declared its variants, or the user picked or kept an option (ADR 0026). It
    /// carries the whole set.
    VariantsUpserted(crate::design::VariantSet),
    /// A block was created or renamed (ADR 0027).
    BlockUpserted(crate::canvas::Block),
    /// A card was added, placed, moved or changed. It carries the whole card.
    CardUpserted(crate::canvas::Card),
    /// A card was deleted or dropped; its links go with it.
    CardRemoved(crate::canvas::CardId),
    /// Two cards were linked, or a link was relabelled.
    CardLinked(crate::canvas::CardLink),
    /// A link was removed.
    CardUnlinked {
        /// The card it started at.
        from: crate::canvas::CardId,
        /// The card it led to.
        to: crate::canvas::CardId,
    },
    /// A card's body, in answer to [`Command::LoadCard`]: the target's text for a reference. Changes
    /// nothing in the snapshot.
    CardLoaded {
        /// The card.
        id: crate::canvas::CardId,
        /// The body, as markdown.
        body: String,
    },
    /// An agent was created or changed.
    AgentUpserted(Agent),
    /// An agent was removed, with its conversation.
    AgentRemoved(AgentId),
    /// A native to-do list was rewritten.
    TodosReplaced {
        /// The list.
        list: TodoListId,
        /// Its items now.
        items: Vec<TodoItem>,
    },
    /// An item arrived in Needs you, or an existing one changed.
    InboxUpserted(InboxItem),
    /// An item left Needs you.
    InboxResolved {
        /// The item.
        id: InboxId,
    },
    /// A slice's worktree state was gathered afresh. A worktree's entry goes with its agent
    /// ([`Event::AgentRemoved`]).
    WorktreeUpserted(WorktreeInfo),
    /// Something happened in the Activity feed.
    ActivityAdded(ActivityItem),
    /// Agents messaged each other.
    MessageAdded(AgentMessage),
    /// A conversation entry was added or replaced.
    ConversationAppended {
        /// Whose conversation.
        agent: AgentId,
        /// The entry.
        entry: ConversationEntry,
    },
    /// More streamed text for an assistant entry.
    AssistantDelta {
        /// Whose conversation.
        agent: AgentId,
        /// The entry it extends.
        entry: EntryId,
        /// Text to append.
        text: String,
    },
    /// Older entries, in answer to [`Command::LoadConversation`].
    ConversationPage {
        /// Whose conversation.
        agent: AgentId,
        /// Entries older than the tail, oldest first.
        entries: Vec<ConversationEntry>,
        /// Still older entries exist.
        has_more: bool,
    },
    /// Status-bar totals changed.
    TotalsChanged(Totals),
    /// A session reported new usage-limit figures.
    UsageChanged(UsageLimits),
    /// A warning was raised or reworded.
    WarningSet(Warning),
    /// A warning no longer applies.
    WarningCleared(WarningId),
    /// An entry was added to a task's decision log.
    DecisionAdded(Decision),
    /// A map was drawn (version 1) or redrawn (a later version): its shelf entry now.
    MapUpserted(MapSummary),
    /// A map version's content, in answer to [`Command::LoadMap`]. Changes nothing in the
    /// snapshot; the UI keeps what it shows.
    MapLoaded(Map),
    /// A task's research changed: a round opened or closed, a finding was filed.
    ResearchUpserted(ResearchSummary),
    /// Research content, in answer to [`Command::LoadResearch`]. Changes nothing in the
    /// snapshot.
    ResearchLoaded {
        /// The task.
        task: TaskId,
        /// What was asked for.
        what: ResearchWhat,
        /// The answer.
        content: ResearchContent,
    },
    /// A tangent was saved or changed: asked, answered, triaged (ADR 0017).
    TangentUpserted(crate::model::TangentSummary),
    /// A tangent left the list: dropped, filed into a project, or attached to a task (its
    /// Tangents task is archived).
    TangentRemoved(TaskId),
    /// A tangent's content, in answer to [`Command::LoadTangent`]. Changes nothing in the
    /// snapshot.
    TangentLoaded {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// What was asked for.
        what: ResearchWhat,
        /// The answer.
        content: ResearchContent,
    },
    /// The project's read-only docs, in answer to [`Command::ListDocs`]. Changes nothing in the
    /// snapshot.
    DocsListed {
        /// Markdown files under the repo's `docs/`, sorted by path; empty for a project with no
        /// repo.
        docs: Vec<DocEntry>,
    },
    /// One doc's text, in answer to [`Command::LoadDoc`]. Changes nothing in the snapshot.
    DocLoaded {
        /// The doc, as [`DocEntry::path`] names it.
        path: PathBuf,
        /// Its text; HTML is left as it is, for the UI to convert.
        document: ResearchDocument,
    },
    /// Another project's library, in answer to [`Command::BrowseProject`]. Changes nothing in
    /// the snapshot.
    ProjectBrowsed(Box<BrowsedProject>),
    /// A browsed project's document, in answer to [`Command::LoadBrowsed`]. Changes nothing in
    /// the snapshot.
    BrowsedLoaded {
        /// Which project's document.
        doc: BrowsedDoc,
        /// Its text; HTML is left as it is, for the UI to convert.
        document: ResearchDocument,
    },
    /// The models the selector offers changed: config was read, or a session listed what its
    /// models accept.
    ModelsChanged(Vec<ModelOption>),
    /// A command could not be carried out; the UI shows the message.
    CommandFailed {
        /// What went wrong.
        message: String,
    },
    /// The clipboard was read, in answer to [`Command::PasteClipboard`]. Changes nothing in the
    /// snapshot; the UI puts the result in that agent's chat bar if it is still open.
    ClipboardPasted {
        /// The agent whose chat bar asked.
        agent: AgentId,
        /// What the clipboard gave.
        paste: Paste,
    },
    /// A session listed its slash commands and skills (at start, and when they change).
    CommandsChanged {
        /// The session.
        agent: AgentId,
        /// Everything it offers, in its own order.
        commands: Vec<SlashCommand>,
    },
    /// Paths for `@` completion in a chat bar, in answer to [`Command::ListPaths`]. Changes
    /// nothing in the snapshot.
    PathsListed {
        /// The agent whose chat bar asked.
        agent: AgentId,
        /// The directory asked about, as typed; `None` for the session's whole tree.
        dir: Option<String>,
        /// Paths relative to the session's folder (whole tree), or names in `dir` with a
        /// trailing `/` on directories. Empty when nothing could be listed.
        paths: Vec<String>,
    },
    /// The core accepted [`Command::OpenEditor`]: the UI lets go of the terminal and answers
    /// [`Command::TerminalReleased`]. Changes nothing in the snapshot.
    LendTerminal,
    /// The program run in the user's terminal ([`Command::OpenEditor`], [`Command::TakeOver`])
    /// has exited, so the UI takes the terminal back. Changes nothing in the snapshot.
    TerminalReturned {
        /// Why the program could not run or failed, for the status bar; `None` when it exited
        /// cleanly.
        error: Option<String>,
    },
    /// The end-of-day journal changed: `:eod` started waiting for the handoff, or a day's file
    /// was written (ADR 0012).
    JournalChanged(JournalState),
    /// `:upgrade` started building the next binary (`true`) or the build finished, failed or was
    /// cancelled (`false`); the result arrives as a Needs-you item.
    UpgradeBuilding(bool),
    /// Every agent is idle and the user said `y` on "Upgrade ready" (ADR 0014): the core is
    /// quitting, and the UI quits too, without asking. The binary then swaps the staged build in
    /// and starts it in the same terminal. Changes nothing in the snapshot.
    UpgradeRestarting,
    /// The user picked another project (B24): the core is quitting so the binary can start on
    /// `id`, and the UI quits too, without asking. Changes nothing in the snapshot.
    OpeningProject {
        /// The project to open next.
        id: ProjectId,
    },
    /// A project was created or changed: renamed, its account set, a repo added or removed
    /// (ADR 0021). Replaces it in `projects`, or adds it; replaces `project` too when it is the
    /// open one.
    ProjectUpserted(ProjectSummary),
    /// [`Command::RegisterProject`] wrote a starter recipe for the project, which had none
    /// (task #31). The UI offers `profiles` (config.toml's, empty for none) and answers with
    /// [`Command::SetRecipeProfile`]. Changes nothing in the snapshot.
    RecipeStarted {
        /// The project just added.
        project: ProjectId,
        /// Its display name.
        name: String,
        /// The recipe file written.
        path: PathBuf,
        /// The profile names in `config.toml`, sorted.
        profiles: Vec<String>,
    },
}

/// What the core found on the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Paste {
    /// An accepted image (PNG, JPEG, GIF or WebP, at most [`crate::MAX_ATTACHMENT_BYTES`]),
    /// already saved under Dovetail's `attachments/` folder: the UI adds it as a chip.
    Image(Attachment),
    /// Text, to insert at the cursor.
    Text(String),
    /// Nothing could be pasted; a short notice for the status bar saying why.
    Refused(String),
}

/// Something the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Send the user's prompt to an agent.
    Prompt {
        /// Recipient.
        agent: AgentId,
        /// Prompt text; may be empty when images are attached.
        text: String,
        /// Images to send with it, saved by the core on [`Command::PasteClipboard`]; the core
        /// reads the files.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
        /// The effort tag for this message and the agent's replies to it; `None` sends it
        /// untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<EffortId>,
    },
    /// Read the system clipboard for an agent's chat bar (Ctrl-V). The core answers with
    /// [`Event::ClipboardPasted`]: an image saved as an attachment, text, or a refusal.
    PasteClipboard {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
    },
    /// Text pasted into an agent's chat bar (a bracketed paste or a dropped file). The core
    /// answers with [`Event::ClipboardPasted`]: an image attachment when the text is a path to
    /// an existing image (`~`, `file://` and shell quoting understood), else the text itself.
    PasteText {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
        /// What the terminal delivered.
        text: String,
    },
    /// List paths for `@` completion in an agent's chat bar. `dir: None` lists the files of the
    /// agent's folder (the main checkout or its worktree) that git does not ignore; `Some` lists
    /// one directory, `~/`, absolute, or relative to that folder. The core answers with
    /// [`Event::PathsListed`].
    ListPaths {
        /// The agent whose chat bar is completing.
        agent: AgentId,
        /// The directory, as typed after `@`, ending in `/`.
        dir: Option<String>,
    },
    /// Put text on the system clipboard (`c` in the worktree view copies a path). A failure
    /// comes back as [`Event::CommandFailed`]; success sends nothing.
    CopyToClipboard {
        /// The text.
        text: String,
    },
    /// Open a folder or file in the user's editor (`$EDITOR`, else `nvim`), in the terminal the
    /// UI runs in. A request: the UI still holds the terminal. The core checks the path first;
    /// a missing one comes back as [`Event::CommandFailed`] and the UI never lets go (no
    /// flicker). Otherwise the core answers [`Event::LendTerminal`], the UI lets go of the
    /// terminal (leaves the alternate screen and raw mode, stops reading keys) and sends
    /// [`Command::TerminalReleased`], and the core runs the editor. The UI takes the terminal
    /// back on [`Event::TerminalReturned`], which the core sends once the editor exits or fails
    /// to start.
    OpenEditor {
        /// What to open; the editor also starts in it when it is a folder.
        path: PathBuf,
    },
    /// The UI has let go of the terminal, as [`Event::LendTerminal`] asked: the core runs the
    /// editor it accepted with [`Command::OpenEditor`]. Ignored when none is waiting.
    TerminalReleased,
    /// Stop an agent's current turn.
    Interrupt {
        /// The agent.
        agent: AgentId,
    },
    /// Answer a permission request.
    AnswerPermission {
        /// The inbox item.
        item: InboxId,
        /// The answer.
        answer: PermissionAnswer,
    },
    /// Answer a question.
    AnswerQuestion {
        /// The inbox item.
        item: InboxId,
        /// The answer.
        text: String,
    },
    /// Remove an inbox item without answering it.
    DismissInbox {
        /// The inbox item.
        item: InboxId,
    },
    /// Switch a session's model and effort (task #39). A running session switches live and the
    /// core answers with [`Event::AgentUpserted`] once Claude Code confirms, or
    /// [`Event::CommandFailed`] with the old model kept. Either way the choice is used from the
    /// session's next start.
    SetModel {
        /// The session.
        agent: AgentId,
        /// The model, as [`ModelOption::name`].
        model: String,
        /// The effort; `None` leaves it to the model's default (a model with no levels).
        effort: Option<ReasoningEffort>,
    },
    /// Move a session to its next permission mode.
    CyclePermissionMode {
        /// The session.
        agent: AgentId,
    },
    /// Start a slice.
    NewSlice {
        /// The task it works on.
        task: Option<TaskId>,
        /// Slice name, also used for its branch.
        name: String,
        /// First prompt.
        brief: String,
        /// The repo it works in, by name (ADR 0021). May be left out when the project has one.
        #[serde(default)]
        repo: Option<String>,
    },
    /// End a slice session, or a side chat.
    CloseSlice {
        /// The slice or side chat.
        agent: AgentId,
    },
    /// Close the day (`:eod`, ADR 0012): ask the coordinator for its handoff, then write today's
    /// journal file with it and Dovetail's facts. The core answers with
    /// [`Event::JournalChanged`]: `closing` while it waits, then the new `latest`.
    EndOfDay,
    /// A mid-day fresh start (`:fresh`, ADR 0012 amendment): ask the coordinator for the same
    /// handoff as [`Command::EndOfDay`], append it to today's journal file as a "Handoff (fresh
    /// start HH:MM)" section, and start a new coordinator session seeded with it and the open
    /// tasks. Refused with an error while a turn runs. The previous session stays resumable
    /// through a Needs-you item.
    FreshStart,
    /// Open a side chat with `question` as its first prompt (ADR 0011).
    NewSideChat {
        /// The first prompt.
        question: String,
    },
    /// Ask a tangent (`:tangent [question]`, `T` in the side panel; ADR 0017): a question that
    /// does not belong to the open project. The core saves it as a task in Tangents' Unsorted
    /// list and starts a read-only session for it in Tangents' folder (side chats' launch lists,
    /// Core suite only), shown in the side panel. Its answers and the sources it and its
    /// subagents reached are filed on that task as findings. The core answers with
    /// [`Event::TangentUpserted`] and [`Event::AgentUpserted`].
    NewTangent {
        /// The question, also its first prompt.
        question: String,
    },
    /// Attach a tangent to a task of the open project (`a` in the side panel or the tangents
    /// list): its answers and sources become quick findings of `task`, and the tangent leaves
    /// Tangents (its task there is archived, with a note saying where it went).
    AttachTangent {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// The open project's task.
        task: TaskId,
    },
    /// File a tangent into a project (`f` in the tangents list): a new task in that project's
    /// Unsorted list with the tangent's question, notes, answers and sources; the tangent leaves
    /// Tangents (archived there, with a note). Refused for Tangents itself.
    FileTangent {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// The project.
        project: ProjectId,
    },
    /// Start a task from a tangent (`s` in the tangents list): a new task in the project's
    /// Unsorted list whose notes link back to the tangent, which stays in Tangents, marked done.
    StartFromTangent {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// The project.
        project: ProjectId,
    },
    /// Drop a tangent (`d` in the tangents list): its task in Tangents is archived, nothing is
    /// deleted.
    DropTangent {
        /// The tangent's task in Tangents.
        tangent: TaskId,
    },
    /// Fetch a tangent's content, as [`Command::LoadResearch`] does for the open project's
    /// tasks. The core answers with [`Event::TangentLoaded`].
    LoadTangent {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// What to fetch.
        what: ResearchWhat,
    },
    /// Hand a session to native `claude` in the UI's terminal (`<leader>st`, M14). Like
    /// [`Command::OpenEditor`], the UI sends it only after it has let go of the terminal, and
    /// takes the terminal back on [`Event::TerminalReturned`], which the core always sends: at
    /// once when it refuses, else when native exits. Headless resumes after.
    TakeOver {
        /// The session.
        agent: AgentId,
    },
    /// `:upgrade` (ADR 0014): build the next binary in the background. An "Upgrade ready" item
    /// appears in Needs you when it is done, an Error item when the build fails; a refusal comes
    /// back as [`Event::CommandFailed`].
    Upgrade {
        /// Build the project's checkout instead of `origin/main` (`:upgrade --local`).
        local: bool,
    },
    /// Create a task.
    CreateTask(NewTask),
    /// Change fields of a task.
    UpdateTask {
        /// The task.
        id: TaskId,
        /// Changes, applied in order.
        patch: Vec<TaskChange>,
    },
    /// Add a block to an effort's canvas (ADR 0027). The core places it right of everything on
    /// that canvas. Answers with [`Event::BlockUpserted`].
    CreateBlock {
        /// The effort.
        effort: EffortId,
        /// Its title; empty becomes the layout's name.
        title: String,
        /// How it lays out its cards.
        layout: crate::canvas::Layout,
    },
    /// Rename a block.
    RenameBlock {
        /// The block.
        id: crate::canvas::BlockId,
        /// The new title.
        title: String,
    },
    /// Add a card: into `card.block` where the core finds room (under `link_from` when that card
    /// is in the same block), else into the effort's incoming strip. With `link_from`, the new
    /// card is also linked from that card. The core checks that a reference's target exists and
    /// answers with [`Event::CardUpserted`].
    AddCard {
        /// The card.
        card: crate::canvas::NewCard,
        /// A card to link from, of the same effort.
        #[serde(default)]
        link_from: Option<crate::canvas::CardId>,
    },
    /// Place an incoming card in a block, where the core finds room: under the placed card that
    /// links to it, if any. With no block named: that card's block, else the effort's newest
    /// block, else a new free area. Refused for a card already placed.
    PlaceCard {
        /// The card.
        id: crate::canvas::CardId,
        /// The block, if the user chose one.
        #[serde(default)]
        block: Option<crate::canvas::BlockId>,
    },
    /// The user has seen a card an agent put into a block (ADR 0027).
    SeeCard {
        /// The card.
        id: crate::canvas::CardId,
    },
    /// Move a placed card. Refused for an incoming card.
    MoveCard {
        /// The card.
        id: crate::canvas::CardId,
        /// Position in canvas pixels.
        x: i64,
        /// Position in canvas pixels.
        y: i64,
    },
    /// Delete a card and its links. The card's target is not touched.
    DeleteCard {
        /// The card.
        id: crate::canvas::CardId,
    },
    /// Drop an incoming card (`x` in the strip). Refused for a placed card.
    DropIncoming {
        /// The card.
        id: crate::canvas::CardId,
    },
    /// Link two cards of one effort; linking them again changes the label.
    LinkCards {
        /// Where the link starts.
        from: crate::canvas::CardId,
        /// Where it leads.
        to: crate::canvas::CardId,
        /// What it says.
        #[serde(default)]
        label: String,
    },
    /// Remove a link.
    UnlinkCards {
        /// Where the link starts.
        from: crate::canvas::CardId,
        /// Where it leads.
        to: crate::canvas::CardId,
    },
    /// Turn a chat reply into an idea card in an effort's incoming strip, by copying its text
    /// (ADR 0025, point 6). Refused for an unknown entry or one with no text.
    CardFromReply {
        /// Whose conversation.
        agent: AgentId,
        /// The entry.
        entry: EntryId,
        /// The effort the card goes to.
        effort: EffortId,
    },
    /// Start a slice's preview (ADR 0026) unless it already starts or runs. The viewer sends it
    /// when it opens on the slice. Answered with [`Event::PreviewChanged`].
    StartPreview {
        /// The slice.
        agent: AgentId,
    },
    /// Stop a slice's preview and start it again.
    RestartPreview {
        /// The slice.
        agent: AgentId,
    },
    /// Pick an option of a declared variant slot. Refused for an unknown slot or option.
    SelectVariant {
        /// The slice.
        agent: AgentId,
        /// The slot's name.
        slot: String,
        /// The option.
        option: String,
    },
    /// "Keep this combination": mark the picked options as kept and prompt the slice to make
    /// them permanent and delete the rest.
    KeepVariants {
        /// The slice.
        agent: AgentId,
    },
    /// Fetch a card's body. The core answers with [`Event::CardLoaded`].
    LoadCard {
        /// The card.
        id: crate::canvas::CardId,
    },
    /// Create an effort. The core converts the name to lower kebab case
    /// ([`crate::effort_name`]) and refuses an empty one or one already taken. It answers with
    /// [`Event::EffortUpserted`].
    CreateEffort(NewEffort),
    /// Rename an effort; the name is converted as for [`Command::CreateEffort`]. Refused for
    /// Unsorted.
    RenameEffort {
        /// The effort.
        id: EffortId,
        /// The new name, as typed.
        name: String,
    },
    /// Change fields of an effort. Closing Unsorted is refused.
    UpdateEffort {
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
    /// Put a task in an effort and say which of its criteria the task serves. Moving to another
    /// effort with `criterion: None` clears the criterion. Refused for a closed effort, or a
    /// criterion of another effort. The core answers with [`Event::TaskUpserted`].
    MoveTask {
        /// The task.
        id: TaskId,
        /// The effort it belongs to from now on.
        effort: EffortId,
        /// The criterion it serves, if any.
        criterion: Option<CriterionId>,
    },
    /// Add an entry to a task's decision log, as the user.
    LogDecision(NewDecision),
    /// Archive a task.
    ArchiveTask {
        /// The task.
        id: TaskId,
    },
    /// Move a task up (negative) or down within its bucket.
    ReorderTask {
        /// The task.
        id: TaskId,
        /// Places to move.
        delta: i32,
    },
    /// Remove a slice's worktree.
    RemoveWorktree {
        /// The slice.
        agent: AgentId,
        /// The user has confirmed losing unpushed work.
        confirmed: bool,
    },
    /// Gather every slice worktree's git and PR state again (sent when the worktree view
    /// opens): `git fetch origin <base>` first, then local git and `gh pr list`. The core
    /// answers with one [`Event::WorktreeUpserted`] per worktree.
    RefreshWorktrees,
    /// Switch to another registered project (B24): the core answers with
    /// [`Event::OpeningProject`] and quits, ending this project's sessions, and the binary starts
    /// again on the other one.
    OpenProject {
        /// The project.
        id: ProjectId,
    },
    /// Create and register a research project named `name` (`:project new <name> --research`).
    /// It has no repo; its folder is made under Dovetail's state folder. The core answers with a
    /// fresh snapshot, whose `projects` lists it.
    NewResearchProject {
        /// Display name; the id is its slug.
        name: String,
    },
    /// Create a project (ADR 0021): a name, an optional account and an optional first repo.
    /// Answered with [`Event::ProjectUpserted`]. Home and the core both take it.
    CreateProject {
        /// Display name; the id is its slug, numbered when taken.
        name: String,
        /// A profile name from `config.toml`; `None` for the default Claude home.
        #[serde(default)]
        account: Option<String>,
        /// A folder inside a git repo, added as its first repo.
        #[serde(default)]
        repo: Option<PathBuf>,
    },
    /// Rename a project. Its id, and so its folders, stay.
    RenameProject {
        /// The project.
        id: ProjectId,
        /// The new display name.
        name: String,
    },
    /// Set the Claude account a project's sessions run under; `None` goes back to the recipe's
    /// `profile`, else the default. Running sessions keep theirs until they next start.
    SetProjectAccount {
        /// The project.
        id: ProjectId,
        /// A profile name from `config.toml`.
        account: Option<String>,
    },
    /// Add a repo to a project, by a path inside it. The open project's coordinator gets it at
    /// its next start, at once when it is idle.
    AddRepo {
        /// The project.
        project: ProjectId,
        /// A folder inside the repo.
        path: PathBuf,
        /// What agents call it; `None` for the slug of the folder's name.
        #[serde(default)]
        name: Option<String>,
    },
    /// Remove a repo from a project. Refused while a slice works in it. Nothing on disk changes.
    RemoveRepo {
        /// The project.
        project: ProjectId,
        /// The repo.
        repo: RepoId,
    },
    /// Register a repo as a project.
    RegisterProject {
        /// Path inside the repo.
        path: PathBuf,
    },
    /// Uncomment `profile = "<profile>"` in the starter recipe [`Event::RecipeStarted`] named
    /// (task #31). Fails when the line is gone (the file was edited) or config.toml lacks it.
    SetRecipeProfile {
        /// The open project.
        project: ProjectId,
        /// A profile name from `config.toml`.
        profile: String,
    },
    /// Fetch conversation entries older than the tail.
    LoadConversation {
        /// Whose conversation.
        agent: AgentId,
        /// Fetch entries before this one; `None` means before the tail.
        before: Option<EntryId>,
    },
    /// Fetch a map's content. The core answers with [`Event::MapLoaded`], or
    /// [`Event::CommandFailed`] for an unknown map or version.
    LoadMap {
        /// The map.
        id: MapId,
        /// The version; `None` means the latest.
        version: Option<u32>,
    },
    /// Open a research round on a task (`:round <task> <brief>`, `docs/research.md` §5.1):
    /// subagents the coordinator spawns while it is open are filed under it. While another round
    /// is open the new one is queued instead (its `brief.md` is written at once) and opens, in
    /// request order, when the open one closes. Refused for an unknown or archived task, an
    /// empty brief, or a folder that is not a project yet; the refusal comes back as
    /// [`Event::CommandFailed`].
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
    SetResearchKind {
        /// The task.
        task: TaskId,
        /// The kind.
        kind: crate::research::ResearchKind,
    },
    /// File an unfiled coordinator finding under a task (`y` on its Needs-you item, then a task
    /// from the picker): the finding and its source files move to the task's research folder.
    /// `d` on the item drops the finding with [`Command::DismissInbox`].
    FileUnfiled {
        /// The "Unfiled finding" item.
        item: InboxId,
        /// The task to file it under.
        task: TaskId,
    },
    /// Fetch research content. The core answers with [`Event::ResearchLoaded`], or
    /// [`Event::CommandFailed`] when there is no such thing.
    LoadResearch {
        /// The task.
        task: TaskId,
        /// What to fetch.
        what: ResearchWhat,
    },
    /// Open a web page in the user's browser (`gx` in reading mode, ADR 0018): `$BROWSER` if
    /// set, else `xdg-open`. Only `http` and `https` URLs; anything else, or a launch that fails,
    /// comes back as [`Event::CommandFailed`]. Success sends nothing.
    OpenExternal {
        /// The page.
        url: String,
    },
    /// Make the Ask session of a research task ready (`i` in reading mode, ADR 0018): a
    /// read-only session grounded in the task's report and the sources it cites, apart from
    /// the coordinator. The core answers with [`Event::AgentUpserted`] for an
    /// [`crate::AgentKind::Ask`] agent of the task (the one there is, or a new one); questions
    /// then go to it as [`Command::Prompt`]s. Refused for a task with no report.
    StartAsk {
        /// The research task.
        task: TaskId,
    },
    /// List the project's read-only docs (reading mode's Docs · ADRs). The core answers with
    /// [`Event::DocsListed`].
    ListDocs,
    /// Fetch one doc listed by [`Event::DocsListed`]. The core answers with
    /// [`Event::DocLoaded`], or [`Event::CommandFailed`] for a path outside the docs.
    LoadDoc {
        /// As [`DocEntry::path`] names it.
        path: PathBuf,
    },
    /// Load another registered project's library read-only (`<leader>fr` in reading mode, ADR
    /// 0018 decided 2026-11-11 later): its research, decision log and docs, from its
    /// `state.db` opened read-only and its files. Nothing of that project is written and the
    /// open project does not change. The core answers with [`Event::ProjectBrowsed`], or
    /// [`Event::CommandFailed`].
    BrowseProject {
        /// The project.
        id: ProjectId,
    },
    /// Fetch one document of a browsed project, read-only. The core answers with
    /// [`Event::BrowsedLoaded`], or [`Event::CommandFailed`] when there is no such thing.
    LoadBrowsed {
        /// The project.
        project: ProjectId,
        /// The document.
        what: BrowseWhat,
    },
    /// Send a fresh snapshot (after a [`crate::ApplyResult::Gap`]).
    Resync,
    /// Shut down.
    Quit {
        /// Also end running sessions instead of leaving them to resume.
        end_sessions: bool,
    },
}

/// The user's answer to a permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PermissionAnswer {
    /// Allow this once.
    Allow,
    /// Allow, and add the offered rule.
    AllowAlways,
    /// Deny, optionally telling Claude why.
    Deny {
        /// Reason passed back to Claude.
        message: Option<String>,
    },
}

/// Fields for a new task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewTask {
    /// One-line title.
    pub title: String,
    /// P-level, if given.
    pub p_level: Option<PLevel>,
    /// Bucket; `None` puts it in Unsorted.
    pub bucket: Option<Bucket>,
    /// Notes.
    #[serde(default)]
    pub notes: String,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Linked pull request number.
    pub pr: Option<u32>,
    /// Tasks it is related to (both ways).
    #[serde(default)]
    pub related: Vec<TaskId>,
    /// The effort it belongs to; `None` puts it in Unsorted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortId>,
    /// The criterion of that effort it serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<CriterionId>,
}

/// Fields for a new decision-log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewDecision {
    /// The task it belongs to.
    pub task: TaskId,
    /// The decision, in one line.
    pub text: String,
    /// Why, if given.
    pub why: Option<String>,
    /// Options considered and not taken.
    pub alternatives: Vec<Alternative>,
    /// The earlier entry it replaces.
    pub supersedes: Option<DecisionId>,
}

/// One field change in [`Command::UpdateTask`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TaskChange {
    /// New title.
    Title(String),
    /// New status.
    Status(TaskStatus),
    /// New P-level, or none.
    PLevel(Option<PLevel>),
    /// New bucket, or Unsorted.
    Bucket(Option<Bucket>),
    /// New notes.
    Notes(String),
    /// New dependencies.
    Deps(Vec<TaskId>),
    /// New linked PR, or none.
    Pr(Option<u32>),
    /// The user has seen a coordinator-created task.
    Seen,
    /// Mark a task as not yet seen by the user: the coordinator moved it into Now.
    Unseen,
    /// Relate this task to another, both ways.
    Relate(TaskId),
    /// Remove a relation, both ways.
    Unrelate(TaskId),
}

impl TaskChange {
    /// A few words for the Activity feed; `None` for changes that are not worth a line.
    pub fn describe(&self) -> Option<String> {
        Some(match self {
            Self::Title(t) => format!("renamed to \"{t}\""),
            Self::Status(s) => format!("status {s:?}").to_lowercase(),
            Self::PLevel(Some(p)) => format!("priority {p:?}"),
            Self::PLevel(None) => "priority cleared".to_owned(),
            Self::Bucket(Some(b)) => format!("moved to {b:?}"),
            Self::Bucket(None) => "moved to Unsorted".to_owned(),
            Self::Notes(_) => "notes edited".to_owned(),
            Self::Deps(_) => "dependencies changed".to_owned(),
            Self::Pr(Some(n)) => format!("linked PR #{n}"),
            Self::Pr(None) => "PR unlinked".to_owned(),
            Self::Relate(t) => format!("related to #{}", t.0),
            Self::Unrelate(t) => format!("unrelated from #{}", t.0),
            Self::Seen | Self::Unseen => return None,
        })
    }
}

/// The Activity text for a task patch, `None` when it holds nothing worth a line.
pub fn describe_task_patch(id: TaskId, title: &str, patch: &[TaskChange]) -> Option<String> {
    let parts: Vec<String> = patch.iter().filter_map(TaskChange::describe).collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("task #{} {title}: {}", id.0, parts.join(", ")))
}

/// The UI's end of a connection to a core.
///
/// Both the real core and `MockCore` hand one out. In v1 it is a pair of in-process channels;
/// a daemon would carry the same values as NDJSON over a unix socket.
#[derive(Debug)]
pub struct CoreLink {
    /// Commands to the core.
    pub commands: Sender<Command>,
    /// Messages from the core, starting with [`ServerMsg::Hello`].
    pub messages: Receiver<ServerMsg>,
}
