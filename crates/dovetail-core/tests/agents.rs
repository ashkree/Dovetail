//! Integration tests for the `agents` domain: slices, side chats, journal, native to-do lists,
//! attachments, models, and MCP menus.

mod common;

use common::*;

use std::path::Path;
use std::time::{Duration, Instant};

use dovetail_api::{
    Command, Snapshot,
    agent::{AgentId, PermissionMode},
    conversation::{Attachment, EntryKind, ImageType},
    inbox::{InboxKind, PermissionAnswer},
    task::{Bucket, NewTask, TaskId, TaskStatus},
};
use dovetail_api::{
    agent::AgentCommand, conversation::ConversationCommand, inbox::InboxCommand,
    journal::JournalCommand, mcp::McpCommand, model::ModelCommand, task::TaskCommand,
};
use dovetail_core::config::Config;
use dovetail_driver::{FakeDriver, ReplayOptions, Sent};
use dovetail_tools::project::ProjectTool;
use dovetail_tools::{Tool, project::StartSlice};

#[test]
fn a_prompt_with_an_image_sends_an_image_block_and_keeps_the_path_after_a_restart() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let dir = h.paths.data.join("attachments");
    std::fs::create_dir_all(&dir).expect("attachments dir");
    let path = dir.join("1.png");
    let bytes = b"\x89PNG\r\n\x1a\nfake";
    std::fs::write(&path, bytes).expect("write image");
    let attachment = Attachment {
        path: path.clone(),
        media_type: ImageType::Png,
        size: bytes.len() as u64,
    };
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "what is this?".into(),
        attachments: vec![attachment.clone()],
        effort: None,
    }));
    h.wait_for("the reply", |s| finished_turns(s) == 1);

    let lines = fake.user_messages();
    assert_eq!(lines.len(), 1);
    let content = lines[0]["message"]["content"]
        .as_array()
        .expect("content blocks");
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["type"], "base64");
    assert_eq!(content[0]["source"]["media_type"], "image/png");
    // base64 of the 12 bytes above.
    assert_eq!(content[0]["source"]["data"], "iVBORw0KGgpmYWtl");
    assert_eq!(content[1]["text"], "what is this?");

    let user = |h: &Harness| {
        h.snap.conversations[&AgentId(COORDINATOR.into())]
            .entries
            .iter()
            .find(|e| e.kind == EntryKind::User)
            .cloned()
            .expect("the user entry")
    };
    assert_eq!(user(&h).attachments, vec![attachment.clone()]);

    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(user(&h).attachments, vec![attachment]);
    h.quit();
}

#[test]
fn slices_cannot_call_coordinator_tools_and_bad_names_are_refused() {
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
    let socket = h.socket();
    let start = |name: &str| {
        Tool::Project(ProjectTool::StartSlice(StartSlice {
            repo: None,
            model: None,
            effort: None,
            driver: None,
            task: TaskId(1),
            name: name.into(),
            brief: "go".into(),
        }))
    };
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, start("agent-x"))));
    assert!(
        is_error && text.starts_with("denied") && text.contains("agent-"),
        "{text}"
    );

    // A folder without git: the slice shares it and is ready at once.
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, start("one"))));
    assert!(!is_error && text.starts_with("started"), "{text}");
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "one"));
    let (text, is_error) = result_text(socket_call(&socket, tool("slice-one", start("two"))));
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );
    h.wait_for("the shared-folder brief", |s| {
        s.conversations
            .get(&AgentId("slice-one".into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.text.contains("shared project folder"))
            })
    });
    h.quit();
}

#[test]
fn slices_over_the_cap_queue_until_one_is_archived() {
    let config = Config {
        max_slices: 1,
        ..Config::default()
    };
    let mut h = Harness::start(fixture("plain_turn"), true, config);
    for name in ["first", "second"] {
        h.send(Command::Agent(AgentCommand::NewSlice {
            repo: None,
            task: None,
            name: name.into(),
            brief: "go".into(),
        }));
    }
    h.wait_for("the queue", |s| {
        s.activity.iter().any(|a| a.text.contains("second queued"))
    });
    assert!(!h.snap.agents.iter().any(|a| a.name == "second"));
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "third".into(),
        brief: "go".into(),
    }));
    h.wait_for("the second queued start", |s| {
        s.activity.iter().any(|a| a.text.contains("third queued"))
    });
    h.send(Command::Agent(AgentCommand::CloseSlice {
        agent: AgentId("slice-first".into()),
    }));
    h.wait_for("the queued slice", |s| {
        s.agents.iter().any(|a| a.name == "second") && !s.agents.iter().any(|a| a.name == "first")
    });

    // The queue survives a restart: `third` starts as soon as there is room (cap 3 here).
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the slice queued before the restart", |s| {
        s.agents.iter().any(|a| a.name == "third")
    });
    h.quit();
}

#[test]
fn start_slice_outside_auto_mode_returns_pending_and_the_answer_comes_as_a_message() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    // auto → acceptEdits: approval is needed now.
    h.send(Command::Agent(AgentCommand::CyclePermissionMode {
        agent: coordinator.clone(),
    }));
    h.wait_for("the mode", |s| {
        s.agents
            .iter()
            .any(|a| a.permission_mode == PermissionMode::AcceptEdits)
    });
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::Project(ProjectTool::StartSlice(StartSlice {
                repo: None,
                model: None,
                effort: None,
                driver: None,
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            })),
        ),
    ));
    // No task #1 yet: refused before anyone is asked.
    assert!(is_error && text.starts_with("denied"), "{text}");
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
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::Project(ProjectTool::StartSlice(StartSlice {
                repo: None,
                model: None,
                effort: None,
                driver: None,
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            })),
        ),
    ));
    assert!(!is_error && text.starts_with("pending"), "{text}");
    h.wait_for("the approval item", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == dovetail_api::inbox::START_SLICE_PERMISSION))
    });
    let id = h.snap.inbox[0].id;
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: id,
        answer: PermissionAnswer::Allow,
    }));
    h.wait_for("the approval message", |s| {
        s.conversations.get(&coordinator).is_some_and(|t| {
            t.entries
                .iter()
                .any(|e| e.text.contains("project_start_slice later approved"))
        })
    });
    assert!(h.snap.agents.iter().any(|a| a.name == "later"));
    h.quit();
}

