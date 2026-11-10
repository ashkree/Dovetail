//! What the picker lists and previews: Dovetail's agents for the switcher (`<leader>ss`), and
//! tasks, agents and inbox items for the everything picker (`<leader><leader>`).

use std::cmp::Reverse;
use std::collections::HashSet;

use dovetail_api::{
    Agent, AgentId, AgentKind, AgentStatus, InboxItem, InboxKind, SlashCommand, Snapshot,
    TaskStatus,
};
use jiff::tz::TimeZone;
use ratatui::text::{Line, Span};

use crate::buffers::task_detail;
use crate::buffers::{bucket_label, status_glyph, status_word};
use crate::fmt;
use crate::picker::{PickerItem, PickerKey};
use crate::render::transcript::{self, Cache};
use crate::theme::Theme;

/// Agents in switcher order: those that need you first, then the most recently active, each
/// session followed by its subagents. Returns the agent and its indent.
pub fn agent_order(snap: &Snapshot) -> Vec<(&Agent, usize)> {
    let waiting: HashSet<&AgentId> = snap.inbox.iter().map(|i| &i.agent).collect();
    let needs = |a: &Agent| waiting.contains(&a.id) || a.status == AgentStatus::NeedsYou;
    let known = |id: &AgentId| snap.agents.iter().any(|a| &a.id == id);
    let mut tops: Vec<&Agent> = snap
        .agents
        .iter()
        .filter(|a| a.parent.as_ref().is_none_or(|p| !known(p)))
        .collect();
    tops.sort_by_key(|a| (!needs(a), Reverse(a.last_activity_ms)));
    let mut out = Vec::new();
    for top in tops {
        push_with_children(snap, top, 0, &mut out);
    }
    out
}

fn push_with_children<'a>(
    snap: &'a Snapshot,
    agent: &'a Agent,
    depth: usize,
    out: &mut Vec<(&'a Agent, usize)>,
) {
    out.push((agent, depth));
    let mut children: Vec<&Agent> = snap
        .agents
        .iter()
        .filter(|a| a.parent.as_ref() == Some(&agent.id))
        .collect();
    children.sort_by_key(|a| Reverse(a.last_activity_ms));
    for child in children {
        push_with_children(snap, child, depth + 1, out);
    }
}

/// The branch an agent works on: its own, or its parent session's.
fn branch_of<'a>(snap: &'a Snapshot, agent: &'a Agent) -> Option<&'a str> {
    agent
        .worktree
        .as_ref()
        .or_else(|| {
            let parent = agent.parent.as_ref()?;
            snap.agents
                .iter()
                .find(|a| &a.id == parent)?
                .worktree
                .as_ref()
        })
        .map(|w| w.branch.as_str())
}

/// Rows for the agent switcher.
pub fn agent_items(snap: &Snapshot, now_ms: i64, theme: &Theme) -> Vec<PickerItem> {
    agent_order(snap)
        .into_iter()
        .map(|(agent, depth)| {
            let (glyph, color) = status_glyph(agent.status, theme);
            let branch = branch_of(snap, agent).unwrap_or("");
            let task = agent
                .task
                .and_then(|id| snap.tasks.iter().find(|t| t.id == id))
                .map_or("", |t| t.title.as_str());
            let age = fmt::ago(now_ms, agent.last_activity_ms);
            PickerItem {
                key: PickerKey::Agent(agent.id.clone()),
                label: agent.name.clone(),
                haystack: format!("{} {branch} {task}", agent.name),
                detail: if branch.is_empty() {
                    age
                } else {
                    format!("{branch}  {age}")
                },
                glyph,
                color,
                depth,
                section: "Agents",
            }
        })
        .collect()
}

/// Slash command and skill rows, in the session's order.
pub fn command_items(commands: &[SlashCommand], theme: &Theme) -> Vec<PickerItem> {
    commands
        .iter()
        .map(|c| PickerItem {
            key: PickerKey::Command(c.name.clone()),
            label: format!("/{}", c.name),
            haystack: format!("{} {}", c.name, c.description),
            detail: c.argument_hint.clone(),
            glyph: "/",
            color: theme.dim,
            depth: 0,
            section: "Commands and skills",
        })
        .collect()
}

