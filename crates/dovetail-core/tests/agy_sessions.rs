//! Session tests for the agy driver, driver switching and usage reports.

mod common;

use common::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dovetail_api::tools::{StartSlice, Tool};
use dovetail_api::{
    AgentId, AgentKind, AgentStatus, Command, NewTask, PermissionMode, Snapshot, TaskId,
};
use dovetail_core::config::Config;
use dovetail_core::paths::Paths;
use dovetail_driver::{FakeDriver, ReplayOptions, Sent};

#[test]
fn agy_coordinator_launches_with_settings_mcp_and_broken_claude_profile() {
    use dovetail_api::DriverKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    // A missing Claude profile makes `self.claude` an Err: agy must still launch.
    std::fs::create_dir_all(paths.recipe("p").parent().unwrap()).unwrap();
    std::fs::write(paths.recipe("p"), "profile = \"missing\"\n").unwrap();

    let mut row = session_row(
        COORDINATOR,
        AgentKind::Coordinator,
        None,
        dir.path(),
        PathBuf::new(),
        None,
    );
    row.agent.driver = DriverKind::Agy;
    row.session_id = None;
    row.transcript_path = None;
    seed(&paths, row);

    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn");
    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths.clone(),
        None,
        Config::default(),
    );
    prompt(&h, "Reply with the single word: pong");
    h.wait_for("agy reply", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "pong"))
    });

    assert!(claude_fake.started_specs().is_empty());
    let specs = agy_fake.started_specs();
    assert_eq!(specs.len(), 1);
    let spec = &specs[0];
    assert!(!spec.resume);
    let settings_dir = spec.agy.settings_dir.as_ref().expect("agy settings_dir");
    assert_eq!(settings_dir, &paths.run_dir("p", COORDINATOR).join("agy"));
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(settings_dir.join("settings.json")).expect("settings.json"),
    )
    .expect("json");
    assert_eq!(
        settings,
        serde_json::json!({ "toolPermission": "request-review" })
    );
    assert_eq!(spec.agy.mcp_servers.len(), 1);
    assert_eq!(spec.agy.mcp_servers[0].name, "dovetail");
    assert_eq!(spec.agy.mcp_servers[0].args, vec!["mcp"]);
    assert!(spec.agy.mcp_servers[0].env.contains(&(
        dovetail_api::tools::env::AGENT_ID.into(),
        COORDINATOR.into()
    )));
    assert!(
        spec.agy
            .allowed_tools
            .contains(&"mcp_dovetail_core_report_status".into())
    );
    assert!(!spec.env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR"));
    h.quit();
}

