//! Integration tests for the `tracking` domain: projects, multi-repo management, tasks,
//! decisions, efforts, and automatic task status.

mod common;

use common::*;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dovetail_api::{
    Command, Snapshot,
    agent::{AgentId, AgentKind},
    conversation::EntryKind,
    decision::{Alternative, DecisionId},
    inbox::InboxKind,
    task::{Bucket, NewTask, PLevel, TaskId, TaskStatus},
};
use dovetail_api::{
    agent::AgentCommand, conversation::ConversationCommand, effort::EffortCommand,
    project::ProjectCommand, task::TaskCommand, worktree::WorktreeCommand,
};
use dovetail_core::config::Config;
use dovetail_core::paths::Paths;
use dovetail_driver::{ReplayOptions, Sent};
use dovetail_tools::project::archive_result::ARCHIVED;
use dovetail_tools::{SocketRequestBody, SocketResponseBody};
use dovetail_tools::{
    Tool, report::LogDecision, task::ArchiveTask, task::BucketFilter, task::GetTask,
    task::ListTasks, task::UpdateTask,
};
use dovetail_tools::{coordination::CoordinationTool, report::ReportTool, task::TaskTool};
use tempfile::TempDir;

#[test]
fn starting_a_slice_on_a_next_task_moves_it_to_now_in_progress() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: Some(Bucket::Next),
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    assert_eq!(h.snap.tasks[0].bucket, Some(Bucket::Next));
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    }));
    let started = |s: &Snapshot| {
        s.tasks.iter().any(|t| {
            t.id == id && t.status == TaskStatus::InProgress && t.bucket == Some(Bucket::Now)
        })
    };
    h.wait_for("the task in Now, in progress", started);

    // Through the storage thread: still there after a restart.
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the task after the restart", started);
    h.quit();
}

#[test]
fn an_unregistered_folder_keeps_nothing_until_added() {
    let mut h = Harness::start(fixture("plain_turn"), false, Config::default());
    assert!(h.snap.warnings.iter().any(|w| w.id.0 == "unregistered"));
    let socket = h.socket();
    match socket_call(
        &socket,
        SocketRequestBody::AddTask {
            task: NewTask {
                title: "t".into(),
                p_level: None,
                bucket: None,
                notes: String::new(),
                follow_up_of: None,
                pr: None,
                related: vec![],
                effort: Default::default(),
                criterion: None,
            },
        },
    ) {
        SocketResponseBody::Error { message } => {
            assert!(message.contains("not a Dovetail project"))
        }
        other => panic!("{other:?}"),
    }
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "kept after adding".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    // Only Dovetail's own built-in Tangents (ADR 0017) has a folder; nothing for this one.
    let written: Vec<String> = std::fs::read_dir(h.paths.data.join("projects"))
        .map(|dir| {
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(written, ["tangents"], "nothing written yet");
    // A slice may start before the folder is a project (decided 2026-11-06).
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "early".into(),
        brief: "go".into(),
    }));
    h.wait_for("the early slice's brief", |s| {
        s.conversations
            .get(&AgentId("slice-early".into()))
            .is_some_and(|t| !t.entries.is_empty())
    });
    let path = h.repo.clone();
    h.send(Command::Project(ProjectCommand::Register { path }));
    h.wait_for("registration", |s| {
        s.warnings.iter().all(|w| w.id.0 != "unregistered")
    });
    assert!(h.paths.state_db("work").exists());

    // Regression: after registration everything is named after the project id, not `new-<pid>`.
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(TaskId(1)),
        name: "after".into(),
        brief: "go".into(),
    }));
    h.wait_for("the slice's brief", |s| {
        s.conversations
            .get(&AgentId("slice-after".into()))
            .is_some_and(|t| !t.entries.is_empty())
    });
    let fake = h.fake.clone();
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| s.name.as_deref().is_some_and(|n| n.ends_with(".after")))
        .expect("slice spec");
    let env = |key: &str| {
        spec.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        env("CLAUDE_CODE_TASK_LIST_ID").as_deref(),
        Some("dovetail-work-task-1")
    );
    assert_eq!(env("DOVETAIL_PROJECT").as_deref(), Some("work"));
    let settings = spec.claude.settings_path.clone().expect("settings");
    assert!(
        settings.starts_with(h.paths.state.join("run").join("work")),
        "{settings:?}"
    );
    let paths = h.paths.clone();
    let _state = h.quit();
    // The slice started before registration was written to the project's state.db.
    let db = dovetail_core::db::Db::open(&paths, Some("work")).expect("db");
    let agents = db.load_agents().wait().expect("agents");
    assert!(agents.iter().any(|r| r.agent.name == "early"), "{agents:?}");
    // Sessions that ran before registration keep the list id their process used.
    for name in ["early", COORDINATOR] {
        let row = agents.iter().find(|r| r.agent.name == name).expect("agent");
        let list = row.task_list_id.as_deref().unwrap_or_default();
        assert!(list.starts_with("dovetail-new-"), "{name}: {list}");
    }
}

