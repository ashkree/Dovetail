//! Tests for the `mcp` feature.

use crate::common::{Case, id, roundtrip, run};
use dovetail_api::mcp::{McpCommand, McpEvent, McpOutcome, McpRequest, McpServer, McpStatus};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn sample_reply() -> Event {
    Event::Mcp(McpEvent::Reply {
        agent: id("coordinator"),
        request: McpRequest::Status,
        outcome: McpOutcome::Status(vec![McpServer {
            name: "dovetail".into(),
            status: McpStatus::Connected,
            error: None,
            server_info: Some("dovetail 0.1".into()),
            transport: "stdio".into(),
            target: "dovetail mcp".into(),
            scope: "dynamic".into(),
            tools: vec![],
        }]),
    })
}

pub fn cases() -> Vec<Case> {
    vec![Case::no_op(
        "mcp",
        Snapshot::default(),
        vec![(1, sample_reply())],
    )]
}

#[test]
fn mcp_commands_and_events_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: sample_reply(),
    });
    roundtrip(&Command::Mcp(McpCommand::Request {
        agent: id("coordinator"),
        request: McpRequest::Reconnect {
            server: "dovetail".into(),
        },
    }));
}

#[test]
fn mcp_reply_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(&mut state, vec![sample_reply()]);
    assert_eq!(state, expected);
}
