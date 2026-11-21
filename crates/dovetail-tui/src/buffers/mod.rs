//! Buffers: every view is one. A region shows a buffer; the buffer owns its state, draws itself
//! and handles its buffer-local actions.

mod ask;
mod board;
mod conversation;
mod feed;
mod inbox;
mod inbox_detail;
pub mod library;
mod map_canvas;
mod map_outline;
mod map_shelf;
pub mod reader;
mod side_panel;
mod tangents;
pub mod task_detail;
mod task_research;
pub(crate) use task_research::blocking_round;
mod tree;
mod worktrees;

use dovetail_api::map::MapId;
use dovetail_api::{
    Command, Snapshot,
    agent::{AgentId, AgentStatus},
    inbox::InboxId,
    task::{Bucket, PLevel, TaskId, TaskStatus},
};
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

pub use crate::canvas::kitty::Figure;
pub use ask::{AskBuffer, ask_agent};
pub use board::{BoardBuffer, COLUMNS as BOARD_COLUMNS};
pub use conversation::ConversationBuffer;
pub use feed::{ActivityBuffer, MessagesBuffer};
pub use inbox::{InboxBuffer, ordered as inbox_order};
pub(crate) use inbox::{input_summary, tangents_title, unfiled_title};
pub use inbox_detail::InboxDetailBuffer;
pub use library::LibraryBuffer;
pub use map_canvas::Canvas as MapCanvas;
pub use map_outline::{MapOutlineBuffer, MouseKind as MapMouse, Reaction as MapReaction};
pub use map_shelf::MapShelfBuffer;
pub use reader::{ReadTarget, ReaderBuffer};
pub use side_panel::{SidePanelBuffer, side_chats};
pub use tangents::TangentsBuffer;
pub use task_detail::TaskDetailBuffer;
pub use tree::{TreeBuffer, p_label};
pub use worktrees::WorktreesBuffer;

use crate::action::Action;
use crate::region::BufferKind;
use crate::theme::Theme;

/// What a buffer reads while drawing or handling an action.
pub struct ViewCx<'a> {
    /// The UI's copy of the core's state.
    pub snap: &'a Snapshot,
    /// Colours.
    pub theme: &'a Theme,
    /// The UI's clock, in Unix milliseconds.
    pub now_ms: i64,
    /// Time zone for displayed times.
    pub tz: &'a TimeZone,
    /// When each working indicator began.
    pub activity: &'a crate::activity::Activity,
}

/// What typing in the prompt row is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputTarget {
    /// A `:` command.
    Command,
    /// A prompt to an agent.
    Prompt(AgentId),
    /// The reason for denying a permission request.
    Reason(InboxId),
    /// The answer to a question.
    Reply(InboxId),
    /// A new title for a task.
    Title(TaskId),
    /// The name of a new slice (the first of two prompts).
    SliceName,
    /// The brief of the slice named here (the second prompt).
    SliceBrief(String),
    /// The question that opens a side chat.
    SideChatQuestion,
    /// The question of a tangent (ADR 0017).
    TangentQuestion,
    /// An in-buffer search term.
    Search,
    /// A decision for this task's log.
    Decision(TaskId),
}

