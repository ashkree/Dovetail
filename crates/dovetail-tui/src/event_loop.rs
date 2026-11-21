//! The terminal event loop: one blocking channel, a dirty flag, and no idle ticks.
//!
//! A thread reads terminal input and another forwards the core's messages; the UI thread
//! blocks on one channel. It wakes only for a message, the 30 fps draw window after a change,
//! the timeout of a half-typed key sequence, or a usage-limit reset passing. With nothing
//! changing it sleeps forever.
//!
//! **Lending the terminal** (`e` in the worktree view, [`Command::OpenEditor`]; `<leader>st`,
//! [`Command::TakeOver`]). The input thread reads one key, hands it over, and waits for a go
//! before reading the next, so it is never inside a read when a key asks for the terminal. If
//! the key leaves a hand-off in the app ([`App::take_handoff`]), the loop sends the commands
//! already queued, leaves the alternate screen and raw mode ([`Lend::release`]), sends the
//! hand-off command, and holds the go back: the editor or native `claude` the core starts gets
//! every key. Core messages are still folded in but nothing is drawn. On [`dovetail_api::Event::TerminalReturned`] the loop takes the
//! terminal back ([`Lend::reclaim`]), redraws it in full and lets the input thread read again.
//!
//! **The map canvas** (M15 f). Mouse capture is switched on only while the app is in MAP mode
//! ([`MouseCapture`], decision 23f), so the terminal's own text selection works everywhere else.
//! A painter thread paints maps off the UI thread ([`MapPainter`]); after each frame the loop
//! writes what kitty must change to show the app's [`Scene`] ([`kitty::Screen`]).
//!
//! **Shift-Enter.** Where the terminal speaks the kitty keyboard protocol, the loop asks it to
//! disambiguate keys, so Shift-Enter arrives as itself and starts a new line in the chat bar.
//! Nothing depends on it: other terminals keep Alt-Enter. The flags are pushed on the alternate
//! screen and popped before leaving it.

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use std::io::Write;
use std::sync::Arc;

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as TermEvent, KeyEventKind, KeyboardEnhancementFlags, MouseEvent,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use dovetail_api::{Command, CoreLink, ServerMsg};

use crate::app::{App, Signal};
use crate::canvas::kitty::{self, Scene};
use crate::canvas::view::CellPx;
use crate::canvas::{MapPainter, PaintJob, Painted, detect};

/// Shortest time between two draws: about 30 fps, so streaming text is coalesced.
pub const MIN_FRAME: Duration = Duration::from_millis(33);

/// The UI's clock in Unix milliseconds, injected so tests and the mock control it.
pub type Clock = Box<dyn Fn() -> i64 + Send>;

enum Msg {
    Key(crossterm::event::KeyEvent),
    Mouse(MouseEvent),
    Painted(Painted),
    /// Text from a bracketed paste: pasted or dropped, not typed.
    Paste(String),
    Resize,
    Server(Box<ServerMsg>),
    ServerClosed,
    /// SIGINT, SIGTERM or SIGHUP.
    Signal(Signal),
}

/// How long the loop may sleep: until the next draw window if the screen is dirty, until the
/// key-sequence deadline if one is pending, until the next spinner frame (`frame_in`, given
/// only while a spinner is on screen), and forever (`None`) if none of them.
pub fn next_wake(
    dirty: bool,
    since_last_draw: Duration,
    key_deadline_in: Option<Duration>,
    frame_in: Option<Duration>,
) -> Option<Duration> {
    let draw_in = dirty.then(|| MIN_FRAME.saturating_sub(since_last_draw));
    [draw_in, key_deadline_in, frame_in]
        .into_iter()
        .flatten()
        .min()
}

/// Lets go of the terminal and takes it back, for a program the core runs in it.
pub trait Lend {
    /// Leaves the alternate screen and raw mode and shows the cursor.
    fn release(&mut self) -> io::Result<()>;
    /// Enters raw mode and the alternate screen again. The loop then clears the screen so the
    /// next frame is drawn in full.
    fn reclaim(&mut self) -> io::Result<()>;
}