#[test]
fn agy_init_records_id_and_transcript_keeps_idle_and_auto_and_resumes() {
    use dovetail_api::DriverKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    // Broken Claude profile verifies `restore.rs` still starts an `agy` coordinator.
    std::fs::create_dir_all(paths.recipe("p").parent().unwrap()).unwrap();
    std::fs::write(paths.recipe("p"), "profile = \"missing\"\n").unwrap();

    let mut row = session_row(
        COORDINATOR,
        AgentKind::Coordinator,
        None,
        dir.path(),
        dovetail_core::session::launch::agy_transcript_path("old-agy-id"),
        None,
    );
    row.agent.driver = DriverKind::Agy;
    row.session_id = Some("old-agy-id".into());
    seed(&paths, row);

    // Pause right after the `session/new` response (`Init`), before any prompt or `config_option_update`.
    let agy_fake = agy_fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![2],
        ..Default::default()
    });
    let mut h = Harness::start_both(
        fixture("plain_turn"),
        agy_fake.clone(),
        paths.clone(),
        None,
        Config::default(),
    );
    h.wait_for("agy init", |s| {
        s.agents
            .iter()
            .any(|a| a.id.0 == COORDINATOR && a.model.as_deref() == Some("gemini-4-neon-high"))
    });

    let coord = h
        .snap
        .agents
        .iter()
        .find(|a| a.id.0 == COORDINATOR)
        .expect("coordinator");
    assert_eq!(coord.status, AgentStatus::Idle);
    assert_eq!(coord.permission_mode, PermissionMode::Auto);
    assert!(
        !h.snap
            .warnings
            .iter()
            .any(|w| w.id.0 == format!("mode-{COORDINATOR}")),
        "unexpected downgrade warning: {:?}",
        h.snap.warnings
    );
    assert_eq!(agy_fake.started_specs()[0].session_id, "old-agy-id");
    assert!(agy_fake.started_specs()[0].resume);

    agy_fake.resume();
    let _state = h.quit();

    let agy_id = "bfbc337e-0bc1-4d82-b5e8-1db97b63f1ed";
    let expected_transcript = dovetail_core::session::launch::agy_transcript_path(agy_id);
    let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
    let loaded = db.load_agents().wait().expect("agents");
    let coord_row = loaded
        .iter()
        .find(|r| r.agent.id.0 == COORDINATOR)
        .expect("coord row");
    assert_eq!(coord_row.session_id.as_deref(), Some(agy_id));
    assert_eq!(
        coord_row.transcript_path.as_ref(),
        Some(&expected_transcript)
    );
    drop(db);

    // Next open resumes using the agy conversation id recorded from `Init`.
    let agy_fake_2 = agy_fixture("plain_turn");
    let mut h = Harness::start_both(
        fixture("plain_turn"),
        agy_fake_2.clone(),
        paths,
        None,
        Config::default(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while agy_fake_2.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "agy coordinator never resumed");
        h.drain(Duration::from_millis(20));
    }
    let resumed = &agy_fake_2.started_specs()[0];
    assert!(resumed.resume);
    assert_eq!(resumed.session_id, agy_id);
    h.quit();
}

