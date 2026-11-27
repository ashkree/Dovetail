//! Home: the GUI's first screen, with no project open (ADR 0023, point 6).
//!
//! Home needs only the project list, so it is served by this small loop on its own thread
//! instead of the whole core: the core always has a project, and none of its paths has to learn
//! to run without one. It speaks the same [`CoreLink`] as the core: `Hello`, then a [`Snapshot`]
//! with `project: None` and every registered project, then events. Picking a project answers
//! [`ProjectEvent::Opening`] and ends the loop, as the core does when the user switches project;
//! the caller then spawns the core for it.
//!
//! Home also creates projects and changes them (ADR 0021): the five project commands
//! ([`crate::project_admin::ProjectOp`]) touch only the registry, so Home serves them itself and
//! answers each with [`ProjectEvent::Upserted`]. Each project's counts come from its `state.db`,
//! read without migrating it.

use std::sync::mpsc;
use std::thread::JoinHandle;

use dovetail_api::activity::status::{Warning, WarningId};
use dovetail_api::app::{AppCommand, AppEvent};
use dovetail_api::tracking::project::{ProjectCommand, ProjectEvent};
use dovetail_api::{Command, CoreLink, Event, PROTOCOL_VERSION, ServerMsg, Snapshot};

use crate::CoreError;
use crate::config::Config;
use crate::db::{Db, now_ms};
use crate::paths::Paths;
use crate::project_admin::{self, ProjectOp};
use crate::projects::Project;

/// The id of the warning Home shows when sessions cannot start (the reason is its text).
pub const NO_SESSIONS_WARNING: &str = "no-sessions";

/// A running Home loop. Wait for it with [`HomeHandle::wait`].
#[derive(Debug)]
pub struct HomeHandle {
    thread: JoinHandle<Option<String>>,
}

impl HomeHandle {
    /// Waits for the loop to end. Returns the project the user picked, or `None` when the UI
    /// quit or let go of its end of the link.
    pub fn wait(self) -> Option<String> {
        self.thread.join().ok().flatten()
    }
}

/// Starts Home for the registry under `paths`. `blocked`: why no session can start, shown as a
/// warning; picking a project is then refused with that reason.
pub fn spawn_home(
    paths: Paths,
    blocked: Option<String>,
) -> Result<(CoreLink, HomeHandle), CoreError> {
    let db = Db::open(&paths, None)?;
    // The accounts a project may pick; without a readable config.toml, none.
    let config = Config::load(&paths).unwrap_or_else(|e| {
        tracing::warn!("Home: reading config.toml: {e}");
        Config::default()
    });
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (message_tx, message_rx) = mpsc::channel::<ServerMsg>();
    let thread = std::thread::Builder::new()
        .name("dovetail-home".into())
        .spawn(move || run(&db, &paths, &config, blocked, &command_rx, &message_tx))
        .map_err(CoreError::Thread)?;
    Ok((
        CoreLink {
            commands: command_tx,
            messages: message_rx,
        },
        HomeHandle { thread },
    ))
}

/// `project` as Home lists it, with its counts (zero, logged, when they cannot be read).
fn summary(
    db: &Db,
    paths: &Paths,
    project: &Project,
) -> dovetail_api::tracking::project::ProjectSummary {
    let mut summary = project.summary(paths);
    match db.project_counts(paths.state_db(&project.id)).wait() {
        Ok(counts) => summary.counts = counts,
        Err(e) => tracing::warn!(project = %project.id, "Home: reading its counts: {e}"),
    }
    summary
}

/// One project command, answered with the changed project or why it failed.
fn project_op(db: &Db, paths: &Paths, config: &Config, op: ProjectOp) -> Event {
    let creating = matches!(op, ProjectOp::Create { .. });
    let done = op
        .start(db, paths, config)
        .and_then(|pending| pending.wait().map_err(Into::into));
    match done {
        Ok(project) => {
            if creating && let Err(e) = project_admin::write_starter_recipe(paths, config, &project)
            {
                tracing::warn!(project = %project.id, "Home: writing its starter recipe: {e}");
            }
            Event::Project(ProjectEvent::Upserted(summary(db, paths, &project)))
        }
        Err(e) => Event::App(AppEvent::CommandFailed {
            message: e.to_string(),
        }),
    }
}

