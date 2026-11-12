//! The one action registry: every key and `:` command resolves to an [`Action`].

/// Something the user can do. Plain and `Copy` so the keymap can be a constant table; anything
/// an action needs (the selected row, the focused agent) comes from the UI state when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // --- movement: the same keys in every buffer ---
    /// Move down one row.
    MoveDown,
    /// Move up one row.
    MoveUp,
    /// Scroll down a page. Also works while typing in the chat bar.
    PageDown,
    /// Scroll up a page. Also works while typing in the chat bar.
    PageUp,
    /// Jump to the first row.
    GoTop,
    /// Jump to the last row.
    GoBottom,
    /// Focus the region to the left (the conversation).
    FocusLeft,
    /// Focus the region to the right (back to the side region last used).
    FocusRight,
    /// Focus the region below in the left column.
    FocusDown,
    /// Focus the region above in the left column.
    FocusUp,
    /// Focus the inbox.
    FocusInbox,
    // --- modes and quitting ---
    /// Enter Command mode.
    EnterCommand,
    /// Quit, asking first when a turn is running (plan B23).
    Quit,
    /// Quit without asking.
    ForceQuit,
    /// Ctrl-C: the first press shows "Press Ctrl-C again to exit", a second within
    /// [`crate::app::EXIT_CONFIRM_MS`] runs [`Action::Quit`] (which asks first when a turn is
    /// running). Any other key or the timeout disarms it.
    QuitOnRepeat,
    /// Close the day (`:eod`, ADR 0012): the coordinator writes its handoff and Dovetail the
    /// day's journal file.
    EndOfDay,
    /// A mid-day fresh coordinator with a handoff (`:fresh`, ADR 0012 amendment).
    FreshStart,
    /// Build the next Dovetail in the background (`:upgrade`, ADR 0014); `:upgrade --local` builds
    /// the checkout instead of `origin/main`.
    Upgrade,
    /// Open a research round on a task (`:round <task> <brief>`), or close the open one
    /// (`:round close`) (R1, `docs/research.md` §5).
    StartRound,
    /// List the tasks that have research (`:research`); choosing one opens its detail (R3).
    ListResearch,
    /// Panic on purpose, to check that the terminal is restored. Debug builds only.
    #[cfg(debug_assertions)]
    DebugPanic,
    /// Enter Insert mode to chat with an agent.
    EnterInsert,
    // --- agents ---
    /// Show the coordinator.
    FocusCoordinator,
    /// Switch to the previous agent (vim's alternate buffer).
    AlternateAgent,
    // --- conversation ---
    /// Jump to the next turn.
    NextTurn,
    /// Jump to the previous turn.
    PrevTurn,
    /// Fold the whole turn under the cursor to one line (`zC`; the reader's `zc`).
    FoldTurn,
    /// Unfold the whole turn under the cursor and show its long replies in full (`zO`; the reader's `zo`).
    UnfoldTurn,
    /// Cut the long entry under the cursor to its first lines (`zc`); on a folded turn, nothing.
    FoldEntry,
    /// Show the entry under the cursor in full (`zo`); on a folded turn, unfold the turn.
    UnfoldEntry,
    /// Leave the chat bar for the conversation above it, keeping the draft and its cursor.
    LeaveChatBar,
    /// Ctrl-V in a chat bar: attach the clipboard's image, or paste its text at the cursor.
    PasteClipboard,
    /// Backspace right after an image marker in a chat input: delete the whole marker (and with
    /// it the image). Anywhere else the key edits text as usual.
    DropLastAttachment,
    /// Focus mode: hide the side column and read the conversation alone (`<leader>z`).
    ToggleFocusMode,
    /// Open quick capture.
    QuickCapture,
    /// Start an in-buffer search.
    Search,
    /// Jump to the next search match.
    SearchNext,
    /// Jump to the previous search match.
    SearchPrev,
    /// Open the agent switcher.
    PickAgent,
    /// Open a side chat: ask for its question, or take it from `:ask <question>` (ADR 0011).
    NewSideChat,
    /// Show or hide the side panel (`<leader>?`): side chats in the side column, in place of
    /// the task tree, beside the conversation.
    ToggleSidePanel,
    /// Show the next side chat in the panel (`Tab`).
    NextSideChat,
    /// Show the previous side chat in the panel (`Shift-Tab`).
    PrevSideChat,
    /// Close the side chat shown in the panel (`dd`): its session stops.
    CloseSideChat,
    // --- tangents (ADR 0017) ---
    /// Ask a tangent: a question that does not belong to this project, answered in Tangents
    /// with its sources (`T` in the side panel, `:tangent [question]`).
    NewTangent,
    /// Attach the tangent shown (in the side panel or the tangents list) to a task of this
    /// project: its answers become quick findings there (`a`).
    AttachTangent,
    /// The tangents list (`:tangents`, or the morning's "new tangents" item): triage them.
    ShowTangents,
    /// Read the selected tangent's latest answer in reading mode (`Enter` in the list).
    TangentOpen,
    /// File the selected tangent into a project, picked next (`f`).
    TangentFile,
    /// Start a task from the selected tangent in a project, picked next (`s`).
    TangentStart,
    /// Drop the selected tangent, after a y/n (`d`).
    TangentDrop,
    // --- reading mode (ADR 0018) ---
    /// Reading mode on or off (`<leader>r`): library, reading column and Ask, in place of
    /// Layout B, which comes back as it was.
    ToggleReadingMode,
    /// `Esc` in Normal mode inside reading mode: leave it.
    LeaveReadingMode,
    /// Open the library row under the cursor in the reading column, or fold or unfold it.
    LibraryOpen,
    /// Fold or unfold the library row under the cursor.
    LibraryToggleFold,
    /// Pick a project whose library to browse, read-only, without opening it (`<leader>fr`,
    /// reading mode only; ADR 0018, decided 2026-11-11 later).
    BrowseProjectLibrary,
    /// Select the next citation id in the reading column (`]s`), or in Ask's latest answer.
    NextCitation,
    /// Select the previous citation id (`[s`).
    PrevCitation,
    /// Open the selected citation's finding or source in the reading column (`gd`).
    OpenCitation,
    /// Open the selected citation's web source in the browser (`gx`), through the core.
    BrowseCitation,
    /// Go back to the document read before the last `gd` (`Ctrl-o`).
    ReaderBack,
    /// `i` in the reading column or Ask: type a question about the open report in Ask.
    AskAboutReport,
    /// Close the open report's Ask session (`dd` in Ask): its session stops.
    CloseAsk,
    /// List every key and `:` command with a description (`:keys`); choosing one runs it when it
    /// can run from here.
    ShowKeys,
    /// A key that does nothing in its scope, on purpose, so a global binding on the same key
    /// does not run there (`i` in the library).
    Ignore,
    /// Pick one of the session's slash commands or skills (`:skills`); it goes into the chat
    /// bar as `/name `.
    PickCommand,
    /// Open the picker over tasks, agents and inbox items.
    PickAny,
    /// Show the Activity feed in the main region (or go back if it is showing).
    ShowActivity,
    /// Show the message timeline in the main region (or go back if it is showing).
    ShowMessages,
    /// Show the board in the main region (or go back if it is showing).
    ShowBoard,
    /// Show the worktree view in the main region and refresh its data (or go back if it is
    /// showing).
    ShowWorktrees,
    /// Show the map shelf in the main region (or go back if it is showing).
    ShowMaps,
    /// Return the main region to the conversation.
    BackToConversation,
    /// Open the Activity or message item's agent.
    FeedOpen,
    /// Start a slice: asks for a name, then a brief.
    NewSlice,
    /// Interrupt the focused agent's turn.
    Interrupt,
    /// Cycle the focused session's permission mode: auto, acceptEdits, manual.
    CyclePermissionMode,
    /// Pick the focused session's model and effort (task #39): `<leader>sM`, `:model`, and
    /// `/model` in the chat bar.
    PickModel,
    /// Take the focused session over: native `claude` on it in this terminal, headless again
    /// when it exits (a subagent's goes to its session).
    TakeOver,
    /// Switch project: the project picker (`<leader>pp`, `:project`); opening one stops this
    /// project's sessions and starts the app again on it (B24).
    SwitchProject,
    /// Register the open folder as a project (`:project add`, ADR 0005).
    RegisterProject,
    /// Create a project: `:project new <name> --research` makes a research project.
    NewProject,
    /// Edit the task's notes in `$EDITOR` (arrives with storage in M5).
    TaskEditNotes,
    /// Relate the task to another, or remove the relation, through the picker.
    TaskRelate,
    /// Add an entry to the task's decision log.
    TaskAddDecision,
    /// Open the research row under the cursor (report, round brief, finding, source) in the
    /// task detail's reading view (R3).
    ResearchOpen,
    /// Task detail `Tab`: select the next research row (the one `Enter` opens and `e` edits).
    ResearchNext,
    /// Task detail `Shift-Tab`: select the previous research row.
    ResearchPrev,
    // --- tree ---
    /// Open the selected agent's conversation, or the selected task's detail.
    TreeOpen,
    /// Fold or unfold the selected node.
    TreeToggleFold,
    /// Edit the selected task's title.
    TaskEditTitle,
    /// Cycle the selected task's status: todo, in progress, done.
    TaskCycleStatus,
    /// Cycle the selected task's bucket.
    TaskCycleBucket,
    /// Cycle the selected task's P-level.
    TaskCyclePLevel,
    /// Move the selected task down within Now.
    TreeMoveDown,
    /// Move the selected task up within Now.
    TreeMoveUp,
    /// Archive the selected task.
    TaskArchive,
    // --- board ---
    /// Move the cursor to the column on the left, wrapping from the first to the last.
    BoardColumnLeft,
    /// Move the cursor to the column on the right, wrapping from the last to the first.
    BoardColumnRight,
    /// Move the selected card to the previous status (its column on the left).
    BoardMoveCardLeft,
    /// Move the selected card to the next status (its column on the right).
    BoardMoveCardRight,
    /// Move the selected card up within its column.
    BoardCardUp,
    /// Move the selected card down within its column.
    BoardCardDown,
    /// Open the selected card's task detail.
    BoardOpen,
    // --- worktree view ---
    /// Open the selected worktree's slice conversation.
    WorktreeOpen,
    /// Open the selected worktree in the editor, with the UI suspended.
    WorktreeEdit,
    /// Copy the selected worktree's path to the clipboard.
    WorktreeCopyPath,
    /// Remove the selected worktree by archiving its slice; asks first if work is unsaved.
    WorktreeRemove,
    // --- maps ---
    /// Open the selected shelf map in the outline.
    MapOpen,
    /// Fold or unfold the selected group in the outline.
    MapToggleFold,
    /// Show the map's next flow.
    MapNextFlow,
    /// Select the node of the flow's next step.
    MapNextStep,
    /// Select the node of the flow's previous step.
    MapPrevStep,
    /// Enter MAP mode on the map in the main region (`<leader>v`, decision 23f).
    MapMode,
    /// Leave MAP mode; the map stays open.
    LeaveMapMode,
    /// MAP mode `h`: the nearest node to the left on the canvas.
    MapSelectLeft,
    /// MAP mode `j`: the nearest node below (the next row in the outline alone).
    MapSelectDown,
    /// MAP mode `k`: the nearest node above (the previous row in the outline alone).
    MapSelectUp,
    /// MAP mode `l`: the nearest node to the right.
    MapSelectRight,
    /// MAP mode `Tab`: follow the selected node's first outgoing connection.
    MapFollow,
    /// MAP mode `Shift-Tab`: follow its first incoming connection back.
    MapFollowBack,
    /// MAP mode `Enter`: drill into the selected group.
    MapDrillIn,
    /// MAP mode `Backspace`: back up a level.
    MapDrillOut,
    /// MAP mode `+`: zoom in.
    MapZoomIn,
    /// MAP mode `-`: zoom out.
    MapZoomOut,
    /// MAP mode `0`: fit the whole map.
    MapZoomFit,
    /// MAP mode `Ctrl-h`: pan left.
    MapPanLeft,
    /// MAP mode `Ctrl-j`: pan down.
    MapPanDown,
    /// MAP mode `Ctrl-k`: pan up.
    MapPanUp,
    /// MAP mode `Ctrl-l`: pan right.
    MapPanRight,
    /// MAP mode `v`: show or hide the outline beside the canvas.
    MapToggleOutline,
    /// MAP mode `u`: ask the coordinator to redraw the map from the current code (23h).
    MapRedraw,
    // --- inbox ---
    /// Allow a permission request once.
    InboxAllow,
    /// Allow it and add the offered rule.
    InboxAllowAlways,
    /// Deny it.
    InboxDeny,
    /// Deny it with a reason typed in.
    InboxDenyWithReason,
    /// Reply to a question or a finished turn.
    InboxReply,
    /// Dismiss a finished turn or error.
    InboxDismiss,
    /// Open the item in full (a finished turn: its agent); from the item, open its agent.
    InboxOpen,
}

