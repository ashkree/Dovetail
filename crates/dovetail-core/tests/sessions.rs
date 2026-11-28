//! Integration tests for the `sessions` infrastructure: process lifecycle, restore, takeover,
//! cgroup memory caps, OOM recovery, and profile configuration.

mod common;

use common::*;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::{
    Command, Snapshot,
    agent::{AgentId, AgentKind, AgentStatus},
    conversation::EntryKind,
    inbox::{InboxKind, PermissionAnswer},
};
use dovetail_api::{
    agent::AgentCommand, app::AppCommand, conversation::ConversationCommand, inbox::InboxCommand,
    worktree::WorktreeCommand,
};
use dovetail_core::config::Config;
use dovetail_core::memory::Scopes;
use dovetail_core::paths::Paths;
use dovetail_driver::{FakeDriver, ReplayOptions, Sent};
use dovetail_tools::SocketRequestBody;
use dovetail_tools::report::ReportTool;
use dovetail_tools::{Tool, report::AskUser};
use tempfile::TempDir;

#[test]
fn a_second_input_sent_mid_turn_keeps_the_session_working_until_its_own_result() {
    // Two inputs, two results; the replay holds before each result so the state in between
    // can be checked.
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"first"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"one","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"second"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"two"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"two","total_cost_usd":0.02}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![3, 4],
            ..ReplayOptions::default()
        });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "first");
    prompt(&h, "second");
    h.wait_for("both inputs and the first reply", |s| {
        let entries = s.conversations.get(&AgentId(COORDINATOR.into()));
        entries.is_some_and(|t| {
            t.entries
                .iter()
                .filter(|e| e.kind == EntryKind::User)
                .count()
                == 2
                && t.entries.iter().any(|e| e.text == "one")
        })
    });
    fake.resume();
    h.wait_for("the first result", |s| {
        s.agents.iter().any(|a| (a.cost_usd - 0.01).abs() < 1e-9)
    });
    // The second input is still queued: not idle, and no finished turn yet.
    assert_eq!(finished_turns(&h.snap), 0);
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .unwrap();
    assert_eq!(
        coordinator.status,
        dovetail_api::agent::AgentStatus::Working
    );
    fake.resume();
    h.wait_for("the second result", |s| finished_turns(s) == 1);
    h.quit();
}

#[test]
fn a_plain_turn_streams_into_the_conversation_and_survives_a_restart() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let entries = h.entries(COORDINATOR);
    assert_eq!(
        entries[0],
        (EntryKind::User, "Reply with exactly the word: alpha".into())
    );
    assert!(entries.contains(&(EntryKind::Assistant, "alpha".into())));
    assert_eq!(
        fake.sent(),
        vec![Sent::User("Reply with exactly the word: alpha".into())]
    );
    // Task #16: the turn's rate-limit event and the session's model reach the UI.
    h.wait_for("the usage limits", |s| s.usage.five_hour.is_some());
    assert_eq!(s_util(&h.snap.usage.five_hour), Some(0.11));
    assert_eq!(s_util(&h.snap.usage.seven_day), Some(0.17));
    let model = h
        .snap
        .agents
        .iter()
        .find(|a| a.id.0 == COORDINATOR)
        .and_then(|a| a.model.clone());
    assert_eq!(model.as_deref(), Some("claude-sonnet-5-5"));
    // M13c: 19 564 tokens in context (input, cache read and creation, output) of the 1M window
    // the result reports.
    let fill = coordinator_of(&h.snap).context_pct;
    assert_eq!(fill, Some(2));

    // Launch flags: auto mode, our MCP config without --strict-mcp-config, our settings.
    let spec = &fake.started_specs()[0];
    let args = spec.claude_args();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "auto")
    );
    assert!(args.contains(&"--mcp-config".to_string()));
    assert!(
        !args
            .iter()
            .any(|a| a.contains("strict-mcp-config") || a == "--bare")
    );
    assert!(
        spec.claude
            .allowed_tools
            .contains(&"mcp__dovetail__project_start_slice".to_string())
    );
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CODE_TASK_LIST_ID" && v == "dovetail-p-coordinator")
    );
    let mcp =
        std::fs::read_to_string(spec.claude.mcp_config_path.as_ref().expect("mcp")).expect("read");
    assert!(mcp.contains("DOVETAIL_AGENT_ID") && mcp.contains("\"coordinator\""));
    // No profile in the recipe: the session inherits Dovetail's own Claude home (ADR 0009).
    assert!(!spec.env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR"));

    // The hook and the bridge are pinned to this app's state, whatever a slice's env says.
    let settings = std::fs::read_to_string(spec.claude.settings_path.as_ref().expect("settings"))
        .expect("read");
    let data = h.paths.data.to_string_lossy().into_owned();
    assert!(
        settings.contains(&format!("DOVETAIL_DATA_DIR='{data}'")),
        "{settings}"
    );
    assert!(mcp.contains("DOVETAIL_RUNTIME_DIR"), "{mcp}");

    let paths = h.paths.clone();
    let _state = h.quit();
    // Reopen on the same state: the conversation and the session id are still there.
    let fake = fixture("plain_turn");
    let h = Harness::start_on(fake.clone(), paths);
    assert!(
        h.entries(COORDINATOR)
            .contains(&(EntryKind::Assistant, "alpha".into()))
    );
    h.quit();
}

#[test]
fn a_background_subagent_means_one_finished_turn_and_none_of_its_events() {
    let mut h = Harness::start(fixture("double_result"), true, Config::default());
    prompt(&h, "start a background subagent");
    h.wait_for("the last result", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::FinishedTurn { preview } if preview.contains("finished"))
        })
    });
    // Three results, but the first came while the subagent ran, and the later ones replace.
    assert_eq!(finished_turns(&h.snap), 1);
    let entries = h.entries(COORDINATOR);
    // The subagent's Bash and handback are routed away from the parent's conversation.
    assert!(entries.contains(&(EntryKind::ToolSummary, "Agent".into())));
    assert!(!entries.iter().any(|(_, t)| t.contains("Bash")));
    h.quit();
}