/// Task rows, in the order the tree shows them (Done last).
pub fn task_items(snap: &Snapshot, theme: &Theme) -> Vec<PickerItem> {
    let mut tasks: Vec<_> = snap
        .tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Archived)
        .collect();
    tasks.sort_by_key(|t| {
        (
            t.status == TaskStatus::Done,
            t.bucket.is_none(),
            t.bucket,
            t.order,
            t.id,
        )
    });
    tasks
        .into_iter()
        .map(|t| {
            let (glyph, color) = match t.status {
                TaskStatus::Done => ("✓", theme.ok),
                TaskStatus::InProgress => ("◐", theme.working),
                _ => ("○", theme.dim),
            };
            let p = t.p_level.map_or("", crate::buffers::p_label);
            PickerItem {
                key: PickerKey::Task(t.id),
                label: t.title.clone(),
                haystack: t.title.clone(),
                detail: format!("{} {p}", bucket_label(t.bucket)).trim().to_string(),
                glyph,
                color,
                depth: 0,
                section: "Tasks",
            }
        })
        .collect()
}

/// Rows for relating `task` to another: every other task, with the ones already related marked.
pub fn relate_items(snap: &Snapshot, theme: &Theme, task: dovetail_api::TaskId) -> Vec<PickerItem> {
    let related = snap
        .tasks
        .iter()
        .find(|t| t.id == task)
        .map(|t| t.related.clone())
        .unwrap_or_default();
    task_items(snap, theme)
        .into_iter()
        .filter(|item| item.key != PickerKey::Task(task))
        .map(|mut item| {
            if let PickerKey::Task(id) = item.key
                && related.contains(&id)
            {
                item.glyph = "↔";
                item.detail = "related · ⏎ to unlink".into();
            }
            item
        })
        .collect()
}

/// Rows for the everything picker: tasks, then agents, then inbox items.
pub fn all_items(snap: &Snapshot, now_ms: i64, theme: &Theme) -> Vec<PickerItem> {
    let mut items = task_items(snap, theme);
    items.extend(agent_items(snap, now_ms, theme));
    let mut inbox: Vec<&InboxItem> = snap.inbox.iter().collect();
    inbox.sort_by_key(|i| Reverse((i.at_ms, i.id)));
    items.extend(inbox.into_iter().map(|item| {
        let name = snap
            .agents
            .iter()
            .find(|a| a.id == item.agent)
            .map_or(item.agent.0.as_str(), |a| a.name.as_str());
        let (glyph, color, label, body) = match &item.kind {
            InboxKind::Permission { tool, input, .. } => (
                "⚠",
                theme.needs_you,
                tool.clone(),
                crate::buffers::input_summary(snap, tool, input),
            ),
            InboxKind::Question { text, .. } => {
                ("?", theme.needs_you, "Question".into(), text.clone())
            }
            InboxKind::FinishedTurn { preview } => {
                ("✓", theme.ok, "Finished turn".into(), preview.clone())
            }
            InboxKind::Error { text } => ("✗", theme.error, "Error".into(), text.clone()),
        };
        PickerItem {
            key: PickerKey::Inbox(item.id),
            label: format!("{name} {label}"),
            haystack: format!("{name} {label} {body}"),
            detail: fmt::ago(now_ms, item.at_ms),
            glyph,
            color,
            depth: 0,
            section: "Inbox",
        }
    }));
    items
}