#[test]
fn archive_task_archives_the_task_and_its_live_slice() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    }));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let archive = Tool::Task(TaskTool::Archive(ArchiveTask { id }));

    let (text, is_error) = result_text(socket_call(&socket, tool("slice-work", archive.clone())));
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );

    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, archive.clone())));
    assert!(
        !is_error && text.starts_with(ARCHIVED) && text.contains("work"),
        "{text}"
    );
    h.wait_for("the task archived and the slice gone", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == id && t.status == TaskStatus::Archived)
            && !s.agents.iter().any(|a| a.name == "work")
    });

    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, archive)));
    assert!(is_error && text.contains("already archived"), "{text}");
    h.quit();
}

#[test]
fn list_tasks_and_get_task_read_tasks_slices_and_decisions_for_the_coordinator_only() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    for (title, bucket, p_level) in [
        ("later one", Some(Bucket::Later), Some(PLevel::P0)),
        ("unsorted one", None, None),
        ("next one", Some(Bucket::Next), Some(PLevel::P2)),
        ("urgent next", Some(Bucket::Next), Some(PLevel::P1)),
    ] {
        h.send(Command::Task(TaskCommand::Create(NewTask {
            title: title.into(),
            p_level,
            bucket,
            notes: format!("notes for {title}"),
            follow_up_of: None,
            pr: None,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        })));
    }
    h.wait_for("the tasks", |s| s.tasks.len() == 4);
    // Task 3 gets a slice, which moves it to Now.
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(TaskId(3)),
        name: "work".into(),
        brief: "go".into(),
    }));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let call = |agent: &str, t: Tool| result_text(socket_call(&socket, tool(agent, t)));

    let decide = |text: &str, supersedes: Option<DecisionId>| {
        Tool::Report(ReportTool::LogDecision(LogDecision {
            task: Some(TaskId(3)),
            decision: text.into(),
            why: Some(format!("why {text}")),
            alternatives: vec![Alternative {
                option: "other".into(),
                why_not: Some("worse".into()),
            }],
            supersedes,
        }))
    };
    let (text, is_error) = call(COORDINATOR, decide("first", None));
    assert!(!is_error, "{text}");
    let (text, is_error) = call(COORDINATOR, decide("second", Some(DecisionId(1))));
    assert!(!is_error, "{text}");

    let (text, is_error) = call(
        COORDINATOR,
        Tool::Task(TaskTool::List(ListTasks::default())),
    );
    assert!(!is_error, "{text}");
    let ids: Vec<&str> = text
        .lines()
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(ids, ["#3", "#4", "#2", "#1"], "{text}");
    assert!(
        text.contains("#3 [in_progress · now · P2] next one (slice work)"),
        "{text}"
    );

    let (text, _) = call(
        COORDINATOR,
        Tool::Task(TaskTool::List(ListTasks {
            bucket: Some(BucketFilter::Next),
            ..ListTasks::default()
        })),
    );
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("#4 "), "{text}");

    let (text, is_error) = call(
        COORDINATOR,
        Tool::Task(TaskTool::Get(GetTask { id: TaskId(3) })),
    );
    assert!(!is_error, "{text}");
    for want in [
        "# #3 next one",
        "notes for next one",
        "- slice work:",
        "- D1 (agent coordinator): first [superseded by D2]",
        "  Why: why first",
        "  Rejected: other, because worse",
        "- D2 (agent coordinator): second",
        "  Supersedes: D1",
    ] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }

    let (text, is_error) = call(
        COORDINATOR,
        Tool::Task(TaskTool::Get(GetTask { id: TaskId(99) })),
    );
    assert!(is_error && text.contains("no task #99"), "{text}");

    for refused in [
        Tool::Task(TaskTool::List(ListTasks::default())),
        Tool::Task(TaskTool::Get(GetTask { id: TaskId(3) })),
    ] {
        let (text, is_error) = call("slice-work", refused);
        assert!(
            is_error && text.contains("not in this session's tool suites"),
            "{text}"
        );
    }
    h.quit();
}