fn run(
    db: &Db,
    paths: &Paths,
    config: &Config,
    blocked: Option<String>,
    commands: &mpsc::Receiver<Command>,
    out: &mpsc::Sender<ServerMsg>,
) -> Option<String> {
    let mut seq = 0;
    let snapshot = |seq: u64| -> Snapshot {
        let projects = db.list_projects().wait().unwrap_or_else(|e| {
            tracing::warn!("Home: listing projects: {e}");
            Vec::new()
        });
        let mut warnings = Vec::new();
        if let Some(why) = &blocked {
            warnings.push(Warning {
                id: WarningId(NO_SESSIONS_WARNING.into()),
                text: why.clone(),
            });
        }
        Snapshot {
            seq,
            now_ms: now_ms(),
            project: None,
            projects: projects.iter().map(|p| summary(db, paths, p)).collect(),
            warnings,
            effort_limit: config.max_efforts,
            ..Snapshot::default()
        }
    };
    out.send(ServerMsg::Hello {
        protocol: PROTOCOL_VERSION,
    })
    .ok()?;
    out.send(ServerMsg::Snapshot(snapshot(seq))).ok()?;
    for command in commands {
        if ProjectOp::matches(&command) {
            if let Some(op) = ProjectOp::parse(command) {
                seq += 1;
                let event = project_op(db, paths, config, op);
                out.send(ServerMsg::Event { seq, event }).ok()?;
            }
            continue;
        }
        let event = match command {
            Command::App(AppCommand::Resync) => {
                out.send(ServerMsg::Snapshot(snapshot(seq))).ok()?;
                continue;
            }
            Command::App(AppCommand::Quit { .. }) => return None,
            Command::Project(ProjectCommand::Open { id }) => {
                match (&blocked, db.get_project(id.0.clone()).wait()) {
                    (Some(why), _) => Event::App(AppEvent::CommandFailed {
                        message: why.clone(),
                    }),
                    (None, Ok(Some(_))) => {
                        seq += 1;
                        let _ = out.send(ServerMsg::Event {
                            seq,
                            event: Event::Project(ProjectEvent::Opening { id: id.clone() }),
                        });
                        return Some(id.0);
                    }
                    (None, Ok(None)) => Event::App(AppEvent::CommandFailed {
                        message: format!("there is no project {}", id.0),
                    }),
                    (None, Err(e)) => Event::App(AppEvent::CommandFailed {
                        message: e.to_string(),
                    }),
                }
            }
            _ => Event::App(AppEvent::CommandFailed {
                message: "open a project first".into(),
            }),
        };
        seq += 1;
        out.send(ServerMsg::Event { seq, event }).ok()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dovetail_api::app::{AppCommand, AppEvent};
    use dovetail_api::code::worktree::WorktreeCommand;
    use dovetail_api::tracking::project::{ProjectCommand, ProjectEvent, ProjectId};

    use super::*;

    fn paths(dir: &std::path::Path) -> Paths {
        Paths {
            data: dir.join("data"),
            config: dir.join("config"),
            state: dir.join("state"),
            runtime: dir.join("run"),
        }
    }

    fn next(link: &CoreLink) -> ServerMsg {
        link.messages
            .recv_timeout(Duration::from_secs(5))
            .expect("a message")
    }

    #[test]
    fn home_lists_projects_with_none_open_and_ends_on_a_pick() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open(&paths, None).unwrap();
        db.register_research("notes".into(), "Notes".into())
            .wait()
            .unwrap();
        drop(db);

        let (link, home) = spawn_home(paths, None).unwrap();
        assert!(matches!(next(&link), ServerMsg::Hello { .. }));
        let ServerMsg::Snapshot(snapshot) = next(&link) else {
            panic!("a snapshot");
        };
        assert_eq!(snapshot.project, None);
        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].name, "Notes");

        // Anything but opening, resyncing or quitting is refused, and numbered.
        link.commands
            .send(Command::Worktree(WorktreeCommand::Refresh))
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event {
                seq: 1,
                event: Event::App(AppEvent::CommandFailed { .. })
            }
        ));
        link.commands
            .send(Command::Project(ProjectCommand::Open {
                id: ProjectId("nope".into()),
            }))
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event {
                seq: 2,
                event: Event::App(AppEvent::CommandFailed { .. })
            }
        ));
        link.commands
            .send(Command::App(AppCommand::Resync))
            .unwrap();
        assert!(matches!(next(&link), ServerMsg::Snapshot(s) if s.seq == 2));

        link.commands
            .send(Command::Project(ProjectCommand::Open {
                id: ProjectId("notes".into()),
            }))
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event {
                seq: 3,
                event: Event::Project(ProjectEvent::Opening { .. })
            }
        ));
        assert_eq!(home.wait().as_deref(), Some("notes"));
    }

    fn git_repo(dir: &std::path::Path) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git init");
        dir.canonicalize().unwrap()
    }

    fn upserted(link: &CoreLink) -> dovetail_api::tracking::project::ProjectSummary {
        match next(link) {
            ServerMsg::Event {
                event: Event::Project(ProjectEvent::Upserted(p)),
                ..
            } => p,
            other => panic!("expected a project, got {other:?}"),
        }
    }

    fn failed(link: &CoreLink) -> String {
        match next(link) {
            ServerMsg::Event {
                event: Event::App(AppEvent::CommandFailed { message }),
                ..
            } => message,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    /// ADR 0021: Home creates a project, adds and removes repos, renames it and sets its
    /// account; each change comes back as the whole project.
    #[test]
    fn home_creates_projects_and_changes_their_repos() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let api = git_repo(&dir.path().join("code/shop-api"));
        let web = git_repo(&dir.path().join("code/shop-web"));
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("config.toml"),
            format!(
                "[profiles]\nwork = \"{}\"\n",
                dir.path().join("claude-work").display()
            ),
        )
        .unwrap();

        let (link, home) = spawn_home(paths.clone(), None).unwrap();
        next(&link);
        next(&link);

        link.commands
            .send(Command::Project(ProjectCommand::Create {
                name: "Shop front".into(),
                account: None,
                repo: Some(api.join(".")),
            }))
            .unwrap();
        let shop = upserted(&link);
        assert_eq!(shop.id.0, "shop-front");
        assert_eq!(
            shop.kind,
            dovetail_api::tracking::project::ProjectKind::Code
        );
        assert_eq!(shop.repos.len(), 1);
        assert_eq!(shop.repos[0].name, "shop-api");
        assert_eq!(shop.repos[0].path, api);
        assert!(paths.recipe("shop-front").exists(), "starter recipe");

        link.commands
            .send(Command::Project(ProjectCommand::AddRepo {
                project: shop.id.clone(),
                path: web.clone(),
                name: Some("Web".into()),
            }))
            .unwrap();
        let two = upserted(&link);
        let names: Vec<&str> = two.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["shop-api", "web"]);

        // The same repo cannot join twice, here or in another project.
        link.commands
            .send(Command::Project(ProjectCommand::AddRepo {
                project: shop.id.clone(),
                path: web.join("."),
                name: None,
            }))
            .unwrap();
        assert!(failed(&link).contains("already a repo"));
        // A folder outside git is refused.
        let plain = dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        link.commands
            .send(Command::Project(ProjectCommand::AddRepo {
                project: shop.id.clone(),
                path: plain,
                name: None,
            }))
            .unwrap();
        assert!(failed(&link).contains("not inside a git repository"));

        // A slice works in `web` (its project is not open): that repo stays.
        drop(Db::open(&paths, Some("shop-front")).unwrap());
        rusqlite::Connection::open(paths.state_db("shop-front"))
            .unwrap()
            .execute(
                "INSERT INTO agents (id, kind, name, status, permission_mode, created_ms, repo_id) \
                 VALUES ('slice-ui', 'slice', 'ui', 'idle', 'auto', 1, ?1)",
                [two.repos[1].id.0],
            )
            .unwrap();
        link.commands
            .send(Command::Project(ProjectCommand::RemoveRepo {
                project: shop.id.clone(),
                repo: two.repos[1].id,
            }))
            .unwrap();
        assert!(failed(&link).contains("open slices (ui)"));
        link.commands
            .send(Command::Project(ProjectCommand::RemoveRepo {
                project: shop.id.clone(),
                repo: two.repos[0].id,
            }))
            .unwrap();
        assert_eq!(upserted(&link).repos.len(), 1);

        link.commands
            .send(Command::Project(ProjectCommand::Rename {
                id: shop.id.clone(),
                name: "Shop".into(),
            }))
            .unwrap();
        assert_eq!(upserted(&link).name, "Shop");
        link.commands
            .send(Command::Project(ProjectCommand::SetAccount {
                id: shop.id.clone(),
                account: Some("home".into()),
            }))
            .unwrap();
        assert!(failed(&link).contains("no profile `home`"));
        link.commands
            .send(Command::Project(ProjectCommand::SetAccount {
                id: shop.id.clone(),
                account: Some("work".into()),
            }))
            .unwrap();
        assert_eq!(upserted(&link).account.as_deref(), Some("work"));

        // No repos: research-only.
        link.commands
            .send(Command::Project(ProjectCommand::Create {
                name: "Market notes".into(),
                account: None,
                repo: None,
            }))
            .unwrap();
        let notes = upserted(&link);
        assert!(notes.repos.is_empty());
        assert_eq!(
            notes.kind,
            dovetail_api::tracking::project::ProjectKind::Research
        );
        link.commands
            .send(Command::Project(ProjectCommand::Create {
                name: "  ".into(),
                account: None,
                repo: None,
            }))
            .unwrap();
        assert!(failed(&link).contains("needs a name"));

        // A fresh snapshot lists both, with counts.
        link.commands
            .send(Command::App(AppCommand::Resync))
            .unwrap();
        let ServerMsg::Snapshot(snapshot) = next(&link) else {
            panic!("a snapshot");
        };
        assert_eq!(snapshot.projects.len(), 2);
        drop(link);
        assert_eq!(home.wait(), None);
    }

    #[test]
    fn with_sessions_blocked_home_says_why_and_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open(&paths, None).unwrap();
        db.register_research("notes".into(), "Notes".into())
            .wait()
            .unwrap();
        drop(db);

        let why = "the dovetail binary is not installed".to_string();
        let (link, home) = spawn_home(paths, Some(why.clone())).unwrap();
        next(&link);
        let ServerMsg::Snapshot(snapshot) = next(&link) else {
            panic!("a snapshot");
        };
        assert_eq!(snapshot.warnings[0].text, why);
        link.commands
            .send(Command::Project(ProjectCommand::Open {
                id: ProjectId("notes".into()),
            }))
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event { event: Event::App(AppEvent::CommandFailed { message }), .. } if message == why
        ));
        drop(link);
        assert_eq!(home.wait(), None);
    }
}
