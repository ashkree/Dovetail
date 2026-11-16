// A UI command in the core's terms: the Commands to send (generated types), a change to the
// pending state, or why it cannot be done. Pure; store.ts carries it out.

import type * as W from './generated';
import type { Pending, PendingAction } from './pending';
import type { UiCommand } from './protocol';
import { COORDINATOR, variant } from './wire';

export interface Translated {
  core: W.Command[];
  pending?: PendingAction;
  /** Shown as a toast; nothing is sent. */
  refused?: string;
}

const only = (...core: W.Command[]): Translated => ({ core });
const refuse = (refused: string): Translated => ({ core: [], refused });

export function translate(cmd: UiCommand, core: W.Snapshot, pending: Pending): Translated {
  switch (cmd.type) {
    case 'resync': return only('resync');
    case 'open_project': return only({ open_project: { id: cmd.project_id } });
    case 'send_message':
      return only({ prompt: { agent: COORDINATOR, text: cmd.text, effort: Number(cmd.effort_id) } });
    case 'add_task':
      return only({
        create_task: {
          title: cmd.title, p_level: null, bucket: null, notes: '', follow_up_of: null, pr: null, related: [],
          effort: Number(cmd.effort_id),
        },
      });
    case 'rename_effort': return only({ rename_effort: { id: Number(cmd.effort_id), name: cmd.name } });
    case 'new_effort':
      return only({ create_effort: { name: cmd.name, kind: cmd.kind, question_or_goal: cmd.question_or_goal } });
    case 'answer_inbox': return answer(cmd, core);
    case 'keep_variants': {
      const set = pending.variants.find(v => v.agent_id === cmd.agent_id);
      const chosen = set
        ? set.slots.map((slot, i) => `${slot.name}: ${slot.options[cmd.choice[i] ?? 0] ?? '?'}`).join('; ')
        : cmd.choice.join(', ');
      const text = `Keep this combination of variants: ${chosen}. Make it permanent and delete the other variants.`;
      return { core: [{ prompt: { agent: cmd.agent_id, text } }], pending: cmd };
    }
    default: return { core: [], pending: cmd };
  }
}

function answer(cmd: Extract<UiCommand, { type: 'answer_inbox' }>, core: W.Snapshot): Translated {
  const reason = cmd.reason?.trim() || null;
  // A claimed criterion: only approving or sending it back answers it.
  if (cmd.id.startsWith('c')) {
    const id = Number(cmd.id.slice(1));
    if (cmd.answer === 'approve') return only({ approve_criterion: { id } });
    if (cmd.answer === 'send_back') return only({ return_criterion: { id, reason } });
    return refuse('A claimed criterion is approved or sent back.');
  }
  const item = Number(cmd.id.slice(1));
  const found = core.inbox.find(i => i.id === item);
  if (!found) return refuse('That item has gone.');
  const [kind] = variant(found.kind);
  if (cmd.answer === 'dismiss') return only({ dismiss_inbox: { item } });
  if (kind === 'permission' || kind === 'proposal') {
    if (cmd.answer === 'allow') return only({ answer_permission: { item, answer: 'allow' } });
    if (cmd.answer === 'allow_always') return only({ answer_permission: { item, answer: 'allow_always' } });
    if (cmd.answer === 'deny') return only({ answer_permission: { item, answer: { deny: { message: reason } } } });
  }
  if (cmd.answer === 'reply' && reason) return only({ answer_question: { item, text: reason } });
  return refuse('That answer does not fit this item.');
}
