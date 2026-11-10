//! The mock's two maps, for the map views (M15): a structure map of Dovetail itself, and a
//! swimlane map of a permission request whose lanes link that structure map's nodes.
//!
//! Written with the API types, so the mock (and the TUI tests that use it) stay clear of
//! `dovetail-maps`; `tests/mock.rs` checks they pass `draw_map` validation.

use dovetail_api::map::{
    Connection, Flow, FlowStep, Group, Lane, LaneId, Map, MapContent, MapId, MapKind, MapSummary,
    Node, NodeId, NodeKind,
};
use dovetail_api::{AgentId, TaskId};

use crate::world::{COORDINATOR, agent_id};

fn node(id: &str, kind: NodeKind, label: &str, description: &str) -> Node {
    Node {
        id: NodeId(id.into()),
        kind,
        label: label.into(),
        description: description.into(),
        file: None,
        pseudocode: None,
    }
}

fn ids(list: &[&str]) -> Vec<NodeId> {
    list.iter().map(|id| NodeId((*id).into())).collect()
}

fn connection(from: &str, to: &str, label: &str) -> Connection {
    Connection {
        from: NodeId(from.into()),
        to: NodeId(to.into()),
        label: label.into(),
    }
}

/// A flow step; `connection` on structure maps, `lane` and `arrow` on swimlanes.
fn step(
    node: &str,
    connection: Option<usize>,
    lane: Option<&str>,
    arrow: Option<&str>,
    note: &str,
) -> FlowStep {
    FlowStep {
        node: NodeId(node.into()),
        connection,
        arrow: arrow.map(Into::into),
        note: note.into(),
        lane: lane.map(|l| LaneId(l.into())),
    }
}

fn overview() -> MapContent {
    let mut keymap = node(
        "keymap",
        NodeKind::File,
        "keymap.rs",
        "One table from mode, buffer and keys to an Action.",
    );
    keymap.file = Some("crates/dovetail-tui/src/keymap.rs".into());
    let mut apply = node(
        "apply",
        NodeKind::Logic,
        "apply()",
        "The reducer: the only way client state changes.",
    );
    apply.pseudocode = Some(
        "if seq != state.seq + 1: return Gap\nfold the event into the snapshot\nstate.seq = seq"
            .into(),
    );
    MapContent {
        title: "Dovetail at a glance".into(),
        kind: MapKind::Structure,
        nodes: vec![
            node(
                "user",
                NodeKind::Actor,
                "You",
                "Works with the coordinator.",
            ),
            node(
                "tui",
                NodeKind::Part,
                "dovetail-tui",
                "Buffers, regions, keymap and theme.",
            ),
            keymap,
            node(
                "api",
                NodeKind::Part,
                "dovetail-api",
                "Plain data: snapshot, events, commands.",
            ),
            apply,
            node(
                "core",
                NodeKind::Part,
                "dovetail-core",
                "Sessions, storage, worktrees, tools.",
            ),
            node(
                "db",
                NodeKind::Data,
                "state.db",
                "Tasks, agents, inbox and maps, per project.",
            ),
            node(
                "driver",
                NodeKind::Part,
                "dovetail-driver",
                "Speaks Claude Code's stream protocol.",
            ),
            node(
                "claude",
                NodeKind::External,
                "Claude Code",
                "Each session runs headless.",
            ),
            node(
                "mcp",
                NodeKind::Endpoint,
                "dovetail mcp",
                "The tools Dovetail serves to sessions.",
            ),
        ],
        groups: vec![
            Group {
                parent: NodeId("tui".into()),
                children: ids(&["keymap"]),
            },
            Group {
                parent: NodeId("api".into()),
                children: ids(&["apply"]),
            },
        ],
        connections: vec![
            connection("user", "tui", "types keys"),
            connection("tui", "core", "sends commands"),
            connection("core", "tui", "sends events"),
            connection("core", "driver", "starts sessions"),
            connection("driver", "claude", "streams NDJSON"),
            connection("core", "db", "stores"),
            connection("claude", "mcp", "calls tools"),
            connection("mcp", "core", "forwards over the socket"),
        ],
        flows: vec![Flow {
            name: "A prompt's trip".into(),
            steps: vec![
                step(
                    "user",
                    None,
                    None,
                    None,
                    "You type a prompt and press Enter.",
                ),
                step(
                    "tui",
                    Some(0),
                    None,
                    None,
                    "The chat bar sends Command::Prompt.",
                ),
                step(
                    "core",
                    Some(1),
                    None,
                    None,
                    "The core stores it and hands it on.",
                ),
                step(
                    "driver",
                    Some(3),
                    None,
                    None,
                    "The driver writes a user message.",
                ),
                step(
                    "claude",
                    Some(4),
                    None,
                    None,
                    "Claude Code streams its reply back.",
                ),
            ],
        }],
        lanes: vec![],
    }
}

