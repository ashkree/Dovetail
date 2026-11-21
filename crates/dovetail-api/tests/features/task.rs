//! Tests for the `task` feature.

use crate::common::{Case, effort, rich_task, roundtrip, run, task};
use dovetail_api::effort::{
    Criterion, CriterionId, CriterionState, EffortEvent, EffortId, UNSORTED_EFFORT,
};
use dovetail_api::task::{
    Bucket, NewTask, PLevel, TaskChange, TaskCommand, TaskEvent, TaskId, TaskStatus,
};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    let mut claimed = effort(2, "guest-checkout");
    claimed.criteria.push(Criterion {
        id: CriterionId(3),
        effort: EffortId(2),
        text: "pay as guest".into(),
        state: CriterionState::Claimed,
        evidence: Some("PR #41".into()),
    });
    let mut moved = task(1, "form");
    moved.effort = EffortId(2);
    moved.criterion = Some(CriterionId(3));
    moved.status = TaskStatus::Review;
    vec![Case::new(
        "tasks_and_efforts",
        Snapshot::default(),
        vec![
            (
                1,
                Event::Effort(EffortEvent::Upserted(effort(1, "unsorted"))),
            ),
            (
                2,
                Event::Effort(EffortEvent::Upserted(effort(2, "guest-pay"))),
            ),
            (3, Event::Task(TaskEvent::Upserted(task(1, "form")))),
            (4, Event::Task(TaskEvent::Upserted(task(2, "email")))),
            (5, Event::Effort(EffortEvent::Upserted(claimed))),
            (6, Event::Task(TaskEvent::Upserted(moved))),
            (7, Event::Task(TaskEvent::Removed(TaskId(2)))),
        ],
    )]
}

#[test]
fn task_commands_and_events_roundtrip() {
    for (n, event) in [
        Event::Task(TaskEvent::Upserted(rich_task())),
        Event::Task(TaskEvent::Removed(TaskId(3))),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Task(TaskCommand::Create(NewTask {
            title: "x".into(),
            p_level: Some(PLevel::P2),
            bucket: None,
            notes: String::new(),
            follow_up_of: Some(TaskId(1)),
            pr: Some(1231),
            related: vec![TaskId(2)],
            effort: Default::default(),
            criterion: None,
        })),
        Command::Task(TaskCommand::Update {
            id: TaskId(3),
            patch: vec![
                TaskChange::Title("y".into()),
                TaskChange::Status(TaskStatus::Done),
                TaskChange::PLevel(None),
                TaskChange::PLevel(Some(PLevel::P3)),
                TaskChange::Bucket(None),
                TaskChange::Bucket(Some(Bucket::Next)),
                TaskChange::Notes("n".into()),
                TaskChange::Deps(vec![TaskId(1)]),
                TaskChange::Pr(None),
                TaskChange::Pr(Some(7)),
                TaskChange::Seen,
                TaskChange::Unseen,
                TaskChange::Relate(TaskId(2)),
                TaskChange::Unrelate(TaskId(2)),
            ],
        }),
        Command::Task(TaskCommand::Move {
            id: TaskId(3),
            effort: EffortId(7),
            criterion: Some(CriterionId(3)),
        }),
        Command::Task(TaskCommand::Move {
            id: TaskId(3),
            effort: UNSORTED_EFFORT,
            criterion: None,
        }),
        Command::Task(TaskCommand::Archive { id: TaskId(3) }),
        Command::Task(TaskCommand::Reorder {
            id: TaskId(3),
            delta: -1,
        }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn task_upsert_inserts_then_replaces_by_id() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Task(TaskEvent::Upserted(task(1, "first"))),
            Event::Task(TaskEvent::Upserted(task(2, "second"))),
            Event::Task(TaskEvent::Upserted(task(1, "renamed"))),
        ],
    );
    let titles: Vec<_> = state.tasks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["renamed", "second"]);
}

#[test]
fn task_removed_drops_only_that_task() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Task(TaskEvent::Upserted(task(1, "a"))),
            Event::Task(TaskEvent::Upserted(task(2, "b"))),
            Event::Task(TaskEvent::Removed(TaskId(1))),
        ],
    );
    assert_eq!(state.tasks, vec![task(2, "b")]);
}

#[test]
fn a_moved_task_arrives_as_a_task_upsert() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![Event::Task(TaskEvent::Upserted(task(1, "Guest form")))],
    );
    assert_eq!(state.tasks[0].effort, UNSORTED_EFFORT);

    let mut moved = task(1, "Guest form");
    moved.effort = EffortId(2);
    moved.criterion = Some(CriterionId(5));
    run(&mut state, vec![Event::Task(TaskEvent::Upserted(moved))]);
    assert_eq!(state.tasks.len(), 1);
    assert_eq!(state.tasks[0].effort, EffortId(2));
    assert_eq!(state.tasks[0].criterion, Some(CriterionId(5)));
}
