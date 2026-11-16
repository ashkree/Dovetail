// MockCore: a fake core behind the CoreLink seam. It answers every Command with Events (each
// with the next seq), folded into its own copy with the same apply() the client uses, and a
// scripted coordinator reply 600 ms after a message (the only timer in the frontend, plan §3).
// Only main.tsx imports this file.

import { apply } from './apply';
import { homeSnapshot, projectSnapshot } from './fixtures';
import type { CoreLink } from './link';
import type { Effort, Id, InboxItem, Snapshot, TagColour } from './model';
import type { Command, Event, ServerMsg } from './protocol';

const REPLY_DELAY_MS = 600;
const COLOURS: TagColour[] = ['blue', 'coral', 'lilac', 'mint', 'gold'];

export class MockCore implements CoreLink {
  private state: Snapshot = homeSnapshot(0);
  /** Each project's state as last left, so going Home and back keeps what changed. */
  private saved = new Map<Id, Snapshot>();
  private subs = new Set<(msg: ServerMsg) => void>();
  private nextId = 1;

  subscribe(fn: (msg: ServerMsg) => void): () => void {
    this.subs.add(fn);
    fn({ type: 'snapshot', snapshot: this.state });
    return () => { this.subs.delete(fn); };
  }

  send(cmd: Command): void {
    const s = this.state;
    switch (cmd.type) {
      case 'resync':
        this.broadcast({ type: 'snapshot', snapshot: s });
        return;
      case 'open_project': {
        if (s.open_project === cmd.project_id) return;
        if (s.open_project) this.saved.set(s.open_project, s);
        const kept = this.saved.get(cmd.project_id);
        this.state = kept ? { ...kept, seq: s.seq } : projectSnapshot(s.seq, cmd.project_id);
        this.broadcast({ type: 'snapshot', snapshot: this.state });
        return;
      }
      case 'send_message': {
        const effort = s.efforts.find(e => e.id === cmd.effort_id);
        this.emit(
          { type: 'chat_entry_added', entry: { id: this.id('m'), who: 'you', report: false, at: clock(), effort_id: cmd.effort_id, text: cmd.text } },
          { type: 'status_changed', status: { ...s.status, busy: true } },
        );
        setTimeout(() => this.emit(
          { type: 'chat_entry_added', entry: { id: this.id('m'), who: 'coordinator', report: false, at: clock(), effort_id: cmd.effort_id, text: `Noted for **${effort?.name ?? 'this effort'}**. *(MockCore's coordinator only echoes.)*` } },
          { type: 'status_changed', status: { ...this.state.status, busy: false } },
        ), REPLY_DELAY_MS);
        return;
      }
      case 'answer_inbox': {
        const item = s.inbox.find(i => i.id === cmd.id);
        if (item) this.emit(...this.answer(item, cmd.answer, cmd.reason));
        return;
      }
      case 'place_card': {
        const inc = s.incoming.find(c => c.id === cmd.incoming_id);
        if (!inc) return;
        this.emit(
          { type: 'incoming_removed', id: inc.id },
          { type: 'card_added', card: { id: inc.id, canvas_id: inc.canvas_id, kind: inc.kind, x: cmd.x, y: cmd.y, title: inc.title, body: inc.body } },
          { type: 'card_link_added', link: { from: inc.under, to: inc.id } },
        );
        return;
      }
      case 'drop_incoming':
        this.emit({ type: 'incoming_removed', id: cmd.incoming_id });
        return;
      case 'move_card':
        this.emit({ type: 'card_moved', id: cmd.card_id, x: cmd.x, y: cmd.y });
        return;
      case 'delete_card':
        this.emit({ type: 'card_deleted', id: cmd.card_id });
        return;
      case 'add_card': {
        let n = s.cards.length + 1;
        const prefix = cmd.kind.charAt(0).toUpperCase();
        while (s.cards.some(c => c.id === prefix + n)) n++;
        const id = prefix + n;
        const events: Event[] = [{ type: 'card_added', card: { id, canvas_id: cmd.canvas_id, kind: cmd.kind, x: cmd.x, y: cmd.y, title: cmd.title, body: cmd.body } }];
        if (cmd.link_from) events.push({ type: 'card_link_added', link: { from: cmd.link_from, to: id } });
        this.emit(...events);
        return;
      }
      case 'add_task':
        this.emit({ type: 'task_added', task: { id: this.id('t'), effort_id: cmd.effort_id, title: cmd.title, status: 'todo' } });
        return;
      case 'rename_effort':
        this.emit({ type: 'effort_renamed', id: cmd.effort_id, name: cmd.name });
        return;
      case 'new_effort': {
        const effort: Effort = {
          id: this.id('e'), name: cmd.name, kind: cmd.kind, question_or_goal: cmd.question_or_goal, running: false,
          colour: COLOURS[s.efforts.length % COLOURS.length] ?? 'blue',
          where_was_i: { last: 'Effort created; no agents yet.', waits: 'Nothing.', next: 'Ask the coordinator to plan it.' },
          criteria: [],
        };
        this.emit({ type: 'effort_added', effort });
        return;
      }
      case 'keep_variants': {
        const set = s.variants.find(v => v.agent_id === cmd.agent_id);
        if (!set) return;
        const desc = set.slots.map((slot, i) => `${slot.name}: ${slot.options[cmd.choice[i] ?? 0] ?? '?'}`).join(' · ');
        this.emit(
          { type: 'variants_kept', agent_id: cmd.agent_id, kept: cmd.choice },
          { type: 'chat_entry_added', entry: { id: this.id('m'), who: 'you', report: false, at: clock(), effort_id: set.effort_id, text: `Keep this combination for **${cmd.agent_id}**: ${desc}` } },
        );
        return;
      }
    }
  }

