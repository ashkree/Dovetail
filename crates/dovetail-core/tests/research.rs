//! Research capture (R1, `docs/research.md` §5) through the real core and `FakeDriver`: the two
//! spike S7 recordings, a source fetched twice, a restart, and the opt-in research folder.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Harness, fixture, prompt, read, test_paths};
use dovetail_api::{
    Command, Snapshot,
    inbox::InboxKind,
    research::{ResearchCommand, ResearchContent, ResearchWhat},
    task::{NewTask, TaskCommand, TaskId},
};
use dovetail_core::paths::Paths;
use dovetail_core::projects::Project;
use dovetail_driver::{FakeDriver, Sent};

fn start(driver: &FakeDriver, paths: &Paths, repo: &Path) -> Harness {
    Harness::start_project(
        driver.clone(),
        paths.clone(),
        Project::with_repo("p", "work", repo.to_path_buf(), None),
    )
}

/// Creates the task "Example research" and opens round 1 on it.
fn task_with_round(app: &mut Harness) -> TaskId {
    app.send(Command::Task(TaskCommand::Create(NewTask {
        title: "Example research".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    app.wait_for("the task", |s| !s.tasks.is_empty());
    let task = app.snap.tasks[0].id;
    app.send(Command::Research(ResearchCommand::StartRound {
        task,
        brief: "What is the codeword, and the page's title?".into(),
    }));
    app.wait_for("the open round", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.open_round().is_some())
    });
    task
}

fn finished_turn(s: &Snapshot) -> bool {
    s.inbox
        .iter()
        .any(|i| matches!(i.kind, InboxKind::FinishedTurn { .. }))
}

/// Every file under `dir`, recursively, sorted.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Replays a S7 recording in a round and checks the filed finding, its sources and that only
/// the user's prompt reached the session. Resolves to the task's research folder.
fn replay_round(name: &str, paths: &Paths, repo: &Path) -> PathBuf {
    let fake = fixture(name);
    let mut app = start(&fake, paths, repo);
    let task = task_with_round(&mut app);
    prompt(&app, "go");
    app.wait_for("the finding", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.findings == 1 && r.sources == 2)
    });
    app.wait_for("the finished turn", finished_turn);
    let summary = app.research(task).expect("summary").clone();
    assert_eq!(summary.rounds.len(), 1);
    assert_eq!(summary.rounds[0].findings, 1);
    assert_eq!(summary.quick_findings, 0);
    assert!(app.failures.is_empty(), "{:?}", app.failures);

    // Context isolation (§5.6): the user's prompt is all the session was sent.
    let sent = fake.sent();
    assert_eq!(sent, vec![Sent::User("go".into())], "{sent:#?}");

    let round_dir = summary.dir.join(&summary.rounds[0].dir);
    assert!(read(&round_dir.join("brief.md")).contains("What is the codeword"));
    let finding = round_dir.join("findings/01-fetch-page-and-read-notes.md");
    let finding = if name.ends_with("background") {
        round_dir.join("findings/01-fetch-page-read-notes-report.md")
    } else {
        finding
    };
    let text = read(&finding);
    // Sources from the tool calls, labelled and linked.
    assert!(text.contains("https://example.com (WebFetch)"), "{text}");
    assert!(text.contains("research/repo/notes.txt (Read)"), "{text}");
    assert!(
        text.contains("[what the agent saw](../../../sources/"),
        "{text}"
    );
    // The report is the handback, never the boilerplate summary.
    assert!(text.contains("Codeword: MARIGOLD-42"), "{text}");
    assert!(!text.contains("delivered to you as a message"), "{text}");
    // Filed on `task_updated` from the handback call; the notification's identical text changes
    // nothing.
    assert!(text.contains("body_from: subagent_handback"), "{text}");

    let sources = files_under(&summary.dir.join("sources"));
    assert_eq!(sources.len(), 2, "{sources:?}");
    let web = sources
        .iter()
        .map(|p| read(p))
        .find(|t| t.contains("tool: WebFetch"))
        .expect("the WebFetch source");
    assert!(web.contains("What the agent saw"), "{web}");
    assert!(
        web.contains("**Question asked of the page:** What is"),
        "{web}"
    );
    assert!(web.contains("Example Domain"), "{web}");

    // The listing names the same things, with absolute paths.
    app.send(Command::Research(ResearchCommand::Load {
        task,
        what: ResearchWhat::Listing,
    }));
    app.wait_until("the listing", |app| !app.loaded.is_empty());
    let Some((_, ResearchWhat::Listing, ResearchContent::Listing(listing))) = app.loaded.first()
    else {
        panic!("no listing: {:?}", app.loaded);
    };
    assert_eq!(listing.findings.len(), 1);
    assert_eq!(listing.findings[0].path, finding);
    assert_eq!(listing.findings[0].sources.len(), 2);
    assert_eq!(listing.sources.len(), 2);
    // ADR 0025: filing the finding put one finding card in its effort's incoming strip.
    app.wait_for("the finding card", |s| !s.cards.is_empty());
    assert_eq!(app.snap.cards.len(), 1, "{:?}", app.snap.cards);
    let card = &app.snap.cards[0];
    assert_eq!(card.kind, dovetail_api::canvas::CardKind::Finding);
    assert_eq!(card.block, None);
    assert_eq!(card.effort, dovetail_api::effort::UNSORTED_EFFORT);
    assert!(!card.missing && !card.title.is_empty(), "{card:?}");
    assert_eq!(
        card.reference.as_ref().map(|r| (r.kind, r.id.as_str())),
        Some((dovetail_api::canvas::RefKind::Finding, "1"))
    );
    assert_eq!(card.text, "");
    let dir = summary.dir.clone();
    app.quit();
    dir
}

