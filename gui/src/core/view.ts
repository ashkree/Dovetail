// The view: what the components read (model.ts), made from the core's Snapshot (the generated
// types, dovetail-api's names) and the pending state the core does not serve yet (pending.ts).
// Pure. This file is the one place that knows both shapes; components never see the core's.
//
// Ids become strings: an effort, task or criterion keeps its number as text ("7"); a core
// inbox item is "i<id>"; a claimed criterion waiting for the user is an inbox row "c<id>".

import type * as W from './generated';
import type {
  Agent, AgentState, ChatEntry, Criterion, Effort, Id, InboxItem, ProjectSummary, Snapshot, Status, TagColour,
  Task,
} from './model';
import { viewCanvas } from './canvasView';
import type { Pending } from './pending';
import { COORDINATOR, UNSORTED_EFFORT, variant } from './wire';

const COLOURS: TagColour[] = ['blue', 'coral', 'lilac', 'mint', 'gold'];

/** The view id of a core inbox item, and of a claimed criterion shown as an inbox row. */
export const inboxId = (id: number): Id => `i${id}`;
export const claimId = (id: number): Id => `c${id}`;

export function toView(core: W.Snapshot, pending: Pending, nowMs: number = Date.now()): Snapshot {
  const tasks = core.tasks.filter(t => t.status !== 'archived');
  const agents = viewAgents(core);
  const efforts = core.efforts
    .filter(e => e.status === 'open')
    // Unsorted shows only while it holds tasks.
    .filter(e => e.id !== UNSORTED_EFFORT || tasks.some(t => t.effort === e.id))
    .map(e => viewEffort(e, tasks, agents, pending));
  const inbox = [...claims(core, nowMs), ...core.inbox.map(i => viewInboxItem(core, i, nowMs))];
  const open = core.project?.id ?? null;
  const canvas = viewCanvas(core, pending);
  return {
    seq: core.seq,
    projects: core.projects.map(p => viewProject(p, open, efforts, inbox, core.profile, nowMs)),
    open_project: open,
    efforts,
    tasks: tasks.map(viewTask),
    agents,
    inbox,
    chat: viewChat(core),
    canvases: canvas.canvases,
    cards: canvas.cards,
    card_links: canvas.card_links,
    incoming: canvas.incoming,
    variants: pending.variants,
    reports: [...pending.reports, ...canvas.reports],
    status: viewStatus(core, pending),
  };
}

function viewProject(
  p: W.ProjectSummary, open: Id | null, efforts: Effort[], inbox: InboxItem[],
  profile: string | null | undefined, nowMs: number,
): ProjectSummary {
  const base: ProjectSummary = {
    id: p.id, name: p.name,
    repos: p.repos.map(r => r.name),
    account: p.account ?? '',
    efforts: p.counts.efforts, running: p.counts.running, needs_you: p.counts.needs_you,
    opened: openedText(p.last_opened_ms ?? null, nowMs),
  };
  if (p.id !== open) return base;
  // The open project's counts are live.
  return {
    ...base,
    account: profile ?? base.account,
    efforts: efforts.length,
    running: efforts.filter(e => e.running).length,
    needs_you: inbox.length,
  };
}

function viewEffort(e: W.Effort, tasks: W.Task[], agents: Agent[], pending: Pending): Effort {
  const id = String(e.id);
  const own = new Set(tasks.filter(t => t.effort === e.id).map(t => String(t.id)));
  return {
    id,
    name: e.name,
    kind: e.kind,
    question_or_goal: e.question_or_goal,
    running: agents.some(a => own.has(a.task_id) && (a.state === 'working' || a.state === 'needs_you')),
    colour: pending.colours[id] ?? COLOURS[(e.id + 3) % COLOURS.length] ?? 'blue',
    where_was_i: { last: e.where_was_i.last_done, waits: e.where_was_i.waits_on_you, next: e.where_was_i.next_step },
    criteria: e.criteria.map((c): Criterion => ({ id: String(c.id), text: c.text, state: c.state })),
  };
}

const viewTask = (t: W.Task): Task => ({
  id: String(t.id), effort_id: String(t.effort), title: t.title,
  // Archived tasks were filtered out before.
  status: t.status === 'archived' ? 'done' : t.status,
});