/// The preview for a row: its heading and lines. An agent shows its latest turns last.
pub fn preview(
    snap: &Snapshot,
    key: &PickerKey,
    width: usize,
    theme: &Theme,
    tz: &TimeZone,
) -> (&'static str, Vec<Line<'static>>) {
    match key {
        PickerKey::Agent(id) => ("Preview", agent_preview(snap, id, width, theme, tz)),
        PickerKey::Task(id) => (
            "Task",
            snap.tasks
                .iter()
                .find(|t| t.id == *id)
                .map(|t| task_detail::lines(snap, t, width, theme, tz))
                .unwrap_or_default(),
        ),
        PickerKey::Command(name) => (
            "Command",
            snap.commands
                .values()
                .flatten()
                .find(|c| &c.name == name)
                .map(|c| command_preview(c, width, theme))
                .unwrap_or_default(),
        ),
        PickerKey::Inbox(id) => (
            "Inbox item",
            snap.inbox
                .iter()
                .find(|i| i.id == *id)
                .map(|i| inbox_preview(snap, i, width, theme))
                .unwrap_or_default(),
        ),
    }
}

fn agent_preview(
    snap: &Snapshot,
    id: &AgentId,
    width: usize,
    theme: &Theme,
    tz: &TimeZone,
) -> Vec<Line<'static>> {
    let Some(agent) = snap.agents.iter().find(|a| &a.id == id) else {
        return Vec::new();
    };
    let (glyph, color) = status_glyph(agent.status, theme);
    let mut out = vec![Line::from(vec![
        Span::styled(glyph, theme.fg(color)),
        Span::raw(" "),
        Span::styled(agent.name.clone(), theme.accent()),
    ])];
    let kind = match agent.kind {
        AgentKind::Coordinator => "coordinator",
        AgentKind::Slice => "slice",
        AgentKind::SideChat => "side chat",
        AgentKind::Subagent { background: true } => "background subagent",
        AgentKind::Subagent { background: false } => "subagent",
    };
    let mut facts = vec![kind.to_string()];
    if let Some(branch) = branch_of(snap, agent) {
        facts.push(branch.to_string());
    }
    if let Some(mb) = agent.memory_mb {
        facts.push(fmt::memory(mb));
    }
    facts.push(status_word(agent.status).to_string());
    if agent.subagent_tokens > 0 {
        facts.push(format!(
            "{} tokens in subagents",
            fmt::tokens(agent.subagent_tokens)
        ));
    }
    out.push(Line::from(Span::styled(facts.join(" · "), theme.dim())));
    out.push(Line::default());

    if let Some(progress) = &agent.progress {
        out.push(Line::from(Span::styled(
            progress.description.clone(),
            theme.body(),
        )));
        let mut facts = vec![format!("{} tool uses", progress.tool_uses)];
        if let Some(t) = &progress.last_tool {
            facts.push(format!("last {t}"));
        }
        if progress.total_tokens > 0 {
            facts.push(format!("{} tokens", fmt::tokens(progress.total_tokens)));
        }
        if progress.duration_ms > 0 {
            facts.push(fmt::duration(progress.duration_ms));
        }
        out.push(Line::from(Span::styled(facts.join(" · "), theme.dim())));
        if let Some(file) = &progress.output_file {
            out.push(Line::from(Span::styled(
                format!("output {}", file.display()),
                theme.dim(),
            )));
        }
        if let Some(summary) = &progress.summary {
            out.push(Line::default());
            for line in fmt::wrap(summary, width) {
                out.push(Line::from(Span::styled(line, theme.body())));
            }
        }
        return out;
    }
    if let Some(tail) = snap.conversations.get(id) {
        let from = tail.entries.len().saturating_sub(6);
        let folded = HashSet::new();
        let expanded = HashSet::new();
        let params = transcript::Params {
            width,
            theme,
            tz,
            agent: &agent.name,
            streaming: None,
            folded: &folded,
            expanded: &expanded,
        };
        out.extend(transcript::build(&tail.entries[from..], &params, &mut Cache::default()).lines);
    }
    out
}

fn command_preview(command: &SlashCommand, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut head = format!("/{}", command.name);
    if !command.argument_hint.is_empty() {
        head = format!("{head} {}", command.argument_hint);
    }
    let mut out = vec![
        Line::from(Span::styled(head, theme.accent())),
        Line::default(),
    ];
    for line in fmt::wrap(&command.description, width) {
        out.push(Line::from(Span::styled(line, theme.body())));
    }
    out
}