#[test]
fn native_take_over_is_refused_for_agy_agent() {
    use dovetail_api::DriverKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let mut row = session_row(
        COORDINATOR,
        AgentKind::Coordinator,
        None,
        dir.path(),
        dovetail_core::session::launch::agy_transcript_path("agy-1"),
        None,
    );
    row.agent.driver = DriverKind::Agy;
    row.session_id = Some("agy-1".into());
    seed(&paths, row);

    let mut h = Harness::start_both(
        fixture("plain_turn"),
        agy_fixture("plain_turn"),
        paths,
        None,
        Config::default(),
    );
    h.send(Command::TakeOver {
        agent: AgentId(COORDINATOR.into()),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.returned.is_empty() {
        assert!(Instant::now() < deadline, "no TerminalReturned");
        h.drain(Duration::from_millis(20));
    }
    let err = h.returned[0].as_deref().expect("expected refusal");
    assert!(err.contains("agy") && err.contains("Claude Code"), "{err}");
    h.quit();
}

#[test]
fn switch_driver_to_agy_with_handoff_seeds_new_agy_coordinator() {
    use dovetail_api::{DriverKind, ReasoningEffort};

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let claude_fake = fresh_fixture(4);
    let agy_fake = agy_fixture("plain_turn");
    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths,
        None,
        Config::default(),
    );
    prompt(&h, "first");
    h.wait_for("first reply", |s| finished_turns(s) == 1);

    h.send(Command::SwitchDriver {
        agent: AgentId(COORDINATOR.into()),
        driver: DriverKind::Agy,
        model: "gemini-3.8-flash".into(),
        effort: Some(ReasoningEffort::Low),
        handoff: true,
    });
    h.wait_for("closing handoff", |s| journal(s).closing);
    // Coordinator must still be Claude until the old session exits.
    assert_eq!(coordinator_of(&h.snap).driver, DriverKind::Claude);

    claude_fake.resume();
    wait_specs(&agy_fake, 1);
    h.wait_for("agy coordinator ready", |s| {
        let c = coordinator_of(s);
        c.driver == DriverKind::Agy && c.status != AgentStatus::SettingUp
    });

    let spec = &agy_fake.started_specs()[0];
    assert!(!spec.resume);
    assert_eq!(spec.model.as_deref(), Some("gemini-3.8-flash"));
    assert_eq!(spec.effort, Some(ReasoningEffort::Low));
    let seed = agy_fake
        .sent()
        .into_iter()
        .find_map(|s| match s {
            Sent::User(text) => Some(text),
            _ => None,
        })
        .expect("seed prompt sent to agy");
    assert!(seed.contains("Open: PR 20 waits on review."), "{seed}");
    assert!(seed.contains("Its transcript is at "), "{seed}");
    assert!(seed.contains(".jsonl."), "{seed}");
    h.quit();
}

#[test]
fn switch_driver_with_handoff_falls_back_to_facts_when_old_session_errors() {
    use dovetail_api::DriverKind;

    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"first"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"one","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"fresh"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"result":"You've hit your limit","total_cost_usd":0.0}"#,
    ];
    let claude_fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![4],
            ..ReplayOptions::default()
        });
    let agy_fake = agy_fixture("plain_turn");

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths,
        None,
        Config::default(),
    );
    prompt(&h, "first");
    h.wait_for("first reply", |s| finished_turns(s) == 1);

    h.send(Command::SwitchDriver {
        agent: AgentId(COORDINATOR.into()),
        driver: DriverKind::Agy,
        model: "claude-opus-5-5".into(),
        effort: None,
        handoff: true,
    });
    h.wait_for("closing handoff", |s| journal(s).closing);
    claude_fake.resume();

    wait_specs(&agy_fake, 1);
    h.wait_for("switched to agy despite error", |s| {
        coordinator_of(s).driver == DriverKind::Agy && !journal(s).closing
    });
    assert!(
        !h.failures.iter().any(|f| f.contains("cancelled")),
        "switch must not cancel on NoHandoff: {:?}",
        h.failures
    );

    let today = local_date(0);
    let journal_text = std::fs::read_to_string(
        h.paths
            .data
            .join("projects/p/journal")
            .join(format!("{today}.md")),
    )
    .expect("journal written");
    assert!(
        journal_text.contains("_No handoff from the coordinator: its turn ended with an error: You've hit your limit._"),
        "{journal_text}"
    );
    assert!(journal_text.contains("## Facts"), "{journal_text}");

    let seed = agy_fake
        .sent()
        .into_iter()
        .find_map(|s| match s {
            Sent::User(text) => Some(text),
            _ => None,
        })
        .expect("fallback seed sent to agy");
    assert!(seed.contains("You've hit your limit"), "{seed}");
    assert!(seed.contains("# Facts"), "{seed}");
    assert!(seed.contains("Its transcript is at "), "{seed}");
    assert!(seed.contains(".jsonl."), "{seed}");
    h.quit();
}

#[test]
fn switch_driver_clean_start_and_refusal_for_slice() {
    use dovetail_api::DriverKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn");
    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths,
        None,
        Config::default(),
    );
    prompt(&h, "first");
    h.wait_for("first reply", |s| finished_turns(s) == 1);

    // Refused for a non-coordinator agent.
    h.send(Command::NewSlice {
        task: None,
        name: "work".into(),
        brief: "go".into(),
        repo: None,
    });
    h.wait_for("slice created", |s| {
        s.agents.iter().any(|a| a.name == "work")
    });
    h.send(Command::SwitchDriver {
        agent: AgentId("slice-work".into()),
        driver: DriverKind::Agy,
        model: "gemini-3.8-flash".into(),
        effort: None,
        handoff: false,
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h
        .failures
        .iter()
        .any(|f| f.contains("only the coordinator can switch driver"))
    {
        assert!(Instant::now() < deadline, "no refusal: {:?}", h.failures);
        h.drain(Duration::from_millis(20));
    }

    // Clean start on coordinator: no journal, no seed, starts on agy.
    h.send(Command::SwitchDriver {
        agent: AgentId(COORDINATOR.into()),
        driver: DriverKind::Agy,
        model: "gemini-3.8-flash".into(),
        effort: None,
        handoff: false,
    });
    wait_specs(&agy_fake, 1);
    h.wait_for("coordinator on agy", |s| {
        coordinator_of(s).driver == DriverKind::Agy
    });
    h.wait_for("kept transcript notice", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.text.contains("transcript is kept at ") && e.text.contains(".jsonl"))
            })
    });
    let spec = &agy_fake.started_specs()[0];
    assert!(!spec.resume);
    assert_eq!(spec.model.as_deref(), Some("gemini-3.8-flash"));
    assert!(
        !agy_fake.sent().iter().any(|s| matches!(s, Sent::User(_))),
        "clean start sends no seed: {:?}",
        agy_fake.sent()
    );
    let journal_file = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{}.md", local_date(0)));
    assert!(!journal_file.exists(), "clean start writes no journal");
    h.quit();
}