const STATE: Record<W.AgentStatus, AgentState> = {
  setting_up: 'working', working: 'working', idle: 'idle', needs_you: 'needs_you', done: 'done', error: 'needs_you',
};

/** Slices and their subagents, each under a task; other sessions are not in the tree. */
function viewAgents(core: W.Snapshot): Agent[] {
  const byId = new Map(core.agents.map(a => [a.id, a]));
  const out: Agent[] = [];
  for (const a of core.agents) {
    const [kind] = variant(a.kind);
    if (kind !== 'slice' && kind !== 'subagent') continue;
    const parent = a.parent ? byId.get(a.parent) : undefined;
    const task = a.task ?? parent?.task ?? null;
    if (task === null) continue;
    out.push({
      id: a.id,
      task_id: String(task),
      ...(kind === 'subagent' && a.parent ? { parent_id: a.parent } : {}),
      kind,
      name: a.name,
      state: STATE[a.status],
      ...(a.worktree ? { branch: a.worktree.branch } : {}),
    });
  }
  return out;
}

/** The effort an agent's work belongs to: its task's, or for the coordinator its current tag. */
function effortOfAgent(core: W.Snapshot, agent: W.AgentId): Id {
  const a = core.agents.find(x => x.id === agent);
  const task = a?.task ?? (a?.parent ? core.agents.find(x => x.id === a.parent)?.task : null);
  const effort = core.tasks.find(t => t.id === task)?.effort ?? lastTag(core, agent) ?? UNSORTED_EFFORT;
  return String(effort);
}

function lastTag(core: W.Snapshot, agent: W.AgentId): W.EffortId | null {
  const entries = core.conversations[agent]?.entries ?? [];
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e?.kind === 'user') return e.effort ?? null;
  }
  return null;
}

/** Claimed criteria wait for the user: each is one inbox row. */
function claims(core: W.Snapshot, nowMs: number): InboxItem[] {
  return core.efforts.flatMap(e => e.criteria.filter(c => c.state === 'claimed').map((c): InboxItem => ({
    id: claimId(c.id),
    kind: 'criterion',
    effort_id: String(e.id),
    agent_id: COORDINATOR,
    criterion_id: String(c.id),
    title: `Criterion · ${c.text}`,
    at: age(core.now_ms, nowMs),
    body: `**Acceptance criterion:** ${c.text}\n\n**Evidence:**\n\n${c.evidence ?? '*none given*'}\n\n`
      + 'Approve only when you are satisfied. Dovetail ticks the criterion on approval.',
  })));
}

function viewInboxItem(core: W.Snapshot, item: W.InboxItem, nowMs: number): InboxItem {
  const name = core.agents.find(a => a.id === item.agent)?.name ?? item.agent;
  const base = { id: inboxId(item.id), effort_id: effortOfAgent(core, item.agent), agent_id: item.agent, at: age(item.at_ms, nowMs) };
  const [tag, data] = variant(item.kind);
  switch (tag) {
    case 'permission': {
      const p = data as Extract<W.InboxKind, { permission: unknown }>['permission'];
      const arg = mainArgument(p.input);
      const reason = p.reason ? `\n\n${p.reason}` : '';
      return {
        ...base, kind: 'permission', title: arg ? `${p.tool} · ${arg}` : p.tool,
        body: `**${name}** wants to use **${p.tool}**:\n\n\`\`\`\n${arg || JSON.stringify(p.input, null, 2)}\n\`\`\`${reason}`,
      };
    }
    case 'question': {
      const { text } = data as { text: string };
      return { ...base, kind: 'question', title: firstLine(text), body: `**${name} asks:** ${text}` };
    }
    case 'finished_turn': {
      const { preview } = data as { preview: string };
      return { ...base, kind: 'finished', title: `${name} finished a turn`, body: preview };
    }
    case 'error': {
      const { text } = data as { text: string };
      return { ...base, kind: 'finished', title: `Error · ${firstLine(text)}`, body: text };
    }
    case 'proposal': {
      const p = data as { kind: string; text: string; why: string | null };
      return { ...base, kind: 'question', title: `Proposed ${p.kind} · ${firstLine(p.text)}`, body: p.why ? `${p.text}\n\n**Why:** ${p.why}` : p.text };
    }
    case 'unfiled_finding': {
      const f = data as { label: string; sources: number };
      return { ...base, kind: 'finished', title: `Unfiled finding · ${f.label}`, body: `A research finding with ${f.sources} sources waits to be filed under a task.` };
    }
    default: {
      const { count } = (data ?? { count: 0 }) as { count: number };
      return { ...base, kind: 'finished', title: `${count} new tangents`, body: 'Tangents wait to be sorted.' };
    }
  }
}

