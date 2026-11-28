//! Discoverability (task #19): what the keymap table offers, shown two ways. The which-key
//! popup lists the keys that can follow a half-typed sequence; `:keys` lists every action with its
//! keys. (The status bar no longer carries a key hint row, task #40.)
//!
//! None of it keeps a copy of the keymap. Keys come from [`Keymap`] and descriptions from the
//! action registry ([`Action::describe`]).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::keymap::action::{Action, Group};
use crate::keymap::command::COMMANDS;
use crate::keymap::mode::Mode;
use crate::keymap::{Key, Keymap, Scope, label_sequence};
use crate::picker::{PickerItem, PickerKey};
use crate::theme::Theme;

// --- the which-key popup ---

/// One line of the which-key popup: a key (or the rest of a short chain) and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhichKeyRow {
    /// The next key as shown (`s`, `⇥`); a chain with only one way on shows all of it (`fr`).
    pub keys: String,
    /// What it does, or `+group` when it opens more keys.
    pub text: String,
    /// The group the row is listed under.
    pub group: Group,
}

impl std::fmt::Display for WhichKeyRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {:<4} {}", self.group.name(), self.keys, self.text)
    }
}

/// The rows of the popup for `pending` keys in `mode` and `scopes`, grouped by purpose (groups in
/// [`Group`] order, rows in table order within one). Empty when nothing can follow.
pub fn which_key(map: &Keymap, mode: Mode, scopes: &[Scope], pending: &[Key]) -> Vec<WhichKeyRow> {
    let mut rows: Vec<WhichKeyRow> = map
        .continuations(mode, scopes, pending)
        .into_iter()
        .map(|next| {
            let key = next.key.label();
            let leaf = next
                .leads_to
                .iter()
                .find(|(rest, _)| rest.is_empty())
                .map(|&(_, action)| action);
            let deeper: Vec<_> = next
                .leads_to
                .iter()
                .filter(|(rest, _)| !rest.is_empty())
                .collect();
            match (leaf, deeper.as_slice()) {
                (Some(action), []) => WhichKeyRow {
                    keys: key,
                    text: action.describe().to_owned(),
                    group: action.group(),
                },
                // Also the start of a longer sequence: the short one runs after a pause.
                (Some(action), _) => WhichKeyRow {
                    keys: key,
                    text: format!("{} …", action.describe()),
                    group: action.group(),
                },
                // A single way on: show the whole chain and its action.
                (None, [(rest, action)]) => WhichKeyRow {
                    keys: format!("{key}{}", label_sequence(rest)),
                    text: action.describe().to_owned(),
                    group: action.group(),
                },
                (None, many) => {
                    let first = many.first().map_or(Group::App, |(_, a)| a.group());
                    let same = many.iter().all(|(_, a)| a.group() == first);
                    WhichKeyRow {
                        keys: format!("{key}…"),
                        text: if same {
                            format!("+{}", first.name().to_lowercase())
                        } else {
                            "+more".to_owned()
                        },
                        group: first,
                    }
                }
            }
        })
        .collect();
    rows.sort_by_key(|r| r.group);
    rows
}

enum Item<'a> {
    Heading(Group),
    Row(&'a WhichKeyRow),
}