fn permission() -> MapContent {
    let lane = |id: &str, label: &str, node: &str| Lane {
        id: LaneId(id.into()),
        label: label.into(),
        node: Some(NodeId(node.into())),
    };
    let mut raise = node(
        "raise",
        NodeKind::Logic,
        "raise",
        "The core files the request.",
    );
    raise.pseudocode = Some("add an inbox item\nwait for the user's answer".into());
    MapContent {
        title: "Answering a permission request".into(),
        kind: MapKind::Swimlane,
        nodes: vec![
            node(
                "ask",
                NodeKind::External,
                "can_use_tool",
                "The session asks to run Bash.",
            ),
            raise,
            node(
                "inbox",
                NodeKind::Endpoint,
                "Needs you",
                "The request shows in the inbox.",
            ),
            node("decide", NodeKind::Actor, "You", "Press y to allow once."),
            node("allowed", NodeKind::Outcome, "Allowed", "The tool runs."),
        ],
        groups: vec![],
        connections: vec![],
        flows: vec![Flow {
            name: "Allow once".into(),
            steps: vec![
                step(
                    "ask",
                    None,
                    Some("session"),
                    None,
                    "Claude Code sends a control request.",
                ),
                step(
                    "raise",
                    None,
                    Some("core"),
                    Some("control request"),
                    "Filed as an inbox item.",
                ),
                step(
                    "inbox",
                    None,
                    Some("ui"),
                    Some("InboxUpserted"),
                    "Shown under Needs you.",
                ),
                step("decide", None, Some("you"), Some("shows"), "You press y."),
                step(
                    "allowed",
                    None,
                    Some("session"),
                    Some("allow"),
                    "The answer goes back.",
                ),
            ],
        }],
        lanes: vec![
            lane("session", "Claude Code", "claude"),
            lane("core", "Core", "core"),
            lane("ui", "TUI", "tui"),
            lane("you", "You", "user"),
        ],
    }
}

/// The seeded maps with their shelf entries, most recently drawn first.
pub(crate) fn seeded(now_ms: i64) -> Vec<(MapSummary, Map)> {
    let entry = |id: &str,
                 content: MapContent,
                 task: Option<TaskId>,
                 by: AgentId,
                 version: u32,
                 ago_ms: i64| {
        let summary = MapSummary {
            id: MapId(id.into()),
            title: content.title.clone(),
            kind: content.kind,
            task,
            drawn_by: Some(by),
            version,
            created_ms: now_ms - 3_600_000,
            updated_ms: now_ms - ago_ms,
        };
        let map = Map {
            id: MapId(id.into()),
            version,
            content,
        };
        (summary, map)
    };
    vec![
        entry(
            "permission-round-trip",
            permission(),
            Some(TaskId(1)),
            agent_id("tree-view"),
            1,
            120_000,
        ),
        entry(
            "dovetail-overview",
            overview(),
            None,
            agent_id(COORDINATOR),
            2,
            900_000,
        ),
    ]
}
