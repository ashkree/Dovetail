//! Tests for the `worktree` feature.

use crate::common::{AgentStatus, Case, agent, id, roundtrip, run, worktree_infos};
use dovetail_api::agent::AgentEvent;
use dovetail_api::worktree::{
    Changes, PrLookup, WorktreeCommand, WorktreeEvent, WorktreeInfo, WorktreeSetup,
};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    let events = worktree_infos()
        .into_iter()
        .enumerate()
        .map(|(i, w)| (i as u64 + 1, Event::Worktree(WorktreeEvent::Upserted(w))))
        .collect();
    vec![Case::new("worktree", Snapshot::default(), events)]
}

#[test]
fn worktree_commands_and_events_roundtrip() {
    for (n, info) in worktree_infos().into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event: Event::Worktree(WorktreeEvent::Upserted(info)),
        });
    }

    for command in [
        Command::Worktree(WorktreeCommand::Remove {
            agent: id("slice-a"),
            confirmed: true,
        }),
        Command::Worktree(WorktreeCommand::Refresh),
    ] {
        roundtrip(&command);
    }
}

#[test]
fn worktree_info_is_replaced_per_slice_and_goes_with_it() {
    let info = |agent: &str, staged: u32| WorktreeInfo {
        agent: id(agent),
        path: format!("/w/{agent}").into(),
        branch: format!("dovetail/{agent}"),
        base: "main".into(),
        sync: None,
        changes: Some(Changes {
            staged,
            ..Changes::default()
        }),
        unpushed: None,
        pr: PrLookup::None,
        setup: WorktreeSetup::None,
        checked_ms: 1_000,
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Agent(AgentEvent::Upserted(agent("s1", AgentStatus::Idle))),
            Event::Agent(AgentEvent::Upserted(agent("s2", AgentStatus::Idle))),
            Event::Worktree(WorktreeEvent::Upserted(info("s1", 1))),
            Event::Worktree(WorktreeEvent::Upserted(info("s2", 1))),
            Event::Worktree(WorktreeEvent::Upserted(info("s1", 4))),
        ],
    );
    assert_eq!(state.worktrees, vec![info("s1", 4), info("s2", 1)]);
    run(
        &mut state,
        vec![Event::Agent(AgentEvent::Removed(id("s1")))],
    );
    assert_eq!(state.worktrees, vec![info("s2", 1)]);
}
