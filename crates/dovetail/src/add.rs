//! `dovetail add "title !p1 @now"`: quick capture from the shell (B19: socket if the app runs, else SQLite).

use anyhow::{Context, Result, bail};
use dovetail_api::NewTask;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::{projects, socket};
use dovetail_tui::capture;

/// Run the subcommand with the arguments after its name.
///
/// The words are joined and read with the TUI's quick-capture parser, so `!p0`-`!p3`,
/// `@now`/`@next`/`@later` and `#<pr>` work the same. The task goes to the running app over its
/// socket when there is one, so the app shows it at once; otherwise it is written straight to
/// the project's `state.db` (migrating it first).
pub fn run(args: Vec<String>) -> Result<()> {
    let line = args.join(" ");
    let captured = capture::parse(&line);
    if captured.title.is_empty() {
        bail!("usage: dovetail add \"title !p1 @now #123\"");
    }
    let task = NewTask {
        title: captured.title,
        p_level: captured.p_level,
        bucket: captured.bucket,
        notes: String::new(),
        follow_up_of: None,
        pr: captured.pr,
        related: Vec::new(),
    };

    let paths = Paths::from_env();
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let registry = Db::open(&paths, None).context("opening the project registry")?;
    let (project, _) = projects::resolve(&registry, &cwd)?;
    drop(registry);

    let socket_path = paths.socket(&project.id);
    if let Some(id) = socket::try_add_task(&socket_path, task.clone())
        .context("sending the task to the running app")?
    {
        println!("added task {} to {} (app is running)", id.0, project.id);
        return Ok(());
    }

    let db = Db::open(&paths, Some(&project.id)).context("opening the project database")?;
    let created = db.create_task(task, false).wait()?;
    let id = created.first().context("the task was not created")?.id;
    println!("added task {} to {}", id.0, project.id);
    Ok(())
}
