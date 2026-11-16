//! Spike (ADR 0023): does ts-rs export the serde shapes `dovetail-api` uses, as serde writes them?
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Serialize, Deserialize, TS, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct TaskId(pub i64);

#[derive(Serialize, Deserialize, TS, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct AgentId(pub String);

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus { Todo, InProgress }

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    Permission { tool: String, input: serde_json::Value, reason: Option<String> },
    Question { text: String },
}

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    TaskRemoved(TaskId),
    AssistantDelta { agent: AgentId, text: String },
    LendTerminal,
}

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Tagged { ToolResult { text: String }, Other }

#[derive(Serialize, Deserialize, TS)]
pub struct Snapshot {
    pub seq: u64,
    pub now_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub conversations: BTreeMap<AgentId, Vec<String>>,
    pub repo_path: PathBuf,
    pub status: TaskStatus,
    pub cost: f64,
}

#[derive(Serialize, Deserialize, TS)]
pub enum ImageType {
    #[serde(rename = "image/png")]
    Png,
}

fn main() {
    let cfg = ts_rs::Config::default();
    for decl in [
        TaskId::decl(&cfg), AgentId::decl(&cfg), TaskStatus::decl(&cfg), InboxKind::decl(&cfg),
        Event::decl(&cfg), Tagged::decl(&cfg), Snapshot::decl(&cfg), ImageType::decl(&cfg),
    ] {
        println!("{decl}");
    }
    let ev = serde_json::to_string(&Event::AssistantDelta { agent: AgentId("a".into()), text: "x".into() }).unwrap();
    println!("// serde: {ev} | {} | {}", serde_json::to_string(&Event::LendTerminal).unwrap(), serde_json::to_string(&Event::TaskRemoved(TaskId(3))).unwrap());
}