#[test]
fn a_foreground_subagent_in_a_round_is_filed_with_its_sources() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("work");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = test_paths(dir.path());
    let research = replay_round("research_round_foreground", &paths, &repo);
    assert!(research.starts_with(paths.data.join("projects/p/research")));
    assert!(research.ends_with("1-example-research"));
}

#[test]
fn a_background_subagent_is_filed_on_its_notification_not_the_turn_result() {
    // The recording's `result` comes before the subagent's WebFetch: the finding still has it.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("work");
    std::fs::create_dir_all(&repo).unwrap();
    replay_round("research_round_background", &test_paths(dir.path()), &repo);
}

#[test]
fn an_opt_in_research_dir_writes_only_under_that_folder_of_the_repo() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("work");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
    let paths = test_paths(dir.path());
    std::fs::create_dir_all(paths.config.join("projects")).unwrap();
    std::fs::write(
        paths.recipe("p"),
        "name = \"work\"\nresearch_dir = \"docs/research\"\n",
    )
    .unwrap();
    let before = files_under(&repo);
    let research = replay_round("research_round_foreground", &paths, &repo);
    assert_eq!(research, repo.join("docs/research/1-example-research"));
    let new: Vec<PathBuf> = files_under(&repo)
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect();
    assert!(!new.is_empty());
    for p in &new {
        assert!(p.starts_with(repo.join("docs/research")), "{}", p.display());
    }
    assert!(!paths.data.join("projects/p/research").exists());
}

#[test]
fn a_restart_keeps_rounds_findings_and_sources_and_rebuilds_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("work");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = test_paths(dir.path());
    let research = replay_round("research_round_foreground", &paths, &repo);
    let before: Vec<(PathBuf, String)> = files_under(&research)
        .into_iter()
        .map(|p| {
            let t = read(&p);
            (p, t)
        })
        .collect();
    assert_eq!(before.len(), 4, "brief, finding, two sources: {before:?}");
    std::fs::remove_dir_all(&research).unwrap();

    let mut app = start(&fixture("plain_turn"), &paths, &repo);
    let summary = app
        .snap
        .research
        .first()
        .expect("research after restart")
        .clone();
    assert_eq!(summary.findings, 1);
    assert_eq!(summary.sources, 2);
    assert_eq!(summary.rounds.len(), 1);
    // The round stays open across the restart.
    assert!(summary.open_round().is_some());
    let deadline = Instant::now() + Duration::from_secs(5);
    while files_under(&research).len() < before.len() && Instant::now() < deadline {
        app.drain(Duration::from_millis(20));
    }
    let after: Vec<(PathBuf, String)> = files_under(&research)
        .into_iter()
        .map(|p| {
            let t = read(&p);
            (p, t)
        })
        .collect();
    assert_eq!(after, before);
    app.quit();
}

