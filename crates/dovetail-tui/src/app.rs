//! The application state: snapshot, mode, regions, buffers and the glue between keys, buffers
//! and core commands. It is plain state with no terminal or thread in it, so tests drive it with
//! key events and read what it would draw or send.

/// A `ViewCx` over the app's fields, written as a macro so the buffers can be borrowed mutably
/// beside it (a method would borrow all of `self`).
macro_rules! cx {
    ($app:expr) => {
        $crate::buffers::ViewCx {
            snap: &$app.snap,
            theme: &$app.theme,
            now_ms: $app.now_ms,
            tz: &$app.tz,
        }
    };
}

mod draw;
mod focus;
mod overlay;
mod search;

use crossterm::event::KeyEvent;
use dovetail_api::{
    AgentId, AgentKind, AgentStatus, ApplyResult, Command, Event, InboxKind, PROTOCOL_VERSION,
    PermissionAnswer, ServerMsg, Snapshot, TaskChange, apply,
};
use jiff::tz::TimeZone;

use crate::action::Action;
use crate::buffers::{Buffers, Effect, InputTarget, ViewCx};
use crate::command;
use crate::input::{InputOutcome, TextInput};
use crate::keymap::{Key, Keymap, Resolved, Resolver, Scope};
use crate::mode::Mode;
use crate::region::{BufferKind, RegionId, Regions};
use crate::theme::Theme;
use overlay::Overlay;

/// How long the "Press Ctrl-C again to exit" hint waits for the second press.
pub const EXIT_CONFIRM_MS: u64 = 2000;

/// Hints on the right of the status bar.
const HINTS: &str = "i chat · ␣ss switch · ␣t capture · / search · ␣n inbox · : command";
const HINTS_SHORT: &str = "i chat · ␣ss switch · : command";

/// A signal the event loop caught.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGINT (`kill -INT`; in raw mode Ctrl-C arrives as a key instead).
    Interrupt,
    /// SIGTERM or SIGHUP: nobody is there to answer a question.
    Terminate,
}

/// An open prompt row.
struct Input {
    target: InputTarget,
    line: TextInput,
}

/// What `y` does in a confirmation.
enum Then {
    /// Run an action.
    Action(Action),
    /// Send commands to the core.
    Commands(Vec<Command>),
}