/// The real terminal, through crossterm. `enhanced`: the keyboard flags were pushed.
struct Crossterm {
    enhanced: bool,
}

/// Asks the terminal to report Shift-Enter and friends, if it can. Returns whether it did.
/// Must run before the input thread starts: the support query reads the terminal's answer.
fn push_keyboard_flags() -> bool {
    matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    ) && crossterm::execute!(
        io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .is_ok()
}

fn pop_keyboard_flags() {
    let _ = crossterm::execute!(io::stdout(), PopKeyboardEnhancementFlags);
}

impl Lend for Crossterm {
    fn release(&mut self) -> io::Result<()> {
        if self.enhanced {
            pop_keyboard_flags();
        }
        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(
            io::stdout(),
            DisableBracketedPaste,
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        )
    }

    fn reclaim(&mut self) -> io::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            EnableBracketedPaste
        )?;
        if self.enhanced {
            crossterm::execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        Ok(())
    }
}

/// Turns the terminal's mouse reporting on and off to follow [`App::wants_mouse`].
#[derive(Debug, Default)]
pub struct MouseCapture {
    on: bool,
}

impl MouseCapture {
    /// Whether capture is on now.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Writes the switch to `out` if `want` differs from what the terminal has.
    pub fn sync(&mut self, want: bool, out: &mut dyn Write) -> io::Result<()> {
        if want == self.on {
            return Ok(());
        }
        let mut ansi = String::new();
        let written = if want {
            crossterm::Command::write_ansi(&EnableMouseCapture, &mut ansi)
        } else {
            crossterm::Command::write_ansi(&DisableMouseCapture, &mut ansi)
        };
        written.map_err(|e| io::Error::other(e.to_string()))?;
        out.write_all(ansi.as_bytes())?;
        out.flush()?;
        self.on = want;
        Ok(())
    }
}

/// The cell size in pixels when the terminal shows kitty graphics and reports its pixel size;
/// `None` means the outline alone.
fn graphics_cell() -> Option<CellPx> {
    if !detect::kitty_graphics(|k| std::env::var(k).ok()) {
        return None;
    }
    let size = crossterm::terminal::window_size().ok()?;
    if size.columns == 0 || size.rows == 0 || size.width == 0 || size.height == 0 {
        tracing::info!("the terminal reports no pixel size; maps show as outlines");
        return None;
    }
    Some(CellPx {
        w: size.width / size.columns,
        h: size.height / size.rows,
    })
}

/// Restores the terminal when dropped, including on an early return or panic unwind.
struct TerminalGuard {
    enhanced: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Harmless when capture is off or the screen is already left; a shell left with mouse
        // reporting on is not.
        let mut out = io::stdout();
        let _ = crossterm::execute!(out, DisableMouseCapture);
        let mut clear = Vec::new();
        kitty::delete(&mut clear, kitty::MAP_ID);
        kitty::delete(&mut clear, kitty::RING_ID);
        let _ = out.write_all(&clear);
        let _ = out.flush();
        // On a panic ratatui's hook has already left the alternate screen, whose flags went with
        // it; popping now would pop the shell's.
        if self.enhanced && !thread::panicking() {
            pop_keyboard_flags();
        }
        if !thread::panicking() {
            let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste);
        }
        ratatui::restore();
    }
}