/// Something a buffer asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send a command to the core.
    Command(Command),
    /// Show an agent's conversation in the main region.
    OpenAgent(AgentId),
    /// Show a task's detail.
    OpenTask(TaskId),
    /// Show an inbox item in full.
    OpenInboxItem(InboxId),
    /// Fetch a map and show it in the outline.
    OpenMap(MapId),
    /// Fetch a research document of a task and read it in the task detail.
    OpenResearch {
        /// The task.
        task: TaskId,
        /// Which document.
        what: dovetail_api::research::ResearchWhat,
        /// Short name for the pane title.
        title: String,
    },
    /// Open the prompt row.
    StartInput {
        /// What the text is for.
        target: InputTarget,
        /// Text to start with.
        initial: String,
    },
    /// Open the picker to relate this task to another.
    PickRelated(TaskId),
    /// Ask `prompt` (y/n) and send `then` on `y`.
    Confirm {
        /// The question.
        prompt: String,
        /// What `y` sends.
        then: Vec<Command>,
    },
    /// Open the picker of tasks to file an unfiled finding (this inbox item) under.
    PickFileTarget(InboxId),
    /// A one-line note for the status bar.
    Note(String),
    /// Let go of the terminal and have the core open this path in the user's editor
    /// ([`TerminalCommand::OpenEditor`](dovetail_api::terminal::TerminalCommand::OpenEditor)); the terminal comes back on `TerminalEvent::Returned`.
    OpenEditor(std::path::PathBuf),
    /// Show a document in reading mode's reading column (fetching it from the core first, for
    /// research documents and docs).
    Read {
        /// What to show.
        target: ReadTarget,
        /// The research task its citations belong to.
        task: Option<TaskId>,
        /// Pane title part.
        title: String,
    },
    /// Follow a citation (`gd`), or open its web source in the browser (`gx`).
    Cite {
        /// The research task the ids belong to; `None` when the document is not research.
        task: Option<TaskId>,
        /// The id; `None` when there is none in view.
        evidence: Option<dovetail_api::research::Evidence>,
        /// `gx`: the browser, not the reading column.
        browse: bool,
        /// The document it was pressed in (`gx` with no citation opens a web source's own page).
        from: ReadTarget,
    },
    /// Show a project's library in reading mode, read-only: another project is browsed, the
    /// open one comes back (ADR 0018, decided 2026-11-11 later).
    BrowseProject(dovetail_api::project::ProjectId),
    /// Show the tangents list (ADR 0017).
    OpenTangents,
    /// Read a tangent's latest answer in reading mode.
    ReadTangent(TaskId),
    /// Pick the project to file a tangent into (`start: false`) or to start a task from it in.
    PickTangentProject {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// Start a task from it rather than file it.
        start: bool,
    },
    /// Attach a tangent to the focused task of this project (or one picked, when none is).
    AttachTangent(TaskId),
}

/// A view with its own state and buffer-local keys.
pub trait Buffer {
    /// Which kind this is, for the keymap scope.
    fn kind(&self) -> BufferKind;

    /// Draws the buffer, border included, into `area`.
    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool);

    /// Handles a movement or buffer-local action. Returns whether it was handled.
    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool;

    /// The text of each row, for in-buffer search (`/`, `n`, `N`). Empty when the buffer has
    /// nothing to search.
    fn search_rows(&self, _cx: &ViewCx) -> Vec<String> {
        Vec::new()
    }

    /// The row the cursor (or the top of the view) is on, in the numbering of
    /// [`search_rows`](Buffer::search_rows).
    fn current_row(&self, _cx: &ViewCx) -> usize {
        0
    }

    /// Moves the cursor (or the view) to a row of [`search_rows`](Buffer::search_rows).
    fn goto_row(&mut self, _row: usize, _cx: &ViewCx, _effects: &mut Vec<Effect>) {}
}

/// All buffer instances, one per kind.
pub struct Buffers {
    /// The task tree.
    pub tree: TreeBuffer,
    /// Needs you.
    pub inbox: InboxBuffer,
    /// A conversation.
    pub conversation: ConversationBuffer,
    /// A task's detail.
    pub task_detail: TaskDetailBuffer,
    /// An inbox item in full.
    pub inbox_detail: InboxDetailBuffer,
    /// The Activity feed.
    pub activity: ActivityBuffer,
    /// The message timeline.
    pub messages: MessagesBuffer,
    /// The board.
    pub board: BoardBuffer,
    /// The worktree view.
    pub worktrees: WorktreesBuffer,
    /// The map shelf.
    pub map_shelf: MapShelfBuffer,
    /// One map as an outline.
    pub map_outline: MapOutlineBuffer,
    /// The side panel: side chats beside the conversation.
    pub panel: SidePanelBuffer,
    /// Reading mode's library.
    pub library: LibraryBuffer,
    /// Reading mode's reading column.
    pub reader: ReaderBuffer,
    /// Reading mode's Ask column.
    pub ask: AskBuffer,
    /// The tangents list (ADR 0017).
    pub tangents: TangentsBuffer,
}

impl Buffers {
    /// Fresh buffers.
    pub fn new() -> Self {
        Self {
            tree: TreeBuffer::default(),
            inbox: InboxBuffer::default(),
            conversation: ConversationBuffer::default(),
            task_detail: TaskDetailBuffer::default(),
            inbox_detail: InboxDetailBuffer::default(),
            activity: ActivityBuffer::default(),
            messages: MessagesBuffer::default(),
            board: BoardBuffer::default(),
            worktrees: WorktreesBuffer::default(),
            map_shelf: MapShelfBuffer::default(),
            map_outline: MapOutlineBuffer::default(),
            panel: SidePanelBuffer::default(),
            library: LibraryBuffer::default(),
            reader: ReaderBuffer::default(),
            ask: AskBuffer::default(),
            tangents: TangentsBuffer::default(),
        }
    }