#[test]
fn load_usage_emits_both_reports_in_driver_order_with_claude_config_dir() {
    use dovetail_api::{DriverKind, DriverUsage, LimitWindow, UsageGroup};

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let custom_home = dir.path().join("custom-claude");
    std::fs::create_dir_all(paths.recipe("p").parent().unwrap()).unwrap();
    std::fs::write(paths.recipe("p"), "profile = \"custom\"\n").unwrap();
    let mut config = Config::default();
    config
        .profiles
        .insert("custom".into(), custom_home.to_string_lossy().into_owned());

    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn");
    let claude_report = DriverUsage {
        driver: DriverKind::Claude,
        plan: Some("max".into()),
        groups: vec![UsageGroup {
            name: "Claude".into(),
            models: String::new(),
            windows: vec![LimitWindow {
                name: "5-hour".into(),
                used_fraction: 0.25,
                resets_at_ms: Some(1_000),
            }],
        }],
        note: None,
        failed: false,
    };
    let agy_report = DriverUsage {
        driver: DriverKind::Agy,
        plan: Some("Google AI Pro".into()),
        groups: vec![UsageGroup {
            name: "Gemini".into(),
            models: "Gemini 3.8".into(),
            windows: vec![LimitWindow {
                name: "5-hour".into(),
                used_fraction: 0.1,
                resets_at_ms: Some(2_000),
            }],
        }],
        note: None,
        failed: false,
    };
    claude_fake.set_usage(claude_report.clone());
    agy_fake.set_usage(agy_report.clone());

    let mut h = Harness::start_both(claude_fake.clone(), agy_fake.clone(), paths, None, config);
    h.send(Command::LoadUsage);
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.usage_loaded.is_empty() {
        assert!(Instant::now() < deadline, "UsageLoaded never arrived");
        h.drain(Duration::from_millis(20));
    }
    assert_eq!(h.usage_loaded[0], vec![claude_report, agy_report]);

    let claude_queries = claude_fake.usage_queries();
    let agy_queries = agy_fake.usage_queries();
    assert_eq!(claude_queries.len(), 1);
    assert_eq!(agy_queries.len(), 1);
    assert_eq!(
        claude_queries[0].env,
        vec![(
            "CLAUDE_CONFIG_DIR".into(),
            custom_home.to_string_lossy().into_owned()
        )]
    );
    assert!(agy_queries[0].env.is_empty());
    h.quit();
}

