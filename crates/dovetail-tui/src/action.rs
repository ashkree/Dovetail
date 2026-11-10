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
    /// Give the reading column a solid background, or take it away.
    ToggleSolidBackground,
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
    /// Copy the shown agent's last answer into the coordinator's chat bar (`<leader>y`).
    YankToCoordinator,
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
    /// Switch project (a stub until M5).
    SwitchProject,
    /// Register a project (a stub until M5).
    RegisterProject,
    /// Edit the task's notes in `$EDITOR` (arrives with storage in M5).
    TaskEditNotes,
    /// Relate the task to another, or remove the relation, through the picker.
    TaskRelate,
    /// Add an entry to the task's decision log.
    TaskAddDecision,
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
