//! The Dovetail GUI: a Tauri window around the web frontend in `gui/`, whose process hosts the
//! core (ADR 0023).
//!
//! The frontend talks to the core through two commands and nothing else:
//!
//! - `core_send(command)`: one [`Command`], passed on to the core unchanged;
//! - `core_subscribe(channel)`: every [`ServerMsg`] from then on, in order, on that channel. It
//!   asks the core for a fresh snapshot first, so a reloaded window starts from one.
//!
//! The shell never looks inside a message, except to see when a core is done:
//! [`Event::OpeningProject`] means the running core is stopping so another project can open.
//! A supervisor thread then waits for it and spawns the next one, in this same process.
//!
//! At start no project is open: [`dovetail_core::home`] serves Home. Closing the window quits the
//! core, which stops its sessions; they resume on the next open, as in the TUI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use dovetail_api::{Command, CoreLink, Event, ServerMsg};
use dovetail_core::config::Config;
use dovetail_core::db::Db;
use dovetail_core::home::{HomeHandle, spawn_home};
use dovetail_core::paths::Paths;
use dovetail_core::{CoreHandle, CoreOptions, projects, upgrade};
use dovetail_driver::ClaudeDriver;
use tauri::ipc::Channel;
use tauri::{RunEvent, State};

/// How long closing the window waits for sessions to stop, as the TUI does.
const QUIT_WAIT: Duration = Duration::from_secs(10);

/// What the two commands share with the supervisor.
#[derive(Default)]
struct Shell {
    /// Commands into the running core (Home or a project's); `None` between two cores.
    commands: Mutex<Option<mpsc::Sender<Command>>>,
    /// Where messages go: the frontend's channel, once it subscribed.
    channel: Mutex<Option<Channel<ServerMsg>>>,
    /// The window is closing: the supervisor starts no new core.
    closing: Mutex<bool>,
}

/// A lock that survives a panicked holder: the data is a sender or a flag, always usable.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sends one command to the running core.
#[tauri::command]
fn core_send(shell: State<'_, Arc<Shell>>, command: Command) -> Result<(), String> {
    match lock(&shell.commands).as_ref() {
        Some(tx) => tx
            .send(command)
            .map_err(|_| "the core has stopped".to_owned()),
        None => Err("the core is starting; try again".to_owned()),
    }
}

/// Routes every message from now on to `channel`, and asks for a fresh snapshot.
#[tauri::command]
fn core_subscribe(shell: State<'_, Arc<Shell>>, channel: Channel<ServerMsg>) {
    *lock(&shell.channel) = Some(channel);
    if let Some(tx) = lock(&shell.commands).as_ref() {
        let _ = tx.send(Command::Resync);
    }
}

/// Which core runs.
enum Running {
    Home(HomeHandle),
    Project(CoreHandle),
}

/// What the supervisor needs to start a project's core.
struct Setup {
    paths: Paths,
    config: Config,
    options: CoreOptions,
}

/// Why no session can start, if one cannot: the core's sessions run `dovetail mcp` and
/// `dovetail hook` from the installed binary.
fn installed_binary() -> Result<PathBuf, String> {
    let path = upgrade::install_path().map_err(|e| e.to_string())?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "No session can start: the dovetail binary is not installed at {}. Run \
             scripts/bootstrap.sh, then open Dovetail again.",
            path.display()
        ))
    }
}

/// Runs cores one after another until the window closes: Home first, then whatever project
/// the user opens.
fn supervise(shell: Arc<Shell>, setup: Option<Setup>, paths: Paths, blocked: Option<String>) {
    let mut next: Option<String> = None;
    loop {
        if *lock(&shell.closing) {
            return;
        }
        let started = match (&next, &setup) {
            (Some(id), Some(setup)) => {
                start_project(setup, id).map(|(link, core)| (link, Running::Project(core)))
            }
            _ => spawn_home(paths.clone(), blocked.clone())
                .map(|(link, home)| (link, Running::Home(home)))
                .map_err(|e| e.to_string()),
        };
        let (link, running) = match started {
            Ok(started) => started,
            Err(why) => {
                tracing::error!("starting a core: {why}");
                if next.take().is_some() {
                    continue; // back to Home
                }
                return;
            }
        };
        *lock(&shell.commands) = Some(link.commands.clone());
        // A window already showing gets a fresh snapshot of the new core.
        if lock(&shell.channel).is_some() {
            let _ = link.commands.send(Command::Resync);
        }
        let opening = pump(&shell, &link);
        *lock(&shell.commands) = None;
        drop(link);
        next = match running {
            Running::Home(home) => home.wait(),
            Running::Project(core) => {
                let switch = core.switch_requested();
                if !core.wait(QUIT_WAIT) {
                    tracing::warn!("the core did not stop within {}s", QUIT_WAIT.as_secs());
                }
                switch.or(opening)
            }
        };
        if next.is_none() {
            return;
        }
    }
}

