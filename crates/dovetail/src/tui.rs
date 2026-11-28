//! Opens the terminal UI on a core.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dovetail_core::app::upgrade;
use dovetail_core::engine::{self, CoreOptions, UpgradeOptions};
use dovetail_core::sessions::drivers::Drivers;
use dovetail_core::settings::config::Config;
use dovetail_core::settings::paths::Paths;
use dovetail_core::storage::Db;
use dovetail_core::tracking::projects;
use dovetail_driver::{AgyDriver, ClaudeDriver};
use dovetail_mock::{MockConfig, spawn};
use dovetail_tui::{App, Clock};
use jiff::tz::TimeZone;

/// Runs the TUI on the mock core until the user quits.
///
/// The UI owns "now": here it starts at the mock's virtual start time and advances with real
/// time scaled by `speed`, so times in the UI match the mock's stream.
pub fn run_mock(seed: u64, speed: f64) -> Result<()> {
    let config = MockConfig {
        seed,
        ..MockConfig::default()
    };
    let start_ms = config.start_ms;
    let link = spawn(config, speed);
    let began = Instant::now();
    let clock: Clock =
        Box::new(move || start_ms + (began.elapsed().as_secs_f64() * 1_000.0 * speed) as i64);
    let mut app = App::new(TimeZone::system());
    dovetail_tui::run(&mut app, link, clock, Some(crate::paint::painter()))
        .context("terminal UI failed")
}

/// Runs the TUI on the real core for `project` (an id, from `--project` or the project
/// switcher), else for the folder the user is in. Nothing is registered on the way (ADR 0005): a
/// folder that is not a project yet runs from memory until `:project add`.
pub fn run_real(project: Option<&str>) -> Result<()> {
    let paths = Paths::from_env();
    let config = Config::load(&paths).context("reading config.toml")?;
    let registry = Db::open(&paths, None).context("opening the project registry")?;
    // Tangents is always there, so `--project tangents` opens it on the very first run too
    // (ADR 0017); the core warns in the status bar when it cannot be.
    if let Err(e) = registry.ensure_tangents().wait() {
        tracing::warn!("tangents are off: {e}");
    }
    let resolution = match project {
        Some(id) => projects::Resolution::Registered(projects::open(&registry, id)?),
        None => {
            let cwd = std::env::current_dir().context("finding the current directory")?;
            projects::locate(&registry, &cwd)?
        }
    };
    drop(registry);
    let program = config
        .claude
        .clone()
        .map_or_else(|| "claude".into(), |p| p.into_os_string());
    let mut drivers = Drivers::new();
    drivers.insert(Arc::new(ClaudeDriver::with_program(program.clone())));
    // No agy binary: no agy driver, so no agy rows, usage group or model refresh.
    if let Some(agy) = AgyDriver::if_installed() {
        drivers.insert(Arc::new(agy));
    }
    let options = CoreOptions {
        drivers,
        exe: std::env::current_exe().context("finding the dovetail binary")?,
        tasks_dir: None,
        editor: None,
        // Take-over runs the same binary as headless sessions.
        claude: Some(program),
        processes: None,
        memory_caps: true,
        check_claude: true,
        scopes: None,
        // `:upgrade` (ADR 0014); without a HOME there is no install path to replace.
        upgrade: upgrade::install_path().ok().map(|install| UpgradeOptions {
            exec: Arc::new(upgrade::SystemExec),
            install,
        }),
        // `gx` in reading mode: `$BROWSER`, else `xdg-open`.
        opener: None,
        gh: None,
    };
    let (link, core) =
        engine::spawn(resolution, paths.clone(), config, options).context("starting the core")?;
    let clock: Clock = Box::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    });
    let mut app = App::new(TimeZone::system());
    let result = dovetail_tui::run(&mut app, link, clock, Some(crate::paint::painter()))
        .context("terminal UI failed");
    let restart = core.restart_requested();
    let switch = core.switch_requested();
    // Sessions end with the app (B23): wait until they have exited and been reaped.
    if !core.wait(Duration::from_secs(10)) {
        tracing::warn!("the core did not stop within 10 s");
    }
    // The terminal is ours again (and nothing runs): a staged upgrade goes in now (ADR 0014).
    finish_upgrade(&paths, restart);
    if let (Ok(()), Some(id), false) = (&result, switch, restart) {
        open_project(&paths, &id);
    }
    result
}

/// After the app has shut down because the user picked another project (B24): start this binary
/// again on it, in this terminal, with the same flags. A fresh process gets a fresh core, socket
/// and input thread; `--project` names the project, so it opens research projects too.
fn open_project(paths: &Paths, id: &str) {
    use std::os::unix::process::CommandExt;
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    let mut old = std::env::args_os().skip(1);
    while let Some(arg) = old.next() {
        if arg == "--project" {
            old.next();
        } else if arg.to_str().is_some_and(|s| s.starts_with("--project=")) {
            // Drop `--project=<id>` as well as `--project <id>`.
        } else {
            args.push(arg);
        }
    }
    args.extend(["--project".into(), id.into()]);
    let mut command =
        std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "dovetail".into()));
    command.args(args);
    // A code project opens in its folder, as if `dovetail` had been run there.
    let folder = Db::open(paths, None)
        .ok()
        .and_then(|db| db.get_project(id.to_owned()).wait().ok().flatten())
        .and_then(|p| p.first_repo().map(|r| r.path.clone()));
    if let Some(folder) = folder {
        command.current_dir(folder);
    }
    let error = command.exec();
    eprintln!("could not open project {id}: {error}; run `dovetail --project {id}`");
}

/// After the app has shut down: swap a staged build over the installed binary, and when the user
/// asked for the restart, `exec` it in this terminal with the same arguments (the folder is the
/// same too, so sessions resume as after any restart). A failure leaves the old binary in place
/// and the build staged, and says so on the restored terminal.
fn finish_upgrade(paths: &Paths, restart: bool) {
    let Ok(install) = upgrade::install_path() else {
        return;
    };
    let staging = upgrade::Staging::new(paths);
    if staging.ready().is_none() {
        return;
    }
    // A dev build never replaces the installed binary.
    let is_installed =
        std::env::current_exe().is_ok_and(|exe| upgrade::is_installed_binary(&exe, &install));
    if !is_installed {
        return;
    }
    let guard = upgrade::SwapGuard {
        runtime: paths.runtime.clone(),
        proc_root: "/proc".into(),
        own_pid: std::process::id(),
        wait: upgrade::HELPER_EXIT_WAIT,
    };
    match upgrade::install_staged(&staging, &install, &guard) {
        Ok(upgrade::Swap::Installed(ready)) if restart => {
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(&install)
                .args(std::env::args_os().skip(1))
                .exec();
            eprintln!(
                "upgraded to {}, but could not start it: {error}",
                ready.source
            );
        }
        Ok(upgrade::Swap::Installed(ready)) => {
            eprintln!(
                "upgraded to {}; the next dovetail you open runs it",
                ready.source
            );
        }
        Ok(upgrade::Swap::NothingStaged) => {}
        Err(error) => {
            tracing::warn!("upgrade not installed: {error}");
            eprintln!("upgrade not installed, the build stays staged: {error}");
        }
    }
}
