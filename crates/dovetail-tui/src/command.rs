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
    // `:project new <name> --research`: the subcommand is part of the command's name.
    if name == "project"
        && let Some(rest) = arg.strip_prefix("new")
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        return Ok(Parsed {
            action: Action::NewProject,
            arg: rest.trim().to_string(),
        });
    }
    let action = match name {
        "" => return Err(CommandError::Empty),
        "q" | "quit" => Action::Quit,
        "q!" | "quit!" => Action::ForceQuit,
        "eod" => Action::EndOfDay,
        "fresh" => Action::FreshStart,
        "upgrade" => Action::Upgrade,
        "round" => Action::StartRound,
        "research" => Action::ListResearch,
        "spawn" => Action::NewSlice,
        "ask" => Action::NewSideChat,
        "interrupt" => Action::Interrupt,
        "decide" => Action::TaskAddDecision,
        // Aliases: every action a key reaches can be named here too.
        "focus" => Action::ToggleFocusMode,
        "read" | "reading" => Action::ToggleReadingMode,
        "inbox" => Action::FocusInbox,
        "coordinator" => Action::FocusCoordinator,
        "alternate" => Action::AlternateAgent,
        "activity" => Action::ShowActivity,
        "messages" => Action::ShowMessages,
        "board" => Action::ShowBoard,
        "worktrees" => Action::ShowWorktrees,
        "maps" => Action::ShowMaps,
        "switch" => Action::PickAgent,
        "skills" | "commands" => Action::PickCommand,
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
        assert_eq!(parse("eod").map(|p| p.action), Ok(Action::EndOfDay));
        assert_eq!(parse("fresh").map(|p| p.action), Ok(Action::FreshStart));
    }

    #[test]
    fn upgrade_takes_local_as_its_argument() {
        let plain = parse("upgrade").unwrap();
        assert_eq!((plain.action, plain.arg.as_str()), (Action::Upgrade, ""));
        let local = parse("upgrade --local").unwrap();
        assert_eq!(
            (local.action, local.arg.as_str()),
            (Action::Upgrade, "--local")
        );
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
    fn project_new_takes_the_rest_as_its_argument() {
        let new = parse("project new Market sizing --research").unwrap();
        assert_eq!(
            (new.action, new.arg.as_str()),
            (Action::NewProject, "Market sizing --research")
        );
        assert_eq!(
            parse("project new").map(|p| p.action),
            Ok(Action::NewProject)
        );
        // A project switch named like the subcommand is not one.
        assert_eq!(
            parse("project newsletter").map(|p| p.action),
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