/// Forwards every message to the frontend until the core is done. Returns the project an
/// [`Event::OpeningProject`] named.
fn pump(shell: &Shell, link: &CoreLink) -> Option<String> {
    let mut opening = None;
    loop {
        match link.messages.recv_timeout(Duration::from_millis(200)) {
            Ok(msg) => {
                if let ServerMsg::Event {
                    event: Event::OpeningProject { id },
                    ..
                } = &msg
                {
                    opening = Some(id.0.clone());
                }
                if let Some(channel) = lock(&shell.channel).as_ref()
                    && let Err(e) = channel.send(msg)
                {
                    tracing::warn!("sending to the window: {e}");
                }
            }
            // Nothing yet; the core lets go of its end when it stops.
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return opening,
        }
    }
}

/// Spawns the core for project `id`.
fn start_project(setup: &Setup, id: &str) -> Result<(CoreLink, CoreHandle), String> {
    let registry = Db::open(&setup.paths, None).map_err(|e| e.to_string())?;
    let project = projects::open(&registry, id).map_err(|e| e.to_string())?;
    drop(registry);
    dovetail_core::spawn(
        projects::Resolution::Registered(project),
        setup.paths.clone(),
        setup.config.clone(),
        setup.options.clone(),
    )
    .map_err(|e| e.to_string())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let paths = Paths::from_env();
    let config = match Config::load(&paths) {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("reading config.toml: {e}");
            Config::default()
        }
    };
    if let Ok(registry) = Db::open(&paths, None)
        && let Err(e) = registry.ensure_tangents().wait()
    {
        tracing::warn!("tangents are off: {e}");
    }
    let (setup, blocked) = match installed_binary() {
        Ok(exe) => {
            let program = config
                .claude
                .clone()
                .map_or_else(|| "claude".into(), |p| p.into_os_string());
            let options = CoreOptions {
                driver: Arc::new(ClaudeDriver::with_program(program.clone())),
                exe,
                tasks_dir: None,
                editor: None,
                claude: Some(program),
                processes: None,
                memory_caps: true,
                check_claude: true,
                scopes: None,
                // `:upgrade` and take-over need a terminal; the GUI sends neither.
                upgrade: None,
                opener: None,
                gh: None,
            };
            let setup = Setup {
                paths: paths.clone(),
                config,
                options,
            };
            (Some(setup), None)
        }
        Err(why) => {
            tracing::warn!("{why}");
            (None, Some(why))
        }
    };

    let shell = Arc::new(Shell::default());
    let supervisor = {
        let shell = shell.clone();
        std::thread::Builder::new()
            .name("dovetail-gui-supervisor".into())
            .spawn(move || supervise(shell, setup, paths, blocked))
            .expect("starting the supervisor thread")
    };
    let supervisor = Mutex::new(Some(supervisor));

    let app = tauri::Builder::default()
        .manage(shell.clone())
        .invoke_handler(tauri::generate_handler![core_send, core_subscribe])
        .build(tauri::generate_context!())
        .expect("building the Tauri app");
    app.run(move |_, event| {
        if let RunEvent::Exit = event {
            // Sessions end with the window; they resume on the next open.
            *lock(&shell.closing) = true;
            if let Some(tx) = lock(&shell.commands).as_ref() {
                let _ = tx.send(Command::Quit {
                    end_sessions: false,
                });
            }
            if let Some(thread) = lock(&supervisor).take() {
                let _ = thread.join();
            }
        }
    });
}
