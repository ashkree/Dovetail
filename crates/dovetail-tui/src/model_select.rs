//! The model selector (task #39): a small box listing the models the core offers, with the
//! effort levels of the highlighted one. `j`/`k` pick the model, `h`/`l` the effort, Enter
//! applies and Esc cancels. It applies to one session; the core does the switch and the
//! usage row changes only when the session confirms it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{AgentId, Effort, ModelOption, model_option_index};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::theme::Theme;

/// The effort a move starts from when the session has not reported one.
const START_EFFORT: Effort = Effort::High;

/// What a key did to the selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorOutcome {
    /// Still open.
    Pending,
    /// Closed without a change.
    Cancel,
    /// Switch `agent` to `model` at `effort`.
    Apply {
        /// The session.
        agent: AgentId,
        /// The model, as [`ModelOption::name`].
        model: String,
        /// The effort; `None` when the model has no levels, or none was picked and none is
        /// known.
        effort: Option<Effort>,
    },
}

/// The selector's state.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSelector {
    agent: AgentId,
    name: String,
    options: Vec<ModelOption>,
    /// The highlighted model.
    row: usize,
    /// The model the session runs now, if it is one of the options.
    current: Option<usize>,
    /// The session's effort now.
    current_effort: Option<Effort>,
    /// The effort picked so far; carried from model to model, clamped to what each accepts.
    effort: Option<Effort>,
}

impl ModelSelector {
    /// A selector for session `agent` (shown as `name`) running `model` at `effort`, over
    /// `options`. It opens on the current model, or on `preselect` when that names an option.
    pub fn new(
        agent: AgentId,
        name: impl Into<String>,
        options: Vec<ModelOption>,
        model: Option<&str>,
        effort: Option<Effort>,
        preselect: Option<&str>,
    ) -> Self {
        let current = model.and_then(|m| model_option_index(&options, m));
        let row = preselect
            .and_then(|p| options.iter().position(|o| o.name == p))
            .or(current)
            .unwrap_or(0);
        Self {
            agent,
            name: name.into(),
            options,
            row,
            current,
            current_effort: effort,
            effort,
        }
    }

    /// The highlighted model.
    pub fn selected(&self) -> Option<&ModelOption> {
        self.options.get(self.row)
    }

    /// The effort Enter would apply with the highlighted model: the one picked, or the nearest
    /// level below it that the model accepts (else the lowest it accepts); `None` when the model
    /// has no levels.
    pub fn effort(&self) -> Option<Effort> {
        let option = self.selected()?;
        let wanted = self.effort?;
        if option.supports(wanted) {
            return Some(wanted);
        }
        let levels = levels(option);
        levels
            .iter()
            .rev()
            .find(|e| **e <= wanted)
            .or_else(|| levels.first())
            .copied()
    }

    /// Handles one key.
    pub fn key(&mut self, event: KeyEvent) -> SelectorOutcome {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return SelectorOutcome::Pending;
        }
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.row = (self.row + 1).min(self.options.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => self.row = self.row.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Right => self.step_effort(true),
            KeyCode::Char('h') | KeyCode::Left => self.step_effort(false),
            KeyCode::Enter => {
                let Some(option) = self.selected() else {
                    return SelectorOutcome::Cancel;
                };
                return SelectorOutcome::Apply {
                    agent: self.agent.clone(),
                    model: option.name.clone(),
                    effort: self.effort(),
                };
            }
            KeyCode::Esc | KeyCode::Char('q') => return SelectorOutcome::Cancel,
            _ => {}
        }
        SelectorOutcome::Pending
    }

    /// One level up or down among those the highlighted model accepts.
    fn step_effort(&mut self, up: bool) {
        let Some(option) = self.selected() else {
            return;
        };
        let levels = levels(option);
        if levels.is_empty() {
            return;
        }
        let now = self.effort().unwrap_or(START_EFFORT);
        let at = levels.iter().position(|e| *e == now);
        let next = match (at, up) {
            (Some(i), true) => levels.get(i + 1).or(levels.get(i)).copied(),
            (Some(i), false) => levels.get(i.saturating_sub(1)).copied(),
            // Not a level of this model (no effort known yet): start from the nearest.
            (None, true) => levels.iter().copied().find(|e| *e >= now),
            (None, false) => levels.iter().rev().copied().find(|e| *e <= now),
        }
        .or(levels.first().copied());
        if next.is_some() {
            self.effort = next;
        }
    }

    /// Draws the box at the bottom left of `area` (the conversation), above its chat bar.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let name_width = self
            .options
            .iter()
            .map(|o| o.name.chars().count())
            .max()
            .unwrap_or(0);
        let rows = self.options.len() as u16;
        let width = area.width.saturating_sub(2).min(64);
        // The models, a blank, the effort line, a blank, the key hint, and the border.
        let height = (rows + 6).min(area.height);
        if width < 20 || height < 6 {
            return;
        }
        let place = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(height + 1),
            width,
            height,
        };
        frame.render_widget(Clear, place);
        let block = Block::bordered()
            .border_style(theme.fg(theme.focus))
            .title(Line::from(Span::styled(
                format!(" model · {} ", self.name),
                theme.fg(theme.focus),
            )));
        let inner = block.inner(place);
        frame.render_widget(block, place);
        let mut lines: Vec<Line> = Vec::new();
        for (i, option) in self.options.iter().enumerate() {
            let current = Some(i) == self.current;
            let marker = if current { "● " } else { "  " };
            let name = format!("{:<name_width$}", option.name);
            let style = if i == self.row {
                theme.selected()
            } else if current {
                theme.accent()
            } else {
                theme.body()
            };
            let mut spans = vec![
                Span::styled(marker, theme.accent()),
                Span::styled(name, style),
            ];
            if let Some(resolved) = option.resolved.as_ref().filter(|r| **r != option.name) {
                spans.push(Span::styled(format!("  {resolved}"), theme.dim()));
            }
            if current {
                spans.push(Span::styled("  now", theme.dim()));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::default());
        lines.push(self.effort_line(theme));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "j/k model · h/l effort · ⏎ apply · esc cancel",
            theme.dim(),
        )));
        frame.render_widget(Paragraph::new(lines), theme_pad(inner));
    }

    /// `effort  low  medium [high]  xhigh  max`: the pick in the selection style, the current
    /// level marked, levels the model does not accept grayed.
    fn effort_line(&self, theme: &Theme) -> Line<'static> {
        let mut spans = vec![Span::styled("effort ", theme.body())];
        let Some(option) = self.selected() else {
            return Line::from(spans);
        };
        if levels(option).is_empty() {
            spans.push(Span::styled(
                " this model has no effort levels",
                theme.dim(),
            ));
            return Line::from(spans);
        }
        let picked = self.effort();
        for level in Effort::ALL {
            let mut text = level.as_str().to_owned();
            if Some(level) == self.current_effort && Some(self.row) == self.current {
                text.push('●');
            }
            let style = if Some(level) == picked {
                theme.selected()
            } else if !option.supports(level) {
                theme.fg(theme.line)
            } else {
                theme.body()
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!(" {text} "), style));
        }
        Line::from(spans)
    }
}

