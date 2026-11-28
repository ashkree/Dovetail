//! OOM kills in capped sessions (M13b).
//!
//! Each capped session's scope counts kernel OOM kills in `memory.events` (`oom_kill`). The
//! file is watched with inotify and read in the watcher's thread the moment it changes: the
//! kernel counts a kill before the victim dies, and the scope (started with `--collect`) is
//! gone soon after its last process exits, so a later read would miss a kill that took the
//! session down (S4). The 5 s memory sample reads the count too, as a backstop.
//!
//! Scopes run with `OOMPolicy=continue`, so a kill may take one tool (a `cargo` run) and leave
//! the session alive, or take the session itself ([`memory::oom_verdict`]):
//!
//! - a tool: a Needs-you error says so; nothing restarts;
//! - the session: a Needs-you error, and the session is resumed from its session id with the
//!   subagent work that was in flight listed in the item and sent to it as a short note. At
//!   most one automatic restart per agent per [`memory::RESTART_WINDOW`]; after that only the
//!   item, and the user resumes it by writing to it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dovetail_api::activity::{
    ActivityKind,
    inbox::{InboxId, InboxKind},
};
use dovetail_api::agents::agent::{AgentId, AgentKind, AgentStatus};
use notify::Watcher;
use tokio::sync::mpsc as tokio_mpsc;

use crate::engine::{Core, Input};
use crate::inbox::Waiter;
use crate::memory::{self, OomKill};

/// The watcher, the next verdict due and the restart guard.
#[derive(Default)]
pub(crate) struct Oom {
    /// Watches every running capped session's `memory.events`; created on first use.
    watcher: Option<notify::RecommendedWatcher>,
    /// When a kill seen while its session still runs counts as a tool's.
    pub(crate) check_at: Option<tokio::time::Instant>,
    restarts: memory::RestartGuard,
    /// Capped sessions that died by SIGKILL before any OOM kill was seen, by cgroup: the
    /// watcher's count may still be queued behind the exit, after the scope is gone.
    exits: HashMap<PathBuf, UnexplainedExit>,
}

/// A capped session's SIGKILL exit, reported as a crash, that a late `oom_kill` rise within
/// [`memory::OOM_GRACE`] turns into an OOM kill of the session.
pub(crate) struct UnexplainedExit {
    pub(crate) agent: AgentId,
    pub(crate) at: Instant,
    /// The scope's `oom_kill` count as last seen before the exit.
    pub(crate) kills: u64,
    /// Its subagents in flight at the exit.
    pub(crate) work: Vec<String>,
    pub(crate) restartable: bool,
    /// The "ended unexpectedly" item, replaced by the out-of-memory one.
    pub(crate) item: InboxId,
}

/// A watcher that reads `memory.events` as soon as it changes and sends the `oom_kill` count on
/// as [`Input::OomKills`] when it differs from the last one sent for that scope (the file also
/// changes on every `max` and `high` event while a scope sits at its cap).
fn new_watcher(
    inputs: tokio_mpsc::UnboundedSender<Input>,
) -> notify::Result<notify::RecommendedWatcher> {
    let mut sent: HashMap<PathBuf, u64> = HashMap::new();
    notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if !matches!(event.kind, notify::EventKind::Modify(_)) {
            return;
        }
        for path in &event.paths {
            let Some(cgroup) = path.parent() else {
                continue;
            };
            let Some(count) = memory::read_oom_kills(cgroup) else {
                continue;
            };
            if sent.insert(cgroup.to_path_buf(), count) == Some(count) {
                continue;
            }
            let _ = inputs.send(Input::OomKills {
                cgroup: cgroup.to_path_buf(),
                count,
            });
        }
    })
}

/// The note a session gets when it is restarted after an OOM kill: one line, then the subagent
/// work that was in flight, if any.
fn restart_note(cap_gb: u32, work: &[String]) -> String {
    let mut note = format!(
        "You were killed for running out of memory (cap {cap_gb} GB) and have been restarted."
    );
    if !work.is_empty() {
        note.push_str(
            "\nThis subagent work was in flight and stopped with you; restart what still \
             matters:\n",
        );
        note.push_str(&work.join("\n"));
    }
    note
}

