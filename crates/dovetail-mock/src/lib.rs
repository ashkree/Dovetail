//! `MockCore`: a seeded stand-in for the Dovetail core, speaking the same API.
//!
//! It invents believable activity (status changes, streamed markdown replies, inbox items,
//! subagents) and answers [`Command`]s. With the same seed it produces the same stream, so it
//! drives demos (`dovetail --mock`) and snapshot tests. Time is virtual: [`MockCore::next_message`] says
//! how long to wait before each message, and only [`spawn`] actually sleeps.

#![warn(missing_docs)]

mod rng;
mod world;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use dovetail_api::*;
use rng::Rng;
use world::{COORDINATOR, agent_id, round_cents};

/// How a [`MockCore`] starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockConfig {
    /// Seed for every random choice.
    pub seed: u64,
    /// Virtual clock at the start, in Unix milliseconds.
    pub start_ms: i64,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            // 2026-11-01 09:00 UTC: a fixed time keeps output reproducible.
            start_ms: 1_793_523_600_000,
        }
    }
}

/// A message and how long to wait before delivering it.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheduled {
    /// Virtual milliseconds since the previous message.
    pub delay_ms: u64,
    /// The message.
    pub msg: ServerMsg,
}

type Action = Box<dyn FnOnce(&mut MockCore) + Send>;

struct Pending {
    delay_ms: u64,
    /// The reply this step belongs to, so an interrupt can drop the rest of it.
    reply_of: Option<AgentId>,
    action: Action,
}

/// The mock core. Holds the authoritative state and folds its own events in with [`apply`].
pub struct MockCore {
    rng: Rng,
    state: Snapshot,
    /// Messages already produced, delivered with no delay.
    ready: VecDeque<ServerMsg>,
    /// Future steps, each run when its delay has passed.
    pending: VecDeque<Pending>,
    /// Agents in the middle of a streamed reply.
    replying: BTreeSet<AgentId>,
    next_entry: u64,
    next_inbox: u64,
    next_activity: u64,
    next_message: u64,
    next_subagent: u64,
    next_task: i64,
    reply_count: u64,
    /// What [`Command::PasteClipboard`] answers, in turn; see [`MockCore::queue_clipboard`].
    clipboard: VecDeque<Paste>,
    /// Tokens in each session's context, as the core counts them from the stream (M13c).
    context_tokens: BTreeMap<AgentId, u64>,
    quit: bool,
}

/// The mock sessions' window: they run `claude-opus-5-5`, a 1M model.
const CONTEXT_WINDOW: u64 = 1_000_000;

impl MockCore {
    /// A mock core with its starting world.
    pub fn new(config: MockConfig) -> Self {
        let state = world::initial(config.start_ms);
        let mut core = Self {
            rng: Rng::new(config.seed),
            next_task: state.tasks.iter().map(|t| t.id.0).max().unwrap_or(0) + 1,
            next_inbox: state.inbox.iter().map(|i| i.id.0).max().unwrap_or(0) + 1,
            next_activity: state.activity.iter().map(|a| a.id).max().unwrap_or(0) + 1,
            next_message: state.messages.iter().map(|m| m.id).max().unwrap_or(0) + 1,
            state,
            ready: VecDeque::new(),
            pending: VecDeque::new(),
            replying: BTreeSet::new(),
            next_entry: 100,
            next_subagent: 1,
            reply_count: 0,
            clipboard: VecDeque::new(),
            context_tokens: BTreeMap::new(),
            quit: false,
        };
        core.after(3_000, |core| core.finish_setup(&agent_id("picker")));
        core
    }

    /// The opening messages of a connection: [`ServerMsg::Hello`] and the current snapshot.
    pub fn connect(&self) -> Vec<ServerMsg> {
        vec![
            ServerMsg::Hello {
                protocol: PROTOCOL_VERSION,
            },
            ServerMsg::Snapshot(self.state.trimmed()),
        ]
    }

    /// The core's own copy of the state.
    pub fn state(&self) -> &Snapshot {
        &self.state
    }

    /// Queues what the next [`Command::PasteClipboard`] answers. The mock reads no real
    /// clipboard: with nothing queued it refuses, as the core does with no clipboard tool.
    pub fn queue_clipboard(&mut self, paste: Paste) {
        self.clipboard.push_back(paste);
    }

    /// Whether a [`Command::Quit`] has been handled.
    pub fn has_quit(&self) -> bool {
        self.quit
    }

    /// The next message and its delay, or `None` after [`Command::Quit`].
    pub fn next_message(&mut self) -> Option<Scheduled> {
        if self.quit {
            return None;
        }
        let delay_ms = self.peek_delay();
        let msg = self.pop();
        Some(Scheduled { delay_ms, msg })
    }

    /// Virtual milliseconds until the next message.
    pub fn peek_delay(&mut self) -> u64 {
        if !self.ready.is_empty() {
            return 0;
        }
        if self.pending.is_empty() {
            self.plan_something();
        }
        self.pending.front().map_or(0, |p| p.delay_ms)
    }

    /// Lets `ms` of virtual time pass without delivering anything.
    pub fn advance(&mut self, ms: u64) {
        if let Some(front) = self.pending.front_mut() {
            let passed = ms.min(front.delay_ms);
            front.delay_ms -= passed;
            self.state.now_ms += passed as i64;
        }
    }

    /// Delivers the next message, running steps until one produces something.
    pub fn pop(&mut self) -> ServerMsg {
        loop {
            if let Some(msg) = self.ready.pop_front() {
                return msg;
            }
            if self.pending.is_empty() {
                self.plan_something();
            }
            if let Some(step) = self.pending.pop_front() {
                self.state.now_ms += step.delay_ms as i64;
                (step.action)(self);
            }
        }
    }

