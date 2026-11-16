//! The end-of-day journal and the fresh coordinator of a new day (ADR 0012, the minimal slice of
//! decisions 17 and 22 pulled into v1).
//!
//! - **The file.** `<data>/projects/<id>/journal/YYYY-MM-DD.md`, local date. Free-form markdown:
//!   a file the coordinator wrote by hand counts as a handoff as it is. A block Dovetail writes
//!   starts with [`MARKER`] and holds a "Handoff" section (the coordinator's reply) and a "Facts"
//!   section (Dovetail's own data). Writing never replaces a file: a second `:eod` the same day
//!   appends its block below the first.
//! - **`:eod`.** The coordinator is asked for its handoff; when its turn ends, its reply becomes
//!   the Handoff section. If it cannot answer, the file still gets the facts and says why.
//! - **`:fresh`.** The manual, mid-day half of decision 22: the same handoff request, appended
//!   as a "Handoff (fresh start HH:MM)" block, then a new coordinator session seeded as the
//!   morning one is. The previous session stays resumable (one Needs-you item).
//! - **A new day.** On open, when the newest journal file is from an earlier day and the
//!   coordinator was last used before today, the coordinator gets a new Claude Code session
//!   seeded with that file's handoff and the open tasks. Yesterday's session is kept: one
//!   Needs-you item (`y`) goes back to it.

use std::path::{Path, PathBuf};

use dovetail_api::{
    Agent, AgentId, AgentKind, AgentStatus, Bucket, EntryKind, FRESH_COORDINATOR_PERMISSION,
    InboxItem, InboxKind, JournalState, KEEP_COORDINATOR_PERMISSION, Task, TaskStatus,
};
use jiff::civil::Date;
use serde_json::json;

use crate::inbox::Waiter;
use crate::service::{COORDINATOR, Core, Input};

/// The first line of every block Dovetail writes into a journal file. A file without it was
/// written by hand, and all of it is the handoff.
pub(crate) const MARKER: &str = "<!-- dovetail: end of day -->";

/// The heading of the coordinator's part of a block.
const HANDOFF_HEADING: &str = "## Handoff";

/// Today's date in the system's time zone.
pub(crate) fn today() -> Date {
    jiff::Zoned::now().date()
}

/// The local date of a Unix time in milliseconds.
pub(crate) fn date_of(ms: i64) -> Option<Date> {
    let at = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(at.to_zoned(jiff::tz::TimeZone::system()).date())
}

/// The file for `date` in the journal directory `dir`.
pub(crate) fn file(dir: &Path, date: Date) -> PathBuf {
    dir.join(format!("{date}.md"))
}

/// The newest dated file in `dir` (`YYYY-MM-DD.md`); other files are ignored. `None` when the
/// directory is missing or holds none.
pub(crate) fn latest(dir: &Path) -> Option<Date> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".md")?.parse::<Date>().ok()
        })
        .max()
}

/// [`latest`] with when its file was last written. `None` when the time cannot be read, so a
/// file whose age is unknown is not trusted to start a fresh session.
pub(crate) fn newest(dir: &Path) -> Option<Newest> {
    let date = latest(dir)?;
    let written = std::fs::metadata(file(dir, date)).ok()?.modified().ok()?;
    let written_ms = i64::try_from(
        written
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis(),
    )
    .ok()?;
    Some(Newest { date, written_ms })
}

/// How long after the coordinator's last real work a journal file still counts as closing it.
/// The handoff's own tool summary can land after the file is written, a hand-written file is
/// followed by "done", and a `:fresh` seed is answered a moment after its block is written.
pub(crate) const CLOSE_GRACE_MS: i64 = 10 * 60 * 1000;

/// When the coordinator was last used, read before anything is added to its conversation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LastUse {
    /// The local date of its newest entry of any kind: the day it was last open.
    pub(crate) date: Option<Date>,
    /// The start of its newest entry that is not a Dovetail notice: when the user and the
    /// coordinator last worked, in Unix milliseconds.
    pub(crate) work_ms: Option<i64>,
}

/// The newest journal file: its day and when it was last written, in Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Newest {
    pub(crate) date: Date,
    pub(crate) written_ms: i64,
}

/// What the first open of `today` does with the coordinator (ADR 0012 and its 2026-11-11
/// amendment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Morning {
    /// Resume the session as before: nothing to decide.
    Resume,
    /// A new session seeded with the journal of this day.
    Fresh(Date),
    /// The coordinator was used on this day and the newest journal is older than that use:
    /// the day was never closed, so its session is kept and the user is told.
    Unclosed(Date),
}

/// Decides the first open of `today`. A fresh coordinator needs a journal file from an earlier
/// day, a coordinator last used before today, and the file at least as new as that use (within
/// [`CLOSE_GRACE_MS`]). With an older file the day was not closed, and starting from it would
/// lose that day.
pub(crate) fn morning(newest: Option<Newest>, used: LastUse, today: Date) -> Morning {
    let (Some(newest), Some(date)) = (newest, used.date) else {
        return Morning::Resume;
    };
    if newest.date >= today || date >= today {
        return Morning::Resume;
    }
    match used.work_ms {
        Some(work) if newest.written_ms + CLOSE_GRACE_MS < work => Morning::Unclosed(date),
        _ => Morning::Fresh(newest.date),
    }
}