#[test]
fn update_task_changes_only_the_given_fields_and_may_move_into_now() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    for title in ["one", "two"] {
        h.send(Command::Task(TaskCommand::Create(NewTask {
            title: title.into(),
            p_level: Some(PLevel::P2),
            bucket: Some(Bucket::Next),
            notes: "old notes".into(),
            follow_up_of: None,
            pr: None,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        })));
    }
    h.wait_for("the tasks", |s| s.tasks.len() == 2);
    // Task 2 gets a slice, which moves it to Now.
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(TaskId(2)),
        name: "work".into(),
        brief: "go".into(),
    }));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let call = |agent: &str, u: UpdateTask| {
        result_text(socket_call(
            &socket,
            tool(agent, Tool::Task(TaskTool::Update(u))),
        ))
    };

    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            status: Some(TaskStatus::Done),
            ..update(1)
        },
    );
    // Status is Dovetail's to set (ADR 0024): the call is refused and nothing changes.
    assert!(is_error, "{text}");
    assert!(text.contains("Dovetail moves a task"), "{text}");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Later),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · later · P2] one");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            p_level: Some(Some(PLevel::P0)),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · later · P0] one");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            p_level: Some(None),
            bucket: Some(BucketFilter::Unsorted),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · unsorted] one");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            title: Some("  renamed ".into()),
            notes: Some("new notes".into()),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · unsorted] renamed");
    // The TUI sees each change live.
    h.wait_for("the edits in the snapshot", |s| {
        s.tasks.iter().any(|t| {
            t.id == TaskId(1)
                && t.title == "renamed"
                && t.notes == "new notes"
                && t.bucket.is_none()
                && t.p_level.is_none()
                && t.status == TaskStatus::Todo
        })
    });

    // Into Now is allowed, and the task is flagged as new until the user sees it.
    h.send(Command::Task(TaskCommand::Update {
        id: TaskId(1),
        patch: vec![dovetail_api::task::TaskChange::Seen],
    }));
    h.wait_for("task 1 seen", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == TaskId(1) && !t.coordinator_created)
    });
    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Now),
            ..update(1)
        },
    );
    assert!(!is_error, "{text}");
    assert!(text.starts_with("#1 [todo · now]"), "{text}");
    h.wait_for("task 1 unseen in Now", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == TaskId(1) && t.bucket == Some(Bucket::Now) && t.coordinator_created)
    });
    // Already in Now: a later update does not flag it again once the user has looked.
    h.send(Command::Task(TaskCommand::Update {
        id: TaskId(1),
        patch: vec![dovetail_api::task::TaskChange::Seen],
    }));
    h.wait_for("task 1 seen again", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == TaskId(1) && !t.coordinator_created)
    });
    let (_, is_error) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Now),
            p_level: Some(Some(PLevel::P1)),
            ..update(1)
        },
    );
    assert!(!is_error);
    h.drain(Duration::from_millis(100));
    assert!(
        h.snap
            .tasks
            .iter()
            .any(|t| t.id == TaskId(1) && !t.coordinator_created),
        "a task already in Now was flagged again"
    );

    // Out of Now is fine, and leaves the slice running.
    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Next),
            ..update(2)
        },
    );
    assert!(!is_error, "{text}");
    assert_eq!(text, "#2 [in_progress · next · P2] two (slice work)");

    let (text, is_error) = call(
        "slice-work",
        UpdateTask {
            status: Some(TaskStatus::Done),
            ..update(2)
        },
    );
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );

    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            title: Some("gone".into()),
            ..update(99)
        },
    );
    assert!(is_error && text.contains("no task #99"), "{text}");
    let (text, is_error) = call(COORDINATOR, update(1));
    assert!(is_error && text.contains("at least one field"), "{text}");
    h.quit();
}

#[test]
fn project_add_with_an_unknown_profile_raises_the_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::start(fixture("plain_turn"), false, profiles_config(dir.path()));
    assert_eq!(profile_errors(&h.snap, "\"ghost\""), 0);
    // `:project add` names it `work`, whose recipe picks a profile config.toml lacks.
    write_recipe_for(&h.paths, "work", "profile = \"ghost\"\n");
    let path = h.repo.clone();
    h.send(Command::Project(ProjectCommand::Register { path }));
    h.wait_for("the profile error", |s| profile_errors(s, "\"ghost\"") > 0);
    h.drain(Duration::from_millis(200));
    assert_eq!(profile_errors(&h.snap, "\"ghost\""), 1);
    // The recipe was there already: never overwritten by the starter (task #31).
    let text = std::fs::read_to_string(h.paths.recipe("work")).expect("recipe");
    assert_eq!(text, "profile = \"ghost\"\n");
    h.quit();
}

#[test]
fn project_add_writes_a_starter_recipe_and_a_picked_profile_lands_in_it() {
    use dovetail_core::recipe::Recipe;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::start(fixture("plain_turn"), false, profiles_config(dir.path()));
    let path = h.repo.clone();
    h.send(Command::Project(ProjectCommand::Register { path }));
    h.wait_for("registration", |s| {
        s.warnings.iter().all(|w| w.id.0 != "unregistered")
    });
    let file = h.paths.recipe("work");
    let recipe = Recipe::load(&file).expect("the starter parses");
    assert_eq!(recipe.name.as_deref(), Some("work"));
    assert_eq!(recipe.profile, None, "the profile starts commented out");

    h.send(Command::Project(ProjectCommand::SetRecipeProfile {
        project: dovetail_api::project::ProjectId("work".into()),
        profile: "work".into(),
    }));
    h.drain(Duration::from_millis(300));
    assert!(h.failures.is_empty(), "{:?}", h.failures);
    let recipe = Recipe::load(&file).expect("still parses");
    assert_eq!(recipe.profile.as_deref(), Some("work"));

    // A profile config.toml lacks is refused, and the file is left alone.
    h.send(Command::Project(ProjectCommand::SetRecipeProfile {
        project: dovetail_api::project::ProjectId("work".into()),
        profile: "ghost".into(),
    }));
    h.drain(Duration::from_millis(300));
    assert_eq!(h.failures.len(), 1, "{:?}", h.failures);
    assert_eq!(
        Recipe::load(&file).expect("parses").profile.as_deref(),
        Some("work")
    );
    h.quit();
}