    /// Carries out a command. Its effects are delivered before anything already scheduled.
    pub fn handle(&mut self, command: Command) {
        match command {
            Command::Prompt {
                agent,
                text,
                attachments,
            } => {
                if self.agent(&agent).is_none() {
                    return self.fail(format!("no agent {}", agent.0));
                }
                // Like the core: a chat message answers the agent's open questions (ADR 0007),
                // but an image-only one has no words to answer with and leaves them open.
                let open: Vec<InboxId> = self
                    .state
                    .inbox
                    .iter()
                    .filter(|i| i.agent == agent && matches!(i.kind, InboxKind::Question { .. }))
                    .filter(|_| !text.trim().is_empty())
                    .map(|i| i.id)
                    .collect();
                self.append_with(&agent, EntryKind::User, text, attachments);
                for id in open {
                    self.resolve(id);
                }
                self.update_agent(&agent, |a| a.status = AgentStatus::Working);
                self.schedule_reply(agent, true);
            }
            Command::ListPaths { agent, dir } => {
                let paths = world::paths(dir.as_deref());
                self.emit(Event::PathsListed { agent, dir, paths });
            }
            Command::PasteClipboard { agent } => {
                let paste = self
                    .clipboard
                    .pop_front()
                    .unwrap_or_else(|| Paste::Refused("the mock core has no clipboard".into()));
                self.emit(Event::ClipboardPasted { agent, paste });
            }
            // The mock touches nothing outside itself: no clipboard and no editor. The editor
            // answer still comes, so the UI takes its terminal back.
            Command::CopyToClipboard { .. } => {}
            Command::OpenEditor { .. } => self.emit(Event::TerminalReturned {
                error: Some("the mock core opens no editor".into()),
            }),
            Command::Interrupt { agent } => {
                if !self.replying.remove(&agent) {
                    return self.fail(format!("{} is not mid-turn", agent.0));
                }
                self.pending.retain(|p| p.reply_of.as_ref() != Some(&agent));
                self.append(&agent, EntryKind::Notice, "Interrupted".into());
                self.settle(&agent);
            }
            Command::AnswerPermission { item, answer } => {
                let Some(found) = self.inbox_item(item) else {
                    return self.fail(format!("no inbox item {}", item.0));
                };
                if !matches!(found.kind, InboxKind::Permission { .. }) {
                    return self.fail("that item is not a permission request".into());
                }
                self.resolve(item);
                let removal = matches!(&found.kind, InboxKind::Permission { tool, .. }
                    if tool == REMOVE_WORKTREE_PERMISSION);
                if removal {
                    // Yes removes the worktree with the slice; no keeps both.
                    match answer {
                        PermissionAnswer::Deny { .. } => self.append(
                            &found.agent,
                            EntryKind::Notice,
                            "Kept: the worktree has unsaved work".into(),
                        ),
                        _ => self.remove_agent(&found.agent),
                    }
                    return;
                }
                if let PermissionAnswer::Deny { message } = answer {
                    let text = match message {
                        Some(m) => format!("Denied: {m}"),
                        None => "Denied".into(),
                    };
                    self.append(&found.agent, EntryKind::Notice, text);
                }
            }
            Command::AnswerQuestion { item, text } => {
                let Some(found) = self.inbox_item(item) else {
                    return self.fail(format!("no inbox item {}", item.0));
                };
                self.resolve(item);
                self.append(&found.agent, EntryKind::User, text);
            }
            Command::DismissInbox { item } => {
                if self.inbox_item(item).is_none() {
                    return self.fail(format!("no inbox item {}", item.0));
                }
                self.resolve(item);
            }
            Command::CyclePermissionMode { agent } => {
                if self.agent(&agent).is_none() {
                    return self.fail(format!("no agent {}", agent.0));
                }
                self.update_agent(&agent, |a| {
                    a.permission_mode = match a.permission_mode {
                        PermissionMode::Auto => PermissionMode::AcceptEdits,
                        PermissionMode::AcceptEdits => PermissionMode::Manual,
                        PermissionMode::Manual => PermissionMode::Auto,
                    }
                });
            }
            Command::NewSlice { task, name, brief } => {
                if self.agent(&agent_id(&name)).is_some() {
                    return self.fail(format!("a slice named {name} exists"));
                }
                let mut slice = world::session(
                    &name,
                    AgentKind::Slice,
                    AgentStatus::SettingUp,
                    self.state.now_ms,
                );
                slice.task = task;
                slice.worktree = Some(world::worktree(&name));
                slice.status_line = "setup: cargo fetch".into();
                self.emit(Event::AgentUpserted(slice));
                // Starting work on a task marks it started and puts it in Now; nothing moves it
                // back (plan B4, task #7).
                if let Some(mut started) = task.and_then(|t| self.task(t)) {
                    let before = started.clone();
                    if started.status == TaskStatus::Todo {
                        started.status = TaskStatus::InProgress;
                    }
                    if started.status != TaskStatus::Done {
                        started.bucket = Some(Bucket::Now);
                    }
                    if started != before {
                        self.emit(Event::TaskUpserted(started));
                    }
                }
                self.append(&agent_id(&name), EntryKind::User, brief);
                self.refresh_totals();
                let id = agent_id(&name);
                self.after(2_500, move |core| core.finish_setup(&id));
            }
            Command::EndOfDay => {
                if self.state.journal.as_ref().is_some_and(|j| j.closing) {
                    return self.fail("already closing the day".into());
                }
                let latest = self.state.journal.as_ref().and_then(|j| j.latest.clone());
                self.emit(Event::JournalChanged(JournalState {
                    latest,
                    closing: true,
                }));
                let coordinator = agent_id(COORDINATOR);
                self.append(
                    &coordinator,
                    EntryKind::Notice,
                    "Dovetail: the user is closing the day. Write your handoff.".into(),
                );
                self.after(1_500, move |core| {
                    core.append(
                        &coordinator,
                        EntryKind::Assistant,
                        "Handoff: the tree view is close to done; the picker waits on review."
                            .into(),
                    );
                    let date = world::civil_date(core.state.now_ms);
                    core.emit(Event::JournalChanged(JournalState {
                        latest: Some(date),
                        closing: false,
                    }));
                });
            }
            Command::NewSideChat { question } => {
                let n = (1..)
                    .find(|n| self.agent(&agent_id(&format!("side-{n}"))).is_none())
                    .unwrap_or(1);
                let name = format!("side-{n}");
                let chat = world::session(
                    &name,
                    AgentKind::SideChat,
                    AgentStatus::Working,
                    self.state.now_ms,
                );
                self.emit(Event::AgentUpserted(chat));
                let id = agent_id(&name);
                self.append(&id, EntryKind::User, question);
                self.refresh_totals();
                self.schedule_reply(id, true);
            }
            Command::CloseSlice { agent } => {
                let Some(found) = self.agent(&agent) else {
                    return self.fail(format!("no agent {}", agent.0));
                };
                // Like the core's archive path: unsaved work in the worktree asks first.
                if let Some(wt) = found.worktree.clone()
                    && self.unsaved(&found, &wt)
                {
                    if !self.asking_to_remove(&agent) {
                        let kind = InboxKind::Permission {
                            tool: REMOVE_WORKTREE_PERMISSION.into(),
                            input: serde_json::json!({
                                "worktree": wt.path,
                                "branch": wt.branch,
                            }),
                            reason: Some(
                                "the slice has uncommitted or unpushed work, which removing it \
                                 would lose"
                                    .into(),
                            ),
                            suggestions: vec![],
                        };
                        self.raise(&agent, kind);
                    }
                    return;
                }
                self.remove_agent(&agent);
            }
            Command::CreateTask(new) => {
                let task = Task {
                    id: TaskId(self.next_task),
                    title: new.title,
                    status: TaskStatus::Todo,
                    p_level: new.p_level,
                    bucket: new.bucket,
                    order: self.next_order(new.bucket),
                    deps: vec![],
                    notes: new.notes,
                    pr: new.pr,
                    follow_up_of: new.follow_up_of,
                    coordinator_created: false,
                    related: vec![],
                };
                self.next_task += 1;
                let id = task.id;
                self.emit(Event::TaskUpserted(task));
                for other in new.related {
                    self.relate(id, other, true);
                }
            }
            Command::LogDecision(new) => {
                if self.task(new.task).is_none() {
                    return self.fail(format!("no task {}", new.task.0));
                }
                self.log_decision(DecisionAuthor::User, new);
            }
            Command::UpdateTask { id, patch } => {
                let Some(mut task) = self.task(id) else {
                    return self.fail(format!("no task {}", id.0));
                };
                let mut relations = Vec::new();
                for change in patch {
                    match change {
                        TaskChange::Title(title) => task.title = title,
                        TaskChange::Status(status) => task.status = status,
                        TaskChange::PLevel(p_level) => task.p_level = p_level,
                        TaskChange::Bucket(bucket) => {
                            if bucket != task.bucket {
                                task.order = self.next_order(bucket);
                            }
                            task.bucket = bucket;
                        }
                        TaskChange::Notes(notes) => task.notes = notes,
                        TaskChange::Deps(deps) => task.deps = deps,
                        TaskChange::Pr(pr) => task.pr = pr,
                        TaskChange::Seen => task.coordinator_created = false,
                        TaskChange::Relate(other) | TaskChange::Unrelate(other) => {
                            relations.push((other, matches!(change, TaskChange::Relate(_))));
                        }
                    }
                }
                self.emit(Event::TaskUpserted(task));
                for (other, on) in relations {
                    self.relate(id, other, on);
                }
            }
            Command::ArchiveTask { id } => {
                let Some(mut task) = self.task(id) else {
                    return self.fail(format!("no task {}", id.0));
                };
                task.status = TaskStatus::Archived;
                self.emit(Event::TaskUpserted(task));
            }
            Command::ReorderTask { id, delta } => self.reorder(id, delta),
            Command::LoadConversation { agent, .. } => {
                // The mock keeps no history beyond the tail.
                self.emit(Event::ConversationPage {
                    agent,
                    entries: vec![],
                    has_more: false,
                });
            }
            Command::RefreshWorktrees => {
                let now_ms = self.state.now_ms;
                let infos: Vec<WorktreeInfo> = self
                    .state
                    .agents
                    .iter()
                    .filter(|a| a.kind == AgentKind::Slice)
                    .filter_map(|a| Some(world::worktree_info(a, a.worktree.as_ref()?, now_ms)))
                    .collect();
                for info in infos {
                    self.emit(Event::WorktreeUpserted(info));
                }
            }
            Command::Resync => self
                .ready
                .push_back(ServerMsg::Snapshot(self.state.trimmed())),
            Command::Quit { .. } => self.quit = true,
            Command::TakeOver { .. }
            | Command::RemoveWorktree { .. }
            | Command::OpenProject { .. }
            | Command::RegisterProject { .. } => self.fail("not available in the mock".into()),
        }
    }

