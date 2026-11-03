//! Dovetail: wires the core (or the mock) to the UI.
//!
//! The TUI arrives in M2; for now `dovetail --mock --dump` prints the mock core's stream.

use std::io::{self, BufWriter, Write};

use anyhow::{Context, Result, bail};
use dovetail_mock::{MockConfig, MockCore};

const USAGE: &str = "usage: dovetail --mock --dump [--seed N] [--count N]

  --mock       use the built-in mock core instead of real sessions
  --dump       print the core's messages as JSON lines instead of opening the UI
  --seed N     mock seed (default 1); the same seed prints the same stream
  --count N    events to print after the snapshot (default 200)";

#[derive(Debug, Default)]
struct Args {
    mock: bool,
    dump: bool,
    seed: Option<u64>,
    count: Option<usize>,
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
    let args = parse_args(std::env::args().skip(1))?;
    match (args.mock, args.dump) {
        (true, true) => match dump(args.seed.unwrap_or(1), args.count.unwrap_or(200)) {
            // Piped into `head` or `cmp`: the reader has seen enough.
            Err(e) if is_broken_pipe(&e) => Ok(()),
            other => other,
        },
        (_, true) => bail!("--dump needs --mock until the real core lands"),
        _ => bail!("the TUI arrives in M2; try `dovetail --mock --dump`\n\n{USAGE}"),
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
