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
    AgentId, AgentKind, AgentStatus, ApplyResult, Command, EntryKind, Event, InboxKind,
    PROTOCOL_VERSION, PermissionAnswer, ServerMsg, SlashCommand, Snapshot, TaskChange, apply,
};
use jiff::tz::TimeZone;

use crate::action::Action;
use crate::buffers::{Buffers, Effect, InputTarget, MapReaction, ViewCx};
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

/// What `y` (or `n`) does in a confirmation.
enum Then {
    /// Run an action.
    Action(Action),
    /// Send commands to the core.
    Commands(Vec<Command>),
    /// Close the day ([`Command::EndOfDay`]), then quit once the journal is written.
    CloseDayThenQuit,
    /// Quit without closing the day, still asking first when a turn is running.
    QuitDayOpen,
    /// Lend the terminal for a command ([`App::take_handoff`]).
    Handoff(Command),
}

/// A yes/no question waiting for `y` or `n`. Any other key cancels it.
struct Confirm {
    prompt: String,
    then: Then,
    /// What `n` does; `None` when `n` only cancels.
    otherwise: Option<Then>,
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
    /// The task detail was opened from the board, so `Esc` goes back to the board.
    detail_from_board: bool,
    /// A task saved with `Tab` in quick capture opens its detail once the core announces it.
    open_after_create: Option<String>,
    note: Option<String>,
    outbox: Vec<Command>,
    /// A command that needs the terminal ([`Command::OpenEditor`], [`Command::TakeOver`]), for
    /// the loop to send once it has let go of the terminal.
    handoff: Option<Command>,
    /// The terminal is lent out until the core sends [`Event::TerminalReturned`].
    terminal_away: bool,
    /// A resync was requested and its snapshot has not arrived yet.
    resyncing: bool,
    /// The agent whose conversation had focus at the last settle, so focusing it counts as
    /// opening it once, not on every event while it stays focused.
    reading: Option<AgentId>,
    quit: bool,
    /// Ctrl-C was pressed once: the loop-clock time (ms) at which the hint expires.
    exit_armed_until: Option<u64>,
    /// A side chat was asked for: open it when it appears.
    open_side_chat: bool,
    /// Focus mode (`<leader>z`): the side column is hidden.
    focus_mode: bool,
    /// Quit once the day's journal is written: set by "Close the day?" `y`, holding the
    /// newest journal date from before, so a write that failed is told from one that worked.
    quit_after_close: Option<Option<String>>,
    /// A command failure seen while that quit waits, to show if the journal is not written.
    close_failure: Option<String>,
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
            detail_from_board: false,
            open_after_create: None,
            note: None,
            outbox: Vec::new(),
            handoff: None,
            terminal_away: false,
            resyncing: false,
            reading: None,
            quit: false,
            exit_armed_until: None,
            open_side_chat: false,
            focus_mode: false,
            quit_after_close: None,
            close_failure: None,
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
        // Nobody is left to hand the terminal back.
        self.terminal_away = false;
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

    /// A command that must run with the terminal lent out (`e` in the worktree view,
    /// `<leader>st`). The loop
    /// lets go of the terminal (alternate screen, raw mode, key reading) and then sends it; from
    /// here until [`Event::TerminalReturned`] arrives, [`App::terminal_away`] is true.
    pub fn take_handoff(&mut self) -> Option<Command> {
        let command = self.handoff.take();
        if command.is_some() {
            self.terminal_away = true;
        }
        command
    }

    /// The hand-off could not start (its path is gone), so the terminal was never lent out:
    /// shows why and carries on.
    pub fn handoff_refused(&mut self, why: String) {
        self.terminal_away = false;
        self.note = Some(why);
        self.dirty = true;
    }

