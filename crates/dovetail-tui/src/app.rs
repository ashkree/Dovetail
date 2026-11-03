//! The application state: snapshot, mode, regions, buffers and the glue between keys, buffers
//! and core commands. It is plain state with no terminal or thread in it, so tests drive it with
//! key events and read what it would draw or send.

use crossterm::event::KeyEvent;
use dovetail_api::{
    AgentId, AgentKind, AgentStatus, ApplyResult, Command, Event, InboxKind, NewTask,
    PROTOCOL_VERSION, PermissionAnswer, ServerMsg, Snapshot, TaskChange, apply,
};
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::action::Action;
use crate::buffers::{Buffers, Effect, InputTarget, ViewCx, spread};
use crate::command;
use crate::fmt;
use crate::input::{InputOutcome, LineInput};
use crate::keymap::{Key, Keymap, Resolved, Resolver, Scope};
use crate::mode::Mode;
use crate::region::{BufferKind, RegionId, Regions, layout};
use crate::theme::Theme;

/// Hints on the right of the status bar.
const HINTS: &str = "␣n inbox · : command";

/// An open prompt row.
struct Input {
    target: InputTarget,
    line: LineInput,
}

/// A yes/no question waiting for `y` or `n`.
struct Confirm {
    prompt: String,
    action: Action,
}

/// The whole UI state.
pub struct App {
    snap: Snapshot,
    theme: Theme,
    tz: TimeZone,
    now_ms: i64,
    keymap: Keymap,
    resolver: Resolver,
    mode: Mode,
    regions: Regions,
    buffers: Buffers,
    input: Option<Input>,
    confirm: Option<Confirm>,
    note: Option<String>,
    outbox: Vec<Command>,
    /// A resync was requested and its snapshot has not arrived yet.
    resyncing: bool,
    quit: bool,
    dirty: bool,
}

impl App {
    /// An empty UI, waiting for the core's snapshot.
    pub fn new(tz: TimeZone) -> Self {
        Self {
            snap: Snapshot::default(),
            theme: Theme::default(),
            tz,
            now_ms: 0,
            keymap: Keymap::default_map(),
            resolver: Resolver::default(),
            mode: Mode::Normal,
            regions: Regions::default(),
            buffers: Buffers::new(),
            input: None,
            confirm: None,
            note: None,
            outbox: Vec::new(),
            resyncing: false,
            quit: false,
            dirty: true,
        }
    }