#[test]
fn sessions_keep_their_launch_prefix_after_project_add() {
    // ADR 0022: a coordinator launched before `:project add` runs as `new-<pid>.coordinator`
    // until the next start, so slices launched after it must name it so too.
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), false, Config::default());
    let path = h.repo.clone();
    h.send(Command::Project(ProjectCommand::Register { path }));
    h.wait_for("registration", |s| {
        s.warnings.iter().all(|w| w.id.0 != "unregistered")
    });
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "api".into(),
        brief: "build the endpoint".into(),
    }));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    let prefix = format!("new-{}.", std::process::id());
    wait_sent_containing(
        &fake,
        &format!("The coordinator's session is named `{prefix}coordinator`"),
    );
    let names: Vec<String> = fake
        .started_specs()
        .iter()
        .filter_map(|s| s.name.clone())
        .collect();
    assert!(names.iter().all(|n| n.starts_with(&prefix)), "{names:?}");
    h.quit();
}

/// Commands are handled in order, so once a later command's effect shows, an earlier one that
/// was refused has reported its failure.
#[test]
fn an_effort_is_named_in_kebab_case_and_its_criterion_goes_claimed_then_met() {
    use dovetail_api::effort::{
        CriterionState, EffortChange, EffortKind, UNSORTED_EFFORT, WhereWasI,
    };

    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    h.wait_for("the first snapshot", |s| !s.efforts.is_empty());
    assert_eq!(h.snap.efforts[0].id, UNSORTED_EFFORT);

    let id = create_effort(&mut h, "Guest Checkout!", "guest-checkout");
    let effort = effort_named(&h.snap, "guest-checkout").expect("effort");
    assert_eq!(
        effort.question_or_goal,
        "A buyer can pay without an account"
    );

    // The same name typed another way is the same name: refused. So is renaming Unsorted.
    h.send(Command::Effort(EffortCommand::Create(
        dovetail_api::effort::NewEffort {
            name: "guest_checkout".into(),
            kind: EffortKind::Fix,
            question_or_goal: String::new(),
        },
    )));
    h.send(Command::Effort(EffortCommand::Rename {
        id: UNSORTED_EFFORT,
        name: "misc".into(),
    }));
    h.send(Command::Effort(EffortCommand::Rename {
        id,
        name: "Guest Pay".into(),
    }));
    h.wait_for("the rename", |s| effort_named(s, "guest-pay").is_some());
    assert_eq!(
        h.failures,
        vec![
            "there is already an effort named guest-checkout".to_string(),
            "Unsorted cannot be renamed".to_string(),
        ]
    );
    assert_eq!(h.snap.efforts.len(), 2);

    let card = WhereWasI {
        last_done: "API endpoint merged (PR #41)".into(),
        waits_on_you: "Approve merge of PR #42".into(),
        next_step: "Order email".into(),
    };
    h.send(Command::Effort(EffortCommand::SetWhereWasI {
        id,
        where_was_i: card.clone(),
    }));
    h.send(Command::Effort(EffortCommand::Update {
        id,
        patch: vec![EffortChange::Kind(EffortKind::Experiment)],
    }));
    h.send(Command::Effort(EffortCommand::AddCriterion {
        effort: id,
        text: "pay as guest end to end".into(),
    }));
    h.wait_for("the criterion", |s| {
        effort_named(s, "guest-pay").is_some_and(|e| !e.criteria.is_empty())
    });
    let effort = effort_named(&h.snap, "guest-pay").expect("effort").clone();
    assert_eq!(effort.where_was_i, card);
    assert_eq!(effort.kind, EffortKind::Experiment);
    let criterion = effort.criteria[0].id;
    assert_eq!(effort.criteria[0].state, CriterionState::Open);

    let state_of = move |s: &Snapshot| effort_named(s, "guest-pay").map(|e| e.criteria[0].state);
    h.send(Command::Effort(EffortCommand::ClaimCriterion {
        id: criterion,
        evidence: "PR #41".into(),
    }));
    h.wait_for("the claim", |s| {
        state_of(s) == Some(CriterionState::Claimed)
    });
    // A claim alone tells the coordinator nothing and ticks nothing.
    assert!(fake.sent().is_empty(), "{:?}", fake.sent());

    h.send(Command::Effort(EffortCommand::ReturnCriterion {
        id: criterion,
        reason: Some("the order email is missing".into()),
    }));
    h.wait_for("the return", |s| state_of(s) == Some(CriterionState::Open));
    wait_sent_containing(
        &fake,
        "sent criterion 1 of effort guest-pay back as not met (\"pay as guest end to end\"): \
         the order email is missing",
    );

    h.send(Command::Effort(EffortCommand::ClaimCriterion {
        id: criterion,
        evidence: "PR #44".into(),
    }));
    h.send(Command::Effort(EffortCommand::ApproveCriterion {
        id: criterion,
    }));
    h.wait_for("the approval", |s| state_of(s) == Some(CriterionState::Met));
    wait_sent_containing(&fake, "approved criterion 1 of effort guest-pay");
    let met = effort_named(&h.snap, "guest-pay").expect("effort").clone();
    assert_eq!(met.criteria[0].evidence.as_deref(), Some("PR #44"));

    // Reopen: efforts, the card and the criterion come back in the first snapshot.
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the efforts", |s| s.efforts.len() == 2);
    assert_eq!(effort_named(&h.snap, "guest-pay"), Some(&met));
    h.quit();
}

