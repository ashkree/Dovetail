//! The multiline chat input bar: text editing, pasted image chips, `@` path completion popup
//! and `/` command-picker handoff.

use crossterm::event::{KeyCode, KeyEvent};
use dovetail_api::agents::conversation::{Attachment, MAX_ATTACHMENTS};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::complete::{self, Completion, KeyResult};
use super::{InputOutcome, TextInput};
use crate::fmt;
use crate::theme::Theme;

/// Most rows the chat input grows to before it scrolls.
pub const MAX_INPUT_ROWS: usize = 6;

/// Result of applying one key event to an open [`ChatInput`].
#[derive(Debug, PartialEq, Eq)]
pub enum ChatKeyOutcome {
    /// Still editing.
    Pending,
    /// `Enter` submitted text and/or images.
    Submit(String, Vec<Attachment>),
    /// `Esc` cancelled editing (caller parks the draft).
    Cancel,
}

/// Active chat bar state for one conversation.
pub struct ChatInput {
    text: TextInput,
    /// Images attached to the message being typed; their markers are in `text`.
    images: Vec<Attachment>,
    /// The `@` path popup over the chat input.
    completion: Completion,
    /// A `/` was just typed as the first character.
    wants_commands: bool,
    /// A path listing the popup needs, for the app to ask the core for.
    wants_paths: Option<Option<String>>,
}

impl ChatInput {
    /// Opens a chat bar prefilled with a saved draft.
    pub fn open(draft: &str, cursor: usize, images: Vec<Attachment>) -> Self {
        let mut text = TextInput::multiline(draft);
        text.set_cursor(cursor.min(draft.len()));
        Self {
            text,
            images,
            completion: Completion::default(),
            wants_commands: false,
            wants_paths: None,
        }
    }

    /// Consumes the chat bar, returning `(draft_text, cursor, draft_images)` to save.
    pub fn into_draft(self) -> (String, usize, Vec<Attachment>) {
        (
            self.text.text().to_string(),
            self.text.cursor(),
            self.images,
        )
    }

    /// How many images the message being typed carries: those whose marker is still in the text.
    pub fn attachment_count(&self) -> usize {
        count_present_markers(self.text.text(), &self.images)
    }

    /// Inserts pasted text at the cursor.
    pub fn paste_text(&mut self, text: &str) {
        self.text.insert_str(text);
    }

    /// Attaches a pasted image at the cursor, or returns a status-bar notice if full.
    pub fn paste_image(&mut self, image: Attachment) -> Option<String> {
        if self.attachment_count() >= MAX_ATTACHMENTS {
            return Some(format!("at most {MAX_ATTACHMENTS} images per message"));
        }
        // Numbers are never reused, so a marker names one image for as long as it lives.
        let tag = format!("{} ", marker(self.images.len(), &image));
        self.images.push(image);
        self.text.insert_str(&tag);
        None
    }

    /// Deletes a whole image marker when the cursor sits right after one (and the space pasted
    /// with it). Returns whether one was deleted.
    pub fn drop_last_attachment(&mut self) -> bool {
        let end = self.text.cursor();
        let before = self.text.before_cursor();
        let bare = before.strip_suffix(' ').unwrap_or(before);
        let found = self.images.iter().enumerate().find_map(|(i, a)| {
            let tag = marker(i, a);
            [before, bare]
                .into_iter()
                .any(|b| b.ends_with(&tag))
                .then_some(tag)
        });
        let Some(tag) = found else { return false };
        let tail = if before.ends_with(&format!("{tag} ")) {
            tag.len() + 1
        } else {
            tag.len()
        };
        self.text.replace(end - tail..end, "");
        true
    }

    /// Applies a key to the chat input. While the `@` popup is open it takes its keys first.
    pub fn key(&mut self, key: KeyEvent) -> ChatKeyOutcome {
        let outcome = match self.completion.key(key) {
            KeyResult::Handled => return ChatKeyOutcome::Pending,
            KeyResult::Accept(accept) => {
                self.text.replace(accept.range, &accept.text);
                InputOutcome::Edited
            }
            KeyResult::Pass => self.text.handle(key),
        };
        if outcome == InputOutcome::Edited {
            let (text, cursor) = (self.text.text().to_string(), self.text.cursor());
            if let Some(dir) = self.completion.update(&text, cursor) {
                self.wants_paths = Some(dir);
            }
            // Only a `/` just typed opens the picker: backspacing to one does not.
            self.wants_commands = text == "/" && cursor == 1 && key.code == KeyCode::Char('/');
        }
        match outcome {
            InputOutcome::Submit => {
                let (text, images) =
                    take_markers(self.text.text(), std::mem::take(&mut self.images));
                ChatKeyOutcome::Submit(text, images)
            }
            InputOutcome::Cancel => ChatKeyOutcome::Cancel,
            InputOutcome::Edited | InputOutcome::Ignored => ChatKeyOutcome::Pending,
        }
    }

    /// Whether the last key typed a `/` into an empty chat bar: the app opens the command picker.
    pub fn take_command_request(&mut self) -> bool {
        std::mem::take(&mut self.wants_commands)
    }

    /// The `/` picker handed over on a space: the bar becomes `/` plus what was typed in the
    /// picker plus the space, cursor at the end, as if it had been typed in the bar.
    pub fn continue_slash(&mut self, typed: &str) {
        let end = self.text.text().len();
        self.text.replace(0..end, &format!("/{typed} "));
    }

