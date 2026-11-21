//! Dovetail's own slash commands in the chat bar. They are answered here and never reach Claude
//! Code (task #34; `/model` is task #39). One table feeds both the command picker and
//! [`intercept`].

use crate::action::Action;

/// A slash command Dovetail answers itself: name, description and argument hint (empty for
/// none). The description starts "Dovetail: " so the popup shows who answers.
pub const DOVETAIL_SLASH: &[(&str, &str, &str)] = &[
    ("model", "Dovetail: pick model and effort", "[name]"),
    (
        "handoff",
        "Dovetail: fresh coordinator seeded with a handoff",
        "",
    ),
    ("fresh", "Dovetail: fresh coordinator, no handoff", ""),
];

/// Whether Dovetail answers `/name` itself.
pub fn is_dovetail(name: &str) -> bool {
    DOVETAIL_SLASH.iter().any(|(n, ..)| *n == name)
}

/// What the chat bar does with a line instead of sending it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intercept<'a> {
    /// `/model [arg]`: open the selector; `arg` is empty for none.
    Model(&'a str),
    /// An exact `/handoff` or `/fresh`: run the action.
    Run(Action),
    /// `/handoff …` or `/fresh …`: refused, the text stays in the bar. Holds the command name.
    Refused(&'static str),
}

/// What `text` means to Dovetail; `None` sends it to Claude Code as it is.
pub fn intercept(text: &str) -> Option<Intercept<'_>> {
    let text = text.trim();
    let (word, rest) = match text.split_once(char::is_whitespace) {
        Some((w, r)) => (w, r.trim()),
        None => (text, ""),
    };
    match word {
        "/model" => Some(Intercept::Model(rest)),
        "/handoff" if rest.is_empty() => Some(Intercept::Run(Action::Handoff)),
        "/fresh" if rest.is_empty() => Some(Intercept::Run(Action::FreshSession)),
        "/handoff" => Some(Intercept::Refused("/handoff")),
        "/fresh" => Some(Intercept::Refused("/fresh")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_command_runs_its_action() {
        assert_eq!(intercept("/handoff"), Some(Intercept::Run(Action::Handoff)));
        assert_eq!(
            intercept("/fresh"),
            Some(Intercept::Run(Action::FreshSession))
        );
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert_eq!(
            intercept("  /fresh \n"),
            Some(Intercept::Run(Action::FreshSession))
        );
    }

    #[test]
    fn trailing_text_is_refused() {
        assert_eq!(
            intercept("/handoff now please"),
            Some(Intercept::Refused("/handoff"))
        );
        assert_eq!(intercept("/fresh x"), Some(Intercept::Refused("/fresh")));
    }

    #[test]
    fn model_keeps_its_argument() {
        assert_eq!(intercept("/model"), Some(Intercept::Model("")));
        assert_eq!(intercept("/model  opus "), Some(Intercept::Model("opus")));
    }

    #[test]
    fn a_longer_word_is_not_a_match() {
        assert_eq!(intercept("/handoffs"), None);
        assert_eq!(intercept("/freshen"), None);
        assert_eq!(intercept("/models"), None);
    }

    #[test]
    fn other_text_goes_to_claude_code() {
        assert_eq!(intercept("hello"), None);
        assert_eq!(intercept("please /fresh"), None);
        assert_eq!(intercept("/compact"), None);
    }

    #[test]
    fn every_table_row_is_intercepted() {
        for (name, ..) in DOVETAIL_SLASH {
            assert!(intercept(&format!("/{name}")).is_some(), "{name}");
        }
    }
}