#[test]
fn a_start_slice_approval_pending_at_quit_is_answered_after_reopen() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    pending_start(&mut h);
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sent_users(&fake).contains(&EXPIRED.to_string()) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
    h.drain(Duration::from_millis(100));
    assert!(
        !h.snap
            .activity
            .iter()
            .any(|a| a.text.contains("request expired")),
        "Dovetail's own approval is not logged as an expired request"
    );
    h.quit();
}

#[test]
fn todo_items_attach_to_the_agent_that_created_them_and_survive_a_restart() {
    let fake = fixture("todo_tools");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    // What Claude Code leaves on disk after the recorded turn, plus an item nobody created in
    // the stream (an external edit).
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "completed");
    write_item(&list, "2", "beta", "pending");
    write_item(&list, "3", "outside", "pending");
    prompt(&h, "make two to-dos");
    h.wait_for("both items attributed", |s| {
        owner(s, "1").as_deref() == Some(COORDINATOR)
            && owner(s, "2").as_deref() == Some(COORDINATOR)
    });
    assert_eq!(
        todo(&h.snap, "1").map(|t| t.status),
        Some(dovetail_api::agent::TodoStatus::Completed)
    );
    // No TaskCreate in the stream: it falls back to the session whose list it is.
    assert_eq!(owner(&h.snap, "3").as_deref(), Some(COORDINATOR));
    assert!(
        fake.started_specs()[0]
            .env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CODE_ENABLE_TODO_TOOLS" && v == "1")
    );

    // An edit outside the stream reaches the UI through the watcher.
    write_item(&list, "2", "beta", "in_progress");
    h.wait_for("the external edit", |s| {
        todo(s, "2").map(|t| t.status) == Some(dovetail_api::agent::TodoStatus::InProgress)
    });
    std::fs::remove_file(list.join("3.json")).expect("delete");
    h.wait_for("the deleted item gone", |s| todo(s, "3").is_none());

    // The owners are in state.db: a reopen shows the items where they were.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(owner(&h.snap, "1").as_deref(), Some(COORDINATOR));
    assert_eq!(owner(&h.snap, "2").as_deref(), Some(COORDINATOR));
    h.quit();
}

#[test]
fn a_foreground_subagents_todo_is_its_own() {
    // The recorded turn, with the second TaskCreate (call and result) made inside a subagent.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures/todo_tools.ndjson");
    let text = std::fs::read_to_string(path).expect("fixture");
    let text: String = text
        .lines()
        .map(|line| {
            let line = if line.contains("toolu_01PDiMXF9E6gftnQXY5xYUhf") {
                line.replace(
                    "\"parent_tool_use_id\":null",
                    "\"parent_tool_use_id\":\"toolu_sub\"",
                )
            } else {
                line.to_owned()
            };
            line + "\n"
        })
        .collect();
    let fake = FakeDriver::from_ndjson(&text).expect("fixture");
    let mut h = Harness::start(fake, true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "completed");
    write_item(&list, "2", "beta", "pending");
    prompt(&h, "make two to-dos");
    h.wait_for("the subagent's item", |s| {
        owner(s, "2").as_deref() == Some("coordinator:toolu_sub")
    });
    assert_eq!(owner(&h.snap, "1").as_deref(), Some(COORDINATOR));
    // The stream's attribution is in state.db, not just the list owner's fallback.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(
        owner(&h.snap, "2").as_deref(),
        Some("coordinator:toolu_sub")
    );
    h.quit();
}

#[test]
fn a_pruned_subagents_todo_falls_back_to_the_list_owner() {
    // As above, then the subagent ends, and a second one runs and ends: only the newest finished
    // subagent keeps its node, so the first one's item must not point at a gone agent.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures/todo_tools.ndjson");
    let text = std::fs::read_to_string(path).expect("fixture");
    let session = "f0e55c68-6629-4132-b963-ccb23955bc1a";
    let result = |tool: &str| {
        format!(
            "{{\"dir\":\"out\",\"msg\":{{\"message\":{{\"content\":[{{\"content\":\"ok\",\
             \"tool_use_id\":\"{tool}\",\"type\":\"tool_result\"}}],\"role\":\"user\"}},\
             \"parent_tool_use_id\":null,\"session_id\":\"{session}\",\"type\":\"user\"}},\"t\":6000}}\n"
        )
    };
    let second = format!(
        "{{\"dir\":\"out\",\"msg\":{{\"message\":{{\"content\":[{{\"text\":\"hi\",\"type\":\"text\"}}],\
         \"id\":\"msg_sub2\",\"model\":\"claude-sonnet-5-5\",\"role\":\"assistant\",\"type\":\"message\"}},\
         \"parent_tool_use_id\":\"toolu_sub2\",\"session_id\":\"{session}\",\"type\":\"assistant\"}},\"t\":6000}}\n"
    );
    let mut out = String::new();
    for line in text.lines() {
        if line.contains("\"text\":\"done\"") {
            out += &result("toolu_sub");
            out += &second;
            out += &result("toolu_sub2");
        }
        let line = if line.contains("toolu_01PDiMXF9E6gftnQXY5xYUhf") {
            line.replace(
                "\"parent_tool_use_id\":null",
                "\"parent_tool_use_id\":\"toolu_sub\"",
            )
        } else {
            line.to_owned()
        };
        out += &line;
        out.push('\n');
    }
    let fake = FakeDriver::from_ndjson(&out).expect("fixture");
    let mut h = Harness::start(fake, true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "completed");
    write_item(&list, "2", "beta", "pending");
    prompt(&h, "make two to-dos");
    h.wait_for("the first subagent pruned", |s| {
        !s.agents.iter().any(|a| a.id.0 == "coordinator:toolu_sub")
            && s.agents.iter().any(|a| a.id.0 == "coordinator:toolu_sub2")
    });
    h.wait_for("its item back under the coordinator", |s| {
        owner(s, "2").as_deref() == Some(COORDINATOR)
    });
    // A reopen agrees: the recorded creator is gone, so the list owner shows it.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(owner(&h.snap, "2").as_deref(), Some(COORDINATOR));
    h.quit();
}

#[test]
fn items_with_no_known_creator_sit_under_the_session_whose_list_it_is() {
    // No TaskCreate in the stream at all: the items were put there some other way.
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "by hand", "pending");
    prompt(&h, "hello");
    h.wait_for("the item under the coordinator", |s| {
        owner(s, "1").as_deref() == Some(COORDINATOR)
    });
    h.quit();
}

