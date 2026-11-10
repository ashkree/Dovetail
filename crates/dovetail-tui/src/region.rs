//! Regions of the fixed layout, and which buffer each shows (Decision 6a: buffers are never
//! hardcoded to a region, so splits can come later on the same abstraction).

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
}

impl RegionId {
    /// Whether the region is in the side column (inbox over tree, on the right).
    pub fn is_side(self) -> bool {
        matches!(self, RegionId::SideTop | RegionId::SideBottom)
    }
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
        }
    }
}

impl Regions {
    /// The buffer a region shows.
    pub fn buffer(&self, region: RegionId) -> BufferKind {
        match region {
            RegionId::SideTop => self.side_top,
            RegionId::SideBottom => self.side_bottom,
            RegionId::Main => self.main,
        }
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

    /// Makes a region show another buffer.
    pub fn show(&mut self, region: RegionId, kind: BufferKind) {
        match region {
            RegionId::SideTop => self.side_top = kind,
            RegionId::SideBottom => self.side_bottom = kind,
            RegionId::Main => self.main = kind,
        }
    }

    /// The focused region.
    pub fn focus(&self) -> RegionId {
        self.focus
    }

    /// The buffer in the focused region.
    pub fn focused_buffer(&self) -> BufferKind {
        self.buffer(self.focus)
    }

    /// Moves focus to a region.
    pub fn set_focus(&mut self, region: RegionId) {
        if self.focus.is_side() {
            self.last_side = self.focus;
        }
        self.focus = region;
    }

    /// `h`: from the side column to the main region.
    pub fn focus_left(&mut self) {
        if self.focus.is_side() {
            self.set_focus(RegionId::Main);
        }
    }

    /// `l`: from the main region to the side region used last.
    pub fn focus_right(&mut self) {
        if self.focus == RegionId::Main {
            self.focus = self.last_side;
        }
    }

    /// `Ctrl-j`: down within the side column.
    pub fn focus_down(&mut self) {
        if self.focus == RegionId::SideTop {
            self.set_focus(RegionId::SideBottom);
        }
    }

    /// `Ctrl-k`: up within the side column.
    pub fn focus_up(&mut self) {
        if self.focus == RegionId::SideBottom {
            self.set_focus(RegionId::SideTop);
        }
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
