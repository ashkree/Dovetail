//! Opens the terminal UI on a core.

use std::time::Instant;

use anyhow::{Context, Result};
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