/// Decided 2026-11-16: an agent claims a criterion with `coordination_claim_criterion`; the
/// claim waits for the user, and no tool call can make a criterion met.
#[test]
fn an_agent_claims_a_criterion_and_only_the_user_approves_it() {
    use dovetail_api::effort::CriterionState;
    use dovetail_tools::coordination::ClaimCriterion;

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    h.send(Command::Effort(EffortCommand::AddCriterion {
        effort,
        text: "fees compared".into(),
    }));
    h.wait_for("the criterion", |s| {
        effort_named(s, "providers").is_some_and(|e| !e.criteria.is_empty())
    });
    let criterion = effort_named(&h.snap, "providers").expect("effort").criteria[0].id;
    let state_of = move |s: &Snapshot| effort_named(s, "providers").map(|e| e.criteria[0].state);
    let claim = |evidence: &str| {
        Tool::Coordination(CoordinationTool::ClaimCriterion(ClaimCriterion {
            criterion,
            evidence: evidence.into(),
        }))
    };
    let socket = h.socket();

    let (text, is_error) =
        result_text(socket_call(&socket, tool(COORDINATOR, claim("finding 41"))));
    assert!(!is_error, "{text}");
    assert!(text.contains("waits for the user's approval"), "{text}");
    h.wait_for("claimed", |s| state_of(s) == Some(CriterionState::Claimed));
    let evidence = effort_named(&h.snap, "providers").expect("effort").criteria[0]
        .evidence
        .clone();
    assert_eq!(evidence.as_deref(), Some("finding 41"));

    // Empty evidence and an unknown criterion are refused.
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, claim("  "))));
    assert!(is_error && text.contains("evidence"), "{text}");
    let unknown = Tool::Coordination(CoordinationTool::ClaimCriterion(ClaimCriterion {
        criterion: dovetail_api::effort::CriterionId(999),
        evidence: "x".into(),
    }));
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, unknown)));
    assert!(is_error, "{text}");

    // Only the user makes it met; a claim of a met criterion is refused and changes nothing.
    h.send(Command::Effort(EffortCommand::ApproveCriterion {
        id: criterion,
    }));
    h.wait_for("met", |s| state_of(s) == Some(CriterionState::Met));
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, claim("again"))));
    assert!(is_error && text.contains("already met"), "{text}");
    h.drain(Duration::from_millis(100));
    assert_eq!(state_of(&h.snap), Some(CriterionState::Met));
    h.quit();
}

#[test]
fn a_task_is_created_in_unsorted_and_moved_to_an_effort_and_its_criterion() {
    use dovetail_api::{effort::UNSORTED_EFFORT, task::NewTask};

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    h.send(Command::Effort(EffortCommand::AddCriterion {
        effort,
        text: "fees compared".into(),
    }));
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "Compare three providers".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: None,
        criterion: None,
    })));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let task = h.snap.tasks[0].clone();
    assert_eq!(task.effort, UNSORTED_EFFORT);
    let criterion = effort_named(&h.snap, "providers").expect("effort").criteria[0].id;

    // A criterion of another effort is refused; the right pair goes through.
    h.send(Command::Task(TaskCommand::Move {
        id: task.id,
        effort: UNSORTED_EFFORT,
        criterion: Some(criterion),
    }));
    h.send(Command::Task(TaskCommand::Move {
        id: task.id,
        effort,
        criterion: Some(criterion),
    }));
    h.wait_for("the move", |s| s.tasks[0].effort == effort);
    assert_eq!(h.snap.tasks[0].criterion, Some(criterion));
    assert_eq!(
        h.failures,
        vec!["criterion 1 belongs to effort 2, not effort 1".to_string()]
    );
    assert_eq!(h.snap.tasks[0].title, task.title);
    h.quit();
}

#[test]
fn a_tagged_message_tags_the_reply_and_the_tag_survives_a_restart() {
    use dovetail_api::effort::EffortId;

    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");

    // An effort this project does not have: nothing is sent.
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "lost".into(),
        attachments: Vec::new(),
        effort: Some(EffortId(99)),
    }));
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "Reply with exactly the word: alpha".into(),
        attachments: Vec::new(),
        effort: Some(effort),
    }));
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    assert_eq!(h.failures, vec!["no effort 99".to_string()]);
    assert_eq!(
        fake.sent(),
        vec![Sent::User("Reply with exactly the word: alpha".into())]
    );
    let tags = |s: &Snapshot| -> Vec<(EntryKind, Option<EffortId>)> {
        s.conversations[&AgentId(COORDINATOR.into())]
            .entries
            .iter()
            .map(|e| (e.kind, e.effort))
            .collect()
    };
    let before = tags(&h.snap);
    assert_eq!(before[0], (EntryKind::User, Some(effort)));
    assert!(before.contains(&(EntryKind::Assistant, Some(effort))));
    // Dovetail's own notices carry no tag.
    assert!(
        before
            .iter()
            .all(|(kind, tag)| *kind != EntryKind::Notice || tag.is_none())
    );

    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the conversation", |s| {
        s.conversations.contains_key(&AgentId(COORDINATOR.into()))
    });
    assert_eq!(tags(&h.snap), before);

    // An untagged message after the restart is untagged; the thread's tag does not leak onto it.
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the untagged message", |s| {
        tags(s).contains(&(EntryKind::User, None))
    });
    h.quit();
}

