//! Modes: Normal navigates and acts, Insert types, Command takes `:` commands, Map works the
//! map canvas (decision 23f).

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
    /// The open map fills the screen; `hjkl` move between nodes and the mouse is captured.
    /// Mouse capture is on in this mode only, so the terminal's own text selection works
    /// everywhere else.
    Map,
}

impl Mode {
    /// Label for the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
            Mode::Map => "MAP",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_is_the_default_mode() {
        assert_eq!(Mode::default(), Mode::Normal);
    }

    #[test]
    fn every_mode_has_its_own_status_bar_label() {
        let labels: Vec<_> = [Mode::Normal, Mode::Insert, Mode::Command, Mode::Map]
            .into_iter()
            .map(Mode::label)
            .collect();
        assert_eq!(labels, ["NORMAL", "INSERT", "COMMAND", "MAP"]);
    }
}