/// M11: a task file caught mid-write (truncated, empty) keeps its last good version on screen,
/// the rest of the list still updates, and the file is read again once it is whole.
#[test]
fn a_todo_file_caught_mid_write_keeps_its_last_version_until_it_is_whole() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "pending");
    write_item(&list, "2", "beta", "pending");
    prompt(&h, "hello");
    h.wait_for("both items", |s| {
        todo(s, "1").is_some() && todo(s, "2").is_some()
    });

    // Item 2 is rewritten and read half way; item 3 arrives whole in the same window.
    std::fs::write(list.join("2.json"), "{\"id\": \"2\", \"subject\": \"be").expect("truncate");
    write_item(&list, "3", "gamma", "pending");
    h.wait_for("the whole item", |s| todo(s, "3").is_some());
    let kept = todo(&h.snap, "2").expect("the last good version stays");
    assert_eq!(kept.subject, "beta");
    assert_eq!(kept.status, dovetail_api::agent::TodoStatus::Pending);

    // An empty file is the same state (a writer that truncates first).
    std::fs::write(list.join("1.json"), "").expect("empty");
    write_item(&list, "4", "delta", "pending");
    h.wait_for("the next item", |s| todo(s, "4").is_some());
    assert_eq!(todo(&h.snap, "1").map(|t| t.subject), Some("alpha".into()));

    // Once the writer finishes, the new content shows.
    write_item(&list, "2", "beta", "in_progress");
    h.wait_for("the finished write", |s| {
        todo(s, "2").map(|t| t.status) == Some(dovetail_api::agent::TodoStatus::InProgress)
    });
    h.quit();
}

#[test]
fn archiving_a_task_deletes_its_todo_list() {
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
    let list = tasks_dir(&h.paths).join(format!("dovetail-p-task-{}", id.0));
    write_item(&list, "1", "alpha", "pending");
    // The coordinator's list is not the task's and stays.
    let other = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&other, "1", "keep", "pending");
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    }));
    h.wait_for("the slice's list read", |s| {
        s.todos.iter().any(|t| t.subject == "alpha")
    });
    h.send(Command::Task(TaskCommand::Archive { id }));
    h.wait_for("the list gone", |s| {
        !s.todos.iter().any(|t| t.subject == "alpha")
    });
    assert!(!list.exists());
    assert!(other.join("1.json").exists());
    h.quit();
}

#[test]
fn eod_asks_the_coordinator_and_writes_its_reply_and_the_facts() {
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"first"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"one","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"eod"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Open: PR 20 waits on review."}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Open: PR 20 waits on review.","total_cost_usd":0.02}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![4],
            ..ReplayOptions::default()
        });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    assert_eq!(journal(&h.snap).latest, None);
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "ship the journal".into(),
        p_level: None,
        bucket: Some(Bucket::Now),
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: Default::default(),
        criterion: None,
    })));
    prompt(&h, "first");
    h.wait_for("the first reply", |s| finished_turns(s) == 1);

    h.send(Command::Journal(JournalCommand::EndOfDay));
    h.wait_for("the handoff request", |s| {
        journal(s).closing
            && s.conversations[&AgentId(COORDINATOR.into())]
                .entries
                .iter()
                .any(|e| e.kind == EntryKind::Notice && e.text.contains("closing the day"))
    });
    fake.resume();
    let today = local_date(0);
    h.wait_for("the journal", |s| {
        let j = journal(s);
        !j.closing && j.latest.as_deref() == Some(today.as_str())
    });
    let path = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{today}.md"));
    let text = std::fs::read_to_string(&path).expect("the journal file");
    assert!(text.starts_with(&format!("# {today}\n")), "{text}");
    assert!(
        text.contains("## Handoff\n\nOpen: PR 20 waits on review.\n"),
        "{text}"
    );
    assert!(text.contains("## Facts"), "{text}");
    assert!(text.contains("**Now**\n\n- #1 ship the journal"), "{text}");
    h.quit();
}

#[test]
fn fresh_appends_the_handoff_and_seeds_a_new_coordinator_with_no_item() {
    let fake = fresh_fixture(4);
    let mut h = Harness::start(fake.clone(), true, Config::default());
    // A journal file for today already exists: `:fresh` adds to it.
    let dir = h.paths.data.join("projects/p/journal");
    std::fs::create_dir_all(&dir).expect("journal dir");
    let today = local_date(0);
    let path = dir.join(format!("{today}.md"));
    std::fs::write(&path, "Hand-written notes\n").expect("a day's file");
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "ship the journal".into(),
        p_level: None,
        bucket: Some(Bucket::Now),
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: Default::default(),
        criterion: None,
    })));
    prompt(&h, "first");
    h.wait_for("the first reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();

    h.send(Command::Journal(JournalCommand::Handoff));
    h.wait_for("the handoff request", |s| journal(s).closing);
    fake.resume();
    wait_started(&fake, 2);
    h.wait_for("the journal block", |s| !journal(s).closing);

    // The day's file was added to, not replaced.
    let text = std::fs::read_to_string(&path).expect("the journal file");
    assert!(text.starts_with("Hand-written notes\n\n---\n\n"), "{text}");
    assert!(text.contains("## Handoff (fresh start "), "{text}");
    assert!(text.contains("Open: PR 20 waits on review."), "{text}");
    assert!(text.contains("**Now**\n\n- #1 ship the journal"), "{text}");

    // A new session (not `--resume`), seeded with the handoff and the open tasks.
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 2);
    assert!(!specs[1].resume);
    assert_ne!(specs[1].session_id, old);
    let seed = fake
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::User(text) => Some(text),
            _ => None,
        })
        .find(|t| t.contains("This is a fresh coordinator session, started at"))
        .expect("the seed prompt");
    assert!(seed.contains("Open: PR 20 waits on review."), "{seed}");
    assert!(seed.contains("# Open tasks"), "{seed}");
    assert!(seed.contains("#1 ship the journal"), "{seed}");
    assert!(no_permission_item(&h.snap));
    h.quit();
}