#[test]
fn agy_models_event_leaves_claude_options_untouched_and_adds_agy_options() {
    use dovetail_api::{DriverKind, ReasoningEffort};
    use dovetail_driver::{DriverEvent, ModelInfo};

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let claude_fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let agy_fake = agy_fixture("plain_turn");
    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths,
        None,
        Config::default(),
    );
    prompt(&h, "hi");
    wait_specs(&claude_fake, 1);
    claude_fake.send_event(DriverEvent::Models(vec![ModelInfo {
        value: "sonnet".into(),
        resolved: Some("claude-sonnet-5-5".into()),
        display_name: None,
        efforts: ReasoningEffort::ALL.to_vec(),
    }]));
    h.wait_for("claude models listed", |s| {
        s.models
            .iter()
            .any(|m| m.driver == DriverKind::Claude && m.name == "sonnet" && m.efforts.is_some())
    });

    // Start an agy slice so agy emits its own DriverEvent::Models.
    h.send(Command::CreateTask(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    }));
    h.wait_for("task", |s| !s.tasks.is_empty());
    let (text, is_error) = call_coordinator(
        &h,
        Tool::StartSlice(StartSlice {
            task: TaskId(1),
            name: "agy-slice".into(),
            brief: "go".into(),
            repo: None,
            model: None,
            effort: None,
            driver: Some(DriverKind::Agy),
        }),
    );
    assert!(!is_error && text.starts_with("started"), "{text}");
    h.wait_for("agy models added", |s| {
        s.models.iter().any(|m| m.driver == DriverKind::Agy)
    });

    let claude_opts: Vec<_> = h
        .snap
        .models
        .iter()
        .filter(|m| m.driver == DriverKind::Claude)
        .collect();
    assert_eq!(claude_opts.len(), dovetail_api::BUILTIN_MODELS.len());
    assert!(
        claude_opts
            .iter()
            .any(|m| m.name == "sonnet" && m.efforts == Some(ReasoningEffort::ALL.to_vec())),
        "Claude options must stay intact: {claude_opts:?}"
    );
    let agy_opts: Vec<_> = h
        .snap
        .models
        .iter()
        .filter(|m| m.driver == DriverKind::Agy)
        .collect();
    assert!(
        agy_opts
            .iter()
            .any(|m| m.name == "gemini-3.8-flash" && m.efforts.is_some()),
        "{agy_opts:?}"
    );
    h.quit();
}

#[test]
fn project_start_slice_on_agy_creates_worktree_and_starts_agy_without_sonnet_default() {
    use dovetail_api::DriverKind;

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
    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn");
    let config = Config {
        default_model: Some("opus".into()),
        ..Config::default()
    };
    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths.clone(),
        Some(repo),
        config,
    );
    make_task(&mut h);

    // Effort without model on agy is denied.
    let (err_text, is_error) = call_coordinator(
        &h,
        Tool::StartSlice(StartSlice {
            task: TaskId(1),
            name: "bad".into(),
            brief: "go".into(),
            repo: None,
            model: None,
            effort: Some(dovetail_api::ReasoningEffort::High),
            driver: Some(DriverKind::Agy),
        }),
    );
    assert!(is_error && err_text.starts_with("denied"), "{err_text}");

    let (text, is_error) = call_coordinator(
        &h,
        Tool::StartSlice(StartSlice {
            task: TaskId(1),
            name: "agy-wt".into(),
            brief: "work in worktree".into(),
            repo: None,
            model: None,
            effort: None,
            driver: Some(DriverKind::Agy),
        }),
    );
    assert!(!is_error && text.starts_with("started"), "{text}");

    wait_specs(&agy_fake, 1);
    h.wait_for("agy slice ready and answered", |s| {
        s.agents.iter().any(|a| {
            a.name == "agy-wt" && a.driver == DriverKind::Agy && a.status != AgentStatus::SettingUp
        })
    });

    assert!(
        !claude_fake
            .started_specs()
            .iter()
            .any(|s| s.name.as_deref() == Some("p.agy-wt")),
        "agy slice must not launch on claude"
    );
    let expected_wt = paths.worktrees("p").join("agy-wt");
    assert!(expected_wt.join(".git").exists(), "worktree was created");
    assert_eq!(agy_fake.started_specs().len(), 1);
    let spec = &agy_fake.started_specs()[0];
    assert_eq!(spec.name.as_deref(), Some("p.agy-wt"));
    assert_eq!(spec.cwd, expected_wt);
    assert_eq!(spec.model, None, "must not default agy slice to sonnet");
    assert_eq!(spec.claude.worktree, None);
    assert!(spec.agy.settings_dir.is_some());
    assert!(
        agy_fake
            .sent()
            .iter()
            .any(|s| matches!(s, Sent::User(text) if text.contains("work in worktree"))),
        "brief sent to agy slice: {:?}",
        agy_fake.sent()
    );
    h.wait_for("coordinator notified of agy slice", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries.iter().any(|e| {
                    e.text
                        .contains("slice agy-wt ready (runs on agy; cannot receive SendMessage)")
                })
            })
    });
    h.quit();
}

