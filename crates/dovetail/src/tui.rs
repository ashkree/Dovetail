//! Opens the terminal UI on a core.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dovetail_core::config::Config;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::service::UpgradeOptions;
use dovetail_core::upgrade;
use dovetail_core::{CoreOptions, projects};
use dovetail_driver::ClaudeDriver;
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

/// Runs the TUI on the real core for the folder the user is in. Nothing is registered on the way
/// (ADR 0005): a folder that is not a project yet runs from memory until `:project add`.
pub fn run_real() -> Result<()> {
    let paths = Paths::from_env();
    let config = Config::load(&paths).context("reading config.toml")?;
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let registry = Db::open(&paths, None).context("opening the project registry")?;
    let resolution = projects::locate(&registry, &cwd)?;
    drop(registry);
    let program = config
        .claude
        .clone()
        .map_or_else(|| "claude".into(), |p| p.into_os_string());
    let options = CoreOptions {
        driver: Arc::new(ClaudeDriver::with_program(program.clone())),
        exe: std::env::current_exe().context("finding the dovetail binary")?,
        tasks_dir: None,
        editor: None,
        // Take-over runs the same binary as headless sessions.
        claude: Some(program),
        processes: None,
        memory_caps: true,
        scopes: None,
        // `:upgrade` (ADR 0014); without a HOME there is no install path to replace.
        upgrade: upgrade::install_path().ok().map(|install| UpgradeOptions {
            exec: Arc::new(upgrade::SystemExec),
            install,
        }),
    };
    let (link, core) = dovetail_core::spawn(resolution, paths.clone(), config, options)
        .context("starting the core")?;
    let clock: Clock = Box::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    });
    let mut app = App::new(TimeZone::system());
    let result = dovetail_tui::run(&mut app, link, clock, Some(crate::paint::painter()))
        .context("terminal UI failed");
    let restart = core.restart_requested();
    // Sessions end with the app (B23): wait until they have exited and been reaped.
    if !core.wait(Duration::from_secs(10)) {
        tracing::warn!("the core did not stop within 10 s");
    }
    // The terminal is ours again (and nothing runs): a staged upgrade goes in now (ADR 0014).
    finish_upgrade(&paths, restart);
    result
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
