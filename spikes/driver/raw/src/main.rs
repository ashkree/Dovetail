//! S1 checks against the hand-written NDJSON layer.
//! Usage: spike-raw <check> ; run in a scratch repo given by $SPIKE_REPO,
//! logs to $SPIKE_OUT (default /tmp/dovetail-spikes/driver/out).

use serde_json::{Value, json};
use spike_raw::*;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

fn repo() -> PathBuf {
    std::env::var("SPIKE_REPO")
        .unwrap_or_else(|_| "/tmp/dovetail-spikes/driver/repo".into())
        .into()
}
fn out() -> PathBuf {
    let p: PathBuf = std::env::var("SPIKE_OUT")
        .unwrap_or_else(|_| "/tmp/dovetail-spikes/driver/out".into())
        .into();
    ensure_dir(&p);
    p
}
fn args(extra: &[&str]) -> Vec<String> {
    let mut a = base_args();
    a.extend(extra.iter().map(|s| s.to_string()));
    a
}
fn spec(name: &str, extra: &[&str]) -> Spec {
    Spec { cwd: repo(), args: args(extra), env: vec![], log: out().join(format!("{name}.log.ndjson")) }
}

/// Drain events until `n` results arrived (or timeout). Auto-allows
/// permission requests if `allow` is set. Returns all lines.
async fn until_results(
    s: &Session,
    rx: &mut UnboundedReceiver<Ev>,
    n: usize,
    timeout_s: u64,
    allow: bool,
) -> Vec<Value> {
    let mut seen = vec![];
    let mut results = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_s);
    while results < n {
        let ev = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(ev)) => ev,
            _ => {
                println!("  [{:.1}s] timeout/closed waiting for result {}", s.secs(), results + 1);
                break;
            }
        };
        match ev {
            Ev::Line(v) => {
                if v["type"] == "result" {
                    results += 1;
                    println!(
                        "  [{:.1}s] result #{results} subtype={} is_error={} text={:?}",
                        s.secs(),
                        v["subtype"],
                        v["is_error"],
                        v["result"].as_str().map(|t| t.chars().take(80).collect::<String>())
                    );
                }
                seen.push(v);
            }
            Ev::Perm { request_id, request } => {
                println!("  [{:.1}s] perm {} {}", s.secs(), request["tool_name"], request["input"]);
                if allow {
                    let _ = s.answer(&request_id, true, &request["input"]).await;
                }
                seen.push(json!({"_perm": request}));
            }
            Ev::OtherControl(v) => {
                println!("  [{:.1}s] other control {}", s.secs(), v);
                seen.push(v)
            }
            Ev::Closed => {
                println!("  [{:.1}s] stdout closed", s.secs());
                break;
            }
        }
    }
    seen
}

fn init_of(lines: &[Value]) -> Option<&Value> {
    lines.iter().find(|v| v["type"] == "system" && v["subtype"] == "init")
}

fn summary(lines: &[Value]) {
    let mut counts = std::collections::BTreeMap::new();
    for v in lines {
        *counts.entry(ty(v)).or_insert(0) += 1;
    }
    println!("  types: {counts:?}");
}

#[tokio::main]
async fn main() {
    let check = std::env::args().nth(1).unwrap_or_default();
    match check.as_str() {
        "a" => check_a().await,
        "b" => check_b().await,
        "c" => check_c().await,
        "d" => check_d().await,
        "e" => check_e().await,
        "g" => check_g().await,
        "bg" => check_bg().await,
        "todo" => check_todo().await,
        "pt" => check_prompt_tool().await,
        _ => eprintln!("checks: a b c d e g bg todo pt"),
    }
}