#[test]
fn queued_agy_slice_preserves_driver_across_queue() {
    use dovetail_api::DriverKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn");
    let config = Config {
        max_slices: 1,
        ..Config::default()
    };
    let mut h = Harness::start_both(claude_fake.clone(), agy_fake.clone(), paths, None, config);
    make_task(&mut h);

    let (text, is_error) = call_coordinator(
        &h,
        Tool::StartSlice(StartSlice {
            task: TaskId(1),
            name: "first".into(),
            brief: "go".into(),
            repo: None,
            model: None,
            effort: None,
            driver: None,
        }),
    );
    assert!(!is_error && text.starts_with("started"), "{text}");

    let (text, is_error) = call_coordinator(
        &h,
        Tool::StartSlice(StartSlice {
            task: TaskId(1),
            name: "second-agy".into(),
            brief: "go agy".into(),
            repo: None,
            model: Some("gemini-3.8-flash".into()),
            effort: None,
            driver: Some(DriverKind::Agy),
        }),
    );
    assert!(!is_error && text.starts_with("queued"), "{text}");
    assert!(agy_fake.started_specs().is_empty());

    h.send(Command::CloseSlice {
        agent: AgentId("slice-first".into()),
    });
    wait_specs(&agy_fake, 1);
    h.wait_for("queued agy slice started", |s| {
        s.agents
            .iter()
            .any(|a| a.name == "second-agy" && a.driver == DriverKind::Agy)
    });
    let spec = &agy_fake.started_specs()[0];
    assert_eq!(spec.name.as_deref(), Some("p.second-agy"));
    assert_eq!(spec.model.as_deref(), Some("gemini-3.8-flash"));
    h.quit();
}

fn agy_model_names(s: &Snapshot) -> Vec<String> {
    s.models
        .iter()
        .filter(|m| m.driver == dovetail_api::DriverKind::Agy)
        .map(|m| m.name.clone())
        .collect()
}

fn agy_listing(name: &str) -> Vec<dovetail_driver::ModelInfo> {
    vec![dovetail_driver::ModelInfo {
        value: name.into(),
        resolved: None,
        display_name: None,
        efforts: vec![dovetail_api::ReasoningEffort::Low],
    }]
}

fn test_paths(dir: &tempfile::TempDir) -> Paths {
    Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    }
}

#[test]
fn a_first_run_with_no_saved_list_and_a_failed_read_offers_the_built_in_agy_models() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agy_fake = agy_fixture("plain_turn");
    agy_fake.set_models(Err(dovetail_driver::ModelsError(
        "agy is not logged in".into(),
    )));
    let mut h = Harness::start_both(
        fixture("plain_turn"),
        agy_fake.clone(),
        test_paths(&dir),
        None,
        Config::default(),
    );
    h.wait_for("built-in agy models", |s| {
        agy_model_names(s).contains(&"claude-opus-5-5".to_string())
    });
    h.wait_for("the refresh was tried", |_| agy_fake.models_asked() == 1);
    h.drain(Duration::from_millis(200));
    assert!(agy_model_names(&h.snap).contains(&"claude-opus-5-5".to_string()));
    h.quit();
}

