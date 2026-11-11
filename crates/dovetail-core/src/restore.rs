//! Opening again after a quit, a kill or a crash (M10, decisions of 2026-11-06).
//!
//! - **Cut-off turns.** An agent stored as working, needing you or setting up was cut off. It
//!   gets one Needs-you item with its last prompt; it is never continued automatically.
//! - **In-flight items.** A pending permission request died with its session: it leaves Needs
//!   you and becomes an Activity note. A pending `core_ask_user` question stays; answering it sends the
//!   agent a normal message. Finished turns and errors stay as they were.
//! - **Restore.** The coordinator's session resumes at once (if it has run before); slices resume
//!   when first touched (`ensure_live`).
//!
//! - **Sweep.** A slice whose worktree directory is gone is archived, with an Activity note. A
//!   worktree under Dovetail's worktrees directory that no agent owns raises one "Remove
//!   worktree" item; `y` removes it, `d` keeps it and records that, so it is not asked again.
//!   Nothing is deleted without `y`.
//!
//! The state at quit is kept as it was: statuses are not recomputed while quitting, so a turn
//! running at quit reads as cut off next time.

use dovetail_api::{
    ActivityKind, AgentId, AgentStatus, EntryKind, Event, FRESH_COORDINATOR_PERMISSION, InboxItem,
    InboxKind, KEEP_COORDINATOR_PERMISSION, RELEASE_TAKEOVER_PERMISSION,
    REMOVE_WORKTREE_PERMISSION, START_SLICE_PERMISSION, UPGRADE_READY_PERMISSION,
};

use crate::inbox::Waiter;
use crate::service::{COORDINATOR, Core, InboxEntry, Input};
use crate::worktree;
use serde_json::json;

/// A directory under the worktrees directory that no agent owns, as the sweep found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Orphan {
    pub(crate) name: String,
    pub(crate) path: std::path::PathBuf,
    /// Device, inode and change time when inspected; `y` removes it only if the path still
    /// names that same directory (an inode alone can be reused by a new one).
    pub(crate) identity: (u64, u64, i64, i64),
    /// For a folder with no `.git`: how many files it holds (shown, since `y` deletes them).
    pub(crate) files: Option<usize>,
}

/// Most files counted in a folder that is not a worktree.
const FILES_COUNTED: usize = 100_000;

impl Orphan {
    /// Looks at a directory (blocking).
    fn look(name: String, path: std::path::PathBuf) -> Orphan {
        let files = (!path.join(".git").exists()).then(|| count_files(&path));
        Orphan {
            identity: Self::identity(&path).unwrap_or_default(),
            name,
            path,
            files,
        }
    }

    fn identity(path: &std::path::Path) -> Option<(u64, u64, i64, i64)> {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path)
            .ok()
            .map(|m| (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()))
    }
}

/// Files under `dir`, not following symlinks, up to [`FILES_COUNTED`].
fn count_files(dir: &std::path::Path) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(_) => count += 1,
                Err(_) => {}
            }
            if count >= FILES_COUNTED {
                return count;
            }
        }
    }
    count
}

/// What `load` found that needs acting on once the UI has its snapshot.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    /// Agents whose turn (or setup) was running when the app stopped, with the status stored.
    pub(crate) cut_off: Vec<(AgentId, AgentStatus)>,
    /// Inbox items never resolved.
    pub(crate) inbox: Vec<InboxItem>,
}

/// `Write a.txt`, `Bash cargo test`: the tool and the one thing it acts on.
fn request_summary(tool: &str, input: &serde_json::Value) -> String {
    let field = |k: &str| input.get(k).and_then(|v| v.as_str());
    let target = field("command")
        .map(one_line)
        .or_else(|| {
            field("file_path").map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map_or_else(|| p.to_owned(), |n| n.to_string_lossy().into_owned())
            })
        })
        .or_else(|| field("name").map(str::to_owned));
    match target {
        Some(t) => format!("{tool} {t}"),
        None => tool.to_owned(),
    }
}

