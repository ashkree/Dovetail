//! Claude Code's native to-do lists (M11, digest §4): reading the task files, watching the list
//! directories of live sessions, attributing items to the agent that created them, and deleting
//! the lists Dovetail created when their task is archived.
//!
//! - Files live at `<tasks dir>/<list-id>/<item-id>.json`, one item each, rewritten in place under
//!   a lock directory while reads take no lock. A file that does not parse is being written: its
//!   last good version is kept and the list is read again shortly after, a few times at most.
//! - The watcher (inotify through `notify`) acts on close-write, create, delete and rename only,
//!   never on plain modify events, so a half-written file is rarely read at all. It holds one
//!   watch per list directory of a live session and costs nothing while idle. Every `TaskCreate`
//!   or `TaskUpdate` result also schedules a reread, which covers a directory that did not exist
//!   yet when its session started.
//! - Nothing on disk says who created an item. `TaskCreate` results carry the new item's id, and a
//!   foreground subagent's call carries `parent_tool_use_id`, so the core records item → agent in
//!   `state.db` (`todo_items`). Background subagents cannot use the task tools. An item with no
//!   known creator (edited in by hand, or made while Dovetail was not watching) is shown under
//!   the session whose list it is.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use dovetail_api::Event;
use dovetail_api::agents::agent::{
    AgentEvent, AgentId, AgentKind, TodoItem, TodoListId, TodoStatus,
};
use dovetail_api::tracking::task::TaskId;
use dovetail_driver::{ContentBlock, DriverEvent};
use notify::Watcher;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc as tokio_mpsc;

use crate::engine::{Core, Input};
use crate::sessions::subagent_id;

/// How long changes to one list are gathered before it is read again.
pub(crate) const TODO_WINDOW: Duration = Duration::from_millis(50);
/// How many times in a row a list with an unreadable file is read again before waiting for the
/// next change.
const MAX_REREADS: u8 = 5;

/// One task file as Claude Code writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct TaskFile {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) subject: String,
    pub(crate) status: TodoStatus,
    #[serde(default, rename = "blockedBy")]
    pub(crate) blocked_by: Vec<String>,
}

/// Parses one task file.
pub(crate) fn parse_task_file(text: &str) -> Result<TaskFile, serde_json::Error> {
    serde_json::from_str(text)
}

/// What reading a list directory found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ListRead {
    /// Items that parsed, by id order.
    pub(crate) items: Vec<TaskFile>,
    /// Ids (file stems) of files that did not parse: being written, read again later.
    pub(crate) unreadable: Vec<String>,
}

/// Reads every `<id>.json` in `dir`. A missing directory is an empty list.
pub(crate) fn read_list(dir: &Path) -> ListRead {
    let mut out = ListRead::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse_task_file(&text) {
                Ok(item) => out.items.push(item),
                Err(_) => out.unreadable.push(stem),
            },
            // Deleted between listing and reading.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => out.unreadable.push(stem),
        }
    }
    out.items.sort_by(|a, b| id_order(&a.id, &b.id));
    out.unreadable.sort();
    out
}

