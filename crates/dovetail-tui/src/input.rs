//! Text input for Insert mode: plain editing with no vim keys. One line for the prompt row,
//! several for chat (`Ctrl-j` or `Alt-Enter` start a new line, `Enter` sends).

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

/// What a key did to a [`TextInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    /// The text or cursor changed.
    Edited,
    /// `Enter`: the caller takes the text.
    Submit,
    /// `Esc`: the caller drops or keeps the draft.
    Cancel,
    /// Not a key the input handles.
    Ignored,
}

/// Editable text with a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    /// Byte offset of the cursor; always on a char boundary.
    cursor: usize,
    multiline: bool,
}

impl TextInput {
    /// A single-line input holding `text`, cursor at the end.
    pub fn line(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.len(),
            multiline: false,
        }
    }

    /// A multiline input holding `text`, cursor at the end.
    pub fn multiline(text: &str) -> Self {
        Self {
            multiline: true,
            ..Self::line(text)
        }
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text before the cursor, for placing the terminal cursor on a single line.
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor]
    }

    /// Inserts pasted text at the cursor. A single-line input takes only the first line.
    pub fn insert_str(&mut self, text: &str) {
        let text = if self.multiline {
            text
        } else {
            text.lines().next().unwrap_or("")
        };
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// Applies a key.
    pub fn handle(&mut self, key: KeyEvent) -> InputOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter if alt => self.newline(),
            KeyCode::Char('j') if ctrl => self.newline(),
            KeyCode::Enter => InputOutcome::Submit,
            KeyCode::Esc => InputOutcome::Cancel,
            KeyCode::Char('w') if ctrl => {
                self.delete_word();
                InputOutcome::Edited
            }
            KeyCode::Char('u') if ctrl => {
                let start = self.line_start();
                self.text.drain(start..self.cursor);
                self.cursor = start;
                InputOutcome::Edited
            }
            KeyCode::Char('a') if ctrl => {
                self.cursor = self.line_start();
                InputOutcome::Edited
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.line_end();
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
                self.cursor = self.line_start();
                InputOutcome::Edited
            }
            KeyCode::End => {
                self.cursor = self.line_end();
                InputOutcome::Edited
            }
            KeyCode::Up if self.multiline => {
                self.move_vertically(-1);
                InputOutcome::Edited
            }
            KeyCode::Down if self.multiline => {
                self.move_vertically(1);
                InputOutcome::Edited
            }
            _ => InputOutcome::Ignored,
        }
    }

    fn newline(&mut self) -> InputOutcome {
        if !self.multiline {
            return InputOutcome::Ignored;
        }
        self.text.insert(self.cursor, '\n');
        self.cursor += 1;
        InputOutcome::Edited
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    /// Start of the line the cursor is on.
    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    /// End of the line the cursor is on.
    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i)
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

    fn move_vertically(&mut self, delta: i32) {
        let column = self.text[self.line_start()..self.cursor].chars().count();
        let target = if delta < 0 {
            let start = self.line_start();
            if start == 0 {
                return;
            }
            // The line above ends just before this line's newline.
            let above_end = start - 1;
            let above_start = self.text[..above_end].rfind('\n').map_or(0, |i| i + 1);
            (above_start, above_end)
        } else {
            let end = self.line_end();
            if end >= self.text.len() {
                return;
            }
            let below_start = end + 1;
            let below_end = self.text[below_start..]
                .find('\n')
                .map_or(self.text.len(), |i| below_start + i);
            (below_start, below_end)
        };
        let line = &self.text[target.0..target.1];
        let offset = line
            .char_indices()
            .nth(column)
            .map_or(line.len(), |(i, _)| i);
        self.cursor = target.0 + offset;
    }

    /// The text cut into display rows no wider than `width` (one column is kept free for the
    /// cursor), as byte ranges. Every logical line gives at least one row.
    pub fn rows(&self, width: usize) -> Vec<Range<usize>> {
        let limit = width.saturating_sub(1).max(1);
        let mut rows = Vec::new();
        let mut start = 0;
        let mut used = 0;
        for (i, c) in self.text.char_indices() {
            if c == '\n' {
                rows.push(start..i);
                start = i + 1;
                used = 0;
                continue;
            }
            let w = c.width().unwrap_or(0);
            if used + w > limit {
                rows.push(start..i);
                start = i;
                used = 0;
            }
            used += w;
        }
        rows.push(start..self.text.len());
        rows
    }

    /// The cursor's row and column in the rows [`rows`](Self::rows) gives for `width`.
    pub fn cursor_position(&self, width: usize) -> (usize, usize) {
        let rows = self.rows(width);
        let row = rows
            .iter()
            .rposition(|r| r.start <= self.cursor)
            .unwrap_or(0);
        let col = self.text[rows[row].start..self.cursor]
            .chars()
            .map(|c| c.width().unwrap_or(0))
            .sum();
        (row, col)
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

    fn typed(input: &mut TextInput, s: &str) {
        for c in s.chars() {
            input.handle(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_and_backspace_respect_char_boundaries() {
        let mut input = TextInput::line("");
        typed(&mut input, "né");
        assert_eq!(input.text(), "né");
        input.handle(key(KeyCode::Backspace));
        assert_eq!(input.text(), "n");
    }

    #[test]
    fn ctrl_w_deletes_a_word_and_ctrl_u_the_line() {
        let mut input = TextInput::line("");
        typed(&mut input, "fix the  build");
        input.handle(ctrl('w'));
        assert_eq!(input.text(), "fix the  ");
        input.handle(ctrl('w'));
        assert_eq!(input.text(), "fix ");
        input.handle(ctrl('u'));
        assert_eq!(input.text(), "");
    }

    #[test]
    fn ctrl_u_clears_only_the_current_line_of_a_multiline_input() {
        let mut input = TextInput::multiline("");
        typed(&mut input, "first");
        input.handle(ctrl('j'));
        typed(&mut input, "second");
        input.handle(ctrl('u'));
        assert_eq!(input.text(), "first\n");
    }

    #[test]
    fn enter_submits_and_esc_cancels() {
        let mut input = TextInput::line("x");
        assert_eq!(input.handle(key(KeyCode::Enter)), InputOutcome::Submit);
        assert_eq!(input.handle(key(KeyCode::Esc)), InputOutcome::Cancel);
    }

    #[test]
    fn ctrl_j_and_alt_enter_insert_a_newline_only_in_multiline() {
        let mut chat = TextInput::multiline("a");
        chat.handle(ctrl('j'));
        chat.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        typed(&mut chat, "b");
        assert_eq!(chat.text(), "a\n\nb");
        let mut row = TextInput::line("a");
        assert_eq!(row.handle(ctrl('j')), InputOutcome::Ignored);
        assert_eq!(row.text(), "a");
    }

    #[test]
    fn the_cursor_moves_and_inserts_in_the_middle() {
        let mut input = TextInput::line("ac");
        input.handle(key(KeyCode::Left));
        typed(&mut input, "b");
        assert_eq!(input.text(), "abc");
        assert_eq!(input.before_cursor(), "ab");
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let mut input = TextInput::multiline("abcd\nef\nghij");
        // Cursor at the end of "ghij": column 4.
        input.handle(key(KeyCode::Up));
        // "ef" is shorter: the cursor lands at its end.
        typed(&mut input, "!");
        assert_eq!(input.text(), "abcd\nef!\nghij");
        input.handle(key(KeyCode::Up));
        typed(&mut input, "?");
        assert_eq!(input.text(), "abc?d\nef!\nghij");
    }

    #[test]
    fn rows_wrap_long_lines_and_keep_blank_ones() {
        let input = TextInput::multiline("abcdefgh\n\nxy");
        let rows: Vec<&str> = input
            .rows(5)
            .into_iter()
            .map(|r| &input.text()[r])
            .collect();
        assert_eq!(rows, vec!["abcd", "efgh", "", "xy"]);
    }

    #[test]
    fn the_cursor_position_follows_the_wrapped_rows() {
        let mut input = TextInput::multiline("abcdefgh");
        assert_eq!(input.cursor_position(5), (1, 4));
        input.handle(ctrl('j'));
        assert_eq!(input.cursor_position(5), (2, 0));
        let empty = TextInput::multiline("");
        assert_eq!(empty.cursor_position(5), (0, 0));
    }
}
