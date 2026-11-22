//! Drawing: the screens bar, the regions, the status bar, the prompt row and the overlays.

use dovetail_api::{agent::AgentId, status::UNREGISTERED_WARNING};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ratatui::widgets::{Block, Clear};

use super::App;
use super::overlay::{Overlay, Purpose};

use crate::buffers::{self, InputTarget, ViewCx, spread_fit};
use crate::capture::{self, Capture};
use crate::help;
use crate::keymap::Key;
use crate::mode::Mode;
use crate::picker::{PickerItem, PickerKey};
use crate::region::{RegionId, layout, reading_layout};
use crate::switcher;
use crate::tabs;
use crate::usage;

impl App {
    /// Draws the whole screen.
    pub fn draw(&mut self, frame: &mut Frame) {
        let prompting = self.input.is_some() || self.confirm.is_some();
        // Each conversation says again, as it draws, whether a spinner is on screen.
        self.buffers.conversation.clear_working();
        self.buffers.tree.clear_working();
        self.buffers.panel.conversation_mut().clear_working();
        self.buffers.ask.conversation_mut().clear_working();
        // Whatever moved focus to the side column (a picker choice, Esc from an inbox item)
        // ends focus mode, so the focused region is always on screen.
        if self.focus_mode && self.regions.focus().is_side() {
            self.toggle_focus_mode();
        }
        if self.reading_mode() {
            self.draw_reading(frame, prompting);
            return;
        }
        let screen = frame.area();
        self.buffers
            .map_outline
            .set_screen(screen.width, screen.height);
        self.buffers
            .map_outline
            .set_map_mode(self.mode == Mode::Map);
        // MAP mode gives the map the whole width, as focus mode does the conversation; neither
        // shows the tab row.
        let full = self.focus_mode || self.mode == Mode::Map;
        let mut areas = layout(screen, prompting, !full);
        if full {
            areas = areas.focused();
        }
        self.draw_tabs(frame, areas.tabs);
        let cx = ViewCx {
            snap: &self.snap,
            theme: &self.theme,
            now_ms: self.now_ms,
            tz: &self.tz,
            activity: &self.activity,
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
        self.draw_which_key(frame, areas.status);
        self.draw_overlay(frame);
    }

    /// Reading mode: library, reading column and Ask, between the header and the status bar.
    fn draw_reading(&mut self, frame: &mut Frame, prompting: bool) {
        let areas = reading_layout(frame.area(), prompting);
        self.draw_tabs(frame, areas.tabs);
        let cx = ViewCx {
            snap: &self.snap,
            theme: &self.theme,
            now_ms: self.now_ms,
            tz: &self.tz,
            activity: &self.activity,
        };
        for region in RegionId::READING {
            let area = areas.of(region);
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
        self.draw_which_key(frame, areas.status);
        self.draw_overlay(frame);
    }

    /// The which-key popup, while a half-typed sequence waits (task #19): bottom-right, above the
    /// status bar, listing what can come next.
    fn draw_which_key(&self, frame: &mut Frame, status: Rect) {
        let rows = self.which_key();
        if !rows.is_empty() {
            let pending = pending_label(self.resolver.pending());
            help::render_which_key(frame, status, &pending, &rows, &self.theme);
        }
    }

    /// The picker or quick-capture box, floating over everything else.
    fn draw_overlay(&self, frame: &mut Frame) {
        let screen = frame.area();
        match &self.overlay {
            Some(Overlay::Picker(picker, purpose)) => {
                let width = picker_preview_width(screen);
                let (title, lines) = match (picker.selected(), purpose) {
                    (Some(item), Purpose::Browse) => {
                        switcher::browse_preview(&self.snap, &item.key, width, &self.theme)
                    }
                    (Some(item), Purpose::Profile { path, .. }) => (
                        "Profile",
                        switcher::profile_preview(&item.key, path, width, &self.theme),
                    ),
                    (
                        Some(PickerItem {
                            key: PickerKey::Action(action),
                            ..
                        }),
                        _,
                    ) => (
                        "Keys",
                        help::keys_preview(&self.keymap, &self.scopes(), *action, &self.theme),
                    ),
                    (Some(item), _) => {
                        switcher::preview(&self.snap, &item.key, width, &self.theme, &self.tz)
                    }
                    (None, _) => ("Preview", Vec::new()),
                };
                picker.render(frame, screen, &self.theme, title, &lines);
            }
            Some(Overlay::Capture(capture)) => self.draw_capture(frame, screen, capture),
            Some(Overlay::Mcp(menu)) => {
                let width = picker_preview_width(screen);
                let lines = menu.preview(width, &self.theme);
                menu.picker()
                    .render(frame, screen, &self.theme, "Server", &lines);
            }
            Some(Overlay::Usage(menu)) => {
                let width = picker_preview_width(screen);
                let lines = menu.preview(width, self.now_ms, &self.tz, &self.theme);
                menu.picker()
                    .render(frame, screen, &self.theme, "Limits", &lines);
            }
            Some(Overlay::Model(menu)) => {
                let width = picker_preview_width(screen);
                let lines = menu.preview(width, &self.theme);
                menu.picker()
                    .render(frame, screen, &self.theme, "Model", &lines);
            }
            Some(Overlay::Confirm(confirm)) => confirm.render(frame, screen, &self.theme),
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

    /// The screens bar (task #20), boxed like the other regions; the tab table and its marking
    /// live in [`tabs`].
    fn draw_tabs(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let inner = buffers::pane(
            frame,
            area,
            buffers::title("screens", false, &self.theme),
            false,
            &self.theme,
        );
        let view = tabs::ViewState {
            main: self.regions.buffer(RegionId::Main),
            reading: self.reading_mode(),
            research_list: matches!(
                &self.overlay,
                Some(Overlay::Picker(picker, Purpose::Open)) if picker.title() == "Research"
            ),
        };
        let spans = tabs::spans(&view, &self.theme, usize::from(inner.width));
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let theme = &self.theme;
        let badge_color = match self.mode {
            Mode::Normal => theme.mode_normal,
            Mode::Insert => theme.mode_insert,
            Mode::Command => theme.mode_command,
            Mode::Map => theme.mode_map,
        };
        let snap = &self.snap;
        let mut left = vec![
            Span::styled(
                format!(" {} ", self.mode_label()),
                theme
                    .badge(badge_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {}",
                    snap.project
                        .as_ref()
                        .map_or("no project", |p| p.name.as_str())
                ),
                theme.body(),
            ),
        ];
        // A long command (:eod, :fresh, :upgrade) shows its spinner next to the mode badge,
        // where the bar clips last.
        if let Some((busy, ran)) = self.activity.busy(self.now_ms) {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(crate::render::working::busy_spans(
                theme,
                busy,
                ran,
                self.now_ms,
            ));
            left.splice(1..1, spans);
        }
        if self.reading_mode() {
            left.push(Span::raw("  "));
            left.push(Span::styled(
                " READING ",
                theme
                    .badge(theme.cite)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ));
            if let Some(cite) = self.buffers.reader.citation_label() {
                left.push(Span::styled(format!(" {cite}"), theme.fg(theme.cite)));
            }
        }
        if !self.resolver.pending().is_empty() {
            left.push(Span::styled(
                format!("  {} …", pending_label(self.resolver.pending())),
                theme.fg(theme.italic),
            ));
        }
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
            None => self.status_rights(),
        };
        frame.render_widget(
            Paragraph::new(spread_fit(left, &rights, usize::from(area.width))),
            area,
        );
    }

    /// The right of the status bar: the not-a-project warning (when it applies) and the usage
    /// line with the context fill, each in versions from fullest to shortest.
    fn status_rights(&self) -> Vec<Vec<Span<'static>>> {
        let theme = &self.theme;
        let snap = &self.snap;
        let in_view = self
            .buffers
            .conversation
            .agent()
            .and_then(|id| snap.agents.iter().find(|a| &a.id == id));
        let context_pct = self
            .coordinator()
            .and_then(|id| snap.agents.iter().find(|a| a.id == id))
            .and_then(|a| a.context_pct);
        // Task #16: the project's profile (ADR 0009), the model of the conversation in view and
        // the profile's usage limits.
        let usage = usage::variants(
            &snap.usage,
            &usage::Lead {
                profile: snap.profile.as_deref(),
                model: in_view.and_then(|a| a.model.as_deref()),
                effort: in_view.and_then(|a| a.effort),
                context_pct,
            },
            self.now_ms,
            &self.tz,
            theme,
        );
        if !snap.warnings.iter().any(|w| w.id.0 == UNREGISTERED_WARNING) {
            return usage;
        }
        let warnings = [
            "⚠ not a Dovetail project yet: :project add keeps it ",
            "⚠ not a project yet ",
        ]
        .map(|text| vec![Span::styled(text, theme.fg(theme.needs_you))]);
        // Every warning version with every usage version: the usage shortens through all of its
        // versions before the warning shortens.
        if usage.is_empty() {
            return warnings.into();
        }
        warnings
            .iter()
            .flat_map(|w| {
                usage.iter().map(move |u| {
                    let mut both = w.clone();
                    both.extend(u.iter().cloned());
                    both
                })
            })
            .collect()
    }

    /// Mode label for the status bar; while chatting it names the agent (as in the mocks).
    fn mode_label(&self) -> String {
        match (self.mode, self.chat_view().agent()) {
            (Mode::Insert, Some(agent)) if self.chat_view().is_chatting() => {
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
                // A three-way question names its keys in the prompt itself.
                Span::styled(
                    if confirm.otherwise.is_some() {
                        ""
                    } else {
                        " (y/n)"
                    },
                    theme.dim(),
                ),
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
            InputTarget::TangentQuestion => "tangent › ".into(),
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

/// The preview pane is the right 55 % of a picker box at most 130 wide, less its border.
fn picker_preview_width(screen: Rect) -> usize {
    (usize::from(screen.width.saturating_sub(4).min(130)) * 55 / 100).saturating_sub(4)
}

/// Keys of an unfinished sequence, for the status bar.
fn pending_label(keys: &[Key]) -> String {
    keys.iter().map(Key::label).collect()
}