    // State helpers

    fn emit(&mut self, event: Event) {
        let seq = self.state.seq + 1;
        let applied = apply(&mut self.state, seq, event.clone());
        debug_assert_eq!(applied, ApplyResult::Applied);
        self.ready.push_back(ServerMsg::Event { seq, event });
    }

    fn fail(&mut self, message: String) {
        self.emit(Event::CommandFailed { message });
    }

    fn after(&mut self, delay_ms: u64, action: impl FnOnce(&mut MockCore) + Send + 'static) {
        self.pending.push_back(Pending {
            delay_ms,
            reply_of: None,
            action: Box::new(action),
        });
    }

    fn reply_step(
        &mut self,
        agent: &AgentId,
        delay_ms: u64,
        action: impl FnOnce(&mut MockCore) + Send + 'static,
    ) {
        self.pending.push_back(Pending {
            delay_ms,
            reply_of: Some(agent.clone()),
            action: Box::new(action),
        });
    }

    fn entry_id(&mut self) -> EntryId {
        self.next_entry += 1;
        EntryId(self.next_entry)
    }

    /// Adds a finished entry to an agent's conversation.
    fn append(&mut self, agent: &AgentId, kind: EntryKind, text: String) {
        self.append_with(agent, kind, text, Vec::new());
    }