#[test]
fn a_background_subagent_is_a_node_with_progress_cost_and_its_summary_at_the_end() {
    // Out lines 8 and 22 are the two `result`s; 17 is `task_updated` (completed).
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![8, 17, 22],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let sub = subagent(&h.snap).expect("node").clone();
    assert_eq!(sub.kind, AgentKind::Subagent { background: true });
    assert_eq!(sub.parent, Some(AgentId(COORDINATOR.into())));
    assert_eq!(
        sub.id,
        AgentId("coordinator:toolu_01WrSGRZBAmUyjsWgzW1pdsV".into())
    );
    assert_eq!(sub.name, "Background sleep task");
    assert_eq!(sub.status, AgentStatus::Working);

    // Past the first `result` and the subagent's progress: the parent is not done.
    fake.resume();
    h.wait_for("the last progress", |s| {
        subagent(s)
            .and_then(|a| a.progress.as_ref())
            .is_some_and(|p| p.tool_uses == 2)
    });
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .expect("coordinator");
    assert!((coordinator.cost_usd - 0.0494268).abs() < 1e-6);
    assert_eq!(coordinator.status, AgentStatus::Working);
    assert_eq!(finished_turns(&h.snap), 0);
    let sub = subagent(&h.snap).expect("node");
    // M13c: a subagent's calls fill its own context, not its parent's; it shows no fill.
    assert_eq!(sub.context_pct, None);
    let progress = sub.progress.as_ref().expect("progress");
    assert_eq!(progress.total_tokens, 17164);
    assert_eq!(progress.duration_ms, 11302);
    assert_eq!(progress.last_tool.as_deref(), Some("SubagentHandback"));
    assert_eq!(sub.status, AgentStatus::Working);

    // Completed, with the hand-back as its summary, before the parent's second `result`.
    fake.resume();
    h.wait_for("the summary", |s| {
        subagent(s)
            .and_then(|a| a.progress.as_ref())
            .is_some_and(|p| p.summary.is_some())
    });
    let sub = subagent(&h.snap).expect("node");
    assert_eq!(sub.status, AgentStatus::Done);
    assert_eq!(sub.status_line, "bg-done");
    let progress = sub.progress.as_ref().expect("progress");
    assert_eq!(progress.summary.as_deref(), Some("bg-done"));
    assert_eq!(progress.total_tokens, 17175);
    assert!(
        progress
            .output_file
            .as_ref()
            .is_some_and(|f| f.ends_with("ad6b787476c37b6da.output"))
    );
    // The session's total is summed from the task events, not taken from its `result`.
    h.wait_for("the session total", |s| {
        coordinator_of(s).subagent_tokens == 17175
    });
    // Its own Bash task is not a node.
    assert_eq!(
        h.snap
            .agents
            .iter()
            .filter(|a| matches!(a.kind, AgentKind::Subagent { .. }))
            .count(),
        1
    );
    assert_eq!(finished_turns(&h.snap), 0);

    fake.resume();
    h.wait_for("the finished turn", |s| finished_turns(s) == 1);
    // The parent's cost includes the subagent's; the subagent adds nothing to the totals.
    h.wait_for("the totals", |s| (s.totals.cost_usd - 0.1).abs() < 1e-9);
    assert_eq!(subagent(&h.snap).expect("node").cost_usd, 0.0);
    // Its Bash call never reached the parent's conversation.
    assert!(
        !h.entries(COORDINATOR)
            .iter()
            .any(|(_, t)| t.contains("Bash"))
    );

    // The node survives a restart, summary and all.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    let sub = subagent(&h.snap).expect("node after restart");
    assert_eq!(coordinator_of(&h.snap).subagent_tokens, 17175);
    assert_eq!(sub.status, AgentStatus::Done);
    assert_eq!(sub.parent, Some(AgentId(COORDINATOR.into())));
    assert_eq!(
        sub.progress.as_ref().and_then(|p| p.summary.as_deref()),
        Some("bg-done")
    );
    h.quit();
}

#[test]
fn a_subagent_cut_off_by_a_restart_reopens_as_stopped() {
    // Hold before the subagent's first progress: it is still running at quit.
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![9],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    let sub = subagent(&h.snap).expect("node after restart");
    assert_eq!(sub.status, AgentStatus::Error);
    assert!(
        sub.status_line.starts_with("stopped"),
        "{}",
        sub.status_line
    );
    // Only the parent's cut-off turn is reported, not the subagent.
    assert!(
        h.snap
            .inbox
            .iter()
            .all(|i| i.agent == AgentId(COORDINATOR.into()))
    );
    h.quit();
}