    /// The UI's copy of the core's state.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snap
    }

    /// The current mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The focused region.
    pub fn focus(&self) -> RegionId {
        self.regions.focus()
    }

    /// The status-bar note, if one is showing.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The agent whose conversation is open.
    pub fn open_agent(&self) -> Option<&AgentId> {
        self.buffers.conversation.agent()
    }

    /// Whether the user asked to quit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Sets the UI's clock (Unix milliseconds). The UI owns "now"; the core's clock is not used.
    pub fn set_now(&mut self, now_ms: i64) {
        self.now_ms = now_ms;
    }

    /// Whether the screen needs drawing.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The core's channel closed; the UI stays up so the user can read it and quit.
    pub fn on_core_closed(&mut self) {
        self.dirty = true;
        self.note = Some("core disconnected; :q to quit".into());
    }

    /// Clears the dirty flag after a draw.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Commands waiting to go to the core.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    /// When the unfinished key sequence gives up, on the loop's monotonic clock. `None` when
    /// nothing is pending, so an idle UI has no deadline and sleeps.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.resolver.deadline_ms()
    }

    /// The loop's deadline passed: run an ambiguous shorter binding or drop the sequence.
    pub fn on_timeout(&mut self, now_ms: u64) {
        if self.resolver.deadline_ms().is_some_and(|d| d <= now_ms) {
            self.dirty = true;
            if let Some(action) = self.resolver.on_timeout() {
                self.run(action);
            }
        }
    }

    /// The terminal was resized.
    pub fn on_resize(&mut self) {
        self.dirty = true;
    }

    // --- messages from the core ---

    /// Folds one core message into the UI's state.
    pub fn on_server(&mut self, msg: ServerMsg) {
        self.dirty = true;
        match msg {
            ServerMsg::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    tracing::warn!(protocol, "core speaks another protocol version");
                    self.note = Some(format!(
                        "core speaks protocol {protocol}, this UI speaks {PROTOCOL_VERSION}"
                    ));
                }
            }
            ServerMsg::Snapshot(snapshot) => {
                self.now_ms = self.now_ms.max(snapshot.now_ms);
                self.snap = snapshot;
                self.resyncing = false;
            }
            ServerMsg::Event { seq, event } => {
                let failure = match &event {
                    Event::CommandFailed { message } => Some(message.clone()),
                    _ => None,
                };
                match apply(&mut self.snap, seq, event) {
                    ApplyResult::Applied => {
                        if let Some(message) = failure {
                            tracing::info!(%message, "command failed");
                            self.note = Some(message);
                        }
                    }
                    ApplyResult::Gap { expected, got } => {
                        tracing::warn!(expected, got, "event gap; asking for a resync");
                        if !self.resyncing {
                            self.resyncing = true;
                            self.outbox.push(Command::Resync);
                        }
                    }
                    ApplyResult::Stale => {}
                }
            }
        }
        self.settle();
    }

    /// Keeps the open conversation pointing at a live agent, and clears the finished turn of an
    /// agent the user is reading (ADR 0001).
    fn settle(&mut self) {
        let shown_exists = self
            .buffers
            .conversation
            .agent()
            .is_some_and(|id| self.snap.agents.iter().any(|a| &a.id == id));
        if !shown_exists {
            let fallback = self
                .snap
                .agents
                .iter()
                .find(|a| a.kind == AgentKind::Coordinator)
                .or_else(|| self.snap.agents.iter().find(|a| a.kind.is_session()));
            if let Some(agent) = fallback {
                self.buffers.conversation.show(agent.id.clone());
            }
        }
        if self.regions.focus() == RegionId::Main
            && let Some(id) = self.buffers.conversation.agent().cloned()
        {
            self.clear_finished_turn(&id);
        }
    }

    fn clear_finished_turn(&mut self, agent: &AgentId) {
        let pending = self
            .snap
            .inbox
            .iter()
            .find(|i| &i.agent == agent && matches!(i.kind, InboxKind::FinishedTurn { .. }));
        if let Some(item) = pending {
            let command = Command::DismissInbox { item: item.id };
            if !self.outbox.contains(&command) {
                self.outbox.push(command);
            }
        }
    }

    // --- keys ---

    /// Handles a key press. `mono_ms` is a monotonic clock for key-sequence timeouts.
    pub fn on_key(&mut self, event: KeyEvent, mono_ms: u64) {
        self.dirty = true;
        self.note = None;
        if let Some(confirm) = self.confirm.take() {
            if matches!(event.code, crossterm::event::KeyCode::Char('y' | 'Y')) {
                self.run(confirm.action);
            }
            return;
        }
        match self.mode {
            Mode::Normal => self.normal_key(event, mono_ms),
            Mode::Insert | Mode::Command => self.input_key(event),
        }
    }

    fn normal_key(&mut self, event: KeyEvent, mono_ms: u64) {
        let scopes = [Scope::Buffer(self.regions.focused_buffer()), Scope::Global];
        let resolved = self.resolver.feed(
            &self.keymap,
            Mode::Normal,
            &scopes,
            Key::from_event(event),
            mono_ms,
        );
        if let Resolved::Action(action) = resolved {
            self.run(action);
        }
    }

    fn input_key(&mut self, event: KeyEvent) {
        let Some(input) = self.input.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match input.line.handle(event) {
            InputOutcome::Submit => {
                if let Some(input) = self.input.take() {
                    self.mode = Mode::Normal;
                    self.submit(input.target, input.line.text().trim().to_string());
                }
            }
            InputOutcome::Cancel => {
                self.input = None;
                self.mode = Mode::Normal;
            }
            InputOutcome::Edited | InputOutcome::Ignored => {}
        }
    }

    fn start_input(&mut self, target: InputTarget, initial: &str) {
        self.mode = if target == InputTarget::Command {
            Mode::Command
        } else {
            Mode::Insert
        };
        self.resolver.reset();
        self.input = Some(Input {
            target,
            line: LineInput::with_text(initial),
        });
    }

    fn submit(&mut self, target: InputTarget, text: String) {
        match target {
            InputTarget::Command => match command::parse(&text) {
                Ok(parsed) => self.run(parsed.action),
                Err(message) => self.note = Some(message),
            },
            InputTarget::Prompt(agent) if !text.is_empty() => {
                self.outbox.push(Command::Prompt { agent, text });
            }
            InputTarget::Reason(item) => {
                let message = (!text.is_empty()).then_some(text);
                self.outbox.push(Command::AnswerPermission {
                    item,
                    answer: PermissionAnswer::Deny { message },
                });
            }
            InputTarget::Reply(item) if !text.is_empty() => {
                self.outbox.push(Command::AnswerQuestion { item, text });
            }
            InputTarget::Title(id) if !text.is_empty() => self.outbox.push(Command::UpdateTask {
                id,
                patch: vec![TaskChange::Title(text)],
            }),
            InputTarget::NewTask if !text.is_empty() => {
                self.outbox.push(Command::CreateTask(NewTask {
                    title: text,
                    p_level: None,
                    bucket: None,
                    notes: String::new(),
                    follow_up_of: None,
                }));
            }
            _ => {}
        }
    }

    /// Runs an action: global ones here, the rest in the focused buffer.
    pub fn run(&mut self, action: Action) {
        self.dirty = true;
        match action {
            Action::FocusLeft => self.regions.focus_left(),
            Action::FocusRight => self.regions.focus_right(),
            Action::FocusDown => self.regions.focus_down(),
            Action::FocusUp => self.regions.focus_up(),
            Action::FocusInbox => self.regions.set_focus(RegionId::LeftBottom),
            Action::EnterCommand => self.start_input(InputTarget::Command, ""),
            Action::Quit => {
                let running = self
                    .snap
                    .agents
                    .iter()
                    .any(|a| a.kind.is_session() && a.status == AgentStatus::Working);
                if running {
                    self.confirm = Some(Confirm {
                        prompt: "A turn is running. Quit anyway?".into(),
                        action: Action::ForceQuit,
                    });
                } else {
                    self.quit = true;
                }
            }
            Action::ForceQuit => self.quit = true,
            #[cfg(debug_assertions)]
            Action::DebugPanic => panic!("deliberate panic from :panic"),
            _ => {
                let kind = self.regions.focused_buffer();
                let cx = ViewCx {
                    snap: &self.snap,
                    theme: &self.theme,
                    now_ms: self.now_ms,
                    tz: &self.tz,
                };
                let mut effects = Vec::new();
                self.buffers
                    .get_mut(kind)
                    .on_action(action, &cx, &mut effects);
                for effect in effects {
                    self.apply_effect(effect);
                }
            }
        }
        // Focusing the conversation of an agent counts as opening it (ADR 0001).
        self.settle();
    }

    fn apply_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Command(command) => self.outbox.push(command),
            Effect::OpenAgent(id) => self.open_agent_in_main(id),
            Effect::OpenTask(_) => self.note = Some("task detail arrives in M4".into()),
            Effect::StartInput { target, initial } => self.start_input(target, &initial),
            Effect::Note(text) => self.note = Some(text),
        }
    }

    fn open_agent_in_main(&mut self, id: AgentId) {
        self.clear_finished_turn(&id);
        self.buffers.conversation.show(id);
        self.regions.show(RegionId::Main, BufferKind::Conversation);
        self.regions.set_focus(RegionId::Main);
    }

    // --- drawing ---

    /// Draws the whole screen.
    pub fn draw(&mut self, frame: &mut Frame) {
        let prompting = self.input.is_some() || self.confirm.is_some();
        let areas = layout(frame.area(), prompting);
        let cx = ViewCx {
            snap: &self.snap,
            theme: &self.theme,
            now_ms: self.now_ms,
            tz: &self.tz,
        };
        for (region, area) in [
            (RegionId::LeftTop, areas.left_top),
            (RegionId::LeftBottom, areas.left_bottom),
            (RegionId::Main, areas.main),
        ] {
            let focused = self.regions.focus() == region;
            self.buffers
                .get_mut(self.regions.buffer(region))
                .render(&cx, frame, area, focused);
        }
        self.draw_status(frame, areas.status);
        if let Some(area) = areas.prompt {
            self.draw_prompt(frame, area);
        }
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
                format!(" {} ", self.mode.label()),
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
        if !self.resolver.pending().is_empty() {
            left.push(Span::styled(
                format!("  {} …", pending_label(self.resolver.pending())),
                theme.fg(theme.italic),
            ));
        }
        if let Some(warning) = snap.warnings.first() {
            left.push(Span::styled(
                format!("  ⚠ {}", warning.text),
                theme.fg(theme.needs_you),
            ));
        }
        let right = match &self.note {
            Some(note) => Span::styled(note.clone(), theme.fg(theme.needs_you)),
            None => Span::styled(HINTS, theme.dim()),
        };
        frame.render_widget(
            Paragraph::new(spread(left, vec![right], usize::from(area.width))),
            area,
        );
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
            InputTarget::NewTask => "new task › ".into(),
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

/// Keys of an unfinished sequence, for the status bar.
fn pending_label(keys: &[Key]) -> String {
    keys.iter().map(Key::label).collect()
}
