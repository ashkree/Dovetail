//! `dovetail upgrade [--local]` (ADR 0014): build the next binary into the staging folder, then
//! install it at once when no app runs, or hand it to the running app, which offers "Upgrade
//! ready" in Needs you.
//!
//! The same staging, build and swap code as `:upgrade` in the app (`dovetail_core::upgrade`).
//! Output goes to the terminal: this is the command line, no TUI is running.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::projects;
use dovetail_core::socket;
use dovetail_core::upgrade::{
    BuildRequest, Source, Staging, Swap, SwapError, SwapGuard, SystemExec, UpgradeError, build,
    install_path, install_staged, is_installed_binary, running_blockers,
};

const USAGE: &str = "usage: dovetail upgrade [--local]";

/// Run the subcommand with the arguments after its name.
pub fn run(args: Vec<String>) -> Result<()> {
    let source = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Source::OriginMain,
        ["--local"] => Source::Local,
        _ => bail!("{USAGE}"),
    };

    let paths = Paths::from_env();
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let registry = Db::open(&paths, None).context("opening the project registry")?;
    let project = projects::require(&registry, &cwd)?;
    drop(registry);
    // A folder never resolves to a research project, but say so rather than assume it.
    let Some(repo) = project.repo_path.clone().filter(|_| !project.is_research()) else {
        bail!(
            "{} is a research project: it has no repo to build from",
            project.id
        );
    };

    let install = install_path()?;
    let exe = std::env::current_exe().context("finding the dovetail binary")?;
    if !is_installed_binary(&exe, &install) {
        return Err(UpgradeError::NotInstalled(install).into());
    }

    // An app of this project answers on its socket: it gets the build. Any other process running
    // the installed binary, or another project's app, means nothing may be replaced now.
    let socket_path = paths.socket(&project.id);
    let blockers = running_blockers(
        &install,
        &paths.runtime,
        Path::new("/proc"),
        std::process::id(),
    );
    let app_here = blockers.iter().any(|b| Path::new(b) == socket_path);
    if !app_here && !blockers.is_empty() {
        bail!(
            "close Dovetail first: it is running ({})",
            blockers.join(", ")
        );
    }

    let staging = Staging::new(&paths);
    let request = BuildRequest {
        repo,
        source,
        config_dir: paths.config.clone(),
        recipe: Some(paths.recipe(&project.id)),
    };
    println!(
        "building {} (a release build; this takes a few minutes)",
        match source {
            Source::OriginMain => "origin/main",
            Source::Local => "this checkout",
        }
    );
    // Ctrl-C stops the build and reaps cargo instead of leaving it running.
    let cancel = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, cancel.clone()).context("catching Ctrl-C")?;
    }
    let ready = build(&SystemExec, &staging, &request, &cancel, now_ms()).inspect_err(|_| {
        if !cancel.load(Ordering::Relaxed) {
            eprintln!("build output: {}", staging.log().display());
        }
    })?;
    println!("built {}", ready.source);
    if !ready.differing.is_empty() {
        println!(
            "installed copies differ from the new build's and are kept: {}\n\
             `scripts/bootstrap.sh --force` takes the new ones",
            ready.differing.join(", ")
        );
    }

    if app_here {
        match socket::try_staged_upgrade(&socket_path)
            .context("handing the build to the running app")?
        {
            true => println!("handed to the running Dovetail: \"Upgrade ready\" is in Needs you"),
            false => bail!("the running Dovetail went away; run `dovetail upgrade` again"),
        }
        return Ok(());
    }

    let guard = SwapGuard {
        runtime: paths.runtime.clone(),
        proc_root: "/proc".into(),
        own_pid: std::process::id(),
        wait: std::time::Duration::ZERO,
    };
    match install_staged(&staging, &install, &guard) {
        Ok(Swap::Installed(ready)) => {
            println!("installed {} ({})", install.display(), ready.source)
        }
        Ok(Swap::NothingStaged) => bail!("the staged build disappeared"),
        Err(SwapError::Blocked(running)) => {
            bail!(
                "close Dovetail first: it is running ({})",
                running.join(", ")
            )
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
