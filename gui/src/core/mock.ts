// MockCore: a fake core behind the CoreLink seam, speaking the real core's protocol (the
// generated types). It answers Commands with Events, each with the next seq, folded into its
// own copy with the same apply() the client uses, and sends a scripted coordinator reply 600 ms
// after a message (the only timer in the frontend). Only main.tsx imports this file.

import { apply } from './apply';
import { coreSnapshot } from './fixtures';
import type * as W from './generated';
import type { CoreLink } from './link';
import { COORDINATOR, variant } from './wire';

const REPLY_DELAY_MS = 600;

export class MockCore implements CoreLink {
  private state: W.Snapshot = coreSnapshot(0, null);
  /** Each project's state as last left, so going Home and back keeps what changed. */
  private saved = new Map<string, W.Snapshot>();
  private subs = new Set<(msg: W.ServerMsg) => void>();
  private nextId = 1000;

  subscribe(fn: (msg: W.ServerMsg) => void): () => void {
    this.subs.add(fn);
    fn({ snapshot: this.state });
    return () => { this.subs.delete(fn); };
  }

  send(cmd: W.Command): void {
    const s = this.state;
    const [tag, data] = variant(cmd);
    switch (tag) {
      case 'resync':
        this.broadcast({ snapshot: s });
        return;
      case 'open_project': {
        const { id } = data as { id: string };
        if (s.project?.id === id) return;
        // As the real core does: say so, stop, and the next project's core sends its snapshot.
        this.emit({ opening_project: { id } });
        if (s.project) this.saved.set(s.project.id, this.state);
        this.state = this.saved.get(id) ?? coreSnapshot(0, id);
        this.broadcast({ snapshot: this.state });
        return;
      }
      case 'prompt': {
        const p = data as { agent: string; text: string; effort?: number | null };
        if (p.agent !== COORDINATOR) return; // a slice's conversation is not shown in the mock
        const effort = p.effort ?? null;
        const name = s.efforts.find(e => e.id === effort)?.name;
        this.emit({ conversation_appended: { agent: COORDINATOR, entry: this.entry('user', p.text, effort) } });
        this.coordinator('working');
        setTimeout(() => {
          const text = `Noted for **${name ?? 'this effort'}**. *(MockCore's coordinator only echoes.)*`;
          this.emit({ conversation_appended: { agent: COORDINATOR, entry: this.entry('assistant', text, effort) } });
          this.coordinator('idle');
        }, REPLY_DELAY_MS);
        return;
      }
      case 'answer_permission': {
        const { item, answer } = data as { item: number; answer: W.PermissionAnswer };
        const found = s.inbox.find(i => i.id === item);
        if (!found) return;
        const [, kind] = variant(found.kind);
        const command = ((kind as { input?: { command?: string } }).input?.command) ?? 'the tool';
        const name = s.agents.find(a => a.id === found.agent)?.name ?? found.agent;
        const [how, detail] = variant(answer);
        const text = how === 'deny'
          ? `Denied${(detail as { message: string | null }).message ? `: “${(detail as { message: string }).message}”` : ''}`
          : `Running ${command} (approved ${how === 'allow' ? 'once' : 'always'})`;
        const effort = this.effortOf(found.agent);
        this.emit(
          { inbox_resolved: { id: item } },
          { conversation_appended: { agent: COORDINATOR, entry: this.entry('agent_message', `agent ${name}\n${text}`, effort) } },
        );
        return;
      }
      case 'answer_question':
      case 'dismiss_inbox': {
        const { item } = data as { item: number };
        this.emit({ inbox_resolved: { id: item } });
        return;
      }
      case 'approve_criterion':
      case 'return_criterion': {
        const { id } = data as { id: number };
        const effort = s.efforts.find(e => e.criteria.some(c => c.id === id));
        if (!effort) return this.fail(`no criterion ${id}`);
        const state: W.CriterionState = tag === 'approve_criterion' ? 'met' : 'open';
        this.emit({ effort_upserted: { ...effort, criteria: effort.criteria.map(c => (c.id === id ? { ...c, state } : c)) } });
        return;
      }
      case 'create_effort': {
        const n = data as W.NewEffort;
        const name = kebab(n.name);
        if (!name || s.efforts.some(e => e.name === name)) return this.fail(`there is already an effort named ${name}`);
        this.emit({ effort_upserted: {
          id: Math.max(0, ...s.efforts.map(e => e.id)) + 1, name, kind: n.kind, question_or_goal: n.question_or_goal.trim(),
          status: 'open', where_was_i: { last_done: '', waits_on_you: '', next_step: '' }, criteria: [],
        } });
        return;
      }
      case 'rename_effort': {
        const { id, name } = data as { id: number; name: string };
        const effort = s.efforts.find(e => e.id === id);
        if (effort) this.emit({ effort_upserted: { ...effort, name: kebab(name) } });
        return;
      }
      case 'create_task': {
        const t = data as W.NewTask;
        this.emit({ task_upserted: {
          id: Math.max(0, ...s.tasks.map(x => x.id)) + 1, title: t.title, status: 'todo', p_level: null, bucket: null,
          order: 0, deps: [], notes: '', pr: null, follow_up_of: null, coordinator_created: false, related: [],
          effort: t.effort ?? 1,
        } });
        return;
      }
      case 'create_project': {
        const c = data as { name: string; account: string | null; repo: string | null };
        const name = c.name.trim();
        if (!name) return this.fail('a project needs a name');
        let id = kebab(name) || 'project';
        for (let n = 2; s.projects.some(p => p.id === id); n++) id = `${kebab(name)}-${n}`;
        const repos = c.repo ? [this.repo(c.repo, null)] : [];
        this.emit({ project_upserted: {
          id, name, repo_path: repos[0]?.path ?? `/home/you/.local/share/dovetail/projects/${id}`,
          kind: repos.length ? 'code' : 'research', repos, account: c.account, last_opened_ms: Date.now(),
          counts: { efforts: 0, running: 0, needs_you: 0 },
        } });
        return;
      }
      case 'add_repo': {
        const a = data as { project: string; path: string; name: string | null };
        const p = s.projects.find(x => x.id === a.project);
        if (!p) return this.fail(`there is no project ${a.project}`);
        const repo = this.repo(a.path, a.name);
        if (p.repos.some(r => r.name === repo.name || r.path === repo.path)) {
          return this.fail(`project ${p.id} already has a repo named ${repo.name} or at ${repo.path}`);
        }
        this.emit({ project_upserted: { ...p, kind: 'code', repos: [...p.repos, repo] } });
        return;
      }
      case 'new_slice': {
        const n = data as { task: number | null; name: string; brief: string; repo: string | null };
        const repos = s.project?.repos ?? [];
        if (!repos.length) return this.fail('this is a research project: it has no repo, so it runs no slices');
        if (!n.repo && repos.length > 1) {
          return this.fail(`this project has several repos: name one with \`repo\` (${repos.map(r => r.name).join(', ')})`);
        }
        if (s.agents.some(a => a.name === n.name)) return this.fail(`a slice named \`${n.name}\` already exists`);
        const coord = s.agents.find(a => a.id === COORDINATOR);
        if (!coord) return;
        this.emit({ agent_upserted: {
          ...coord, id: `slice-${n.name}`, name: n.name, kind: 'slice', task: n.task, status: 'setting_up',
          worktree: { path: `/home/you/.local/share/dovetail/projects/${s.project?.id}/worktrees/${n.name}`, branch: `dovetail/${n.name}` },
        } });
        return;
      }
      default:
        this.fail(`${tag} is not in the mock core`);
    }
  }

