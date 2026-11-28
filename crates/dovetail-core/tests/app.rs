//! Integration tests for the `app` domain: external editor checks and `:upgrade`.

mod common;

use common::*;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::app::AppCommand;
use dovetail_api::{
    Command, Event, ServerMsg,
    agent::AgentId,
    inbox::{InboxKind, PermissionAnswer},
};
use dovetail_core::config::Config;
use dovetail_driver::{FakeDriver, ReplayOptions};
use dovetail_tools::{SocketRequestBody, SocketResponseBody};
use tempfile::TempDir;

#[test]
fn the_core_checks_the_editors_path_before_asking_the_ui_to_let_go() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let missing = h.repo.join("no-such-folder");
    h.send(Command::App(AppCommand::OpenEditor {
        path: missing.clone(),
    }));
    h.drain(Duration::from_millis(200));
    assert_eq!(h.lent, 0, "the UI was told to let go of the terminal");
    assert_eq!(h.returned, Vec::<Option<String>>::new());
    assert_eq!(
        h.failures,
        vec![format!("{} does not exist", missing.display())]
    );

    // A path that exists is accepted; until the UI answers, a second request is refused and a
    // stray release does nothing.
    h.send(Command::App(AppCommand::OpenEditor {
        path: h.repo.clone(),
    }));
    h.drain(Duration::from_millis(200));
    assert_eq!(h.lent, 1);
    h.send(Command::App(AppCommand::OpenEditor {
        path: h.repo.clone(),
    }));
    h.drain(Duration::from_millis(200));
    assert_eq!(h.lent, 1);
    assert_eq!(h.failures.len(), 2);
    assert_eq!(h.failures[1], "the terminal is already lent out");
}

mod upgrade {
    use std::sync::atomic::AtomicBool;

    use dovetail_api::{
        app::{AppCommand, AppEvent},
        inbox::{InboxCommand, UPGRADE_READY_PERMISSION},
    };
    use dovetail_core::service::UpgradeOptions;
    use dovetail_core::upgrade::{Call, Exec, ExecError};

    use super::*;

    /// Plays git and cargo: a worktree add makes the folder, cargo writes a binary into
    /// `CARGO_TARGET_DIR`, `rev-parse` answers `abc1234`.
    struct FakeExec {
        fail_cargo: bool,
    }

    impl Exec for FakeExec {
        fn run(&self, call: &Call, _cancel: &AtomicBool) -> Result<String, ExecError> {
            if call.program == "cargo" {
                if self.fail_cargo {
                    if let Some(log) = &call.log {
                        std::fs::write(log, "error[E0432]: unresolved import\n").unwrap();
                    }
                    return Err(ExecError::Failed {
                        program: "cargo".into(),
                        code: Some(101),
                    });
                }
                let target = call.env.iter().find(|(k, _)| k == "CARGO_TARGET_DIR");
                let target = PathBuf::from(&target.expect("target dir").1);
                std::fs::create_dir_all(target.join("release")).unwrap();
                std::fs::write(target.join("release/dovetail"), "new binary").unwrap();
            }
            if call.args.starts_with(&["worktree".into(), "add".into()]) {
                std::fs::create_dir_all(&call.args[3]).unwrap();
            }
            Ok(if call.args.first().is_some_and(|a| a == "rev-parse") {
                "abc1234\n".into()
            } else {
                String::new()
            })
        }
    }

    struct Upgrading {
        h: Harness,
        /// Holds the fake installed binary.
        _install: TempDir,
    }

    impl Upgrading {
        fn start(driver: FakeDriver, fail_cargo: bool) -> Upgrading {
            let install_dir = tempfile::tempdir().expect("tempdir");
            let install = install_dir.path().join("dovetail");
            std::fs::write(&install, "old binary").unwrap();
            UPGRADE.with(|u| {
                *u.borrow_mut() = Some(UpgradeOptions {
                    exec: Arc::new(FakeExec { fail_cargo }),
                    install,
                });
            });
            let h = Harness::start(driver, true, Config::default());
            // A Dovetail checkout, for the build's first check.
            std::fs::create_dir_all(h.repo.join("crates/dovetail")).unwrap();
            std::fs::write(h.repo.join("crates/dovetail/Cargo.toml"), "").unwrap();
            Upgrading {
                h,
                _install: install_dir,
            }
        }

        fn ready_item(&self) -> Option<dovetail_api::inbox::InboxItem> {
            self.h.snap.inbox.iter().find(|i| is_ready(i)).cloned()
        }

        fn wait_for_item(&mut self) -> dovetail_api::inbox::InboxItem {
            self.h
                .wait_for("Upgrade ready", |s| s.inbox.iter().any(is_ready));
            self.ready_item().expect("the item")
        }

        fn answer(&self, item: &dovetail_api::inbox::InboxItem, answer: PermissionAnswer) {
            self.h.send(Command::Inbox(InboxCommand::AnswerPermission {
                item: item.id,
                answer,
            }));
        }

