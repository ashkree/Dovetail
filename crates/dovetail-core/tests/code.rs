//! Integration tests for the `code` domain: worktree archiving, unpushed-work guards, and
//! orphan worktree sweeps.

mod common;

use common::*;

use std::path::Path;
use std::time::{Duration, Instant};

use dovetail_api::{
    Command, Snapshot,
    agent::{AgentId, AgentKind, PermissionMode},
    inbox::{InboxKind, PermissionAnswer},
};
use dovetail_api::{agent::AgentCommand, inbox::InboxCommand};
use dovetail_core::config::Config;
use dovetail_core::paths::Paths;
use dovetail_tools::project::archive_result::{ARCHIVED, WAITING_FOR_USER};

#[test]
fn the_sweep_archives_lost_slices_and_asks_about_unowned_worktrees() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    // Two slices whose worktree directories have disappeared: "lost" is gone for git too,
    // "listed" is still in git's worktree list, so it is not proof enough and is kept.
    std::fs::create_dir_all(paths.worktrees("p")).unwrap();
    let listed = paths.worktrees("p").join("listed");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "dovetail/listed",
            listed.to_str().unwrap(),
        ],
    );
    std::fs::remove_dir_all(&listed).unwrap();
    for name in ["lost", "listed"] {
        let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
        let mut agent = dovetail_api::agent::Agent {
            id: AgentId(format!("slice-{name}")),
            name: name.into(),
            kind: AgentKind::Slice,
            parent: None,
            task: None,
            worktree: Some(dovetail_api::agent::WorktreeRef {
                path: paths.worktrees("p").join(name),
                branch: format!("dovetail/{name}"),
            }),
            status: dovetail_api::agent::AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: dovetail_api::agent::SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::model::DriverKind::Claude,
        };
        agent.status_line.clear();
        db.upsert_agent(dovetail_core::db::AgentRow {
            agent,
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        })
        .wait()
        .expect("agent");
    }
    // Two worktrees no agent owns, one with an unpushed commit.
    std::fs::create_dir_all(paths.worktrees("p")).unwrap();
    for name in ["orphan", "spare"] {
        let wt = paths.worktrees("p").join(name);
        let branch = format!("dovetail/{name}");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", &branch, wt.to_str().unwrap()],
        );
    }
    std::fs::write(paths.worktrees("p").join("orphan/new.txt"), "x").unwrap();
    git(&paths.worktrees("p").join("orphan"), &["add", "."]);
    git(
        &paths.worktrees("p").join("orphan"),
        &["commit", "-qm", "work"],
    );

    // Two plain folders: "junk" holds two files; "swapped" is replaced after it is inspected.
    for name in ["junk", "swapped"] {
        let dir = paths.worktrees("p").join(name);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "b").unwrap();
    }

    let mut h = Harness::start_in(fixture("plain_turn"), paths.clone(), Some(repo.clone()));
    h.wait_for("the sweep", |s| {
        s.activity.iter().any(|a| a.text.contains("lost archived: its worktree is gone"))
            && s.inbox
                .iter()
                .filter(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == dovetail_api::inbox::REMOVE_WORKTREE_PERMISSION))
                .count()
                == 4
    });
    assert!(!h.snap.agents.iter().any(|a| a.name == "lost"));
    assert!(
        h.snap.agents.iter().any(|a| a.name == "listed"),
        "kept: git still lists it"
    );
    let item = |s: &Snapshot, name: &str| {
        s.inbox
            .iter()
            .find(|i| i.agent.0 == format!("worktree:{name}"))
            .map(|i| (i.id, i.kind.clone()))
    };
    let (orphan, kind) = item(&h.snap, "orphan").expect("orphan item");
    match kind {
        InboxKind::Permission { input, .. } => assert_eq!(input["unpushed_commits"], 1),
        other => panic!("{other:?}"),
    }
    let (spare, _) = item(&h.snap, "spare").expect("spare item");
    let (junk, kind) = item(&h.snap, "junk").expect("junk item");
    match kind {
        InboxKind::Permission { input, .. } => assert_eq!(input["files"], 2),
        other => panic!("{other:?}"),
    }
    // Replaced since it was inspected: `y` leaves it alone.
    let (swapped, _) = item(&h.snap, "swapped").expect("swapped item");
    let swapped_dir = paths.worktrees("p").join("swapped");
    std::fs::remove_dir_all(&swapped_dir).unwrap();
    std::fs::create_dir_all(&swapped_dir).unwrap();
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: swapped,
        answer: PermissionAnswer::Allow,
    }));
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: junk,
        answer: PermissionAnswer::Allow,
    }));
    h.wait_for("the swapped folder left alone", |s| {
        s.activity
            .iter()
            .any(|a| a.text.contains("swapped: changed since Dovetail asked"))
            && s.activity.iter().any(|a| a.text == "worktree junk removed")
    });
    assert!(swapped_dir.exists());
    assert!(!paths.worktrees("p").join("junk").exists(), "removed on y");
    // y removes; d keeps and is remembered.
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: orphan,
        answer: PermissionAnswer::Allow,
    }));
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: spare,
        answer: PermissionAnswer::Deny { message: None },
    }));
    h.wait_for("removal and keep", |s| {
        s.activity
            .iter()
            .any(|a| a.text == "worktree orphan removed")
            && s.activity
                .iter()
                .any(|a| a.text.contains("worktree spare kept"))
    });
    assert!(!paths.worktrees("p").join("orphan").exists());
    assert!(
        paths.worktrees("p").join("spare").exists(),
        "never deleted without y"
    );
    let _state = h.quit();

    let mut h = Harness::start_in(fixture("plain_turn"), paths.clone(), Some(repo));
    h.drain(Duration::from_millis(500));
    assert!(
        item(&h.snap, "spare").is_none(),
        "a kept worktree is not asked about again"
    );
    h.quit();
}