#[test]
fn fresh_session_starts_a_new_coordinator_with_no_seed_and_no_item() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "first");
    h.wait_for("the first reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();
    let users_before = fake
        .sent()
        .into_iter()
        .filter(|s| matches!(s, Sent::User(_)))
        .count();

    h.send(Command::Journal(JournalCommand::FreshSession));
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < 2 {
        assert!(Instant::now() < deadline, "the new session never started");
        std::thread::sleep(Duration::from_millis(10));
    }

    // A new session, no handoff written, and no first message.
    let specs = fake.started_specs();
    assert!(!specs[1].resume);
    assert_ne!(specs[1].session_id, old);
    let users_after = fake
        .sent()
        .into_iter()
        .filter(|s| matches!(s, Sent::User(_)))
        .count();
    assert_eq!(
        users_before, users_after,
        "nothing was sent to the new session"
    );
    assert!(!journal(&h.snap).closing);
    let file = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{}.md", local_date(0)));
    assert!(!file.exists(), "nothing is written to the journal");
    assert!(no_permission_item(&h.snap));
    h.quit();
}

#[test]
fn fresh_is_refused_while_a_turn_runs() {
    // Held before the `result`: the first turn is still running.
    let fake = fresh_fixture(3);
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "first");
    h.wait_for("the reply text", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|c| {
                c.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Assistant && e.text == "one")
            })
    });
    h.send(Command::Journal(JournalCommand::Handoff));
    h.drain(Duration::from_millis(300));
    assert_eq!(
        h.failures,
        vec!["turn running; try again when it finishes".to_owned()]
    );
    assert!(!journal(&h.snap).closing);
    assert_eq!(fake.started_specs().len(), 1);
    let file = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{}.md", local_date(0)));
    assert!(!file.exists());
    fake.resume();
    h.quit();
}

#[test]
fn eod_with_a_coordinator_that_never_ran_writes_the_facts_alone() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::Journal(JournalCommand::EndOfDay));
    let today = local_date(0);
    h.wait_for("the journal", |s| {
        journal(s).latest.as_deref() == Some(today.as_str())
    });
    let path = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{today}.md"));
    let text = std::fs::read_to_string(&path).expect("the journal file");
    assert!(
        text.contains("_No handoff from the coordinator: the coordinator has not run yet._"),
        "{text}"
    );
    assert!(h.fake.started_specs().is_empty(), "no session was started");
    h.quit();
}

#[test]
fn an_unregistered_folder_has_no_journal() {
    let mut h = Harness::start(fixture("plain_turn"), false, Config::default());
    assert_eq!(h.snap.journal, None);
    h.send(Command::Journal(JournalCommand::EndOfDay));
    h.drain(Duration::from_millis(200));
    assert_eq!(h.snap.journal, None);
    h.quit();
}

#[test]
fn the_first_open_of_a_new_day_starts_a_fresh_coordinator_with_no_item() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();
    let paths = h.paths.clone();
    let _state = h.quit();

    // Yesterday: the conversation happened then, and the coordinator wrote its handoff by hand.
    let db = rusqlite::Connection::open(paths.state_db("p")).expect("state db");
    db.execute(
        "UPDATE conversation_entries SET at_ms = at_ms - 86400000 * 2",
        [],
    )
    .expect("backdate");
    drop(db);
    let dir = paths.data.join("projects/p/journal");
    std::fs::create_dir_all(&dir).expect("journal dir");
    let yesterday = local_date(1);
    std::fs::write(
        dir.join(format!("{yesterday}.md")),
        "# Handoff\n\nFirst thing: merge PR 20.\n",
    )
    .expect("handoff");

    let fake = fixture("plain_turn");
    let h = Harness::start_on(fake.clone(), paths.clone());
    wait_started(&fake, 1);
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 1);
    assert!(!specs[0].resume, "a new session, not --resume");
    assert_ne!(specs[0].session_id, old);
    let seed = fake
        .sent()
        .into_iter()
        .find_map(|s| match s {
            Sent::User(text) => Some(text),
            _ => None,
        })
        .expect("the seed prompt");
    assert!(seed.contains("First thing: merge PR 20."), "{seed}");
    assert!(seed.contains("# Open tasks"), "{seed}");

    assert!(no_permission_item(&h.snap));
    h.quit();

    // The day is not new any more: the next open resumes as before, with no item.
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    assert!(no_permission_item(&h.snap));
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 1);
    assert!(specs[0].resume);
    h.quit();
}

/// Go-back items stored by builds before task #38 are cleared at the next open, quietly, with
/// their own Activity line (not the generic "request expired").
#[test]
fn legacy_go_back_items_in_an_old_state_db_expire_quietly() {
    let h = Harness::start(fixture("plain_turn"), true, Config::default());
    let paths = h.paths.clone();
    let _state = h.quit();

    let db = rusqlite::Connection::open(paths.state_db("p")).expect("state db");
    let legacy = [
        ("Keep yesterday's coordinator", Some("/old/yesterday.jsonl")),
        ("Fresh coordinator started", None),
    ];
    for (n, (tool, transcript)) in legacy.iter().enumerate() {
        let payload = serde_json::to_string(&InboxKind::Permission {
            tool: (*tool).into(),
            input: serde_json::json!({
                "journal": "2026-11-10",
                "session_id": "old-session",
                "transcript_path": transcript,
            }),
            reason: None,
            suggestions: Vec::new(),
        })
        .expect("payload");
        db.execute(
            "INSERT INTO inbox_items (id, agent_id, kind, payload, blocking, created_ms) \
             VALUES (?1, 'coordinator', 'permission', ?2, 1, 1)",
            rusqlite::params![9_000 + n as i64, payload],
        )
        .expect("legacy item");
    }
    drop(db);

    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("both legacy items cleared", |s| {
        s.activity
            .iter()
            .filter(|a| {
                a.text
                    .contains("an old 'go back to the previous coordinator' offer")
            })
            .count()
            == 2
    });
    let texts: Vec<&str> = h.snap.activity.iter().map(|a| a.text.as_str()).collect();
    assert!(
        texts.contains(
            &"an old 'go back to the previous coordinator' offer was cleared; its transcript \
              is kept at /old/yesterday.jsonl"
        ),
        "{texts:?}"
    );
    assert!(
        texts.contains(
            &"an old 'go back to the previous coordinator' offer was cleared; its transcript \
              is kept"
        ),
        "{texts:?}"
    );
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("request expired at restart")),
        "{texts:?}"
    );
    assert!(
        h.snap
            .inbox
            .iter()
            .all(|i| !matches!(i.kind, InboxKind::Permission { .. }))
    );
    h.quit();
}