/// A yes/no question waiting for `y` or `n`.
struct Confirm {
    prompt: String,
    then: Then,
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
    /// A picker or the quick-capture box floating over the layout.
    overlay: Option<Overlay>,
    /// The last in-buffer search term, for `n` and `N`.
    search: Option<String>,
    /// A task saved with `Tab` in quick capture opens its detail once the core announces it.
    open_after_create: Option<String>,
    note: Option<String>,
    outbox: Vec<Command>,
    /// A resync was requested and its snapshot has not arrived yet.
    resyncing: bool,
    /// The agent whose conversation had focus at the last settle, so focusing it counts as
    /// opening it once, not on every event while it stays focused.
    reading: Option<AgentId>,
    quit: bool,
    /// Ctrl-C was pressed once: the loop-clock time (ms) at which the hint expires.
    exit_armed_until: Option<u64>,
    /// The key being handled is a second Ctrl-C inside the hint's window.
    exit_armed: bool,
    /// The loop clock at the key being handled, for actions that arm a timeout.
    mono_ms: u64,
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
            overlay: None,
            search: None,
            open_after_create: None,
            note: None,
            outbox: Vec::new(),
            resyncing: false,
            reading: None,
            quit: false,
            exit_armed_until: None,
            exit_armed: false,
            mono_ms: 0,
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
        // A reset time passing changes the header with no event from the core.
        if self.next_clock_change_ms().is_some_and(|d| d <= now_ms) {
            self.dirty = true;
        }
        self.now_ms = now_ms;
    }

    /// When the screen next changes with the wall clock alone (a usage-limit reset passing),
    /// in Unix ms. `None` when nothing is pending, so an idle UI sleeps.
    pub fn next_clock_change_ms(&self) -> Option<i64> {
        crate::usage::next_change_ms(&self.snap.usage, self.now_ms)
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
        match (self.resolver.deadline_ms(), self.exit_armed_until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The loop's deadline passed: run an ambiguous shorter binding or drop the sequence.
    pub fn on_timeout(&mut self, now_ms: u64) {
        if self.exit_armed_until.is_some_and(|d| d <= now_ms) {
            self.exit_armed_until = None;
            self.note = None;
            self.dirty = true;
        }
        if self.resolver.deadline_ms().is_some_and(|d| d <= now_ms) {
            self.dirty = true;
            if let Some(action) = self.resolver.on_timeout() {
                self.run(action);
            }
        }
    }

    /// A signal arrived. SIGINT takes the same path as `:q` ([`Action::Quit`]): it asks first
    /// when a turn is running, and a second SIGINT while that question is open answers yes.
    /// SIGTERM and SIGHUP come from something that cannot answer (a closed terminal, the end of
    /// the login session), so they quit as a confirmed quit does ([`Action::ForceQuit`]).
    pub fn on_signal(&mut self, signal: Signal) {
        self.dirty = true;
        match signal {
            Signal::Interrupt if self.confirm.is_some() => {
                self.confirm = None;
                self.run(Action::ForceQuit);
            }
            Signal::Interrupt => self.run(Action::Quit),
            Signal::Terminate => self.run(Action::ForceQuit),
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
                if let Event::ClipboardPasted { agent, paste } = &event
                    && let Some(notice) = self
                        .buffers
                        .conversation
                        .receive_paste(agent, paste.clone())
                {
                    self.note = Some(notice);
                }
                let created = match &event {
                    Event::TaskUpserted(task)
                        if self.open_after_create.as_deref() == Some(task.title.as_str())
                            && !self.snap.tasks.iter().any(|t| t.id == task.id) =>
                    {
                        Some(task.id)
                    }
                    _ => None,
                };
                match apply(&mut self.snap, seq, event) {
                    ApplyResult::Applied => {
                        if let Some(message) = failure {
                            tracing::info!(%message, "command failed");
                            self.note = Some(message);
                            // A failed save must not leave a later task of the same title to
                            // steal the screen.
                            self.open_after_create = None;
                        }
                        if let Some(id) = created {
                            self.open_after_create = None;
                            self.open_task(id);
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

    /// Keeps the open conversation pointing at a live agent, and clears an agent's finished turn
    /// when the user moves focus onto its conversation (ADR 0001: opening it). A turn that
    /// finishes while the conversation already has focus stays in Needs you (Decision 7a; live
    /// check 2026-11-06).
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
        let reading = (self.regions.focus() == RegionId::Main
            && self.regions.buffer(RegionId::Main) == BufferKind::Conversation)
            .then(|| self.buffers.conversation.agent().cloned())
            .flatten();
        if reading != self.reading
            && let Some(id) = &reading
        {
            self.clear_finished_turn(id);
        }
        self.reading = reading;
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
        self.mono_ms = mono_ms;
        self.exit_armed = self.exit_armed_until.take().is_some_and(|d| d > mono_ms);
        if let Some(confirm) = self.confirm.take() {
            if matches!(event.code, crossterm::event::KeyCode::Char('y' | 'Y')) {
                match confirm.then {
                    Then::Action(action) => self.run(action),
                    Then::Commands(commands) => self.outbox.extend(commands),
                }
            }
            return;
        }
        // Ctrl-C works in every mode and over pickers, mid-sequence too, so it is looked up
        // before them; it runs through `run` like any other action.
        if self.keymap.global_chord(self.mode, Key::from_event(event)) == Some(Action::QuitOnRepeat)
        {
            self.resolver.reset();
            self.run(Action::QuitOnRepeat);
            return;
        }
        if self.overlay.is_some() {
            self.overlay_key(event);
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
        if self.input.is_none() {
            if !self.buffers.conversation.is_chatting() {
                self.mode = Mode::Normal;
                return;
            }
            let chord = self.keymap.text_chord(
                Mode::Insert,
                BufferKind::Conversation,
                Key::from_event(event),
            );
            match chord {
                Some(Action::PasteClipboard) => {
                    self.paste_clipboard();
                    return;
                }
                Some(Action::DropLastAttachment)
                    if self.buffers.conversation.drop_last_attachment() =>
                {
                    return;
                }
                _ => {}
            }
            if let Some((agent, text, attachments)) = self.buffers.conversation.chat_key(event) {
                self.outbox.push(Command::Prompt {
                    agent,
                    text,
                    attachments,
                });
            }
            if !self.buffers.conversation.is_chatting() {
                self.mode = Mode::Normal;
            }
            return;
        }
        let Some(input) = self.input.as_mut() else {
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

    /// Ctrl-V in the chat bar: ask the core to read the clipboard. It answers with
    /// [`Event::ClipboardPasted`]: an image saved as an attachment, text, or a refusal.
    fn paste_clipboard(&mut self) {
        if !self.buffers.conversation.is_chatting() {
            return;
        }
        if let Some(agent) = self.buffers.conversation.agent().cloned() {
            self.outbox.push(Command::PasteClipboard { agent });
        }
    }

    fn start_input(&mut self, target: InputTarget, initial: &str) {
        self.resolver.reset();
        // Chat is typed inside the conversation pane, not on the prompt row.
        if let InputTarget::Prompt(agent) = target {
            self.start_chat(agent);
            return;
        }
        self.mode = if target == InputTarget::Command {
            Mode::Command
        } else {
            Mode::Insert
        };
        self.input = Some(Input {
            target,
            line: TextInput::line(initial),
        });
    }

    fn submit(&mut self, target: InputTarget, text: String) {
        match target {
            InputTarget::Command => match command::parse(&text) {
                Ok(parsed) => self.run_with_arg(parsed.action, &parsed.arg),
                Err(e) => self.note = Some(e.to_string()),
            },
            // Chat never comes through the prompt row.
            InputTarget::Prompt(_) => {}
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
            InputTarget::SliceName if !text.is_empty() => {
                self.start_input(InputTarget::SliceBrief(text), "");
            }
            InputTarget::SliceBrief(name) => self.new_slice(name, text),
            InputTarget::Search => self.start_search(text),
            InputTarget::Decision(task) if !text.is_empty() => self.log_decision(task, text),
            _ => {}
        }
    }

    fn log_decision(&mut self, task: dovetail_api::TaskId, text: String) {
        self.outbox
            .push(Command::LogDecision(dovetail_api::NewDecision {
                task,
                text,
                why: None,
                alternatives: Vec::new(),
                supersedes: None,
            }));
    }

    fn new_slice(&mut self, name: String, brief: String) {
        let task = self.focused_task();
        self.outbox.push(Command::NewSlice { task, name, brief });
    }

    /// Runs an action: global ones here, the rest in the focused buffer.
    pub fn run(&mut self, action: Action) {
        self.dirty = true;
        match action {
            Action::FocusLeft => self.regions.focus_left(),
            Action::FocusRight => self.regions.focus_right(),
            Action::FocusDown => self.regions.focus_down(),
            Action::FocusUp => self.regions.focus_up(),
            Action::FocusInbox => self.focus_inbox(),
            Action::EnterCommand => self.start_input(InputTarget::Command, ""),
            Action::EnterInsert => {
                if let Some(agent) = self.chat_target() {
                    self.start_chat(agent);
                }
            }
            Action::FocusCoordinator => {
                if let Some(id) = self.coordinator() {
                    self.open_agent_in_main(id);
                }
            }
            Action::AlternateAgent => match self.buffers.conversation.previous().cloned() {
                Some(id) => self.open_agent_in_main(id),
                None => self.note = Some("no previous agent".into()),
            },
            Action::ToggleSolidBackground => self.buffers.conversation.toggle_solid(),
            Action::PasteClipboard => self.paste_clipboard(),
            Action::QuickCapture => self.open_capture(),
            Action::PickAgent => self.open_switcher(),
            Action::PickAny => self.open_finder(),
            Action::Search => self.start_input(InputTarget::Search, ""),
            Action::SearchNext => self.search_jump(true),
            Action::SearchPrev => self.search_jump(false),
            Action::ShowActivity => self.toggle_main(BufferKind::Activity),
            Action::ShowMessages => self.toggle_main(BufferKind::Messages),
            Action::BackToConversation => {
                // Leaving an inbox item goes back to the inbox, for the next one.
                if self.regions.buffer(RegionId::Main) == BufferKind::InboxDetail {
                    self.focus_inbox();
                }
                self.regions.show(RegionId::Main, BufferKind::Conversation);
            }
            Action::NewSlice => self.run_with_arg(Action::NewSlice, ""),
            Action::Interrupt => self.run_with_arg(Action::Interrupt, ""),
            Action::CyclePermissionMode => self.cycle_permission_mode(),
            Action::TakeOver => self.note = Some("take-over arrives in M14".into()),
            Action::RegisterProject => {
                // The open folder (ADR 0005); the core refuses any other.
                let path = self
                    .snap
                    .project
                    .as_ref()
                    .map(|p| p.repo_path.clone())
                    .unwrap_or_default();
                self.outbox.push(Command::RegisterProject { path });
            }
            Action::SwitchProject => {
                self.note =
                    Some("to switch projects, quit and open dovetail in the other folder".into());
            }
            Action::Quit => {
                let running = self
                    .snap
                    .agents
                    .iter()
                    .any(|a| a.kind.is_session() && a.status == AgentStatus::Working);
                if running {
                    self.confirm = Some(Confirm {
                        prompt: "A turn is running. Quit anyway?".into(),
                        then: Then::Action(Action::ForceQuit),
                    });
                } else {
                    self.quit = true;
                }
            }
            Action::ForceQuit => self.quit = true,
            Action::QuitOnRepeat => {
                if std::mem::take(&mut self.exit_armed) {
                    // The second press quits the way `:q` does, asking first if a turn runs.
                    self.run(Action::Quit);
                } else {
                    self.exit_armed_until = Some(self.mono_ms + EXIT_CONFIRM_MS);
                    self.note = Some("Press Ctrl-C again to exit".into());
                }
            }
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
            Effect::OpenTask(id) => self.open_task(id),
            Effect::OpenInboxItem(id) => self.open_inbox_item(id),
            Effect::StartInput { target, initial } => self.start_input(target, &initial),
            Effect::PickRelated(task) => self.open_relate_picker(task),
            Effect::Note(text) => self.note = Some(text),
        }
    }

    /// Shows an agent's conversation in the main region without moving focus. Opening an agent
    /// clears its finished turn (ADR 0001).
    fn show_agent(&mut self, id: AgentId) {
        self.clear_finished_turn(&id);
        self.buffers.conversation.show(id);
        self.regions.show(RegionId::Main, BufferKind::Conversation);
    }

    fn open_agent_in_main(&mut self, id: AgentId) {
        self.show_agent(id);
        self.regions.set_focus(RegionId::Main);
    }
}
