//! Home: the GUI's first screen, with no project open (ADR 0023, point 6).
//!
//! Home needs only the project list, so it is served by this small loop on its own thread
//! instead of the whole core: the core always has a project, and none of its paths has to learn
//! to run without one. It speaks the same [`CoreLink`] as the core: `Hello`, then a [`Snapshot`]
//! with `project: None` and every registered project, then events. Picking a project answers
//! [`Event::OpeningProject`] and ends the loop, as the core does when the user switches project;
//! the caller then spawns the core for it.

use std::sync::mpsc;
use std::thread::JoinHandle;

use dovetail_api::{
    Command, CoreLink, Event, PROTOCOL_VERSION, ServerMsg, Snapshot, Warning, WarningId,
};

use crate::CoreError;
use crate::db::{Db, now_ms};
use crate::paths::Paths;

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
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (message_tx, message_rx) = mpsc::channel::<ServerMsg>();
    let thread = std::thread::Builder::new()
        .name("dovetail-home".into())
        .spawn(move || run(&db, &paths, blocked, &command_rx, &message_tx))
        .map_err(CoreError::Thread)?;
    Ok((
        CoreLink {
            commands: command_tx,
            messages: message_rx,
        },
        HomeHandle { thread },
    ))
}

fn run(
    db: &Db,
    paths: &Paths,
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
            projects: projects.iter().map(|p| p.summary(paths)).collect(),
            warnings,
            ..Snapshot::default()
        }
    };
    out.send(ServerMsg::Hello {
        protocol: PROTOCOL_VERSION,
    })
    .ok()?;
    out.send(ServerMsg::Snapshot(snapshot(seq))).ok()?;
    for command in commands {
        let event = match command {
            Command::Resync => {
                out.send(ServerMsg::Snapshot(snapshot(seq))).ok()?;
                continue;
            }
            Command::Quit { .. } => return None,
            Command::OpenProject { id } => match (&blocked, db.get_project(id.0.clone()).wait()) {
                (Some(why), _) => Event::CommandFailed {
                    message: why.clone(),
                },
                (None, Ok(Some(_))) => {
                    seq += 1;
                    let _ = out.send(ServerMsg::Event {
                        seq,
                        event: Event::OpeningProject { id: id.clone() },
                    });
                    return Some(id.0);
                }
                (None, Ok(None)) => Event::CommandFailed {
                    message: format!("there is no project {}", id.0),
                },
                (None, Err(e)) => Event::CommandFailed {
                    message: e.to_string(),
                },
            },
            _ => Event::CommandFailed {
                message: "open a project first".into(),
            },
        };
        seq += 1;
        out.send(ServerMsg::Event { seq, event }).ok()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dovetail_api::ProjectId;

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
        link.commands.send(Command::RefreshWorktrees).unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event {
                seq: 1,
                event: Event::CommandFailed { .. }
            }
        ));
        link.commands
            .send(Command::OpenProject {
                id: ProjectId("nope".into()),
            })
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event {
                seq: 2,
                event: Event::CommandFailed { .. }
            }
        ));
        link.commands.send(Command::Resync).unwrap();
        assert!(matches!(next(&link), ServerMsg::Snapshot(s) if s.seq == 2));

        link.commands
            .send(Command::OpenProject {
                id: ProjectId("notes".into()),
            })
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event {
                seq: 3,
                event: Event::OpeningProject { .. }
            }
        ));
        assert_eq!(home.wait().as_deref(), Some("notes"));
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
            .send(Command::OpenProject {
                id: ProjectId("notes".into()),
            })
            .unwrap();
        assert!(matches!(
            next(&link),
            ServerMsg::Event { event: Event::CommandFailed { message }, .. } if message == why
        ));
        drop(link);
        assert_eq!(home.wait(), None);
    }
}
