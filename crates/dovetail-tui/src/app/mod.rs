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
            activity: &$app.activity,
        }
    };
}

mod draw;
mod focus;
mod overlay;
mod panel;
mod projects;
mod reading;
mod search;
mod tangents;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use dovetail_api::{
    ApplyResult, Command, Event, PROTOCOL_VERSION, ServerMsg, Snapshot,
    activity::inbox::{InboxCommand, InboxKind, PermissionAnswer},
    activity::status::{StatusEvent, UsageLimits},
    agents::agent::{AgentCommand, AgentEvent, AgentId, AgentKind, AgentStatus, SlashCommand},
    agents::conversation::{ConversationCommand, ConversationEvent},
    agents::journal::{JournalCommand, JournalEvent},
    agents::mcp::McpEvent,
    agents::model::DriverKind,
    app::{AppCommand, AppEvent},
    apply,
    code::worktree::WorktreeCommand,
    research::{ResearchCommand, ResearchEvent},
    tracking::decision::DecisionCommand,
    tracking::project::{ProjectCommand, ProjectEvent},
    tracking::task::{TaskChange, TaskCommand, TaskEvent},
    visual::map::MapCommand,
};
use jiff::tz::TimeZone;

use crate::buffers::{Buffers, Effect, InputTarget, MapMouse, MapReaction, ViewCx};
use crate::canvas::kitty::Scene;
use crate::canvas::view::CellPx;
use crate::canvas::{PaintJob, Painted};
use crate::chrome::activity::{self, Activity};
use crate::input::slash;
use crate::input::{InputOutcome, TextInput};
use crate::keymap::action::Action;
use crate::keymap::command;
use crate::keymap::mode::Mode;
use crate::keymap::{Key, Keymap, Resolved, Resolver};
use crate::layout::region::{BufferKind, RegionId, Regions};
use crate::theme::Theme;
use overlay::Overlay;

/// How long the "Press Ctrl-C again to exit" hint waits for the second press.
pub const EXIT_CONFIRM_MS: u64 = 2000;

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
    /// Close the day ([`JournalCommand::EndOfDay`]), then quit once the journal is written.
    CloseDayThenQuit,
    /// Quit without closing the day, still asking first when a turn is running.
    QuitDayOpen,
    /// Lend the terminal for a command ([`App::take_handoff`]).
    Handoff(Command),
}

/// Which conversation view's chat bar is open, or was open last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ChatSite {
    /// The main region's conversation.
    #[default]
    Main,
    /// The side panel's.
    Panel,
    /// Reading mode's Ask column (ADR 0018).
    Ask,
}

/// A yes/no question waiting for `y` or `n`. Any other key cancels it.
struct Confirm {
    prompt: String,
    then: Then,
    /// What `n` does; `None` when `n` only cancels.
    otherwise: Option<Then>,
}

/// Pending handshakes and in-flight requests between the UI and the core.
#[derive(Default)]
struct Pending {
    /// A task saved with `Tab` in quick capture opens its detail once the core announces it.
    open_after_create: Option<String>,
    /// A command that needs the terminal ([`AppCommand::OpenEditor`], [`AppCommand::TakeOver`]),
    /// for the loop to send once it has let go of the terminal.
    handoff: Option<Command>,
    /// The terminal is lent out until the core sends [`AppEvent::TerminalReturned`].
    terminal_away: bool,
    /// `OpenEditor` is out and the core has not answered yet: the loop keeps the terminal and
    /// reads no key until it does ([`App::awaiting_lend`]).
    awaiting_lend: bool,
    /// The name of a research project `:project new` asked the core to make: once the snapshot
    /// lists it, the app asks whether to open it.
    new_project: Option<String>,
    /// A resync was requested and its snapshot has not arrived yet.
    resyncing: bool,
    /// Ctrl-C was pressed once: the loop-clock time (ms) at which the hint expires.
    exit_armed_until: Option<u64>,
    /// A side chat was asked for: open it in the side panel when it appears.
    open_side_chat: bool,
    /// `i` asked for a new Ask session for this task: type in it when it appears.
    ask: Option<dovetail_api::tracking::task::TaskId>,
    /// A tangent whose latest answer opens once its listing arrives (ADR 0017).
    tangent_read: Option<dovetail_api::tracking::task::TaskId>,
    /// Quit once the day's journal is written: set by "Close the day?" `y`, holding the
    /// newest journal date from before, so a write that failed is told from one that worked.
    quit_after_close: Option<Option<String>>,
    /// A command failure seen while `quit_after_close` waits, to show if the journal is not
    /// written.
    close_failure: Option<String>,
}