fn inbox_preview(
    snap: &Snapshot,
    item: &InboxItem,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let name = snap
        .agents
        .iter()
        .find(|a| a.id == item.agent)
        .map_or(item.agent.0.as_str(), |a| a.name.as_str());
    let (head, body) = match &item.kind {
        InboxKind::Permission { tool, input, .. } => (
            format!("{name} asks to use {tool}"),
            crate::buffers::input_summary(snap, tool, input),
        ),
        InboxKind::Question { text } => (format!("{name} asks"), text.clone()),
        InboxKind::FinishedTurn { preview } => (format!("{name} finished a turn"), preview.clone()),
        InboxKind::Error { text } => (format!("{name} hit an error"), text.clone()),
    };
    let mut out = vec![
        Line::from(Span::styled(head, theme.accent())),
        Line::default(),
    ];
    for line in fmt::wrap(&body, width) {
        out.push(Line::from(Span::styled(line, theme.body())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::*;

    fn agent(id: &str, status: AgentStatus, last: i64, parent: Option<&str>) -> Agent {
        Agent {
            id: AgentId(id.into()),
            name: id.into(),
            kind: if parent.is_some() {
                AgentKind::Subagent { background: false }
            } else {
                AgentKind::Slice
            },
            parent: parent.map(|p| AgentId(p.into())),
            task: None,
            worktree: None,
            status,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: last,
            lock: SessionLock::Headless,
            model: None,
            subagent_tokens: 0,
        }
    }

    fn names(snap: &Snapshot) -> Vec<(String, usize)> {
        agent_order(snap)
            .into_iter()
            .map(|(a, d)| (a.name.clone(), d))
            .collect()
    }

    #[test]
    fn agents_that_need_you_come_first_then_the_most_recent() {
        let snap = Snapshot {
            agents: vec![
                agent("old", AgentStatus::Idle, 10, None),
                agent("recent", AgentStatus::Working, 90, None),
                agent("blocked", AgentStatus::NeedsYou, 5, None),
            ],
            ..Snapshot::default()
        };
        let order: Vec<String> = names(&snap).into_iter().map(|(n, _)| n).collect();
        assert_eq!(order, vec!["blocked", "recent", "old"]);
    }

    #[test]
    fn an_inbox_item_counts_as_needing_you_even_before_the_status_catches_up() {
        let mut snap = Snapshot {
            agents: vec![
                agent("recent", AgentStatus::Working, 90, None),
                agent("quiet", AgentStatus::Idle, 1, None),
            ],
            ..Snapshot::default()
        };
        snap.inbox.push(InboxItem {
            id: InboxId(1),
            agent: AgentId("quiet".into()),
            at_ms: 0,
            kind: InboxKind::FinishedTurn {
                preview: String::new(),
            },
        });
        assert_eq!(names(&snap)[0].0, "quiet");
    }

    #[test]
    fn subagents_sit_under_their_parent_one_level_in() {
        let snap = Snapshot {
            agents: vec![
                agent("sub-old", AgentStatus::Done, 1, Some("parent")),
                agent("other", AgentStatus::Working, 50, None),
                agent("parent", AgentStatus::Working, 100, None),
                agent("sub-new", AgentStatus::Working, 60, Some("parent")),
            ],
            ..Snapshot::default()
        };
        assert_eq!(
            names(&snap),
            vec![
                ("parent".into(), 0),
                ("sub-new".into(), 1),
                ("sub-old".into(), 1),
                ("other".into(), 0),
            ]
        );
    }

    #[test]
    fn a_subagent_whose_parent_is_gone_is_listed_on_its_own() {
        let snap = Snapshot {
            agents: vec![agent("orphan", AgentStatus::Working, 1, Some("gone"))],
            ..Snapshot::default()
        };
        assert_eq!(names(&snap), vec![("orphan".into(), 0)]);
    }
}