#[test]
fn a_task_follows_its_slices_pull_request_to_review_done_and_back() {
    let (dir, repo, paths) = repo_with_task_slices(&["api"], false);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    // No pull request: nothing moves.
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("the first lookup", |s| {
        s.worktrees
            .iter()
            .any(|w| w.pr == dovetail_api::worktree::PrLookup::None)
    });
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    // Open: review.
    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("review", |s| task_status(s, 1) == Some(TaskStatus::Review));

    // Closed without merging: back to in progress.
    set_pr(dir.path(), "api", "CLOSED");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("back in progress", |s| {
        task_status(s, 1) == Some(TaskStatus::InProgress)
    });

    // Reopened, then merged: review, then done.
    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("review again", |s| {
        task_status(s, 1) == Some(TaskStatus::Review)
    });
    set_pr(dir.path(), "api", "MERGED");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("done", |s| task_status(s, 1) == Some(TaskStatus::Done));

    // The user's own word is never overruled: marked to do, a merged PR does not move it.
    h.send(Command::Task(TaskCommand::Update {
        id: TaskId(1),
        patch: vec![dovetail_api::task::TaskChange::Status(TaskStatus::Todo)],
    }));
    h.wait_for("to do", |s| task_status(s, 1) == Some(TaskStatus::Todo));
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.drain(Duration::from_millis(300));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::Todo));
    h.quit();
}

/// Decided 2026-11-16: a task the user sends back from review stays in progress until
/// something new happens: its pull request changes, or its slice reports again.
#[test]
fn a_task_sent_back_from_review_stays_in_progress_until_something_new_happens() {
    use dovetail_tools::coordination::ReportState;
    let (dir, repo, paths) = repo_with_task_slices(&["api"], false);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("review", |s| task_status(s, 1) == Some(TaskStatus::Review));

    // Sent back while the PR stays open: a new lookup of the same open PR moves nothing.
    h.send(Command::Task(TaskCommand::Update {
        id: TaskId(1),
        patch: vec![dovetail_api::task::TaskChange::Status(
            TaskStatus::InProgress,
        )],
    }));
    h.wait_for("sent back", |s| {
        task_status(s, 1) == Some(TaskStatus::InProgress)
    });
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.drain(Duration::from_millis(300));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    // The slice reports again: the hold ends, and done moves it to review.
    let (text, is_error) = result_text(socket_call(
        &h.socket(),
        tool("slice-api", report(ReportState::Done, "fixed", Some(42))),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("review again", |s| {
        task_status(s, 1) == Some(TaskStatus::Review)
    });

    // Sent back again; then the PR is merged, which is new: done.
    h.send(Command::Task(TaskCommand::Update {
        id: TaskId(1),
        patch: vec![dovetail_api::task::TaskChange::Status(
            TaskStatus::InProgress,
        )],
    }));
    h.wait_for("sent back again", |s| {
        task_status(s, 1) == Some(TaskStatus::InProgress)
    });
    set_pr(dir.path(), "api", "MERGED");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("done", |s| task_status(s, 1) == Some(TaskStatus::Done));
    h.quit();
}

#[test]
fn a_task_with_two_slices_follows_the_one_furthest_behind() {
    let (dir, repo, paths) = repo_with_task_slices(&["api", "web"], true);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slices", |s| {
        s.agents
            .iter()
            .filter(|a| a.kind == AgentKind::Slice)
            .count()
            == 2
    });
    let looked_up = |s: &Snapshot, state: dovetail_api::worktree::PrState| {
        s.worktrees
            .iter()
            .filter(|w| matches!(&w.pr, dovetail_api::worktree::PrLookup::Found(pr) if pr.state == state))
            .count()
    };

    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("one open PR seen", |s| {
        looked_up(s, dovetail_api::worktree::PrState::Open) == 1 && s.worktrees.len() == 2
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    set_pr(dir.path(), "web", "OPEN");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("review", |s| task_status(s, 1) == Some(TaskStatus::Review));

    set_pr(dir.path(), "api", "MERGED");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("one merged PR seen", |s| {
        looked_up(s, dovetail_api::worktree::PrState::Merged) == 1
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::Review));

    set_pr(dir.path(), "web", "MERGED");
    h.send(Command::Worktree(WorktreeCommand::Refresh));
    h.wait_for("done", |s| task_status(s, 1) == Some(TaskStatus::Done));
    h.quit();
}

/// The coordinator merges and archives in one turn, so no slice turn ends in between: the
/// lookup made as the slice is archived is what shows the merge.
#[test]
fn a_merge_is_seen_when_the_slice_is_archived() {
    let (dir, repo, paths) = repo_with_task_slices(&["api"], false);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    set_pr(dir.path(), "api", "MERGED");
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("api")),
    ));
    assert!(!is_error && text.starts_with(ARCHIVED), "{text}");
    h.wait_for("done, and the slice gone", |s| {
        task_status(s, 1) == Some(TaskStatus::Done) && !s.agents.iter().any(|a| a.name == "api")
    });
    h.quit();
}