#[test]
fn a_session_killed_for_memory_is_resumed_with_its_subagent_work_listed() {
    // A temp dir stands in for the cgroup slice; the coordinator's scope is in it.
    let slice = tempfile::tempdir().expect("tempdir");
    let cgroup = slice.path().join("dovetail-p-coordinator.scope");
    std::fs::create_dir(&cgroup).expect("scope dir");
    std::fs::write(cgroup.join("memory.events"), memory_events(0)).expect("events");
    // Hold before the background subagent's first progress, then die by SIGKILL there.
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![9],
        kill_first_before: Some((9, 9)),
        ..ReplayOptions::default()
    });
    let scopes = Scopes::Available {
        slice_dir: slice.path().to_path_buf(),
        escape_dollars: false,
    };
    let mut h = Harness::start_scoped(fake.clone(), true, Config::default(), Some(scopes));
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let description = subagent(&h.snap)
        .and_then(|a| a.progress.as_ref())
        .map(|p| p.description.clone())
        .expect("its description");
    // The kernel counts the kill (seen through the watch), then the scope goes with the
    // session, as `--collect` makes it.
    std::fs::write(cgroup.join("memory.events"), memory_events(1)).expect("events");
    h.drain(Duration::from_millis(300));
    std::fs::remove_dir_all(&cgroup).expect("remove scope");
    fake.resume();
    h.wait_for("the out-of-memory item", |s| !oom_items(s).is_empty());
    let text = oom_items(&h.snap).remove(0);
    assert!(text.contains("restarted from its session"), "{text}");
    assert!(text.contains(&description), "{text}");
    // Resumed from its session id, and told what was in flight.
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 2);
    assert!(specs[1].resume);
    assert_eq!(specs[1].session_id, specs[0].session_id);
    wait_restart_note(&fake, &description);
    h.wait_for("the coordinator running again", |s| {
        coordinator_of(s).status != AgentStatus::Error
    });
    // Only one kill was seen: no item about a tool.
    assert_eq!(oom_items(&h.snap).len(), 1);
    fake.resume();
    h.quit();
}

#[test]
fn an_oom_count_that_arrives_after_the_exit_still_restarts_the_session() {
    let slice = tempfile::tempdir().expect("tempdir");
    let cgroup = slice.path().join("dovetail-p-coordinator.scope");
    std::fs::create_dir(&cgroup).expect("scope dir");
    std::fs::write(cgroup.join("memory.events"), memory_events(0)).expect("events");
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![9],
        kill_first_before: Some((9, 9)),
        ..ReplayOptions::default()
    });
    let scopes = Scopes::Available {
        slice_dir: slice.path().to_path_buf(),
        escape_dollars: false,
    };
    let mut h = Harness::start_scoped(fake.clone(), true, Config::default(), Some(scopes));
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let description = subagent(&h.snap)
        .and_then(|a| a.progress.as_ref())
        .map(|p| p.description.clone())
        .expect("its description");
    // The exit is handled first, with no kill counted yet: it reads as a crash.
    fake.resume();
    let crashed = |s: &Snapshot| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Error { text } if text.starts_with("session ended unexpectedly"))
        })
    };
    h.wait_for("the crash item", crashed);
    // Then the watcher's count arrives.
    std::fs::write(cgroup.join("memory.events"), memory_events(1)).expect("events");
    h.wait_for("the out-of-memory item", |s| !oom_items(s).is_empty());
    assert!(!crashed(&h.snap), "the crash item is replaced");
    let text = oom_items(&h.snap).remove(0);
    assert!(text.contains("restarted from its session"), "{text}");
    assert!(text.contains(&description), "{text}");
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 2);
    assert!(specs[1].resume);
    wait_restart_note(&fake, &description);
    fake.resume();
    h.quit();
}