    /// Adds a finished entry with attached images.
    fn append_with(
        &mut self,
        agent: &AgentId,
        kind: EntryKind,
        text: String,
        attachments: Vec<Attachment>,
    ) {
        let id = self.entry_id();
        self.emit(Event::ConversationAppended {
            agent: agent.clone(),
            entry: ConversationEntry {
                id,
                at_ms: self.state.now_ms,
                kind,
                text,
                attachments,
            },
        });
    }

    fn agent(&self, id: &AgentId) -> Option<Agent> {
        self.state.agents.iter().find(|a| &a.id == id).cloned()
    }

    fn task(&self, id: TaskId) -> Option<Task> {
        self.state.tasks.iter().find(|t| t.id == id).cloned()
    }

    fn inbox_item(&self, id: InboxId) -> Option<InboxItem> {
        self.state.inbox.iter().find(|i| i.id == id).cloned()
    }

    /// Adds `call` tokens to a session's context, or starts over at `compacted` tokens when the
    /// context would pass 95 %, and returns the fill the core would show. A session with no fill
    /// yet keeps none.
    fn grow_context(&mut self, agent: &AgentId, call: u64, compacted: u64) -> Option<u8> {
        let start = self
            .state
            .agents
            .iter()
            .find(|a| &a.id == agent)?
            .context_pct?;
        let tokens = self
            .context_tokens
            .entry(agent.clone())
            .or_insert(u64::from(start) * CONTEXT_WINDOW / 100);
        *tokens += call;
        if *tokens * 100 > CONTEXT_WINDOW * 95 {
            *tokens = compacted;
        }
        let pct = (*tokens * 100 + CONTEXT_WINDOW / 2) / CONTEXT_WINDOW;
        Some(u8::try_from(pct.min(100)).unwrap_or(100))
    }

    fn update_agent(&mut self, id: &AgentId, change: impl FnOnce(&mut Agent)) {
        if let Some(before) = self.agent(id) {
            let mut agent = before.clone();
            change(&mut agent);
            if agent != before {
                agent.last_activity_ms = self.state.now_ms;
                self.emit(Event::AgentUpserted(agent));
            }
        }
    }

    fn refresh_totals(&mut self) {
        let totals = world::totals_of(&self.state);
        if totals != self.state.totals {
            self.emit(Event::TotalsChanged(totals));
        }
    }

    fn next_order(&self, bucket: Option<Bucket>) -> i64 {
        self.state
            .tasks
            .iter()
            .filter(|t| t.bucket == bucket)
            .map(|t| t.order + 1)
            .max()
            .unwrap_or(0)
    }

    fn reorder(&mut self, id: TaskId, delta: i32) {
        let Some(task) = self.task(id) else {
            return self.fail(format!("no task {}", id.0));
        };
        let mut peers: Vec<Task> = self
            .state
            .tasks
            .iter()
            .filter(|t| t.bucket == task.bucket && t.status != TaskStatus::Archived)
            .cloned()
            .collect();
        peers.sort_by_key(|t| (t.order, t.id));
        let Some(from) = peers.iter().position(|t| t.id == id) else {
            return;
        };
        let to = (from as i64 + i64::from(delta)).clamp(0, peers.len() as i64 - 1) as usize;
        let moved = peers.remove(from);
        peers.insert(to, moved);
        for (order, mut peer) in peers.into_iter().enumerate() {
            if peer.order != order as i64 {
                peer.order = order as i64;
                self.emit(Event::TaskUpserted(peer));
            }
        }
    }

    /// Whether the agent is blocked on the user. A finished turn leaves it idle, not blocked.
    fn has_inbox(&self, agent: &AgentId) -> bool {
        self.state
            .inbox
            .iter()
            .any(|i| &i.agent == agent && !matches!(i.kind, InboxKind::FinishedTurn { .. }))
    }

    fn running_background_children(&self, parent: &AgentId) -> bool {
        self.state.agents.iter().any(|a| {
            a.parent.as_ref() == Some(parent)
                && a.kind == AgentKind::Subagent { background: true }
                && a.status == AgentStatus::Working
        })
    }

    /// Whether removing the slice's worktree would lose work: uncommitted changes or unpushed
    /// commits, from the worktree view's data (or what a refresh would show).
    fn unsaved(&self, agent: &Agent, wt: &WorktreeRef) -> bool {
        let info = self
            .state
            .worktrees
            .iter()
            .find(|w| w.agent == agent.id)
            .cloned()
            .unwrap_or_else(|| world::worktree_info(agent, wt, self.state.now_ms));
        info.unpushed.is_some_and(|n| n > 0) || info.changes.is_some_and(|c| !c.is_clean())
    }

    /// Whether the user is already asked about removing the slice's worktree.
    fn asking_to_remove(&self, agent: &AgentId) -> bool {
        self.state.inbox.iter().any(|i| {
            &i.agent == agent
                && matches!(&i.kind, InboxKind::Permission { tool, .. }
                    if tool == REMOVE_WORKTREE_PERMISSION)
        })
    }

    /// Adds an inbox item and marks its agent as needing the user.
    fn raise(&mut self, agent: &AgentId, kind: InboxKind) {
        if self.agent(agent).is_none() {
            return;
        }
        if let InboxKind::Question { text } = &kind {
            // Like the core: the question shows in the conversation too.
            let notice = format!("Question for you (also in Needs you): {text}");
            self.append(agent, EntryKind::Notice, notice);
        }
        let item = InboxItem {
            id: InboxId(self.next_inbox),
            agent: agent.clone(),
            at_ms: self.state.now_ms,
            kind,
        };
        self.next_inbox += 1;
        self.emit(Event::InboxUpserted(item));
        if self.has_inbox(agent) {
            self.update_agent(agent, |a| a.status = AgentStatus::NeedsYou);
        } else {
            self.settle(agent);
        }
        self.refresh_totals();
    }

    /// Resolves an inbox item; its agent carries on if nothing else waits on the user.
    fn resolve(&mut self, item: InboxId) {
        let Some(found) = self.inbox_item(item) else {
            return;
        };
        self.emit(Event::InboxResolved { id: item });
        if !self.has_inbox(&found.agent)
            && self.agent(&found.agent).map(|a| a.status) == Some(AgentStatus::NeedsYou)
        {
            let next = if self.replying.contains(&found.agent) {
                AgentStatus::Working
            } else {
                AgentStatus::Idle
            };
            self.update_agent(&found.agent, |a| a.status = next);
        }
        self.refresh_totals();
    }