/// Why a missing worktree could not be confirmed gone.
#[derive(Debug, thiserror::Error)]
enum GoneCheck {
    #[error("the path has no parent directory")]
    NoParent,
    #[error("{0}: {1}")]
    Unreadable(std::path::PathBuf, #[source] std::io::Error),
    #[error(transparent)]
    Git(#[from] crate::worktree::WorktreeError),
}

/// Whether a slice's worktree is really gone: its parent directory reads fine, it is not
/// there, and `git worktree list` in the main checkout does not name it.
fn worktree_gone(repo: &std::path::Path, path: &std::path::Path) -> Result<bool, GoneCheck> {
    let parent = path.parent().ok_or(GoneCheck::NoParent)?;
    std::fs::read_dir(parent).map_err(|e| GoneCheck::Unreadable(parent.to_owned(), e))?;
    if path.exists() {
        return Ok(false);
    }
    let listed = crate::worktree::listed_worktrees(repo)?;
    Ok(!listed.iter().any(|p| p == path))
}

/// Longest command shown in an Activity note.
const COMMAND_MAX: usize = 60;

/// A command's first line, cut to [`COMMAND_MAX`] characters with an ellipsis when it is longer
/// or has more lines.
fn one_line(command: &str) -> String {
    let mut lines = command.trim().lines();
    let first = lines.next().unwrap_or("").trim_end();
    let more = lines.next().is_some();
    if first.chars().count() > COMMAND_MAX {
        let cut: String = first.chars().take(COMMAND_MAX - 1).collect();
        format!("{}…", cut.trim_end())
    } else if more {
        format!("{first} …")
    } else {
        first.to_owned()
    }
}

impl Core {
    /// Acts on what [`Restored`] holds. Runs after the first snapshot, so every change goes out
    /// as an event.
    pub(crate) async fn reconcile(&mut self, restored: Restored) {
        let coordinator_id = AgentId(COORDINATOR.into());
        // Read before anything below adds to its conversation.
        let last_used = self.coordinator_last_used();
        if restored
            .cut_off
            .iter()
            .any(|(a, s)| a == &coordinator_id && *s != AgentStatus::SettingUp)
        {
            self.notes_wait_for_user = true;
        }
        for item in restored.inbox {
            match &item.kind {
                InboxKind::Permission { tool, input, .. } => {
                    self.persist(
                        "an expired request",
                        self.db
                            .resolve_inbox(item.id, Some("expired at restart".into())),
                    );
                    // Dovetail's own items: an approval the coordinator was told is `pending`
                    // gets an answer; a worktree question is simply asked again (by the sweep,
                    // or by archiving the slice again).
                    if tool == START_SLICE_PERMISSION {
                        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        self.tell_coordinator(format!(
                            "project_start_slice {name} expired at restart; call it again if still needed"
                        ));
                        continue;
                    }
                    // The lock is checked afresh below, and asked about again if still unclear.
                    if tool == REMOVE_WORKTREE_PERMISSION || tool == RELEASE_TAKEOVER_PERMISSION {
                        continue;
                    }
                    // A staged build is offered again by `resume_upgrade` (ADR 0014).
                    if tool == UPGRADE_READY_PERMISSION {
                        continue;
                    }
                    if tool == KEEP_COORDINATOR_PERMISSION || tool == FRESH_COORDINATOR_PERMISSION {
                        self.activity(
                            ActivityKind::Lifecycle,
                            Some(&item.agent),
                            "the offer to keep yesterday's coordinator expired at restart; its \
                             transcript stays on disk"
                                .into(),
                        );
                        continue;
                    }
                    let name = self.name_of(&item.agent);
                    self.activity(
                        ActivityKind::Lifecycle,
                        Some(&item.agent),
                        format!(
                            "{name}: {}: request expired at restart",
                            request_summary(tool, input)
                        ),
                    );
                }
                InboxKind::Question { text } => {
                    let question = text.clone();
                    self.restore_item(item, Waiter::RestoredQuestion { question });
                }
                InboxKind::FinishedTurn { .. } | InboxKind::Error { .. } => {
                    self.restore_item(item, Waiter::None);
                }
                // The finding is in `state.db`; the question of where it goes is still open.
                InboxKind::UnfiledFinding { finding, .. } => {
                    let finding = *finding;
                    self.restore_item(item, Waiter::UnfiledFinding { finding });
                }
                // The call returned `pending` and nothing was written: say so, so it is
                // proposed again if still wanted.
                InboxKind::Proposal { kind, text, .. } => {
                    self.persist(
                        "an expired proposal",
                        self.db
                            .resolve_inbox(item.id, Some("expired at restart".into())),
                    );
                    let first = text.lines().next().unwrap_or("");
                    self.tell_coordinator(format!(
                        "research_propose expired at restart: your {} \"{first}\" was not \
                         written; propose it again if still needed",
                        kind.as_str()
                    ));
                }
            }
        }
        for (agent, status) in restored.cut_off {
            if !self.agents.contains_key(&agent) {
                continue;
            }
            // Store the reset status, so the next open does not report it again.
            self.save_agent(&agent);
            let name = self.name_of(&agent);
            let text = if status == AgentStatus::SettingUp {
                format!("{name}'s setup was cut off by a restart; archive it and start it again")
            } else {
                let prompt = self.last_prompt(&agent);
                match prompt {
                    Some(p) => {
                        format!("{name}'s turn was cut off by a restart\n\nLast prompt: {p}")
                    }
                    None => format!("{name}'s turn was cut off by a restart"),
                }
            };
            self.raise(&agent, InboxKind::Error { text }, Waiter::None);
        }
        // Locks left by a crash, before anything can start a session (M14).
        self.reconcile_locks();
        self.sweep().await;
        self.profile_notices();
        let ids: Vec<AgentId> = self.agents.keys().cloned().collect();
        for id in &ids {
            self.refresh_status(id);
        }
        // The coordinator comes back at once; its turn is not continued (no input is sent). On
        // the first open of a new day it starts fresh instead, from the journal (ADR 0012).
        let coordinator = AgentId(COORDINATOR.into());
        let has_run = self
            .agents
            .get(&coordinator)
            .is_some_and(|r| r.transcript_path.is_some());
        // One still taken over waits: Needs you says why.
        if has_run && self.claude.is_ok() && !self.locked(&coordinator) {
            self.start_coordinator(last_used).await;
        }
    }

    /// Says in Needs you when the project's Claude profile cannot be used, or when it changed
    /// since sessions were recorded (ADR 0009). The change is told once: the coordinator, which
    /// comes back at open, is one of those sessions, and its relaunch records the new profile.
    fn profile_notices(&mut self) {
        let coordinator = AgentId(COORDINATOR.into());
        let home = match &self.claude {
            Ok(home) => home.clone(),
            Err(_) => {
                self.profile_error_notice();
                return;
            }
        };
        let moved = |r: &crate::db::AgentRow| {
            r.agent.kind.is_session()
                && r.transcript_path.is_some()
                && !crate::profile::same_dir(
                    &r.claude_home
                        .clone()
                        .unwrap_or_else(crate::profile::default_dir),
                    &home.dir,
                )
        };
        if !self.agents.get(&coordinator).is_some_and(moved) {
            return;
        }
        let count = self.agents.values().filter(|r| moved(r)).count();
        let sessions = if count == 1 {
            "1 session was".to_owned()
        } else {
            format!("{count} sessions were")
        };
        let text = format!(
            "This project now runs under Claude profile {}. {sessions} recorded under another \
             profile and will start fresh, not resume. Tasks, decisions and worktrees are kept.",
            home.label()
        );
        self.raise(&coordinator, InboxKind::Error { text }, Waiter::None);
    }

    /// The one Needs-you error for a profile that cannot be used, unless it is already open (from
    /// an earlier open, or raised at `:project add`).
    pub(crate) fn profile_error_notice(&mut self) {
        let Err(e) = &self.claude else { return };
        let text = format!("Sessions cannot start: {e}");
        let open = self
            .inbox
            .values()
            .any(|p| matches!(&p.item.kind, InboxKind::Error { text: t } if *t == text));
        if !open {
            let coordinator = AgentId(COORDINATOR.into());
            self.raise(&coordinator, InboxKind::Error { text }, Waiter::None);
        }
    }

    /// The stale-state sweep (decision 4 of 2026-11-06).
    async fn sweep(&mut self) {
        // Only worktrees are swept, and a research project has none: no git calls.
        if self.project.is_research() {
            return;
        }
        let missing: Vec<(AgentId, String, std::path::PathBuf)> = self
            .agents
            .values()
            .filter_map(|r| {
                let w = r.agent.worktree.as_ref()?;
                (!w.path.exists())
                    .then(|| (r.agent.id.clone(), r.agent.name.clone(), w.path.clone()))
            })
            .collect();
        let repo = self.project.folder(&self.paths);
        for (agent, name, path) in missing {
            // One failed `exists()` is not proof: archive only when the parent directory reads
            // fine and git no longer lists the worktree either.
            match worktree_gone(&repo, &path) {
                Ok(true) => {
                    self.closed(&agent);
                    self.activity(
                        ActivityKind::Lifecycle,
                        None,
                        format!("slice {name} archived: its worktree is gone"),
                    );
                }
                Ok(false) => {
                    tracing::warn!(slice = %name, "worktree missing on disk but listed by git; kept")
                }
                Err(why) => {
                    tracing::warn!(slice = %name, "could not check its worktree ({why}); kept")
                }
            }
        }

        let key = self.project.key().to_owned();
        let dir = self.paths.worktrees(&key);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return; // no worktrees yet
        };
        let owned: Vec<std::path::PathBuf> = self
            .agents
            .values()
            .filter_map(|r| r.agent.worktree.as_ref().map(|w| w.path.clone()))
            .collect();
        let kept = match self.db.kept_worktrees().await {
            Ok(kept) => kept,
            Err(e) => {
                tracing::warn!("reading kept worktrees: {e}");
                return;
            }
        };
        let repo = self.project.folder(&self.paths);
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !path.is_dir() || owned.contains(&path) || kept.contains(&path) {
                continue;
            }
            let paths = self.paths.clone();
            let key = key.clone();
            let repo = repo.clone();
            let inputs = self.inputs.clone();
            tokio::task::spawn_blocking(move || {
                let found = Orphan::look(name, path);
                let result = worktree::inspect(&paths, &key, &found.name, &repo);
                let _ = inputs.send(Input::OrphanInspected { found, result });
            });
        }
    }