#[test]
fn interrupt_reaches_a_session_waiting_on_background_tasks() {
    // Live check 2026-11-06: `sleep 30` went to the background, the turn's result came, the
    // row said working, and interrupt was refused as "not mid-turn".
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","task_type":"local_bash","description":"sleep 30"}]}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"started it","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![3],
            ..ReplayOptions::default()
        });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "run sleep 30");
    h.wait_for("the result with a background task left", |s| {
        s.agents.iter().any(|a| (a.cost_usd - 0.01).abs() < 1e-9)
    });
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .unwrap();
    assert_eq!(
        coordinator.status,
        dovetail_api::agent::AgentStatus::Working
    );
    h.send(Command::Agent(AgentCommand::Interrupt {
        agent: AgentId(COORDINATOR.into()),
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake.sent().contains(&Sent::Interrupt) {
        assert!(
            Instant::now() < deadline,
            "no interrupt sent: {:?}",
            fake.sent()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    fake.resume();
    h.quit();
}

#[test]
fn a_turn_cut_off_by_quit_is_reported_on_reopen_and_not_continued() {
    let mut h = Harness::start(held_turn(), true, Config::default());
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    let paths = h.paths.clone();
    let _state = h.quit();
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    let text = cut_off_text(&h.snap).unwrap_or_default();
    assert!(
        text.contains("coordinator's turn") && text.contains("rename the flag"),
        "{text}"
    );
    // The coordinator's session is back, but nothing was sent to it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "the coordinator was not resumed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fake.started_specs()[0].resume, "resumed, not created");
    assert!(
        !fake.sent().iter().any(|s| matches!(s, Sent::User(_))),
        "{:?}",
        fake.sent()
    );
    h.quit();
}

#[test]
fn killing_the_app_mid_turn_loses_no_state() {
    let mut h = Harness::start(held_turn(), true, Config::default());
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.agents
            .iter()
            .any(|a| a.status == dovetail_api::agent::AgentStatus::Working)
            && s.conversations
                .get(&AgentId(COORDINATOR.into()))
                .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    // Give the DB thread a moment, then copy the database files as a kill would leave them:
    // the app never gets to quit.
    std::thread::sleep(Duration::from_millis(200));
    let crashed = tempfile::tempdir().expect("tempdir");
    let from = h.paths.state_db("p");
    let to_paths = Paths {
        data: crashed.path().join("data"),
        ..h.paths.clone()
    };
    let to = to_paths.state_db("p");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{suffix}", from.display()));
        if src.exists() {
            std::fs::copy(&src, format!("{}{suffix}", to.display())).unwrap();
        }
    }
    let mut reopened = Harness::start_on(fixture("plain_turn"), to_paths);
    reopened.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    let entries = reopened.entries(COORDINATOR);
    assert!(
        entries.contains(&(EntryKind::User, "rename the flag".into())),
        "{entries:?}"
    );
    reopened.quit();
    h.quit();
}

#[test]
fn at_restart_permissions_expire_and_questions_stay_answerable() {
    let fake = fixture("concurrent_permissions").with_options(ReplayOptions {
        wait_for_answers: true,
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake, true, Config::default());
    prompt(&h, "read two files");
    h.wait_for("two permission requests", |s| {
        s.inbox
            .iter()
            .filter(|i| matches!(i.kind, InboxKind::Permission { .. }))
            .count()
            == 2
    });
    let socket = h.socket();
    let (text, _) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::Report(ReportTool::AskUser(AskUser {
                question: "Which port?".into(),
            })),
        ),
    ));
    assert!(text.starts_with("Question sent"), "{text}");
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.wait_for("the expired requests", |s| {
        s.activity
            .iter()
            .filter(|a| a.text.contains("request expired at restart"))
            .count()
            == 2
            && s.inbox
                .iter()
                .any(|i| matches!(i.kind, InboxKind::Question { .. }))
    });
    assert!(
        h.snap
            .activity
            .iter()
            .any(|a| a.text.contains("Read one.txt"))
    );
    assert!(
        !h.snap
            .inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::Permission { .. }))
    );
    let question = h
        .snap
        .inbox
        .iter()
        .find(|i| matches!(i.kind, InboxKind::Question { .. }))
        .unwrap_or_else(|| panic!("the question stays: {:#?}", h.snap.inbox))
        .id;
    // A question does not explain the stopped turn (the agent kept working): its cut-off is
    // reported too.
    h.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    h.send(Command::Inbox(InboxCommand::AnswerQuestion {
        item: question,
        text: "8080".into(),
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    let wanted = Sent::User("Answer to your question 'Which port?': 8080".into());
    while !fake.sent().contains(&wanted) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
    h.quit();
}

#[test]
fn on_reopen_the_coordinator_resumes_at_once_and_a_slice_on_first_touch() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    prompt(&h, "hello");
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "later".into(),
        brief: "go".into(),
    }));
    // Both sessions have run once their replies are in.
    h.wait_for("both sessions to have run", |s| {
        ["coordinator", "slice-later"].iter().all(|a| {
            s.conversations
                .get(&AgentId((*a).into()))
                .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
        })
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    let names = |fake: &FakeDriver| -> Vec<(String, bool)> {
        fake.started_specs()
            .iter()
            .map(|s| (s.name.clone().unwrap_or_default(), s.resume))
            .collect()
    };
    assert_eq!(names(&fake), vec![("p.coordinator".to_string(), true)]);
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId("slice-later".into()),
        text: "carry on".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while names(&fake).len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", names(&fake));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(names(&fake)[1], ("p.later".to_string(), true));
    h.quit();
}

/// ADR 0016 migration: a conversation from before the rename, resumed, keeps calling the old
/// names. They stay allowed, and a call by one answers as the new tool with a note naming it.
#[test]
fn a_resumed_conversation_calling_an_old_tool_name_still_works() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    prompt(&h, "hello");
    h.wait_for("the first run", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    // The resumed session's model still knows the tool as `report_status`.
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"carry on"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__dovetail__report_status","input":{"status":"resumed"}}]},"parent_tool_use_id":null}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"carried on"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"carried on","total_cost_usd":0.01}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n")).expect("fixture");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    let spec = &fake.started_specs()[0];
    assert!(spec.resume);
    for name in ["report_status", "core_report_status"] {
        let tool = format!("mcp__dovetail__{name}");
        assert!(
            spec.claude.allowed_tools.contains(&tool),
            "{tool} not allowed"
        );
    }

    prompt(&h, "carry on");
    h.wait_for("the resumed turn, its old-name tool use tallied", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries.iter().any(|e| e.text == "carried on")
                    && t.entries.iter().any(|e| {
                        e.kind == EntryKind::ToolSummary && e.text.contains("report_status")
                    })
            })
    });

    // What the bridge forwards for that call: the new tool, and the name it was called by.
    let call = Tool::parse("report_status", serde_json::json!({"status": "resumed"}))
        .expect("the old name parses");
    assert_eq!(call.name(), "core_report_status");
    let (text, is_error) = result_text(socket_call(
        &h.socket(),
        SocketRequestBody::ToolCall {
            agent: AgentId(COORDINATOR.into()),
            tool_use_id: Some("t1".into()),
            call,
            called_as: Some("report_status".into()),
        },
    ));
    assert!(!is_error, "{text}");
    assert!(
        text.ends_with("Note: `report_status` is deprecated and will be removed; call `core_report_status` instead."),
        "{text}"
    );
    h.wait_for("the status line", |s| {
        s.agents.iter().any(|a| a.status_line == "resumed")
    });
    h.quit();
}

/// Live rehearsal 2026-11-06: two "slice ready" notes sent mid-turn were folded into the
/// running turn (both echoed before its one `result`); the coordinator stayed "working".
#[test]
fn inputs_folded_into_the_running_turn_end_with_its_result() {
    // Out line 13 is the echo of the first note; hold before it so the notes can be sent.
    let fake = fixture("merged_inputs").with_options(ReplayOptions {
        pause_before: vec![13],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "create tasks and start two slices");
    h.wait_for("the turn to reach the notes", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::ToolSummary))
    });
    prompt(&h, "slice a1 ready");
    prompt(&h, "slice b1 ready");
    h.wait_for("both notes sent", |_| {
        fake.sent()
            .iter()
            .filter(|s| matches!(s, Sent::User(_)))
            .count()
            == 3
    });
    fake.resume();
    h.wait_for("the one result to end the turn", |s| {
        finished_turns(s) == 1
            && s.agents.iter().any(|a| {
                a.kind == AgentKind::Coordinator
                    && a.status != dovetail_api::agent::AgentStatus::Working
            })
    });
    h.quit();
}

