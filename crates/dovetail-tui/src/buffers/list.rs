//! Shared ID-keyed list selection and viewport scroll, used by list buffers so live snapshot
//! updates keep the cursor on the same item and clamp cleanly when it disappears.

use crate::action::Action;

use super::{scroll_to, step};

/// Selection and scroll state for an ID-keyed list.
#[derive(Debug, Clone)]
pub struct ListCursor<K> {
    selected: Option<K>,
    /// Where the selection was last, for when its item leaves the list.
    last_index: usize,
    offset: usize,
}

impl<K> Default for ListCursor<K> {
    fn default() -> Self {
        Self {
            selected: None,
            last_index: 0,
            offset: 0,
        }
    }
}

impl<K> ListCursor<K> {
    /// The currently selected key, if one has been recorded.
    pub fn key(&self) -> Option<&K> {
        self.selected.as_ref()
    }

    /// Sets the selected key without changing the fallback index.
    pub fn set_key(&mut self, key: K) {
        self.selected = Some(key);
    }

    /// Sets the selected key and its known index.
    pub fn select_key(&mut self, key: K, index: usize) {
        self.selected = Some(key);
        self.last_index = index;
    }

    /// Clears the selection and resets the fallback index and scroll offset.
    pub fn reset(&mut self) {
        self.selected = None;
        self.last_index = 0;
        self.offset = 0;
    }

    /// First visible row index.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Index of the selected item in `items`, or the nearest index to where it was.
    pub fn resolve<T>(&self, items: &[T], matches: impl Fn(&T, &K) -> bool) -> Option<usize> {
        if items.is_empty() {
            return None;
        }
        if let Some(key) = &self.selected
            && let Some(i) = items.iter().position(|item| matches(item, key))
        {
            return Some(i);
        }
        Some(self.last_index.min(items.len() - 1))
    }

    /// Records the item at `index` as selected (no-op if out of bounds).
    pub fn select_index<T>(&mut self, items: &[T], index: usize, key: impl Fn(&T) -> K) {
        if let Some(item) = items.get(index) {
            self.selected = Some(key(item));
            self.last_index = index;
        }
    }

    /// Resolves and records the selection so subsequent renders keep the same item.
    pub fn sync<T>(
        &mut self,
        items: &[T],
        matches: impl Fn(&T, &K) -> bool,
        key: impl Fn(&T) -> K,
    ) -> Option<usize> {
        let idx = self.resolve(items, matches)?;
        self.select_index(items, idx, key);
        Some(idx)
    }

    /// Adjusts `offset` so `index` is visible in a window of `height` rows, and clamps `offset`
    /// to `len`.
    pub fn scroll_into_view(&mut self, index: usize, len: usize, height: usize) {
        self.offset = scroll_to(self.offset, index, index, height).min(len.saturating_sub(height));
    }

    /// Clamps `offset` to a list of `len` rows in a window of `height` rows.
    pub fn clamp_offset(&mut self, len: usize, height: usize) {
        self.offset = self.offset.min(len.saturating_sub(height));
    }

    /// Steps the selection by `delta`, clamping at both ends.
    pub fn step<T>(
        &mut self,
        items: &[T],
        delta: isize,
        matches: impl Fn(&T, &K) -> bool,
        key: impl Fn(&T) -> K,
    ) -> Option<usize> {
        let cur = self.resolve(items, matches)?;
        let next = step(cur, items.len(), delta);
        self.select_index(items, next, key);
        Some(next)
    }

    /// Moves the selection to `index`, clamping to the end of `items`.
    pub fn goto<T>(&mut self, items: &[T], index: usize, key: impl Fn(&T) -> K) -> Option<usize> {
        if items.is_empty() {
            return None;
        }
        let target = index.min(items.len() - 1);
        self.select_index(items, target, key);
        Some(target)
    }

    /// Handles standard list navigation (`MoveDown`, `MoveUp`, `GoTop`, `GoBottom`).
    pub fn handle_nav<T>(
        &mut self,
        action: Action,
        items: &[T],
        matches: impl Fn(&T, &K) -> bool,
        key: impl Fn(&T) -> K,
    ) -> bool {
        match action {
            Action::MoveDown => {
                self.step(items, 1, matches, key);
                true
            }
            Action::MoveUp => {
                self.step(items, -1, matches, key);
                true
            }
            Action::GoTop => {
                self.goto(items, 0, key);
                true
            }
            Action::GoBottom => {
                self.goto(items, items.len().saturating_sub(1), key);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_selection_by_key_and_falls_back_to_last_index() {
        let mut cursor = ListCursor::<u32>::default();
        let items = [10, 20, 30];
        assert_eq!(cursor.resolve(&items, |x, k| x == k), Some(0));
        cursor.step(&items, 1, |x, k| x == k, |x| *x);
        assert_eq!(cursor.key(), Some(&20));

        // Item 20 moves to index 2: cursor follows it.
        let reordered = [10, 30, 20];
        assert_eq!(cursor.resolve(&reordered, |x, k| x == k), Some(2));

        // Item 20 disappears: cursor falls back to last_index (1), clamped.
        let shrunk = [10, 30];
        assert_eq!(cursor.resolve(&shrunk, |x, k| x == k), Some(1));
    }
}