  private answer(item: InboxItem, answer: string, reason?: string): Event[] {
    const done: Event = { type: 'inbox_resolved', id: item.id };
    const report = (text: string): Event => ({ type: 'chat_entry_added', entry: { id: this.id('m'), who: item.agent_id, report: true, at: clock(), effort_id: item.effort_id, text } });
    switch (item.kind) {
      case 'permission':
        if (answer === 'allow' || answer === 'allow_always') {
          const what = item.title.split(' · ').pop() ?? item.title;
          return [done, report(`Running ${what} (approved ${answer === 'allow' ? 'once' : 'always'})`)];
        }
        if (answer === 'deny') return [done, report(`Denied ${reason ? `: “${reason}”` : ''}`.trim())];
        return answer === 'dismiss' ? [done] : [];
      case 'criterion':
        if (answer === 'approve' && item.criterion_id) return [done, { type: 'criterion_changed', effort_id: item.effort_id, criterion_id: item.criterion_id, state: 'met' }];
        if (answer === 'send_back' && item.criterion_id) return [done, { type: 'criterion_changed', effort_id: item.effort_id, criterion_id: item.criterion_id, state: 'open' }];
        return answer === 'dismiss' ? [done] : [];
      case 'question':
        if (answer === 'reply' && reason) return [done, { type: 'chat_entry_added', entry: { id: this.id('m'), who: 'you', report: false, at: clock(), effort_id: item.effort_id, text: `@${item.agent_id} ${reason}` } }];
        return answer === 'dismiss' ? [done] : [];
      case 'finished':
        return answer === 'dismiss' ? [done] : [];
    }
  }

  private emit(...events: Event[]): void {
    for (const event of events) {
      const seq = this.state.seq + 1;
      const r = apply(this.state, seq, event);
      if (r.kind !== 'applied') continue;
      this.state = r.snapshot;
      this.broadcast({ type: 'event', seq, event });
    }
  }

  private broadcast(msg: ServerMsg): void {
    for (const fn of this.subs) fn(msg);
  }

  private id(prefix: string): Id {
    return `${prefix}-new-${this.nextId++}`;
  }
}

function clock(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}