    /// The buffer of a kind.
    pub fn get_mut(&mut self, kind: BufferKind) -> &mut dyn Buffer {
        match kind {
            BufferKind::Tree => &mut self.tree,
            BufferKind::Inbox => &mut self.inbox,
            BufferKind::Conversation => &mut self.conversation,
            BufferKind::TaskDetail => &mut self.task_detail,
            BufferKind::InboxDetail => &mut self.inbox_detail,
            BufferKind::Activity => &mut self.activity,
            BufferKind::Messages => &mut self.messages,
            BufferKind::Board => &mut self.board,
            BufferKind::Worktrees => &mut self.worktrees,
            BufferKind::MapShelf => &mut self.map_shelf,
            BufferKind::MapOutline => &mut self.map_outline,
            BufferKind::SidePanel => &mut self.panel,
            BufferKind::Library => &mut self.library,
            BufferKind::Reader => &mut self.reader,
            BufferKind::Ask => &mut self.ask,
            BufferKind::Tangents => &mut self.tangents,
        }
    }
}

impl Default for Buffers {
    fn default() -> Self {
        Self::new()
    }
}

/// Draws a bordered pane with `title` and returns its inner area.
pub(crate) fn pane(
    frame: &mut Frame,
    area: Rect,
    title: Line<'static>,
    focused: bool,
    theme: &Theme,
) -> Rect {
    let border = if focused { theme.focus } else { theme.line };
    let block = Block::bordered()
        .border_style(theme.fg(border))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // One column of padding inside the border, as in the screens.
    Rect {
        x: inner.x.saturating_add(1),
        width: inner.width.saturating_sub(2),
        ..inner
    }
}

/// A pane title: ` text `, in the focus colour when focused.
pub(crate) fn title(text: &str, focused: bool, theme: &Theme) -> Line<'static> {
    let color = if focused { theme.focus } else { theme.dim };
    Line::from(Span::styled(format!(" {text} "), theme.fg(color)))
}

/// Draws a hint line on the last row of `inner`: the longest of `options` that fits, or the
/// shortest cut to size.
pub(crate) fn footer(frame: &mut Frame, inner: Rect, options: &[&str], theme: &Theme) {
    if inner.height < 5 || inner.width == 0 || options.is_empty() {
        return;
    }
    let row = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    let width = usize::from(row.width);
    let text = options
        .iter()
        .find(|o| o.width() <= width)
        .or(options.last())
        .copied()
        .unwrap_or("");
    let clipped = crate::fmt::truncate(text, width);
    frame.render_widget(Paragraph::new(Span::styled(clipped, theme.dim())), row);
}

/// Total cell width of some spans.
pub(crate) fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Cuts spans to `max` cells, ending in `…` when anything was cut.
pub(crate) fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    if spans_width(&spans) <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let w = span.content.width();
        if used + w < max {
            used += w;
            out.push(span);
        } else {
            let keep = max.saturating_sub(used);
            let cut = crate::fmt::truncate(&span.content, keep);
            out.push(Span::styled(cut, span.style));
            return out;
        }
    }
    out
}

/// One row: `left` at the start, `right` flush against the end, `left` clipped to make room.
pub(crate) fn spread(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    spread_fit(left, &[right], width)
}

/// Index of the right-hand version [`spread_fit`] uses beside a left of `left_w` cells: the first
/// that fits, else the last.
pub fn pick_right(left_w: usize, rights: &[Vec<Span<'static>>], width: usize) -> usize {
    rights
        .iter()
        .position(|r| {
            let w = spans_width(r);
            left_w + w + usize::from(w > 0) <= width
        })
        .unwrap_or(rights.len().saturating_sub(1))
}

/// Like [`spread`], with several versions of the right side, fullest first. The first one that
/// leaves `left` unclipped is used; if none does, the last (shortest) is.
pub fn spread_fit(
    left: Vec<Span<'static>>,
    rights: &[Vec<Span<'static>>],
    width: usize,
) -> Line<'static> {
    let right = rights
        .get(pick_right(spans_width(&left), rights, width))
        .cloned()
        .unwrap_or_default();
    let right_w = spans_width(&right);
    let room = width.saturating_sub(right_w + usize::from(right_w > 0));
    let mut left = clip_spans(left, room);
    let left_w = spans_width(&left);
    if right_w > 0 && left_w + right_w < width {
        left.push(Span::raw(" ".repeat(width - left_w - right_w)));
        left.extend(right);
    }
    Line::from(left)
}

/// Glyph and colour for an agent's status.
pub(crate) fn status_glyph(status: AgentStatus, theme: &Theme) -> (&'static str, Color) {
    match status {
        AgentStatus::SettingUp => ("◌", theme.dim),
        AgentStatus::Working => ("◐", theme.working),
        AgentStatus::Idle => ("○", theme.dim),
        AgentStatus::NeedsYou => ("⚠", theme.needs_you),
        AgentStatus::Done => ("✓", theme.ok),
        AgentStatus::Error => ("✗", theme.error),
    }
}

/// Word for an agent's status.
pub(crate) fn status_word(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::SettingUp => "setting up",
        AgentStatus::Working => "working",
        AgentStatus::Idle => "idle",
        AgentStatus::NeedsYou => "needs you",
        AgentStatus::Done => "done",
        AgentStatus::Error => "error",
    }
}

