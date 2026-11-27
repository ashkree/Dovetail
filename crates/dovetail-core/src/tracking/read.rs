//! The text of `tasks_list` and `tasks_get` (task #20): compact lines and markdown a model reads
//! without parsing JSON. Pure functions, so the wording is tested without a core.

use std::fmt::Write as _;

use dovetail_api::agents::agent::Agent;
use dovetail_api::tracking::{
    decision::{Decision, DecisionAuthor},
    task::{Bucket, PLevel, Task, TaskId, TaskStatus},
};
use dovetail_tools::task::{BucketFilter, ListTasks};
use serde::Serialize;

/// Longest title `tasks_list` shows before cutting it with `…`.
const TITLE_MAX: usize = 100;

/// The wire word of a snake_case enum value (`in_progress`, `needs_you`).
fn word(value: impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        other => format!("{other:?}"),
    }
}

fn bucket_word(bucket: Option<Bucket>) -> String {
    bucket.map_or_else(|| "unsorted".into(), word)
}

/// Now, Next, Unsorted, Later.
fn bucket_rank(bucket: Option<Bucket>) -> u8 {
    match bucket {
        Some(Bucket::Now) => 0,
        Some(Bucket::Next) => 1,
        None => 2,
        Some(Bucket::Later) => 3,
    }
}

fn ids(list: &[TaskId]) -> String {
    list.iter()
        .map(|id| format!("#{}", id.0))
        .collect::<Vec<_>>()
        .join(", ")
}