/// Draws the popup bottom-right, directly above the status bar at `status`, over whatever is
/// there. `pending` titles it. Rows that do not fit the height flow into more columns; columns
/// that do not fit the width are dropped.
pub fn render_which_key(
    frame: &mut Frame,
    status: Rect,
    pending: &str,
    rows: &[WhichKeyRow],
    theme: &Theme,
) {
    let screen = frame.area();
    // One row stays for the header; two for the border.
    let inner_h = usize::from(status.y.saturating_sub(1).saturating_sub(2));
    if rows.is_empty() || inner_h == 0 {
        return;
    }
    let key_w = rows.iter().map(|r| r.keys.width()).max().unwrap_or(1);
    let mut entries: Vec<Item> = Vec::new();
    let mut last = None;
    for row in rows {
        if last != Some(row.group) {
            entries.push(Item::Heading(row.group));
            last = Some(row.group);
        }
        entries.push(Item::Row(row));
    }
    let mut columns: Vec<Vec<Item>> = vec![Vec::new()];
    for entry in entries {
        let full = columns.last().is_some_and(|c| c.len() >= inner_h);
        if full {
            let mut next = Vec::new();
            // A heading never ends a column.
            if inner_h >= 2
                && let Some(col) = columns.last_mut()
                && matches!(col.last(), Some(Item::Heading(_)))
                && let Some(heading) = col.pop()
            {
                next.push(heading);
            }
            columns.push(next);
        }
        if let Some(col) = columns.last_mut() {
            col.push(entry);
        }
    }
    let row_text = |row: &WhichKeyRow| format!("{:<key_w$}  {}", row.keys, row.text);
    let widths: Vec<usize> = columns
        .iter()
        .map(|col| {
            col.iter()
                .map(|e| match e {
                    Item::Heading(g) => g.name().width(),
                    Item::Row(r) => row_text(r).width(),
                })
                .max()
                .unwrap_or(0)
        })
        .collect();
    // Border and one column of padding on each side; two between columns.
    let budget = usize::from(screen.width).saturating_sub(4);
    let mut shown = 0;
    let mut used = 0;
    for w in &widths {
        let need = used + w + if shown > 0 { 2 } else { 0 };
        if need > budget && shown > 0 {
            break;
        }
        used = need;
        shown += 1;
    }
    let width = (used + 4).min(usize::from(screen.width)) as u16;
    let height = (columns.iter().take(shown).map(Vec::len).max().unwrap_or(0) + 2) as u16;
    let area = Rect {
        x: screen.right().saturating_sub(width),
        y: status.y.saturating_sub(height),
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_style(theme.fg(theme.focus))
        .title(Line::from(Span::styled(
            format!(" {pending} "),
            theme.fg(theme.focus),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut x = inner.x + 1;
    for (col, w) in columns.iter().zip(&widths).take(shown) {
        let lines: Vec<Line> = col
            .iter()
            .map(|e| match e {
                Item::Heading(g) => Line::from(Span::styled(
                    g.name(),
                    theme.dim().add_modifier(Modifier::BOLD),
                )),
                Item::Row(r) => Line::from(vec![
                    Span::styled(format!("{:<key_w$}", r.keys), theme.accent()),
                    Span::raw("  "),
                    Span::styled(r.text.clone(), theme.body()),
                ]),
            })
            .collect();
        let rect = Rect {
            x,
            y: inner.y,
            width: (*w as u16).min(inner.right().saturating_sub(x)),
            height: inner.height,
        };
        frame.render_widget(Paragraph::new(lines), rect);
        x += *w as u16 + 2;
    }
}

// --- :keys ---

/// Whether choosing `action` in `:keys` runs it from here: it has a `:` command, which works
/// anywhere, or a Normal-mode binding in the focused buffer's scope or the global one. Actions
/// that only mean something inside another view, in MAP mode or in a text input do not.
pub fn runnable(map: &Keymap, scopes: &[Scope], action: Action) -> bool {
    if matches!(action, Action::Ignore | Action::QuitOnRepeat) {
        return false;
    }
    COMMANDS.iter().any(|&(_, a)| a == action)
        || map.label_for(action, Mode::Normal, scopes).is_some()
}

/// The keys of `action` that apply here (Normal mode, the focused buffer's scope and the global
/// one), then its `:` commands, as shown in the detail column.
fn keys_here(map: &Keymap, scopes: &[Scope], action: Action) -> Vec<String> {
    let mut out: Vec<String> = map
        .bindings_of(action)
        .into_iter()
        .filter(|(mode, scope, _)| *mode == Mode::Normal && scopes.contains(scope))
        .map(|(_, _, keys)| keys)
        .collect();
    out.extend(commands_of(action).map(|name| format!(":{name}")));
    out
}

fn commands_of(action: Action) -> impl Iterator<Item = &'static str> {
    COMMANDS
        .iter()
        .filter(move |(_, a)| *a == action)
        .map(|&(name, _)| name)
}

/// Where else the action has keys: `Board: H`, `MAP: h`.
fn keys_elsewhere(map: &Keymap, scopes: &[Scope], action: Action) -> Vec<String> {
    map.bindings_of(action)
        .into_iter()
        .filter(|(mode, scope, _)| !(*mode == Mode::Normal && scopes.contains(scope)))
        .map(|(mode, scope, keys)| match (mode, scope) {
            (Mode::Map, _) => format!("MAP: {keys}"),
            (Mode::Insert, _) => format!("typing: {keys}"),
            (_, Scope::Buffer(kind)) => format!("{kind:?}: {keys}"),
            (_, Scope::Confirm) => format!("Confirm: {keys}"),
            (_, Scope::Global) => keys,
        })
        .collect()
}

/// The `:keys` picker's rows: every action with its description, searchable by description, keys,
/// command names and group. Actions that apply here come first in each group.
pub fn keys_items(map: &Keymap, scopes: &[Scope], theme: &Theme) -> Vec<PickerItem> {
    let mut items: Vec<(Group, bool, PickerItem)> = Action::ALL
        .iter()
        .map(|&action| {
            let here = keys_here(map, scopes, action);
            let elsewhere = keys_elsewhere(map, scopes, action);
            let applies = runnable(map, scopes, action);
            let detail = if here.is_empty() {
                elsewhere.first().cloned().unwrap_or_default()
            } else {
                here.join(" ")
            };
            let haystack = format!(
                "{} {} {} {} {action:?}",
                action.describe(),
                here.join(" "),
                elsewhere.join(" "),
                action.group().name(),
            );
            let (glyph, color): (&'static str, Color) = if applies {
                ("▸", theme.focus)
            } else {
                ("·", theme.dim)
            };
            let item = PickerItem {
                key: PickerKey::Action(action),
                label: action.describe().to_owned(),
                haystack,
                detail,
                glyph,
                color,
                depth: 0,
                section: action.group().name(),
            };
            (action.group(), applies, item)
        })
        .collect();
    // Sections must be contiguous for their headings.
    items.sort_by_key(|(group, applies, _)| (*group, !applies));
    items.into_iter().map(|(_, _, item)| item).collect()
}

/// The preview beside a `:keys` row: what the action does and every way to reach it.
pub fn keys_preview(
    map: &Keymap,
    scopes: &[Scope],
    action: Action,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(action.describe().to_owned(), theme.body())),
        Line::from(Span::styled(action.group().name(), theme.dim())),
        Line::default(),
    ];
    let mut section = |title: &str, rows: Vec<String>| {
        if rows.is_empty() {
            return;
        }
        lines.push(Line::from(Span::styled(title.to_owned(), theme.dim())));
        for row in rows {
            lines.push(Line::from(Span::styled(format!("  {row}"), theme.accent())));
        }
        lines.push(Line::default());
    };
    section("here", keys_here(map, scopes, action));
    section(
        "in other views and modes",
        keys_elsewhere(map, scopes, action),
    );
    lines.push(Line::from(Span::styled(
        if runnable(map, scopes, action) {
            "Enter runs it"
        } else {
            "Enter only closes: it works in another view"
        },
        theme.dim(),
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Resolver;
    use crate::layout::region::BufferKind;

    fn rows(scopes: &[Scope], keys: &str) -> Vec<WhichKeyRow> {
        let map = Keymap::default_map();
        let pending = Key::parse_sequence(keys).expect("valid keys");
        which_key(&map, Mode::Normal, scopes, &pending)
    }

    #[test]
    fn leader_lists_leaf_keys_and_groups() {
        let rows = rows(&[Scope::Global], "<leader>");
        let s = rows.iter().find(|r| r.keys == "s…").expect("an s group");
        assert_eq!(s.text, "+agents");
        let board = rows.iter().find(|r| r.keys == "b").expect("board");
        assert_eq!(board.text, Action::ShowBoard.describe());
    }

    #[test]
    fn a_single_chain_shows_whole() {
        // Reading mode's `<leader>fr` is the only way on from `f`.
        let scopes = [Scope::Buffer(BufferKind::Library), Scope::Global];
        let rows = rows(&scopes, "<leader>f");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].keys, "r");
        assert_eq!(rows[0].text, Action::BrowseProjectLibrary.describe());
    }

    #[test]
    fn other_prefixes_list_their_keys() {
        let scopes = [Scope::Buffer(BufferKind::Conversation), Scope::Global];
        let keys: Vec<_> = rows(&scopes, "z").iter().map(|r| r.keys.clone()).collect();
        assert_eq!(keys, ["c", "o", "C", "O"]);
        assert!(rows(&scopes, "q").is_empty());
    }

    #[test]
    fn the_popup_follows_the_resolver() {
        let map = Keymap::default_map();
        let scopes = [Scope::Global];
        let mut r = Resolver::default();
        for key in Key::parse_sequence("<leader>s").expect("valid") {
            r.feed(&map, Mode::Normal, &scopes, key, 0);
        }
        let shown = which_key(&map, Mode::Normal, &scopes, r.pending());
        assert!(shown.iter().any(|row| row.keys == "n"));
        assert!(shown.iter().all(|row| row.group == Group::Agents));
    }

    #[test]
    fn keys_items_cover_every_action() {
        let map = Keymap::default_map();
        let items = keys_items(&map, &[Scope::Global], &Theme::default());
        assert_eq!(items.len(), Action::ALL.len());
        assert!(items.iter().all(|i| !i.label.is_empty()));
    }

    #[test]
    fn every_bound_or_commanded_action_is_in_the_registry() {
        for &(_, action) in COMMANDS {
            #[cfg(debug_assertions)]
            if action == Action::DebugPanic {
                continue;
            }
            assert!(Action::ALL.contains(&action), "{action:?} is not in ALL");
        }
        for binding in crate::keymap::default_bindings() {
            assert!(
                Action::ALL.contains(&binding.action),
                "{:?} is not in ALL",
                binding.action
            );
        }
    }
}