#[test]
fn the_background_read_replaces_agy_models_and_saves_them_and_a_later_failure_keeps_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = agy_fixture("plain_turn");
    first.set_models(Ok(agy_listing("gemini-9")));
    let mut h = Harness::start_both(
        fixture("plain_turn"),
        first.clone(),
        test_paths(&dir),
        None,
        Config::default(),
    );
    h.wait_for("the listed models replace the built-in ones", |s| {
        agy_model_names(s) == vec!["gemini-9".to_string()]
    });
    h.quit();

    // The next start: agy cannot list its models, yet the saved list is there at once.
    let second = agy_fixture("plain_turn");
    second.set_models(Err(dovetail_driver::ModelsError("timed out".into())));
    let mut h = Harness::start_both(
        fixture("plain_turn"),
        second.clone(),
        test_paths(&dir),
        None,
        Config::default(),
    );
    h.wait_for("the saved list at start", |s| {
        agy_model_names(s) == vec!["gemini-9".to_string()]
    });
    h.wait_for("the refresh was tried", |_| second.models_asked() == 1);
    h.drain(Duration::from_millis(200));
    assert_eq!(agy_model_names(&h.snap), vec!["gemini-9".to_string()]);
    h.quit();
}

#[test]
fn without_agy_there_are_no_agy_models_usage_group_or_refresh() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Only Claude Code is registered, as on a machine with no `agy` binary.
    let mut h = Harness::start_on(fixture("plain_turn"), test_paths(&dir));
    h.drain(Duration::from_millis(300));
    assert!(agy_model_names(&h.snap).is_empty(), "{:?}", h.snap.models);

    h.send(Command::LoadUsage);
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.usage_loaded.is_empty() {
        assert!(Instant::now() < deadline, "UsageLoaded never arrived");
        h.drain(Duration::from_millis(20));
    }
    assert!(
        h.usage_loaded[0]
            .iter()
            .all(|r| r.driver == dovetail_api::DriverKind::Claude),
        "{:?}",
        h.usage_loaded[0]
    );
    h.quit();
}

#[test]
fn agy_session_start_refreshes_agy_usage_on_snapshot_and_throttles_turn() {
    use dovetail_api::{DriverKind, DriverUsage, LimitWindow, UsageGroup};

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    std::fs::create_dir_all(paths.recipe("p").parent().unwrap()).unwrap();
    std::fs::write(paths.recipe("p"), "profile = \"missing\"\n").unwrap();

    let mut row = session_row(
        COORDINATOR,
        AgentKind::Coordinator,
        None,
        dir.path(),
        dovetail_core::session::launch::agy_transcript_path("old-agy-id"),
        None,
    );
    row.agent.driver = DriverKind::Agy;
    row.session_id = Some("old-agy-id".into());
    seed(&paths, row);

    let agy_report = DriverUsage {
        driver: DriverKind::Agy,
        plan: Some("Pro".into()),
        groups: vec![UsageGroup {
            name: "Gemini Models".into(),
            models: "Gemini 4 Neon".into(),
            windows: vec![LimitWindow {
                name: "5-hour".into(),
                used_fraction: 0.15,
                resets_at_ms: Some(1_700_000_000_000),
            }],
        }],
        note: None,
        failed: false,
    };

    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn");
    agy_fake.set_usage(agy_report.clone());

    let mut h = Harness::start_both(
        claude_fake.clone(),
        agy_fake.clone(),
        paths.clone(),
        None,
        Config::default(),
    );

    // Initial session start (on_init) triggers background agy usage refresh
    h.wait_for("agy usage loaded into snapshot", |s| {
        s.agy_usage.as_ref() == Some(&agy_report)
    });
    assert_eq!(agy_fake.usage_queries().len(), 1);

    // Prompting and running a turn finishes (on_result), but refresh is throttled (within 3 mins)
    prompt(&h, "Reply with the single word: pong");
    h.wait_for("turn finished", |s| finished_turns(s) == 1);

    // Ensure no additional usage query was triggered due to throttling
    h.drain(Duration::from_millis(200));
    assert_eq!(agy_fake.usage_queries().len(), 1);
    assert_eq!(h.snap.agy_usage.as_ref(), Some(&agy_report));

    h.quit();
}