  /** A repo as the mock registers one: named after its folder, or as asked. */
  private repo(path: string, name: string | null): W.RepoSummary {
    const clean = path.trim().replace(/\/+$/, '');
    return { id: this.nextId++, name: kebab(name || clean.split('/').pop() || 'repo'), path: clean };
  }

  private entry(kind: W.EntryKind, text: string, effort: number | null): W.ConversationEntry {
    return { id: this.nextId++, at_ms: Date.now(), kind, text, ...(effort !== null ? { effort } : {}) };
  }

  private effortOf(agent: string): number | null {
    const a = this.state.agents.find(x => x.id === agent);
    const task = a?.task ?? this.state.agents.find(x => x.id === a?.parent)?.task;
    return this.state.tasks.find(t => t.id === task)?.effort ?? null;
  }

  private coordinator(status: W.AgentStatus): void {
    const c = this.state.agents.find(a => a.id === COORDINATOR);
    if (c) this.emit({ agent_upserted: { ...c, status } });
  }

  private fail(message: string): void {
    this.emit({ command_failed: { message } });
  }

  private emit(...events: W.Event[]): void {
    for (const event of events) {
      const seq = this.state.seq + 1;
      const r = apply(this.state, seq, event);
      if (r.kind !== 'applied') continue;
      this.state = r.snapshot;
      this.broadcast({ event: { seq, event } });
    }
  }

  private broadcast(msg: W.ServerMsg): void {
    for (const fn of this.subs) fn(msg);
  }
}

/** As the core does (dovetail_api::effort_name): lower case, runs of anything else to one hyphen. */
function kebab(name: string): string {
  return name.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '');
}
