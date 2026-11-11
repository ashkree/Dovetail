//! Does this terminal show kitty graphics? Decided from the environment, before the UI starts:
//! no query is written to the terminal, so nothing waits on an answer that may never come, and
//! nothing here touches the kitty keyboard protocol.
//!
//! `DOVETAIL_GRAPHICS=kitty` or `=none` overrides the guess. Otherwise kitty itself and the
//! terminals known to speak its graphics protocol (WezTerm, Ghostty) say yes, and tmux or
//! screen say no, because they do not pass the images through. The terminal must also report
//! its size in pixels, or the canvas cannot place anything; see [`super::view::CellPx`].

/// The override variable.
pub const OVERRIDE: &str = "DOVETAIL_GRAPHICS";

/// Whether to use kitty graphics, given a way to read environment variables.
pub fn kitty_graphics(env: impl Fn(&str) -> Option<String>) -> bool {
    match env(OVERRIDE).as_deref().map(str::trim) {
        Some("kitty") => return true,
        Some("none" | "off" | "0") => return false,
        _ => {}
    }
    if env("TMUX").is_some() || env("STY").is_some() {
        return false;
    }
    let term = env("TERM").unwrap_or_default();
    let program = env("TERM_PROGRAM").unwrap_or_default();
    term == "xterm-kitty"
        || env("KITTY_WINDOW_ID").is_some()
        || term == "xterm-ghostty"
        || matches!(program.as_str(), "WezTerm" | "ghostty")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(vars: &[(&str, &str)]) -> bool {
        kitty_graphics(|k| {
            vars.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn kitty_and_friends_say_yes() {
        assert!(with(&[("TERM", "xterm-kitty")]));
        assert!(with(&[
            ("KITTY_WINDOW_ID", "3"),
            ("TERM", "xterm-256color")
        ]));
        assert!(with(&[("TERM_PROGRAM", "WezTerm")]));
        assert!(with(&[("TERM", "xterm-ghostty")]));
    }

    #[test]
    fn other_terminals_and_multiplexers_say_no() {
        assert!(!with(&[]));
        assert!(!with(&[
            ("TERM", "xterm-256color"),
            ("TERM_PROGRAM", "Apple_Terminal")
        ]));
        // tmux started inside kitty keeps kitty's variables.
        assert!(!with(&[
            ("TERM", "tmux-256color"),
            ("KITTY_WINDOW_ID", "3"),
            ("TMUX", "/tmp/t,1,0")
        ]));
    }

    #[test]
    fn the_override_wins_both_ways() {
        assert!(with(&[("DOVETAIL_GRAPHICS", "kitty"), ("TMUX", "x")]));
        assert!(!with(&[
            ("DOVETAIL_GRAPHICS", "none"),
            ("TERM", "xterm-kitty")
        ]));
    }
}