/// `day` as a person says it: its weekday, with the date when that would be ambiguous.
fn day_name(day: Date, today: Date) -> String {
    if today.since(day).is_ok_and(|s| s.get_days() < 7) {
        day.strftime("%A").to_string()
    } else {
        day.strftime("%A %-d %B").to_string()
    }
}

/// The handoff in a journal file: the Handoff section of the last block Dovetail wrote, or the
/// whole file when Dovetail wrote none (the coordinator wrote it by hand).
pub(crate) fn handoff(text: &str) -> &str {
    let Some(at) = text.rfind(MARKER) else {
        return text.trim();
    };
    let block = &text[at + MARKER.len()..];
    let Some((start, len)) = find_handoff_heading(block) else {
        return block.trim();
    };
    let body = &block[start + len..];
    let end = body
        .match_indices("\n## ")
        .map(|(i, _)| i)
        .next()
        .unwrap_or(body.len());
    body[..end].trim()
}

/// The offset and length of the first Handoff heading line of `text`: `## Handoff`, or
/// `## Handoff (fresh start HH:MM)` in a block `:fresh` wrote.
fn find_handoff_heading(text: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for l in text.split_inclusive('\n') {
        let line = l.trim_end();
        if line == HANDOFF_HEADING
            || (line.starts_with("## Handoff (fresh start ") && line.ends_with(')'))
        {
            return Some((offset, line.len()));
        }
        offset += l.len();
    }
    None
}

/// Why a closed day has no handoff from the coordinator.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NoHandoff {
    #[error("the coordinator has not run yet")]
    NeverRan,
    #[error("it could not be asked: {0}")]
    NotSent(String),
    #[error("its turn was interrupted")]
    Interrupted,
    #[error("its turn ended with no reply")]
    NoReply,
    #[error("its session ended before it answered")]
    SessionEnded,
}

/// The block Dovetail writes for a closed day. `handoff` is the coordinator's reply, or why it
/// has none.
pub(crate) fn block(handoff: Result<&str, &NoHandoff>, facts: &str) -> String {
    block_titled(HANDOFF_HEADING, handoff, facts)
}

/// [`block`] with the Handoff section's heading given: `## Handoff (fresh start 14:05)` for a
/// mid-day fresh start.
fn block_titled(heading: &str, handoff: Result<&str, &NoHandoff>, facts: &str) -> String {
    let handoff = match handoff {
        Ok(text) => text.trim().to_owned(),
        Err(why) => format!("_No handoff from the coordinator: {why}._"),
    };
    format!(
        "{MARKER}\n\n{heading}\n\n{handoff}\n\n## Facts\n\n{}\n",
        facts.trim()
    )
}

/// Writes `block` as the journal file for `date`, or appends it to the file already there: a
/// journal file is never replaced. The new content goes to a temporary file first and is renamed
/// over the old one, so a failed write leaves the old file as it was.
pub(crate) fn write(dir: &Path, date: Date, block: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = file(dir, date);
    let text = match std::fs::read_to_string(&path) {
        Ok(old) => format!("{}\n\n---\n\n{block}", old.trim_end()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => format!("# {date}\n\n{block}"),
        Err(e) => return Err(e),
    };
    let tmp = dir.join(format!(".{date}.md.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Open tasks by bucket, Now first, in board order.
pub(crate) fn open_tasks(tasks: &[Task]) -> String {
    let mut open: Vec<&Task> = tasks
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Todo | TaskStatus::InProgress | TaskStatus::Review
            )
        })
        .collect();
    if open.is_empty() {
        return "No open tasks.".into();
    }
    // `None` (Unsorted) sorts after Later.
    open.sort_by_key(|t| (t.bucket.is_none(), t.bucket, t.order, t.id.0));
    let mut out = String::new();
    let mut current: Option<Option<Bucket>> = None;
    for task in open {
        if current != Some(task.bucket) {
            current = Some(task.bucket);
            let name = match task.bucket {
                Some(Bucket::Now) => "Now",
                Some(Bucket::Next) => "Next",
                Some(Bucket::Later) => "Later",
                None => "Unsorted",
            };
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("**{name}**\n\n"));
        }
        let mut tags = Vec::new();
        if let Some(p) = task.p_level {
            tags.push(format!("{p:?}"));
        }
        if task.status == TaskStatus::InProgress {
            tags.push("in progress".into());
        }
        if task.status == TaskStatus::Review {
            tags.push("in review".into());
        }
        if let Some(pr) = task.pr {
            tags.push(format!("PR #{pr}"));
        }
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!(" ({})", tags.join(", "))
        };
        out.push_str(&format!("- #{} {}{tags}\n", task.id.0, task.title));
    }
    out.trim_end().to_owned()
}

