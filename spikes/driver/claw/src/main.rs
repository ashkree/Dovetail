//! S1: rusty_claw 0.1.0. Two concurrent sessions:
//! s1 plain turn (A, H); s2 manual mode, two parallel permission requests,
//! the handler holds the first 60 s and answers the second at once (C).
use rusty_claw::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio_stream::StreamExt;

const REPO: &str = "/tmp/dovetail-spikes/driver/repo";

struct Holder {
    n: AtomicUsize,
    t: Instant,
}

#[async_trait::async_trait]
impl CanUseToolHandler for Holder {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
    ) -> Result<rusty_claw::permissions::PermissionDecision, ClawError> {
        let k = self.n.fetch_add(1, Ordering::SeqCst) + 1;
        println!("s2: [{:.1}s] handler call #{k}: {tool_name} {tool_input}", self.t.elapsed().as_secs_f64());
        if k == 1 {
            tokio::time::sleep(Duration::from_secs(60)).await;
            println!("s2: [{:.1}s] releasing #1", self.t.elapsed().as_secs_f64());
        }
        Ok(rusty_claw::permissions::PermissionDecision::Allow { updated_input: None })
    }
}

async fn run(name: &str, opts: ClaudeAgentOptions, prompt: &str, t: Instant) {
    let mut c = match ClaudeClient::new(opts) {
        Ok(c) => c,
        Err(e) => return println!("{name}: new error {e}"),
    };
    if let Err(e) = c.connect().await {
        return println!("{name}: connect error {e}");
    }
    let mut stream = match c.send_message(prompt).await {
        Ok(s) => s,
        Err(e) => return println!("{name}: send error {e}"),
    };
    let mut kinds = vec![];
    let fut = async {
        while let Some(m) = stream.next().await {
            match m {
                Ok(Message::Result(r)) => {
                    println!("{name}: [{:.1}s] result {r:?}", t.elapsed().as_secs_f64());
                    break;
                }
                Ok(Message::User(u)) => {
                    println!("{name}: [{:.1}s] user/tool_result {}", t.elapsed().as_secs_f64(), format!("{u:?}").chars().take(160).collect::<String>());
                    kinds.push("user".to_string())
                }
                Ok(m) => kinds.push(format!("{m:?}").chars().take(24).collect()),
                Err(e) => kinds.push(format!("ERR {e}")),
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(150), fut).await.is_err() {
        println!("{name}: [{:.1}s] TIMEOUT", t.elapsed().as_secs_f64());
    }
    println!("{name}: messages {kinds:?}");
    let _ = c.close().await;
}

#[tokio::main]
async fn main() {
    let t = Instant::now();
    let o1 = ClaudeAgentOptions::builder().cwd(REPO).build();
    let o2 = ClaudeAgentOptions::builder()
        .cwd(REPO)
        .permission_mode(PermissionMode::Default)
        .extra_arg("permission-prompt-tool", Some("stdio".into()))
        .permission_handler(Holder { n: AtomicUsize::new(0), t })
        .build();
    let _ = Arc::new(());
    tokio::join!(
        run("s1", o1, "Reply with exactly the word: alpha", t),
        run(
            "s2",
            o2,
            "In a single message, call the Read tool twice in parallel: /tmp/dovetail-spikes/driver/outside/one.txt and /tmp/dovetail-spikes/driver/outside/two.txt. Then reply with both contents on one line.",
            t
        )
    );
}
