//! Dovetail: wires the core (or the mock) to the UI.
//!
//! `dovetail --mock` opens the TUI on the mock core; `--dump` prints the core's stream instead.

mod add;
mod hook;
mod logging;
mod login;
mod mcp;
mod paint;
mod tui;
mod upgrade;

use std::io::{self, BufWriter, Write};

use anyhow::{Context, Result, bail};
use dovetail_mock::{MockConfig, MockCore};

const USAGE: &str = "usage: dovetail                     open the TUI for the repo you're in
       dovetail add \"title !p1 @now\"  capture a task
       dovetail login <profile>     sign a Claude profile in (--list: show profiles)
       dovetail upgrade [--local]   build origin/main (or this checkout) and install it
       dovetail --mock [--seed N] [--speed X]
       dovetail --mock --dump [--seed N] [--count N]

  --mock       use the built-in mock core instead of real sessions
  --dump       print the core's messages as JSON lines instead of opening the UI
  --seed N     mock seed (default 1); the same seed gives the same stream
  --speed X    mock pace: 1 is real time, 2 twice as fast (default 1)
  --count N    events to print after the snapshot (default 200)";

#[derive(Debug, Default)]
struct Args {
    mock: bool,
    dump: bool,
    seed: Option<u64>,
    count: Option<usize>,
    speed: Option<f64>,
}

fn parse_args(raw: impl IntoIterator<Item = String>) -> Result<Args> {
    let mut args = Args::default();
    let mut raw = raw.into_iter();
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "--mock" => args.mock = true,
            "--dump" => args.dump = true,
            "--seed" => {
                let value = raw.next().context("--seed needs a number")?;
                args.seed = Some(
                    value
                        .parse()
                        .with_context(|| format!("bad --seed {value}"))?,
                );
            }
            "--count" => {
                let value = raw.next().context("--count needs a number")?;
                args.count = Some(
                    value
                        .parse()
                        .with_context(|| format!("bad --count {value}"))?,
                );
            }
            "--speed" => {
                let value = raw.next().context("--speed needs a number")?;
                let speed: f64 = value
                    .parse()
                    .with_context(|| format!("bad --speed {value}"))?;
                if !(speed.is_finite() && speed > 0.0) {
                    bail!("--speed must be above 0");
                }
                args.speed = Some(speed);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("dovetail {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
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
    let mut raw: Vec<String> = std::env::args().skip(1).collect();
    // Subcommands come first; everything else is the TUI's flags.
    match raw.first().map(String::as_str) {
        Some("add") => return add::run(raw.split_off(1)),
        Some("mcp") => return mcp::run(raw.split_off(1)),
        Some("hook") => return hook::run(raw.split_off(1)),
        Some("login") => return login::run(raw.split_off(1)),
        Some("upgrade") => return upgrade::run(raw.split_off(1)),
        _ => {}
    }
    let args = parse_args(raw)?;
    match (args.mock, args.dump) {
        (true, true) => match dump(args.seed.unwrap_or(1), args.count.unwrap_or(200)) {
            // Piped into `head` or `cmp`: the reader has seen enough.
            Err(e) if is_broken_pipe(&e) => Ok(()),
            other => other,
        },
        (false, true) => bail!("--dump needs --mock"),
        (true, false) => {
            logging::init();
            tui::run_mock(args.seed.unwrap_or(1), args.speed.unwrap_or(1.0))
        }
        (false, false) => {
            logging::init();
            tui::run_real()
        }
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