/// A + F + re-checks: two concurrent sessions with chosen ids; one plain,
/// one with partial messages. Also `permissionMode` in init and modelUsage.
async fn check_a() {
    let id1 = uuid();
    let id2 = uuid();
    let (s1, mut r1) = Session::spawn(spec(
        "plain_turn",
        &["--permission-mode", "auto", "--session-id", &id1, "--name", "spike-alpha"],
    ))
    .await
    .expect("spawn 1");
    let (s2, mut r2) = Session::spawn(spec(
        "partial_stream",
        &["--permission-mode", "auto", "--session-id", &id2, "--include-partial-messages"],
    ))
    .await
    .expect("spawn 2");
    s1.send_user("Reply with exactly the word: alpha").await.unwrap();
    s2.send_user("Write two short sentences about rivers. No tools.").await.unwrap();
    let (l1, l2) = tokio::join!(
        until_results(&s1, &mut r1, 1, 120, false),
        until_results(&s2, &mut r2, 1, 120, false)
    );
    for (name, id, lines) in [("s1", &id1, &l1), ("s2", &id2, &l2)] {
        let foreign: Vec<_> = lines
            .iter()
            .filter_map(|v| v.get("session_id").and_then(Value::as_str))
            .filter(|sid| sid != id)
            .collect();
        println!("{name}: chosen id {id}; foreign session ids: {foreign:?}");
        if let Some(init) = init_of(lines) {
            println!(
                "  init session_id={} permissionMode={} model={} cwd={}",
                init["session_id"], init["permissionMode"], init["model"], init["cwd"]
            );
        }
        summary(lines);
        if let Some(r) = lines.iter().find(|v| v["type"] == "result") {
            println!("  result.modelUsage = {}", r["modelUsage"]);
            println!("  result.usage = {}", r["usage"]);
        }
    }
    // F: first text delta before the final assistant message?
    let first_delta = l2.iter().position(|v| {
        v["type"] == "stream_event" && v["event"]["delta"]["type"] == "text_delta"
    });
    let first_assistant = l2.iter().position(|v| v["type"] == "assistant");
    let n_deltas = l2
        .iter()
        .filter(|v| v["type"] == "stream_event" && v["event"]["delta"]["type"] == "text_delta")
        .count();
    println!("F: first text_delta idx={first_delta:?}, first assistant idx={first_assistant:?}, deltas={n_deltas}");
    println!("exit s1 {:?}", s1.finish(20).await);
    println!("exit s2 {:?}", s2.finish(20).await);
    println!("ids: {id1} {id2}");
}

/// B: interrupt during `sleep 60`, then another prompt on the same process.
async fn check_b() {
    let (s, mut rx) = Session::spawn(spec(
        "interrupt",
        &["--permission-mode", "auto", "--allowedTools", "Bash(sleep:*)"],
    ))
    .await
    .unwrap();
    let pid = s.child.id();
    s.send_user("Run the shell command `sleep 60` in the foreground (not in background) with the Bash tool, then reply 'slept'.").await.unwrap();
    // wait for the Bash tool_use
    let mut lines = vec![];
    loop {
        match tokio::time::timeout(Duration::from_secs(90), rx.recv()).await {
            Ok(Some(Ev::Line(v))) => {
                let is_bash = v["type"] == "assistant"
                    && v["message"]["content"]
                        .as_array()
                        .map(|c| c.iter().any(|b| b["type"] == "tool_use" && b["name"] == "Bash"))
                        .unwrap_or(false);
                lines.push(v);
                if is_bash {
                    break;
                }
            }
            Ok(Some(Ev::Perm { request_id, request })) => {
                println!("  perm {} (allowing)", request["tool_name"]);
                s.answer(&request_id, true, &request["input"]).await.unwrap();
            }
            other => {
                println!("no bash tool_use: {other:?}");
                return;
            }
        }
    }
    tokio::time::sleep(Duration::from_secs(3)).await; // let sleep start
    let t0 = s.secs();
    println!("  [{t0:.1}s] sending interrupt");
    let resp = s.control(json!({"subtype": "interrupt"})).await;
    println!("  [{:.1}s] interrupt control_response: {:?}", s.secs(), resp.map(|r| r["response"].clone()));
    let l = until_results(&s, &mut rx, 1, 30, true).await;
    println!("  turn stopped {:.1}s after interrupt", s.secs() - t0);
    summary(&l);
    s.send_user("Reply with exactly: after").await.unwrap();
    let l = until_results(&s, &mut rx, 1, 60, true).await;
    summary(&l);
    println!("  same pid: {:?} (alive: {})", pid, s.child.id().is_some());
    println!("exit {:?}", s.finish(20).await);
}