/// The levels `option` accepts, lowest first.
fn levels(option: &ModelOption) -> Vec<Effort> {
    Effort::ALL
        .into_iter()
        .filter(|e| option.supports(*e))
        .collect()
}

/// One column of padding either side of the box's text.
fn theme_pad(inner: Rect) -> Rect {
    Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn options() -> Vec<ModelOption> {
        let listed = |name: &str, resolved: &str, efforts: &[Effort]| ModelOption {
            name: name.into(),
            resolved: Some(resolved.into()),
            efforts: Some(efforts.to_vec()),
        };
        vec![
            listed("opus", "claude-opus-5-5", &Effort::ALL),
            listed("sonnet", "claude-sonnet-5-5", &Effort::ALL),
            listed("old", "claude-old", &[]),
            listed("mid", "claude-mid", &[Effort::Low, Effort::Medium]),
        ]
    }

    fn selector(model: &str, effort: Option<Effort>) -> ModelSelector {
        ModelSelector::new(
            AgentId("coordinator".into()),
            "coordinator",
            options(),
            Some(model),
            effort,
            None,
        )
    }

    #[test]
    fn it_opens_on_the_running_model_and_applies_it_unchanged() {
        let mut s = selector("claude-sonnet-5-5", Some(Effort::Medium));
        assert_eq!(s.selected().map(|o| o.name.as_str()), Some("sonnet"));
        assert_eq!(
            s.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            SelectorOutcome::Apply {
                agent: AgentId("coordinator".into()),
                model: "sonnet".into(),
                effort: Some(Effort::Medium),
            }
        );
    }

    #[test]
    fn j_k_move_between_models_and_stop_at_the_ends() {
        let mut s = selector("claude-opus-5-5", Some(Effort::High));
        s.key(key('k'));
        assert_eq!(s.selected().map(|o| o.name.as_str()), Some("opus"));
        for _ in 0..10 {
            s.key(key('j'));
        }
        assert_eq!(s.selected().map(|o| o.name.as_str()), Some("mid"));
    }

    #[test]
    fn h_l_step_through_the_levels_and_stop_at_the_ends() {
        let mut s = selector("claude-opus-5-5", Some(Effort::High));
        s.key(key('l'));
        assert_eq!(s.effort(), Some(Effort::Xhigh));
        s.key(key('l'));
        s.key(key('l'));
        assert_eq!(s.effort(), Some(Effort::Max));
        for _ in 0..6 {
            s.key(key('h'));
        }
        assert_eq!(s.effort(), Some(Effort::Low));
    }

    #[test]
    fn a_model_without_levels_applies_no_effort_and_one_with_fewer_clamps_it() {
        let mut s = selector("claude-opus-5-5", Some(Effort::Max));
        s.key(key('j'));
        s.key(key('j'));
        assert_eq!(s.selected().map(|o| o.name.as_str()), Some("old"));
        assert_eq!(s.effort(), None);
        s.key(key('l'));
        assert_eq!(s.effort(), None, "nothing to step through");
        s.key(key('j'));
        // `mid` lacks max: the nearest level below it.
        assert_eq!(s.effort(), Some(Effort::Medium));
        // Back on a model with every level, the pick is still max.
        s.key(key('k'));
        s.key(key('k'));
        assert_eq!(s.effort(), Some(Effort::Max));
    }

    #[test]
    fn with_no_effort_known_enter_keeps_it_and_a_step_starts_from_high() {
        let mut s = selector("claude-opus-5-5", None);
        assert_eq!(s.effort(), None);
        s.key(key('l'));
        assert_eq!(s.effort(), Some(Effort::Xhigh));
    }

    #[test]
    fn esc_and_q_cancel() {
        let mut s = selector("claude-opus-5-5", None);
        assert_eq!(
            s.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            SelectorOutcome::Cancel
        );
        assert_eq!(s.key(key('q')), SelectorOutcome::Cancel);
    }

    #[test]
    fn a_preselected_name_wins_over_the_running_model() {
        let s = ModelSelector::new(
            AgentId("c".into()),
            "c",
            options(),
            Some("claude-opus-5-5"),
            None,
            Some("sonnet"),
        );
        assert_eq!(s.selected().map(|o| o.name.as_str()), Some("sonnet"));
    }
}