#[test]
fn notes_for_a_coordinator_cut_off_by_a_restart_wait_for_the_users_next_message() {
    let mut h = Harness::start(held_turn(), true, Config::default());
    pending_start(&mut h);
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    h.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    h.drain(Duration::from_millis(200));
    assert!(
        sent_users(&fake).is_empty(),
        "nothing continues it: {:?}",
        fake.sent()
    );
    prompt(&h, "carry on");
    let deadline = Instant::now() + Duration::from_secs(5);
    while sent_users(&fake).len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        sent_users(&fake),
        vec![EXPIRED.to_string(), "carry on".to_string()]
    );
    let _state = h.quit();
    // Delivered, so not sent again on the next open.
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    assert!(!sent_users(&fake).contains(&EXPIRED.to_string()));
    h.quit();
}

#[test]
fn a_turn_that_ends_normally_during_quit_is_not_reported_as_cut_off() {
    let fake = held_turn();
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    // Quit while the turn runs; its result arrives inside the quit window.
    h.send(Command::App(AppCommand::Quit { end_sessions: true }));
    std::thread::sleep(Duration::from_millis(100));
    fake.resume();
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.drain(Duration::from_millis(300));
    assert!(cut_off_text(&h.snap).is_none(), "{:#?}", h.snap.inbox);
    h.quit();
}

#[test]
fn a_project_profile_sets_claude_config_dir_and_a_changed_one_is_not_resumed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    let work = dir.path().join("profiles/work");
    write_recipe(&paths, "profile = \"mine\"\n");

    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
    assert_eq!(h.snap.profile.as_deref(), Some("mine"));
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    h.wait_for("the usage limits", |s| s.usage.five_hour.is_some());
    let first = fake.started_specs()[0].clone();
    assert_eq!(
        config_dir_env(&first),
        Some(mine.to_string_lossy().into_owned())
    );
    let _state = h.quit();

    // The recipe now picks another account: the coordinator is not resumed under it.
    write_recipe(&paths, "profile = \"work\"\n");
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
    h.wait_for("the profile notice", |s| {
        error_texts(s)
            .iter()
            .any(|t| t.contains("Claude profile work"))
    });
    assert_eq!(h.snap.profile.as_deref(), Some("work"));
    // Limits are per account: `mine`'s figures are not shown for `work`.
    assert_eq!(h.snap.usage, dovetail_api::status::UsageLimits::default());
    wait_started(&fake, 1);
    let second = fake.started_specs()[0].clone();
    assert!(!second.resume, "resumed across profiles");
    assert_ne!(second.session_id, first.session_id);
    assert_eq!(
        config_dir_env(&second),
        Some(work.to_string_lossy().into_owned())
    );
    h.wait_for("the fresh-session notice", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text.contains("fresh session")))
    });
    let _state = h.quit();

    // Told once: the next open, under the same profile, resumes and says nothing new.
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths, None, config);
    wait_started(&fake, 1);
    h.drain(Duration::from_millis(200));
    assert!(fake.started_specs()[0].resume);
    let notices = error_texts(&h.snap)
        .iter()
        .filter(|t| t.contains("now runs under Claude profile"))
        .count();
    assert!(notices <= 1, "{:?}", error_texts(&h.snap));
    h.quit();
}

/// Review of PR #26: exactly one inbox error, also for a coordinator that would resume at open,
/// and not again on the next open while it is still unanswered.
#[test]
fn an_unknown_profile_is_one_inbox_error_and_starts_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    write_recipe(&paths, "profile = \"mine\"\n");
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let _state = h.quit();

    write_recipe(&paths, "profile = \"ghost\"\n");
    for _ in 0..2 {
        let fake = fixture("plain_turn");
        let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
        h.wait_for("the profile error", |s| profile_errors(s, "\"ghost\"") > 0);
        prompt(&h, "hello");
        h.drain(Duration::from_millis(300));
        assert_eq!(
            profile_errors(&h.snap, "\"ghost\""),
            1,
            "{:?}",
            error_texts(&h.snap)
        );
        assert!(fake.started_specs().is_empty());
        let _state = h.quit();
    }
}

#[test]
fn a_recipe_env_setting_claude_config_dir_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    write_recipe(&paths, "[env]\nCLAUDE_CONFIG_DIR = \"/elsewhere\"\n");
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths, None, profiles_config(dir.path()));
    h.wait_for("the env error", |s| profile_errors(s, "profile = ") > 0);
    assert!(error_texts(&h.snap)[0].contains("CLAUDE_CONFIG_DIR"));
    prompt(&h, "hello");
    h.drain(Duration::from_millis(300));
    assert!(fake.started_specs().is_empty());
    assert_eq!(profile_errors(&h.snap, "profile = "), 1);
    h.quit();
}

