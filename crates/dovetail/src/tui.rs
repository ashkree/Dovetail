//! Opens the terminal UI on a core.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dovetail_core::config::Config;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
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
    dovetail_tui::run(&mut app, link, clock).context("terminal UI failed")
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
    };
    let (link, core) =
        dovetail_core::spawn(resolution, paths, config, options).context("starting the core")?;
    let clock: Clock = Box::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    });
    let mut app = App::new(TimeZone::system());
    let result = dovetail_tui::run(&mut app, link, clock).context("terminal UI failed");
    // Sessions end with the app (B23): wait until they have exited and been reaped.
    if !core.wait(Duration::from_secs(10)) {
        tracing::warn!("the core did not stop within 10 s");
    }
    result
}
