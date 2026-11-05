//! Opens the terminal UI on a core.

use std::time::Instant;

use anyhow::{Context, Result};
use dovetail_core::config::Config;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::projects;
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

/// Runs the TUI on the real core for the repo in the current directory (B25: an unregistered repo
/// is registered on the spot; outside any repo the last opened project is used).
pub fn run_real() -> Result<()> {
    let paths = Paths::from_env();
    let config = Config::load(&paths).context("reading config.toml")?;
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let registry = Db::open(&paths, None).context("opening the project registry")?;
    let (project, registered) = projects::resolve(&registry, &cwd)?;
    drop(registry);
    if registered {
        tracing::info!(
            "registered {} as project {}",
            project.repo_path.display(),
            project.id
        );
    }
    let link = dovetail_core::spawn(project, paths, config).context("starting the core")?;
    let clock: Clock = Box::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    });
    let mut app = App::new(TimeZone::system());
    dovetail_tui::run(&mut app, link, clock).context("terminal UI failed")
}
