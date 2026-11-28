//! Integration tests for the `activity` domain: inbox permissions and questions, status
//! updates, peer messages, and usage/rate-limit events.

mod common;

use common::*;

use dovetail_api::{
    Command,
    agent::{AgentId, AgentKind, AgentStatus, PermissionMode},
    conversation::{Attachment, EntryKind, ImageType},
    inbox::{InboxKind, PermissionAnswer},
};
use dovetail_api::{conversation::ConversationCommand, inbox::InboxCommand};
use dovetail_core::config::Config;
use dovetail_driver::{ControlAnswer, ReplayOptions, Sent};
use dovetail_tools::report::ReportTool;
use dovetail_tools::{Tool, report::ReportStatus};

#[test]
fn a_peer_message_becomes_an_agent_message_entry() {
    let fake = fixture("peer_message");
    let mut h = Harness::start(fake, true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let entries = h.entries(COORDINATOR);
    let text =
        dovetail_api::conversation::agent_message_text("scout", false, "The report.\nSecond line.");
    assert!(
        entries.contains(&(EntryKind::AgentMessage, text)),
        "{entries:?}"
    );
    assert!(
        !entries
            .iter()
            .any(|(k, t)| *k == EntryKind::Notice && t.contains("scout"))
    );
}

/// Task #26: Claude Code 2.1.319 can send a `rate_limit_event` with only the weekly window
/// (`rate_limit_week_only` holds the live line). The core reports the weekly window and no 5-hour
/// one; the header turns that into `5h —` (dovetail-tui `usage_header` tests).
#[test]
fn a_rate_limit_event_with_only_the_weekly_window_reaches_the_ui_as_such() {
    let mut h = Harness::start(fixture("rate_limit_week_only"), true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the usage limits", |s| s.usage.seven_day.is_some());
    assert_eq!(s_util(&h.snap.usage.seven_day), Some(0.06));
    assert_eq!(
        h.snap.usage.seven_day.as_ref().and_then(|w| w.resets_at_ms),
        Some(1_794_754_800_000)
    );
    assert_eq!(h.snap.usage.five_hour, None);
    h.quit();
}

#[test]
fn concurrent_permissions_are_answered_in_any_order_and_non_auto_init_warns() {
    let fake = fixture("concurrent_permissions").with_options(ReplayOptions {
        wait_for_answers: true,
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "read two files");
    h.wait_for("two permission requests", |s| {
        s.inbox
            .iter()
            .filter(|i| matches!(i.kind, InboxKind::Permission { .. }))
            .count()
            == 2
    });
    // The fixture's init reports `default`: run as manual and warn.
    assert!(h.snap.warnings.iter().any(|w| w.text.contains("not auto")));
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .unwrap();
    assert_eq!(coordinator.permission_mode, PermissionMode::Manual);

    let ids: Vec<_> = h.snap.inbox.iter().map(|i| i.id).collect();
    // The second first, then the first: neither blocks the other.
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: ids[1],
        answer: PermissionAnswer::Allow,
    }));
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: ids[0],
        answer: PermissionAnswer::AllowAlways,
    }));
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let answers: Vec<_> = fake
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::Answer { id, answer } => Some((id, answer)),
            _ => None,
        })
        .collect();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0].1, ControlAnswer::allow());
    // "Always allow" sends the offered rule, kept to this session (never written to the repo).
    match &answers[1].1 {
        ControlAnswer::Allow {
            updated_permissions,
            ..
        } => assert_eq!(updated_permissions[0]["destination"], "session"),
        other => panic!("{other:?}"),
    }
    assert!(
        h.entries(COORDINATOR)
            .contains(&(EntryKind::Assistant, "uno dos".into()))
    );
    h.quit();
}

#[test]
fn a_subagents_permission_request_reaches_the_inbox_marked_as_its() {
    let fake = fixture("subagent_permission").with_options(ReplayOptions {
        wait_for_answers: true,
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake, true, Config::default());
    prompt(&h, "foreground subagent");
    h.wait_for("the request", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Permission { reason: Some(r), .. } if r.starts_with("from subagent"))
        })
    });
    // The foreground subagent is a node under the coordinator while it runs.
    let sub = subagent(&h.snap).expect("a subagent node");
    assert_eq!(sub.kind, AgentKind::Subagent { background: false });
    assert_eq!(sub.name, "Read one.txt file");
    assert_eq!(sub.status, AgentStatus::Working);
    let id = h.snap.inbox[0].id;
    h.send(Command::Inbox(InboxCommand::AnswerPermission {
        item: id,
        answer: PermissionAnswer::Deny { message: None },
    }));
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let sub = subagent(&h.snap).expect("a subagent node");
    assert_eq!(sub.status, AgentStatus::Done);
    let progress = sub.progress.as_ref().expect("progress");
    assert_eq!(progress.tool_uses, 1);
    assert!(
        progress
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("uno")),
        "{progress:?}"
    );
    h.quit();
}

#[test]
fn report_status_sets_the_line_cut_at_120_characters() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::Report(ReportTool::ReportStatus(ReportStatus {
                status: format!("{}\nsecond line", "x".repeat(200)),
            })),
        ),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("the status line", |s| {
        s.agents.iter().any(|a| a.status_line.len() == 120)
    });

    h.quit();
}

#[test]
fn ask_user_returns_at_once_and_an_inbox_answer_arrives_as_a_message() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    // `socket_call` times out after 5 s, so a held call would fail here.
    let (text, is_error) = ask(&h, "Which port?");
    assert!(!is_error && text.starts_with("Question sent"), "{text}");
    h.wait_for("the question", |s| open_questions(s).len() == 1);
    // It is in the conversation too.
    assert!(
        h.entries(COORDINATOR)
            .iter()
            .any(|(k, t)| *k == EntryKind::Notice && t.contains("Which port?")),
        "{:?}",
        h.entries(COORDINATOR)
    );
    let id = open_questions(&h.snap)[0];
    h.send(Command::Inbox(InboxCommand::AnswerQuestion {
        item: id,
        text: "8080".into(),
    }));
    wait_sent(&fake, "Answer to your question 'Which port?': 8080");
    h.wait_for("the item to clear", |s| open_questions(s).is_empty());
    h.quit();
}

#[test]
fn a_chat_message_with_open_questions_goes_through_and_clears_them() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    ask(&h, "Which port?");
    ask(&h, "Which host?");
    h.wait_for("both questions", |s| open_questions(s).len() == 2);
    prompt(&h, "8080 on localhost");
    wait_sent(&fake, "8080 on localhost");
    h.wait_for("the items to clear", |s| open_questions(s).is_empty());
    h.quit();
}

/// Review of 2026-11-09: an image-only message answered open questions with "".
#[test]
fn an_image_only_message_leaves_open_questions_open() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    ask(&h, "Which port?");
    h.wait_for("the question", |s| open_questions(s).len() == 1);
    let dir = h.paths.data.join("attachments");
    std::fs::create_dir_all(&dir).expect("attachments dir");
    let path = dir.join("1.png");
    let bytes = b"\x89PNG\r\n\x1a\nfake";
    std::fs::write(&path, bytes).expect("write image");
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: String::new(),
        attachments: vec![Attachment {
            path,
            media_type: ImageType::Png,
            size: bytes.len() as u64,
        }],
        effort: None,
    }));
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    assert_eq!(fake.user_messages().len(), 1, "the image went out");
    assert_eq!(open_questions(&h.snap).len(), 1, "still waiting for words");
    h.quit();
}
