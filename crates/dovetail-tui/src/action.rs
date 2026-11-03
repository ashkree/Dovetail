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
    /// Jump to the first row.
    GoTop,
    /// Jump to the last row.
    GoBottom,
    /// Focus the region to the left (back to the left region last used).
    FocusLeft,
    /// Focus the region to the right.
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
    /// Give the reading column a solid background, or take it away.
    ToggleSolidBackground,
    // --- tree ---
    /// Open the selected agent's conversation, or the selected task's detail.
    TreeOpen,
    /// Fold or unfold the selected node.
    TreeToggleFold,
    /// Add a task.
    TreeAddTask,
    /// Edit the selected task's title.
    TreeEditTitle,
    /// Cycle the selected task's status: todo, in progress, done.
    TreeCycleStatus,
    /// Cycle the selected task's bucket.
    TreeCycleBucket,
    /// Cycle the selected task's P-level.
    TreeCyclePLevel,
    /// Move the selected task down within Now.
    TreeMoveDown,
    /// Move the selected task up within Now.
    TreeMoveUp,
    /// Archive the selected task.
    TreeArchive,
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
    /// Open the item's agent.
    InboxOpen,
}