/// Runs the UI until the user quits. Sets up the terminal and a panic hook that restores it.
/// `painter` draws map pictures for the canvas; without one maps show as outlines.
pub fn run(
    app: &mut App,
    link: CoreLink,
    clock: Clock,
    painter: Option<Arc<dyn MapPainter>>,
) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let enhanced = push_keyboard_flags();
    // Without it a terminal types a paste as keys, and a dropped file path cannot be told from
    // typing. Terminals that lack it keep doing that.
    let _ = crossterm::execute!(io::stdout(), EnableBracketedPaste);
    let _guard = TerminalGuard { enhanced };
    let CoreLink { commands, messages } = link;
    let (tx, rx) = mpsc::channel::<Msg>();
    // One go per key: the input thread reads the next key only once the loop has handled the
    // last one and kept the terminal (see the module docs).
    let (go_tx, go_rx) = mpsc::channel::<()>();

    let input_tx = tx.clone();
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            let gone = match event {
                TermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                    input_tx.send(Msg::Key(key)).is_err() || go_rx.recv().is_err()
                }
                TermEvent::Paste(text) => input_tx.send(Msg::Paste(text)).is_err(),
                TermEvent::Resize(..) => input_tx.send(Msg::Resize).is_err(),
                TermEvent::Mouse(mouse) => input_tx.send(Msg::Mouse(mouse)).is_err(),
                _ => false,
            };
            if gone {
                break;
            }
        }
    });
    // Raw mode turns Ctrl-C into a key, but `kill` and a closing terminal still send signals.
    // SIGINT asks like `:q`; SIGTERM and SIGHUP quit at once (see `App::on_signal`). If the
    // handlers cannot be installed the UI still works; it just cannot catch signals.
    let signal_tx = tx.clone();
    match signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ]) {
        Ok(mut signals) => {
            thread::spawn(move || {
                for number in signals.forever() {
                    let signal = if number == signal_hook::consts::SIGINT {
                        Signal::Interrupt
                    } else {
                        Signal::Terminate
                    };
                    if signal_tx.send(Msg::Signal(signal)).is_err() {
                        break;
                    }
                }
            });
        }
        Err(e) => tracing::warn!(%e, "could not install signal handlers"),
    }
    if painter.is_some() {
        app.set_graphics(graphics_cell());
    }
    // The painter thread: paints the newest job, skipping any that a newer one replaced.
    let paint = painter.map(|painter| {
        let (job_tx, job_rx) = mpsc::channel::<PaintJob>();
        let painted_tx = tx.clone();
        thread::spawn(move || {
            while let Ok(mut job) = job_rx.recv() {
                while let Ok(newer) = job_rx.try_recv() {
                    job = newer;
                }
                let started = Instant::now();
                let result = painter.paint(&job.map, job.key.fit).map(Arc::new);
                tracing::debug!(map = %job.key.map.0, ms = started.elapsed().as_millis(), ok = result.is_ok(), "map painted");
                let painted = Painted {
                    key: job.key,
                    result,
                };
                if painted_tx.send(Msg::Painted(painted)).is_err() {
                    return;
                }
            }
        });
        job_tx
    });
    thread::spawn(move || {
        for msg in messages {
            if tx.send(Msg::Server(Box::new(msg))).is_err() {
                return;
            }
        }
        let _ = tx.send(Msg::ServerClosed);
    });

    let mut lend = Crossterm { enhanced };
    let mut out = io::stdout();
    let mut io = LoopIo::new(&commands, &go_tx, &mut lend, &mut out);
    io.paint = paint;
    io.graphics = app.graphics_on();
    drive(&mut terminal, app, &rx, &mut io, &clock)
}

/// What the loop talks to besides the app: the core's command channel, the input thread's go,
/// and the terminal it can lend out.
struct LoopIo<'a> {
    commands: &'a mpsc::Sender<Command>,
    go: &'a mpsc::Sender<()>,
    lend: &'a mut dyn Lend,
    /// The terminal, for kitty graphics and mouse capture.
    out: &'a mut dyn Write,
    mouse: MouseCapture,
    kitty: kitty::Screen,
    /// The painter thread's job queue, when there is a painter.
    paint: Option<mpsc::Sender<PaintJob>>,
    /// The terminal shows kitty graphics: re-read the cell size on a resize.
    graphics: bool,
}

impl<'a> LoopIo<'a> {
    fn new(
        commands: &'a mpsc::Sender<Command>,
        go: &'a mpsc::Sender<()>,
        lend: &'a mut dyn Lend,
        out: &'a mut dyn Write,
    ) -> Self {
        Self {
            commands,
            go,
            lend,
            out,
            mouse: MouseCapture::default(),
            kitty: kitty::Screen::default(),
            paint: None,
            graphics: false,
        }
    }