/// Two foreground subagents: `A` fetches one page twice, `B` once, with the same answer.
const FETCHED_TWICE: &[&str] = &[
    r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
    r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_A","name":"Agent","input":{"description":"Twice","subagent_type":"general-purpose","prompt":"fetch twice"}},{"type":"tool_use","id":"toolu_B","name":"Agent","input":{"description":"Once","subagent_type":"general-purpose","prompt":"fetch once"}}]},"parent_tool_use_id":null}"#,
    r#"{"type":"system","subtype":"task_started","task_id":"ta","tool_use_id":"toolu_A","description":"Twice","subagent_type":"general-purpose","is_backgrounded":false,"task_type":"local_agent"}"#,
    r#"{"type":"system","subtype":"task_started","task_id":"tb","tool_use_id":"toolu_B","description":"Once","subagent_type":"general-purpose","is_backgrounded":false,"task_type":"local_agent"}"#,
    r#"{"type":"assistant","message":{"id":"a1","content":[{"type":"tool_use","id":"toolu_W1","name":"WebFetch","input":{"url":"https://example.com","prompt":"Title?"}}]},"parent_tool_use_id":"toolu_A"}"#,
    r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_W1","content":"Example Domain"}]},"parent_tool_use_id":"toolu_A"}"#,
    r#"{"type":"assistant","message":{"id":"a2","content":[{"type":"tool_use","id":"toolu_W2","name":"WebFetch","input":{"url":"https://example.com","prompt":"Title?"}}]},"parent_tool_use_id":"toolu_A"}"#,
    r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_W2","content":"Example Domain"}]},"parent_tool_use_id":"toolu_A"}"#,
    r#"{"type":"assistant","message":{"id":"b1","content":[{"type":"tool_use","id":"toolu_W3","name":"WebFetch","input":{"url":"https://example.com","prompt":"Title?"}}]},"parent_tool_use_id":"toolu_B"}"#,
    r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_W3","content":"Example Domain"}]},"parent_tool_use_id":"toolu_B"}"#,
    r#"{"type":"assistant","message":{"id":"a3","content":[{"type":"tool_use","id":"toolu_H1","name":"SubagentHandback","input":{"message":"A: Example Domain, twice"}}]},"parent_tool_use_id":"toolu_A"}"#,
    r#"{"type":"assistant","message":{"id":"b2","content":[{"type":"tool_use","id":"toolu_H2","name":"SubagentHandback","input":{"message":"B: Example Domain"}}]},"parent_tool_use_id":"toolu_B"}"#,
    r#"{"type":"system","subtype":"task_updated","task_id":"ta","patch":{"status":"completed"}}"#,
    r#"{"type":"system","subtype":"task_notification","task_id":"ta","tool_use_id":"toolu_A","status":"completed","summary":"boilerplate","handback":"send","handback_report":{"text":"A: Example Domain, twice"}}"#,
    r#"{"type":"system","subtype":"task_updated","task_id":"tb","patch":{"status":"completed"}}"#,
    r#"{"type":"system","subtype":"task_notification","task_id":"tb","tool_use_id":"toolu_B","status":"completed","summary":"boilerplate","handback":"send","handback_report":{"text":"B: Example Domain"}}"#,
    r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_A","content":"boilerplate"},{"type":"tool_result","tool_use_id":"toolu_B","content":"boilerplate"}]},"parent_tool_use_id":null}"#,
    r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"done"}]},"parent_tool_use_id":null}"#,
    r#"{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.01}"#,
];

#[test]
fn the_same_page_fetched_twice_is_one_source_row_linked_by_each_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("work");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = test_paths(dir.path());
    let fake = FakeDriver::from_ndjson(&FETCHED_TWICE.join("\n")).expect("fixture");
    let mut app = start(&fake, &paths, &repo);
    let task = task_with_round(&mut app);
    prompt(&app, "go");
    app.wait_for("two findings", |s| {
        s.research.iter().any(|r| r.task == task && r.findings == 2)
    });
    assert_eq!(app.research(task).expect("summary").sources, 1);
    app.quit();

    let db = rusqlite::Connection::open(paths.state_db("p")).unwrap();
    let count = |sql: &str| -> i64 { db.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM sources"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM finding_sources"), 3);
    // `A` reached it twice: two links from one finding, told apart by the tool use.
    assert_eq!(
        count(
            "SELECT COUNT(*) FROM finding_sources fs JOIN findings f ON f.id = fs.finding_id \
             WHERE f.label = 'twice'"
        ),
        2
    );
    let body: String = db
        .query_row("SELECT body FROM findings WHERE label = 'once'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(body, "B: Example Domain");
}

#[test]
fn with_no_round_open_the_coordinators_subagents_are_not_filed_under_any_task() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("work");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = test_paths(dir.path());
    let fake = fixture("research_round_foreground");
    let mut app = start(&fake, &paths, &repo);
    prompt(&app, "go");
    app.wait_for("the finished turn", finished_turn);
    // No task has research; the finding waits, unfiled, in Needs you (follow-ups of 2026-11-11).
    assert!(app.snap.research.is_empty());
    assert!(app.snap.inbox.iter().any(|i| matches!(
        i.kind,
        dovetail_api::inbox::InboxKind::UnfiledFinding { .. }
    )));
    assert!(paths.data.join("projects/p/research/unfiled").exists());
    app.quit();
}