    /// Whether the terminal is lent out: the loop neither draws nor reads keys until it returns.
    pub fn terminal_away(&self) -> bool {
        self.terminal_away
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
            // The editor has the terminal: Ctrl-C there is the editor's, not a request to quit.
            Signal::Interrupt if self.terminal_away => {}
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
                match self.buffers.map_outline.on_event(&event) {
                    Some(MapReaction::Show) => {
                        self.regions.show(RegionId::Main, BufferKind::MapOutline);
                        self.regions.set_focus(RegionId::Main);
                        self.note = None;
                    }
                    Some(MapReaction::Request(command)) => self.outbox.push(command),
                    None => {}
                }
                if let Some(note) = self.buffers.worktrees.on_event(&event) {
                    self.note = Some(note);
                }
                if let Event::TerminalReturned { error } = &event {
                    self.terminal_away = false;
                    if let Some(error) = error {
                        self.note = Some(error.clone());
                    }
                }
                if let Event::PathsListed { agent, dir, paths } = &event {
                    let commands = commands_for(&self.snap, agent);
                    self.buffers.conversation.receive_paths(
                        agent,
                        dir.clone(),
                        paths.clone(),
                        commands,
                    );
                    if let Some((agent, dir)) = self.buffers.conversation.take_path_request() {
                        self.outbox.push(Command::ListPaths { agent, dir });
                    }
                }
                if let Event::ClipboardPasted { agent, paste } = &event
                    && let Some(notice) = self
                        .buffers
                        .conversation
                        .receive_paste(agent, paste.clone())
                {
                    self.note = Some(notice);
                }
                let side_chat = match &event {
                    Event::AgentUpserted(agent)
                        if self.open_side_chat
                            && agent.kind == AgentKind::SideChat
                            && !self.snap.agents.iter().any(|a| a.id == agent.id) =>
                    {
                        Some(agent.id.clone())
                    }
                    _ => None,
                };
                let created = match &event {
                    Event::TaskUpserted(task)
                        if self.open_after_create.as_deref() == Some(task.title.as_str())
                            && !self.snap.tasks.iter().any(|t| t.id == task.id) =>
                    {
                        Some(task.id)
                    }
                    _ => None,
                };
                let journal = matches!(event, Event::JournalChanged(_));
                // Every agent is idle and the user said so (ADR 0014): quit without asking, so
                // the binary can swap the new build in and start it.
                if matches!(event, Event::UpgradeRestarting) {
                    self.quit = true;
                }
                match apply(&mut self.snap, seq, event) {
                    ApplyResult::Applied => {
                        if let Some(message) = failure {
                            tracing::info!(%message, "command failed");
                            if self.quit_after_close.is_some() {
                                self.close_failure = Some(message.clone());
                            }
                            self.note = Some(message);
                            // A failed save must not leave a later task of the same title to
                            // steal the screen.
                            self.open_after_create = None;
                            self.open_side_chat = false;
                        }
                        if let Some(id) = created {
                            self.open_after_create = None;
                            self.open_task(id, false);
                        }
                        if let Some(id) = side_chat {
                            self.open_side_chat = false;
                            self.open_agent_in_main(id);
                        }
                        if journal {
                            self.on_journal();
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
            use crossterm::event::KeyCode::Char;
            let then = match event.code {
                Char('y' | 'Y') => Some(confirm.then),
                Char('n' | 'N') => confirm.otherwise,
                _ => None,
            };
            match then {
                Some(Then::Action(action)) => self.run(action),
                Some(Then::Commands(commands)) => self.outbox.extend(commands),
                Some(Then::Handoff(command)) => self.handoff = Some(command),
                Some(Then::CloseDayThenQuit) => self.close_day(true),
                Some(Then::QuitDayOpen) => self.quit_asking(false),
                None => {}
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
                Some(Action::LeaveChatBar) => {
                    self.buffers.conversation.park_chat();
                    self.mode = Mode::Normal;
                    return;
                }
                Some(action @ (Action::PageDown | Action::PageUp)) => {
                    self.run(action);
                    return;
                }
                _ => {}
            }
            let commands = self
                .buffers
                .conversation
                .agent()
                .map_or(&[][..], |a| commands_for(&self.snap, a));
            if let Some((agent, text, attachments)) =
                self.buffers.conversation.chat_key(event, commands)
            {
                self.outbox.push(Command::Prompt {
                    agent,
                    text,
                    attachments,
                });
            }
            if let Some((agent, dir)) = self.buffers.conversation.take_path_request() {
                self.outbox.push(Command::ListPaths { agent, dir });
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
            InputTarget::SideChatQuestion if !text.is_empty() => self.new_side_chat(text),
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

    /// Asks the core for a side chat; the first one to appear afterwards opens in the main
    /// region.
    fn new_side_chat(&mut self, question: String) {
        self.open_side_chat = true;
        self.outbox.push(Command::NewSideChat { question });
    }

    /// `<leader>y`: the shown agent's last answer goes into the coordinator's chat bar, after
    /// anything already typed there (Decision 18).
    fn yank_to_coordinator(&mut self) {
        let (Some(shown), Some(coordinator)) = (
            self.buffers.conversation.agent().cloned(),
            self.coordinator(),
        ) else {
            return;
        };
        if shown == coordinator {
            self.note = Some("<leader>y copies another agent's answer to the coordinator".into());
            return;
        }
        let answer = self
            .snap
            .conversations
            .get(&shown)
            .and_then(|t| {
                t.entries
                    .iter()
                    .rev()
                    .find(|e| e.kind == EntryKind::Assistant)
            })
            .map(|e| e.text.trim().to_string());
        let Some(answer) = answer.filter(|a| !a.is_empty()) else {
            self.note = Some("no answer to copy yet".into());
            return;
        };
        self.start_chat(coordinator);
        self.buffers.conversation.insert_text(&answer);
    }

    fn new_slice(&mut self, name: String, brief: String) {
        let task = self.focused_task();
        self.outbox.push(Command::NewSlice { task, name, brief });
    }

    /// Today's date on the UI's clock, `YYYY-MM-DD`, as journal files are named.
    fn today(&self) -> Option<String> {
        let now = jiff::Timestamp::from_millisecond(self.now_ms).ok()?;
        Some(now.to_zoned(self.tz.clone()).date().to_string())
    }

    /// Whether today has no journal file yet, and none is being written (ADR 0012). A folder
    /// with no journal (not a registered project) never counts as open.
    fn day_open(&self) -> bool {
        let Some(journal) = &self.snap.journal else {
            return false;
        };
        match self.today() {
            Some(today) => !journal.closing && journal.latest.as_deref() != Some(today.as_str()),
            None => false,
        }
    }

    /// `:q`: asks "Close the day?" first when `ask_day` and today has no journal yet; then, as
    /// before, asks when a turn is running (plan B23).
    fn quit_asking(&mut self, ask_day: bool) {
        if ask_day && self.day_open() {
            self.confirm = Some(Confirm {
                prompt: "Close the day? y: handoff and journal, then quit · n: quit now · \
                         Esc: later"
                    .into(),
                then: Then::CloseDayThenQuit,
                otherwise: Some(Then::QuitDayOpen),
            });
            return;
        }
        let running = self
            .snap
            .agents
            .iter()
            .any(|a| a.kind.is_session() && a.status == AgentStatus::Working);
        if running {
            self.confirm = Some(Confirm {
                prompt: "A turn is running. Quit anyway?".into(),
                then: Then::Action(Action::ForceQuit),
                otherwise: None,
            });
        } else {
            self.quit = true;
        }
    }

    /// `:eod`, or `y` on "Close the day?" (`quit`: quit once the journal is written).
    fn close_day(&mut self, quit: bool) {
        let Some(journal) = &self.snap.journal else {
            self.note = Some("not a Dovetail project yet, so it has no journal".into());
            return;
        };
        if journal.closing {
            self.note = Some("already closing the day".into());
            if quit {
                self.quit_after_close = Some(journal.latest.clone());
            }
            return;
        }
        if quit {
            self.quit_after_close = Some(journal.latest.clone());
            self.close_failure = None;
        }
        self.outbox.push(Command::EndOfDay);
        self.note = Some(if quit {
            "closing the day: Dovetail quits once the coordinator's handoff is saved".into()
        } else {
            "closing the day: the journal is written when the coordinator's handoff arrives".into()
        });
    }

    /// The journal changed: if a quit waits on it and the day's file is written, quit.
    fn on_journal(&mut self) {
        let Some(journal) = &self.snap.journal else {
            return;
        };
        if journal.closing {
            return;
        }
        let Some(before) = self.quit_after_close.take() else {
            return;
        };
        if journal.latest.is_some() && journal.latest != before {
            self.quit = true;
        } else {
            // The core's reason (a failed write), when it gave one.
            let why = self
                .close_failure
                .take()
                .unwrap_or_else(|| "the journal was not written".into());
            self.note = Some(format!("{why}; :q to quit anyway"));
        }
    }

    /// Focus mode on or off (A1, Decision 16): the conversation alone, full width; the status
    /// bar keeps the Needs you count. The background stays as it is (ADR 0013). It shows the
    /// agent already in view, so a side chat or slice can be read the same way.
    pub(super) fn toggle_focus_mode(&mut self) {
        self.focus_mode = !self.focus_mode;
        if self.focus_mode {
            if self.regions.buffer(RegionId::Main) != BufferKind::Conversation {
                self.regions.show(RegionId::Main, BufferKind::Conversation);
            }
            self.regions.set_focus(RegionId::Main);
        }
    }

    /// Whether focus mode hides the side column.
    pub fn focus_mode(&self) -> bool {
        self.focus_mode
    }

    /// Runs an action: global ones here, the rest in the focused buffer.
    pub fn run(&mut self, action: Action) {
        self.dirty = true;
        // Going to the side column leaves focus mode, so focus never lands on a hidden region.
        if self.focus_mode
            && matches!(
                action,
                Action::FocusRight | Action::FocusInbox | Action::FocusUp | Action::FocusDown
            )
        {
            self.toggle_focus_mode();
        }
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
            Action::ToggleFocusMode => self.toggle_focus_mode(),
            Action::PasteClipboard => self.paste_clipboard(),
            Action::QuickCapture => self.open_capture(),
            Action::PickAgent => self.open_switcher(),
            Action::PickCommand => self.open_command_picker(),
            Action::PickAny => self.open_finder(),
            Action::Search => self.start_input(InputTarget::Search, ""),
            Action::SearchNext => self.search_jump(true),
            Action::SearchPrev => self.search_jump(false),
            Action::ShowActivity => self.toggle_main(BufferKind::Activity),
            Action::ShowMessages => self.toggle_main(BufferKind::Messages),
            Action::ShowBoard => self.toggle_main(BufferKind::Board),
            Action::ShowMaps => self.toggle_main(BufferKind::MapShelf),
            Action::ShowWorktrees => {
                let opening = self.regions.buffer(RegionId::Main) != BufferKind::Worktrees;
                self.toggle_main(BufferKind::Worktrees);
                if opening {
                    self.buffers.worktrees.begin_refresh(&self.snap);
                    self.outbox.push(Command::RefreshWorktrees);
                }
            }
            Action::BackToConversation => {
                // Leaving an inbox item goes back to the inbox, for the next one.
                let main = self.regions.buffer(RegionId::Main);
                if main == BufferKind::InboxDetail {
                    self.focus_inbox();
                }
                // A task opened from the board goes back to the board.
                let back = if main == BufferKind::TaskDetail && self.detail_from_board {
                    BufferKind::Board
                } else if main == BufferKind::MapOutline {
                    // A map opened from the shelf goes back to the shelf.
                    BufferKind::MapShelf
                } else {
                    BufferKind::Conversation
                };
                self.regions.show(RegionId::Main, back);
            }
            Action::NewSlice => self.run_with_arg(Action::NewSlice, ""),
            Action::NewSideChat => self.run_with_arg(Action::NewSideChat, ""),
            Action::YankToCoordinator => self.yank_to_coordinator(),
            Action::Interrupt => self.run_with_arg(Action::Interrupt, ""),
            Action::CyclePermissionMode => self.cycle_permission_mode(),
            Action::TakeOver => self.take_over(),
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
            Action::Quit => self.quit_asking(true),
            Action::EndOfDay => self.close_day(false),
            Action::Upgrade => self.run_with_arg(Action::Upgrade, ""),
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
            // Deliberate debug-build exception to "no panic outside tests": `:panic` exercises the panic hook.
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
            Effect::OpenTask(id) => {
                // Effects come from the focused buffer.
                let from_board = self.regions.focused_buffer() == BufferKind::Board;
                self.open_task(id, from_board);
            }
            Effect::OpenInboxItem(id) => self.open_inbox_item(id),
            Effect::OpenMap(id) => {
                self.buffers.map_outline.await_map(id.clone());
                self.outbox.push(Command::LoadMap { id, version: None });
                self.note = Some("loading map…".into());
            }
            Effect::StartInput { target, initial } => self.start_input(target, &initial),
            Effect::PickRelated(task) => self.open_relate_picker(task),
            Effect::Note(text) => self.note = Some(text),
            Effect::OpenEditor(path) => self.handoff = Some(Command::OpenEditor { path }),
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

/// The slash commands `agent` offers: its own list, else another session's (one project's
/// sessions share a profile, so a slice not yet started borrows the coordinator's).
pub(crate) fn commands_for<'a>(snap: &'a Snapshot, agent: &AgentId) -> &'a [SlashCommand] {
    snap.commands
        .get(agent)
        .or_else(|| snap.commands.values().find(|c| !c.is_empty()))
        .map_or(&[], Vec::as_slice)
}