#[test]
fn a_day_that_was_never_closed_keeps_its_coordinator_and_says_so() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();
    let paths = h.paths.clone();
    let _state = h.quit();

    // The coordinator was used two days ago and never closed; the newest journal is older.
    let db = rusqlite::Connection::open(paths.state_db("p")).expect("state db");
    db.execute(
        "UPDATE conversation_entries SET at_ms = at_ms - 86400000 * 2",
        [],
    )
    .expect("backdate");
    drop(db);
    let dir = paths.data.join("projects/p/journal");
    std::fs::create_dir_all(&dir).expect("journal dir");
    let file = dir.join(format!("{}.md", local_date(4)));
    std::fs::write(&file, "# Handoff\n\nThe day before.\n").expect("old handoff");
    std::fs::File::options()
        .write(true)
        .open(&file)
        .and_then(|f| {
            f.set_modified(std::time::SystemTime::now() - Duration::from_secs(4 * 86_400))
        })
        .expect("backdate the file");
    let used_on = jiff::Zoned::now()
        .checked_sub(jiff::Span::new().days(2))
        .expect("date")
        .strftime("%A")
        .to_string();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    let unclosed = format!("{used_on} wasn't closed; kept that coordinator. `:eod` to close it.");
    h.wait_for("the unclosed-day item", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(&i.kind, InboxKind::Error { text } if *text == unclosed))
    });
    // The same session resumes, with no fresh seed and no item.
    wait_started(&fake, 1);
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 1);
    assert!(specs[0].resume);
    assert_eq!(specs[0].session_id, old);
    assert!(no_permission_item(&h.snap));
    h.quit();
}

#[test]
fn a_side_chat_gets_only_the_core_suite_whatever_the_config_says() {
    let fake = fixture("plain_turn");
    let mut config = Config::default();
    config.suites.side_chat = dovetail_tools::Suite::ALL.to_vec();
    let mut h = Harness::start(fake.clone(), true, config);
    h.send(Command::Agent(AgentCommand::NewSideChat {
        question: "what does the reducer do?".into(),
    }));
    h.wait_for("the side chat", |s| {
        s.agents.iter().any(|a| a.name == "side-1")
    });
    wait_started(&fake, 1);
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| s.name.as_deref().is_some_and(|n| n.ends_with(".side-1")))
        .expect("side chat spec");
    let mcp =
        std::fs::read_to_string(spec.claude.mcp_config_path.as_ref().expect("mcp")).expect("read");
    assert!(mcp.contains(r#""DOVETAIL_SUITES": "core""#), "{mcp}");
    // The config's list is reported once, as a warning.
    assert!(
        h.snap
            .warnings
            .iter()
            .any(|w| w.id.0 == "config-suites" && w.text.contains("ignored")),
        "{:?}",
        h.snap.warnings
    );
    h.quit();
}

#[test]
fn a_side_chat_runs_read_only_in_dont_ask_mode_and_keeps_it() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    h.send(Command::Agent(AgentCommand::NewSideChat {
        question: "what does the reducer do?".into(),
    }));
    h.wait_for("the side chat", |s| {
        s.agents.iter().any(|a| a.name == "side-1")
    });
    wait_started(&fake, 1);
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| s.name.as_deref().is_some_and(|n| n.ends_with(".side-1")))
        .expect("side chat spec");
    let args = spec.claude_args();
    // Anything not allowed is denied without asking (decided 2026-11-11).
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "dontAsk"),
        "{args:?}"
    );
    let allowed = args
        .windows(2)
        .find(|w| w[0] == "--allowedTools")
        .map(|w| w[1].clone())
        .expect("--allowedTools");
    for rule in [
        "Bash(git log *)",
        "Bash(rg *)",
        "Bash(cargo tree --locked *)",
        "WebFetch",
    ] {
        assert!(allowed.contains(rule), "{rule} in {allowed}");
    }
    assert!(!allowed.split(',').any(|r| r == "Bash"), "{allowed}");
    let denied = args
        .windows(2)
        .find(|w| w[0] == "--disallowedTools")
        .map(|w| w[1].clone())
        .expect("--disallowedTools");
    for rule in ["Edit", "Write", "NotebookEdit", "Bash(find *-exec*)"] {
        assert!(denied.contains(rule), "{rule} in {denied}");
    }

    // Its mode never cycles: auto would let the classifier approve other commands.
    h.send(Command::Agent(AgentCommand::CyclePermissionMode {
        agent: AgentId("side-1".into()),
    }));
    h.drain(Duration::from_millis(300));
    assert!(
        h.failures.iter().any(|f| f.contains("read-only")),
        "{:?}",
        h.failures
    );
    let side = h
        .snap
        .agents
        .iter()
        .find(|a| a.name == "side-1")
        .expect("side-1");
    assert_eq!(side.permission_mode, PermissionMode::Manual);
    h.quit();
}