/// Ids are integers as strings; order them as numbers.
fn id_order(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// The item id a `TaskCreate` result reports: `tool_use_result.task.id`, else the
/// "Task #N created" text.
pub(crate) fn created_id(meta: Option<&Value>, text: &str) -> Option<String> {
    if let Some(id) = meta
        .and_then(|m| m.get("task"))
        .and_then(|t| t.get("id"))
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    {
        return Some(id);
    }
    let rest = text.split_once("Task #")?.1;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// A task tool call waiting for its result.
pub(crate) struct TodoCall {
    /// The agent that made it: the session's own, or a foreground subagent.
    owner: AgentId,
    /// `TaskCreate` (else `TaskUpdate`).
    create: bool,
}

/// What the core keeps about native lists.
#[derive(Default)]
pub(crate) struct Todos {
    /// The latest items of each list read so far.
    lists: BTreeMap<TodoListId, Vec<TodoItem>>,
    /// Who created which item: `(list, item id)` → agent.
    owners: HashMap<(TodoListId, String), AgentId>,
    /// Task tool calls by `tool_use_id`, until their results arrive.
    calls: HashMap<String, TodoCall>,
    /// Lists to read again at `flush_at`, with how many rereads in a row each has had.
    dirty: BTreeMap<TodoListId, u8>,
    pub(crate) flush_at: Option<tokio::time::Instant>,
    /// The inotify watcher, created on first use.
    watcher: Option<notify::RecommendedWatcher>,
    watched: BTreeSet<TodoListId>,
    /// Lists deleted on archive: late writes from a stopping session are removed again.
    deleted: BTreeSet<TodoListId>,
}

impl Todos {
    /// Every item, for the snapshot.
    pub(crate) fn items(&self) -> Vec<TodoItem> {
        self.lists.values().flatten().cloned().collect()
    }
}

/// A watcher that turns events in list directories into [`Input::TodosChanged`].
fn new_watcher(
    inputs: tokio_mpsc::UnboundedSender<Input>,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::event::{AccessKind, AccessMode, EventKind, ModifyKind};
    notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        let relevant = matches!(
            event.kind,
            EventKind::Access(AccessKind::Close(AccessMode::Write))
                | EventKind::Create(_)
                | EventKind::Remove(_)
                | EventKind::Modify(ModifyKind::Name(_))
        );
        if !relevant {
            return;
        }
        let lists: BTreeSet<String> = event
            .paths
            .iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .filter_map(|p| p.parent()?.file_name()?.to_str().map(str::to_owned))
            .collect();
        for list in lists {
            let _ = inputs.send(Input::TodosChanged {
                list: TodoListId(list),
            });
        }
    })
}

impl Core {
    /// A list's folder: `tasks/<list>` in the Claude home of the session it belongs to (its
    /// profile's folder, ADR 0009), unless tests override the tasks dir.
    fn list_dir(&self, list: &TodoListId) -> PathBuf {
        let tasks = match &self.options.tasks_dir {
            Some(dir) => dir.clone(),
            None => self
                .list_owner(list)
                .map_or_else(|| self.current_home_dir(), |id| self.home_of(&id))
                .join("tasks"),
        };
        tasks.join(&list.0)
    }

    /// The native list agent `id`'s session uses.
    pub(crate) fn list_of(&self, id: &AgentId) -> Option<TodoListId> {
        let row = self.agents.rows.get(id)?;
        if matches!(row.agent.kind, AgentKind::Subagent { .. }) {
            return row.agent.parent.as_ref().and_then(|p| self.list_of(p));
        }
        Some(TodoListId(row.task_list_id.clone().unwrap_or_else(|| {
            self.task_list_id(row.agent.task, &row.agent.name)
        })))
    }

    /// The agent an item with no known creator is shown under: the session whose list it is
    /// (decided 2026-11-09, PR #13). When sessions share a task's list, a running one first,
    /// then the first by id.
    fn list_owner(&self, list: &TodoListId) -> Option<AgentId> {
        let mut owners = self.agents.rows.iter().filter(|(id, row)| {
            !matches!(row.agent.kind, AgentKind::Subagent { .. })
                && self.list_of(id).as_ref() == Some(list)
        });
        let first = owners.clone().next().map(|(id, _)| id.clone());
        owners
            .find(|(id, _)| self.sessions.live.contains_key(*id))
            .map(|(id, _)| id.clone())
            .or(first)
    }

    /// Agent `gone` was closed: its items move to the session whose list they are on, the same
    /// rule as items with no known creator, so no item points at an agent the UI no longer has.
    /// The recorded owner stays in `state.db`; only what is shown changes.
    pub(crate) fn reassign_todos_of(&mut self, gone: &AgentId) {
        let lists: Vec<TodoListId> = self
            .agents
            .todos
            .lists
            .iter()
            .filter(|(_, items)| items.iter().any(|t| t.created_by.as_ref() == Some(gone)))
            .map(|(list, _)| list.clone())
            .collect();
        for list in lists {
            let fallback = self.list_owner(&list);
            let Some(items) = self.agents.todos.lists.get_mut(&list) else {
                continue;
            };
            for item in items.iter_mut() {
                if item.created_by.as_ref() == Some(gone) {
                    item.created_by = fallback.clone();
                }
            }
            let items = items.clone();
            self.emit(Event::Agent(AgentEvent::TodosReplaced { list, items }));
        }
    }

