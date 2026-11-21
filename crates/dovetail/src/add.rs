//! `dovetail add "title !p1 @now"`: quick capture from the shell (B19: socket if the app runs, else SQLite).
//! `--project <id>` names the project, for one that has no folder to run from (a research project).

use anyhow::{Context, Result, bail};
use dovetail_api::task::NewTask;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::{projects, socket};
use dovetail_tui::capture;

const USAGE: &str = "usage: dovetail add [--project <id>] \"title !p1 @now #123\"";

/// Takes `--project <id>` (or `--project=<id>`) out of the arguments; the rest is the task.
fn split_project(args: Vec<String>) -> Result<(Option<String>, Vec<String>)> {
    let mut project = None;
    let mut words = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--project" {
            let id = args.next().context("--project needs a project id")?;
            project = Some(id);
        } else if let Some(id) = arg.strip_prefix("--project=") {
            project = Some(id.to_owned());
        } else {
            words.push(arg);
        }
    }
    if project.as_deref().is_some_and(str::is_empty) {
        bail!("--project needs a project id");
    }
    Ok((project, words))
}

/// Run the subcommand with the arguments after its name.
///
/// The words are joined and read with the TUI's quick-capture parser, so `!p0`-`!p3`,
/// `@now`/`@next`/`@later` and `#<pr>` work the same. The task goes to the running app over its
/// socket when there is one, so the app shows it at once; otherwise it is written straight to
/// the project's `state.db` (migrating it first). Without `--project <id>` the project is the
/// one the current folder belongs to.
pub fn run(args: Vec<String>) -> Result<()> {
    let (project_id, words) = split_project(args)?;
    let line = words.join(" ");
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
    let project = match &project_id {
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
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn project_is_taken_out_of_the_task_words() {
        let (project, words) =
            split_project(args(&["--project", "market", "size", "!p1"])).unwrap();
        assert_eq!(project.as_deref(), Some("market"));
        assert_eq!(words, ["size", "!p1"]);
        let (project, words) = split_project(args(&["size it", "--project=market"])).unwrap();
        assert_eq!(project.as_deref(), Some("market"));
        assert_eq!(words, ["size it"]);
        let (project, words) = split_project(args(&["just a task"])).unwrap();
        assert_eq!((project, words), (None, args(&["just a task"])));
    }

    #[test]
    fn project_without_an_id_is_an_error() {
        assert!(split_project(args(&["task", "--project"])).is_err());
        assert!(split_project(args(&["task", "--project="])).is_err());
    }
}
