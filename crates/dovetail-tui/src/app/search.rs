//! In-buffer search: `/` takes a term, `n` and `N` step through the matching rows of the
//! focused buffer.

use super::App;
use crate::buffers::Effect;

/// Smart case: a term with a capital letter matches exactly, otherwise case is ignored.
fn matches(row: &str, term: &str) -> bool {
    if term.chars().any(char::is_uppercase) {
        row.contains(term)
    } else {
        row.to_lowercase().contains(&term.to_lowercase())
    }
}

/// The row to go to: the next match after `current`, wrapping round; or the previous one.
pub(super) fn next_match(
    rows: &[String],
    term: &str,
    current: usize,
    forward: bool,
) -> Option<usize> {
    let hits: Vec<usize> = (0..rows.len())
        .filter(|&i| matches(&rows[i], term))
        .collect();
    if forward {
        hits.iter()
            .copied()
            .find(|&i| i > current)
            .or(hits.first().copied())
    } else {
        hits.iter()
            .copied()
            .rfind(|&i| i < current)
            .or(hits.last().copied())
    }
}

impl App {
    /// Sets the search term (an empty one clears it) and jumps to the first match.
    pub(super) fn start_search(&mut self, term: String) {
        if term.is_empty() {
            self.search = None;
            self.note = Some("search cleared".into());
            return;
        }
        self.search = Some(term);
        self.search_jump(true);
    }

    /// `n` (forward) and `N` (backward) in the focused buffer.
    pub(super) fn search_jump(&mut self, forward: bool) {
        let Some(term) = self.search.clone() else {
            self.note = Some("no search yet: press / first".into());
            return;
        };
        let kind = self.regions.focused_buffer();
        let cx = cx!(self);
        let buffer = self.buffers.get_mut(kind);
        let rows = buffer.search_rows(&cx);
        let current = buffer.current_row(&cx);
        let Some(row) = next_match(&rows, &term, current, forward) else {
            self.note = Some(format!("no match for {term}"));
            return;
        };
        let position = (0..rows.len())
            .filter(|&i| matches(&rows[i], &term))
            .position(|i| i == row)
            .map_or(1, |p| p + 1);
        let total = rows.iter().filter(|r| matches(r, &term)).count();
        let mut effects: Vec<Effect> = Vec::new();
        buffer.goto_row(row, &cx, &mut effects);
        for effect in effects {
            self.apply_effect(effect);
        }
        self.note = Some(format!("/{term}  {position}/{total}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn smart_case_matching() {
        assert!(matches("Business Home", "business"));
        assert!(!matches("business home", "Business"));
        assert!(matches("Business home", "Business"));
    }

    #[test]
    fn next_goes_forward_and_wraps_previous_goes_back_and_wraps() {
        let r = rows(&["a", "key one", "b", "key two", "c"]);
        assert_eq!(next_match(&r, "key", 0, true), Some(1));
        assert_eq!(next_match(&r, "key", 1, true), Some(3));
        assert_eq!(
            next_match(&r, "key", 3, true),
            Some(1),
            "wraps to the first"
        );
        assert_eq!(next_match(&r, "key", 3, false), Some(1));
        assert_eq!(
            next_match(&r, "key", 1, false),
            Some(3),
            "wraps to the last"
        );
    }

    #[test]
    fn no_match_is_none() {
        assert_eq!(next_match(&rows(&["a", "b"]), "zzz", 0, true), None);
        assert_eq!(next_match(&[], "a", 0, true), None);
    }
}