    /// At start: the recorded owners, and every known agent's list as it is on disk now.
    pub(crate) async fn load_todos(&mut self) -> Result<(), crate::storage::DbError> {
        for (list, item, agent) in self.db.load_todo_owners().await? {
            self.agents
                .todos
                .owners
                .insert((TodoListId(list), item), agent);
        }
        let lists: BTreeSet<TodoListId> = self
            .agents
            .rows
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .iter()
            .filter_map(|id| self.list_of(id))
            .collect();
        for list in lists {
            self.reread_list(&list, false);
        }
        Ok(())
    }

    /// A session started: watch its list and read it.
    pub(crate) fn watch_todos(&mut self, agent: &AgentId) {
        let Some(list) = self.list_of(agent) else {
            return;
        };
        self.ensure_watch(&list);
        self.mark_todos(list);
    }

    /// A session exited: stop watching its list unless another live session shares it.
    pub(crate) fn unwatch_todos(&mut self, agent: &AgentId) {
        let Some(list) = self.list_of(agent) else {
            return;
        };
        let shared = self
            .sessions
            .live
            .keys()
            .any(|other| other != agent && self.list_of(other).as_ref() == Some(&list));
        if shared || !self.agents.todos.watched.remove(&list) {
            return;
        }
        let dir = self.list_dir(&list);
        if let Some(w) = self.agents.todos.watcher.as_mut() {
            let _ = w.unwatch(&dir);
        }
    }

