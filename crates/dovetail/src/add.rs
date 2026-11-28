//! `dovetail add "title !p1 @now"`: quick capture from the shell (B19: socket if the app runs, else SQLite).
//! `--project <id>` names the project, for one that has no folder to run from (a research project).

use anyhow::{Context, Result, bail};
use dovetail_api::tracking::task::NewTask;
use dovetail_core::mcp::socket;
use dovetail_core::settings::paths::Paths;
use dovetail_core::storage::Db;
use dovetail_core::tracking::projects;
use dovetail_tui::capture;

const USAGE: &str = "usage: dovetail add [--project <id>] \"title !p1 @now #123\"";

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Project id (for a project with no folder to run from, e.g. a research project).
    #[arg(long, value_name = "ID", value_parser = non_empty_project)]
    pub project: Option<String>,
    /// Task title and optional quick-capture tags (`!p0`-`!p3`, `@now`/`@next`/`@later`, `#123`).
    #[arg(required = true, value_name = "TEXT")]
    pub words: Vec<String>,
}

fn non_empty_project(s: &str) -> Result<String, String> {
    if s.is_empty() {
        Err("--project needs a project id".to_owned())
    } else {
        Ok(s.to_owned())
    }
}

/// Run the subcommand with the parsed arguments.
///
/// The words are joined and read with the TUI's quick-capture parser, so `!p0`-`!p3`,
/// `@now`/`@next`/`@later` and `#<pr>` work the same. The task goes to the running app over its
/// socket when there is one, so the app shows it at once; otherwise it is written straight to
/// the project's `state.db` (migrating it first). Without `--project <id>` the project is the
/// one the current folder belongs to.
pub fn run(args: Args) -> Result<()> {
    let line = args.words.join(" ");
    let captured = capture::parse(&line);
    if captured.title.is_empty() {
        bail!("{USAGE}");
    }
    let task = NewTask {
        title: captured.title,
        p_level: captured.p_level,
        bucket: captured.bucket,
        notes: String::new(),
        follow_up_of: None,
        pr: captured.pr,
        related: Vec::new(),
        effort: Default::default(),
        criterion: None,
    };

    let paths = Paths::from_env();
    let registry = Db::open(&paths, None).context("opening the project registry")?;
    // ADR 0005: only a registered project takes tasks; nothing is registered here.
    let project = match &args.project {
        Some(id) => projects::find(&registry, id)?,
        None => {
            let cwd = std::env::current_dir().context("finding the current directory")?;
            projects::require(&registry, &cwd)?
        }
    };
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    fn parse(words: &[&str]) -> Result<Args, clap::Error> {
        let argv = std::iter::once("add").chain(words.iter().copied());
        TestCli::try_parse_from(argv).map(|c| c.args)
    }

    #[test]
    fn project_is_taken_out_of_the_task_words() {
        let parsed = parse(&["--project", "market", "size", "!p1"]).unwrap();
        assert_eq!(parsed.project.as_deref(), Some("market"));
        assert_eq!(parsed.words, ["size", "!p1"]);

        let parsed = parse(&["size it", "--project=market"]).unwrap();
        assert_eq!(parsed.project.as_deref(), Some("market"));
        assert_eq!(parsed.words, ["size it"]);

        let parsed = parse(&["just a task"]).unwrap();
        assert_eq!(parsed.project, None);
        assert_eq!(parsed.words, ["just a task"]);
    }

    #[test]
    fn project_without_an_id_is_an_error() {
        assert!(parse(&["task", "--project"]).is_err());
        assert!(parse(&["task", "--project="]).is_err());
    }
}