/// C: manual mode, two pending permission requests; hold the first 60 s,
/// answer the second at once.
async fn check_c() {
    let outside = "/tmp/dovetail-spikes/driver/outside";
    ensure_dir(std::path::Path::new(outside));
    std::fs::write(format!("{outside}/one.txt"), "uno\n").unwrap();
    std::fs::write(format!("{outside}/two.txt"), "dos\n").unwrap();
    let (s, mut rx) = Session::spawn(spec("concurrent_permissions", &["--permission-mode", "manual"]))
        .await
        .unwrap();
    s.send_user(&format!(
        "In a single message, call the Read tool twice in parallel: {outside}/one.txt and {outside}/two.txt. Then reply with both contents on one line."
    ))
    .await
    .unwrap();
    let s = std::sync::Arc::new(s);
    let mut perms = 0;
    let mut held: Option<tokio::task::JoinHandle<()>> = None;
    let mut lines = vec![];
    let mut t_first_answer: Option<f64> = None;
    loop {
        let ev = match tokio::time::timeout(Duration::from_secs(150), rx.recv()).await {
            Ok(Some(ev)) => ev,
            _ => break,
        };
        match ev {
            Ev::Perm { request_id, request } => {
                perms += 1;
                println!("  [{:.1}s] perm #{perms}: {}", s.secs(), request);
                if perms == 1 {
                    let s2 = s.clone();
                    held = Some(tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        println!("  [{:.1}s] answering held request #1", s2.secs());
                        s2.answer(&request_id, true, &request["input"]).await.unwrap();
                    }));
                } else {
                    println!("  [{:.1}s] answering #{perms} at once", s.secs());
                    t_first_answer = Some(s.secs());
                    s.answer(&request_id, true, &request["input"]).await.unwrap();
                }
            }
            Ev::Line(v) => {
                if v["type"] == "user" {
                    if let Some(c) = v["message"]["content"].as_array() {
                        for b in c.iter().filter(|b| b["type"] == "tool_result") {
                            println!(
                                "  [{:.1}s] tool_result {} -> {}",
                                s.secs(),
                                b["tool_use_id"],
                                b["content"].to_string().chars().take(60).collect::<String>()
                            );
                        }
                    }
                }
                if v["type"] == "result" {
                    println!("  [{:.1}s] result: {}", s.secs(), v["result"]);
                    lines.push(v);
                    break;
                }
                lines.push(v);
            }
            other => println!("  {other:?}"),
        }
    }
    let _ = t_first_answer;
    if let Some(h) = held {
        let _ = h.await;
    }
    summary(&lines);
    match std::sync::Arc::try_unwrap(s) {
        Ok(s) => println!("exit {:?}", s.finish(20).await),
        Err(_) => println!("session still shared"),
    }
}