/// The next status in the cycle todo, in progress, done (plan B4).
pub(crate) fn next_status(status: TaskStatus) -> TaskStatus {
    match status {
        TaskStatus::Todo => TaskStatus::InProgress,
        TaskStatus::InProgress | TaskStatus::Review => TaskStatus::Done,
        TaskStatus::Done | TaskStatus::Archived => TaskStatus::Todo,
    }
}

/// The next bucket in the cycle Unsorted, Now, Next, Later.
pub(crate) fn next_bucket(bucket: Option<Bucket>) -> Option<Bucket> {
    match bucket {
        None => Some(Bucket::Now),
        Some(Bucket::Now) => Some(Bucket::Next),
        Some(Bucket::Next) => Some(Bucket::Later),
        Some(Bucket::Later) => None,
    }
}

/// The next P-level in the cycle none, P0, P1, P2, P3.
pub(crate) fn next_p_level(p: Option<PLevel>) -> Option<PLevel> {
    match p {
        None => Some(PLevel::P0),
        Some(PLevel::P0) => Some(PLevel::P1),
        Some(PLevel::P1) => Some(PLevel::P2),
        Some(PLevel::P2) => Some(PLevel::P3),
        Some(PLevel::P3) => None,
    }
}

/// Words for a task status.
pub(crate) fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "to do",
        TaskStatus::InProgress | TaskStatus::Review => "in progress",
        TaskStatus::Done => "done",
        TaskStatus::Archived => "archived",
    }
}

/// Words for a bucket; no bucket is Unsorted.
pub(crate) fn bucket_label(bucket: Option<Bucket>) -> &'static str {
    match bucket {
        None => "Unsorted",
        Some(Bucket::Now) => "Now",
        Some(Bucket::Next) => "Next",
        Some(Bucket::Later) => "Later",
    }
}

/// Moves a selection through `len` rows, clamping at both ends.
pub(crate) fn step(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    index.saturating_add_signed(delta).min(len - 1)
}

/// Scrolls `offset` so that rows `first..=last` are visible in a window of `height` rows,
/// showing as much of the range as fits.
pub(crate) fn scroll_to(offset: usize, first: usize, last: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    if first < offset {
        return first;
    }
    if last >= offset + height {
        return (last + 1).saturating_sub(height).min(first);
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn spread_pads_between_left_and_right() {
        let line = spread(vec![Span::raw("ab")], vec![Span::raw("cd")], 8);
        assert_eq!(text(&line), "ab    cd");
    }

    #[test]
    fn spread_clips_the_left_side_first() {
        let line = spread(vec![Span::raw("abcdefgh")], vec![Span::raw("xy")], 8);
        assert_eq!(spans_width(&line.spans), 8);
        assert!(text(&line).ends_with("xy"));
    }

    #[test]
    fn the_cycles_wrap_around() {
        assert_eq!(next_status(TaskStatus::Done), TaskStatus::Todo);
        assert_eq!(next_bucket(Some(Bucket::Later)), None);
        assert_eq!(next_p_level(Some(PLevel::P3)), None);
        assert_eq!(next_p_level(None), Some(PLevel::P0));
    }

    #[test]
    fn step_clamps() {
        assert_eq!(step(0, 3, -1), 0);
        assert_eq!(step(2, 3, 1), 2);
        assert_eq!(step(0, 0, 1), 0);
        assert_eq!(step(1, 3, 1), 2);
    }

    #[test]
    fn scroll_keeps_the_selection_in_view() {
        assert_eq!(scroll_to(0, 12, 12, 10), 3);
        assert_eq!(scroll_to(5, 2, 2, 10), 2);
        assert_eq!(scroll_to(0, 3, 3, 10), 0);
    }
}