/// A subagent that was still running when its session died, for the inbox item and the note.
fn in_flight_line(agent: &dovetail_api::agents::agent::Agent) -> String {
    let what = agent
        .progress
        .as_ref()
        .map(|p| p.description.as_str())
        .filter(|d| !d.is_empty())
        .unwrap_or(&agent.name);
    let mut line = format!("- {what}");
    let background = matches!(agent.kind, AgentKind::Subagent { background: true });
    let output = agent.progress.as_ref().and_then(|p| p.output_file.as_ref());
    match (background, output) {
        (true, Some(file)) => line.push_str(&format!(" (background; output: {})", file.display())),
        (true, None) => line.push_str(" (background)"),
        (false, Some(file)) => line.push_str(&format!(" (output: {})", file.display())),
        (false, None) => {}
    }
    line
}

impl Core {
    /// Watches a running capped session's `memory.events`, once its scope exists.
    pub(crate) fn watch_oom(&mut self, id: &AgentId) {
        let Some(live) = self.sessions.live.get(id) else {
            return;
        };
        if live.oom_watched {
            return;
        }
        let Some(file) = live.cgroup.as_deref().map(memory::events_file) else {
            return;
        };
        if !file.exists() {
            return; // the scope is not up yet; the next sample tries again
        }
        if self.sessions.oom.watcher.is_none() {
            match new_watcher(self.inputs.clone()) {
                Ok(w) => self.sessions.oom.watcher = Some(w),
                Err(e) => {
                    tracing::warn!("watching memory.events: {e}; OOM kills seen on samples only");
                    return;
                }
            }
        }
        let Some(watcher) = self.sessions.oom.watcher.as_mut() else {
            return;
        };
        match watcher.watch(&file, notify::RecursiveMode::NonRecursive) {
            Ok(()) => {
                if let Some(live) = self.sessions.live.get_mut(id) {
                    live.oom_watched = true;
                }
            }
            Err(e) => tracing::warn!(agent = %id.0, "watching memory.events: {e}"),
        }
    }

    /// Stops watching a scope's `memory.events` (its session exited).
    pub(crate) fn unwatch_oom(&mut self, cgroup: &Path) {
        if let Some(w) = self.sessions.oom.watcher.as_mut() {
            // The watch is usually gone already with the scope.
            let _ = w.unwatch(&memory::events_file(cgroup));
        }
    }

    /// Keeps a capped session's SIGKILL exit, reported as a crash, open for a late `oom_kill`
    /// count. Its scope stays watched until then.
    pub(crate) fn remember_exit(&mut self, cgroup: PathBuf, exit: UnexplainedExit) {
        self.prune_exits();
        self.sessions.oom.exits.insert(cgroup, exit);
    }

    /// Forgets exits older than the grace: no OOM kill explains them.
    fn prune_exits(&mut self) {
        let now = Instant::now();
        let stale: Vec<PathBuf> = self
            .sessions
            .oom
            .exits
            .iter()
            .filter(|(_, e)| now.saturating_duration_since(e.at) > memory::OOM_GRACE)
            .map(|(cgroup, _)| cgroup.clone())
            .collect();
        for cgroup in stale {
            self.sessions.oom.exits.remove(&cgroup);
            self.unwatch_oom(&cgroup);
        }
    }