/// D: auto -> acceptEdits -> manual, over the control protocol.
async fn check_d() {
    let (s, mut rx) = Session::spawn(spec("mode_switch", &["--permission-mode", "auto"])).await.unwrap();
    let step = |label: &'static str, perms: usize| println!("  {label}: permission requests = {perms}");
    // 1. auto: python one-liner, expect no prompt
    s.send_user("Use the Bash tool to run: python3 -c \"print(6*7)\" . Reply with the output only.").await.unwrap();
    let l = until_results(&s, &mut rx, 1, 90, true).await;
    step("auto/Bash python", l.iter().filter(|v| v.get("_perm").is_some()).count());
    let r = s.control(json!({"subtype": "set_permission_mode", "mode": "acceptEdits"})).await;
    println!("  set acceptEdits -> {:?}", r.map(|r| r["response"].clone()));
    s.send_user("Use the Write tool to create e.txt containing hi. Then use the Bash tool to run: python3 -c \"print(7*7)\" . Reply with the output only.").await.unwrap();
    let l = until_results(&s, &mut rx, 1, 90, true).await;
    for v in l.iter().filter_map(|v| v.get("_perm")) {
        println!("    prompted for {}", v["tool_name"]);
    }
    step("acceptEdits/Write+Bash", l.iter().filter(|v| v.get("_perm").is_some()).count());
    let r = s.control(json!({"subtype": "set_permission_mode", "mode": "manual"})).await;
    println!("  set manual -> {:?}", r.map(|r| r["response"].clone()));
    s.send_user("Use the Write tool to create m.txt containing hi. Reply 'ok'.").await.unwrap();
    let l = until_results(&s, &mut rx, 1, 90, true).await;
    for v in l.iter().filter_map(|v| v.get("_perm")) {
        println!("    prompted for {}", v["tool_name"]);
    }
    step("manual/Write", l.iter().filter(|v| v.get("_perm").is_some()).count());
    for v in &l {
        if v["type"] == "system" && v["subtype"] != "init" {
            println!("    system event {}", ty(v));
        }
    }
    println!("  pid alive: {}", s.child.id().is_some());
    println!("exit {:?}", s.finish(20).await);
}

/// E: flag passthrough. Settings with a WorktreeCreate hook, an MCP server,
/// an appended system prompt file, chosen id, name, partial messages, env.
async fn check_e() {
    let base = "/tmp/dovetail-spikes/driver";
    let id = uuid();
    let s = Spec {
        cwd: repo(),
        args: args(&[
            "--permission-mode", "auto",
            "--settings", &format!("{base}/cfg/settings.json"),
            "--mcp-config", &format!("{base}/cfg/mcp.json"),
            "--strict-mcp-config",
            "--append-system-prompt-file", &format!("{base}/cfg/append.md"),
            "--worktree", "spike-e",
            "--session-id", &id,
            "--name", "spike-e-name",
            "--include-partial-messages",
        ]),
        env: vec![
            ("CLAUDE_CODE_ENABLE_TODO_TOOLS".into(), "1".into()),
            ("DOVETAIL_SPIKE_MARK".into(), "mark-123".into()),
        ],
        log: out().join("flags.log.ndjson"),
    };
    let (s, mut rx) = Session::spawn(s).await.unwrap();
    s.send_user("What is the codeword? Reply with the codeword only.").await.unwrap();
    let l = until_results(&s, &mut rx, 1, 120, true).await;
    if let Some(init) = init_of(&l) {
        println!("  init.session_id = {} (chosen {id})", init["session_id"]);
        println!("  init.cwd = {}", init["cwd"]);
        println!("  init.permissionMode = {}", init["permissionMode"]);
        println!("  init.mcp_servers = {}", init["mcp_servers"]);
        let tools: Vec<_> = init["tools"].as_array().map(|t| t.iter().filter_map(Value::as_str).filter(|n| n.starts_with("Task") || n.starts_with("mcp__")).collect()).unwrap_or_default();
        println!("  init tools (Task*/mcp__*) = {tools:?}");
        let mut keys: Vec<_> = init.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        keys.sort();
        println!("  init keys = {keys:?}");
    }
    println!("  partial stream_events: {}", l.iter().filter(|v| v["type"] == "stream_event").count());
    summary(&l);
    println!("exit {:?}", s.finish(20).await);
    println!("id: {id}");
}

/// G: resume by id. Args: <id> <prompt> [cwd]. Records `resume`.
async fn check_g() {
    let id = std::env::args().nth(2).expect("id");
    let prompt = std::env::args().nth(3).expect("prompt");
    let name = std::env::args().nth(4).unwrap_or_else(|| "resume".into());
    let (s, mut rx) = Session::spawn(spec(&name, &["--permission-mode", "auto", "--resume", &id])).await.unwrap();
    s.send_user(&prompt).await.unwrap();
    let l = until_results(&s, &mut rx, 1, 120, true).await;
    if let Some(init) = init_of(&l) {
        println!("  init.session_id = {} cwd = {}", init["session_id"], init["cwd"]);
    }
    summary(&l);
    println!("exit {:?}", s.finish(20).await);
}