/// The hand-off with a live headless coordinator: the lock is taken, headless stops, native
/// `claude` runs with headless's flags, Claude home and folder, and on exit the terminal comes
/// back, the native turn shows in the conversation and headless resumes the same session.
#[test]
fn take_over_stops_headless_runs_native_with_its_flags_and_resumes_headless() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let out = dir.path().join("native");
    std::fs::create_dir_all(&out).expect("out");
    // The coordinator runs in its `home/` folder (ADR 0021), and the fake driver reports that.
    let claude = fake_claude(&out, &encoded(&paths.project_home("p")), None);
    // Held before its `result`: the session is still running when it is taken over.
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![4],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start_native(fake.clone(), paths, None, config, Some(claude));
    prompt(&h, "hello");
    h.wait_for("the reply", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::Assistant))
    });

    h.send(Command::App(AppCommand::TakeOver {
        agent: AgentId(COORDINATOR.into()),
    }));
    h.wait_for("the lock", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::agent::SessionLock::TakenOver
    });
    wait_sent_shutdown(&fake);
    // Native waits for headless to exit.
    h.drain(Duration::from_millis(100));
    assert!(!out.join("argv").exists(), "native ran beside headless");
    fake.resume();

    // The note, then the turn; its tool summary comes last.
    h.wait_for("the native turn", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::ToolSummary && e.text == "Bash")
            })
    });
    assert_eq!(
        h.returned,
        vec![None],
        "the terminal came back once, cleanly"
    );
    let entries = h.entries(COORDINATOR);
    let at = entries
        .iter()
        .position(|(_, t)| t.contains("1 native turn"))
        .expect("note");
    assert_eq!(
        entries[at..at + 4].to_vec(),
        vec![
            (
                EntryKind::Notice,
                "Back from the native terminal: 1 native turn, shown below.".into()
            ),
            (EntryKind::User, "native hello".into()),
            (EntryKind::Assistant, "native reply".into()),
            (EntryKind::ToolSummary, "Bash".into()),
        ]
    );

    // The same flags as headless, minus the stream ones, on the same session.
    let headless = fake.started_specs()[0].clone();
    let argv: Vec<String> = read(&out.join("argv")).lines().map(String::from).collect();
    assert_eq!(argv, headless.native_args());
    let resume = argv.iter().position(|a| a == "--resume").expect("--resume");
    assert_eq!(argv[resume + 1], headless.session_id);
    for flag in [
        "--settings",
        "--mcp-config",
        "--allowedTools",
        "--permission-mode",
    ] {
        assert!(argv.contains(&flag.to_string()), "{flag} missing: {argv:?}");
    }
    assert!(!argv.contains(&"-p".to_string()));
    assert_eq!(read(&out.join("config_dir")), mine.to_string_lossy());
    assert_eq!(read(&out.join("agent")), COORDINATOR);
    // Native runs where headless did: the coordinator's `home/` (ADR 0021).
    assert_eq!(
        read(&out.join("cwd")).trim(),
        h.paths
            .project_home("p")
            .canonicalize()
            .expect("home")
            .to_string_lossy()
    );

    // Headless again, resuming the same session.
    wait_started(&fake, 2);
    let again = fake.started_specs()[1].clone();
    assert!(again.resume);
    assert_eq!(again.session_id, headless.session_id);
    h.wait_for("the lock released", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::agent::SessionLock::Headless
    });
    fake.resume();
    h.quit();
}

/// While a slice is taken over nothing starts it headless, archives it or removes its
/// worktree; native runs in the worktree, and exiting it leaves the worktree and the slice
/// alone and resumes headless there.
#[test]
fn while_taken_over_headless_never_starts_and_exiting_native_keeps_the_worktree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let worktree = paths.worktrees("p").join("s4");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let worktree = worktree.canonicalize().expect("canonical");
    let folder = encoded(&worktree);
    let transcript = mine
        .join("projects")
        .join(&folder)
        .join("slice-s4-session.jsonl");
    seed(
        &paths,
        session_row(
            "slice-s4",
            AgentKind::Slice,
            Some(worktree.clone()),
            &mine,
            transcript,
            None,
        ),
    );
    let out = dir.path().join("native");
    std::fs::create_dir_all(&out).expect("out");
    let release = out.join("exit-now");
    let claude = fake_claude(&out, &folder, Some(&release));
    let fake = fixture("plain_turn");
    let slice = AgentId("slice-s4".into());
    let mut h = Harness::start_native(fake.clone(), paths, None, config, Some(claude));

    h.send(Command::App(AppCommand::TakeOver {
        agent: slice.clone(),
    }));
    wait_file(&out.join("started"));
    h.wait_for("the lock", |s| {
        lock_of(s, "slice-s4") == dovetail_api::agent::SessionLock::TakenOver
    });
    assert_eq!(read(&out.join("cwd")).trim(), worktree.to_string_lossy());

    // Every way in is refused while native runs.
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: slice.clone(),
        text: "are you there?".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    h.send(Command::Agent(AgentCommand::CloseSlice {
        agent: slice.clone(),
    }));
    h.send(Command::Worktree(WorktreeCommand::Remove {
        agent: slice.clone(),
        confirmed: true,
    }));
    h.send(Command::App(AppCommand::TakeOver {
        agent: slice.clone(),
    }));
    h.drain(Duration::from_millis(300));
    assert!(fake.started_specs().is_empty(), "headless started");
    assert_eq!(h.failures.len(), 3, "{:?}", h.failures);
    assert!(h.failures[0].contains("taken over in a native terminal: write to it there"));
    assert!(h.failures[1].contains("archive it once that claude has exited"));
    assert!(h.failures[2].contains("archive it once that claude has exited"));
    assert_eq!(
        h.returned,
        vec![Some("the terminal is already lent out".into())]
    );
    assert!(worktree.exists());

    std::fs::write(&release, "").expect("let native exit");
    wait_started(&fake, 1);
    let resumed = fake.started_specs()[0].clone();
    assert!(resumed.resume);
    assert_eq!(resumed.cwd, worktree);
    assert_eq!(resumed.session_id, "slice-s4-session");
    h.wait_for("the lock released", |s| {
        lock_of(s, "slice-s4") == dovetail_api::agent::SessionLock::Headless
    });
    h.drain(Duration::from_millis(200));
    assert!(worktree.exists(), "exiting native removed the worktree");
    assert!(
        h.snap.agents.iter().any(|a| a.id == slice),
        "slice archived"
    );
    h.quit();
}

/// A lock left by a crash whose native run is gone is released at open, with the native turn
/// shown, and the coordinator resumes as usual.
#[test]
fn a_crash_lock_whose_native_is_gone_is_released_at_open() {
    let mut child = std::process::Command::new("true").spawn().expect("true");
    let dead = child.id();
    child.wait().expect("reaped");
    let (mut h, fake, _dir) = open_after_crash(dead, Some(1));
    h.wait_for("the release", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::agent::SessionLock::Headless
    });
    wait_started(&fake, 1);
    assert!(fake.started_specs()[0].resume);
    h.wait_for("the native turn", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::User && e.text == "typed while Dovetail was down")
            })
    });
    let entries = h.entries(COORDINATOR);
    assert!(
        entries
            .iter()
            .any(|(k, t)| *k == EntryKind::Notice && t.contains("lock is released: 1 native turn")),
        "{entries:?}"
    );
    h.quit();
}

