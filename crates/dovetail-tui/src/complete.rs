//! Completion in the chat bar: `@` lists paths in an inline popup. (`/` at the start opens the
//! picker instead; see `App::open_command_picker`.) The popup ranks with `nucleo` like the
//! picker; the text is sent as typed and Claude Code reads `@path` itself
//! (docs/spikes/slash-and-at.md).
//!
//! A plain `@foo` matches the session's whole tree (files git does not ignore). `@~/`, `@/`,
//! `@./` and `@../` complete one directory at a time, like a shell. Paths come from the core
//! ([`ConversationCommand::ListPaths`](dovetail_api::ConversationCommand::ListPaths)); until they arrive the popup says
//! so.

use std::collections::HashMap;
use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

/// Most rows the popup shows.
pub const MAX_ROWS: usize = 8;

/// What a token completes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// `@path` against the session's tree (`dir: None`) or one directory.
    Path { dir: Option<String> },
}

/// The word under the cursor that completion works on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    /// Byte range of the whole token, sigil included, up to the cursor.
    range: Range<usize>,
    kind: Kind,
    /// What filters the items: the text after the sigil (and after `dir` for a directory).
    query: String,
}

/// Finds the `@…` token starting the word under `cursor`.
fn token(text: &str, cursor: usize) -> Option<Token> {
    let before = text.get(..cursor)?;
    let at = before.rfind('@')?;
    let word = &before[at + 1..];
    let starts_word = before[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    if !starts_word || word.contains(char::is_whitespace) {
        return None;
    }
    let browsing = ["~/", "/", "./", "../"].iter().any(|p| word.starts_with(p));
    let (dir, query) = match word.rfind('/') {
        Some(slash) if browsing => (Some(word[..=slash].to_string()), &word[slash + 1..]),
        _ => (None, word),
    };
    Some(Token {
        range: at..cursor,
        kind: Kind::Path { dir },
        query: query.to_string(),
    })
}

/// One popup row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What is shown: `/name` or the path.
    pub label: String,
    /// What replaces the token on accepting it.
    insert: String,
    /// A directory: accepting it keeps completing inside it.
    is_dir: bool,
}

/// What accepting a row does to the chat text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accept {
    /// Byte range to replace.
    pub range: Range<usize>,
    /// The replacement; the cursor goes after it.
    pub text: String,
}

/// What a key did to the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyResult {
    /// Not the popup's key, or no popup: the chat input takes it.
    Pass,
    /// Moved or closed; nothing else to do.
    Handled,
    /// Replace part of the text.
    Accept(Accept),
}

/// Most directory listings the `@` popup keeps.
const MAX_LISTINGS: usize = 32;

/// The chat bar's completion state.
pub struct Completion {
    token: Option<Token>,
    rows: Vec<Row>,
    selected: usize,
    /// Esc closed the popup for the token starting here; it opens again on a new token.
    dismissed: Option<usize>,
    /// Listings from the core, by directory (`None`: the tree).
    paths: HashMap<Option<String>, Vec<String>>,
    /// Listings asked for and not yet answered.
    asked: Vec<Option<String>>,
    matcher: Matcher,
}

impl Default for Completion {
    fn default() -> Self {
        Self {
            token: None,
            rows: Vec::new(),
            selected: 0,
            dismissed: None,
            paths: HashMap::new(),
            asked: Vec::new(),
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }
}

impl Completion {
    /// Forgets listings and closes the popup, for a chat bar opened on another agent.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether the popup is showing.
    pub fn is_open(&self) -> bool {
        self.token.is_some() && self.dismissed != self.token.as_ref().map(|t| t.range.start)
    }

    /// The rows showing, and which is selected.
    pub fn rows(&self) -> (&[Row], usize) {
        if self.is_open() {
            (&self.rows, self.selected)
        } else {
            (&[], 0)
        }
    }

    /// A path listing is on its way and the popup has nothing yet.
    pub fn waiting(&self) -> bool {
        self.is_open()
            && self.rows.is_empty()
            && matches!(&self.token, Some(Token { kind: Kind::Path { dir }, .. }) if !self.paths.contains_key(dir))
    }

    /// Recomputes the popup for `text` with the cursor at `cursor`. Returns a directory listing
    /// to ask the core for, if the token needs one not asked for yet.
    pub fn update(&mut self, text: &str, cursor: usize) -> Option<Option<String>> {
        let token = token(text, cursor);
        let moved =
            token.as_ref().map(|t| t.range.start) != self.token.as_ref().map(|t| t.range.start);
        if moved {
            self.dismissed = None;
        }
        let previous = self.rows.get(self.selected).map(|r| r.label.clone());
        self.token = token;
        let Some(token) = self.token.clone() else {
            self.rows.clear();
            return None;
        };
        let mut ask = None;
        self.rows = match &token.kind {
            Kind::Path { dir } => match self.paths.get(dir) {
                Some(paths) => path_rows(dir.as_deref(), paths, &token.query),
                None => {
                    if !self.asked.contains(dir) {
                        self.asked.push(dir.clone());
                        ask = Some(dir.clone());
                    }
                    Vec::new()
                }
            },
        };
        self.rank(&token.query);
        // Typing on keeps the highlight on the same row if it still matches.
        self.selected = previous
            .and_then(|label| self.rows.iter().position(|r| r.label == label))
            .unwrap_or(0);
        ask
    }

    /// Takes a listing from the core. Call [`update`](Self::update) after it to show it.
    pub fn receive(&mut self, dir: Option<String>, paths: Vec<String>) {
        self.asked.retain(|d| *d != dir);
        // A listing is cheap to ask for again; keep only the latest few directories.
        if self.paths.len() >= MAX_LISTINGS {
            self.paths.clear();
        }
        self.paths.insert(dir, paths);
    }