    /// Puts an agent back to idle, or needs-you if something still waits on the user.
    fn settle(&mut self, agent: &AgentId) {
        let status = if self.has_inbox(agent) {
            AgentStatus::NeedsYou
        } else {
            AgentStatus::Idle
        };
        self.update_agent(agent, |a| a.status = status);
    }

    fn remove_agent(&mut self, agent: &AgentId) {
        let children: Vec<AgentId> = self
            .state
            .agents
            .iter()
            .filter(|a| a.parent.as_ref() == Some(agent))
            .map(|a| a.id.clone())
            .collect();
        for child in children {
            self.remove_agent(&child);
        }
        let items: Vec<InboxId> = self
            .state
            .inbox
            .iter()
            .filter(|i| &i.agent == agent)
            .map(|i| i.id)
            .collect();
        for item in items {
            self.emit(Event::InboxResolved { id: item });
        }
        self.replying.remove(agent);
        self.pending.retain(|p| p.reply_of.as_ref() != Some(agent));
        self.emit(Event::AgentRemoved(agent.clone()));
        self.refresh_totals();
    }

    fn finish_setup(&mut self, agent: &AgentId) {
        if self.agent(agent).map(|a| a.status) != Some(AgentStatus::SettingUp) {
            return;
        }
        self.update_agent(agent, |a| {
            a.status = AgentStatus::Working;
            a.status_line = "reading the brief".into();
            a.memory_mb = Some(205);
        });
        self.activity(
            Some(agent.clone()),
            ActivityKind::Lifecycle,
            format!("{} is set up", agent.0),
        );
        self.schedule_reply(agent.clone(), false);
    }

    fn activity(&mut self, agent: Option<AgentId>, kind: ActivityKind, text: String) {
        let item = ActivityItem {
            id: self.next_activity,
            at_ms: self.state.now_ms,
            agent,
            kind,
            text,
        };
        self.next_activity += 1;
        self.emit(Event::ActivityAdded(item));
    }

    // Streamed replies

    /// Queues a whole reply: think, stream the text in chunks, fold the tools, end the turn.
    fn schedule_reply(&mut self, agent: AgentId, answering_user: bool) {
        if !self.replying.insert(agent.clone()) {
            return;
        }
        let text = world::REPLIES[(self.reply_count % world::REPLIES.len() as u64) as usize];
        self.reply_count += 1;
        let entry = self.entry_id();

        let think = self.rng.range(400, 1_500);
        // Slices sometimes hand part of the turn to a background subagent; any session may run
        // a foreground one inside its turn.
        let is_slice = self.agent(&agent).map(|a| a.kind) == Some(AgentKind::Slice);
        let delegate = if is_slice && self.rng.chance(40) {
            Some(true)
        } else if self.rng.chance(25) {
            Some(false)
        } else {
            None
        };
        let id = agent.clone();
        self.reply_step(&agent, think, move |core| {
            core.update_agent(&id, |a| {
                if a.status != AgentStatus::NeedsYou {
                    a.status = AgentStatus::Working;
                }
            });
            if let Some(background) = delegate {
                core.spawn_subagent(&id, background);
            }
            core.emit(Event::ConversationAppended {
                agent: id,
                entry: ConversationEntry {
                    id: entry,
                    at_ms: core.state.now_ms,
                    kind: EntryKind::Assistant,
                    text: String::new(),
                    attachments: Vec::new(),
                },
            });
        });

        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let len = self.rng.range(2, 10) as usize;
            let chunk: String = chars[start..(start + len).min(chars.len())]
                .iter()
                .collect();
            start += len;
            let delay = self.rng.range(30, 90);
            let id = agent.clone();
            self.reply_step(&agent, delay, move |core| {
                core.emit(Event::AssistantDelta {
                    agent: id,
                    entry,
                    text: chunk,
                })
            });
        }

        if !answering_user || self.rng.chance(60) {
            let tools = match self.rng.range(0, 2) {
                0 => "Read src/app.rs · Edit src/app.rs",
                1 => "Grep \"Region\" · Read buffers/inbox.rs · Bash cargo test",
                _ => "Edit theme.rs · Bash cargo clippy",
            };
            let id = agent.clone();
            let delay = self.rng.range(200, 600);
            self.reply_step(&agent, delay, move |core| {
                core.append(&id, EntryKind::ToolSummary, tools.into())
            });
        }

