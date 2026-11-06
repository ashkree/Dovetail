//! Opening again after a quit, a kill or a crash (M10, decisions of 2026-11-06).
//!
//! - **Cut-off turns.** An agent stored as working, needing you or setting up was cut off. It
//!   gets one Needs-you item with its last prompt; it is never continued automatically.
//! - **In-flight items.** A pending permission request died with its session: it leaves Needs
//!   you and becomes an Activity note. A pending `ask_user` question stays; answering it sends the
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
    ActivityKind, AgentId, AgentStatus, EntryKind, Event, InboxItem, InboxKind,
    REMOVE_WORKTREE_PERMISSION, START_SLICE_PERMISSION,
};

use crate::inbox::Waiter;
use crate::service::{COORDINATOR, Core, InboxEntry, Input};
use crate::worktree;
use serde_json::json;

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
    #[error("running git: {0}")]
    Git(#[source] std::io::Error),
    #[error("git worktree list failed: {0}")]
    GitFailed(String),
}

/// Whether a slice's worktree is really gone: its parent directory reads fine, it is not
/// there, and `git worktree list` in the main checkout does not name it.
fn worktree_gone(repo: &std::path::Path, path: &std::path::Path) -> Result<bool, GoneCheck> {
    let parent = path.parent().ok_or(GoneCheck::NoParent)?;
    std::fs::read_dir(parent).map_err(|e| GoneCheck::Unreadable(parent.to_owned(), e))?;
    if path.exists() {
        return Ok(false);
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "list", "--porcelain"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(GoneCheck::Git)?;
    if !out.status.success() {
        return Err(GoneCheck::GitFailed(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    let listed = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .any(|p| std::path::Path::new(p) == path);
    Ok(!listed)
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
        if restored
            .cut_off
            .iter()
            .any(|(a, s)| a == &coordinator_id && *s != AgentStatus::SettingUp)
        {
            self.notes_wait_for_user = true;
        }
        let mut asking = std::collections::HashSet::new();
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
                            "start_slice {name} expired at restart; call it again if still needed"
                        ));
                        continue;
                    }
                    if tool == REMOVE_WORKTREE_PERMISSION {
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
                InboxKind::Question { text, blocking } => {
                    // Only a blocking question explains a stopped turn; after a non-blocking
                    // one the agent kept working, so its cut-off is still reported.
                    if *blocking {
                        asking.insert(item.agent.clone());
                    }
                    let question = text.clone();
                    self.restore_item(item, Waiter::RestoredQuestion { question });
                }
                InboxKind::FinishedTurn { .. } | InboxKind::Error { .. } => {
                    self.restore_item(item, Waiter::None);
                }
            }
        }
        for (agent, status) in restored.cut_off {
            if !self.agents.contains_key(&agent) {
                continue;
            }
            // Store the reset status, so the next open does not report it again.
            self.save_agent(&agent);
            // A question still waiting says enough: answering it resumes the agent.
            if asking.contains(&agent) {
                continue;
            }
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
        self.sweep().await;
        let ids: Vec<AgentId> = self.agents.keys().cloned().collect();
        for id in &ids {
            self.refresh_status(id);
        }
        // The coordinator comes back at once; its turn is not continued (no input is sent).
        let coordinator = AgentId(COORDINATOR.into());
        let has_run = self
            .agents
            .get(&coordinator)
            .is_some_and(|r| r.transcript_path.is_some());
        if has_run && let Err(e) = self.ensure_live(&coordinator) {
            self.fail(e);
        }
    }

    /// The stale-state sweep (decision 4 of 2026-11-06).
    async fn sweep(&mut self) {
        let missing: Vec<(AgentId, String, std::path::PathBuf)> = self
            .agents
            .values()
            .filter_map(|r| {
                let w = r.agent.worktree.as_ref()?;
                (!w.path.exists())
                    .then(|| (r.agent.id.clone(), r.agent.name.clone(), w.path.clone()))
            })
            .collect();
        let repo = self.project.repo_path().to_path_buf();
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
        let kept = match self.db.kept_worktrees(key.clone()).await {
            Ok(kept) => kept,
            Err(e) => {
                tracing::warn!("reading kept worktrees: {e}");
                return;
            }
        };
        let repo = self.project.repo_path().to_path_buf();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !path.is_dir() || owned.contains(&path) || kept.contains(&name) {
                continue;
            }
            let paths = self.paths.clone();
            let key = key.clone();
            let repo = repo.clone();
            let inputs = self.inputs.clone();
            tokio::task::spawn_blocking(move || {
                let result = worktree::inspect(&paths, &key, &name, &repo);
                let _ = inputs.send(Input::OrphanInspected { name, result });
            });
        }
    }

    /// The agent id an unowned worktree's item is filed under (no agent owns it).
    fn orphan_agent(name: &str) -> AgentId {
        AgentId(format!("worktree:{name}"))
    }

    pub(crate) fn on_orphan(
        &mut self,
        name: String,
        result: Result<worktree::Inspection, worktree::WorktreeError>,
    ) {
        match result {
            Ok(report) => {
                let input = json!({
                    "worktree": worktree::worktree_path(&self.paths, self.project.key(), &name),
                    "branch": worktree::branch_name(&name),
                    "uncommitted": report.uncommitted,
                    "unpushed_commits": report.unpushed_commits,
                });
                self.raise(
                    &Self::orphan_agent(&name),
                    InboxKind::Permission {
                        tool: REMOVE_WORKTREE_PERMISSION.into(),
                        input,
                        reason: Some("no agent owns this worktree".into()),
                        suggestions: Vec::new(),
                    },
                    Waiter::RemoveOrphan { name },
                );
            }
            Err(e) => self.activity(
                ActivityKind::Lifecycle,
                None,
                format!("worktree {name}: owned by no agent, and could not be inspected: {e}"),
            ),
        }
    }

    /// `y` on an unowned worktree's item.
    pub(crate) fn remove_orphan(&mut self, name: String) {
        let paths = self.paths.clone();
        let key = self.project.key().to_owned();
        let repo = self.project.repo_path().to_path_buf();
        let inputs = self.inputs.clone();
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
    pub(crate) fn keep_orphan(&mut self, name: String) {
        self.persist(
            "a kept worktree",
            self.db
                .keep_worktree(self.project.key().to_owned(), name.clone()),
        );
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!("worktree {name} kept; Dovetail will not ask about it again"),
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