/// The whole UI state.
pub struct App {
    snap: Snapshot,
    theme: Theme,
    tz: TimeZone,
    now_ms: i64,
    /// When each working indicator began (the working line, the status bar's long command).
    activity: Activity,
    keymap: Keymap,
    resolver: Resolver,
    mode: Mode,
    regions: Regions,
    buffers: Buffers,
    input: Option<Input>,
    confirm: Option<Confirm>,
    /// A picker or the quick-capture box floating over the layout.
    overlay: Option<Overlay>,
    /// The chat bar the overlay was opened from (typing in it), to hand typing back to on close.
    overlay_chat: Option<overlay::ChatMark>,
    /// The last in-buffer search term, for `n` and `N`.
    search: Option<String>,
    /// The task detail was opened from the board, so `Esc` goes back to the board.
    detail_from_board: bool,
    note: Option<String>,
    outbox: Vec<Command>,
    /// Pending handshakes and in-flight requests.
    pending: Pending,
    /// The agent whose conversation had focus at the last settle, so focusing it counts as
    /// opening it once, not on every event while it stays focused.
    reading: Option<AgentId>,
    quit: bool,
    /// Whose chat bar is open (or was open last): the main region's, the side panel's or Ask's.
    chat_site: ChatSite,
    /// Focus mode (`<leader>z`): the side column is hidden.
    focus_mode: bool,
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
            activity: Activity::default(),
            keymap: Keymap::default_map(),
            resolver: Resolver::default(),
            mode: Mode::Normal,
            regions: Regions::default(),
            buffers: Buffers::new(),
            input: None,
            confirm: None,
            overlay: None,
            overlay_chat: None,
            search: None,
            detail_from_board: false,
            note: None,
            outbox: Vec::new(),
            pending: Pending::default(),
            reading: None,
            quit: false,
            chat_site: ChatSite::Main,
            focus_mode: false,
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
        // A new spinner frame is a new picture, but only where a spinner is on screen.
        if self.animating() && activity::frame_index(now_ms) != activity::frame_index(self.now_ms) {
            self.dirty = true;
        }
        self.now_ms = now_ms;
    }

    /// Whether a spinner is on screen: a long command in the status bar, or a working agent's
    /// line under a conversation or row in the tree drawn last frame. Only then does the UI animate.
    pub fn animating(&self) -> bool {
        self.activity.busy(self.now_ms).is_some()
            || self.buffers.tree.shows_working()
            || self.buffers.conversation.shows_working()
            || self.buffers.panel.conversation().shows_working()
            || self.buffers.ask.conversation().shows_working()
    }

    /// Time to the next spinner frame, for the loop's deadline; `None` when nothing animates,
    /// so an idle UI sleeps.
    pub fn next_animation_in(&self) -> Option<std::time::Duration> {
        self.animating()
            .then(|| activity::next_frame_in(self.now_ms))
    }

    /// The active usage limits to show in the status bar and watch for reset timers (task #58).
    pub fn active_usage_limits(&self) -> UsageLimits {
        let in_view = self
            .buffers
            .conversation
            .agent()
            .and_then(|id| self.snap.agents.iter().find(|a| &a.id == id));
        match in_view.map(|a| a.driver) {
            Some(DriverKind::Agy) => {
                let agy = self.snap.agy_usage.as_ref().filter(|u| !u.failed);
                let group = agy.and_then(|u| {
                    crate::chrome::usage::match_usage_group(
                        &u.groups,
                        in_view.and_then(|a| a.model.as_deref()),
                    )
                });
                group
                    .map(crate::chrome::usage::group_to_usage_limits)
                    .unwrap_or_default()
            }
            _ => self.snap.usage.clone(),
        }
    }

    /// When the screen next changes with the wall clock alone (a usage-limit reset passing),
    /// in Unix ms. `None` when nothing is pending, so an idle UI sleeps.
    pub fn next_clock_change_ms(&self) -> Option<i64> {
        let limits = self.active_usage_limits();
        crate::chrome::usage::next_change_ms(&limits, self.now_ms)
    }

    /// Whether the screen needs drawing.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The core's channel closed; the UI stays up so the user can read it and quit.
    pub fn on_core_closed(&mut self) {
        self.dirty = true;
        // Nobody is left to hand the terminal back.
        self.pending.terminal_away = false;
        self.pending.awaiting_lend = false;
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
    /// here until [`AppEvent::TerminalReturned`] arrives, [`App::terminal_away`] is true.
    pub fn take_handoff(&mut self) -> Option<Command> {
        let command = self.pending.handoff.take();
        if command.is_some() {
            self.pending.terminal_away = true;
        }
        command
    }

    /// Whether a hand-off waits to be taken ([`App::take_handoff`]).
    pub fn has_handoff(&self) -> bool {
        self.pending.handoff.is_some()
    }

    /// Whether the core has yet to say if it will take the terminal (it checks the editor's
    /// path first). The loop keeps the terminal and reads no key meanwhile; the answer is
    /// either a hand-off ([`AppEvent::TerminalLend`]) or a note ([`AppEvent::CommandFailed`]).
    pub fn awaiting_lend(&self) -> bool {
        self.pending.awaiting_lend
    }

    /// Whether the terminal is lent out: the loop neither draws nor reads keys until it returns.
    pub fn terminal_away(&self) -> bool {
        self.pending.terminal_away
    }

    /// When the unfinished key sequence gives up, on the loop's monotonic clock. `None` when
    /// nothing is pending, so an idle UI has no deadline and sleeps.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        [
            self.resolver.deadline_ms(),
            self.resolver.hint_deadline_ms(),
            self.pending.exit_armed_until,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The keys the which-key popup lists, when it is open: what can follow the pending keys in
    /// the focused buffer's scope. Empty when it is closed.
    pub fn which_key(&self) -> Vec<crate::keymap::help::WhichKeyRow> {
        if !self.resolver.hint_shown() || self.overlay.is_some() {
            return Vec::new();
        }
        crate::keymap::help::which_key(
            &self.keymap,
            self.mode,
            &self.scopes(),
            self.resolver.pending(),
        )
    }

    /// The loop's deadline passed: run an ambiguous shorter binding or drop the sequence.
    pub fn on_timeout(&mut self, now_ms: u64) {
        if self.pending.exit_armed_until.is_some_and(|d| d <= now_ms) {
            self.pending.exit_armed_until = None;
            self.note = None;
            self.dirty = true;
        }
        // One shot: the popup opens and the hint deadline is gone until the next key.
        if self
            .resolver
            .hint_deadline_ms()
            .is_some_and(|d| d <= now_ms)
        {
            self.resolver.show_hint();
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
            Signal::Interrupt if self.pending.terminal_away => {}
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
            ServerMsg::Snapshot(mut snapshot) => {
                self.now_ms = self.now_ms.max(snapshot.now_ms);
                snapshot.tasks.iter_mut().for_each(without_review);
                self.snap = snapshot;
                self.pending.resyncing = false;
                self.offer_new_project();
            }
            ServerMsg::Event { seq, mut event } => {
                if let Event::Task(TaskEvent::Upserted(task)) = &mut event {
                    without_review(task);
                }
                let failure = match &event {
                    Event::App(AppEvent::CommandFailed { message }) => Some(message.clone()),
                    _ => None,
                };
                if failure.is_some() {
                    self.pending.new_project = None;
                }
                match self.buffers.map_outline.on_event(&event) {
                    Some(MapReaction::Show) => {
                        self.regions.show(RegionId::Main, BufferKind::MapOutline);
                        self.regions.set_focus(RegionId::Main);
                        self.note = None;
                        // Opening a map from the shelf or picker goes straight to MAP mode
                        // (decision 23f), unless the user is typing something meanwhile.
                        if self.mode == Mode::Normal && self.overlay.is_none() {
                            self.enter_map_mode();
                        }
                    }
                    Some(MapReaction::Request(command)) => self.outbox.push(command),
                    None => {}
                }
                if self.buffers.task_detail.on_event(&event) {
                    self.regions.show(RegionId::Main, BufferKind::TaskDetail);
                    self.regions.set_focus(RegionId::Main);
                    self.note = None;
                }
                if let Event::Research(ResearchEvent::Upserted(summary)) = &event
                    && self.buffers.task_detail.task() == Some(summary.task)
                {
                    self.request_research_listing(summary.task);
                }
                if let Some(note) = self.buffers.worktrees.on_event(&event) {
                    self.note = Some(note);
                }
                if self.pending.awaiting_lend {
                    match &event {
                        Event::App(AppEvent::TerminalLend) => {
                            self.pending.awaiting_lend = false;
                            self.pending.handoff = Some(Command::App(AppCommand::TerminalReleased));
                        }
                        Event::App(AppEvent::CommandFailed { .. }) => {
                            self.pending.awaiting_lend = false
                        }
                        _ => {}
                    }
                }
                if let Event::App(AppEvent::TerminalReturned { error }) = &event {
                    self.pending.terminal_away = false;
                    if let Some(error) = error {
                        self.note = Some(error.clone());
                    }
                }
                self.reading_event(&event);
                if let Event::Conversation(ConversationEvent::PathsListed { agent, dir, paths }) =
                    &event
                {
                    let view = chat_view_of(&mut self.buffers, self.chat_site);
                    view.receive_paths(agent, dir.clone(), paths.clone());
                    if let Some((agent, dir)) = view.take_path_request() {
                        self.outbox
                            .push(Command::Conversation(ConversationCommand::ListPaths {
                                agent,
                                dir,
                            }));
                    }
                }
                if let Event::Mcp(McpEvent::Reply {
                    agent,
                    request,
                    outcome,
                }) = &event
                {
                    self.mcp_reply(agent, request, outcome.clone());
                }
                if let Event::Status(StatusEvent::UsageLoaded(reports)) = &event {
                    self.usage_loaded(reports.clone());
                }
                if let Event::App(AppEvent::Pasted { agent, paste }) = &event
                    && let Some(notice) = self.chat_view_mut().receive_paste(agent, paste.clone())
                {
                    self.note = Some(notice);
                }
                let side_chat = match &event {
                    Event::Agent(AgentEvent::Upserted(agent))
                        if self.pending.open_side_chat
                            && matches!(agent.kind, AgentKind::SideChat | AgentKind::Tangent)
                            && !self.snap.agents.iter().any(|a| a.id == agent.id) =>
                    {
                        Some(agent.id.clone())
                    }
                    _ => None,
                };
                let created = match &event {
                    Event::Task(TaskEvent::Upserted(task))
                        if self.pending.open_after_create.as_deref()
                            == Some(task.title.as_str())
                            && !self.snap.tasks.iter().any(|t| t.id == task.id) =>
                    {
                        Some(task.id)
                    }
                    _ => None,
                };
                let journal = matches!(event, Event::Journal(JournalEvent::Changed(_)));
                let ask = match &event {
                    Event::Agent(AgentEvent::Upserted(a)) if a.kind == AgentKind::Ask => {
                        Some(event.clone())
                    }
                    _ => None,
                };
                let recipe = match &event {
                    Event::Project(ProjectEvent::RecipeStarted {
                        project,
                        name,
                        path,
                        profiles,
                    }) => Some((
                        project.clone(),
                        name.clone(),
                        path.clone(),
                        profiles.clone(),
                    )),
                    _ => None,
                };
                // Every agent is idle and the user said so (ADR 0014): quit without asking, so
                // the binary can swap the new build in and start it.
                // Likewise when the user picked another project (B24) and said yes.
                if matches!(
                    event,
                    Event::App(AppEvent::UpgradeRestarting)
                        | Event::Project(ProjectEvent::Opening { .. })
                ) {
                    self.quit = true;
                }
                match apply(&mut self.snap, seq, event) {
                    ApplyResult::Applied => {
                        if let Some(message) = failure {
                            tracing::info!(%message, "command failed");
                            if self.pending.quit_after_close.is_some() {
                                self.pending.close_failure = Some(message.clone());
                            }
                            self.note = Some(message);
                            // A failed save must not leave a later task of the same title to
                            // steal the screen.
                            self.pending.open_after_create = None;
                            self.pending.open_side_chat = false;
                        }
                        if let Some(id) = created {
                            self.pending.open_after_create = None;
                            self.open_task(id, false);
                        }
                        if let Some(id) = side_chat {
                            self.pending.open_side_chat = false;
                            self.open_in_panel(id);
                        }
                        if journal {
                            self.on_journal();
                        }
                        if let Some(event) = ask {
                            self.reading_settle(&event);
                        }
                        if let Some((project, name, path, profiles)) = recipe {
                            self.offer_profile(project, &name, path, &profiles);
                        }
                    }
                    ApplyResult::Gap { expected, got } => {
                        tracing::warn!(expected, got, "event gap; asking for a resync");
                        if !self.pending.resyncing {
                            self.pending.resyncing = true;
                            self.outbox.push(Command::App(AppCommand::Resync));
                        }
                    }
                    ApplyResult::Stale => {}
                }
            }
        }
        if let Some(done) = self.activity.sync(&self.snap, self.now_ms)
            && self.note.is_none()
        {
            self.note = Some(done.done().into());
        }
        self.settle();
    }

    /// Keeps the open conversation pointing at a live agent, and clears an agent's finished turn
    /// when the user moves focus onto its conversation (ADR 0001: opening it). A turn that
    /// finishes while the conversation already has focus stays in Needs you (Decision 7a; live
    /// check 2026-11-06).
    fn settle(&mut self) {
        // Whatever moved focus to the side column (a picker choice, Esc from an inbox item)
        // ends focus mode, so the focused region is always on screen.
        if self.focus_mode && self.regions.focus().is_side() {
            self.toggle_focus_mode();
        }
        // MAP mode is about the map in the main region; without one it ends.
        if self.mode == Mode::Map
            && (self.regions.buffer(RegionId::Main) != BufferKind::MapOutline
                || self.buffers.map_outline.map().is_none())
        {
            self.mode = Mode::Normal;
        }
        self.buffers
            .map_outline
            .set_map_mode(self.mode == Mode::Map);
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
        // The panel follows the side chats: a closed one gives way to its neighbour.
        if self.buffers.panel.settle(&self.snap)
            && self.chat_site == ChatSite::Panel
            && self.mode == Mode::Insert
            && self.input.is_none()
        {
            self.mode = Mode::Normal;
        }
        if self.reading_mode() {
            self.settle_reading();
        }
        // The Ask session went (closed, or a new report): its chat bar goes with it.
        if self.chat_site == ChatSite::Ask
            && self.buffers.ask.agent().is_none()
            && self.mode == Mode::Insert
            && self.input.is_none()
        {
            self.mode = Mode::Normal;
            self.chat_site = ChatSite::Main;
        }
        let reading = match self.regions.focused_buffer() {
            BufferKind::Conversation if self.regions.focus() == RegionId::Main => {
                self.buffers.conversation.agent().cloned()
            }
            BufferKind::SidePanel => self.buffers.panel.agent().cloned(),
            BufferKind::Ask => self.buffers.ask.agent().cloned(),
            _ => None,
        };
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
            let command = Command::Inbox(InboxCommand::Dismiss { item: item.id });
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
        self.exit_armed = self
            .pending
            .exit_armed_until
            .take()
            .is_some_and(|d| d > mono_ms);
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
                Some(Then::Handoff(command)) => self.pending.handoff = Some(command),
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
            Mode::Normal | Mode::Map => self.normal_key(event, mono_ms),
            Mode::Insert | Mode::Command => self.input_key(event),
        }
    }

    /// Handles pasted or dropped text (a bracketed paste). In an open chat bar it goes to the core,
    /// which attaches it when it is a path to an image and hands back the text otherwise
    /// ([`AppCommand::PasteText`]). Into any other input it is typed as keys, one line. In Normal
    /// and MAP mode it is ignored, as the keys it would have been are not commands.
    pub fn on_paste(&mut self, text: &str, mono_ms: u64) {
        if self.confirm.is_some()
            || matches!(self.overlay, Some(Overlay::Confirm(_)))
            || matches!(self.mode, Mode::Normal | Mode::Map) && self.overlay.is_none()
        {
            return;
        }
        if self.overlay.is_none()
            && self.input.is_none()
            && self.chat_view().is_chatting()
            && let Some(agent) = self.chat_view().agent().cloned()
        {
            self.dirty = true;
            self.note = None;
            self.outbox.push(Command::App(AppCommand::PasteText {
                agent,
                text: text.to_string(),
            }));
            return;
        }
        let line = text.lines().next().unwrap_or("");
        for c in line.chars() {
            self.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), mono_ms);
        }
    }

    /// Normal and MAP mode keys go through the keymap; MAP mode has its own rows.
    fn normal_key(&mut self, event: KeyEvent, mono_ms: u64) {
        let scopes = self.scopes();
        let resolved = self.resolver.feed(
            &self.keymap,
            self.mode,
            &scopes,
            Key::from_event(event),
            mono_ms,
        );
        if let Resolved::Action(action) = resolved {
            self.run(action);
        }
    }

    // --- the map canvas ---

    /// Kitty graphics on, with the terminal's cell size in pixels, or off (`None`): the map is
    /// then shown as its outline alone.
    pub fn set_graphics(&mut self, cell: Option<CellPx>) {
        self.buffers.map_outline.set_graphics(cell);
        self.buffers.reader.set_graphics(cell);
        self.dirty = true;
    }

    /// The node selected in the open map.
    pub fn selected_map_node(&self) -> Option<&dovetail_api::visual::map::NodeId> {
        self.buffers.map_outline.selected_node()
    }

    /// Whether the map view uses kitty graphics.
    pub fn graphics_on(&self) -> bool {
        self.buffers.map_outline.canvas().active()
    }

    /// Whether the terminal should report the mouse: in MAP mode only (decision 23f), and never
    /// while the terminal is lent out.
    pub fn wants_mouse(&self) -> bool {
        self.mode == Mode::Map && !self.pending.terminal_away
    }

    /// A mouse event. Only MAP mode uses the mouse, and only on the map.
    pub fn on_mouse(&mut self, event: MouseEvent) {
        if self.mode != Mode::Map || self.overlay.is_some() || self.confirm.is_some() {
            return;
        }
        let kind = match event.kind {
            MouseEventKind::Down(MouseButton::Left) => MapMouse::Press,
            MouseEventKind::Drag(MouseButton::Left) => MapMouse::Drag,
            MouseEventKind::Up(MouseButton::Left) => MapMouse::Release,
            MouseEventKind::ScrollUp => MapMouse::ScrollUp,
            MouseEventKind::ScrollDown => MapMouse::ScrollDown,
            _ => return,
        };
        if self
            .buffers
            .map_outline
            .on_mouse(kind, event.column, event.row)
        {
            self.note = None;
            self.dirty = true;
        }
    }

    /// A map painting for the event loop's painter thread.
    pub fn take_paint_job(&mut self) -> Option<PaintJob> {
        self.buffers.map_outline.take_paint_job()
    }

    /// A painting came back from the painter thread.
    pub fn on_painted(&mut self, painted: Painted) {
        if self.buffers.map_outline.on_painted(painted) {
            self.dirty = true;
        }
    }

    /// What kitty should show after the frame just drawn: the map picture and the selection
    /// ring when the main region shows the map and nothing floats over it.
    pub fn graphics_scene(&self) -> Option<Scene> {
        if self.pending.terminal_away
            || self.overlay.is_some()
            || self.regions.buffer(RegionId::Main) != BufferKind::MapOutline
        {
            return None;
        }
        let ring = match self.theme.map_ring {
            ratatui::style::Color::Rgb(r, g, b) => [r, g, b],
            _ => [0xff, 0xd8, 0x4a],
        };
        self.buffers.map_outline.scene(ring)
    }

    /// MAP mode on the map in the main region.
    fn enter_map_mode(&mut self) {
        if self.regions.buffer(RegionId::Main) != BufferKind::MapOutline
            || self.buffers.map_outline.map().is_none()
        {
            self.note = Some("open a map first: ␣M shows the shelf".into());
            return;
        }
        self.resolver.reset();
        self.regions.set_focus(RegionId::Main);
        self.mode = Mode::Map;
    }

    /// `u` in MAP mode: after a yes, the coordinator is asked to redraw the open map (23h).
    fn ask_redraw_map(&mut self) {
        let (Some(map), Some(coordinator)) =
            (self.buffers.map_outline.map().cloned(), self.coordinator())
        else {
            self.note = Some("no coordinator to ask".into());
            return;
        };
        let text = format!(
            "Please redraw the map \"{}\" (id `{}`) from the current code: call maps_draw with \
             the same id, so it is updated in place.",
            map.content.title, map.id.0
        );
        self.confirm = Some(Confirm {
            prompt: format!("Ask the coordinator to redraw \"{}\"?", map.content.title),
            then: Then::Commands(vec![Command::Conversation(ConversationCommand::Prompt {
                agent: coordinator,
                text,
                attachments: Vec::new(),
                effort: None,
            })]),
            otherwise: None,
        });
    }

    fn input_key(&mut self, event: KeyEvent) {
        if self.input.is_none() {
            if !self.chat_view().is_chatting() {
                self.mode = Mode::Normal;
                return;
            }
            // The side panel's chat bar is a conversation's chat bar: the same keys.
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
                Some(Action::DropLastAttachment) if self.chat_view_mut().drop_last_attachment() => {
                    return;
                }
                Some(Action::LeaveChatBar) => {
                    self.chat_view_mut().park_chat();
                    self.mode = Mode::Normal;
                    return;
                }
                Some(action @ (Action::PageDown | Action::PageUp)) => {
                    self.run(action);
                    return;
                }
                _ => {}
            }
            // Fields borrowed apart from `self`, so the chat view can be used mutably.
            let view = chat_view_of(&mut self.buffers, self.chat_site);
            let sent = view.chat_key(event);
            let paths = view.take_path_request();
            let wants_commands = view.take_command_request();
            let chatting = view.is_chatting();
            // `/model`, `/handoff` and `/fresh` are Dovetail's, never prompts for Claude Code
            // (tasks #39, #34).
            if let Some((agent, text, attachments)) = &sent
                && let Some(found) = slash::intercept(text)
            {
                let agent = agent.clone();
                let dropped = attachments.len();
                let verb = match found {
                    slash::Intercept::Model(arg) => {
                        let arg = arg.to_owned();
                        self.open_model_selector_for(agent, (!arg.is_empty()).then_some(&arg));
                        "/model opens the model selector"
                    }
                    slash::Intercept::Mcp => {
                        self.open_mcp_menu(Some(agent));
                        "/mcp opens the MCP menu"
                    }
                    slash::Intercept::Usage => {
                        self.open_usage_menu();
                        "/usage opens the usage menu"
                    }
                    slash::Intercept::Run(action) => {
                        if !self.run_on_coordinator(Some(&agent), action) {
                            // Refused: its note says so, and the command did not run.
                            return;
                        }
                        "the command ran"
                    }
                    slash::Intercept::Refused(name) => {
                        let (text, images) = (text.clone(), attachments.clone());
                        chat_view_of(&mut self.buffers, self.chat_site)
                            .keep_draft(agent, &text, images);
                        self.note =
                            Some(format!("{name} takes nothing after it; nothing was sent"));
                        return;
                    }
                };
                if dropped > 0 {
                    // The command may have set its own note; this one matters more.
                    self.note = Some(format!(
                        "{verb}; {dropped} attached image{} not sent",
                        if dropped == 1 { " was" } else { "s were" }
                    ));
                }
                return;
            }
            if let Some((agent, text, attachments)) = sent {
                self.outbox
                    .push(Command::Conversation(ConversationCommand::Prompt {
                        agent,
                        text,
                        attachments,
                        effort: None,
                    }));
            }
            if let Some((agent, dir)) = paths {
                self.outbox
                    .push(Command::Conversation(ConversationCommand::ListPaths {
                        agent,
                        dir,
                    }));
            }
            if wants_commands {
                self.open_command_picker(true);
                return;
            }
            if !chatting {
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
    /// [`AppEvent::Pasted`]: an image saved as an attachment, text, or a refusal.
    fn paste_clipboard(&mut self) {
        if !self.chat_view().is_chatting() {
            return;
        }
        if let Some(agent) = self.chat_view().agent().cloned() {
            self.outbox.push(Command::App(AppCommand::Paste { agent }));
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
                self.outbox
                    .push(Command::Inbox(InboxCommand::AnswerPermission {
                        item,
                        answer: PermissionAnswer::Deny { message },
                    }));
            }
            InputTarget::Reply(item) if !text.is_empty() => {
                self.outbox
                    .push(Command::Inbox(InboxCommand::AnswerQuestion { item, text }));
            }
            InputTarget::Title(id) if !text.is_empty() => {
                self.outbox.push(Command::Task(TaskCommand::Update {
                    id,
                    patch: vec![TaskChange::Title(text)],
                }))
            }
            InputTarget::SliceName if !text.is_empty() => {
                self.start_input(InputTarget::SliceBrief(text), "");
            }
            InputTarget::SliceBrief(name) => self.new_slice(name, text),
            InputTarget::SideChatQuestion if !text.is_empty() => self.new_side_chat(text),
            InputTarget::TangentQuestion if !text.is_empty() => self.new_tangent(text),
            InputTarget::Search => self.start_search(text),
            InputTarget::Decision(task) if !text.is_empty() => self.log_decision(task, text),
            _ => {}
        }
    }

    fn log_decision(&mut self, task: dovetail_api::tracking::task::TaskId, text: String) {
        self.outbox.push(Command::Decision(DecisionCommand::Log(
            dovetail_api::tracking::decision::NewDecision {
                task,
                text,
                why: None,
                alternatives: Vec::new(),
                supersedes: None,
            },
        )));
    }

    /// Asks the core for a side chat; the first one to appear afterwards opens in the side
    /// panel.
    fn new_side_chat(&mut self, question: String) {
        self.pending.open_side_chat = true;
        self.outbox
            .push(Command::Agent(AgentCommand::NewSideChat { question }));
    }

    fn new_slice(&mut self, name: String, brief: String) {
        let task = self.focused_task();
        self.outbox.push(Command::Agent(AgentCommand::NewSlice {
            task,
            name,
            brief,
            repo: None,
        }));
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
                self.pending.quit_after_close = Some(journal.latest.clone());
            }
            return;
        }
        if quit {
            self.pending.quit_after_close = Some(journal.latest.clone());
            self.pending.close_failure = None;
        }
        self.outbox.push(Command::Journal(JournalCommand::EndOfDay));
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
        let Some(before) = self.pending.quit_after_close.take() else {
            return;
        };
        if journal.latest.is_some() && journal.latest != before {
            self.quit = true;
        } else {
            // The core's reason (a failed write), when it gave one.
            let why = self
                .pending
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

    /// `:handoff` and `:fresh` (also `/handoff`, `/fresh`): act on the coordinator only. `from`
    /// is the session whose chat bar typed it, else the session the keys now talk to. For any
    /// other session this sets a note and sends nothing. Returns whether the command ran.
    fn run_on_coordinator(&mut self, from: Option<&AgentId>, action: Action) -> bool {
        let target = from.cloned().or_else(|| self.chat_target());
        if target.is_some() && target != self.coordinator() {
            self.note = Some("`:handoff` and `:fresh` act on the coordinator".into());
            return false;
        }
        match action {
            Action::Handoff => {
                self.outbox.push(Command::Journal(JournalCommand::Handoff));
                self.note = Some("handoff: asking the coordinator for its handoff".into());
            }
            Action::FreshSession => {
                self.outbox
                    .push(Command::Journal(JournalCommand::FreshSession));
                self.note = Some("fresh: starting a new coordinator session".into());
            }
            other => {
                unreachable!("only Handoff and FreshSession reach here, got {other:?}")
            }
        }
        true
    }

    /// Runs an action: global ones here, the rest in the focused buffer.
    pub fn run(&mut self, action: Action) {
        self.dirty = true;
        // What shows something in Layout B leaves reading mode first, so it is seen.
        if self.reading_mode() && shows_in_layout_b(action) {
            self.leave_reading_mode();
        }
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
            Action::ListResearch => self.open_research_list(),
            Action::ShowKeys => self.open_keys_picker(),
            Action::ToggleFocusMode => self.toggle_focus_mode(),
            Action::ToggleReadingMode => self.toggle_reading_mode(),
            Action::LeaveReadingMode => self.leave_reading_mode(),
            Action::BrowseProjectLibrary => self.open_browse_picker(),
            Action::AskAboutReport => self.ask_about_report(),
            Action::Ignore => {}
            Action::PasteClipboard => self.paste_clipboard(),
            Action::QuickCapture => self.open_capture(),
            Action::PickAgent => self.open_switcher(),
            Action::PickCommand => self.open_command_picker(false),
            Action::PickAny => self.open_finder(),
            Action::Search => self.start_input(InputTarget::Search, ""),
            Action::SearchNext => self.search_jump(true),
            Action::SearchPrev => self.search_jump(false),
            Action::ShowActivity => self.toggle_main(BufferKind::Activity),
            Action::ShowMessages => self.toggle_main(BufferKind::Messages),
            Action::ShowBoard => self.toggle_main(BufferKind::Board),
            Action::ShowMaps => self.toggle_main(BufferKind::MapShelf),
            Action::MapMode => self.enter_map_mode(),
            Action::LeaveMapMode => self.mode = Mode::Normal,
            Action::MapRedraw => self.ask_redraw_map(),
            // A research project has no repo, so no worktree view: one line says why.
            Action::ShowWorktrees if self.in_research_project() => {
                self.note = Some("a research project has no repo, so no worktrees".into());
            }
            Action::ShowWorktrees => {
                let opening = self.regions.buffer(RegionId::Main) != BufferKind::Worktrees;
                self.toggle_main(BufferKind::Worktrees);
                if opening {
                    self.buffers.worktrees.begin_refresh(&self.snap);
                    self.outbox
                        .push(Command::Worktree(WorktreeCommand::Refresh));
                }
            }
            Action::BackToConversation => {
                let main = self.regions.buffer(RegionId::Main);
                // A research document goes back to the task detail it was opened from.
                if main == BufferKind::TaskDetail && self.buffers.task_detail.is_reading() {
                    self.buffers.task_detail.close_reading();
                    self.settle();
                    return;
                }
                // Leaving an inbox item goes back to the inbox, for the next one.
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
            Action::ToggleSidePanel => self.toggle_side_panel(),
            Action::NewTangent => self.run_with_arg(Action::NewTangent, ""),
            Action::ShowTangents => self.show_tangents(),
            Action::Interrupt => self.run_with_arg(Action::Interrupt, ""),
            Action::CyclePermissionMode => self.cycle_permission_mode(),
            Action::PickModel => self.open_model_selector(None),
            Action::ShowMcp => self.open_mcp_menu(None),
            Action::ShowUsage => self.open_usage_menu(),
            Action::TakeOver => self.take_over(),
            Action::RegisterProject => {
                // The open folder (ADR 0005); the core refuses any other.
                let path = self
                    .snap
                    .project
                    .as_ref()
                    .map(|p| p.repo_path.clone())
                    .unwrap_or_default();
                self.outbox
                    .push(Command::Project(ProjectCommand::Register { path }));
            }
            Action::SwitchProject => self.open_project_picker(),
            Action::NewProject => self.run_with_arg(Action::NewProject, ""),
            Action::Quit => self.quit_asking(true),
            Action::EndOfDay => self.close_day(false),
            Action::Handoff | Action::FreshSession => {
                self.run_on_coordinator(None, action);
            }
            Action::Upgrade => self.run_with_arg(Action::Upgrade, ""),
            Action::StartRound => self.run_with_arg(Action::StartRound, ""),
            Action::ForceQuit => self.quit = true,
            Action::QuitOnRepeat => {
                if std::mem::take(&mut self.exit_armed) {
                    // The second press quits the way `:q` does, asking first if a turn runs.
                    self.run(Action::Quit);
                } else {
                    self.pending.exit_armed_until = Some(self.mono_ms + EXIT_CONFIRM_MS);
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
                    activity: &self.activity,
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
                self.outbox
                    .push(Command::Map(MapCommand::Load { id, version: None }));
                self.note = Some("loading map…".into());
            }
            Effect::OpenResearch { task, what, title } => {
                self.buffers
                    .task_detail
                    .await_document(task, what.clone(), title);
                self.outbox
                    .push(Command::Research(ResearchCommand::Load { task, what }));
                self.note = Some("loading…".into());
            }
            Effect::StartInput { target, initial } => self.start_input(target, &initial),
            Effect::PickRelated(task) => self.open_relate_picker(task),
            Effect::PickFileTarget(item) => self.open_file_picker(item),
            Effect::Confirm { prompt, then } => {
                self.confirm = Some(Confirm {
                    prompt,
                    then: Then::Commands(then),
                    otherwise: None,
                });
            }
            Effect::Note(text) => self.note = Some(text),
            Effect::OpenEditor(path) => {
                // The core checks the path before the terminal is let go of.
                self.outbox
                    .push(Command::App(AppCommand::OpenEditor { path }));
                self.pending.awaiting_lend = true;
            }
            Effect::Read {
                target,
                task,
                title,
            } => self.read(target, task, title),
            Effect::Cite {
                task,
                evidence,
                browse,
                from,
            } => self.follow_citation(task, evidence, browse, from),
            Effect::BrowseProject(id) => self.browse_project(id),
            Effect::OpenTangents => self.show_tangents(),
            Effect::ReadTangent(tangent) => self.read_tangent(tangent),
            Effect::PickTangentProject { tangent, start } => {
                self.pick_tangent_project(tangent, start);
            }
            Effect::AttachTangent(tangent) => self.attach_tangent(tangent),
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
        // A picked agent shows in Layout B (ADR 0018).
        if self.reading_mode() {
            self.leave_reading_mode();
        }
        self.show_agent(id);
        self.regions.set_focus(RegionId::Main);
    }
}

/// Actions that show something in Layout B: run in reading mode, they leave it first.
fn shows_in_layout_b(action: Action) -> bool {
    matches!(
        action,
        Action::FocusInbox
            | Action::FocusCoordinator
            | Action::AlternateAgent
            | Action::ToggleFocusMode
            | Action::ShowActivity
            | Action::ShowMessages
            | Action::ShowBoard
            | Action::ShowWorktrees
            | Action::ShowMaps
            | Action::MapMode
            | Action::ToggleSidePanel
            | Action::NewSideChat
            | Action::NewTangent
            | Action::ShowTangents
            | Action::NewSlice
            | Action::ListResearch
            | Action::EnterInsert
    )
}

/// The conversation view whose chat bar `site` names, borrowed from the buffers alone.
fn chat_view_of(buffers: &mut Buffers, site: ChatSite) -> &mut crate::buffers::ConversationBuffer {
    match site {
        ChatSite::Main => &mut buffers.conversation,
        ChatSite::Panel => buffers.panel.conversation_mut(),
        ChatSite::Ask => buffers.ask.conversation_mut(),
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

/// This UI is frozen as built before the review status (ADR 0024): it shows a task in review as
/// in progress. Done once, where tasks arrive, so nothing downstream meets the status.
fn without_review(task: &mut dovetail_api::tracking::task::Task) {
    if task.status == dovetail_api::tracking::task::TaskStatus::Review {
        task.status = dovetail_api::tracking::task::TaskStatus::InProgress;
    }
}
