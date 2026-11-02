//! S1: claude-agent-sdk-rs 0.6.4. Two concurrent sessions:
//! s1 plain turn (A, H); s2 manual mode with a `can_use_tool` callback (C).
use claude_agent_sdk_rs::{
    ClaudeAgentOptions, ClaudeClient, Message, PermissionMode, PermissionResult,
    PermissionResultAllow,
};
use futures::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const REPO: &str = "/tmp/dovetail-spikes/driver/repo";

async fn run(name: &str, opts: ClaudeAgentOptions, prompt: &str) {
    let t = Instant::now();
    let mut c = ClaudeClient::new(opts);
    if let Err(e) = c.connect().await {
        println!("{name}: connect error {e}");
        return;
    }
    c.query(prompt).await.expect("query");
    let mut ok = 0;
    let mut errs = vec![];
    let mut sids = std::collections::BTreeSet::new();
    let fut = async {
        let mut s = c.receive_response();
        while let Some(m) = s.next().await {
            match m {
                Ok(Message::Result(r)) => {
                    sids.insert(r.session_id.clone());
                    println!("{name}: [{:.1}s] result {:?}", t.elapsed().as_secs_f64(), r.result);
                    ok += 1;
                }
                Ok(Message::Assistant(a)) => {
                    if let Some(s) = a.session_id.clone() {
                        sids.insert(s);
                    }
                    ok += 1
                }
                Ok(_) => ok += 1,
                Err(e) => errs.push(e.to_string()),
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(60), fut).await.is_err() {
        println!("{name}: [{:.1}s] TIMEOUT waiting for result", t.elapsed().as_secs_f64());
    }
    println!("{name}: parsed={ok} parse_errors={errs:?} session_ids={sids:?}");
    let _ = c.disconnect().await;
}

#[tokio::main]
async fn main() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let cb: claude_agent_sdk_rs::CanUseToolCallback = Arc::new(move |tool, input, _ctx| {
        c2.fetch_add(1, Ordering::SeqCst);
        println!("s2: can_use_tool callback: {tool} {input}");
        Box::pin(async move {
            PermissionResult::Allow(PermissionResultAllow { updated_input: Some(input), ..Default::default() })
        })
    });
    let o1 = ClaudeAgentOptions::builder().cwd(REPO).build();
    let o2 = ClaudeAgentOptions::builder()
        .cwd(REPO)
        .permission_mode(PermissionMode::Default)
        .permission_prompt_tool_name("stdio")
        .can_use_tool(cb)
        .build();
    tokio::join!(
        run("s1", o1, "Reply with exactly the word: alpha"),
        run(
            "s2",
            o2,
            "Read the file /tmp/dovetail-spikes/driver/outside/one.txt with the Read tool and reply with its contents."
        )
    );
    println!("can_use_tool callback invocations: {}", calls.load(Ordering::SeqCst));
}
