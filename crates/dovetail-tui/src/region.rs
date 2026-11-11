//! Regions of the layout, and which buffer each shows (Decision 6a: buffers are never hardcoded
//! to a region, so splits can come later on the same abstraction).
//!
//! Two presets share the abstraction: Layout B (Decision 15, the default) and reading mode
//! (ADR 0018), whose three columns are three more regions. Switching preset keeps each one's
//! buffers and focus, so leaving reading mode puts Layout B back exactly as it was.

use ratatui::layout::{Constraint, Layout as RatLayout, Rect};

/// Kinds of buffer. Each has its own state and its own buffer-local keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BufferKind {
    /// The task tree.
    Tree,
    /// Needs you.
    Inbox,
    /// One agent's conversation.
    Conversation,
    /// One task's detail.
    TaskDetail,
    /// One inbox item in full.
    InboxDetail,
    /// The Activity feed.
    Activity,
    /// The message timeline.
    Messages,
    /// Tasks as a kanban, one column per status.
    Board,
    /// Slice worktrees with their sync, PR and setup state.
    Worktrees,
    /// The shelf of maps, newest first.
    MapShelf,
    /// One map as an outline.
    MapOutline,
    /// The side panel: side chats, one at a time, with a chat bar.
    SidePanel,
    /// Reading mode's library: projects, research, decisions and docs (ADR 0018).
    Library,
    /// Reading mode's reading column: one document, citations, images.
    Reader,
    /// Reading mode's Ask: a read-only session about the open report, with a chat bar.
    Ask,
    /// The tangents list: Tangents' tangents, to triage (ADR 0017).
    Tangents,
}

/// A slot in the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionId {
    /// Side column, top.
    SideTop,
    /// Side column, bottom.
    SideBottom,
    /// The main region, left, full height.
    Main,
    /// Reading mode, left column.
    Library,
    /// Reading mode, middle column.
    Reader,
    /// Reading mode, right column.
    Ask,
}

impl RegionId {
    /// Whether the region is in the side column (inbox over tree, on the right).
    pub fn is_side(self) -> bool {
        matches!(self, RegionId::SideTop | RegionId::SideBottom)
    }

    /// Whether the region belongs to reading mode.
    pub fn is_reading(self) -> bool {
        matches!(self, RegionId::Library | RegionId::Reader | RegionId::Ask)
    }

    /// Reading mode's columns, left to right.
    pub const READING: [RegionId; 3] = [RegionId::Library, RegionId::Reader, RegionId::Ask];
}

/// Which arrangement of regions is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Layout B: conversation left, inbox over tree right (Decision 15).
    #[default]
    LayoutB,
    /// Reading mode: library, reading column, Ask (ADR 0018).
    Reading,
}

/// The regions, which buffer each shows, and which one has focus.
#[derive(Debug, Clone)]
pub struct Regions {
    side_top: BufferKind,
    side_bottom: BufferKind,
    main: BufferKind,
    focus: RegionId,
    /// The side region focus last left from, for `l` to return to.
    last_side: RegionId,
    preset: Preset,
    /// The focused column while reading mode is on; Layout B's focus is kept meanwhile.
    reading_focus: RegionId,
}

impl Default for Regions {
    /// Layout B (Decision 15, placement per ADR 0006 and ADR 0008): conversation on the left,
    /// inbox over tree on the right. Focus starts on the tree.
    fn default() -> Self {
        Self {
            side_top: BufferKind::Inbox,
            side_bottom: BufferKind::Tree,
            main: BufferKind::Conversation,
            focus: RegionId::SideBottom,
            last_side: RegionId::SideBottom,
            preset: Preset::LayoutB,
            reading_focus: RegionId::Library,
        }
    }
}

impl Regions {
    /// The buffer a region shows. Reading mode's columns always show their own buffers.
    pub fn buffer(&self, region: RegionId) -> BufferKind {
        match region {
            RegionId::SideTop => self.side_top,
            RegionId::SideBottom => self.side_bottom,
            RegionId::Main => self.main,
            RegionId::Library => BufferKind::Library,
            RegionId::Reader => BufferKind::Reader,
            RegionId::Ask => BufferKind::Ask,
        }
    }

    /// The preset on screen.
    pub fn preset(&self) -> Preset {
        self.preset
    }

    /// Switches preset. Each keeps its own focus, so going back restores it.
    pub fn set_preset(&mut self, preset: Preset) {
        self.preset = preset;
    }

    /// The side region showing a buffer, if one does.
    pub fn side_showing(&self, kind: BufferKind) -> Option<RegionId> {
        [RegionId::SideTop, RegionId::SideBottom]
            .into_iter()
            .find(|&region| self.buffer(region) == kind)
    }

