//! `:` commands. Each resolves to an [`Action`] plus whatever text followed it.

use crate::action::Action;

/// What the user typed after `:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The action the command names.
    pub action: Action,
    /// The rest of the line, trimmed.
    pub arg: String,
}

/// A command line that names no command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Nothing after `:`.
    Empty,
    /// No command has this name.
    Unknown(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Empty => f.write_str("empty command"),
            CommandError::Unknown(name) => write!(f, "unknown command :{name}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Parses a command line (without the leading `:`).
pub fn parse(line: &str) -> Result<Parsed, CommandError> {
    let line = line.trim();
    let (name, arg) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(n, a)| (n, a.trim()));
    let action = match name {
        "" => return Err(CommandError::Empty),
        "q" | "quit" => Action::Quit,
        "q!" | "quit!" => Action::ForceQuit,
        "spawn" => Action::NewSlice,
        "interrupt" => Action::Interrupt,
        "decide" => Action::TaskAddDecision,
        // Aliases: every action a key reaches can be named here too.
        "solid" => Action::ToggleSolidBackground,
        "inbox" => Action::FocusInbox,
        "coordinator" => Action::FocusCoordinator,
        "alternate" => Action::AlternateAgent,
        "activity" => Action::ShowActivity,
        "messages" => Action::ShowMessages,
        "board" => Action::ShowBoard,
        "worktrees" => Action::ShowWorktrees,
        "switch" => Action::PickAgent,
        "find" => Action::PickAny,
        "add" | "capture" => Action::QuickCapture,
        "mode" | "perm" => Action::CyclePermissionMode,
        "takeover" => Action::TakeOver,
        "project" if arg == "add" => Action::RegisterProject,
        "project" => Action::SwitchProject,
        "register" => Action::RegisterProject,
        #[cfg(debug_assertions)]
        "panic" => Action::DebugPanic,
        other => return Err(CommandError::Unknown(other.to_owned())),
    };
    Ok(Parsed {
        action,
        arg: arg.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_and_its_aliases() {
        assert_eq!(parse("q").map(|p| p.action), Ok(Action::Quit));
        assert_eq!(parse(" quit ").map(|p| p.action), Ok(Action::Quit));
        assert_eq!(parse("q!").map(|p| p.action), Ok(Action::ForceQuit));
    }

    #[test]
    fn unknown_and_empty_commands_are_errors() {
        assert!(parse("frobnicate").is_err());
        assert!(parse("  ").is_err());
    }

    #[test]
    fn project_add_registers_the_open_folder() {
        assert_eq!(
            parse("project add").map(|p| p.action),
            Ok(Action::RegisterProject)
        );
        assert_eq!(
            parse("project").map(|p| p.action),
            Ok(Action::SwitchProject)
        );
    }

    #[test]
    fn the_rest_of_the_line_is_the_argument() {
        assert_eq!(
            parse("q now please").map(|p| p.arg),
            Ok("now please".into())
        );
    }
}