    /// Mouse capture follows the app's mode; a painting it asked for goes to the painter.
    fn follow_app(&mut self, app: &mut App) -> io::Result<()> {
        self.mouse.sync(app.wants_mouse(), self.out)?;
        if let Some(job) = app.take_paint_job()
            && let Some(paint) = &self.paint
        {
            let _ = paint.send(job);
        }
        Ok(())
    }

    /// After a frame: what kitty must change to show `scene` and reading mode's `figures`.
    fn show(&mut self, scene: Option<&Scene>, figures: &[kitty::Figure]) -> io::Result<()> {
        let mut bytes = Vec::new();
        self.kitty.sync(scene, &mut bytes);
        self.kitty.sync_figures(figures, &mut bytes);
        if !bytes.is_empty() {
            self.out.write_all(&bytes)?;
            self.out.flush()?;
        }
        Ok(())
    }

    /// Before lending the terminal out: no mouse reporting and no images for the program that
    /// gets it.
    fn let_go(&mut self) -> io::Result<()> {
        self.mouse.sync(false, self.out)?;
        let mut bytes = Vec::new();
        self.kitty.clear(&mut bytes);
        self.out.write_all(&bytes)?;
        self.out.flush()
    }

    fn send_all(&mut self, app: &mut App) {
        for command in app.take_commands() {
            let _ = self.commands.send(command);
        }
    }

    /// After a key: lends the terminal out if the key asked for it, else lets the input thread
    /// read the next key.
    fn after_key(&mut self, app: &mut App) -> io::Result<()> {
        // Commands the key queued go first, in order.
        self.send_all(app);
        if app.awaiting_lend() {
            // The core is checking the editor's path: no key is read until it answers.
            return Ok(());
        }
        self.lend_out(app)
    }

    /// Lends the terminal out if the app has a hand-off, else lets the input thread read the
    /// next key.
    fn lend_out(&mut self, app: &mut App) -> io::Result<()> {
        let Some(handoff) = app.take_handoff() else {
            let _ = self.go.send(());
            return Ok(());
        };
        self.let_go()?;
        self.lend.release()?;
        tracing::debug!(?handoff, "terminal lent out");
        let _ = self.commands.send(handoff);
        Ok(())
    }
}

