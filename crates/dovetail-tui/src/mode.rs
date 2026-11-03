//! Modes: Normal navigates and acts, Insert types, Command takes `:` commands.

/// The input mode. Visual comes later (v2) and slots in as another variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// Navigate and act. The default; `Esc` returns here.
    #[default]
    Normal,
    /// Type text for an agent, or an answer, reason or title.
    Insert,
    /// Type a `:` command.
    Command,
}

impl Mode {
    /// Label for the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
        }
    }
}
