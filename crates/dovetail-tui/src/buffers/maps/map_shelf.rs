//! The map shelf (`<leader>M`): every map of the project, newest first. `Enter` opens the
//! selected one in the outline ([`super::map_outline::MapOutlineBuffer`]); its content is fetched with
//! [`dovetail_api::visual::map::MapCommand::Load`] when it opens, not carried in the snapshot.

use dovetail_api::Snapshot;
use dovetail_api::visual::map::{MapId, MapSummary};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::{Buffer, Effect, ListCursor, ViewCx, footer, pane, spread_fit, title};
use crate::fmt;
use crate::keymap::action::Action;
use crate::layout::region::BufferKind;
use crate::render::reading_column;

/// The shelf: the selected map, kept by id so a live redraw never moves the cursor.
#[derive(Default)]
pub struct MapShelfBuffer {
    cursor: ListCursor<MapId>,
}

/// The shelf's maps, most recently drawn first (ties keep the snapshot's order).
fn shelf(snap: &Snapshot) -> Vec<&MapSummary> {
    let mut maps: Vec<&MapSummary> = snap.maps.iter().collect();
    maps.sort_by_key(|m| std::cmp::Reverse(m.updated_ms));
    maps
}

impl MapShelfBuffer {
    fn resolve(&self, maps: &[&MapSummary]) -> Option<usize> {
        self.cursor.resolve(maps, |m, k| &m.id == k)
    }

    /// The selected map, if the shelf has any.
    pub fn selected(&self, snap: &Snapshot) -> Option<MapId> {
        let maps = shelf(snap);
        self.resolve(&maps).map(|i| maps[i].id.clone())
    }
}

impl Buffer for MapShelfBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::MapShelf
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let maps = shelf(cx.snap);
        let heading = format!("maps · {}", maps.len());
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let column = reading_column::column(inner, reading_column::DEFAULT_WIDTH);
        let width = usize::from(column.width);
        if maps.is_empty() {
            let text = "No maps yet. Ask the coordinator to draw one.";
            frame.render_widget(
                Paragraph::new(Span::styled(fmt::truncate(text, width), theme.dim())),
                Rect {
                    height: 1,
                    ..column
                },
            );
            return;
        }
        // One row and a blank between rows, so a long shelf stays easy on the eyes.
        let reserved = if column.height >= 5 { 2 } else { 0 };
        let fit = usize::from(column.height.saturating_sub(reserved))
            .div_ceil(2)
            .max(1);
        let selected = self.resolve(&maps);
        if let Some(i) = selected {
            self.cursor.scroll_into_view(i, maps.len(), fit);
        } else {
            self.cursor.clamp_offset(maps.len(), fit);
        }
        let mut lines: Vec<Line<'static>> = Vec::new();
        for map in maps.iter().skip(self.cursor.offset()).take(fit) {
            let drawn_by = map
                .drawn_by
                .as_ref()
                .and_then(|id| cx.snap.agents.iter().find(|a| &a.id == id))
                .map(|a| a.name.clone());
            let age = fmt::ago(cx.now_ms, map.updated_ms);
            let mut short = vec![map.kind.as_str().to_owned(), format!("v{}", map.version)];
            let mut full = short.clone();
            if let Some(task) = map.task {
                full.push(format!("#{}", task.0));
            }
            if let Some(name) = drawn_by {
                full.push(name);
            }
            full.push(age.clone());
            short.push(age);
            let right = |parts: &[String]| vec![Span::styled(parts.join(" · "), theme.dim())];
            let left = vec![Span::styled(map.title.clone(), theme.body())];
            lines.push(spread_fit(left, &[right(&full), right(&short)], width));
            lines.push(Line::default());
        }
        frame.render_widget(Paragraph::new(lines), column);
        if focused && let Some(i) = selected {
            let y = column.y + 2 * (i - self.cursor.offset()) as u16;
            if y < column.y + column.height {
                frame.buffer_mut().set_style(
                    Rect {
                        y,
                        height: 1,
                        ..inner
                    },
                    theme.selected(),
                );
            }
        }
        footer(
            frame,
            inner,
            &[
                "⏎ open in the outline · / search · Esc back",
                "⏎ open · Esc back",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let maps = shelf(cx.snap);
        if self
            .cursor
            .handle_nav(action, &maps, |m, k| &m.id == k, |m| m.id.clone())
        {
            return true;
        }
        match action {
            Action::MapOpen => match self.resolve(&maps) {
                Some(i) => effects.push(Effect::OpenMap(maps[i].id.clone())),
                None => effects.push(Effect::Note("no map here".into())),
            },
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        shelf(cx.snap).iter().map(|m| m.title.clone()).collect()
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        self.resolve(&shelf(cx.snap)).unwrap_or(0)
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, _effects: &mut Vec<Effect>) {
        let maps = shelf(cx.snap);
        self.cursor.select_index(&maps, row, |m| m.id.clone());
    }
}
