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
    /// Fold the turn at the top of the view to one line.
    FoldTurn,
    /// Unfold the turn at the top of the view and show its long replies in full.
    UnfoldTurn,
    /// Leave the chat bar for the conversation above it, keeping the draft and its cursor.
    LeaveChatBar,
    /// Ctrl-V in a chat bar: attach the clipboard's image, or paste its text at the cursor.
    PasteClipboard,
    /// Backspace on an empty chat input: drop the last image chip. With text typed, or no chip,
    /// the key edits text as usual.
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