#[test]
fn agy_invoke_subagent_lifecycle_in_tui_tree() {
    use dovetail_api::DriverKind;
    use dovetail_driver::{
        AssistantMessage, ContentBlock, DriverEvent, Origin, ToolResult, TurnResult,
    };
    use serde_json::json;

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    std::fs::create_dir_all(paths.recipe("p").parent().unwrap()).unwrap();
    std::fs::write(paths.recipe("p"), "profile = \"missing\"\n").unwrap();

    let mut row = session_row(
        COORDINATOR,
        AgentKind::Coordinator,
        None,
        dir.path(),
        PathBuf::new(),
        None,
    );
    row.agent.driver = DriverKind::Agy;
    row.session_id = None;
    row.transcript_path = None;
    seed(&paths, row);

    let claude_fake = fixture("plain_turn");
    let agy_fake = agy_fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![10],
        ..Default::default()
    });
    let mut h = Harness::start_both(
        claude_fake,
        agy_fake.clone(),
        paths,
        None,
        Config::default(),
    );

    // Initial prompt starts the agy session and turn
    prompt(&h, "count the lines with subagent");
    h.wait_for("agy coordinator working", |s| {
        s.agents
            .iter()
            .any(|a| a.id.0 == COORDINATOR && a.status == AgentStatus::Working)
    });

    // Send invoke_subagent tool call
    let tool_id = "call_invoke_1";
    agy_fake.send_event(DriverEvent::Assistant(AssistantMessage {
        origin: Origin::default(),
        message_id: None,
        uuid: None,
        blocks: vec![ContentBlock::ToolUse {
            id: tool_id.into(),
            name: "invoke_subagent".into(),
            input: json!({
                "Role": "Line Counter",
                "TypeName": "research",
                "Prompt": "Read lines.txt and count lines."
            }),
        }],
        model: None,
        usage: None,
    }));

    // 1. invoke_subagent creates a subagent row in Snapshot.agents named after Role with background: true
    h.wait_for("subagent created in snapshot", |s| {
        s.agents.iter().any(|a| {
            matches!(a.kind, AgentKind::Subagent { background: true })
                && a.name == "Line Counter"
                && a.status == AgentStatus::Working
        })
    });

    // 2. Immediate ToolResult for invoke_subagent leaves the subagent running (does NOT close it)
    agy_fake.send_event(DriverEvent::ToolResult(ToolResult {
        origin: Origin::default(),
        tool_use_id: tool_id.into(),
        content: serde_json::Value::Null,
        is_error: false,
        meta: None,
    }));

    h.drain(Duration::from_millis(100));
    let sub = h
        .snap
        .agents
        .iter()
        .find(|a| matches!(a.kind, AgentKind::Subagent { .. }))
        .expect("subagent still in snapshot");
    assert_eq!(sub.name, "Line Counter");
    assert_eq!(sub.kind, AgentKind::Subagent { background: true });
    assert_eq!(sub.status, AgentStatus::Working);

    // 3. Parent TurnResult marks the agy subagent Done
    agy_fake.send_event(DriverEvent::TurnResult(TurnResult {
        subtype: "success".into(),
        is_error: false,
        result: Some("3 lines".into()),
        result_index: Some(1),
        terminal_reason: None,
        stop_reason: Some("end_turn".into()),
        num_turns: None,
        duration_ms: None,
        total_cost_usd: None,
        model_usage: vec![],
        permission_denials: vec![],
        queued_turn_count: None,
        raw: serde_json::Value::Null,
    }));

    h.wait_for("subagent marked Done on parent turn completion", |s| {
        s.agents.iter().any(|a| {
            matches!(a.kind, AgentKind::Subagent { .. })
                && a.name == "Line Counter"
                && a.status == AgentStatus::Done
        })
    });

    h.quit();
}