        /// Reads until the UI would be told to quit.
        fn wait_for_restart_event(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let left = deadline
                    .checked_duration_since(Instant::now())
                    .expect("timed out waiting for UpgradeRestarting");
                match self.h.link.messages.recv_timeout(left) {
                    Ok(ServerMsg::Event {
                        event: Event::App(AppEvent::UpgradeRestarting),
                        ..
                    }) => return,
                    Ok(ServerMsg::Event { seq, event }) => self.h.fold(seq, event),
                    Ok(_) => {}
                    Err(_) => panic!("timed out waiting for UpgradeRestarting"),
                }
            }
        }
    }

    fn is_ready(item: &dovetail_api::inbox::InboxItem) -> bool {
        matches!(&item.kind, InboxKind::Permission { tool, .. } if tool == UPGRADE_READY_PERMISSION)
    }

    fn input_of(item: &dovetail_api::inbox::InboxItem) -> serde_json::Value {
        match &item.kind {
            InboxKind::Permission { input, .. } => input.clone(),
            other => panic!("not a permission item: {other:?}"),
        }
    }

    #[test]
    fn upgrade_builds_in_the_background_and_offers_the_build() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        u.h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        let item = u.wait_for_item();
        let input = input_of(&item);
        assert_eq!(input["source"], "abc1234");
        assert_eq!(input["restarting"], false);
        // The staged build is on disk, under the data directory only.
        assert!(u.h.paths.data.join("upgrade/staged/dovetail").is_file());
        // A second `:upgrade` while the item waits is refused.
        u.h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        u.h.drain(Duration::from_millis(300));
        assert!(
            u.h.failures
                .iter()
                .any(|f| f.contains("an upgrade is ready")),
            "{:?}",
            u.h.failures
        );
    }

    #[test]
    fn d_keeps_the_build_staged_and_does_not_restart() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        u.h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        let item = u.wait_for_item();
        u.answer(&item, PermissionAnswer::Deny { message: None });
        u.h.wait_for("the item to go", |s| !s.inbox.iter().any(is_ready));
        u.h.drain(Duration::from_millis(300));
        assert!(u.h.paths.data.join("upgrade/staged/ready.json").is_file());
        let core = u.h.core.take().expect("handle");
        assert!(!core.restart_requested());
        u.h.send(Command::App(AppCommand::Quit { end_sessions: true }));
        assert!(core.wait(Duration::from_secs(10)));
    }

    #[test]
    fn y_with_every_agent_idle_quits_to_restart() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        u.h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        let item = u.wait_for_item();
        u.answer(&item, PermissionAnswer::Allow);
        u.wait_for_restart_event();
        let core = u.h.core.take().expect("handle");
        assert!(core.restart_requested());
        assert!(
            core.wait(Duration::from_secs(10)),
            "the core stopped on its own"
        );
    }

    #[test]
    fn y_waits_for_a_busy_agent_and_says_which() {
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
            r#"{"type":"user","message":{"role":"user","content":"go"},"parent_tool_use_id":null,"isReplay":true}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"working"}]},"parent_tool_use_id":null}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.01}"#,
        ];
        let fake = FakeDriver::from_ndjson(&lines.join("\n"))
            .expect("fixture")
            .with_options(ReplayOptions {
                pause_before: vec![3],
                ..ReplayOptions::default()
            });
        let mut u = Upgrading::start(fake.clone(), false);
        prompt(&u.h, "go");
        u.h.wait_for("the reply", |s| {
            s.conversations
                .get(&AgentId(COORDINATOR.into()))
                .is_some_and(|t| t.entries.iter().any(|e| e.text == "working"))
        });
        u.h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        let item = u.wait_for_item();
        u.answer(&item, PermissionAnswer::Allow);
        u.h.wait_for("the item to list who it waits for", |s| {
            s.inbox.iter().any(|i| {
                is_ready(i) && {
                    let input = input_of(i);
                    input["restarting"] == true
                        && input["waiting"] == serde_json::json!(["coordinator"])
                }
            })
        });
        // Nothing was cut off: the turn is still held.
        assert!(!u.h.core.as_ref().unwrap().restart_requested());

        fake.resume();
        u.wait_for_restart_event();
        let core = u.h.core.take().expect("handle");
        assert!(core.restart_requested());
        assert!(core.wait(Duration::from_secs(10)));
    }

    #[test]
    fn a_failed_build_is_an_error_item_with_the_tail() {
        let mut u = Upgrading::start(fixture("plain_turn"), true);
        u.h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        u.h.wait_for("the error item", |s| {
            s.inbox.iter().any(|i| {
                matches!(&i.kind, InboxKind::Error { text }
                    if text.contains("cargo build failed")
                        && text.contains("unresolved import")
                        && text.contains("Nothing was replaced"))
            })
        });
        assert!(u.ready_item().is_none());
        assert!(!u.h.paths.data.join("upgrade/staged/dovetail").exists());
        u.h.quit();
    }

    #[test]
    fn upgrade_is_refused_when_it_is_off() {
        let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
        h.send(Command::App(AppCommand::StartUpgrade { local: false }));
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.failures.is_empty() {
            assert!(Instant::now() < deadline, "no refusal");
            h.drain(Duration::from_millis(50));
        }
        assert!(h.failures[0].contains("not available"), "{:?}", h.failures);
        h.quit();
    }

    #[test]
    fn a_build_handed_over_the_socket_raises_the_item() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        // Nothing staged yet.
        let refused = socket_call(&u.h.socket(), SocketRequestBody::StagedUpgrade);
        assert!(
            matches!(refused, SocketResponseBody::Error { .. }),
            "{refused:?}"
        );

        // `dovetail upgrade` builds into the staging folder, then hands it over.
        let staged = u.h.paths.data.join("upgrade/staged");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("dovetail"), "new binary").unwrap();
        std::fs::write(
            staged.join("ready.json"),
            r#"{"source":"def5678","differing":["roles/slice.md"],"built_at_ms":1}"#,
        )
        .unwrap();
        let reply = socket_call(&u.h.socket(), SocketRequestBody::StagedUpgrade);
        assert_eq!(reply, SocketResponseBody::UpgradeStaged);
        let input = input_of(&u.wait_for_item());
        assert_eq!(input["source"], "def5678");
        assert_eq!(input["differing"], serde_json::json!(["roles/slice.md"]));
        u.h.quit();
    }
}
