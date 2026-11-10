//! Text input for Insert mode: plain editing with no vim keys. One line for the prompt row,
//! several for chat (`Shift-Enter` where the terminal reports it, or `Alt-Enter` anywhere, starts
//! a new line; `Enter` sends).

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::render::wrap::wrap_ranges;

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
    /// The width last drawn at, so `Up`/`Down` move by display rows; by logical lines if unset.
    wrap_width: Option<usize>,
}

impl TextInput {
    /// A single-line input holding `text`, cursor at the end.
    pub fn line(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.len(),
            multiline: false,
            wrap_width: None,
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

    /// Records the width the input is drawn at, as given to [`rows`](Self::rows), so `Up` and
    /// `Down` move across wrapped rows.
    pub fn set_wrap_width(&mut self, width: usize) {
        self.wrap_width = Some(width);
    }

    /// Byte offset of the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Moves the cursor to byte offset `at`, or to the end if `at` is not a char boundary.
    pub fn set_cursor(&mut self, at: usize) {
        self.cursor = if self.text.is_char_boundary(at) {
            at
        } else {
            self.text.len()
        };
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
            KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => self.newline(),
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
        if let Some(width) = self.wrap_width {
            self.move_by_rows(delta, width);
            return;
        }
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

    /// Moves the cursor one display row up or down at `width`, keeping its screen column.
    fn move_by_rows(&mut self, delta: i32, width: usize) {
        let rows = self.rows(width);
        let (row, column) = self.cursor_position(width);
        let target = if delta < 0 {
            match row.checked_sub(1) {
                Some(t) => t,
                None => return,
            }
        } else if row + 1 < rows.len() {
            row + 1
        } else {
            return;
        };
        let range = rows[target].clone();
        let mut pos = range.end;
        let mut used = 0;
        for (i, c) in self.text[range.clone()].char_indices() {
            let w = c.width().unwrap_or(0);
            if used + w > column {
                pos = range.start + i;
                break;
            }
            used += w;
        }
        // The end of a soft-wrapped row is the start of the next one; stay on this row.
        let soft = rows
            .get(target + 1)
            .is_some_and(|next| next.start == range.end);
        if pos == range.end && soft {
            pos = self.text[range.clone()]
                .char_indices()
                .next_back()
                .map_or(range.start, |(i, _)| range.start + i);
        }
        self.cursor = pos;
    }

    /// The text cut into display rows no wider than `width` (one column is kept free for the
    /// cursor), as byte ranges. Rows break at spaces; only a word longer than the line is split.
    /// Whitespace at a break hangs off the end of its row. Every logical line gives a row.
    pub fn rows(&self, width: usize) -> Vec<Range<usize>> {
        wrap_ranges(&self.text, width.saturating_sub(1).max(1))
    }

    /// The cursor's row and column in the rows [`rows`](Self::rows) gives for `width`. The
    /// column never passes the cursor's spare column, even after spaces hanging at a wrap.
    pub fn cursor_position(&self, width: usize) -> (usize, usize) {
        let rows = self.rows(width);
        let row = rows
            .iter()
            .rposition(|r| r.start <= self.cursor)
            .unwrap_or(0);
        let col = self.text[rows[row].start..self.cursor].width();
        (row, col.min(width.saturating_sub(1)))
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
        input.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
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
    fn shift_and_alt_enter_insert_a_newline_only_in_multiline() {
        let mut chat = TextInput::multiline("a");
        chat.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        chat.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        typed(&mut chat, "b");
        assert_eq!(chat.text(), "a\n\nb");
        let mut row = TextInput::line("a");
        let alt_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
        assert_eq!(row.handle(alt_enter), InputOutcome::Ignored);
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

    fn rows_of(input: &TextInput, width: usize) -> Vec<&str> {
        input
            .rows(width)
            .into_iter()
            .map(|r| &input.text()[r])
            .collect()
    }

    #[test]
    fn rows_break_at_spaces_not_inside_words() {
        let input = TextInput::multiline("fix the build now");
        assert_eq!(rows_of(&input, 9), vec!["fix the ", "build ", "now"]);
    }

    #[test]
    fn the_cursor_at_the_end_of_a_wrapped_line_sits_on_the_next_row() {
        let input = TextInput::multiline("hello world");
        // Limit 7 (one column kept for the cursor): "hello " then "world".
        assert_eq!(input.cursor_position(8), (1, 5));
    }

    #[test]
    fn hanging_spaces_never_push_the_cursor_off_screen() {
        let input = TextInput::multiline("hello      ");
        assert_eq!(rows_of(&input, 6), vec!["hello      "]);
        assert_eq!(input.cursor_position(6), (0, 5));
    }

    #[test]
    fn up_and_down_move_across_wrapped_rows() {
        let mut input = TextInput::multiline("one two three");
        input.set_wrap_width(9);
        // Rows at limit 8: "one two ", "three"; cursor at the end of "three" (column 5).
        input.handle(key(KeyCode::Up));
        assert_eq!(input.cursor_position(9), (0, 5));
        typed(&mut input, "!");
        assert_eq!(input.text(), "one t!wo three");
        input.handle(key(KeyCode::Down));
        assert_eq!(input.before_cursor(), "one t!wo three");
        input.handle(key(KeyCode::Home));
        input.handle(key(KeyCode::Down));
        assert_eq!(input.cursor_position(9), (1, 0));
    }

    #[test]
    fn moving_up_onto_a_shorter_wrapped_row_stays_on_that_row() {
        let mut input = TextInput::multiline("ab cdefgh");
        input.set_wrap_width(7);
        // Rows: "ab ", "cdefgh"; from column 6 the end of "ab " would be the next row's start.
        input.handle(key(KeyCode::Up));
        assert_eq!(input.cursor_position(7), (0, 2));
        typed(&mut input, "x");
        assert_eq!(input.text(), "abx cdefgh");
    }

    #[test]
    fn editing_rewraps_the_rows() {
        let mut input = TextInput::multiline("aaa bbb");
        assert_eq!(rows_of(&input, 8), vec!["aaa bbb"]);
        typed(&mut input, "b");
        assert_eq!(rows_of(&input, 8), vec!["aaa ", "bbbb"]);
        input.handle(key(KeyCode::Backspace));
        assert_eq!(rows_of(&input, 8), vec!["aaa bbb"]);
    }

    #[test]
    fn the_cursor_position_follows_the_wrapped_rows() {
        let mut input = TextInput::multiline("abcdefgh");
        assert_eq!(input.cursor_position(5), (1, 4));
        input.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(input.cursor_position(5), (2, 0));
        let empty = TextInput::multiline("");
        assert_eq!(empty.cursor_position(5), (0, 0));
    }
}