/// Dovetail's facts for the day's block: open tasks by bucket, slices and their status, and
/// what is left in Needs you.
pub(crate) fn facts(tasks: &[Task], agents: &[&Agent], inbox: &[&InboxItem]) -> String {
    let mut out = format!("### Open tasks\n\n{}\n\n### Slices\n\n", open_tasks(tasks));
    let slices: Vec<&&Agent> = agents
        .iter()
        .filter(|a| a.kind == AgentKind::Slice)
        .collect();
    if slices.is_empty() {
        out.push_str("No slices.\n");
    }
    for slice in slices {
        let task = slice
            .task
            .map(|t| format!(" on #{}", t.0))
            .unwrap_or_default();
        let line = if slice.status_line.is_empty() {
            String::new()
        } else {
            format!(": {}", slice.status_line)
        };
        out.push_str(&format!(
            "- {}{task}, {}{line}\n",
            slice.name,
            status_word(slice.status)
        ));
    }
    out.push_str("\n### Needs you\n\n");
    if inbox.is_empty() {
        out.push_str("Nothing left.\n");
    }
    let name = |id: &AgentId| {
        agents
            .iter()
            .find(|a| &a.id == id)
            .map_or_else(|| id.0.clone(), |a| a.name.clone())
    };
    for item in inbox {
        let what = match &item.kind {
            InboxKind::Permission { tool, .. } => format!("permission: {tool}"),
            InboxKind::Question { text } => format!("question: {}", first_line(text)),
            InboxKind::FinishedTurn { preview } => {
                format!("finished turn: {}", first_line(preview))
            }
            InboxKind::Error { text } => format!("error: {}", first_line(text)),
            InboxKind::Proposal { kind, text, .. } => {
                format!("{} proposal: {}", kind.as_str(), first_line(text))
            }
            InboxKind::UnfiledFinding { label, sources, .. } => {
                format!("unfiled finding: {label} ({sources} sources)")
            }
            InboxKind::NewTangents { count } => format!("{count} new tangents to triage"),
        };
        out.push_str(&format!("- {}, {what}\n", name(&item.agent)));
    }
    out.trim_end().to_owned()
}

fn status_word(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::SettingUp => "setting up",
        AgentStatus::Working => "working",
        AgentStatus::Idle => "idle",
        AgentStatus::NeedsYou => "needs you",
        AgentStatus::Done => "done",
        AgentStatus::Error => "failed",
    }
}

/// Longest line quoted in the facts.
const LINE_MAX: usize = 100;

fn first_line(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or("").trim();
    if line.chars().count() > LINE_MAX {
        let cut: String = line.chars().take(LINE_MAX - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

/// What `:eod` asks the coordinator.
fn eod_prompt(date: Date) -> String {
    format!(
        "The user is closing the day. Write your handoff for tomorrow's fresh coordinator, \
         under a page of markdown: the open threads; what you were about to do; decisions in \
         flight and why; anything the user asked you to remember; what matters first tomorrow. \
         Reply with the handoff only: Dovetail saves your reply as the Handoff section of \
         journal/{date}.md, next to its own facts (tasks, slices, Needs you). Do not write the \
         file yourself, and start no other work."
    )
}

/// Why a coordinator gets a new session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fresh {
    /// The first open of a new day: the handoff is in the file of this earlier `date`.
    NewDay { date: Date },
    /// `:fresh` at `time` (HH:MM) today: the handoff is the last block of today's file.
    MidDay { date: Date, time: String },
}

impl Fresh {
    /// The date of the journal file that holds the handoff.
    fn date(&self) -> Date {
        match self {
            Fresh::NewDay { date } | Fresh::MidDay { date, .. } => *date,
        }
    }
}

/// A `:fresh` whose handoff is written: the coordinator's old session is stopping, and the new
/// one starts when it has exited ([`Core::coordinator_exited`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingFresh {
    kind: Fresh,
    seed: String,
}

/// The first prompt of a fresh coordinator: a new day's, or a mid-day `:fresh`'s. Both hold the
/// handoff and the open tasks.
fn seed_prompt(today: Date, kind: &Fresh, handoff: &str, tasks: &str) -> String {
    let date = kind.date();
    let intro = match kind {
        Fresh::NewDay { .. } => format!(
            "This is a fresh coordinator session for {today}. Yesterday's coordinator left the \
             handoff below in journal/{date}.md; its conversation is kept but not continued."
        ),
        Fresh::MidDay { time, .. } => format!(
            "This is a fresh coordinator session, started at {time} on {today}. The previous \
             coordinator left the handoff below in journal/{date}.md; its conversation is kept \
             but not continued."
        ),
    };
    format!(
        "{intro}\n\n# Handoff from {date}\n\n{handoff}\n\n# Open tasks\n\n{tasks}\n\n\
         Read this, check anything you need to, then tell the user in a few lines what you \
         would pick up first. Start no work until they answer."
    )
}

/// An `:eod` waiting for the coordinator's handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Closing {
    /// The day being closed.
    pub(crate) date: Date,
    /// The coordinator's reply so far: the text of its latest `result`.
    pub(crate) reply: Option<String>,
    /// Set when no more is coming: the reply is final, or why there is none.
    pub(crate) done: Option<Result<String, NoHandoff>>,
    /// `Some("HH:MM")` for a `:fresh`: the handoff also starts a new coordinator session.
    pub(crate) fresh: Option<String>,
}