/// Background subagent: double result; task_* events.
async fn check_bg() {
    let (s, mut rx) = Session::spawn(Spec {
        env: vec![("CLAUDE_CODE_ENABLE_TODO_TOOLS".into(), "1".into())],
        ..spec("subagent_background", &["--permission-mode", "auto"])
    })
    .await
    .unwrap();
    s.send_user("Use the Agent tool with run_in_background: true to start one general-purpose subagent whose prompt is: 'Run `sleep 8` with the Bash tool, then reply bg-done.' Do not wait for it: after launching, reply 'launched' and end your turn.").await.unwrap();
    let l = until_results(&s, &mut rx, 2, 180, true).await;
    summary(&l);
    let results: Vec<_> = l.iter().filter(|v| v["type"] == "result").map(|v| v["result"].clone()).collect();
    println!("  results: {results:?}");
    let sub_lines = l.iter().filter(|v| v["parent_tool_use_id"].is_string()).count();
    println!("  lines with parent_tool_use_id set: {sub_lines}");
    // grace: anything after the second result?
    let extra = until_results(&s, &mut rx, 1, 10, true).await;
    println!("  after 2nd result (10 s): {} lines", extra.len());
    println!("exit {:?}", s.finish(30).await);
}

/// `--permission-prompt-tool stdio` re-check. Arg 2: `noflag` drops the flag
/// (relies on `--permission-prompts host`), `sub` asks via a subagent.
async fn check_prompt_tool() {
    let variant = std::env::args().nth(2).unwrap_or_default();
    let mut a: Vec<String> = base_args();
    if variant == "noflag" {
        a.retain(|x| x != "--permission-prompt-tool" && x != "stdio");
    }
    a.extend(["--permission-mode".into(), "manual".into()]);
    let (s, mut rx) = Session::spawn(Spec {
        cwd: repo(),
        args: a,
        env: vec![],
        log: out().join(format!("prompt_tool_{variant}.log.ndjson")),
    })
    .await
    .unwrap();
    let p = "/tmp/dovetail-spikes/driver/outside/one.txt";
    let prompt = if variant == "sub" {
        format!("Use the Agent tool (foreground, general-purpose) with the prompt: 'Read the file {p} with the Read tool and reply with its contents.' Then reply with what it returned.")
    } else {
        format!("Read the file {p} with the Read tool and reply with its contents.")
    };
    s.send_user(&prompt).await.unwrap();
    let l = until_results(&s, &mut rx, 1, 120, true).await;
    for v in l.iter().filter_map(|v| v.get("_perm")) {
        let mut keys: Vec<_> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        keys.sort();
        println!("  perm keys {keys:?} agent_id={}", v["agent_id"]);
    }
    summary(&l);
    println!("exit {:?}", s.finish(20).await);
}

/// To-do tools.
async fn check_todo() {
    let (s, mut rx) = Session::spawn(Spec {
        env: vec![("CLAUDE_CODE_ENABLE_TODO_TOOLS".into(), "1".into())],
        ..spec("todo_tools", &["--permission-mode", "auto"])
    })
    .await
    .unwrap();
    s.send_user("Use TaskCreate to create two to-do items with subjects 'alpha' and 'beta'. Then use TaskUpdate to mark 'alpha' completed. Reply 'done'.").await.unwrap();
    let l = until_results(&s, &mut rx, 1, 120, true).await;
    for v in &l {
        if let Some(c) = v["message"]["content"].as_array() {
            for b in c.iter().filter(|b| b["type"] == "tool_use") {
                println!("  tool_use {} {}", b["name"], b["input"]);
            }
        }
    }
    summary(&l);
    println!("exit {:?}", s.finish(20).await);
}