    /// A scope's `oom_kill` count as the watcher read it.
    pub(crate) fn on_oom_kills(&mut self, cgroup: PathBuf, count: u64) {
        // The count may arrive after the session's exit was handled: the kill was its own.
        self.prune_exits();
        if self
            .sessions
            .oom
            .exits
            .get(&cgroup)
            .is_some_and(|e| count > e.kills)
            && let Some(exit) = self.sessions.oom.exits.remove(&cgroup)
        {
            self.unwatch_oom(&cgroup);
            tracing::warn!(agent = %exit.agent.0, count, "OOM kill seen after the session's exit");
            self.resolve(exit.item, None);
            let restartable = exit.restartable && !self.sessions.live.contains_key(&exit.agent);
            self.session_oom(&exit.agent, exit.work, restartable);
            return;
        }
        let id = self
            .sessions
            .live
            .iter()
            .find(|(_, l)| l.cgroup.as_deref() == Some(cgroup.as_path()))
            .map(|(id, _)| id.clone());
        match id {
            Some(id) => self.oom_count(&id, count),
            None => tracing::debug!(cgroup = %cgroup.display(), count, "oom_kill of no session"),
        }
    }

    /// A session's scope reports `count` OOM kills so far: a rise waits for its verdict.
    pub(crate) fn oom_count(&mut self, id: &AgentId, count: u64) {
        let Some(live) = self.sessions.live.get_mut(id) else {
            return;
        };
        if count <= live.oom_kills {
            return;
        }
        tracing::warn!(agent = %id.0, count, "OOM kill in the session's scope");
        live.oom_kills = count;
        live.oom_seen.get_or_insert_with(Instant::now);
        let at = tokio::time::Instant::now() + memory::OOM_GRACE;
        self.sessions.oom.check_at = Some(self.sessions.oom.check_at.map_or(at, |c| c.min(at)));
    }

    /// The grace after a kill is over: sessions still running lost a tool, not themselves.
    pub(crate) fn check_oom(&mut self) {
        self.sessions.oom.check_at = None;
        let now = Instant::now();
        let mut tools = Vec::new();
        let mut next = None::<Instant>;
        for (id, live) in &mut self.sessions.live {
            let Some(seen) = live.oom_seen else { continue };
            match memory::oom_verdict(seen, now, None) {
                Some(_) => {
                    live.oom_seen = None;
                    tools.push(id.clone());
                }
                None => {
                    let due = seen + memory::OOM_GRACE;
                    next = Some(next.map_or(due, |n| n.min(due)));
                }
            }
        }
        if let Some(due) = next {
            self.sessions.oom.check_at = Some(tokio::time::Instant::from_std(due));
        }
        for id in tools {
            self.tool_oom(&id);
        }
    }

    /// The kernel killed one of the session's tools for memory; the session runs on.
    fn tool_oom(&mut self, id: &AgentId) {
        let cap = self.cap_gb(id);
        let text = format!(
            "Out of memory: the kernel killed a process {} ran (its cap is {cap} GB). The \
             session is still running; the tool call that started the process failed.",
            self.name_of(id)
        );
        self.raise(id, InboxKind::Error { text }, Waiter::None);
        self.activity(
            ActivityKind::Lifecycle,
            Some(id),
            format!("a tool of {} ran out of memory", id.0),
        );
    }

    /// At a session's exit: whether an OOM kill took it (`Some(Session)`), took a tool just
    /// before (`Some(Tool)`, and the exit is handled as usual), or played no part as far as
    /// is known yet (`None`; see [`Core::remember_exit`]). The caller unwatches the scope.
    pub(crate) fn oom_at_exit(
        &mut self,
        id: &AgentId,
        live: &crate::session::Live,
        signal: Option<i32>,
    ) -> Option<OomKill> {
        let cgroup = live.cgroup.as_deref()?;
        if live.stopping {
            return None;
        }
        // The watcher may have missed it; the scope may still be there to read.
        let seen = live.oom_seen.or_else(|| {
            memory::read_oom_kills(cgroup)
                .filter(|c| *c > live.oom_kills)
                .map(|_| Instant::now())
        })?;
        let verdict = memory::oom_verdict(seen, Instant::now(), Some(signal));
        if verdict == Some(OomKill::Tool) {
            self.tool_oom(id);
        }
        verdict
    }

    /// The subagents of `id` still running, one line each.
    pub(crate) fn in_flight(&self, id: &AgentId) -> Vec<String> {
        self.agents
            .rows
            .values()
            .filter(|r| r.agent.parent.as_ref() == Some(id))
            .filter(|r| !matches!(r.agent.status, AgentStatus::Done | AgentStatus::Error))
            .map(|r| in_flight_line(&r.agent))
            .collect()
    }