impl Core {
    /// The project's journal directory; `None` for a folder not registered yet.
    fn journal_dir(&self) -> Option<PathBuf> {
        let project = self.project.registered()?;
        Some(self.paths.journal(&project.id))
    }

    /// The journal as the UI sees it.
    pub(crate) fn journal_state(&self) -> Option<JournalState> {
        let dir = self.journal_dir()?;
        Some(JournalState {
            latest: latest(&dir).map(|d| d.to_string()),
            closing: self.closing.is_some(),
            fresh: self.closing.as_ref().is_some_and(|c| c.fresh.is_some()),
        })
    }

    fn emit_journal(&mut self) {
        if let Some(state) = self.journal_state() {
            self.emit(dovetail_api::Event::JournalChanged(state));
        }
    }

    /// `:eod`: asks the coordinator for its handoff. The file is written once its turn ends
    /// ([`Core::closing_turn_ended`]), or at once with the facts alone if it cannot answer.
    pub(crate) fn end_of_day(&mut self) {
        self.ask_for_handoff(None);
    }

    /// `:fresh`: the same request, and a new coordinator once the handoff is written. Refused
    /// while the coordinator's turn runs (it is not interrupted) or before it has run at all.
    pub(crate) fn fresh_start(&mut self) {
        let id = AgentId(COORDINATOR.into());
        if self.live.get(&id).is_some_and(|l| l.turn.running) {
            self.fail("turn running; try again when it finishes");
            return;
        }
        if self
            .agents
            .get(&id)
            .is_none_or(|r| r.transcript_path.is_none())
        {
            self.fail("the coordinator has not run yet, so there is no handoff to carry over");
            return;
        }
        let time = jiff::Zoned::now().strftime("%H:%M").to_string();
        self.ask_for_handoff(Some(time));
    }

    fn ask_for_handoff(&mut self, fresh: Option<String>) {
        if self.journal_dir().is_none() {
            self.fail("not a Dovetail project yet, so it has no journal: :project add keeps it");
            return;
        }
        if self.closing.is_some() {
            self.fail("already closing the day: waiting for the coordinator's handoff");
            return;
        }
        if self.pending_fresh.is_some() {
            self.fail("a fresh coordinator is starting");
            return;
        }
        let date = today();
        self.closing = Some(Closing {
            date,
            reply: None,
            done: None,
            fresh,
        });
        self.emit_journal();
        let id = AgentId(COORDINATOR.into());
        let has_run = self
            .agents
            .get(&id)
            .is_some_and(|r| r.transcript_path.is_some());
        let sent = if !has_run {
            Err(NoHandoff::NeverRan)
        } else {
            let prompt = eod_prompt(date);
            self.send_text(
                &id,
                &prompt,
                EntryKind::Notice,
                format!("Dovetail: {prompt}"),
            )
            .map_err(|e| NoHandoff::NotSent(e.to_string()))
        };
        if let Err(why) = sent {
            self.closing_done(Err(why));
        }
    }

