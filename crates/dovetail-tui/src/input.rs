//! A one-line text input with the editing keys Insert mode allows (no vim editing inside it).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a key did to a [`LineInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    /// The text or cursor changed.
    Edited,
    /// `Enter`: the caller takes the text.
    Submit,
    /// `Esc`: the caller drops the input.
    Cancel,
    /// Not a key the input handles.
    Ignored,
}

/// Editable text with a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    text: String,
    /// Byte offset of the cursor; always on a char boundary.
    cursor: usize,
}

impl LineInput {
    /// An input holding `text`, cursor at the end.
    pub fn with_text(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.len(),
        }
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text before the cursor, for placing the terminal cursor.
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor]
    }

    /// Applies a key.
    pub fn handle(&mut self, key: KeyEvent) -> InputOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter => InputOutcome::Submit,
            KeyCode::Esc => InputOutcome::Cancel,
            KeyCode::Char('w') if ctrl => {
                self.delete_word();
                InputOutcome::Edited
            }
            KeyCode::Char('u') if ctrl => {
                self.text.drain(..self.cursor);
                self.cursor = 0;
                InputOutcome::Edited
            }
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                InputOutcome::Edited
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.text.len();
                InputOutcome::Edited
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputOutcome::Edited
            }
            KeyCode::Backspace => {
                if let Some(prev) = self.prev_boundary() {
                    self.text.drain(prev..self.cursor);
                    self.cursor = prev;
                }
                InputOutcome::Edited
            }
            KeyCode::Delete => {
                if let Some(c) = self.text[self.cursor..].chars().next() {
                    self.text.drain(self.cursor..self.cursor + c.len_utf8());
                }
                InputOutcome::Edited
            }
            KeyCode::Left => {
                self.cursor = self.prev_boundary().unwrap_or(0);
                InputOutcome::Edited
            }
            KeyCode::Right => {
                if let Some(c) = self.text[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
                InputOutcome::Edited
            }
            KeyCode::Home => {
                self.cursor = 0;
                InputOutcome::Edited
            }
            KeyCode::End => {
                self.cursor = self.text.len();
                InputOutcome::Edited
            }
            _ => InputOutcome::Ignored,
        }
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    /// Deletes back to the start of the previous word (`Ctrl-w`).
    fn delete_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(char::is_whitespace).map_or(0, |i| {
            i + trimmed[i..].chars().next().map_or(1, char::len_utf8)
        });
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(input: &mut LineInput, s: &str) {
        for c in s.chars() {
            input.handle(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_and_backspace_respect_char_boundaries() {
        let mut input = LineInput::default();
        typed(&mut input, "né");
        assert_eq!(input.text(), "né");
        input.handle(key(KeyCode::Backspace));
        assert_eq!(input.text(), "n");
    }

    #[test]
    fn ctrl_w_deletes_a_word_and_ctrl_u_the_line() {
        let mut input = LineInput::default();
        typed(&mut input, "fix the  build");
        input.handle(ctrl('w'));
        assert_eq!(input.text(), "fix the  ");
        input.handle(ctrl('w'));
        assert_eq!(input.text(), "fix ");
        input.handle(ctrl('u'));
        assert_eq!(input.text(), "");
    }

    #[test]
    fn enter_submits_and_esc_cancels() {
        let mut input = LineInput::with_text("x");
        assert_eq!(input.handle(key(KeyCode::Enter)), InputOutcome::Submit);
        assert_eq!(input.handle(key(KeyCode::Esc)), InputOutcome::Cancel);
    }

    #[test]
    fn the_cursor_moves_and_inserts_in_the_middle() {
        let mut input = LineInput::with_text("ac");
        input.handle(key(KeyCode::Left));
        typed(&mut input, "b");
        assert_eq!(input.text(), "abc");
        assert_eq!(input.before_cursor(), "ab");
    }
}