    /// The kernel killed the session itself for memory: say so in Needs you and resume it
    /// from its session id, unless it was restarted that way within the guard's window.
    /// `restartable`: nothing else (a quit, an archive) is about to end it anyway.
    pub(crate) fn session_oom(&mut self, id: &AgentId, work: Vec<String>, restartable: bool) {
        let name = self.name_of(id);
        let cap = self.cap_gb(id);
        let minutes = memory::RESTART_WINDOW.as_secs() / 60;
        self.update_agent(id, |row| row.agent.status = AgentStatus::Error);
        self.activity(
            ActivityKind::Lifecycle,
            Some(id),
            format!("{} ran out of memory", id.0),
        );
        let mut text =
            format!("Out of memory: the kernel killed {name}'s session (its cap is {cap} GB). ");
        let restarted = if !restartable {
            text.push_str("It was not restarted.");
            false
        } else if !self.sessions.oom.restarts.allowed(id, Instant::now()) {
            text.push_str(&format!(
                "It was not restarted: it already ran out of memory less than {minutes} minutes \
                 after an automatic restart. Write to it to resume it."
            ));
            false
        } else {
            match self.ensure_live(id) {
                Ok(()) => {
                    // Only a restart that happened uses up the allowance.
                    self.sessions.oom.restarts.record(id, Instant::now());
                    text.push_str("It was restarted from its session.");
                    true
                }
                Err(e) => {
                    text.push_str(&format!("Restarting it failed: {e}"));
                    false
                }
            }
        };
        if !work.is_empty() {
            text.push_str("\n\nSubagent work in flight when it died:\n");
            text.push_str(&work.join("\n"));
        }
        self.raise(id, InboxKind::Error { text }, Waiter::None);
        if restarted {
            self.update_agent(id, |row| row.agent.status = AgentStatus::Idle);
            // Every automatic restart tells the session (PR #31, the user's answer to Q1).
            self.send_note(id, &restart_note(cap, &work));
        }
        self.refresh_status(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::agents::agent::SubagentProgress;

    fn subagent(
        background: bool,
        description: &str,
        output: Option<&str>,
    ) -> dovetail_api::agents::agent::Agent {
        let mut agent = crate::engine::new_agent(
            AgentId("c:t1".into()),
            "general-purpose".into(),
            AgentKind::Subagent { background },
            None,
        );
        agent.progress = Some(SubagentProgress {
            description: description.into(),
            output_file: output.map(PathBuf::from),
            ..SubagentProgress::default()
        });
        agent
    }

    #[test]
    fn every_restart_note_says_why_and_lists_work_only_when_there_is_some() {
        assert_eq!(
            restart_note(3, &[]),
            "You were killed for running out of memory (cap 3 GB) and have been restarted."
        );
        let note = restart_note(4, &["- Survey (background)".into()]);
        let mut lines = note.lines();
        assert_eq!(
            lines.next(),
            Some("You were killed for running out of memory (cap 4 GB) and have been restarted.")
        );
        assert!(lines.next().is_some_and(|l| l.contains("in flight")));
        assert_eq!(lines.next(), Some("- Survey (background)"));
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn in_flight_work_names_what_it_was_doing_and_where_its_output_is() {
        assert_eq!(
            in_flight_line(&subagent(true, "Run the test suite", Some("/t/out.jsonl"))),
            "- Run the test suite (background; output: /t/out.jsonl)"
        );
        assert_eq!(
            in_flight_line(&subagent(true, "Survey", None)),
            "- Survey (background)"
        );
        assert_eq!(
            in_flight_line(&subagent(false, "Read the docs", None)),
            "- Read the docs"
        );
        // No description: its name stands in.
        assert_eq!(
            in_flight_line(&subagent(false, "", None)),
            "- general-purpose"
        );
    }
}