/// The MCP menu (task #44): a request goes to the live session through the driver and its
/// answer comes back as one `McpReply` carrying the request; a session that is not running
/// refuses.
#[test]
fn an_mcp_request_reaches_the_session_and_its_answer_comes_back() {
    use dovetail_api::mcp::{McpOutcome, McpRequest, McpServer, McpStatus};
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    fake.set_mcp_servers(vec![McpServer {
        name: "good".into(),
        status: McpStatus::Connected,
        error: None,
        server_info: None,
        transport: "stdio".into(),
        target: String::new(),
        scope: String::new(),
        tools: Vec::new(),
    }]);
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    prompt(&h, "hi");
    h.wait_for("the models", |s| !s.models.is_empty());
    let ask = |request| {
        Command::Mcp(McpCommand::Request {
            agent: AgentId(COORDINATOR.into()),
            request,
        })
    };
    h.send(ask(McpRequest::Status));
    h.send(ask(McpRequest::Reconnect {
        server: "nope".into(),
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.mcp.len() < 2 {
        assert!(Instant::now() < deadline, "no answers: {:?}", h.mcp);
        h.drain(Duration::from_millis(20));
    }
    assert!(matches!(
        &h.mcp[0],
        (McpRequest::Status, McpOutcome::Status(servers)) if servers.len() == 1
    ));
    assert_eq!(
        h.mcp[1].1,
        McpOutcome::Failed("Server not found: nope".into())
    );
    assert!(fake.sent().contains(&Sent::Mcp(McpRequest::Status)));
    let _ = coordinator;
    h.quit();
}

/// A request the session cannot take (its process is gone) still reaches the open menu, as a
/// failure reply, so its note does not wait for ever (task #44).
#[test]
fn an_mcp_request_that_cannot_be_sent_fails_in_the_menu() {
    use dovetail_api::mcp::{McpOutcome, McpRequest};
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "hi");
    h.wait_for("the models", |s| !s.models.is_empty());
    fake.close_mcp();
    h.send(Command::Mcp(McpCommand::Request {
        agent: AgentId(COORDINATOR.into()),
        request: McpRequest::Status,
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.mcp.is_empty() {
        assert!(Instant::now() < deadline, "no reply");
        h.drain(Duration::from_millis(20));
    }
    assert!(
        matches!(&h.mcp[0], (McpRequest::Status, McpOutcome::Failed(why)) if why.contains("closed"))
    );
    h.quit();
}

/// The model selector (task #39): a live switch changes the agent only once the session
/// confirms it, a refused one keeps the old model and says why, and the choice is what the
/// next start passes as `--model`/`--effort`.
#[test]
fn a_model_switch_is_confirmed_live_refused_safely_and_used_on_the_next_start() {
    use dovetail_api::model::ReasoningEffort;
    // Paused before the first line, so the session stays live and reports nothing itself.
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    prompt(&h, "hi");
    h.wait_for("the selector's models", |s| {
        s.models.len() == dovetail_api::model::BUILTIN_MODELS.len()
    });
    h.send(Command::Model(ModelCommand::Set {
        agent: coordinator.clone(),
        model: "sonnet".into(),
        effort: Some(ReasoningEffort::High),
    }));
    h.wait_for("the confirmed model", |s| {
        let c = coordinator_of(s);
        c.model.as_deref() == Some("sonnet") && c.effort == Some(ReasoningEffort::High)
    });
    assert!(fake.sent().contains(&Sent::Model {
        model: Some("sonnet".into()),
        effort: Some(ReasoningEffort::High),
    }));
    // The same model at another effort: no `set_model`, only the effort.
    h.send(Command::Model(ModelCommand::Set {
        agent: coordinator.clone(),
        model: "sonnet".into(),
        effort: Some(ReasoningEffort::Low),
    }));
    h.wait_for("the new effort", |s| {
        coordinator_of(s).effort == Some(ReasoningEffort::Low)
    });
    assert!(fake.sent().contains(&Sent::Model {
        model: None,
        effort: Some(ReasoningEffort::Low),
    }));

    h.send(Command::Model(ModelCommand::Set {
        agent: coordinator.clone(),
        model: "bogus-model".into(),
        effort: None,
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.failures.iter().any(|f| f.contains("bogus-model")) {
        assert!(Instant::now() < deadline, "no failure: {:?}", h.failures);
        h.drain(Duration::from_millis(20));
    }
    let failure = h.failures.iter().find(|f| f.contains("bogus-model"));
    assert!(
        failure.is_some_and(|f| f.contains("keeps sonnet")),
        "{failure:?}"
    );
    assert_eq!(coordinator_of(&h.snap).model.as_deref(), Some("sonnet"));

    let paths = h.paths.clone();
    let _state = h.quit();
    let fake = fixture("plain_turn");
    let h = Harness::start_on(fake.clone(), paths);
    prompt(&h, "again");
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "never restarted");
        std::thread::sleep(Duration::from_millis(10));
    }
    let spec = &fake.started_specs()[0];
    assert_eq!(spec.model.as_deref(), Some("sonnet"));
    assert_eq!(spec.effort, Some(ReasoningEffort::Low));
    h.quit();
}

/// Review of task #39: a settings answer confirms only its own switch. A late start-up read, or
/// an earlier switch's answer arriving out of order, changes nothing; and a second switch is
/// refused while the first waits for its answer.
#[test]
fn a_stale_or_out_of_order_settings_answer_does_not_confirm_a_switch() {
    use dovetail_api::model::ReasoningEffort;
    use dovetail_driver::{DriverEvent, SwitchId};
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    prompt(&h, "hi");
    h.wait_for("the session started", |_| !fake.started_specs().is_empty());
    fake.hold_model_answers();
    let set = |model: &str, effort| {
        Command::Model(ModelCommand::Set {
            agent: coordinator.clone(),
            model: model.into(),
            effort: Some(effort),
        })
    };
    let models = |f: &FakeDriver| {
        f.sent()
            .iter()
            .filter(|s| matches!(s, Sent::Model { .. }))
            .count()
    };
    // A held switch makes no event, so poll for it.
    let wait_sent = |h: &mut Harness, n: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while models(&fake) < n {
            assert!(Instant::now() < deadline, "switch {n} never sent");
            h.drain(Duration::from_millis(20));
        }
    };
    h.send(set("sonnet", ReasoningEffort::High));
    wait_sent(&mut h, 1);

    // A second switch while the first is unanswered is refused, and never sent.
    h.send(set("haiku", ReasoningEffort::Max));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.failures.iter().any(|f| f.contains("still switching")) {
        assert!(Instant::now() < deadline, "no refusal: {:?}", h.failures);
        h.drain(Duration::from_millis(20));
    }
    assert_eq!(models(&fake), 1);

    // The start-up read answers late, after the switch was sent: ignored.
    fake.send_event(DriverEvent::ModelSettings {
        switch: None,
        model: Some("claude-opus-5-5".into()),
        effort: Some(ReasoningEffort::Low),
    });
    h.drain(Duration::from_millis(200));
    let c = coordinator_of(&h.snap);
    assert_eq!((c.model.as_deref(), c.effort), (None, None));

    // The switch's own answer confirms it.
    fake.release_model_answers(&[0]);
    h.wait_for("the confirmed switch", |s| {
        coordinator_of(s).effort == Some(ReasoningEffort::High)
    });

    // A later switch, held; then the first switch's answer again, out of order: ignored.
    fake.hold_model_answers();
    h.send(set("sonnet", ReasoningEffort::Max));
    wait_sent(&mut h, 2);
    fake.send_event(DriverEvent::ModelSettings {
        switch: Some(SwitchId(0)),
        model: Some("sonnet".into()),
        effort: Some(ReasoningEffort::Low),
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(coordinator_of(&h.snap).effort, Some(ReasoningEffort::High));
    assert!(
        !h.failures.iter().any(|f| f.contains("kept effort")),
        "{:?}",
        h.failures
    );
    fake.release_model_answers(&[0]);
    h.wait_for("the second switch confirmed", |s| {
        coordinator_of(s).effort == Some(ReasoningEffort::Max)
    });
    h.quit();
}

#[test]
fn sessions_are_named_after_the_project_and_a_slice_reports_through_dovetail() {
    use dovetail_api::task::NewTask;
    use dovetail_tools::coordination::ReportState;

    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let effort = create_effort(&mut h, "guest checkout", "guest-checkout");
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "API: guest order endpoint".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: Some(effort),
        criterion: None,
    })));
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: Some(TaskId(1)),
        name: "api".into(),
        brief: "build the endpoint".into(),
    }));
    h.wait_for("the slice working on its task", |s| {
        s.agents.iter().any(|a| a.name == "api")
            && task_status(s, 1) == Some(TaskStatus::InProgress)
    });
    // The slice is told the coordinator's session name, and its own carries the project.
    wait_sent_containing(&fake, "The coordinator's session is named `p.coordinator`");
    let names: Vec<String> = fake
        .started_specs()
        .iter()
        .filter_map(|s| s.name.clone())
        .collect();
    assert!(names.contains(&"p.api".to_string()), "{names:?}");
    wait_sent_containing(&fake, "slice api ready; its session is named `p.api`");
    assert!(names.iter().all(|n| n.starts_with("p.")), "{names:?}");

    let socket = h.socket();
    // Only a slice reports.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, report(ReportState::Done, "all good", None)),
    ));
    assert!(is_error && text.contains("is for slices"), "{text}");

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-api",
            report(ReportState::Progress, "endpoint drafted", None),
        ),
    ));
    assert!(!is_error, "{text}");
    assert_eq!(text, "Reported to the coordinator.");
    h.drain(Duration::from_millis(100));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-api",
            report(ReportState::Done, " PR open, tests pass ", Some(41)),
        ),
    ));
    assert!(!is_error, "{text}");
    assert_eq!(
        text,
        "Reported to the coordinator. Task #1 is now in review."
    );
    h.wait_for("the task in review", |s| {
        task_status(s, 1) == Some(TaskStatus::Review)
    });

    // One line in the coordinator's chat, tagged with the effort of the slice's task.
    let line = h.snap.conversations[&AgentId(COORDINATOR.into())]
        .entries
        .iter()
        .find(|e| e.text.contains("[done]"))
        .expect("the report line")
        .clone();
    assert_eq!(line.kind, EntryKind::AgentMessage);
    assert_eq!(line.effort, Some(effort));
    let parts = line.agent_message().expect("an agent message");
    assert_eq!(
        (parts.from, parts.body),
        ("api", "[done] PR open, tests pass (PR #41)")
    );
    // And the coordinator's session got it as a message.
    wait_sent_containing(
        &fake,
        "Report from slice api on task #1 API: guest order endpoint: [done] PR open, tests \
         pass (PR #41)",
    );
    h.quit();
}