    /// The agent id an unowned worktree's item is filed under (no agent owns it).
    fn orphan_agent(name: &str) -> AgentId {
        AgentId(format!("worktree:{name}"))
    }

    pub(crate) fn on_orphan(
        &mut self,
        found: Orphan,
        result: Result<worktree::Inspection, worktree::WorktreeError>,
    ) {
        match result {
            Ok(report) => {
                let mut input = json!({
                    "worktree": found.path,
                    "branch": worktree::branch_name(&found.name),
                    "uncommitted": report.uncommitted,
                    "unpushed_commits": report.unpushed_commits,
                });
                if let Some(files) = found.files {
                    input["files"] = json!(files);
                }
                let reason = if found.files.is_some() {
                    "no agent owns this folder, and it is not a git worktree"
                } else {
                    "no agent owns this worktree"
                };
                self.raise(
                    &Self::orphan_agent(&found.name),
                    InboxKind::Permission {
                        tool: REMOVE_WORKTREE_PERMISSION.into(),
                        input,
                        reason: Some(reason.into()),
                        suggestions: Vec::new(),
                    },
                    Waiter::RemoveOrphan { found },
                );
            }
            Err(e) => self.activity(
                ActivityKind::Lifecycle,
                None,
                format!(
                    "worktree {}: owned by no agent, and could not be inspected: {e}",
                    found.name
                ),
            ),
        }
    }

