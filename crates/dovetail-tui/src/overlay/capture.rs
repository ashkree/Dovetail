//! Quick capture (Decision 21): one line that becomes a task. `!p0`–`!p3` set the P-level,
//! `@now`/`@next`/`@later` the bucket and `#1231` a pull request; every other word is the
//! title. With no bucket the task lands in Unsorted.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::tracking::task::{Bucket, NewTask, PLevel, TaskId};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::buffers;
use crate::input::{InputOutcome, TextInput};
use crate::theme::Theme;

/// What a capture line says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Captured {
    /// The words that are not tokens, in order.
    pub title: String,
    /// From `!p0`–`!p3`; the last one wins.
    pub p_level: Option<PLevel>,
    /// From `@now`, `@next` or `@later`; the last one wins.
    pub bucket: Option<Bucket>,
    /// From `#<number>`; the last one wins.
    pub pr: Option<u32>,
}

/// Reads a capture line. A word that looks like a token but is not one (`!p9`, `@soon`, `#abc`)
/// stays in the title.
pub fn parse(text: &str) -> Captured {
    let mut out = Captured::default();
    let mut title: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        match lower.as_str() {
            "!p0" => out.p_level = Some(PLevel::P0),
            "!p1" => out.p_level = Some(PLevel::P1),
            "!p2" => out.p_level = Some(PLevel::P2),
            "!p3" => out.p_level = Some(PLevel::P3),
            "@now" => out.bucket = Some(Bucket::Now),
            "@next" => out.bucket = Some(Bucket::Next),
            "@later" => out.bucket = Some(Bucket::Later),
            _ => match word.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()) {
                Some(pr) => out.pr = Some(pr),
                None => title.push(word),
            },
        }
    }
    out.title = title.join(" ");
    out
}

/// The task the capture can relate the new one to: the one in focus when `<leader>t` was
/// pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// The task.
    pub id: TaskId,
    /// Its title, shown under the line.
    pub title: String,
}

/// What a key did to the capture overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// Still typing.
    Pending,
    /// Save the task; with `open_detail`, then show its detail view (`Tab`).
    Save {
        /// Open the task's detail afterwards.
        open_detail: bool,
    },
    /// Throw the line away.
    Cancel,
}

/// The quick-capture overlay's state.
#[derive(Debug)]
pub struct Capture {
    input: TextInput,
    link: Option<LinkTarget>,
    /// The link is on by default and `Ctrl-l` switches it off and on.
    linked: bool,
}

impl Capture {
    /// An empty capture line, offering to link to `link`.
    pub fn new(link: Option<LinkTarget>) -> Self {
        Self {
            input: TextInput::line(""),
            link,
            linked: true,
        }
    }

    /// The line being typed.
    pub fn input(&self) -> &TextInput {
        &self.input
    }

    /// The task offered as a link while the link is on.
    pub fn active_link(&self) -> Option<&LinkTarget> {
        self.link.as_ref().filter(|_| self.linked)
    }

    /// The task that could be linked, on or off.
    pub fn offered_link(&self) -> Option<&LinkTarget> {
        self.link.as_ref()
    }