#[test]
fn a_slice_starts_on_the_model_and_effort_asked_for_and_defaults_when_left_out() {
    use dovetail_api::model::ReasoningEffort;
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    make_task(&mut h);
    let (text, is_error) = call_coordinator(
        &h,
        start_on("cheap", Some("haiku"), Some(ReasoningEffort::Low)),
    );
    assert!(!is_error && text.starts_with("started"), "{text}");
    let spec = wait_spec(&fake, "cheap");
    assert_eq!(spec.model.as_deref(), Some("haiku"));
    assert_eq!(spec.effort, Some(ReasoningEffort::Low));

    let (text, is_error) = call_coordinator(&h, start_on("plain", None, None));
    assert!(!is_error && text.starts_with("started"), "{text}");
    // Left out, the model is Sonnet, passed explicitly.
    let spec = wait_spec(&fake, "plain");
    assert_eq!(spec.model.as_deref(), Some("sonnet"));
    assert_eq!(spec.effort, None);

    // An effort alone is accepted, on Sonnet.
    let (text, is_error) =
        call_coordinator(&h, start_on("hard", None, Some(ReasoningEffort::High)));
    assert!(!is_error && text.starts_with("started"), "{text}");
    let spec = wait_spec(&fake, "hard");
    assert_eq!(spec.model.as_deref(), Some("sonnet"));
    assert_eq!(spec.effort, Some(ReasoningEffort::High));
    h.quit();
}

#[test]
fn an_unknown_model_is_denied_once_models_are_listed() {
    use dovetail_api::model::ReasoningEffort;
    use dovetail_driver::{DriverEvent, ModelInfo};
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    make_task(&mut h);
    prompt(&h, "hi");
    wait_specs(&fake, 1);
    fake.send_event(DriverEvent::Models(vec![ModelInfo {
        value: "sonnet".into(),
        resolved: Some("claude-sonnet-5-5".into()),
        display_name: None,
        efforts: ReasoningEffort::ALL.to_vec(),
    }]));
    h.wait_for("the models", |s| {
        s.models
            .iter()
            .any(|m| m.name == "sonnet" && m.efforts.is_some())
    });
    let (text, is_error) = call_coordinator(&h, start_on("odd", Some("gpt-9"), None));
    assert!(
        is_error && text.starts_with("denied") && text.contains("sonnet"),
        "{text}"
    );
    assert!(!h.snap.agents.iter().any(|a| a.name == "odd"));
    // A full name Claude Code lists is fine.
    let (text, is_error) = call_coordinator(&h, start_on("full", Some("claude-sonnet-5-5"), None));
    assert!(!is_error && text.starts_with("started"), "{text}");
    h.quit();
}