    /// Puts a picked command into the chat bar as `/name `. A lone `/` (what opened the picker)
    /// becomes the command; otherwise it goes in at the cursor.
    pub fn insert_command(&mut self, name: &str) {
        let text = format!("/{name} ");
        if self.text.text() == "/" {
            self.text.replace(0..1, &text);
        } else {
            self.text.insert_str(&text);
        }
    }

    /// A path listing the `@` popup is waiting for.
    pub fn take_path_request(&mut self) -> Option<Option<String>> {
        self.wants_paths.take()
    }

    /// Takes a path listing from the core and updates the `@` popup.
    pub fn receive_paths(&mut self, dir: Option<String>, paths: Vec<String>) {
        let (text, cursor) = (self.text.text().to_string(), self.text.cursor());
        self.completion.receive(dir, paths);
        if let Some(dir) = self.completion.update(&text, cursor) {
            self.wants_paths = Some(dir);
        }
    }

    /// Sets the wrap width used for visual-line cursor movement.
    pub fn set_wrap_width(&mut self, width: usize) {
        self.text.set_wrap_width(width);
    }

    /// Wrapped visual row count for `text_width`, clamped to `1..=MAX_INPUT_ROWS`.
    pub fn visual_rows(&self, text_width: usize) -> usize {
        self.text.rows(text_width).len().clamp(1, MAX_INPUT_ROWS)
    }

    /// Draws the `@` popup along the bottom of `body`, just over the chat bar's rule.
    pub fn render_popup(&self, frame: &mut Frame, body: Rect, theme: &Theme) {
        if !self.completion.is_open() {
            return;
        }
        let (rows, selected) = self.completion.rows();
        let width = usize::from(body.width);
        let lines: Vec<Line<'static>> = if rows.is_empty() {
            let note = if self.completion.waiting() {
                "  listing files…"
            } else {
                "  nothing matches · Esc closes"
            };
            vec![Line::from(Span::styled(note, theme.dim()))]
        } else {
            let first = (selected + 1).saturating_sub(complete::MAX_ROWS);
            rows.iter()
                .enumerate()
                .skip(first)
                .take(complete::MAX_ROWS)
                .map(|(i, row)| {
                    let line = Line::from(Span::styled(
                        format!("  {}", fmt::truncate(&row.label, width.saturating_sub(2))),
                        theme.accent(),
                    ));
                    if i == selected {
                        line.style(theme.selected())
                    } else {
                        line
                    }
                })
                .collect()
        };
        let height = (lines.len() as u16).min(body.height);
        let area = Rect {
            y: body.y + body.height - height,
            height,
            ..body
        };
        frame.render_widget(Clear, area);
        for (n, line) in lines.into_iter().take(usize::from(height)).enumerate() {
            let rect = Rect {
                y: area.y + n as u16,
                height: 1,
                ..area
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
    }

    /// Draws the chat input into `area` and places the terminal cursor.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let text_width = usize::from(area.width.saturating_sub(2));
        let rows = self.text.rows(text_width);
        let (cursor_row, cursor_col) = self.text.cursor_position(text_width);
        let first = (cursor_row + 1).saturating_sub(usize::from(area.height));
        let markers: Vec<String> = self
            .images
            .iter()
            .enumerate()
            .map(|(i, a)| marker(i, a))
            .collect();
        for (n, range) in rows
            .iter()
            .skip(first)
            .take(usize::from(area.height))
            .enumerate()
        {
            let prefix = if first + n == 0 { "› " } else { "  " };
            let mut spans = vec![Span::styled(prefix, theme.accent())];
            spans.extend(input_spans(
                &self.text.text()[range.clone()],
                &markers,
                theme,
            ));
            let line = Line::from(spans);
            let rect = Rect {
                y: area.y + n as u16,
                height: 1,
                ..area
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        frame.set_cursor_position(Position::new(
            area.x + 2 + cursor_col as u16,
            area.y + (cursor_row - first) as u16,
        ));
    }
}

/// The text that stands for image number `index` in the chat bar: `[image 1 · 412 KB]`.
pub fn marker(index: usize, image: &Attachment) -> String {
    fmt::image_chip(index + 1, image.size)
}

/// Counts how many attachments still have their marker present in `text`.
pub fn count_present_markers(text: &str, images: &[Attachment]) -> usize {
    images
        .iter()
        .enumerate()
        .filter(|(i, a)| text.contains(&marker(*i, a)))
        .count()
}

/// Splits the typed text from its image markers: the markers (and the space pasted after each)
/// come out of the text, and only the images whose marker is still there are sent, in order.
fn take_markers(text: &str, images: Vec<Attachment>) -> (String, Vec<Attachment>) {
    let mut text = text.to_string();
    let mut kept = Vec::new();
    for (i, image) in images.into_iter().enumerate() {
        let tag = marker(i, &image);
        let Some(at) = text.find(&tag) else { continue };
        let end = at + tag.len();
        let end = if text[end..].starts_with(' ') {
            end + 1
        } else {
            end
        };
        text.replace_range(at..end, "");
        kept.push(image);
    }
    (text.trim().to_string(), kept)
}

/// A row of the input as spans, with image markers in the chip style.
fn input_spans(row: &str, markers: &[String], theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = row;
    while !rest.is_empty() {
        let next = markers
            .iter()
            .filter_map(|m| rest.find(m.as_str()).map(|at| (at, m.len())))
            .min();
        let Some((at, len)) = next else {
            spans.push(Span::styled(rest.to_string(), theme.body()));
            break;
        };
        if at > 0 {
            spans.push(Span::styled(rest[..at].to_string(), theme.body()));
        }
        spans.push(Span::styled(rest[at..at + len].to_string(), theme.chip()));
        rest = &rest[at + len..];
    }
    spans
}
