//! File logging. Nothing may print while the TUI owns the terminal, so everything, panics
//! included, goes to `~/.local/state/dovetail/` (or `$XDG_STATE_HOME/dovetail/`).

use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

/// Dovetail's state directory for logs and run files.
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("dovetail"))
}

/// Starts logging to `dovetail.log` in the state directory and records panics there. Logging is
/// best-effort: without a usable directory the app still runs, silently.
pub fn init() {
    let Some(dir) = state_dir() else { return };
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