    /// A `result` from the coordinator while the day closes: its text is the handoff so far.
    /// `last` is true when the session has nothing left to do, so the reply is final.
    pub(crate) fn closing_result(&mut self, text: Option<String>, interrupted: bool, last: bool) {
        let Some(closing) = &mut self.closing else {
            return;
        };
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            closing.reply = Some(text);
        }
        if !last {
            return;
        }
        let reply = match closing.reply.take() {
            Some(text) if !interrupted => Ok(text),
            _ if interrupted => Err(NoHandoff::Interrupted),
            _ => Err(NoHandoff::NoReply),
        };
        self.closing_done(reply);
    }

    /// The coordinator's session ended while the day closes.
    pub(crate) fn closing_exit(&mut self) {
        if self.closing.as_ref().is_some_and(|c| c.done.is_none()) {
            self.closing_done(Err(NoHandoff::SessionEnded));
        }
    }

    /// No more is coming for the handoff: the file is written from the loop
    /// ([`Input::CloseDay`]), which can read the tasks.
    fn closing_done(&mut self, reply: Result<String, NoHandoff>) {
        if let Some(closing) = &mut self.closing {
            closing.done = Some(reply);
            let _ = self.inputs.send(Input::CloseDay);
        }
    }

    /// Writes the day's block and tells the UI.
    pub(crate) async fn write_journal(&mut self) {
        let Some(Closing {
            date,
            done: Some(reply),
            fresh,
            ..
        }) = self.closing.take()
        else {
            return;
        };
        let Some(dir) = self.journal_dir() else {
            return;
        };
        let tasks = match self.db.load_tasks().await {
            Ok(tasks) => tasks,
            Err(e) => {
                tracing::warn!("reading tasks for the journal: {e}");
                Vec::new()
            }
        };
        let agents: Vec<&Agent> = self.agents.values().map(|r| &r.agent).collect();
        let inbox: Vec<&InboxItem> = self.inbox.values().map(|e| &e.item).collect();
        let facts = facts(&tasks, &agents, &inbox);
        // A fresh start without a handoff would lose the context it exists to carry over: the
        // old session stays, and nothing is written.
        if let (Some(_), Err(why)) = (&fresh, &reply) {
            self.fail(format!("fresh start cancelled, no handoff: {why}"));
            self.emit_journal();
            return;
        }
        let text = match &fresh {
            Some(time) => block_titled(
                &format!("{HANDOFF_HEADING} (fresh start {time})"),
                reply.as_deref(),
                &facts,
            ),
            None => block(reply.as_deref(), &facts),
        };
        match write(&dir, date, &text) {
            Ok(path) => {
                tracing::info!(path = %path.display(), "journal written");
                let note = match (&reply, &fresh) {
                    (Ok(_), Some(time)) => {
                        format!("fresh start at {time}: handoff added to journal/{date}.md")
                    }
                    (Ok(_), None) => format!("day closed: journal/{date}.md written"),
                    (Err(why), _) => {
                        format!("day closed without a handoff ({why}): journal/{date}.md written")
                    }
                };
                self.activity(dovetail_api::ActivityKind::Lifecycle, None, note);
                if let (Some(time), Ok(handoff)) = (fresh, &reply) {
                    let kind = Fresh::MidDay { date, time };
                    let seed = seed_prompt(today(), &kind, handoff.trim(), &open_tasks(&tasks));
                    self.begin_fresh(PendingFresh { kind, seed });
                }
            }
            Err(e) => self.fail(format!("writing journal/{date}.md: {e}")),
        }
        self.emit_journal();
    }

    /// Stops the running session; the new one starts once it has exited.
    fn begin_fresh(&mut self, pending: PendingFresh) {
        let id = AgentId(COORDINATOR.into());
        self.pending_fresh = Some(pending);
        match self.live.get_mut(&id) {
            Some(live) => {
                live.stopping = true;
                live.session.shutdown(crate::shutdown::QUIT_GRACE);
            }
            None => self.coordinator_exited(),
        }
    }

    /// When the coordinator was last used, from its newest conversation entries.
    pub(crate) fn coordinator_last_used(&self) -> LastUse {
        let id = AgentId(COORDINATOR.into());
        let Some(tail) = self.tails.get(&id) else {
            return LastUse::default();
        };
        LastUse {
            date: tail.entries.last().and_then(|e| date_of(e.at_ms)),
            work_ms: tail
                .entries
                .iter()
                .rev()
                .find(|e| !matches!(e.kind, EntryKind::Notice | EntryKind::AgentMessage))
                .map(|e| e.at_ms),
        }
    }

    /// Starts the coordinator for the day. On the first open of a new day (see [`morning`]) it
    /// gets a new session seeded with the newest journal file, or, when that file is older than
    /// the coordinator's last use, keeps its session and says in Needs you that the day was not
    /// closed. Otherwise yesterday's session resumes as before. `last_used` is read before
    /// anything was added to its conversation.
    pub(crate) async fn start_coordinator(&mut self, last_used: LastUse) {
        let id = AgentId(COORDINATOR.into());
        let today = today();
        // A coordinator recorded in another folder (the repo, before ADR 0021) cannot resume
        // in its own: it starts fresh there, seeded as `:fresh` seeds one.
        let folder = self.project.coordinator_folder(&self.paths);
        if self.filed_elsewhere(&id, &folder) {
            self.moved_coordinator(today).await;
            return;
        }
        let newest = self.journal_dir().and_then(|dir| newest(&dir));
        let fresh = match morning(newest, last_used, today) {
            Morning::Fresh(date) => Some(date),
            Morning::Unclosed(day) => {
                let text = format!(
                    "{} wasn't closed; kept that coordinator. `:eod` to close it.",
                    day_name(day, today)
                );
                let open = self
                    .inbox
                    .values()
                    .any(|p| matches!(&p.item.kind, InboxKind::Error { text: t } if *t == text));
                if !open {
                    self.raise(&id, InboxKind::Error { text }, Waiter::None);
                }
                None
            }
            Morning::Resume => None,
        };
        let Some((date, path)) = fresh
            .zip(self.journal_dir())
            .map(|(d, dir)| (d, file(&dir, d)))
        else {
            if let Err(e) = self.ensure_live(&id) {
                self.fail(e);
            }
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                // Nothing to seed it with: yesterday's session carries on.
                self.fail(format!("reading {}: {e}", path.display()));
                if let Err(e) = self.ensure_live(&id) {
                    self.fail(e);
                }
                return;
            }
        };
        let tasks = self.db.load_tasks().await.unwrap_or_default();
        let kind = Fresh::NewDay { date };
        let seed = seed_prompt(today, &kind, handoff(&text), &open_tasks(&tasks));
        self.fresh_coordinator(&kind, &seed);
    }

    /// The coordinator moved to its own folder (ADR 0021 point 8): a new session there, seeded
    /// with the newest handoff, if any, and the open tasks. The old session stays on disk; a
    /// Needs-you note says how to reopen it by hand, since Claude Code resumes it only from the
    /// folder it ran in.
    async fn moved_coordinator(&mut self, today: Date) {
        let id = AgentId(COORDINATOR.into());
        let Some(row) = self.agents.get(&id) else {
            return;
        };
        let old_session = row.session_id.clone().unwrap_or_default();
        let old_transcript = row.transcript_path.clone();
        self.update_agent(&id, |row| {
            row.session_id = Some(uuid::Uuid::new_v4().to_string());
            row.transcript_path = None;
        });
        let folder = self.project.coordinator_folder(&self.paths);
        let repos: Vec<String> = self
            .project
            .repos()
            .iter()
            .map(|r| format!("`{}` ({})", r.name, r.path.display()))
            .collect();
        let latest_handoff = self.journal_dir().and_then(|dir| {
            let date = latest(&dir)?;
            let text = std::fs::read_to_string(file(&dir, date)).ok()?;
            Some((date, handoff(&text).to_owned()))
        });
        let handoff_block = match &latest_handoff {
            Some((date, text)) => format!("# Handoff from {date}\n\n{text}"),
            None => "No handoff was written (`:eod` writes one).".to_owned(),
        };
        let tasks = self.db.load_tasks().await.unwrap_or_default();
        let attached = if repos.is_empty() {
            "It has no repos.".to_owned()
        } else {
            format!("Its repos are attached: {}.", repos.join(", "))
        };
        let seed = format!(
            "This is a fresh coordinator session for {today}. Dovetail now runs the coordinator \
             in its own folder, `{}`, not in a checkout. {attached} When you start a slice, \
             name its repo with `repo` if there are several. The previous coordinator's \
             conversation is kept but not continued.\n\n{handoff_block}\n\n# Open tasks\n\n{}\n\n\
             Read this, check anything you need to, then tell the user in a few lines what you \
             would pick up first. Start no work until they answer.",
            folder.display(),
            open_tasks(&tasks)
        );
        tracing::info!("coordinator moved to {}", folder.display());
        self.notice(
            &id,
            format!(
                "The coordinator now runs in {}: a fresh session started, seeded with the open \
                 tasks{}.",
                folder.display(),
                if latest_handoff.is_some() {
                    " and the newest handoff"
                } else {
                    ""
                }
            ),
        );
        if let Err(e) = self.send_text(&id, &seed, EntryKind::Notice, format!("Dovetail: {seed}")) {
            self.fail(e);
        }
        let reopen = match &old_transcript {
            Some(path) => format!(
                " Its transcript is {}; `claude --resume {old_session}` in the folder it ran in \
                 reopens it.",
                path.display()
            ),
            None => String::new(),
        };
        self.raise(
            &id,
            InboxKind::Error {
                text: format!(
                    "The coordinator moved to its own folder (ADR 0021), so a fresh session \
                     started. The previous one is kept.{reopen}"
                ),
            },
            Waiter::None,
        );
    }

    /// Gives the coordinator a new session and sends it `seed`. The old session id and
    /// transcript go in a Needs-you item whose `y` brings them back.
    fn fresh_coordinator(&mut self, kind: &Fresh, seed: &str) {
        let date = kind.date();
        let id = AgentId(COORDINATOR.into());
        let Some(row) = self.agents.get(&id) else {
            return;
        };
        let old_session = row.session_id.clone();
        let old_transcript = row.transcript_path.clone();
        self.update_agent(&id, |row| {
            row.session_id = Some(uuid::Uuid::new_v4().to_string());
            row.transcript_path = None;
        });
        tracing::info!(journal = %date, ?kind, "fresh coordinator");
        let notice = match kind {
            Fresh::NewDay { .. } => format!(
                "New day: a fresh coordinator session, seeded with the {date} handoff. \
                 Yesterday's session is kept; y on its Needs-you item goes back to it."
            ),
            Fresh::MidDay { time, .. } => format!(
                "Fresh start at {time}: a new coordinator session, seeded with the handoff in \
                 journal/{date}.md. The previous session is kept; y on its Needs-you item goes \
                 back to it."
            ),
        };
        self.notice(&id, notice);
        if let Err(e) = self.send_text(&id, seed, EntryKind::Notice, format!("Dovetail: {seed}")) {
            self.fail(e);
        }
        let input = json!({
            "journal": date.to_string(),
            "session_id": old_session,
            "transcript_path": old_transcript,
        });
        let (tool, reason) = match kind {
            Fresh::NewDay { .. } => (
                KEEP_COORDINATOR_PERMISSION,
                format!(
                    "A fresh coordinator started from the {date} handoff. y goes back to \
                     yesterday's session (the fresh one stops); d keeps the fresh one."
                ),
            ),
            Fresh::MidDay { .. } => (
                FRESH_COORDINATOR_PERMISSION,
                "Fresh coordinator started. y: go back to the previous one (the fresh one \
                 stops); d keeps the fresh one."
                    .to_owned(),
            ),
        };
        self.raise(
            &id,
            InboxKind::Permission {
                tool: tool.into(),
                input,
                reason: Some(reason),
                suggestions: Vec::new(),
            },
            Waiter::KeepCoordinator {
                session_id: old_session,
                transcript_path: old_transcript,
            },
        );
    }

    /// `y` on the keep-yesterday item: the fresh session stops, and yesterday's resumes once it
    /// has exited ([`Core::coordinator_exited`]).
    pub(crate) fn keep_previous_coordinator(
        &mut self,
        session_id: Option<String>,
        transcript_path: Option<PathBuf>,
    ) {
        let id = AgentId(COORDINATOR.into());
        self.revert_coordinator = Some((session_id, transcript_path));
        match self.live.get_mut(&id) {
            Some(live) => {
                live.stopping = true;
                live.session.shutdown(crate::shutdown::QUIT_GRACE);
            }
            None => self.coordinator_exited(),
        }
    }

    /// `d` on the keep-yesterday item: the fresh session stays.
    pub(crate) fn kept_fresh_coordinator(&mut self) {
        self.activity(
            dovetail_api::ActivityKind::Lifecycle,
            Some(&AgentId(COORDINATOR.into())),
            "kept the fresh coordinator; yesterday's transcript stays on disk".into(),
        );
    }

    /// The coordinator's session exited: if yesterday's was asked back, it resumes now; if it
    /// stopped to take in changed repos, it resumes now with them and is told.
    pub(crate) fn coordinator_exited(&mut self) {
        if let Some(note) = self.reattach.take() {
            self.hold_for_coordinator(note);
            if self.pending_fresh.is_none() && self.revert_coordinator.is_none() {
                if self.quitting.is_none()
                    && let Err(e) = self.ensure_live(&AgentId(COORDINATOR.into()))
                {
                    self.fail(e);
                }
                return;
            }
        }
        if let Some(PendingFresh { kind, seed }) = self.pending_fresh.take() {
            if self.quitting.is_none() {
                self.fresh_coordinator(&kind, &seed);
            }
            return;
        }
        let Some((session_id, transcript_path)) = self.revert_coordinator.take() else {
            return;
        };
        let id = AgentId(COORDINATOR.into());
        self.update_agent(&id, |row| {
            row.session_id = session_id;
            row.transcript_path = transcript_path;
        });
        self.notice(&id, "Back on yesterday's coordinator session");
        if self.quitting.is_none()
            && let Err(e) = self.ensure_live(&id)
        {
            self.fail(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::{PLevel, TaskId};

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    const HOUR: i64 = 60 * 60 * 1000;

    fn ms(day: &str, hour: i64) -> i64 {
        d(day)
            .at(0, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond()
            + hour * HOUR
    }

    fn newest(day: &str, hour: i64) -> Option<Newest> {
        Some(Newest {
            date: d(day),
            written_ms: ms(day, hour),
        })
    }

    fn used(day: &str, hour: i64) -> LastUse {
        LastUse {
            date: Some(d(day)),
            work_ms: Some(ms(day, hour)),
        }
    }

    #[test]
    fn a_new_day_needs_an_older_journal_written_after_the_last_use() {
        let today = d("2026-11-11");
        // The morning after `:eod`: the file was written after the last work.
        assert_eq!(
            morning(newest("2026-11-10", 18), used("2026-11-10", 17), today),
            Morning::Fresh(d("2026-11-10"))
        );
        // Opened again later the same day: the fresh one carries on.
        assert_eq!(
            morning(newest("2026-11-10", 18), used("2026-11-11", 9), today),
            Morning::Resume
        );
        // Closed today already (an `:eod` and a restart).
        assert_eq!(
            morning(newest("2026-11-11", 8), used("2026-11-10", 17), today),
            Morning::Resume
        );
        // No journal, or a coordinator that never talked.
        assert_eq!(
            morning(None, used("2026-11-09", 12), today),
            Morning::Resume
        );
        assert_eq!(
            morning(newest("2026-11-09", 12), LastUse::default(), today),
            Morning::Resume
        );
        // Several days away: the newest journal still seeds it.
        assert_eq!(
            morning(newest("2026-11-06", 18), used("2026-11-06", 17), today),
            Morning::Fresh(d("2026-11-06"))
        );
    }

    #[test]
    fn a_day_that_was_never_closed_keeps_its_coordinator() {
        let today = d("2026-11-11");
        // Used Monday without `:eod`; the newest journal is Friday's.
        assert_eq!(
            morning(newest("2026-11-06", 18), used("2026-11-09", 15), today),
            Morning::Unclosed(d("2026-11-09"))
        );
        // The same day, closed by hand a little after the last message.
        assert_eq!(
            morning(newest("2026-11-09", 15), used("2026-11-09", 15), today),
            Morning::Fresh(d("2026-11-09"))
        );
    }

    #[test]
    fn a_fresh_start_closes_the_day_only_until_the_work_goes_on() {
        let today = d("2026-11-11");
        // `:fresh` at 12 wrote a block; the seed's notice and its reply came right after.
        let at_noon = newest("2026-11-10", 12);
        let seed_reply = LastUse {
            date: Some(d("2026-11-10")),
            work_ms: Some(ms("2026-11-10", 12) + 30_000),
        };
        assert_eq!(
            morning(at_noon, seed_reply, today),
            Morning::Fresh(d("2026-11-10"))
        );
        // The afternoon's work went on without `:eod`: that handoff is stale.
        assert_eq!(
            morning(at_noon, used("2026-11-10", 17), today),
            Morning::Unclosed(d("2026-11-10"))
        );
    }

    #[test]
    fn a_day_is_named_by_its_weekday_when_that_is_unambiguous() {
        let today = d("2026-11-11");
        assert_eq!(day_name(d("2026-11-09"), today), "Monday");
        assert_eq!(day_name(d("2026-11-02"), today), "Monday 2 November");
    }

    #[test]
    fn a_hand_written_file_is_all_handoff() {
        let text = "# Handoff 2026-11-10\n\n## Open threads\n\n- PR #20\n\n## Tomorrow\n\nM14\n";
        assert_eq!(handoff(text), text.trim());
    }

    #[test]
    fn a_dovetail_block_gives_its_handoff_section() {
        let block = block(
            Ok("Open: PR #20.\n\n### Detail\n\nmore"),
            "### Open tasks\n\n- #1",
        );
        let text = format!("# 2026-11-10\n\n{block}");
        assert_eq!(handoff(&text), "Open: PR #20.\n\n### Detail\n\nmore");
    }

    #[test]
    fn a_fresh_start_block_is_the_newest_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let date = d("2026-11-10");
        write(dir.path(), date, &block(Ok("morning"), "facts")).unwrap();
        let fresh = block_titled("## Handoff (fresh start 14:05)", Ok("midday"), "facts");
        write(dir.path(), date, &fresh).unwrap();
        let text = std::fs::read_to_string(file(dir.path(), date)).unwrap();
        assert!(text.contains("morning"));
        assert_eq!(handoff(&text), "midday");
    }

    #[test]
    fn the_last_block_of_a_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let date = d("2026-11-10");
        std::fs::write(file(dir.path(), date), "Hand-written notes\n").unwrap();
        write(dir.path(), date, &block(Ok("first"), "facts")).unwrap();
        write(dir.path(), date, &block(Ok("second"), "facts")).unwrap();
        let text = std::fs::read_to_string(file(dir.path(), date)).unwrap();
        // Nothing replaced: the hand-written part and both blocks are there.
        assert!(text.starts_with("Hand-written notes\n\n---\n\n"));
        assert!(text.contains("first"));
        assert_eq!(handoff(&text), "second");
        // No temporary file left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_missing_handoff_says_why() {
        let text = block(Err(&NoHandoff::Interrupted), "facts");
        assert_eq!(
            handoff(&text),
            "_No handoff from the coordinator: its turn was interrupted._"
        );
    }

    #[test]
    fn latest_reads_dated_files_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(latest(&dir.path().join("missing")), None);
        for name in [
            "2026-11-09.md",
            "2026-11-10.md",
            "notes.md",
            "2026-13-01.md",
        ] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        std::fs::write(dir.path().join(".2026-11-12.md.tmp"), "x").unwrap();
        assert_eq!(latest(dir.path()), Some(d("2026-11-10")));
        assert_eq!(
            file(dir.path(), d("2026-11-10")),
            dir.path().join("2026-11-10.md")
        );
    }

    #[test]
    fn date_of_is_the_local_date() {
        let ms = jiff::Timestamp::now().as_millisecond();
        assert_eq!(date_of(ms), Some(today()));
    }

    fn task(id: i64, title: &str, bucket: Option<Bucket>, status: TaskStatus) -> Task {
        Task {
            id: TaskId(id),
            title: title.into(),
            status,
            p_level: None,
            bucket,
            order: id,
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

    #[test]
    fn open_tasks_are_grouped_by_bucket_now_first() {
        let mut urgent = task(4, "fix quit", Some(Bucket::Now), TaskStatus::InProgress);
        urgent.p_level = Some(PLevel::P0);
        let tasks = vec![
            task(1, "someday", None, TaskStatus::Todo),
            task(2, "later", Some(Bucket::Later), TaskStatus::Todo),
            task(3, "done", Some(Bucket::Now), TaskStatus::Done),
            urgent,
            task(5, "next", Some(Bucket::Next), TaskStatus::Todo),
        ];
        assert_eq!(
            open_tasks(&tasks),
            "**Now**\n\n- #4 fix quit (P0, in progress)\n\n**Next**\n\n- #5 next\n\n\
             **Later**\n\n- #2 later\n\n**Unsorted**\n\n- #1 someday"
        );
        assert_eq!(open_tasks(&[]), "No open tasks.");
    }
}