    /// Watches `list`'s directory if it exists and is not watched yet.
    fn ensure_watch(&mut self, list: &TodoListId) {
        if self.agents.todos.watched.contains(list) {
            return;
        }
        let dir = self.list_dir(list);
        if !dir.is_dir() {
            return; // made by the first `TaskCreate`; its result calls here again
        }
        if self.agents.todos.watcher.is_none() {
            match new_watcher(self.inputs.clone()) {
                Ok(w) => self.agents.todos.watcher = Some(w),
                Err(e) => {
                    tracing::warn!("watching to-do lists: {e}; they update from the stream only");
                    return;
                }
            }
        }
        if let Some(w) = self.agents.todos.watcher.as_mut() {
            match w.watch(&dir, notify::RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.agents.todos.watched.insert(list.clone());
                }
                Err(e) => tracing::warn!(list = %list.0, "watching a to-do list: {e}"),
            }
        }
    }

    /// Schedules `list` to be read again after [`TODO_WINDOW`].
    pub(crate) fn mark_todos(&mut self, list: TodoListId) {
        self.agents.todos.dirty.entry(list).or_insert(0);
        if self.agents.todos.flush_at.is_none() {
            self.agents.todos.flush_at = Some(tokio::time::Instant::now() + TODO_WINDOW);
        }
    }

    /// Reads every scheduled list again.
    pub(crate) fn flush_todos(&mut self) {
        self.agents.todos.flush_at = None;
        let dirty = std::mem::take(&mut self.agents.todos.dirty);
        for (list, rereads) in dirty {
            if self.agents.todos.deleted.contains(&list) {
                // A stopping session wrote after its list was deleted.
                self.remove_list_dir(&list);
                continue;
            }
            self.ensure_watch(&list);
            if self.reread_list(&list, true) && rereads < MAX_REREADS {
                self.agents.todos.dirty.insert(list, rereads + 1);
            }
        }
        if !self.agents.todos.dirty.is_empty() {
            self.agents.todos.flush_at = Some(tokio::time::Instant::now() + TODO_WINDOW);
        }
    }

    /// Reads `list` from disk and, when it changed, tells the UI (if `emit`). Returns true when
    /// some file did not parse: the list should be read again.
    fn reread_list(&mut self, list: &TodoListId, emit: bool) -> bool {
        let read = read_list(&self.list_dir(list));
        let previous = self
            .agents
            .todos
            .lists
            .get(list)
            .cloned()
            .unwrap_or_default();
        let fallback = self.list_owner(list);
        let mut items: Vec<TodoItem> = read
            .items
            .into_iter()
            .map(|f| TodoItem {
                list: list.clone(),
                created_by: self
                    .agents
                    .todos
                    .owners
                    .get(&(list.clone(), f.id.clone()))
                    .filter(|a| self.agents.rows.contains_key(*a))
                    .cloned()
                    .or_else(|| fallback.clone()),
                id: f.id,
                subject: f.subject,
                status: f.status,
                blocked_by: f.blocked_by,
            })
            .collect();
        // A file being written keeps its last good version.
        for id in &read.unreadable {
            if let Some(old) = previous.iter().find(|t| &t.id == id) {
                items.push(old.clone());
            }
        }
        items.sort_by(|a, b| id_order(&a.id, &b.id));
        if items != previous {
            if items.is_empty() {
                self.agents.todos.lists.remove(list);
            } else {
                self.agents.todos.lists.insert(list.clone(), items.clone());
            }
            if emit {
                self.emit(Event::Agent(AgentEvent::TodosReplaced {
                    list: list.clone(),
                    items,
                }));
            }
        }
        !read.unreadable.is_empty()
    }

    /// Notes task tool calls and results, from the session or any of its subagents. Runs on
    /// every driver event, before subagent events are routed away.
    pub(crate) fn track_todos(&mut self, agent: &AgentId, event: &DriverEvent) {
        match event {
            DriverEvent::Assistant(message) => {
                let owner = match &message.origin.parent_tool_use_id {
                    Some(tool_use) => subagent_id(agent, tool_use),
                    None => agent.clone(),
                };
                for block in &message.blocks {
                    if let ContentBlock::ToolUse { id, name, .. } = block
                        && (name == "TaskCreate" || name == "TaskUpdate")
                    {
                        let call = TodoCall {
                            owner: owner.clone(),
                            create: name == "TaskCreate",
                        };
                        self.agents.todos.calls.insert(id.clone(), call);
                    }
                }
            }
            DriverEvent::ToolResult(result) => {
                let Some(call) = self.agents.todos.calls.remove(&result.tool_use_id) else {
                    return;
                };
                let Some(list) = self.list_of(agent) else {
                    return;
                };
                if call.create
                    && !result.is_error
                    && let Some(item) = created_id(result.meta.as_ref(), &result.content_text())
                {
                    self.persist(
                        "a to-do item's owner",
                        self.db
                            .set_todo_owner(list.0.clone(), item.clone(), call.owner.clone()),
                    );
                    self.agents
                        .todos
                        .owners
                        .insert((list.clone(), item.clone()), call.owner.clone());
                    // An owner changes the item even when its file has not.
                    if let Some(items) = self.agents.todos.lists.get_mut(&list)
                        && let Some(found) = items.iter_mut().find(|t| t.id == item)
                        && found.created_by.as_ref() != Some(&call.owner)
                    {
                        found.created_by = Some(call.owner);
                        let items = items.clone();
                        self.emit(Event::Agent(AgentEvent::TodosReplaced {
                            list: list.clone(),
                            items,
                        }));
                    }
                }
                self.agents.todos.deleted.remove(&list);
                self.ensure_watch(&list);
                self.mark_todos(list);
            }
            _ => {}
        }
    }

    /// The list of task `id` is no longer needed: delete it (it is Dovetail's: the id is the
    /// one Dovetail gave its sessions) and forget its owners.
    pub(crate) fn delete_task_todos(&mut self, id: TaskId) {
        let list = TodoListId(self.task_list_id(Some(id), ""));
        self.delete_todo_list(&list);
    }

    /// Deletes a list Dovetail created, its watch, its owners, and its items in the UI.
    pub(crate) fn delete_todo_list(&mut self, list: &TodoListId) {
        // Only lists Dovetail named; never a session-id list or one the user set up.
        if !list.0.starts_with("dovetail-") {
            return;
        }
        if self.agents.todos.watched.remove(list) {
            let dir = self.list_dir(list);
            if let Some(w) = self.agents.todos.watcher.as_mut() {
                let _ = w.unwatch(&dir);
            }
        }
        self.remove_list_dir(list);
        self.agents.todos.deleted.insert(list.clone());
        self.agents.todos.dirty.remove(list);
        self.agents.todos.owners.retain(|(l, _), _| l != list);
        self.persist(
            "a deleted to-do list",
            self.db.forget_todo_list(list.0.clone()),
        );
        if self.agents.todos.lists.remove(list).is_some() {
            self.emit(Event::Agent(AgentEvent::TodosReplaced {
                list: list.clone(),
                items: Vec::new(),
            }));
        }
    }

    fn remove_list_dir(&self, list: &TodoListId) {
        let dir = self.list_dir(list);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => tracing::info!(list = %list.0, "deleted a to-do list"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(list = %list.0, "deleting a to-do list: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM: &str = r#"{
  "id": "2",
  "subject": "beta",
  "description": "beta",
  "status": "in_progress",
  "blocks": [],
  "blockedBy": ["1"]
}"#;

    #[test]
    fn a_task_file_parses_with_its_fields() {
        let item = parse_task_file(ITEM).unwrap();
        assert_eq!(
            item,
            TaskFile {
                id: "2".into(),
                subject: "beta".into(),
                status: TodoStatus::InProgress,
                blocked_by: vec!["1".into()],
            }
        );
    }

    #[test]
    fn unknown_status_or_half_a_file_does_not_parse() {
        assert!(parse_task_file(&ITEM[..ITEM.len() / 2]).is_err());
        assert!(parse_task_file("").is_err());
        assert!(parse_task_file(&ITEM.replace("in_progress", "deleted")).is_err());
    }

    #[test]
    fn whitespace_a_partial_last_line_and_trailing_junk_do_not_parse() {
        assert!(parse_task_file("  \n").is_err());
        // A complete object followed by the start of a second write.
        assert!(parse_task_file(&format!("{ITEM}\n{{\"id\": \"3\"")).is_err());
        assert!(parse_task_file(&format!("{ITEM}\n{ITEM}")).is_err());
        // A missing status is an unusable file, not an item with a default.
        assert!(parse_task_file(r#"{"id":"1","subject":"x"}"#).is_err());
    }

    #[test]
    fn an_empty_file_is_unreadable_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("1.json"), "").unwrap();
        let read = read_list(dir.path());
        assert!(read.items.is_empty());
        assert_eq!(read.unreadable, vec!["1".to_string()]);
    }

    #[test]
    fn a_file_caught_mid_write_is_unreadable_then_read_on_the_next_try() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("1.json"),
            r#"{"id":"1","subject":"alpha","status":"completed"}"#,
        )
        .unwrap();
        // Half written, as a reader without the lock may see it.
        std::fs::write(dir.path().join("2.json"), &ITEM[..40]).unwrap();
        std::fs::write(dir.path().join(".highwatermark"), "2").unwrap();
        std::fs::create_dir(dir.path().join(".lock")).unwrap();
        let first = read_list(dir.path());
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].subject, "alpha");
        assert_eq!(first.unreadable, vec!["2".to_string()]);

        std::fs::write(dir.path().join("2.json"), ITEM).unwrap();
        let second = read_list(dir.path());
        assert!(second.unreadable.is_empty());
        let ids: Vec<_> = second.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["1", "2"]);
    }

    #[test]
    fn items_are_ordered_by_number_and_a_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["10", "9", "1"] {
            std::fs::write(
                dir.path().join(format!("{id}.json")),
                format!(r#"{{"id":"{id}","subject":"s","status":"pending"}}"#),
            )
            .unwrap();
        }
        let ids: Vec<_> = read_list(dir.path())
            .items
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, ["1", "9", "10"]);
        assert_eq!(read_list(&dir.path().join("gone")), ListRead::default());
    }

    #[test]
    fn the_created_id_comes_from_the_result_meta_or_its_text() {
        let meta = serde_json::json!({"task": {"id": "7", "subject": "x"}});
        assert_eq!(created_id(Some(&meta), ""), Some("7".into()));
        assert_eq!(
            created_id(None, "Task #12 created successfully: x"),
            Some("12".into())
        );
        assert_eq!(created_id(None, "something else"), None);
    }
}