/// What an action is for: the which-key popup and `:keys` group by it, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Moving the cursor and the focus.
    Move,
    /// Talking to an agent and reading a conversation.
    Chat,
    /// Switching what the main region shows.
    Views,
    /// Starting, switching and steering agents.
    Agents,
    /// The task tree, the board and the task detail.
    Tasks,
    /// Projects.
    Projects,
    /// Reading mode, citations and tangents.
    Reading,
    /// Maps.
    Maps,
    /// Answering the inbox.
    Inbox,
    /// The app itself: quitting, the day, upgrades, help.
    App,
}

impl Group {
    /// The heading shown for the group.
    pub fn name(self) -> &'static str {
        match self {
            Group::Move => "Move",
            Group::Chat => "Chat",
            Group::Views => "Views",
            Group::Agents => "Agents",
            Group::Tasks => "Tasks",
            Group::Projects => "Projects",
            Group::Reading => "Reading",
            Group::Maps => "Maps",
            Group::Inbox => "Inbox",
            Group::App => "App",
        }
    }
}

/// The registry: one line per action with its group and one-line description. [`Action::ALL`] and
/// the exhaustive `match` behind [`Action::describe`] both come from these lines, so an action
/// added to the enum without a line here does not compile.
macro_rules! registry {
    ($($variant:ident: $group:ident, $text:literal;)*) => {
        impl Action {
            /// Every action, in registry order (`DebugPanic` is left out: it only exists in
            /// debug builds and is not for users).
            pub const ALL: &'static [Action] = &[$(Action::$variant),*];

            fn entry(self) -> (Group, &'static str) {
                match self {
                    $(Action::$variant => (Group::$group, $text),)*
                    #[cfg(debug_assertions)]
                    Action::DebugPanic => (Group::App, "panic on purpose (debug builds)"),
                }
            }
        }
    };
}