    /// Applies a key while the popup is open: `Up`/`Down` (or `Ctrl-p`/`Ctrl-n`) move, `Tab`
    /// accepts, `Enter` accepts unless the row is already what is typed, `Esc` closes.
    pub fn key(&mut self, key: KeyEvent) -> KeyResult {
        if !self.is_open() {
            return KeyResult::Pass;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
        match key.code {
            KeyCode::Down if !self.rows.is_empty() => self.move_selection(true),
            KeyCode::Char('n') if ctrl && !self.rows.is_empty() => self.move_selection(true),
            KeyCode::Up if !self.rows.is_empty() => self.move_selection(false),
            KeyCode::Char('p') if ctrl => self.move_selection(false),
            KeyCode::Esc => self.dismissed = self.token.as_ref().map(|t| t.range.start),
            KeyCode::Tab | KeyCode::Enter if plain => {
                return self.accept(key.code == KeyCode::Enter);
            }
            _ => return KeyResult::Pass,
        }
        KeyResult::Handled
    }

    /// Moves the selected row one step, stopping at either end.
    fn move_selection(&mut self, down: bool) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 1).min(last)
        } else {
            self.selected.saturating_sub(1)
        };
    }

    fn accept(&mut self, by_enter: bool) -> KeyResult {
        let (Some(token), Some(row)) = (&self.token, self.rows.get(self.selected)) else {
            return KeyResult::Pass;
        };
        let typed = match &token.kind {
            Kind::Path { dir } => format!("@{}{}", dir.as_deref().unwrap_or(""), token.query),
        };
        // Enter on a row that is already typed in full sends the message.
        if by_enter && row.insert.trim_end() == typed {
            return KeyResult::Pass;
        }
        let text = if row.is_dir {
            row.insert.clone()
        } else {
            format!("{} ", row.insert)
        };
        KeyResult::Accept(Accept {
            range: token.range.clone(),
            text,
        })
    }

    /// Ranks the rows by `query`: all in their own order when empty, else by `nucleo` score.
    fn rank(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let hay = Utf32Str::new(&row.label, &mut buf);
                pattern.score(hay, &mut self.matcher).map(|s| (s, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut rows = std::mem::take(&mut self.rows);
        self.rows = scored
            .into_iter()
            .map(|(_, i)| std::mem::replace(&mut rows[i], empty_row()))
            .collect();
    }
}

fn empty_row() -> Row {
    Row {
        label: String::new(),
        insert: String::new(),
        is_dir: false,
    }
}

/// Rows for a listing. In a directory, entries starting with a dot show only once a dot is
/// typed, as in a shell.
fn path_rows(dir: Option<&str>, paths: &[String], query: &str) -> Vec<Row> {
    let hidden = query.starts_with('.');
    paths
        .iter()
        .filter(|p| dir.is_none() || hidden || !p.starts_with('.'))
        .map(|p| {
            let full = format!("{}{p}", dir.unwrap_or(""));
            Row {
                label: if dir.is_some() {
                    p.clone()
                } else {
                    full.clone()
                },
                insert: format!("@{full}"),
                is_dir: p.ends_with('/'),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(c: &Completion) -> Vec<String> {
        c.rows().0.iter().map(|r| r.label.clone()).collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn finds_the_token_under_the_cursor() {
        assert_eq!(token("/comp", 5), None);
        assert_eq!(token("/compact now", 12), None);
        assert_eq!(token("see a/b", 7), None);
        let t = token("look at @crates/tu", 18).unwrap();
        assert_eq!(
            (t.range, t.kind, t.query.as_str()),
            (8..18, Kind::Path { dir: None }, "crates/tu")
        );
        let t = token("@~/notes/ma", 11).unwrap();
        assert_eq!(
            t.kind,
            Kind::Path {
                dir: Some("~/notes/".into())
            }
        );
        assert_eq!(t.query, "ma");
        assert_eq!(token("mail me@host", 12), None);
    }

    #[test]
    fn slash_opens_no_inline_popup() {
        let mut c = Completion::default();
        assert_eq!(c.update("/", 1), None);
        assert!(!c.is_open());
        c.update("/comp", 5);
        assert!(!c.is_open());
    }

    #[test]
    fn at_asks_once_for_the_tree_then_shows_it() {
        let mut c = Completion::default();
        assert_eq!(c.update("@", 1), Some(None));
        assert_eq!(c.update("@a", 2), None);
        assert!(c.waiting());
        c.receive(None, vec!["src/app.rs".into(), "README.md".into()]);
        c.update("@app", 4);
        assert_eq!(labels(&c), vec!["src/app.rs"]);
    }

    #[test]
    fn a_directory_hides_dotfiles_until_a_dot_is_typed_and_keeps_completing() {
        let mut c = Completion::default();
        assert_eq!(c.update("@~/", 3), Some(Some("~/".into())));
        c.receive(Some("~/".into()), vec![".config/".into(), "notes/".into()]);
        c.update("@~/", 3);
        assert_eq!(labels(&c), vec!["notes/"]);
        assert_eq!(
            c.key(key(KeyCode::Tab)),
            KeyResult::Accept(Accept {
                range: 0..3,
                text: "@~/notes/".into()
            })
        );
        c.update("@~/.c", 5);
        assert_eq!(labels(&c), vec![".config/"]);
    }

    #[test]
    fn esc_closes_the_popup_until_a_new_token() {
        let mut c = Completion::default();
        c.update("@a", 2);
        assert_eq!(c.key(key(KeyCode::Esc)), KeyResult::Handled);
        c.update("@ab", 3);
        assert!(!c.is_open());
        c.update("", 0);
        c.update("@", 1);
        assert!(c.is_open());
    }
}