/// A lock left by a crash whose native run still runs is kept: Needs you says so and nothing
/// starts headless, a message included; once native exits, the next message releases it.
#[test]
fn a_crash_lock_whose_native_still_runs_holds_until_it_exits() {
    use dovetail_core::takeover::{Proc, Processes};
    let mut native = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("sleep");
    let pid = native.id();
    let ticks = Proc.start_ticks(pid).expect("stat");
    let (mut h, fake, _dir) = open_after_crash(pid, ticks);
    h.wait_for("the still-taken-over item", |s| {
        error_texts(s)
            .iter()
            .any(|t| t.contains("is still taken over") && t.contains(&pid.to_string()))
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(
        lock_of(&h.snap, COORDINATOR),
        dovetail_api::agent::SessionLock::TakenOver
    );
    assert!(fake.started_specs().is_empty(), "headless beside native");
    prompt(&h, "hello");
    h.drain(Duration::from_millis(200));
    assert!(fake.started_specs().is_empty(), "a prompt started headless");
    assert!(
        h.failures.iter().any(|f| f.contains("taken over")),
        "{:?}",
        h.failures
    );

    native.kill().expect("kill");
    native.wait().expect("reaped");
    prompt(&h, "hello again");
    wait_started(&fake, 1);
    h.wait_for("the release", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::agent::SessionLock::Headless
    });
    h.quit();
}

/// A crash lock that cannot be checked asks the user; `y` goes through the same release as
/// every other path: the lock goes, the coordinator resumes, and the note held for it while it
/// was locked is delivered.
#[test]
fn y_on_an_undecidable_crash_lock_releases_it_and_delivers_the_held_notes() {
    use dovetail_api::inbox::RELEASE_TAKEOVER_PERMISSION;
    use dovetail_core::takeover::TakeoverLock;
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let lock = TakeoverLock {
        since_ms: 1,
        pid: None,
        start_ticks: None,
        transcript_offset: None,
    };
    seed(
        &paths,
        session_row(
            COORDINATOR,
            AgentKind::Coordinator,
            None,
            &mine,
            dovetail_core::session::launch::transcript_path(
                &mine,
                &paths.project_home("p"),
                &format!("{COORDINATOR}-session"),
            ),
            Some(lock),
        ),
    );
    let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
    db.add_note("slice tryit ready".into())
        .wait()
        .expect("note");
    drop(db);
    let fake = fixture("plain_turn");
    let mut h = Harness::start_full(
        fake.clone(),
        paths,
        None,
        config,
        None,
        Some(Arc::new(Blind)),
    );
    h.wait_for("the release question", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == RELEASE_TAKEOVER_PERMISSION)
        })
    });
    h.drain(Duration::from_millis(200));
    assert!(fake.started_specs().is_empty(), "started while undecided");
    let item = h
        .snap
        .inbox
        .iter()
        .find(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == RELEASE_TAKEOVER_PERMISSION))
        .map(|i| i.id)
        .expect("item");
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item,
        answer: PermissionAnswer::Allow,
    }));
    wait_started(&fake, 1);
    wait_sent(&fake, "slice tryit ready");
    h.wait_for("the lock released", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::agent::SessionLock::Headless
    });
    h.quit();
}

/// `core_ask_user` from native returns at once and leaves its question in Needs you. Answering it
/// while native still has the session is refused and the question stays; once headless is
/// back the answer reaches the session.
#[test]
fn ask_user_from_native_returns_at_once_and_is_answered_once_headless_is_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let worktree = paths.worktrees("p").join("s4");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let worktree = worktree.canonicalize().expect("canonical");
    let folder = encoded(&worktree);
    seed(
        &paths,
        session_row(
            "slice-s4",
            AgentKind::Slice,
            Some(worktree.clone()),
            &mine,
            mine.join("projects")
                .join(&folder)
                .join("slice-s4-session.jsonl"),
            None,
        ),
    );
    let out = dir.path().join("native");
    std::fs::create_dir_all(&out).expect("out");
    let release = out.join("exit-now");
    let claude = fake_claude(&out, &folder, Some(&release));
    let fake = fixture("plain_turn");
    let slice = AgentId("slice-s4".into());
    let mut h = Harness::start_native(fake.clone(), paths, None, config, Some(claude));
    h.send(Command::App(AppCommand::TakeOver {
        agent: slice.clone(),
    }));
    wait_file(&out.join("started"));

    // What native's MCP bridge would send: it returns at once.
    let asked = Instant::now();
    let (text, is_error) = result_text(socket_call(
        &h.socket(),
        tool(
            "slice-s4",
            Tool::Report(ReportTool::AskUser(AskUser {
                question: "Which branch?".into(),
            })),
        ),
    ));
    assert!(!is_error, "{text}");
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "core_ask_user waited"
    );
    h.wait_for("the question", |s| !open_questions(s).is_empty());
    let question = open_questions(&h.snap)[0];

    h.send(Command::Inbox(InboxCommand::AnswerQuestion {
        item: question,
        text: "main".into(),
    }));
    h.drain(Duration::from_millis(200));
    assert!(
        h.failures.iter().any(|f| f.contains("taken over")),
        "{:?}",
        h.failures
    );
    assert_eq!(
        open_questions(&h.snap),
        vec![question],
        "the question was lost"
    );

    std::fs::write(&release, "").expect("let native exit");
    wait_started(&fake, 1);
    h.send(Command::Inbox(InboxCommand::AnswerQuestion {
        item: question,
        text: "main".into(),
    }));
    wait_sent(&fake, "Answer to your question 'Which branch?': main");
    h.wait_for("the question answered", |s| open_questions(s).is_empty());
    h.quit();
}