    /// Where the default layout puts a buffer, if it puts it in a side region.
    pub fn default_side_of(kind: BufferKind) -> Option<RegionId> {
        Regions::default().side_showing(kind)
    }

    /// Makes a region show another buffer. Reading mode's columns are fixed.
    pub fn show(&mut self, region: RegionId, kind: BufferKind) {
        match region {
            RegionId::SideTop => self.side_top = kind,
            RegionId::SideBottom => self.side_bottom = kind,
            RegionId::Main => self.main = kind,
            RegionId::Library | RegionId::Reader | RegionId::Ask => {}
        }
    }

    /// The focused region.
    pub fn focus(&self) -> RegionId {
        match self.preset {
            Preset::LayoutB => self.focus,
            Preset::Reading => self.reading_focus,
        }
    }

    /// The buffer in the focused region.
    pub fn focused_buffer(&self) -> BufferKind {
        self.buffer(self.focus())
    }

    /// Moves focus to a region of the preset on screen; a region of the other preset becomes
    /// that preset's focus for when it is back.
    pub fn set_focus(&mut self, region: RegionId) {
        if region.is_reading() {
            self.reading_focus = region;
            return;
        }
        if self.focus.is_side() {
            self.last_side = self.focus;
        }
        self.focus = region;
    }

    /// `h`: from the side column to the main region; in reading mode, one column left.
    pub fn focus_left(&mut self) {
        if self.preset == Preset::Reading {
            self.reading_focus = match self.reading_focus {
                RegionId::Ask => RegionId::Reader,
                _ => RegionId::Library,
            };
            return;
        }
        if self.focus.is_side() {
            self.set_focus(RegionId::Main);
        }
    }

    /// `l`: from the main region to the side region used last; in reading mode, one column
    /// right.
    pub fn focus_right(&mut self) {
        if self.preset == Preset::Reading {
            self.reading_focus = match self.reading_focus {
                RegionId::Library => RegionId::Reader,
                _ => RegionId::Ask,
            };
            return;
        }
        if self.focus == RegionId::Main {
            self.focus = self.last_side;
        }
    }

    /// `Ctrl-j`: down within the side column.
    pub fn focus_down(&mut self) {
        if self.preset == Preset::LayoutB && self.focus == RegionId::SideTop {
            self.set_focus(RegionId::SideBottom);
        }
    }

    /// `Ctrl-k`: up within the side column.
    pub fn focus_up(&mut self) {
        if self.preset == Preset::LayoutB && self.focus == RegionId::SideBottom {
            self.set_focus(RegionId::SideTop);
        }
    }
}

/// Screen areas of reading mode (ADR 0018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingAreas {
    /// Header row, as in Layout B.
    pub header: Rect,
    /// The library, left.
    pub library: Rect,
    /// The reading column, middle: everything the side columns leave.
    pub reader: Rect,
    /// Ask, right.
    pub ask: Rect,
    /// Status bar row.
    pub status: Rect,
    /// Prompt row, present while an input is open.
    pub prompt: Option<Rect>,
}

impl ReadingAreas {
    /// The area of a reading-mode region.
    pub fn of(&self, region: RegionId) -> Rect {
        match region {
            RegionId::Library => self.library,
            RegionId::Ask => self.ask,
            _ => self.reader,
        }
    }
}

/// Splits the screen for reading mode: the library about 18 % wide (24 to 40 columns), Ask
/// about 24 % (28 to 56), and the reading column the rest, so at 220 columns it holds the full
/// 120-column reading width with its border and padding.
pub fn reading_layout(screen: Rect, with_prompt: bool) -> ReadingAreas {
    let rows = RatLayout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(u16::from(with_prompt)),
    ])
    .split(screen);
    let width = screen.width;
    let library = (width * 18 / 100).clamp(24, 40).min(width / 3);
    let ask = (width * 24 / 100).clamp(28, 56).min(width / 3);
    let columns = RatLayout::horizontal([
        Constraint::Length(library),
        Constraint::Min(10),
        Constraint::Length(ask),
    ])
    .split(rows[1]);
    ReadingAreas {
        header: rows[0],
        library: columns[0],
        reader: columns[1],
        ask: columns[2],
        status: rows[2],
        prompt: with_prompt.then_some(rows[3]),
    }
}

/// Screen areas of Layout B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    /// Header row: the folder in view, and whether it is a project.
    pub header: Rect,
    /// Side column, top (the inbox by default).
    pub side_top: Rect,
    /// Side column, bottom (the task tree by default).
    pub side_bottom: Rect,
    /// Conversation.
    pub main: Rect,
    /// Status bar row.
    pub status: Rect,
    /// Prompt row, present while an input is open.
    pub prompt: Option<Rect>,
}