#[test]
fn the_coordinator_attaches_every_repo_and_a_slice_names_its_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, repos) = registered_project(dir.path(), &["api", "web"]);
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    prompt(&h, "hello");
    h.wait_for("the reply", replied);
    let spec = fake.started_specs()[0].clone();
    assert_eq!(spec.cwd, paths.project_home("p"));
    assert_eq!(spec.add_dirs, repos);
    let args = spec.claude_args();
    let attached: Vec<&String> = args
        .iter()
        .zip(args.iter().skip(1))
        .filter(|(flag, _)| *flag == "--add-dir")
        .map(|(_, dir)| dir)
        .collect();
    assert_eq!(attached.len(), 2, "{args:?}");
    assert_eq!(h.snap.project.as_ref().map(|p| p.repos.len()), Some(2));

    // Two repos: a slice must name one, and one the project has.
    let slice = |repo: Option<&str>| {
        Command::Agent(AgentCommand::NewSlice {
            task: None,
            name: "ui".into(),
            brief: "go".into(),
            repo: repo.map(Into::into),
        })
    };
    h.send(slice(None));
    h.send(slice(Some("docs")));
    h.drain(Duration::from_millis(200));
    assert_eq!(h.failures.len(), 2, "{:?}", h.failures);
    assert!(h.failures[0].contains("several repos"), "{:?}", h.failures);
    assert!(h.failures[1].contains("no repo `docs`"), "{:?}", h.failures);

    h.send(slice(Some("web")));
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < 2 {
        assert!(Instant::now() < deadline, "the slice never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    let spec = fake.started_specs()[1].clone();
    assert_eq!(spec.cwd, repos[1]);
    assert_eq!(spec.claude.worktree.as_deref(), Some("ui"));
    assert!(spec.add_dirs.is_empty());
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == dovetail_tools::env::REPO && v == "web"),
        "{:?}",
        spec.env
    );
    h.quit();
}

/// ADR 0021 point 4: a repo added while the coordinator works is attached once its turn ends:
/// it stops, resumes with the new `--add-dir`, and is told.
#[test]
fn adding_a_repo_restarts_the_coordinator_with_it_attached_after_its_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, repos) = registered_project(dir.path(), &["api"]);
    let docs = committed_repo(dir.path(), "docs");
    // Held before its `result`: the coordinator is mid-turn when the repo is added.
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![4],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    prompt(&h, "hello");
    h.wait_for("the reply", replied);

    h.send(Command::Project(ProjectCommand::AddRepo {
        project: dovetail_api::project::ProjectId("p".into()),
        path: docs.clone(),
        name: None,
    }));
    h.wait_for("the project with two repos", |s| {
        s.project.as_ref().is_some_and(|p| p.repos.len() == 2)
            && s.projects
                .iter()
                .any(|p| p.id.0 == "p" && p.repos.len() == 2)
    });
    // Working: not stopped yet.
    h.drain(Duration::from_millis(100));
    assert!(!fake.sent().iter().any(|s| matches!(s, Sent::Shutdown(_))));
    assert_eq!(fake.started_specs().len(), 1);

    fake.resume();
    wait_sent_shutdown(&fake);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < 2 {
        assert!(
            Instant::now() < deadline,
            "the coordinator was not restarted"
        );
        h.drain(Duration::from_millis(20));
    }
    let again = fake.started_specs()[1].clone();
    assert!(again.resume, "the same session, resumed");
    assert_eq!(again.add_dirs, vec![repos[0].clone(), docs.clone()]);
    wait_sent_containing(&fake, "repo `docs` attached at");
    h.quit();
}

#[test]
fn a_repo_with_an_open_slice_cannot_be_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, _repos) = registered_project(dir.path(), &["api", "web"]);
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.send(Command::Agent(AgentCommand::NewSlice {
        task: None,
        name: "ui".into(),
        brief: "go".into(),
        repo: Some("web".into()),
    }));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "ui"));
    let project = h.snap.project.clone().expect("open project");
    let id = |name: &str| {
        project
            .repos
            .iter()
            .find(|r| r.name == name)
            .expect("repo")
            .id
    };
    h.send(Command::Project(ProjectCommand::RemoveRepo {
        project: project.id.clone(),
        repo: id("web"),
    }));
    h.drain(Duration::from_millis(200));
    assert!(
        h.failures.iter().any(|f| f.contains("open slices (ui)")),
        "{:?}",
        h.failures
    );
    h.send(Command::Project(ProjectCommand::RemoveRepo {
        project: project.id.clone(),
        repo: id("api"),
    }));
    h.wait_for("api removed", |s| {
        s.project
            .as_ref()
            .is_some_and(|p| p.repos.iter().map(|r| r.name.as_str()).eq(["web"]))
    });
    h.quit();
}

