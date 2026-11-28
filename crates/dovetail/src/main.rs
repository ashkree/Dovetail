#![doc = include_str!("../ARCHITECTURE.md")]

mod add;
mod hook;
mod logging;
mod login;
mod mcp;
mod paint;
mod tui;
mod upgrade;

use std::io::{self, BufWriter, Write};

use anyhow::Result;
use clap::{Parser, Subcommand};
use dovetail_mock::{MockConfig, MockCore};

#[derive(Debug, Parser)]
#[command(
    name = "dovetail",
    version,
    about = "Keyboard-driven TUI for running many Claude Code agents on one project",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Open a registered project by id (research projects too).
    #[arg(long, value_name = "ID", conflicts_with = "mock")]
    project: Option<String>,

    /// Use the built-in mock core instead of real sessions.
    #[arg(long)]
    mock: bool,

    /// Print the core's messages as JSON lines instead of opening the UI.
    #[arg(long, requires = "mock")]
    dump: bool,

    /// Mock seed (default 1); the same seed gives the same stream.
    #[arg(long, value_name = "N")]
    seed: Option<u64>,

    /// Mock pace: 1 is real time, 2 twice as fast (default 1).
    #[arg(long, value_name = "X", value_parser = parse_speed)]
    speed: Option<f64>,

    /// Events to print after the snapshot (default 200).
    #[arg(long, value_name = "N")]
    count: Option<usize>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Capture a task from the shell.
    Add(add::Args),
    /// Sign a Claude profile in or list profiles.
    Login(login::Args),
    /// Build origin/main (or this checkout) and install it.
    Upgrade(upgrade::Args),
    /// Claude Code worktree hooks (internal).
    #[command(hide = true)]
    Hook(hook::Args),
}

fn parse_speed(value: &str) -> Result<f64, String> {
    let speed: f64 = value
        .parse()
        .map_err(|e| format!("bad --speed {value}: {e}"))?;
    if !(speed.is_finite() && speed > 0.0) {
        return Err("--speed must be above 0".to_owned());
    }
    Ok(speed)
}

fn dump(seed: u64, count: usize) -> Result<()> {
    let mut core = MockCore::new(MockConfig {
        seed,
        ..MockConfig::default()
    });
    let mut out = BufWriter::new(io::stdout().lock());
    for msg in core.connect() {
        writeln!(out, "{}", serde_json::to_string(&msg)?)?;
    }
    for _ in 0..count {
        let Some(next) = core.next_message() else {
            break;
        };
        writeln!(out, "{}", serde_json::to_string(&next.msg)?)?;
    }
    out.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    // Dispatch `dovetail mcp` before clap parses (D42): keeps per-session MCP bridge startup fast
    // and ensures clap never writes help or errors to the MCP stdio channel.
    if std::env::args_os().nth(1).is_some_and(|a| a == "mcp") {
        return mcp::run();
    }

    let cli = Cli::parse();
    if let Some(command) = cli.command {
        return match command {
            Command::Add(args) => add::run(args),
            Command::Login(args) => login::run(args),
            Command::Upgrade(args) => upgrade::run(args),
            Command::Hook(args) => hook::run(args),
        };
    }

    match (cli.mock, cli.dump) {
        (true, true) => match dump(cli.seed.unwrap_or(1), cli.count.unwrap_or(200)) {
            // Piped into `head` or `cmp`: the reader has seen enough.
            Err(e) if is_broken_pipe(&e) => Ok(()),
            other => other,
        },
        (false, true) => unreachable!("clap enforces --dump requires --mock"),
        (true, false) => {
            logging::init();
            tui::run_mock(cli.seed.unwrap_or(1), cli.speed.unwrap_or(1.0))
        }
        (false, false) => {
            logging::init();
            tui::run_real(cli.project.as_deref())
        }
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("dovetail").chain(args.iter().copied()))
    }

    #[test]
    fn tui_and_mock_flags_parse() {
        let cli = parse(&[]).unwrap();
        assert!(cli.command.is_none());
        assert!(!cli.mock && !cli.dump && cli.project.is_none());

        let cli = parse(&["--project", "dovetail"]).unwrap();
        assert_eq!(cli.project.as_deref(), Some("dovetail"));

        let cli = parse(&["--mock", "--seed", "7", "--speed", "2.5"]).unwrap();
        assert!(cli.mock && !cli.dump);
        assert_eq!(cli.seed, Some(7));
        assert_eq!(cli.speed, Some(2.5));

        let cli = parse(&["--mock", "--dump", "--count", "42"]).unwrap();
        assert!(cli.mock && cli.dump);
        assert_eq!(cli.count, Some(42));
    }

    #[test]
    fn invalid_flag_combinations_fail() {
        assert!(parse(&["--project", "foo", "--mock"]).is_err());
        assert!(parse(&["--dump"]).is_err());
        assert!(parse(&["--mock", "--speed", "0"]).is_err());
        assert!(parse(&["--mock", "--speed", "-1"]).is_err());
        assert!(parse(&["--mock", "--speed", "NaN"]).is_err());
        assert!(parse(&["--mock", "add", "title"]).is_err());
    }

    #[test]
    fn subcommands_parse() {
        let cli = parse(&["login", "--list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Login(login::Args {
                profile: None,
                list: true
            }))
        ));

        let cli = parse(&["login", "work"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Login(login::Args {
                profile: Some(_),
                list: false
            }))
        ));
        assert!(parse(&["login"]).is_err());
        assert!(parse(&["login", "--list", "work"]).is_err());

        let cli = parse(&["upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Upgrade(upgrade::Args { local: false }))
        ));
        let cli = parse(&["upgrade", "--local"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Upgrade(upgrade::Args { local: true }))
        ));

        let cli = parse(&["hook", "worktree-create"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Hook(hook::Args {
                event: hook::HookEvent::WorktreeCreate
            }))
        ));
        let cli = parse(&["hook", "worktree-remove"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Hook(hook::Args {
                event: hook::HookEvent::WorktreeRemove
            }))
        ));
        assert!(parse(&["hook"]).is_err());
        assert!(parse(&["hook", "other"]).is_err());
    }
}