#[test]
fn archive_slice_removes_a_clean_worktree_and_branch_and_frees_the_queue() {
    let (_dir, repo, paths) = repo_with_slices(&["done"]);
    let config = Config {
        max_slices: 1,
        ..Config::default()
    };
    let mut h = Harness::start_with(
        fixture("plain_turn"),
        paths.clone(),
        Some(repo.clone()),
        config,
    );
    h.wait_for("the restored slice", |s| {
        s.agents.iter().any(|a| a.name == "done")
    });
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "next".into(),
        brief: "go".into(),
    }));
    h.wait_for("the queued start", |s| {
        s.activity.iter().any(|a| a.text.contains("next queued"))
    });
    let socket = h.socket();

    // A slice may not archive, not even itself.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool("slice-done", archive_slice("done")),
    ));
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("coordinator")),
    ));
    assert!(is_error && text.contains("coordinator cannot"), "{text}");
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("nope")),
    ));
    assert!(is_error && text.contains("no slice named nope"), "{text}");

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("done")),
    ));
    assert!(!is_error && text.starts_with(ARCHIVED), "{text}");
    assert!(
        !paths.worktrees("p").join("done").exists(),
        "worktree removed"
    );
    assert!(!branch_exists(&repo, "dovetail/done"), "branch removed");
    h.wait_for("the queued slice started in the freed place", |s| {
        s.agents.iter().any(|a| a.name == "next") && !s.agents.iter().any(|a| a.name == "done")
    });

    // Archived already: it is unknown now.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("done")),
    ));
    assert!(is_error && text.contains("already archived"), "{text}");
    h.quit();
}

#[test]
fn archive_slice_with_unpushed_work_asks_the_user_and_keeps_it() {
    let (_dir, repo, paths) = repo_with_slices(&["wip"]);
    let wt = paths.worktrees("p").join("wip");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-qm", "unpushed"]);
    let mut h = Harness::start_in(fixture("plain_turn"), paths.clone(), Some(repo.clone()));
    h.wait_for("the restored slice", |s| {
        s.agents.iter().any(|a| a.name == "wip")
    });
    let socket = h.socket();

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("wip")),
    ));
    assert!(!is_error && text.starts_with(WAITING_FOR_USER), "{text}");
    let asked = |s: &Snapshot| {
        s.inbox
            .iter()
            .find(|i| {
                matches!(&i.kind, InboxKind::Permission { tool, .. }
                if tool == dovetail_api::inbox::REMOVE_WORKTREE_PERMISSION)
            })
            .map(|i| i.id)
    };
    h.wait_for("the Permission item", |s| asked(s).is_some());
    assert!(
        wt.exists(),
        "unsaved work is not removed without the user's yes"
    );
    assert!(branch_exists(&repo, "dovetail/wip"));
    assert!(h.snap.agents.iter().any(|a| a.name == "wip"));

    // Asking again does not raise a second item.
    let (text, _) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("wip")),
    ));
    assert!(text.starts_with(WAITING_FOR_USER), "{text}");
    h.drain(Duration::from_millis(300));
    let items = h
        .snap
        .inbox
        .iter()
        .filter(|i| matches!(&i.kind, InboxKind::Permission { .. }))
        .count();
    assert_eq!(items, 1);

    // The user says no: the slice stays and the coordinator is told.
    let item = asked(&h.snap).expect("item");
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item,
        answer: PermissionAnswer::Deny { message: None },
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sent_users(&h.fake)
        .iter()
        .any(|t| t.contains("slice wip kept"))
    {
        assert!(Instant::now() < deadline, "{:?}", sent_users(&h.fake));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(wt.exists());
    h.quit();
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", branch])
        .stdout(std::process::Stdio::null())
        .status()
        .expect("git")
        .success()
}
