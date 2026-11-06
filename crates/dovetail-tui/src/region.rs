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
    /// The Activity feed.
    Activity,
    /// The message timeline.
    Messages,
}

/// A slot in the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionId {
    /// Left column, top.
    LeftTop,
    /// Left column, bottom.
    LeftBottom,
    /// The main region, right, full height.
    Main,
}

impl RegionId {
    /// Whether the region is in the left column.
    pub fn is_left(self) -> bool {
        matches!(self, RegionId::LeftTop | RegionId::LeftBottom)
    }
}

/// The regions, which buffer each shows, and which one has focus.
#[derive(Debug, Clone)]
pub struct Regions {
    left_top: BufferKind,
    left_bottom: BufferKind,
    main: BufferKind,
    focus: RegionId,
    /// The left region focus last left from, for `h` to return to.
    last_left: RegionId,
}

impl Default for Regions {
    /// Layout B (Decision 15): tree over inbox on the left, conversation on the right.
    fn default() -> Self {
        Self {
            left_top: BufferKind::Tree,
            left_bottom: BufferKind::Inbox,
            main: BufferKind::Conversation,
            focus: RegionId::LeftTop,
            last_left: RegionId::LeftTop,
        }
    }
}

impl Regions {
    /// The buffer a region shows.
    pub fn buffer(&self, region: RegionId) -> BufferKind {
        match region {
            RegionId::LeftTop => self.left_top,
            RegionId::LeftBottom => self.left_bottom,
            RegionId::Main => self.main,
        }
    }

    /// Makes a region show another buffer.
    pub fn show(&mut self, region: RegionId, kind: BufferKind) {
        match region {
            RegionId::LeftTop => self.left_top = kind,
            RegionId::LeftBottom => self.left_bottom = kind,
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
        if self.focus.is_left() {
            self.last_left = self.focus;
        }
        self.focus = region;
    }

    /// `l`: from the left column to the main region.
    pub fn focus_right(&mut self) {
        if self.focus.is_left() {
            self.set_focus(RegionId::Main);
        }
    }

    /// `h`: from the main region back to the left region used last.
    pub fn focus_left(&mut self) {
        if self.focus == RegionId::Main {
            self.focus = self.last_left;
        }
    }

    /// `Ctrl-j`: down within the left column.
    pub fn focus_down(&mut self) {
        if self.focus == RegionId::LeftTop {
            self.set_focus(RegionId::LeftBottom);
        }
    }

    /// `Ctrl-k`: up within the left column.
    pub fn focus_up(&mut self) {
        if self.focus == RegionId::LeftBottom {
            self.set_focus(RegionId::LeftTop);
        }
    }
}

/// Screen areas of Layout B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    /// Header row: the folder in view, and whether it is a project.
    pub header: Rect,
    /// Task tree.
    pub left_top: Rect,
    /// Inbox.
    pub left_bottom: Rect,
    /// Conversation.
    pub main: Rect,
    /// Status bar row.
    pub status: Rect,
    /// Prompt row, present while an input is open.
    pub prompt: Option<Rect>,
}

/// Splits the screen. The left column is about 38 % wide but never narrower than 40 columns
/// (when the screen allows) so tree rows stay readable; the inbox takes the lower 40 % of it.
pub fn layout(screen: Rect, with_prompt: bool) -> Areas {
    let rows = RatLayout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(u16::from(with_prompt)),
    ])
    .split(screen);
    let left_width = (screen.width * 38 / 100).max(40).min(screen.width / 2);
    let columns =
        RatLayout::horizontal([Constraint::Length(left_width), Constraint::Min(10)]).split(rows[1]);
    let left = RatLayout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(columns[0]);
    Areas {
        header: rows[0],
        left_top: left[0],
        left_bottom: left[1],
        main: columns[1],
        status: rows[2],
        prompt: with_prompt.then_some(rows[3]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h_returns_to_the_left_region_you_came_from() {
        let mut regions = Regions::default();
        regions.focus_down();
        assert_eq!(regions.focus(), RegionId::LeftBottom);
        regions.focus_right();
        assert_eq!(regions.focus(), RegionId::Main);
        regions.focus_left();
        assert_eq!(regions.focus(), RegionId::LeftBottom);
    }

    #[test]
    fn l_and_h_do_nothing_where_there_is_nowhere_to_go() {
        let mut regions = Regions::default();
        regions.focus_left();
        assert_eq!(regions.focus(), RegionId::LeftTop);
        regions.focus_right();
        regions.focus_right();
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
        assert_eq!(a.left_top.height + a.left_bottom.height, 28);
        assert_eq!(a.main.height, 28);
        assert_eq!(a.prompt, None);
        let b = layout(screen, true);
        assert_eq!(b.main.height, 27);
        assert_eq!(b.prompt.map(|p| p.y), Some(29));
    }
}
