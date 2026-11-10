//! Drawing: the header row, the regions, the status bar, the prompt row and the overlays.

use std::path::Path;

use dovetail_api::{AgentId, AgentStatus, UNREGISTERED_WARNING};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ratatui::widgets::{Block, Clear};

use super::overlay::Overlay;
use super::{App, HINTS, HINTS_SHORT};
use crate::buffers::{self, InputTarget, ViewCx, spread_fit};
use crate::capture::{self, Capture};
use crate::fmt;
use crate::keymap::Key;
use crate::mode::Mode;
use crate::region::{RegionId, layout};
use crate::switcher;
use crate::usage;

impl App {
    /// Draws the whole screen.
    pub fn draw(&mut self, frame: &mut Frame) {
        let prompting = self.input.is_some() || self.confirm.is_some();
        // Whatever moved focus to the side column (a picker choice, Esc from an inbox item)
        // ends focus mode, so the focused region is always on screen.
        if self.focus_mode.is_some() && self.regions.focus().is_side() {
            self.toggle_focus_mode();
        }
        let mut areas = layout(frame.area(), prompting);
        if self.focus_mode.is_some() {
            areas = areas.focused();
        }
        self.draw_header(frame, areas.header);
        let cx = ViewCx {
            snap: &self.snap,
            theme: &self.theme,
            now_ms: self.now_ms,
            tz: &self.tz,
        };
        for (region, area) in [
            (RegionId::SideTop, areas.side_top),
            (RegionId::SideBottom, areas.side_bottom),
            (RegionId::Main, areas.main),
        ] {
            if area.width == 0 {
                continue;
            }
            let focused = self.regions.focus() == region;
            self.buffers
                .get_mut(self.regions.buffer(region))
                .render(&cx, frame, area, focused);
        }
        self.draw_status(frame, areas.status);
        if let Some(area) = areas.prompt {
            self.draw_prompt(frame, area);
        }
        self.draw_overlay(frame);
    }

    /// The picker or quick-capture box, floating over everything else.
    fn draw_overlay(&self, frame: &mut Frame) {
        let screen = frame.area();
        match &self.overlay {
            Some(Overlay::Picker(picker, _)) => {
                // The preview pane is the right 55 % of a box at most 130 wide, less its border.
                let width = (usize::from(screen.width.saturating_sub(4).min(130)) * 55 / 100)
                    .saturating_sub(4);
                let (title, lines) = match picker.selected() {
                    Some(item) => {
                        switcher::preview(&self.snap, &item.key, width, &self.theme, &self.tz)
                    }
                    None => ("Preview", Vec::new()),
                };
                picker.render(frame, screen, &self.theme, title, &lines);
            }
            Some(Overlay::Capture(capture)) => self.draw_capture(frame, screen, capture),
            None => {}
        }
    }