/// The loop itself, on any backend: draws when dirty (at most ~30 fps), sends the app's
/// commands, and sleeps until a message, a draw window or a key-sequence deadline. Returns when
/// the user quits or every sender of `rx` is gone.
fn drive<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    rx: &mpsc::Receiver<Msg>,
    io: &mut LoopIo,
    clock: &Clock,
) -> io::Result<()> {
    let started = Instant::now();
    let mono_ms = |at: Instant| at.duration_since(started).as_millis() as u64;
    let mut last_draw: Option<Instant> = None;
    // The terminal is lent out: nothing is drawn and no key is read until it comes back.
    let mut away = false;
    // `OpenEditor` is out: the key that asked is not released until the core says yes or no.
    let mut waiting = false;
    loop {
        let now = Instant::now();
        let since = last_draw.map_or(MIN_FRAME, |t| now.duration_since(t));
        if app.is_dirty() && !away && since >= MIN_FRAME {
            terminal
                .draw(|frame| app.draw(frame))
                .map_err(|e| io::Error::other(e.to_string()))?;
            io.show(app.graphics_scene().as_ref(), &app.reading_figures())?;
            tracing::trace!("frame");
            app.take_dirty();
            last_draw = Some(now);
        }
        io.follow_app(app)?;
        io.send_all(app);
        if app.should_quit() {
            // Sessions end with the app: there is no daemon in v1 (plan B23).
            let _ = io.commands.send(Command::Quit { end_sessions: true });
            return Ok(());
        }

        let since = last_draw.map_or(MIN_FRAME, |t| Instant::now().duration_since(t));
        let key_in = app
            .next_deadline_ms()
            .map(|d| Duration::from_millis(d.saturating_sub(mono_ms(Instant::now()))));
        // A usage-limit reset passing redraws the header, on the wall clock.
        let clock_in = app
            .next_clock_change_ms()
            .map(|d| Duration::from_millis(u64::try_from(d - clock()).unwrap_or(0)));
        let key_in = match (key_in, clock_in) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let frame_in = if away { None } else { app.next_animation_in() };
        let first = match next_wake(app.is_dirty() && !away, since, key_in, frame_in) {
            // Every sender gone: nothing can wake the loop again.
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => return Ok(()),
            },
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(msg) => Some(msg),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            },
        };
        app.set_now(clock());
        // Take everything already waiting before drawing, so a burst costs one frame.
        for msg in first
            .into_iter()
            .chain(std::iter::from_fn(|| rx.try_recv().ok()))
        {
            match msg {
                Msg::Key(key) => {
                    app.on_key(key, mono_ms(Instant::now()));
                    io.after_key(app)?;
                    away = app.terminal_away();
                    waiting = app.awaiting_lend();
                }
                Msg::Mouse(mouse) => app.on_mouse(mouse),
                Msg::Paste(text) => app.on_paste(&text, mono_ms(Instant::now())),
                Msg::Painted(painted) => app.on_painted(painted),
                Msg::Resize => {
                    // A font size change changes the cell size too.
                    if io.graphics {
                        app.set_graphics(graphics_cell());
                    }
                    app.on_resize();
                }
                Msg::Server(msg) => app.on_server(*msg),
                Msg::ServerClosed => app.on_core_closed(),
                Msg::Signal(signal) => app.on_signal(signal),
            }
        }
        // The core answered the editor request: the terminal goes out, or the keys resume.
        if waiting && !app.awaiting_lend() {
            waiting = false;
            io.lend_out(app)?;
            away = app.terminal_away();
        }
        if away && !app.terminal_away() {
            io.lend.reclaim()?;
            io.kitty.forget();
            terminal
                .clear()
                .map_err(|e| io::Error::other(e.to_string()))?;
            tracing::debug!("terminal back");
            away = false;
            let _ = io.go.send(());
        }
        app.on_timeout(mono_ms(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal that only records being lent out and taken back.
    #[derive(Default)]
    struct Recorder {
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Lend for Recorder {
        fn release(&mut self) -> io::Result<()> {
            self.log.lock().expect("log").push("release".into());
            Ok(())
        }

        fn reclaim(&mut self) -> io::Result<()> {
            self.log.lock().expect("log").push("reclaim".into());
            Ok(())
        }
    }

    /// Rehearsal 2026-11-06: the tree showed no tasks for about 90 s until a key was pressed.
    /// With no input at all, core events must still reach the screen.
    #[test]
    fn core_events_redraw_the_screen_with_no_key_pressed() {
        use dovetail_api::{Event, Snapshot, Task, TaskId, TaskStatus};
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).expect("terminal");
        let mut app = App::new(jiff::tz::TimeZone::UTC);
        let (tx, rx) = mpsc::channel::<Msg>();
        let (commands, _sent) = mpsc::channel::<Command>();
        let (go, _gone) = mpsc::channel::<()>();
        let mut recorder = Recorder::default();
        let mut out = Vec::new();
        let mut io = LoopIo::new(&commands, &go, &mut recorder, &mut out);
        let feeder = thread::spawn(move || {
            let send = |m| tx.send(Msg::Server(Box::new(m))).expect("loop alive");
            send(ServerMsg::Hello {
                protocol: dovetail_api::PROTOCOL_VERSION,
            });
            send(ServerMsg::Snapshot(Snapshot::default()));
            // A pause long enough for the loop to draw and go back to sleep with nothing dirty.
            thread::sleep(Duration::from_millis(150));
            send(ServerMsg::Event {
                seq: 1,
                event: Event::TaskUpserted(Task {
                    id: TaskId(1),
                    title: "alpha from the core".into(),
                    status: TaskStatus::Todo,
                    p_level: None,
                    bucket: None,
                    order: 0,
                    deps: vec![],
                    notes: String::new(),
                    pr: None,
                    follow_up_of: None,
                    coordinator_created: true,
                    related: vec![],
                    effort: Default::default(),
                    criterion: None,
                }),
            });
            // Long enough for the 33 ms draw window; then end the loop with no key pressed.
            thread::sleep(Duration::from_millis(200));
        });
        let clock: Clock = Box::new(|| 0);
        drive(&mut terminal, &mut app, &rx, &mut io, &clock).expect("loop");
        feeder.join().expect("feeder");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("alpha from the core"), "{screen}");
    }

    /// `e` in the worktree view lends the terminal to the core's editor and takes it back:
    /// the terminal is released before `OpenEditor` goes out, the input thread reads no key
    /// while the editor runs, and the terminal is reclaimed (and the next key read) only once
    /// `TerminalReturned` arrives.
    #[test]
    fn e_lends_the_terminal_to_the_editor_and_takes_it_back() {
        let (log, app) = press_on(&std::env::temp_dir(), &[' ', 'g', 'e']);
        assert_eq!(
            log,
            ["release", "editor", "reclaim", "next key read"],
            "each step causes the next"
        );
        assert!(!app.terminal_away());
    }

    /// A worktree that is gone is refused by the core before the terminal is touched: no
    /// flicker, nothing released, the reason in the status bar, and keys keep coming.
    #[test]
    fn e_on_a_missing_worktree_never_lets_go_of_the_terminal() {
        let missing = std::env::temp_dir().join("dovetail-no-such-worktree-4f1c");
        let (log, app) = press_on(&missing, &[' ', 'g', 'e']);
        assert_eq!(log, ["next key read"]);
        assert!(!app.terminal_away());
        assert_eq!(
            app.note(),
            Some(format!("{} does not exist", missing.display()).as_str())
        );
    }

    /// `<leader>st` lends the terminal to native `claude` the same way: released before
    /// `TakeOver` goes out for the focused session (the coordinator here), no key read while it
    /// runs, reclaimed on `TerminalReturned`.
    #[test]
    fn leader_st_lends_the_terminal_to_native_claude_and_takes_it_back() {
        let (log, app) = press_on(&std::env::temp_dir(), &[' ', 's', 't']);
        assert_eq!(
            log,
            ["release", "native coordinator", "reclaim", "next key read"],
            "each step causes the next"
        );
        assert!(!app.terminal_away());
    }

    /// Runs the loop over a coordinator and one slice whose worktree is at `path`, pressing
    /// `keys`. The fake core logs each command, answers `OpenEditor` and `TakeOver` with
    /// `TerminalReturned` and stops at `Quit`; the fake input thread sends one key per go, as
    /// the real one does, then SIGTERM. Returns the log of terminal, core and input steps, and
    /// the app.
    fn press_on(path: &std::path::Path, keys: &'static [char]) -> (Vec<String>, App) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use dovetail_api::{
            Agent, AgentId, AgentKind, AgentStatus, Event, PermissionMode, SessionLock, Snapshot,
            WorktreeRef,
        };

        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let note = |log: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, s: &str| {
            log.lock().expect("log").push(s.to_owned());
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).expect("terminal");
        let mut app = App::new(jiff::tz::TimeZone::UTC);
        let mut snap = Snapshot::default();
        let session = |id: &str, kind, worktree| Agent {
            id: AgentId(id.into()),
            name: id.into(),
            kind,
            parent: None,
            task: None,
            worktree,
            status: AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::DriverKind::Claude,
        };
        snap.agents
            .push(session("coordinator", AgentKind::Coordinator, None));
        snap.agents.push(session(
            "slice-4",
            AgentKind::Slice,
            Some(WorktreeRef {
                path: path.to_path_buf(),
                branch: "dovetail/slice-4".into(),
            }),
        ));
        app.on_server(ServerMsg::Snapshot(snap));

        let (tx, rx) = mpsc::channel::<Msg>();
        let (commands, sent) = mpsc::channel::<Command>();
        let (go, gone) = mpsc::channel::<()>();

        // The fake core: "runs the editor" on OpenEditor, then hands the terminal back.
        let core_tx = tx.clone();
        let core_log = log.clone();
        let core = thread::spawn(move || {
            for command in sent {
                let answer = |event| {
                    let _ = core_tx.send(Msg::Server(Box::new(ServerMsg::Event { seq: 1, event })));
                };
                match command {
                    // Like the core: a path that is gone is refused, else the UI is asked to
                    // let go, and the editor runs once it says it has.
                    Command::OpenEditor { path } if path.exists() => answer(Event::LendTerminal),
                    Command::OpenEditor { path } => answer(Event::CommandFailed {
                        message: format!("{} does not exist", path.display()),
                    }),
                    Command::TerminalReleased | Command::TakeOver { .. } => {
                        let what = match &command {
                            Command::TakeOver { agent } => format!("native {}", agent.0),
                            _ => "editor".into(),
                        };
                        note(&core_log, &what);
                        answer(Event::TerminalReturned { error: None });
                    }
                    Command::Quit { .. } => return,
                    other => note(&core_log, &format!("{other:?}")),
                }
            }
        });
        // The fake input thread: one key per go, as the real one; then SIGTERM ends the loop.
        let input_log = log.clone();
        let input = thread::spawn(move || {
            for &c in keys {
                let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
                tx.send(Msg::Key(key)).expect("loop alive");
                gone.recv().expect("go");
            }
            note(&input_log, "next key read");
            tx.send(Msg::Signal(Signal::Terminate)).expect("loop alive");
        });

        let mut recorder = Recorder { log: log.clone() };
        let mut out = Vec::new();
        let mut io = LoopIo::new(&commands, &go, &mut recorder, &mut out);
        let clock: Clock = Box::new(|| 0);
        drive(&mut terminal, &mut app, &rx, &mut io, &clock).expect("loop");
        input.join().expect("input");
        core.join().expect("core");
        let mut log = log.lock().expect("log").clone();
        // The fake core logs on its own thread, so the refresh `␣g` sent may land anywhere;
        // check it, then leave it out of the ordered steps.
        if keys.contains(&'g') {
            let refresh = log.iter().position(|l| l == "RefreshWorktrees");
            log.remove(refresh.expect("␣g asked for a refresh"));
        }
        (log, app)
    }

    #[test]
    fn an_idle_ui_sleeps_with_no_deadline() {
        assert_eq!(next_wake(false, Duration::from_secs(5), None, None), None);
    }

    #[test]
    fn a_dirty_screen_waits_out_the_frame_window() {
        assert_eq!(
            next_wake(true, Duration::from_millis(10), None, None),
            Some(Duration::from_millis(23))
        );
        assert_eq!(
            next_wake(true, Duration::from_millis(40), None, None),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_pending_key_sequence_sets_the_wake_even_when_clean() {
        let d = Duration::from_millis(700);
        assert_eq!(next_wake(false, Duration::ZERO, Some(d), None), Some(d));
        assert_eq!(
            next_wake(true, Duration::from_millis(30), Some(d), None),
            Some(Duration::from_millis(3))
        );
    }

    #[test]
    fn a_spinner_on_screen_sets_a_one_shot_wake_within_a_frame() {
        let frame = Duration::from_millis(80);
        assert_eq!(
            next_wake(false, Duration::from_secs(5), None, Some(frame)),
            Some(frame)
        );
        // The draw window comes first when it is nearer, a key deadline when that is.
        assert_eq!(
            next_wake(true, Duration::from_millis(10), None, Some(frame)),
            Some(Duration::from_millis(23))
        );
        let key = Duration::from_millis(20);
        assert_eq!(
            next_wake(false, Duration::ZERO, Some(key), Some(frame)),
            Some(key)
        );
    }
}