fn slices_note(slices: &[&Agent]) -> String {
    match slices {
        [] => String::new(),
        [one] => format!(" (slice {})", one.name),
        many => format!(
            " (slices {})",
            many.iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `tasks_list`'s answer: one line per matching task, Now, Next, Unsorted, Later, then by
/// priority (none last) and id. `slices` gives a task's live slices.
pub(crate) fn task_list<'a>(
    tasks: &[Task],
    filter: &ListTasks,
    slices: impl Fn(TaskId) -> Vec<&'a Agent>,
) -> String {
    let mut shown: Vec<&Task> = tasks
        .iter()
        .filter(|t| match filter.status {
            Some(status) => t.status == status,
            None => filter.include_archived || t.status != TaskStatus::Archived,
        })
        .filter(|t| {
            filter
                .bucket
                .is_none_or(|b: BucketFilter| b.matches(t.bucket))
        })
        .collect();
    if shown.is_empty() {
        return "No tasks match.".into();
    }
    shown.sort_by_key(|t| {
        (
            bucket_rank(t.bucket),
            t.p_level.map_or(u8::MAX, |p| p as u8),
            t.id,
        )
    });
    shown
        .into_iter()
        .map(|task| task_line(task, &slices(task.id)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One task as a line of `tasks_list` (also `tasks_update`'s answer):
/// `#3 [in_progress · now · P2] title (slice work)`.
pub(crate) fn task_line(task: &Task, slices: &[&Agent]) -> String {
    let mut tags = vec![word(task.status), bucket_word(task.bucket)];
    if let Some(p) = task.p_level {
        tags.push(word(p));
    }
    let title: String = if task.title.chars().count() > TITLE_MAX {
        let cut: String = task.title.chars().take(TITLE_MAX).collect();
        format!("{}…", cut.trim_end())
    } else {
        task.title.clone()
    };
    format!(
        "#{} [{}] {}{}",
        task.id.0,
        tags.join(" · "),
        title,
        slices_note(slices)
    )
}

fn author(by: &DecisionAuthor) -> String {
    match by {
        DecisionAuthor::User => "user".into(),
        DecisionAuthor::Agent(agent) => format!("agent {}", agent.0),
    }
}

/// `tasks_get`'s answer: the task in compact markdown, its live slices, and its decision log
/// oldest first, each entry marked with what superseded it.
pub(crate) fn task_detail(task: &Task, slices: &[&Agent], decisions: &[Decision]) -> String {
    let mut out = format!("# #{} {}\n\n", task.id.0, task.title);
    let priority = task
        .p_level
        .map_or_else(|| "none".into(), |p: PLevel| word(p));
    let _ = writeln!(
        out,
        "Status: {} · Bucket: {} · Priority: {}",
        word(task.status),
        bucket_word(task.bucket),
        priority
    );
    if let Some(of) = task.follow_up_of {
        let _ = writeln!(out, "Follow-up of: #{}", of.0);
    }
    if !task.related.is_empty() {
        let _ = writeln!(out, "Related: {}", ids(&task.related));
    }
    if !task.deps.is_empty() {
        let _ = writeln!(out, "Depends on: {}", ids(&task.deps));
    }
    if let Some(pr) = task.pr {
        let _ = writeln!(out, "PR: #{pr}");
    }
    if task.coordinator_created {
        out.push_str("New to the user: the coordinator created it or moved it into Now, and the user has not seen it yet.\n");
    }

    out.push_str("\n## Notes\n\n");
    if task.notes.trim().is_empty() {
        out.push_str("None.\n");
    } else {
        let _ = writeln!(out, "{}", task.notes.trim_end());
    }

    out.push_str("\n## Agents\n\n");
    if slices.is_empty() {
        out.push_str("No live slices.\n");
    }
    for slice in slices {
        let _ = write!(out, "- slice {}: {}", slice.name, word(slice.status));
        if !slice.status_line.is_empty() {
            let _ = write!(out, ", \"{}\"", slice.status_line);
        }
        out.push('\n');
    }

    out.push_str("\n## Decisions, oldest first\n\n");
    if decisions.is_empty() {
        out.push_str("None logged.\n");
    }
    for d in decisions {
        let _ = write!(out, "- D{} ({}): {}", d.id.0, author(&d.by), d.text);
        if let Some(later) = decisions.iter().find(|l| l.supersedes == Some(d.id)) {
            let _ = write!(out, " [superseded by D{}]", later.id.0);
        }
        out.push('\n');
        if let Some(old) = d.supersedes {
            let _ = writeln!(out, "  Supersedes: D{}", old.0);
        }
        if let Some(why) = d.why.as_deref().filter(|w| !w.trim().is_empty()) {
            let _ = writeln!(out, "  Why: {why}");
        }
        if !d.evidence.is_empty() {
            let ids: Vec<String> = d.evidence.iter().map(ToString::to_string).collect();
            let _ = writeln!(out, "  Evidence: {}", ids.join(", "));
        }
        for alt in &d.alternatives {
            match alt.why_not.as_deref().filter(|w| !w.trim().is_empty()) {
                Some(why_not) => {
                    let _ = writeln!(out, "  Rejected: {}, because {why_not}", alt.option);
                }
                None => {
                    let _ = writeln!(out, "  Rejected: {}", alt.option);
                }
            }
        }
    }
    out.truncate(out.trim_end().len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::agents::agent::AgentId;
    use dovetail_api::tracking::decision::{Alternative, DecisionId};

    fn task(id: i64, bucket: Option<Bucket>, p: Option<PLevel>, status: TaskStatus) -> Task {
        Task {
            id: TaskId(id),
            title: format!("task {id}"),
            status,
            p_level: p,
            bucket,
            order: 0,
            deps: vec![],
            notes: String::new(),
            pr: None,
            follow_up_of: None,
            coordinator_created: false,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }
    }

    fn sample() -> Vec<Task> {
        use Bucket::*;
        use PLevel::*;
        use TaskStatus::*;
        vec![
            task(1, Some(Later), Some(P0), Todo),
            task(2, None, None, Todo),
            task(3, Some(Now), Some(P2), InProgress),
            task(4, Some(Now), Some(P1), Todo),
            task(5, Some(Next), None, Done),
            task(6, Some(Next), Some(P3), Archived),
            task(7, None, Some(P1), Todo),
        ]
    }

    fn order(text: &str) -> Vec<&str> {
        text.lines()
            .map(|l| l.split_whitespace().next().unwrap_or(""))
            .collect()
    }

    #[test]
    fn list_orders_by_bucket_then_priority_then_id_and_hides_archived() {
        let text = task_list(&sample(), &ListTasks::default(), |_| vec![]);
        assert_eq!(order(&text), ["#4", "#3", "#5", "#7", "#2", "#1"]);
        assert!(text.contains("#4 [todo · now · P1] task 4"), "{text}");
        assert!(text.contains("#2 [todo · unsorted] task 2"), "{text}");
    }

    #[test]
    fn list_filters_by_bucket_status_and_archived() {
        let tasks = sample();
        let list = |f: ListTasks| task_list(&tasks, &f, |_| vec![]);
        let unsorted = list(ListTasks {
            bucket: Some(BucketFilter::Unsorted),
            ..ListTasks::default()
        });
        assert_eq!(order(&unsorted), ["#7", "#2"]);
        let todo = list(ListTasks {
            status: Some(TaskStatus::Todo),
            ..ListTasks::default()
        });
        assert_eq!(order(&todo), ["#4", "#7", "#2", "#1"]);
        let all = list(ListTasks {
            include_archived: true,
            ..ListTasks::default()
        });
        // In Next, #6 (P3) before #5 (no priority).
        assert_eq!(order(&all), ["#4", "#3", "#6", "#5", "#7", "#2", "#1"]);
        let archived = list(ListTasks {
            status: Some(TaskStatus::Archived),
            ..ListTasks::default()
        });
        assert_eq!(order(&archived), ["#6"]);
        let none = list(ListTasks {
            bucket: Some(BucketFilter::Later),
            status: Some(TaskStatus::Done),
            ..ListTasks::default()
        });
        assert_eq!(none, "No tasks match.");
    }

    #[test]
    fn detail_shows_the_decision_log_with_supersession() {
        let mut t = task(
            12,
            Some(Bucket::Now),
            Some(PLevel::P1),
            TaskStatus::InProgress,
        );
        t.notes = "Do the thing.".into();
        t.follow_up_of = Some(TaskId(9));
        t.related = vec![TaskId(3), TaskId(4)];
        let decisions = vec![
            Decision {
                id: DecisionId(1),
                task: TaskId(12),
                at_ms: 0,
                by: DecisionAuthor::User,
                text: "use A".into(),
                why: Some("simple".into()),
                alternatives: vec![Alternative {
                    option: "B".into(),
                    why_not: Some("slow".into()),
                }],
                supersedes: None,
                evidence: Vec::new(),
            },
            Decision {
                id: DecisionId(2),
                task: TaskId(12),
                at_ms: 1,
                by: DecisionAuthor::Agent(AgentId("slice-x".into())),
                text: "use C".into(),
                why: None,
                alternatives: vec![],
                supersedes: Some(DecisionId(1)),
                evidence: Vec::new(),
            },
        ];
        let text = task_detail(&t, &[], &decisions);
        for want in [
            "# #12 task 12",
            "Status: in_progress · Bucket: now · Priority: P1",
            "Follow-up of: #9",
            "Related: #3, #4",
            "Do the thing.",
            "No live slices.",
            "- D1 (user): use A [superseded by D2]",
            "  Why: simple",
            "  Rejected: B, because slow",
            "- D2 (agent slice-x): use C",
            "  Supersedes: D1",
        ] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        assert!(text.find("D1 (").unwrap() < text.find("D2 (").unwrap());
    }
}
