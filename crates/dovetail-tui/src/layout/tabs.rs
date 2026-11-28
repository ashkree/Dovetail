//! The view-tab row under the header (task #20): which views exist and the leader key of each,
//! with the active one marked.
//!
//! The row is data: [`TABS`] holds each tab's label, the [`Action`] that opens it and a
//! predicate for "this view is on screen". The key shown is looked up in the keymap table
//! ([`BINDINGS`]), so a rebinding shows up here without touching this file. Later views
//! (Project/Planning, GitHub, tasks #24 and #25) are new rows.

use ratatui::style::Modifier;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::keymap::action::Action;
use crate::keymap::mode::Mode;
use crate::keymap::{BINDINGS, Key, Scope};
use crate::layout::region::BufferKind;
use crate::theme::Theme;

/// What the tab predicates look at: the part of the app state that says which view is on screen.
#[derive(Debug, Clone, Copy)]
pub struct ViewState {
    /// The buffer in the main region.
    pub main: BufferKind,
    /// Whether reading mode is on.
    pub reading: bool,
    /// Whether the research list (`:research`, `<leader>R`) is open.
    pub research_list: bool,
}

/// One tab.
pub struct Tab {
    /// Name shown next to the key.
    pub label: &'static str,
    /// What opens the view; its key is read from the keymap table.
    pub action: Action,
    /// Whether the view is on screen.
    pub active: fn(&ViewState) -> bool,
}

/// The tabs, left to right.
pub const TABS: &[Tab] = &[
    Tab {
        label: "session",
        action: Action::FocusCoordinator,
        active: |v| !v.reading && !v.research_list && v.main == BufferKind::Conversation,
    },
    Tab {
        label: "board",
        action: Action::ShowBoard,
        active: |v| !v.reading && v.main == BufferKind::Board,
    },
    Tab {
        label: "worktrees",
        action: Action::ShowWorktrees,
        active: |v| !v.reading && v.main == BufferKind::Worktrees,
    },
    Tab {
        label: "maps",
        action: Action::ShowMaps,
        active: |v| !v.reading && matches!(v.main, BufferKind::MapShelf | BufferKind::MapOutline),
    },
    Tab {
        label: "research",
        action: Action::ListResearch,
        active: |v| !v.reading && v.research_list,
    },
    Tab {
        label: "reading",
        action: Action::ToggleReadingMode,
        active: |v| v.reading,
    },
];

/// The keys that open `action` from Normal mode, as shown (`␣c`); `None` if nothing is bound.
pub fn key_label(action: Action) -> Option<String> {
    let binding = BINDINGS
        .iter()
        .find(|b| b.action == action && b.mode == Mode::Normal && b.scope == Scope::Global)?;
    let keys = Key::parse_sequence(binding.keys).ok()?;
    Some(keys.iter().map(Key::label).collect())
}

/// The row's spans for a row `width` columns wide: keys and labels if they fit, otherwise keys
/// only, cut after the last whole tab that fits. The active tab is accent, bold and underlined
/// (so it shows without colour); the others are dim.
pub fn spans(view: &ViewState, theme: &Theme, width: usize) -> Vec<Span<'static>> {
    let shown: Vec<(String, &Tab)> = TABS
        .iter()
        .filter_map(|tab| Some((key_label(tab.action)?, tab)))
        .collect();
    let build = |with_labels: bool, limit: usize| {
        let mut out = Vec::new();
        let mut used = 0;
        let mut complete = true;
        for (i, (key, tab)) in shown.iter().enumerate() {
            let text = if with_labels {
                format!("{key} {}", tab.label)
            } else {
                key.clone()
            };
            let sep = match (i, with_labels) {
                (0, _) => "",
                (_, true) => " · ",
                (_, false) => " ",
            };
            let w = text.width() + sep.width();
            if used + w > limit {
                complete = false;
                break;
            }
            used += w;
            let style = if (tab.active)(view) {
                theme
                    .accent()
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                theme.dim()
            };
            if !sep.is_empty() {
                out.push(Span::styled(sep, theme.dim()));
            }
            out.push(Span::styled(text, style));
        }
        (out, complete)
    };
    match build(true, width) {
        (full, true) => full,
        _ => build(false, width).0,
    }
}