impl Areas {
    /// Focus mode: the conversation takes the side column's width too, and the side regions
    /// get none.
    pub fn focused(self) -> Areas {
        let side_right = self.side_top.x + self.side_top.width;
        let none = Rect {
            width: 0,
            ..self.side_top
        };
        Areas {
            main: Rect {
                width: side_right.max(self.main.x + self.main.width) - self.main.x,
                ..self.main
            },
            side_top: none,
            side_bottom: none,
            ..self
        }
    }
}

/// Splits the screen. The side column (right) is about 38 % wide but never narrower than 40
/// columns (when the screen allows) so tree rows stay readable; the top region (inbox) takes
/// 40 % of it and the bottom region (tree) the rest (ADR 0008). The conversation takes the rest, on the left.
pub fn layout(screen: Rect, with_prompt: bool) -> Areas {
    let rows = RatLayout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(u16::from(with_prompt)),
    ])
    .split(screen);
    let side_width = (screen.width * 38 / 100).max(40).min(screen.width / 2);
    let columns =
        RatLayout::horizontal([Constraint::Min(10), Constraint::Length(side_width)]).split(rows[1]);
    let left = RatLayout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(columns[1]);
    Areas {
        header: rows[0],
        side_top: left[0],
        side_bottom: left[1],
        main: columns[0],
        status: rows[2],
        prompt: with_prompt.then_some(rows[3]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_returns_to_the_side_region_you_came_from() {
        let mut regions = Regions::default();
        regions.focus_up();
        assert_eq!(regions.focus(), RegionId::SideTop);
        regions.focus_left();
        assert_eq!(regions.focus(), RegionId::Main);
        regions.focus_right();
        assert_eq!(regions.focus(), RegionId::SideTop);
    }

    #[test]
    fn l_and_h_do_nothing_where_there_is_nowhere_to_go() {
        let mut regions = Regions::default();
        regions.focus_right();
        assert_eq!(regions.focus(), RegionId::SideBottom);
        regions.focus_left();
        regions.focus_left();
        assert_eq!(regions.focus(), RegionId::Main);
        regions.focus_down();
        assert_eq!(regions.focus(), RegionId::Main);
    }

    #[test]
    fn reading_mode_keeps_layout_b_focus_and_moves_between_its_columns() {
        let mut regions = Regions::default();
        regions.focus_up();
        regions.set_preset(Preset::Reading);
        assert_eq!(regions.focus(), RegionId::Library);
        regions.focus_right();
        assert_eq!(regions.focused_buffer(), BufferKind::Reader);
        regions.focus_right();
        regions.focus_right();
        assert_eq!(regions.focus(), RegionId::Ask);
        regions.focus_up();
        assert_eq!(regions.focus(), RegionId::Ask);
        regions.focus_left();
        assert_eq!(regions.focus(), RegionId::Reader);
        regions.set_preset(Preset::LayoutB);
        assert_eq!(regions.focus(), RegionId::SideTop);
        assert_eq!(regions.buffer(RegionId::SideBottom), BufferKind::Tree);
    }

    #[test]
    fn the_reading_column_gets_the_full_width_at_220() {
        let a = reading_layout(Rect::new(0, 0, 220, 40), false);
        assert_eq!(a.library.width + a.reader.width + a.ask.width, 220);
        // Border and padding take 4 columns.
        assert!(a.reader.width >= 124, "{}", a.reader.width);
        let narrow = reading_layout(Rect::new(0, 0, 100, 30), true);
        assert_eq!((narrow.library.width, narrow.ask.width), (24, 28));
        assert_eq!(narrow.prompt.map(|p| p.y), Some(29));
    }

    #[test]
    fn a_region_can_show_any_buffer() {
        let mut regions = Regions::default();
        regions.show(RegionId::Main, BufferKind::Inbox);
        assert_eq!(regions.buffer(RegionId::Main), BufferKind::Inbox);
    }

    #[test]
    fn layout_fills_the_screen_and_reserves_the_header_and_prompt_rows() {
        let screen = Rect::new(0, 0, 100, 30);
        let a = layout(screen, false);
        assert_eq!(a.header.y, 0);
        assert_eq!(a.main.y, 1);
        assert_eq!(a.side_top.height + a.side_bottom.height, 28);
        assert_eq!(a.main.height, 28);
        assert_eq!(a.prompt, None);
        let b = layout(screen, true);
        assert_eq!(b.main.height, 27);
        assert_eq!(b.prompt.map(|p| p.y), Some(29));
    }
}