    fn draw_capture(&self, frame: &mut Frame, screen: Rect, capture: &Capture) {
        let theme = &self.theme;
        let width = screen.width.saturating_sub(4).min(90);
        let height = 6.min(screen.height);
        let area = Rect {
            x: screen.x + (screen.width - width) / 2,
            y: screen.y + screen.height.saturating_sub(height) / 2,
            width,
            height,
        };
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
        let text = capture.input().text();
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
                + unicode_width::UnicodeWidthStr::width(capture.input().before_cursor()) as u16)
                .min(row(0).x + row(0).width.saturating_sub(1)),
            row(0).y,
        ));

        // What the line will become.
        let parsed = capture::parse(text);
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
        let link = match (capture.active_link(), capture.offered_link()) {
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

    /// The folder the agent in view works in (a slice's worktree, otherwise the project
    /// folder), and a warning while that folder is not a Dovetail project yet.
    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let theme = &self.theme;
        let snap = &self.snap;
        let in_view = self
            .buffers
            .conversation
            .agent()
            .and_then(|id| snap.agents.iter().find(|a| &a.id == id));
        let worktree = in_view.and_then(|a| a.worktree.as_ref());
        let mut left = vec![Span::raw(" ")];
        match (worktree, &snap.project) {
            (Some(wt), _) => {
                left.push(Span::styled("worktree ", theme.dim()));
                left.push(Span::styled(home_relative(&wt.path), theme.body()));
                left.push(Span::styled(" on ", theme.dim()));
                left.push(Span::styled(wt.branch.clone(), theme.fg(theme.code)));
            }
            (None, Some(project)) => {
                left.push(Span::styled("folder ", theme.dim()));
                left.push(Span::styled(
                    home_relative(&project.repo_path),
                    theme.body(),
                ));
            }
            (None, None) => left.push(Span::styled("no folder open", theme.dim())),
        }
        let unregistered = snap.warnings.iter().any(|w| w.id.0 == UNREGISTERED_WARNING);
        let warnings = if unregistered {
            vec![
                vec![Span::styled(
                    "⚠ not a Dovetail project yet: :project add keeps it ",
                    theme.fg(theme.needs_you),
                )],
                vec![Span::styled(
                    "⚠ not a project yet ",
                    theme.fg(theme.needs_you),
                )],
            ]
        } else {
            Vec::new()
        };
        // Task #16: the project's profile (ADR 0009), the model of the conversation in view and
        // the profile's usage limits.
        let usage = usage::variants(
            &snap.usage,
            snap.profile.as_deref(),
            in_view.and_then(|a| a.model.as_deref()),
            self.now_ms,
            &self.tz,
            theme,
        );
        // Every warning version with every usage version: the usage shortens through all of its
        // versions before the warning shortens.
        let rights: Vec<Vec<Span<'static>>> = match (warnings.is_empty(), usage.is_empty()) {
            (true, _) => usage,
            (false, true) => warnings,
            (false, false) => warnings
                .iter()
                .flat_map(|w| {
                    usage.iter().map(move |u| {
                        let mut both = w.clone();
                        both.push(Span::raw(" "));
                        both.extend(u.iter().cloned());
                        both
                    })
                })
                .collect(),
        };
        frame.render_widget(
            Paragraph::new(spread_fit(left, &rights, usize::from(area.width))),
            area,
        );
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let theme = &self.theme;
        let badge_color = match self.mode {
            Mode::Normal => theme.mode_normal,
            Mode::Insert => theme.mode_insert,
            Mode::Command => theme.mode_command,
        };
        let snap = &self.snap;
        let tasks = snap
            .tasks
            .iter()
            .filter(|t| {
                !matches!(
                    t.status,
                    dovetail_api::TaskStatus::Done | dovetail_api::TaskStatus::Archived
                )
            })
            .count();
        let agents = snap
            .agents
            .iter()
            .filter(|a| a.kind.is_session() && a.status != AgentStatus::Done)
            .count();
        let needs_you = snap.inbox.len();
        let mut left = vec![
            Span::styled(
                format!(" {} ", self.mode_label()),
                theme
                    .badge(badge_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {} · {tasks} tasks · {agents} agents · ",
                    snap.project
                        .as_ref()
                        .map_or("no project", |p| p.name.as_str())
                ),
                theme.body(),
            ),
            Span::styled(
                format!("{needs_you} ⚠"),
                if needs_you > 0 {
                    theme.fg(theme.needs_you)
                } else {
                    theme.dim()
                },
            ),
            Span::styled(
                format!(" · {}", fmt::memory(snap.totals.memory_mb)),
                theme.body(),
            ),
        ];
        if let Some(pct) = self
            .coordinator()
            .and_then(|id| snap.agents.iter().find(|a| a.id == id))
            .and_then(|a| a.context_pct)
        {
            left.push(Span::styled(format!(" · ctx {pct}%"), theme.context(pct)));
        }
        if !self.resolver.pending().is_empty() {
            left.push(Span::styled(
                format!("  {} …", pending_label(self.resolver.pending())),
                theme.fg(theme.italic),
            ));
        }
        // The header row carries the not-a-project warning.
        if let Some(warning) = snap
            .warnings
            .iter()
            .find(|w| w.id.0 != UNREGISTERED_WARNING)
        {
            left.push(Span::styled(
                format!("  ⚠ {}", warning.text),
                theme.fg(theme.needs_you),
            ));
        }
        let rights = match &self.note {
            Some(note) => vec![vec![Span::styled(note.clone(), theme.fg(theme.needs_you))]],
            None => vec![
                vec![Span::styled(HINTS, theme.dim())],
                vec![Span::styled(HINTS_SHORT, theme.dim())],
            ],
        };
        frame.render_widget(
            Paragraph::new(spread_fit(left, &rights, usize::from(area.width))),
            area,
        );
    }

    /// Mode label for the status bar; while chatting it names the agent (as in the mocks).
    fn mode_label(&self) -> String {
        match (self.mode, self.buffers.conversation.agent()) {
            (Mode::Insert, Some(agent)) if self.buffers.conversation.is_chatting() => {
                format!("INSERT · {}", self.agent_name(agent))
            }
            (mode, _) => mode.label().to_string(),
        }
    }

    fn draw_prompt(&self, frame: &mut Frame, area: Rect) {
        let theme = &self.theme;
        if let Some(confirm) = &self.confirm {
            let line = Line::from(vec![
                Span::styled(confirm.prompt.clone(), theme.fg(theme.needs_you)),
                Span::styled(" (y/n)", theme.dim()),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        let Some(input) = &self.input else { return };
        let label = match &input.target {
            InputTarget::Command => ":".to_string(),
            InputTarget::Prompt(agent) => format!("message to {} › ", self.agent_name(agent)),
            InputTarget::Reason(_) => "deny reason › ".into(),
            InputTarget::Reply(_) => "reply › ".into(),
            InputTarget::Title(_) => "title › ".into(),
            InputTarget::SliceName => "slice name › ".into(),
            InputTarget::SideChatQuestion => "side chat › ".into(),
            InputTarget::SliceBrief(name) => format!("brief for {name} › "),
            InputTarget::Search => "/".into(),
            InputTarget::Decision(_) => "decision › ".into(),
        };
        let used = unicode_width::UnicodeWidthStr::width(label.as_str()) as u16;
        let line = Line::from(vec![
            Span::styled(label, theme.accent()),
            Span::styled(input.line.text().to_string(), theme.body()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        let typed = unicode_width::UnicodeWidthStr::width(input.line.before_cursor()) as u16;
        frame.set_cursor_position(Position::new(
            (area.x + used + typed).min(area.x + area.width.saturating_sub(1)),
            area.y,
        ));
    }

    fn agent_name(&self, id: &AgentId) -> String {
        self.snap
            .agents
            .iter()
            .find(|a| &a.id == id)
            .map_or_else(|| id.0.clone(), |a| a.name.clone())
    }
}

/// `path` with the home directory shown as `~`.
fn home_relative(path: &Path) -> String {
    match std::env::var_os("HOME").map(std::path::PathBuf::from) {
        Some(home) if !home.as_os_str().is_empty() => match path.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        _ => path.display().to_string(),
    }
}

/// Keys of an unfinished sequence, for the status bar.
fn pending_label(keys: &[Key]) -> String {
    keys.iter().map(Key::label).collect()
}
