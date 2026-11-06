//! The terminal event loop: one blocking channel, a dirty flag, and no idle ticks.
//!
//! A thread reads terminal input and another forwards the core's messages; the UI thread
//! blocks on one channel. It wakes only for a message, the 30 fps draw window after a change,
//! or the timeout of a half-typed key sequence. With nothing changing it sleeps forever.

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyEventKind};
use dovetail_api::{Command, CoreLink, ServerMsg};

use crate::app::App;

/// Shortest time between two draws: about 30 fps, so streaming text is coalesced.
pub const MIN_FRAME: Duration = Duration::from_millis(33);

/// The UI's clock in Unix milliseconds, injected so tests and the mock control it.
pub type Clock = Box<dyn Fn() -> i64 + Send>;

enum Msg {
    Key(crossterm::event::KeyEvent),
    Resize,
    Server(Box<ServerMsg>),
    ServerClosed,
}

/// How long the loop may sleep: until the next draw window if the screen is dirty, until the
/// key-sequence deadline if one is pending, and forever (`None`) if neither.
pub fn next_wake(
    dirty: bool,
    since_last_draw: Duration,
    key_deadline_in: Option<Duration>,
) -> Option<Duration> {
    let draw_in = dirty.then(|| MIN_FRAME.saturating_sub(since_last_draw));
    match (draw_in, key_deadline_in) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Restores the terminal when dropped, including on an early return or panic unwind.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// Runs the UI until the user quits. Sets up the terminal and a panic hook that restores it.
pub fn run(app: &mut App, link: CoreLink, clock: Clock) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard;
    let CoreLink { commands, messages } = link;
    let (tx, rx) = mpsc::channel::<Msg>();

    let input_tx = tx.clone();
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            let msg = match event {
                TermEvent::Key(key) if key.kind != KeyEventKind::Release => Msg::Key(key),
                TermEvent::Resize(..) => Msg::Resize,
                _ => continue,
            };
            if input_tx.send(msg).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        for msg in messages {
            if tx.send(Msg::Server(Box::new(msg))).is_err() {
                return;
            }
        }
        let _ = tx.send(Msg::ServerClosed);
    });

    drive(&mut terminal, app, &rx, &commands, &clock)
}

/// The loop itself, on any backend: draws when dirty (at most ~30 fps), sends the app's
/// commands, and sleeps until a message, a draw window or a key-sequence deadline. Returns when
/// the user quits or every sender of `rx` is gone.
fn drive<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    rx: &mpsc::Receiver<Msg>,
    commands: &mpsc::Sender<Command>,
    clock: &Clock,
) -> io::Result<()> {
    let started = Instant::now();
    let mono_ms = |at: Instant| at.duration_since(started).as_millis() as u64;
    let mut last_draw: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let since = last_draw.map_or(MIN_FRAME, |t| now.duration_since(t));
        if app.is_dirty() && since >= MIN_FRAME {
            terminal
                .draw(|frame| app.draw(frame))
                .map_err(|e| io::Error::other(e.to_string()))?;
            tracing::trace!("frame");
            app.take_dirty();
            last_draw = Some(now);
        }
        for command in app.take_commands() {
            let _ = commands.send(command);
        }
        if app.should_quit() {
            // Sessions end with the app: there is no daemon in v1 (plan B23).
            let _ = commands.send(Command::Quit { end_sessions: true });
            return Ok(());
        }

        let since = last_draw.map_or(MIN_FRAME, |t| Instant::now().duration_since(t));
        let key_in = app
            .next_deadline_ms()
            .map(|d| Duration::from_millis(d.saturating_sub(mono_ms(Instant::now()))));
        let first = match next_wake(app.is_dirty(), since, key_in) {
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
                Msg::Key(key) => app.on_key(key, mono_ms(Instant::now())),
                Msg::Resize => app.on_resize(),
                Msg::Server(msg) => app.on_server(*msg),
                Msg::ServerClosed => app.on_core_closed(),
            }
        }
        app.on_timeout(mono_ms(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                }),
            });
            // Long enough for the 33 ms draw window; then end the loop with no key pressed.
            thread::sleep(Duration::from_millis(200));
        });
        let clock: Clock = Box::new(|| 0);
        drive(&mut terminal, &mut app, &rx, &commands, &clock).expect("loop");
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

    #[test]
    fn an_idle_ui_sleeps_with_no_deadline() {
        assert_eq!(next_wake(false, Duration::from_secs(5), None), None);
    }

    #[test]
    fn a_dirty_screen_waits_out_the_frame_window() {
        assert_eq!(
            next_wake(true, Duration::from_millis(10), None),
            Some(Duration::from_millis(23))
        );
        assert_eq!(
            next_wake(true, Duration::from_millis(40), None),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_pending_key_sequence_sets_the_wake_even_when_clean() {
        let d = Duration::from_millis(700);
        assert_eq!(next_wake(false, Duration::ZERO, Some(d)), Some(d));
        assert_eq!(
            next_wake(true, Duration::from_millis(30), Some(d)),
            Some(Duration::from_millis(3))
        );
    }
}
