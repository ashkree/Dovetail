//! The 2-button confirm modal for switching the coordinator's driver (task #46).
//! Every key is resolved through [`crate::keymap::Keymap`] (`Scope::Confirm`).

use dovetail_api::{
    Command,
    agent::AgentId,
    model::{DriverKind, ModelCommand, ReasoningEffort},
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::action::Action;
use crate::fmt;
use crate::theme::Theme;

/// Which button is selected in [`SwitchConfirm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SwitchChoice {
    /// Seed the new session with a handoff from the current one (default).
    #[default]
    Handoff,
    /// Start a fresh session with no handoff (`:fresh`).
    CleanStart,
}

/// What an action did to [`SwitchConfirm`].
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ConfirmOutcome {
    /// Still waiting on the user.
    Pending,
    /// User cancelled.
    Cancel,
    /// User confirmed; send this command.
    Confirmed(Command),
}

/// Centered 2-button confirm modal for driver switching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SwitchConfirm {
    agent: AgentId,
    driver: DriverKind,
    model: String,
    effort: Option<ReasoningEffort>,
    choice: SwitchChoice,
}

impl SwitchConfirm {
    /// Opens with **Handoff** selected by default.
    pub(super) fn new(
        agent: AgentId,
        driver: DriverKind,
        model: String,
        effort: Option<ReasoningEffort>,
    ) -> Self {
        Self {
            agent,
            driver,
            model,
            effort,
            choice: SwitchChoice::Handoff,
        }
    }

    /// The target driver.
    pub(super) fn driver(&self) -> DriverKind {
        self.driver
    }

    /// Applies an action resolved from the keymap (`Scope::Confirm`).
    pub(super) fn on_action(&mut self, action: Action) -> ConfirmOutcome {
        match action {
            Action::ConfirmNext => {
                self.choice = SwitchChoice::CleanStart;
                ConfirmOutcome::Pending
            }
            Action::ConfirmPrev => {
                self.choice = SwitchChoice::Handoff;
                ConfirmOutcome::Pending
            }
            Action::ConfirmToggle => {
                self.choice = match self.choice {
                    SwitchChoice::Handoff => SwitchChoice::CleanStart,
                    SwitchChoice::CleanStart => SwitchChoice::Handoff,
                };
                ConfirmOutcome::Pending
            }
            Action::ConfirmAccept => {
                ConfirmOutcome::Confirmed(Command::Model(ModelCommand::SwitchDriver {
                    agent: self.agent.clone(),
                    driver: self.driver,
                    model: self.model.clone(),
                    effort: self.effort,
                    handoff: self.choice == SwitchChoice::Handoff,
                }))
            }
            Action::ConfirmCancel => ConfirmOutcome::Cancel,
            _ => ConfirmOutcome::Pending,
        }
    }

    /// Draws the modal centered on `screen`.
    pub(super) fn render(&self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        let width = screen.width.saturating_sub(4).min(64);
        let inner_w = usize::from(width.saturating_sub(4));
        if width < 24 || inner_w < 16 {
            return;
        }
        let message = format!("This starts a new session on {}.", self.driver.label());
        let msg_lines = fmt::wrap(&message, inner_w);
        let summary = match self.effort {
            Some(e) => format!("model: {} · {}", self.model, e.as_str()),
            None => format!("model: {}", self.model),
        };
        let height = (msg_lines.len() as u16 + 7).min(screen.height);
        if height < 7 {
            return;
        }
        let area = Rect {
            x: screen.x + (screen.width - width) / 2,
            y: screen.y + (screen.height - height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_style(theme.fg(theme.focus))
            .title(Line::from(Span::styled(
                " switch driver ",
                theme.fg(theme.focus),
            )));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let handoff_style = if self.choice == SwitchChoice::Handoff {
            theme.selected().fg(theme.bold).add_modifier(Modifier::BOLD)
        } else {
            theme.dim()
        };
        let clean_style = if self.choice == SwitchChoice::CleanStart {
            theme.selected().fg(theme.bold).add_modifier(Modifier::BOLD)
        } else {
            theme.dim()
        };

        let mut lines: Vec<Line<'static>> = msg_lines
            .into_iter()
            .map(|l| Line::from(Span::styled(l, theme.body())))
            .collect();
        lines.push(Line::from(Span::styled(summary, theme.dim())));
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(" Handoff ", handoff_style),
            Span::raw("  "),
            Span::styled(" Clean start ", clean_style),
        ]));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "⏎ confirm · ⇥/h/l choose · esc cancel",
            theme.dim(),
        )));

        let content = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(2),
            ..inner
        };
        frame.render_widget(Paragraph::new(lines), content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Key, Keymap, Scope};
    use crate::mode::Mode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use dovetail_api::model::ModelCommand;

    fn resolve(map: &Keymap, code: KeyCode) -> Option<Action> {
        map.chord(
            Mode::Normal,
            Scope::Confirm,
            Key::from_event(KeyEvent::new(code, KeyModifiers::NONE)),
        )
    }

    #[test]
    fn enter_defaults_to_handoff_and_keymap_toggles_clean_start_and_esc_cancels() {
        let map = Keymap::default_map();
        let make = || {
            SwitchConfirm::new(
                AgentId("coordinator".into()),
                DriverKind::Agy,
                "gemini-3.8-flash".into(),
                Some(ReasoningEffort::High),
            )
        };

        let mut c = make();
        let act = resolve(&map, KeyCode::Enter).expect("enter bound in Confirm");
        assert_eq!(
            c.on_action(act),
            ConfirmOutcome::Confirmed(Command::Model(ModelCommand::SwitchDriver {
                agent: AgentId("coordinator".into()),
                driver: DriverKind::Agy,
                model: "gemini-3.8-flash".into(),
                effort: Some(ReasoningEffort::High),
                handoff: true,
            }))
        );

        let mut c = make();
        c.on_action(resolve(&map, KeyCode::Tab).expect("tab bound"));
        assert_eq!(c.choice, SwitchChoice::CleanStart);
        c.on_action(resolve(&map, KeyCode::Tab).expect("tab bound"));
        assert_eq!(c.choice, SwitchChoice::Handoff);
        c.on_action(resolve(&map, KeyCode::Char('l')).expect("l bound"));
        assert_eq!(c.choice, SwitchChoice::CleanStart);
        c.on_action(resolve(&map, KeyCode::Char('h')).expect("h bound"));
        assert_eq!(c.choice, SwitchChoice::Handoff);
        c.on_action(resolve(&map, KeyCode::Right).expect("right bound"));
        assert_eq!(
            c.on_action(resolve(&map, KeyCode::Enter).expect("enter bound")),
            ConfirmOutcome::Confirmed(Command::Model(ModelCommand::SwitchDriver {
                agent: AgentId("coordinator".into()),
                driver: DriverKind::Agy,
                model: "gemini-3.8-flash".into(),
                effort: Some(ReasoningEffort::High),
                handoff: false,
            }))
        );

        let mut c = make();
        assert_eq!(
            c.on_action(resolve(&map, KeyCode::Esc).expect("esc bound")),
            ConfirmOutcome::Cancel
        );
    }
}
