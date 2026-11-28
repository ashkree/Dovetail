//! File logging. Nothing may print while the TUI owns the terminal, so everything, panics
//! included, goes to the state directory (`Paths::state`: `~/.local/state/dovetail/`, or
//! `$XDG_STATE_HOME/dovetail/`, or `$DOVETAIL_STATE_DIR`).

use dovetail_core::settings::paths::Paths;
use tracing_subscriber::EnvFilter;

/// Starts logging to `dovetail.log` in the state directory and records panics there. Logging is
/// best-effort: without a usable directory the app still runs, silently.
pub fn init() {
    let dir = Paths::from_env().state;
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let writer = tracing_appender::rolling::daily(dir, "dovetail.log");
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // A second init (in tests) is harmless.
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        previous(info);
    }));
}
