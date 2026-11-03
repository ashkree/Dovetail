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

/// Parses a command line (without the leading `:`).
pub fn parse(line: &str) -> Result<Parsed, String> {
    let line = line.trim();
    let (name, arg) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(n, a)| (n, a.trim()));
    let action = match name {
        "" => return Err("empty command".into()),
        "q" | "quit" => Action::Quit,
        "q!" | "quit!" => Action::ForceQuit,
        #[cfg(debug_assertions)]
        "panic" => Action::DebugPanic,
        other => return Err(format!("unknown command :{other}")),
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
    fn the_rest_of_the_line_is_the_argument() {
        assert_eq!(
            parse("q now please").map(|p| p.arg),
            Ok("now please".into())
        );
    }
}