registry! {
    MoveDown: Move, "move down one row";
    MoveUp: Move, "move up one row";
    PageDown: Move, "scroll down a page";
    PageUp: Move, "scroll up a page";
    GoTop: Move, "jump to the top";
    GoBottom: Move, "jump to the bottom";
    FocusLeft: Move, "focus the conversation (left)";
    FocusRight: Move, "focus the side column (right)";
    FocusDown: Move, "focus the region below";
    FocusUp: Move, "focus the region above";
    FocusInbox: Move, "focus the inbox";
    EnterCommand: App, "type a : command";
    Quit: App, "quit (asks first while a turn runs)";
    ForceQuit: App, "quit without asking";
    QuitOnRepeat: App, "quit: press twice";
    EndOfDay: App, "close the day: handoff and journal";
    FreshStart: App, "fresh coordinator with a handoff";
    Upgrade: App, "build the next Dovetail (--local: this checkout)";
    StartRound: Reading, "open or close a research round on a task";
    ListResearch: Views, "list tasks that have research";
    EnterInsert: Chat, "chat with the agent in view";
    FocusCoordinator: Agents, "show the coordinator";
    AlternateAgent: Agents, "back to the previous agent";
    NextTurn: Chat, "next turn";
    PrevTurn: Chat, "previous turn";
    FoldTurn: Chat, "fold the turn under the cursor";
    UnfoldTurn: Chat, "unfold the turn, long replies in full";
    FoldEntry: Chat, "cut the long entry to its first lines";
    UnfoldEntry: Chat, "show the entry in full";
    LeaveChatBar: Chat, "leave the chat bar, keep the draft";
    PasteClipboard: Chat, "paste the clipboard (image or text)";
    DropLastAttachment: Chat, "delete the image marker before the cursor";
    ToggleFocusMode: Views, "focus mode: hide the side column";
    QuickCapture: Tasks, "capture a task";
    Search: Move, "search in this view";
    SearchNext: Move, "next search match";
    SearchPrev: Move, "previous search match";
    PickAgent: Agents, "switch agent";
    NewSideChat: Agents, "ask a side question";
    ToggleSidePanel: Views, "show or hide the side chats";
    NextSideChat: Chat, "next side chat";
    PrevSideChat: Chat, "previous side chat";
    CloseSideChat: Chat, "close this side chat";
    NewTangent: Reading, "ask a tangent (a question outside this project)";
    AttachTangent: Reading, "attach the tangent to a task";
    ShowTangents: Reading, "list tangents to triage";
    TangentOpen: Reading, "read the tangent's latest answer";
    TangentFile: Reading, "file the tangent into a project";
    TangentStart: Reading, "start a task from the tangent";
    TangentDrop: Reading, "drop the tangent";
    ToggleReadingMode: Views, "reading mode: library, reader and Ask";
    LeaveReadingMode: Reading, "leave reading mode";
    LibraryOpen: Reading, "open the library row";
    LibraryToggleFold: Reading, "fold or unfold the library row";
    BrowseProjectLibrary: Reading, "browse another project's library";
    NextCitation: Reading, "next citation";
    PrevCitation: Reading, "previous citation";
    OpenCitation: Reading, "open the citation's finding or source";
    BrowseCitation: Reading, "open the citation's web page";
    ReaderBack: Reading, "back to the previous document";
    AskAboutReport: Reading, "ask about the open report";
    CloseAsk: Reading, "close this Ask session";
    ShowKeys: App, "every key and command (this list)";
    Ignore: App, "does nothing here, on purpose";
    PickCommand: Chat, "pick a slash command or skill";
    PickAny: Move, "find any task, agent or inbox item";
    ShowActivity: Views, "activity feed";
    ShowMessages: Views, "message timeline";
    ShowBoard: Views, "task board";
    ShowWorktrees: Views, "worktrees";
    ShowMaps: Views, "maps";
    BackToConversation: Views, "back to the conversation";
    FeedOpen: Views, "open the item's agent";
    NewSlice: Agents, "start a slice (name, then brief)";
    Interrupt: Agents, "interrupt the agent's turn";
    CyclePermissionMode: Agents, "cycle permission mode: auto, acceptEdits, manual";
    PickModel: Agents, "pick the session's model and effort";
    TakeOver: Agents, "take the session over in this terminal";
    SwitchProject: Projects, "switch project";
    RegisterProject: Projects, "register this folder as a project";
    NewProject: Projects, "create a project (--research for a research one)";
    TaskEditNotes: Tasks, "edit the task's notes in $EDITOR";
    TaskRelate: Tasks, "relate to another task";
    TaskAddDecision: Tasks, "add to the task's decision log";
    ResearchOpen: Tasks, "read the research row";
    ResearchNext: Tasks, "next research row";
    ResearchPrev: Tasks, "previous research row";
    TreeOpen: Tasks, "open the agent or task";
    TreeToggleFold: Tasks, "fold or unfold";
    TaskEditTitle: Tasks, "edit the task's title";
    TaskCycleStatus: Tasks, "cycle status: todo, in progress, done";
    TaskCycleBucket: Tasks, "cycle bucket";
    TaskCyclePLevel: Tasks, "cycle P-level";
    TreeMoveDown: Tasks, "move the task down within Now";
    TreeMoveUp: Tasks, "move the task up within Now";
    TaskArchive: Tasks, "archive the task";
    BoardColumnLeft: Tasks, "board: column to the left";
    BoardColumnRight: Tasks, "board: column to the right";
    BoardMoveCardLeft: Tasks, "board: move the card to the previous status";
    BoardMoveCardRight: Tasks, "board: move the card to the next status";
    BoardCardUp: Tasks, "board: move the card up";
    BoardCardDown: Tasks, "board: move the card down";
    BoardOpen: Tasks, "board: open the card's task";
    WorktreeOpen: Views, "worktrees: open the slice conversation";
    WorktreeEdit: Views, "worktrees: open in the editor";
    WorktreeCopyPath: Views, "worktrees: copy the path";
    WorktreeRemove: Views, "worktrees: remove (archives the slice)";
    MapOpen: Maps, "open the map";
    MapToggleFold: Maps, "fold or unfold the group";
    MapNextFlow: Maps, "next flow";
    MapNextStep: Maps, "next step of the flow";
    MapPrevStep: Maps, "previous step of the flow";
    MapMode: Maps, "MAP mode: the map full screen";
    LeaveMapMode: Maps, "leave MAP mode";
    MapSelectLeft: Maps, "select the node to the left";
    MapSelectDown: Maps, "select the node below";
    MapSelectUp: Maps, "select the node above";
    MapSelectRight: Maps, "select the node to the right";
    MapFollow: Maps, "follow the first outgoing connection";
    MapFollowBack: Maps, "follow the first incoming connection";
    MapDrillIn: Maps, "drill into the group";
    MapDrillOut: Maps, "back up a level";
    MapZoomIn: Maps, "zoom in";
    MapZoomOut: Maps, "zoom out";
    MapZoomFit: Maps, "fit the whole map";
    MapPanLeft: Maps, "pan left";
    MapPanDown: Maps, "pan down";
    MapPanUp: Maps, "pan up";
    MapPanRight: Maps, "pan right";
    MapToggleOutline: Maps, "show or hide the outline";
    MapRedraw: Maps, "ask for the map to be redrawn";
    InboxAllow: Inbox, "allow once";
    InboxAllowAlways: Inbox, "allow and add the offered rule";
    InboxDeny: Inbox, "deny";
    InboxDenyWithReason: Inbox, "deny with a reason";
    InboxReply: Inbox, "reply";
    InboxDismiss: Inbox, "dismiss";
    InboxOpen: Inbox, "open the item in full";
}

impl Action {
    /// One line saying what the action does.
    pub fn describe(self) -> &'static str {
        self.entry().1
    }

    /// The group the action is listed under.
    pub fn group(self) -> Group {
        self.entry().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_description_and_is_listed_once() {
        let mut seen = std::collections::HashSet::new();
        for action in Action::ALL {
            assert!(
                !action.describe().trim().is_empty(),
                "{action:?} has no description"
            );
            assert!(seen.insert(*action), "{action:?} is listed twice");
        }
    }
}