#[test]
fn a_queued_slice_keeps_its_model_and_effort_and_a_refused_effort_is_reported() {
    use dovetail_api::model::ReasoningEffort;
    let fake = fixture("plain_turn");
    // Claude Code ignores the effort asked for and keeps `high`.
    fake.apply_at_start(None, Some(ReasoningEffort::High));
    let config = Config {
        max_slices: 1,
        ..Config::default()
    };
    let mut h = Harness::start(fake.clone(), true, config);
    make_task(&mut h);
    let (text, _) = call_coordinator(&h, start_on("first", None, None));
    assert!(text.starts_with("started"), "{text}");
    let (text, is_error) = call_coordinator(
        &h,
        start_on("second", Some("haiku"), Some(ReasoningEffort::Low)),
    );
    assert!(!is_error && text.starts_with("queued"), "{text}");
    h.send(Command::Agent(AgentCommand::CloseSlice {
        agent: AgentId("slice-first".into()),
    }));
    h.wait_for("the queued slice", |s| {
        s.agents.iter().any(|a| a.name == "second")
    });
    // The coordinator is woken by "slice ready" and starts too: find the slice's own session.
    let deadline = Instant::now() + Duration::from_secs(5);
    let spec = loop {
        let found = fake
            .started_specs()
            .into_iter()
            .find(|s| s.name.as_deref() == Some("p.second"));
        if let Some(spec) = found {
            break spec;
        }
        assert!(Instant::now() < deadline, "second never started");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(spec.model.as_deref(), Some("haiku"));
    assert_eq!(spec.effort, Some(ReasoningEffort::Low));

    h.wait_for("the applied effort", |s| {
        s.agents
            .iter()
            .any(|a| a.name == "second" && a.effort == Some(ReasoningEffort::High))
    });
    h.wait_for("the notice", |s| {
        s.conversations
            .get(&AgentId("slice-second".into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.text.contains("did not get what was asked for"))
            })
    });
    h.quit();
}

#[test]
fn a_typo_accepted_while_models_are_unknown_is_reported_after_the_start_up_read() {
    use dovetail_api::model::ReasoningEffort;
    let fake = fixture("plain_turn");
    // No model list yet; Claude Code falls back to Sonnet and says so.
    fake.apply_at_start(Some("claude-sonnet-5-5"), Some(ReasoningEffort::Medium));
    let mut h = Harness::start(fake.clone(), true, Config::default());
    make_task(&mut h);
    let (text, is_error) = call_coordinator(&h, start_on("typo", Some("opuss"), None));
    assert!(!is_error && text.starts_with("started"), "{text}");
    h.wait_for("the mismatch", |s| {
        notices_containing(s, "slice-typo", "model opuss (it runs claude-sonnet-5-5)") == 1
    });
    // A correct alias is not a mismatch: the applied name contains it.
    let (text, _) = call_coordinator(&h, start_on("fine", Some("sonnet"), None));
    assert!(text.starts_with("started"), "{text}");
    wait_specs(&fake, 2);
    h.drain(Duration::from_millis(300));
    assert_eq!(notices_containing(&h.snap, "slice-fine", "did not get"), 0);
    h.quit();
}

#[test]
fn a_model_claude_code_refuses_at_launch_is_named_in_the_failure() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    make_task(&mut h);
    let (text, is_error) = call_coordinator(&h, start_on("refused", Some("bogus-model"), None));
    assert!(!is_error && text.starts_with("started"), "{text}");
    h.wait_for("the failure", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Error { text }
                if text.contains("`bogus-model`") && text.contains("refused that model"))
        })
    });
    h.quit();
}

#[test]
fn a_restart_before_the_start_up_read_still_checks_the_launch_choice_once() {
    use dovetail_api::model::ReasoningEffort;
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    make_task(&mut h);
    let (text, _) = call_coordinator(&h, start_on("r", Some("haiku"), Some(ReasoningEffort::Low)));
    assert!(text.starts_with("started"), "{text}");
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "r"));
    // Quit before any start-up read arrived (the fake sends none): the row keeps the request.
    let paths = h.paths.clone();
    let _state = h.quit();

    // Next run: the slice resumes, and Claude Code applies `high`. The check runs and says so.
    let fake = fixture("plain_turn");
    fake.apply_at_start(None, Some(ReasoningEffort::High));
    let mut h = Harness::start_on(fake.clone(), paths);
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId("slice-r".into()),
        text: "again".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    h.wait_for("the mismatch", |s| {
        notices_containing(s, "slice-r", "effort low (it runs at high)") == 1
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    // Once reported, the row keeps what is applied: the next start passes it and says nothing.
    let fake = fixture("plain_turn");
    fake.apply_at_start(None, Some(ReasoningEffort::High));
    let mut h = Harness::start_on(fake.clone(), paths);
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId("slice-r".into()),
        text: "once more".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    assert_eq!(wait_spec(&fake, "r").effort, Some(ReasoningEffort::High));
    h.drain(Duration::from_millis(300));
    assert_eq!(notices_containing(&h.snap, "slice-r", "did not get"), 1);
    h.quit();
}

/// Writes one task file the way Claude Code does.
fn write_item(list: &Path, id: &str, subject: &str, status: &str) {
    std::fs::create_dir_all(list).expect("list dir");
    let text = format!(
        "{{\n  \"id\": \"{id}\",\n  \"subject\": \"{subject}\",\n  \"description\": \"{subject}\",\n  \
         \"status\": \"{status}\",\n  \"blocks\": [],\n  \"blockedBy\": []\n}}"
    );
    std::fs::write(list.join(format!("{id}.json")), text).expect("task file");
}

fn todo(s: &Snapshot, id: &str) -> Option<dovetail_api::agent::TodoItem> {
    s.todos.iter().find(|t| t.id == id).cloned()
}

fn owner(s: &Snapshot, id: &str) -> Option<String> {
    todo(s, id).and_then(|t| t.created_by).map(|a| a.0)
}

/// No fresh start raises a Needs-you item: the old transcript just stays on disk (task #38).
fn no_permission_item(s: &Snapshot) -> bool {
    s.inbox
        .iter()
        .all(|i| !matches!(i.kind, InboxKind::Permission { .. }))
}

/// The spec the session named `p.<name>` was started with, once it has started. The
/// coordinator is woken by "slice ready" and starts too, so a slice is found by its name.
fn wait_spec(fake: &FakeDriver, name: &str) -> dovetail_driver::SessionSpec {
    let want = format!("p.{name}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = fake
            .started_specs()
            .into_iter()
            .rev()
            .find(|s| s.name.as_deref() == Some(want.as_str()));
        if let Some(spec) = found {
            return spec;
        }
        assert!(Instant::now() < deadline, "{want} never started");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn notices_containing(s: &Snapshot, agent: &str, needle: &str) -> usize {
    s.conversations.get(&AgentId(agent.into())).map_or(0, |t| {
        t.entries.iter().filter(|e| e.text.contains(needle)).count()
    })
}