/// ADR 0021 point 8: a coordinator from before, filed under its repo's folder, starts a fresh
/// session in `home/` at the first open, seeded, and Needs you says how to reopen the old one.
#[test]
fn a_coordinator_recorded_in_its_repo_starts_fresh_in_home() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, repos) = registered_project(dir.path(), &["api"]);
    let claude = dir.path().join("claude");
    let old = dovetail_core::session::launch::transcript_path(
        &claude,
        &repos[0],
        &format!("{COORDINATOR}-session"),
    );
    seed(
        &paths,
        session_row(
            COORDINATOR,
            AgentKind::Coordinator,
            None,
            &claude,
            old.clone(),
            None,
        ),
    );
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "the coordinator never started");
        h.drain(Duration::from_millis(20));
    }
    let spec = fake.started_specs()[0].clone();
    assert!(!spec.resume, "a fresh session");
    assert_ne!(spec.session_id, format!("{COORDINATOR}-session"));
    assert_eq!(spec.cwd, paths.project_home("p"));
    wait_sent_containing(&fake, "Dovetail now runs the coordinator in its own folder");
    h.wait_for("the note in Needs you", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Error { text }
                if text.contains("moved to its own folder") && text.contains(&format!("{COORDINATOR}-session")))
        })
    });
    h.quit();
}

fn update(id: i64) -> UpdateTask {
    UpdateTask {
        id: TaskId(id),
        status: None,
        bucket: None,
        p_level: None,
        title: None,
        notes: None,
    }
}

fn write_recipe_for(paths: &Paths, project: &str, text: &str) {
    let file = paths.recipe(project);
    std::fs::create_dir_all(file.parent().expect("parent")).expect("projects dir");
    std::fs::write(file, text).expect("recipe");
}

/// A fake `gh`: `gh pr list --head <branch> ...` prints `<dir>/<branch with _ for />.json`, or
/// `[]` when there is no such file.
fn fake_gh(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("gh");
    let text = format!(
        "#!/bin/sh\nbranch=\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = --head ]; then branch=$2; fi\n  \
         shift\ndone\nfile=\"{}/$(echo \"$branch\" | tr / _).json\"\nif [ -f \"$file\" ]; then cat \
         \"$file\"; else echo '[]'; fi\n",
        dir.display()
    );
    std::fs::write(&script, text).expect("fake gh");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

fn set_pr(dir: &Path, slice: &str, state: &str) {
    let text = format!(
        "[{{\"number\": 42, \"state\": \"{state}\", \"url\": \"https://example.test/pr/42\"}}]"
    );
    std::fs::write(dir.join(format!("dovetail_{slice}.json")), text).expect("pr file");
}

/// Slices `names` in a git repo, each on its own task (ids from 1, in order), all in progress.
/// With `shared`, every slice works on task 1.
fn repo_with_task_slices(names: &[&str], shared: bool) -> (TempDir, PathBuf, Paths) {
    use dovetail_api::task::{NewTask, TaskChange};
    let (dir, repo, paths) = repo_with_slices(names);
    let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
    let mut rows = db.load_agents().wait().expect("agents");
    for (index, name) in names.iter().enumerate() {
        let task = if shared && index > 0 {
            TaskId(1)
        } else {
            let created = db
                .create_task(
                    NewTask {
                        title: format!("task for {name}"),
                        p_level: None,
                        bucket: None,
                        notes: String::new(),
                        follow_up_of: None,
                        pr: None,
                        related: Vec::new(),
                        effort: None,
                        criterion: None,
                    },
                    false,
                )
                .wait()
                .expect("task");
            let id = created[0].id;
            db.update_task(id, vec![TaskChange::Status(TaskStatus::InProgress)])
                .wait()
                .expect("status");
            id
        };
        let row = rows
            .iter_mut()
            .find(|r| r.agent.name == *name)
            .expect("slice row");
        row.agent.task = Some(task);
        db.upsert_agent(row.clone()).wait().expect("agent");
    }
    drop(db);
    (dir, repo, paths)
}

/// A git repo with one commit at `<dir>/<name>`, by its canonical path.
fn committed_repo(dir: &Path, name: &str) -> PathBuf {
    let repo = dir.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), name).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    repo.canonicalize().unwrap()
}

/// Project `p`, registered in the registry with a repo for each of `names`, and the paths
/// under `dir` it lives in. The next harness opens it.
fn registered_project(dir: &Path, names: &[&str]) -> (Paths, Vec<PathBuf>) {
    let paths = Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("run"),
    };
    let repos: Vec<PathBuf> = names.iter().map(|n| committed_repo(dir, n)).collect();
    let db = dovetail_core::db::Db::open(&paths, None).expect("registry");
    let mut project = db
        .create_project("p".into(), "work".into(), None, None)
        .wait()
        .expect("project");
    for repo in &repos {
        let new = dovetail_core::projects::repo_at(repo, None).expect("repo");
        project = db.add_repo("p".into(), new).wait().expect("add");
    }
    drop(db);
    PROJECT.with(|p| *p.borrow_mut() = Some(project));
    (paths, repos)
}

fn replied(s: &Snapshot) -> bool {
    s.conversations
        .get(&AgentId(COORDINATOR.into()))
        .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::Assistant))
}