/** The coordinator's conversation: what the user wrote, its replies, slices' reports. */
function viewChat(core: W.Snapshot): ChatEntry[] {
  const entries = core.conversations[COORDINATOR]?.entries ?? [];
  const out: ChatEntry[] = [];
  for (const e of entries) {
    const tag = e.effort != null ? { effort_id: String(e.effort) } : {};
    const at = clock(e.at_ms);
    const id = String(e.id);
    switch (e.kind) {
      case 'user': out.push({ id, who: 'you', report: false, at, text: e.text, ...tag }); break;
      case 'assistant': out.push({ id, who: 'coordinator', report: false, at, text: e.text, ...tag }); break;
      case 'notice': out.push({ id, who: 'Dovetail', report: false, at, text: `*${e.text}*`, ...tag }); break;
      case 'agent_message': {
        const [head, ...rest] = e.text.split('\n');
        const from = head?.replace(/^(subagent|agent) /, '') ?? '';
        const who = core.agents.find(a => a.name === from)?.id ?? from;
        out.push({ id, who, report: true, at, text: rest.join('\n'), ...tag });
        break;
      }
      case 'tool_summary': break;
    }
  }
  return out;
}

function viewStatus(core: W.Snapshot, pending: Pending): Status {
  const c = core.agents.find(a => a.id === COORDINATOR);
  return {
    ...pending.status,
    account: core.profile ?? '',
    context_fill: (c?.context_pct ?? 0) / 100,
    usage: { five_hour: core.usage.five_hour?.utilization ?? 0, week: core.usage.seven_day?.utilization ?? 0 },
    model: modelName(c?.model ?? ''),
    effort_level: capital(c?.effort ?? ''),
    permission_mode: capital((c?.permission_mode ?? '').replace('_', ' ')),
    busy: core.agents.some(a => a.status === 'working'),
  };
}

/** When a project was last opened, as Home shows it: "today 09:12", "yesterday", "12 Nov". */
export function openedText(ms: number | null, nowMs: number): string {
  if (ms === null || ms <= 0) return '';
  const at = new Date(ms);
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(new Date(nowMs)) - day(at)) / 86_400_000);
  if (days === 0) return `today ${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')}`;
  if (days === 1) return 'yesterday';
  return `${at.getDate()} ${at.toLocaleString('en-GB', { month: 'short' })}`;
}

/** `claude-opus-5-5` → `Opus 5.5`; anything else as it is. */
export function modelName(model: string): string {
  const m = /^claude-([a-z]+)-(\d+)-(\d+)/.exec(model);
  return m ? `${capital(m[1] ?? '')} ${m[2]}.${m[3]}` : model;
}

/** What a tool call acts on: a command, a path, a pattern or a URL. */
function mainArgument(input: unknown): string {
  if (input && typeof input === 'object' && !Array.isArray(input)) {
    for (const key of ['command', 'file_path', 'path', 'pattern', 'url', 'query']) {
      const v = (input as Record<string, unknown>)[key];
      if (typeof v === 'string' && v) return firstLine(v);
    }
  }
  return '';
}

const firstLine = (text: string): string => text.split('\n')[0]?.trim() ?? '';
const capital = (s: string): string => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

function clock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** "now", "4m", "2h", "3d". */
function age(atMs: number, nowMs: number): string {
  const m = Math.max(0, Math.floor((nowMs - atMs) / 60_000));
  if (m < 1) return 'now';
  if (m < 60) return `${m}m`;
  if (m < 60 * 24) return `${Math.floor(m / 60)}h`;
  return `${Math.floor(m / (60 * 24))}d`;
}