        let preview: String = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .take(80)
            .collect();
        let delay = self.rng.range(100, 300);
        let cost = self.rng.range(1, 6) as f64 / 100.0;
        let id = agent.clone();
        self.reply_step(&agent, delay, move |core| core.end_turn(&id, preview, cost));
    }

    /// Ends a turn. The previous finished-turn item is replaced (ADR 0001), and an agent with
    /// background subagents still running gets none, since its turn has not really ended.
    fn end_turn(&mut self, agent: &AgentId, preview: String, cost: f64) {
        self.replying.remove(agent);
        self.update_agent(agent, |a| a.cost_usd = round_cents(a.cost_usd + cost));
        // Like Claude Code's `rate_limit_event`: every turn uses a little of the 5-hour window.
        let used = self
            .state
            .usage
            .five_hour
            .as_ref()
            .map_or(0.0, |w| w.utilization);
        let usage = world::usage(self.state.now_ms, ((used + 0.01) * 100.0).round() / 100.0);
        self.emit(Event::UsageChanged(usage));
        if !self.running_background_children(agent)
            && self.agent(agent).is_some_and(|a| a.kind.is_session())
        {
            let old: Vec<InboxId> = self
                .state
                .inbox
                .iter()
                .filter(|i| &i.agent == agent && matches!(i.kind, InboxKind::FinishedTurn { .. }))
                .map(|i| i.id)
                .collect();
            for item in old {
                self.emit(Event::InboxResolved { id: item });
            }
            self.raise(agent, InboxKind::FinishedTurn { preview });
        } else {
            self.settle(agent);
            self.refresh_totals();
        }
    }

    // Spontaneous activity

    fn sessions_with(&self, kind: AgentKind, statuses: &[AgentStatus]) -> Vec<AgentId> {
        self.state
            .agents
            .iter()
            .filter(|a| a.kind == kind && statuses.contains(&a.status))
            .map(|a| a.id.clone())
            .collect()
    }

    /// Picks the next thing to happen and queues it after a pause.
    fn plan_something(&mut self) {
        let pause = self.rng.range(300, 2_000);
        for _ in 0..8 {
            let choice = self
                .rng
                .weighted(&[30, 12, 6, 15, 8, 8, 6, 10, 9, 1, 8, 3, 6, 3]);
            if self.try_plan(choice, pause) {
                return;
            }
        }
        // Nothing fitted after several tries; let the coordinator report on things instead.
        let line = self
            .rng
            .pick(&["watching 3 slices", "waiting on CI", "planning M2"])
            .copied()
            .unwrap_or_default();
        self.after(pause, move |core| {
            core.update_agent(&agent_id(COORDINATOR), |a| a.status_line = line.into())
        });
    }

    /// Queues `choice` if it fits the current state.
    fn try_plan(&mut self, choice: usize, pause: u64) -> bool {
        let busy = [AgentStatus::Working, AgentStatus::Idle];
        // Blocked agents still report status and chat; only new work needs a free agent.
        let alive = [
            AgentStatus::Working,
            AgentStatus::Idle,
            AgentStatus::NeedsYou,
        ];
        let full_inbox = self.state.inbox.len() >= 6;
        match choice {
            0 => {
                let free: Vec<AgentId> = self
                    .sessions_with(AgentKind::Slice, &[AgentStatus::Working, AgentStatus::Idle])
                    .into_iter()
                    .filter(|a| !self.replying.contains(a))
                    .collect();
                if let Some(agent) = self.rng.pick(&free).cloned() {
                    self.after(pause, move |core| core.schedule_reply(agent, false));
                    return true;
                }
            }
            1 if !full_inbox => {
                if let Some(agent) = self
                    .rng
                    .pick(&self.sessions_with(AgentKind::Slice, &busy))
                    .cloned()
                {
                    let (tool, input) = world::permission_input(self.rng.next_u64());
                    self.after(pause, move |core| {
                        core.raise(
                            &agent,
                            InboxKind::Permission {
                                tool: tool.into(),
                                input,
                                reason: None,
                                suggestions: vec![],
                            },
                        )
                    });
                    return true;
                }
            }
            2 if !full_inbox => {
                if let Some(agent) = self
                    .rng
                    .pick(&self.sessions_with(AgentKind::Slice, &busy))
                    .cloned()
                {
                    let text = self.rng.pick(world::QUESTIONS).copied().unwrap_or_default();
                    self.after(pause, move |core| {
                        core.raise(&agent, InboxKind::Question { text: text.into() })
                    });
                    return true;
                }
            }
            3 => {
                if let Some(agent) = self
                    .rng
                    .pick(&self.sessions_with(AgentKind::Slice, &alive))
                    .cloned()
                {
                    let line = self
                        .rng
                        .pick(world::STATUS_LINES)
                        .copied()
                        .unwrap_or_default();
                    self.after(pause, move |core| {
                        core.update_agent(&agent, |a| a.status_line = line.into())
                    });
                    return true;
                }
            }
            4 => {
                let running = self
                    .state
                    .agents
                    .iter()
                    .filter(|a| {
                        a.kind == AgentKind::Subagent { background: true }
                            && a.status == AgentStatus::Working
                    })
                    .count();
                if running < 3
                    && let Some(parent) = self
                        .rng
                        .pick(&self.sessions_with(AgentKind::Slice, &[AgentStatus::Working]))
                        .cloned()
                {
                    self.after(pause, move |core| core.spawn_subagent(&parent, true));
                    return true;
                }
            }
            5 => {
                let slices: Vec<(AgentId, Option<u32>)> = self
                    .state
                    .agents
                    .iter()
                    .filter(|a| a.kind == AgentKind::Slice && a.status != AgentStatus::SettingUp)
                    .map(|a| {
                        (
                            a.id.clone(),
                            a.task.and_then(|t| self.task(t)).and_then(|t| t.pr),
                        )
                    })
                    .collect();
                if let Some((agent, pr)) = self.rng.pick(&slices).cloned() {
                    let pr = pr.unwrap_or(1232);
                    let (kind, text) = match self.rng.range(0, 2) {
                        0 => (ActivityKind::Pr, format!("pushed to #{pr}")),
                        1 => (ActivityKind::Ci, format!("#{pr} CI passed")),
                        _ => (ActivityKind::Ci, format!("#{pr} CI failed: clippy")),
                    };
                    self.after(pause, move |core| core.activity(Some(agent), kind, text));
                    return true;
                }
            }
            6 => {
                let slices = self.sessions_with(AgentKind::Slice, &alive);
                if let Some(slice) = self.rng.pick(&slices).cloned() {
                    let text = self.rng.pick(world::MESSAGES).copied().unwrap_or_default();
                    let (from, to) = if self.rng.chance(50) {
                        (agent_id(COORDINATOR), slice)
                    } else {
                        (slice, agent_id(COORDINATOR))
                    };
                    self.after(pause, move |core| core.message(from, to, text.into()));
                    return true;
                }
            }
            7 => {
                let sessions: Vec<AgentId> = self
                    .state
                    .agents
                    .iter()
                    .filter(|a| a.kind.is_session() && a.status != AgentStatus::SettingUp)
                    .map(|a| a.id.clone())
                    .collect();
                if let Some(agent) = self.rng.pick(&sessions).cloned() {
                    // One API call's worth of new context; near full, Claude Code compacts. One
                    // draw only: more would shift the seeded stream the tests rely on.
                    let call = self.rng.range(8_000, 40_000);
                    let compacted = 60_000 + call * 3;
                    let memory = self.rng.range(0, 30) as u32;
                    self.after(pause, move |core| {
                        let pct = core.grow_context(&agent, call, compacted);
                        core.update_agent(&agent, |a| {
                            a.context_pct = pct;
                            a.memory_mb = a
                                .memory_mb
                                .map(|m| (m + memory).saturating_sub(12).max(180));
                        });
                        core.refresh_totals();
                    });
                    return true;
                }
            }
            8 => {
                if let Some(agent) = self
                    .state
                    .todos
                    .iter()
                    .find(|t| t.status != TodoStatus::Completed)
                    .and_then(|t| t.created_by.clone())
                {
                    self.after(pause, move |core| core.advance_todos(&agent));
                    return true;
                }
            }
            9 if !full_inbox => {
                if let Some(agent) = self
                    .rng
                    .pick(&self.sessions_with(AgentKind::Slice, &busy))
                    .cloned()
                {
                    self.after(pause, move |core| {
                        core.raise(
                            &agent,
                            InboxKind::Error {
                                text: "cargo test was killed: memory cap reached (2048 MB)".into(),
                            },
                        )
                    });
                    return true;
                }
            }
            // The coordinator files a follow-up it spotted; it lands in Unsorted, marked.
            10 => {
                let unseen = self
                    .state
                    .tasks
                    .iter()
                    .filter(|t| t.coordinator_created)
                    .count();
                let parent = self
                    .rng
                    .pick(&self.state.tasks.iter().map(|t| t.id).collect::<Vec<_>>())
                    .copied();
                let title = self
                    .rng
                    .pick(world::FOLLOW_UPS)
                    .copied()
                    .unwrap_or_default();
                if unseen < 3 {
                    self.after(pause, move |core| core.file_follow_up(title, parent));
                    return true;
                }
            }
            // Claude Code cancels a held permission request (its turn was interrupted).
            11 => {
                let held = self
                    .state
                    .inbox
                    .iter()
                    .filter(|i| matches!(i.kind, InboxKind::Permission { .. }))
                    .map(|i| i.id)
                    .collect::<Vec<_>>();
                if let Some(item) = self.rng.pick(&held).copied() {
                    self.after(pause * 3, move |core| core.withdraw(item));
                    return true;
                }
            }
            // The coordinator logs a decision on a live task.
            13 => {
                let tasks: Vec<TaskId> = self
                    .state
                    .tasks
                    .iter()
                    .filter(|t| t.status == TaskStatus::InProgress)
                    .map(|t| t.id)
                    .collect();
                let pick = self.rng.pick(world::DECISIONS).copied();
                if let (Some(task), Some((text, why, alternative))) =
                    (self.rng.pick(&tasks).copied(), pick)
                {
                    self.after(pause, move |core| {
                        core.log_decision(
                            DecisionAuthor::Agent(agent_id(COORDINATOR)),
                            NewDecision {
                                task,
                                text: text.into(),
                                why: Some(why.into()),
                                alternatives: vec![Alternative {
                                    option: alternative.into(),
                                    why_not: None,
                                }],
                                supersedes: None,
                            },
                        )
                    });
                    return true;
                }
            }
            // A startup-style warning comes and goes (messaging blockers, plan section 7).
            12 => {
                let id = WarningId("messaging".into());
                if self.state.warnings.iter().any(|w| w.id == id) {
                    self.after(pause * 4, move |core| core.emit(Event::WarningCleared(id)));
                } else {
                    self.after(pause, move |core| {
                        core.emit(Event::WarningSet(Warning {
                            id,
                            text: "DISABLE_TELEMETRY is set: messaging may not work".into(),
                        }))
                    });
                }
                return true;
            }
            _ => {}
        }
        false
    }

    /// Relates two tasks both ways, or removes the relation.
    fn relate(&mut self, a: TaskId, b: TaskId, on: bool) {
        if a == b || self.task(b).is_none() {
            return;
        }
        for (this, that) in [(a, b), (b, a)] {
            if let Some(mut task) = self.task(this) {
                let had = task.related.contains(&that);
                if on && !had {
                    task.related.push(that);
                } else if !on && had {
                    task.related.retain(|t| *t != that);
                } else {
                    continue;
                }
                self.emit(Event::TaskUpserted(task));
            }
        }
    }

    fn log_decision(&mut self, by: DecisionAuthor, new: NewDecision) {
        let id = DecisionId(
            self.state
                .decisions
                .iter()
                .map(|d| d.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        self.emit(Event::DecisionAdded(Decision {
            id,
            task: new.task,
            at_ms: self.state.now_ms,
            by,
            text: new.text,
            why: new.why,
            alternatives: new.alternatives,
            supersedes: new.supersedes,
        }));
    }

    fn file_follow_up(&mut self, title: &str, follow_up_of: Option<TaskId>) {
        let task = Task {
            id: TaskId(self.next_task),
            title: title.into(),
            status: TaskStatus::Todo,
            p_level: None,
            bucket: None,
            order: self.next_order(None),
            deps: vec![],
            notes: String::new(),
            pr: None,
            follow_up_of,
            coordinator_created: true,
            related: vec![],
        };
        self.next_task += 1;
        self.emit(Event::TaskUpserted(task));
        self.activity(
            Some(agent_id(COORDINATOR)),
            ActivityKind::Lifecycle,
            format!("filed “{title}”"),
        );
    }

    /// A permission request disappears because Claude Code withdrew it.
    fn withdraw(&mut self, item: InboxId) {
        let Some(found) = self.inbox_item(item) else {
            return;
        };
        self.resolve(item);
        self.append(
            &found.agent,
            EntryKind::Notice,
            "Permission request withdrawn".into(),
        );
    }

    fn message(&mut self, from: AgentId, to: AgentId, text: String) {
        let message = AgentMessage {
            id: self.next_message,
            at_ms: self.state.now_ms,
            from: from.clone(),
            to: to.clone(),
            text,
        };
        self.next_message += 1;
        self.emit(Event::MessageAdded(message));
        self.activity(
            Some(from.clone()),
            ActivityKind::Message,
            format!("{} → {}", from.0, to.0),
        );
    }

    fn advance_todos(&mut self, agent: &AgentId) {
        let mut lists: BTreeMap<TodoListId, Vec<TodoItem>> = BTreeMap::new();
        for item in self
            .state
            .todos
            .iter()
            .filter(|t| t.created_by.as_ref() == Some(agent))
        {
            lists
                .entry(item.list.clone())
                .or_default()
                .push(item.clone());
        }
        let Some((list, mut items)) = lists.into_iter().next() else {
            return;
        };
        if let Some(current) = items
            .iter_mut()
            .find(|t| t.status == TodoStatus::InProgress)
        {
            current.status = TodoStatus::Completed;
        }
        if let Some(next) = items.iter_mut().find(|t| t.status == TodoStatus::Pending) {
            next.status = TodoStatus::InProgress;
        }
        self.emit(Event::TodosReplaced { list, items });
    }

    // Subagents

    /// Starts a subagent under `parent`: a background one with progress and a summary, or a
    /// foreground one that works inside the parent's turn.
    fn spawn_subagent(&mut self, parent: &AgentId, background: bool) {
        let Some(parent_agent) = self.agent(parent) else {
            return;
        };
        let id = format!(
            "{}-{}",
            if background { "bg" } else { "fg" },
            self.next_subagent
        );
        self.next_subagent += 1;
        let description = self
            .rng
            .pick(world::SUBAGENT_TASKS)
            .copied()
            .unwrap_or_default();
        let mut sub = world::subagent(&id, &parent_agent, background, self.state.now_ms);
        sub.status_line = description.into();
        if background {
            sub.progress = Some(SubagentProgress {
                description: description.into(),
                last_tool: None,
                tool_uses: 0,
                summary: None,
                ..SubagentProgress::default()
            });
        }
        self.emit(Event::AgentUpserted(sub));
        self.prune_finished_subagents(parent);

        let sub_id = agent_id(&id);
        let parent = parent.clone();
        let ticks = self.rng.range(2, 5);
        for _ in 0..ticks {
            let tool = self.rng.pick(world::TOOLS).copied().unwrap_or_default();
            let delay = self.rng.range(800, 2_000);
            let target = sub_id.clone();
            self.after(delay, move |core| {
                core.update_agent(&target, |a| {
                    if let Some(p) = a.progress.as_mut() {
                        p.tool_uses += 1;
                        p.last_tool = Some(tool.into());
                    }
                    a.cost_usd = round_cents(a.cost_usd + 0.01);
                })
            });
        }
        let delay = self.rng.range(800, 2_000);
        self.after(delay, move |core| {
            core.update_agent(&sub_id, |a| {
                a.status = AgentStatus::Done;
                if let Some(p) = a.progress.as_mut() {
                    p.summary = Some(format!(
                        "Finished: {}. Three matches, all in the tree buffer.",
                        p.description
                    ));
                }
            });
            core.activity(
                Some(sub_id.clone()),
                ActivityKind::Lifecycle,
                format!("{} finished", sub_id.0),
            );
            core.refresh_totals();
            // The parent's turn only really ends once its background work has reported.
            if background
                && !core.replying.contains(&parent)
                && !core.running_background_children(&parent)
            {
                core.end_turn(
                    &parent,
                    "Background work finished; results are in.".into(),
                    0.0,
                );
            }
        });
    }

    /// Keeps at most one finished subagent per parent.
    fn prune_finished_subagents(&mut self, parent: &AgentId) {
        let finished: Vec<AgentId> = self
            .state
            .agents
            .iter()
            .filter(|a| a.parent.as_ref() == Some(parent) && a.status == AgentStatus::Done)
            .map(|a| a.id.clone())
            .collect();
        if finished.len() > 1 {
            for old in &finished[..finished.len() - 1] {
                self.remove_agent(old);
            }
        }
    }
}

/// Runs a mock core on its own thread, pacing messages in real time.
///
/// `speed` scales the virtual delays: 1.0 is real time, 2.0 twice as fast. The thread ends on
/// [`Command::Quit`] or when the UI drops its end of the link.
pub fn spawn(config: MockConfig, speed: f64) -> CoreLink {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (message_tx, message_rx) = mpsc::channel::<ServerMsg>();
    thread::spawn(move || {
        let mut core = MockCore::new(config);
        for msg in core.connect() {
            if message_tx.send(msg).is_err() {
                return;
            }
        }
        while !core.has_quit() {
            let delay =
                Duration::from_secs_f64(core.peek_delay() as f64 / 1_000.0 / speed.max(0.01));
            let started = Instant::now();
            match command_rx.recv_timeout(delay) {
                Ok(command) => {
                    let passed = started.elapsed().as_secs_f64() * 1_000.0 * speed;
                    core.advance(passed as u64);
                    core.handle(command);
                    // Deliver the command's effects at once.
                    while core.peek_delay() == 0 && !core.has_quit() {
                        if message_tx.send(core.pop()).is_err() {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if message_tx.send(core.pop()).is_err() {
                        return;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    CoreLink {
        commands: command_tx,
        messages: message_rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sessions_fill_grows_by_tokens_and_drops_when_it_compacts() {
        let mut core = MockCore::new(MockConfig::default());
        let coordinator = agent_id(COORDINATOR);
        let start = core
            .state
            .agents
            .iter()
            .find(|a| a.id == coordinator)
            .and_then(|a| a.context_pct)
            .expect("the coordinator has a fill");
        let grown = core.grow_context(&coordinator, 40_000, 100_000);
        assert_eq!(grown, Some(start + 4));
        // Past 95 % the session compacts to the given size.
        let mut pct = grown;
        while pct.is_some_and(|p| p >= 20) {
            pct = core.grow_context(&coordinator, 40_000, 100_000);
        }
        assert_eq!(pct, Some(10));
        // A subagent has no fill and gets none.
        let sub = core.state.agents.iter().find(|a| !a.kind.is_session());
        if let Some(sub) = sub.map(|a| a.id.clone()) {
            assert_eq!(core.grow_context(&sub, 40_000, 100_000), None);
        }
    }
}