/// `memory.events` text with `oom_kill` at `kills`, as the kernel writes it.
fn memory_events(kills: u64) -> String {
    format!("low 0\nhigh 0\nmax {kills}\noom {kills}\noom_kill {kills}\noom_group_kill 0\n")
}

/// The text of the coordinator's out-of-memory items.
fn oom_items(s: &Snapshot) -> Vec<String> {
    s.inbox
        .iter()
        .filter_map(|i| match &i.kind {
            InboxKind::Error { text } if text.starts_with("Out of memory") => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Waits until the restarted session was told, in one line, that it was OOM-killed and
/// restarted, followed by the in-flight subagent `work`.
fn wait_restart_note(fake: &FakeDriver, work: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let is_note = |s: &Sent| {
        matches!(s, Sent::User(t) if t.lines().next().is_some_and(|l| {
            l.starts_with("You were killed for running out of memory")
                && l.ends_with("and have been restarted.")
        }) && t.contains(work))
    };
    while !fake.sent().iter().any(is_note) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A turn that never reaches its `result`: init, some text, then the replay holds.
fn held_turn() -> FakeDriver {
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"working on it"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.01}"#,
    ];
    FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![2],
            ..ReplayOptions::default()
        })
}

fn cut_off_text(s: &Snapshot) -> Option<String> {
    s.inbox.iter().find_map(|i| match &i.kind {
        InboxKind::Error { text } if text.contains("cut off by a restart") => Some(text.clone()),
        _ => None,
    })
}

/// State dirs for a run whose recipe and config are written before the core starts.
fn profile_paths(dir: &Path) -> Paths {
    Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("unused"),
    }
}

fn config_dir_env(spec: &dovetail_driver::SessionSpec) -> Option<String> {
    spec.env
        .iter()
        .find(|(k, _)| k == "CLAUDE_CONFIG_DIR")
        .map(|(_, v)| v.clone())
}

/// The transcript folder name Claude Code uses for `cwd`.
fn encoded(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A fake native `claude`: records its arguments, folder and Claude home in `out`, appends one
/// native turn to the session's transcript (under `$CLAUDE_CONFIG_DIR/projects/<folder>`), and
/// exits once `wait` exists (at once without one).
fn fake_claude(out: &Path, folder: &str, wait: Option<&Path>) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt;
    let wait = wait.map(|w| w.display().to_string()).unwrap_or_default();
    let script = out.join("fake-claude");
    let body = format!(
        r#"#!/bin/sh
out='{out}'
printf '%s\n' "$@" > "$out/argv"
pwd -P > "$out/cwd"
printf '%s' "$CLAUDE_CONFIG_DIR" > "$out/config_dir"
printf '%s' "$DOVETAIL_AGENT_ID" > "$out/agent"
prev=
for a in "$@"; do [ "$prev" = --resume ] && id=$a; prev=$a; done
t="$CLAUDE_CONFIG_DIR/projects/{folder}/$id.jsonl"
mkdir -p "$(dirname "$t")"
cat >> "$t" <<'LINES'
{{"type":"user","message":{{"role":"user","content":"native hello"}}}}
{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"native reply"}},{{"type":"tool_use","id":"t1","name":"Bash","input":{{}}}}]}}}}
LINES
touch "$out/started"
while [ -n '{wait}' ] && [ ! -e '{wait}' ]; do sleep 0.05; done
exit 0
"#,
        out = out.display(),
    );
    std::fs::write(&script, body).expect("fake claude");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("runnable");
    script.into_os_string()
}

fn lock_of(s: &Snapshot, id: &str) -> dovetail_api::agent::SessionLock {
    s.agents
        .iter()
        .find(|a| a.id.0 == id)
        .map(|a| a.lock)
        .unwrap_or_else(|| panic!("no agent {id}"))
}

fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Opens a project whose coordinator was taken over when Dovetail stopped: its transcript has
/// one line from before the take-over and one native turn after, and its lock names `pid`.
fn open_after_crash(pid: u32, ticks: Option<u64>) -> (Harness, FakeDriver, TempDir) {
    use dovetail_core::takeover::TakeoverLock;
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    // Filed where Claude Code files the coordinator's session: under its `home/` folder.
    let transcript = dovetail_core::session::launch::transcript_path(
        &mine,
        &paths.project_home("p"),
        &format!("{COORDINATOR}-session"),
    );
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("dir");
    std::fs::write(&transcript, "{\"type\":\"summary\"}\n").expect("transcript");
    let offset = std::fs::metadata(&transcript).expect("len").len();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("open");
    writeln!(
        file,
        "{{\"type\":\"user\",\"message\":{{\"content\":\"typed while Dovetail was down\"}}}}"
    )
    .expect("append");
    let lock = TakeoverLock {
        since_ms: 1,
        pid: Some(pid),
        start_ticks: ticks,
        transcript_offset: Some(offset),
    };
    seed(
        &paths,
        session_row(
            COORDINATOR,
            AgentKind::Coordinator,
            None,
            &mine,
            transcript,
            Some(lock),
        ),
    );
    let fake = fixture("plain_turn");
    let h = Harness::start_with(fake.clone(), paths, None, config);
    (h, fake, dir)
}

/// Processes that cannot be checked, as when `/proc` is unreadable.
struct Blind;

impl dovetail_core::takeover::Processes for Blind {
    fn start_ticks(&self, _: u32) -> std::io::Result<Option<u64>> {
        Err(std::io::Error::other("no /proc here"))
    }
    fn find_native(&self, _: &str) -> std::io::Result<Option<u32>> {
        Err(std::io::Error::other("no /proc here"))
    }
}