    /// Applies a key.
    pub fn key(&mut self, key: KeyEvent) -> CaptureOutcome {
        if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.linked = !self.linked;
            return CaptureOutcome::Pending;
        }
        if key.code == KeyCode::Tab {
            return CaptureOutcome::Save { open_detail: true };
        }
        match self.input.handle(key) {
            InputOutcome::Submit => CaptureOutcome::Save { open_detail: false },
            InputOutcome::Cancel => CaptureOutcome::Cancel,
            InputOutcome::Edited | InputOutcome::Ignored => CaptureOutcome::Pending,
        }
    }

    /// The task to create, or `None` for an empty title.
    pub fn build(&self) -> Option<NewTask> {
        let parsed = parse(self.input.text());
        if parsed.title.is_empty() {
            return None;
        }
        Some(NewTask {
            title: parsed.title,
            p_level: parsed.p_level,
            bucket: parsed.bucket,
            notes: String::new(),
            follow_up_of: None,
            pr: parsed.pr,
            // The link relates the two tasks both ways (decided 2026-11-03).
            related: self.active_link().map(|l| l.id).into_iter().collect(),
            effort: Default::default(),
            criterion: None,
        })
    }

    /// Draws the quick-capture box centered on `screen`.
    pub fn render(&self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        let width = screen.width.saturating_sub(4).min(90);
        let height = 6.min(screen.height);
        let area = super::centered_rect(screen, width, height);
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_style(theme.fg(theme.focus))
            .title(Line::from(Span::styled(
                " new task ",
                theme.fg(theme.focus),
            )));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let row = |n: u16| Rect {
            x: inner.x + 1,
            y: inner.y + n,
            width: inner.width.saturating_sub(2),
            height: 1,
        };
        if inner.height < 4 {
            return;
        }
        let text = self.input.text();
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("› ", theme.accent()),
                Span::styled(text.to_string(), theme.body()),
            ])),
            row(0),
        );
        frame.set_cursor_position(Position::new(
            (row(0).x
                + 2
                + unicode_width::UnicodeWidthStr::width(self.input.before_cursor()) as u16)
                .min(row(0).x + row(0).width.saturating_sub(1)),
            row(0).y,
        ));

        // What the line will become.
        let parsed = parse(text);
        let mut facts = Vec::new();
        facts.push(
            parsed
                .p_level
                .map_or("no P-level", buffers::p_label)
                .to_string(),
        );
        facts.push(buffers::bucket_label(parsed.bucket).to_string());
        if let Some(pr) = parsed.pr {
            facts.push(format!("PR #{pr}"));
        }
        frame.render_widget(
            Paragraph::new(Span::styled(facts.join(" · "), theme.dim())),
            row(1),
        );
        let link = match (self.active_link(), self.offered_link()) {
            (Some(l), _) => Line::from(vec![
                Span::styled("linked to: ", theme.dim()),
                Span::styled(l.title.clone(), theme.fg(theme.code)),
                Span::styled("   Ctrl-l unlink", theme.dim()),
            ]),
            (None, Some(l)) => Line::from(Span::styled(
                format!("not linked to {} · Ctrl-l to link", l.title),
                theme.dim(),
            )),
            (None, None) => Line::default(),
        };
        frame.render_widget(Paragraph::new(link), row(2));
        frame.render_widget(
            Paragraph::new(Span::styled(
                "⏎ save · Tab save and open · esc cancel",
                theme.dim(),
            )),
            row(3),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_picked_out_and_the_rest_is_the_title() {
        let c = parse("fix eval key !p1 @now #1231 flow");
        assert_eq!(c.title, "fix eval key flow");
        assert_eq!(c.p_level, Some(PLevel::P1));
        assert_eq!(c.bucket, Some(Bucket::Now));
        assert_eq!(c.pr, Some(1231));
    }

    #[test]
    fn no_tokens_means_unsorted_and_no_p_level() {
        let c = parse("just a title");
        assert_eq!(c.title, "just a title");
        assert_eq!((c.p_level, c.bucket, c.pr), (None, None, None));
    }

    #[test]
    fn the_last_token_of_a_kind_wins() {
        let c = parse("x !p3 !p0 @later @next");
        assert_eq!(c.p_level, Some(PLevel::P0));
        assert_eq!(c.bucket, Some(Bucket::Next));
    }

    #[test]
    fn tokens_are_case_insensitive() {
        let c = parse("x !P2 @Next");
        assert_eq!(
            (c.p_level, c.bucket),
            (Some(PLevel::P2), Some(Bucket::Next))
        );
    }

    #[test]
    fn near_misses_stay_in_the_title() {
        let c = parse("see !p9 @soon #abc #12x email@now");
        assert_eq!(c.title, "see !p9 @soon #abc #12x email@now");
        assert_eq!((c.p_level, c.bucket, c.pr), (None, None, None));
    }

    #[test]
    fn extra_spaces_collapse_and_a_tokens_only_line_has_no_title() {
        assert_eq!(parse("  a    b  ").title, "a b");
        assert_eq!(parse("!p0 @now").title, "");
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn link() -> LinkTarget {
        LinkTarget {
            id: TaskId(7),
            title: "business surface".into(),
        }
    }

    #[test]
    fn enter_saves_tab_saves_and_opens_the_detail_esc_cancels() {
        let mut c = Capture::new(None);
        assert_eq!(c.key(key('a')), CaptureOutcome::Pending);
        assert_eq!(
            c.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            CaptureOutcome::Save { open_detail: false }
        );
        assert_eq!(
            c.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            CaptureOutcome::Save { open_detail: true }
        );
        assert_eq!(
            c.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            CaptureOutcome::Cancel
        );
    }

    #[test]
    fn the_link_is_on_by_default_and_ctrl_l_switches_it() {
        let mut c = Capture::new(Some(link()));
        c.key(key('x'));
        assert_eq!(c.build().map(|t| t.related), Some(vec![TaskId(7)]));
        c.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert_eq!(c.build().map(|t| t.related), Some(vec![]));
        assert!(
            c.offered_link().is_some(),
            "the offer stays so Ctrl-l can bring it back"
        );
        c.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(c.active_link().is_some());
    }

    #[test]
    fn an_empty_title_builds_nothing() {
        let mut c = Capture::new(None);
        for ch in "!p1 @now".chars() {
            c.key(key(ch));
        }
        assert!(c.build().is_none());
    }

    #[test]
    fn build_carries_every_field() {
        let mut c = Capture::new(None);
        for ch in "ship it !p0 @next #99".chars() {
            c.key(key(ch));
        }
        let task = c.build().expect("a title");
        assert_eq!(task.title, "ship it");
        assert_eq!(task.p_level, Some(PLevel::P0));
        assert_eq!(task.bucket, Some(Bucket::Next));
        assert_eq!(task.pr, Some(99));
    }
}