    /// `y` on an unowned worktree's item. Checked again first: still no owner, and still the
    /// same directory that was inspected.
    pub(crate) fn remove_orphan(&mut self, found: Orphan) {
        let owned = self.agents.values().any(|r| {
            r.agent
                .worktree
                .as_ref()
                .is_some_and(|w| w.path == found.path)
        });
        let same = Orphan::identity(&found.path) == Some(found.identity);
        if owned || !same {
            self.activity(
                ActivityKind::Lifecycle,
                None,
                format!(
                    "worktree {}: changed since Dovetail asked, so it was not removed",
                    found.name
                ),
            );
            return;
        }
        let paths = self.paths.clone();
        let key = self.project.key().to_owned();
        let repo = self.project.folder(&self.paths);
        let inputs = self.inputs.clone();
        let name = found.name;
        tokio::task::spawn_blocking(move || {
            let result = worktree::remove(&paths, &key, &name, &repo, true);
            let _ = inputs.send(Input::OrphanRemoved { name, result });
        });
    }

    pub(crate) fn on_orphan_removed(
        &mut self,
        name: String,
        result: Result<(), worktree::WorktreeError>,
    ) {
        let text = match result {
            Ok(()) => format!("worktree {name} removed"),
            Err(e) => format!("worktree {name}: removing it failed: {e}"),
        };
        self.activity(ActivityKind::Lifecycle, None, text);
    }

    /// `d` on an unowned worktree's item: keep it, and never ask again.
    pub(crate) fn keep_orphan(&mut self, found: Orphan) {
        self.persist("a kept worktree", self.db.keep_worktree(found.path.clone()));
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!(
                "worktree {} kept; Dovetail will not ask about it again",
                found.name
            ),
        );
    }

    /// Puts an item read from storage back in Needs you, as it was.
    fn restore_item(&mut self, item: InboxItem, waiter: Waiter) {
        self.emit(Event::InboxUpserted(item.clone()));
        let agent = item.agent.clone();
        self.inbox.insert(item.id, InboxEntry { item, waiter });
        self.refresh_status(&agent);
    }

    /// The agent's most recent prompt from the user, as shown in its conversation.
    fn last_prompt(&self, agent: &AgentId) -> Option<String> {
        self.tails
            .get(agent)?
            .entries
            .iter()
            .rev()
            .find(|e| e.kind == EntryKind::User)
            .map(|e| e.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_requests_name_the_tool_and_its_target() {
        assert_eq!(
            request_summary("Write", &json!({"file_path": "/w/a.txt", "content": "x"})),
            "Write a.txt"
        );
        assert_eq!(
            request_summary("Bash", &json!({"command": "cargo test"})),
            "Bash cargo test"
        );
        assert_eq!(
            request_summary("Bash", &json!({"command": "cd x &&\n  cargo test"})),
            "Bash cd x && …"
        );
        let long = request_summary("Bash", &json!({"command": "x".repeat(200)}));
        assert_eq!(long.chars().count(), "Bash ".len() + COMMAND_MAX);
        assert!(long.ends_with('…'));
    }
}
