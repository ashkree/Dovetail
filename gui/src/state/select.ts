// Derived views of the snapshot that several components and the dispatcher share. Pure.

import type { Agent, ChatEntry, Effort, Id, Report, Snapshot, Task, TaskStatus } from '../core/model';

export const effortById = (s: Snapshot, id: Id | null | undefined): Effort | undefined =>
  id ? s.efforts.find(e => e.id === id) : undefined;

export const runningEfforts = (s: Snapshot): number => s.efforts.filter(e => e.running).length;

/** Inbox items that wait on the user; finished turns are listed but not counted (ADR 0028). */
export const needsYou = (s: Pick<Snapshot, 'inbox'>): number => s.inbox.filter(i => i.kind !== 'finished').length;

export const KIND_GLYPH: Record<Effort['kind'], string> = {
  feature: '✦', fix: '⚒', cleanup: '⌫', research: '⌕', experiment: '⚗',
};

export const STATUS_LABEL: Record<TaskStatus, string> = {
  todo: 'To do', in_progress: 'In progress', review: 'Review', done: 'Done',
};

export const INBOX_GLYPH = { permission: '⚠', criterion: '☑', question: '?', finished: '✓' } as const;

/** A speaker's display name: "You", "coordinator", or the agent's name. */
export function speakerName(s: Snapshot, who: ChatEntry['who']): string {
  if (who === 'you') return 'You';
  if (who === 'coordinator') return 'coordinator';
  return s.agents.find(a => a.id === who)?.name ?? who;
}

export type TreeRow =
  | { type: 'effort'; effort: Effort }
  | { type: 'task'; effort: Effort; task: Task }
  | { type: 'agent'; effort: Effort; agent: Agent; depth: number };

/** The efforts tree flattened to rows: efforts → tasks → agents → their subagents. */
export function treeRows(s: Snapshot, folded: Record<Id, boolean>): TreeRow[] {
  const rows: TreeRow[] = [];
  const addAgents = (effort: Effort, agents: Agent[], depth: number) => {
    for (const agent of agents) {
      rows.push({ type: 'agent', effort, agent, depth });
      addAgents(effort, s.agents.filter(a => a.parent_id === agent.id), depth + 1);
    }
  };
  for (const effort of s.efforts) {
    rows.push({ type: 'effort', effort });
    if (folded[effort.id]) continue;
    for (const task of s.tasks.filter(t => t.effort_id === effort.id)) {
      rows.push({ type: 'task', effort, task });
      addAgents(effort, s.agents.filter(a => a.task_id === task.id && !a.parent_id), 0);
    }
  }
  return rows;
}

/** The chat entries the filter shows, in order. */
export function visibleChat(s: Snapshot, filter: 'all' | Id): ChatEntry[] {
  return filter === 'all' ? s.chat : s.chat.filter(m => m.effort_id === filter);
}

/** Effort names are lower kebab case: "Dark mode" is saved as `dark-mode`. */
export function toKebab(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
}

/** The report the reading view shows: the selected card's, else the focused effort's, else the first. */
export function readingReport(s: Snapshot, cardSel: Id | null, effort: Id | null): Report | undefined {
  const card = s.cards.find(c => c.id === cardSel);
  return s.reports.find(r => r.id === card?.report_id)
    ?? s.reports.find(r => r.effort_id === effort)
    ?? s.reports[0];
}
