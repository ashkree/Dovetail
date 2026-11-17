// A UI command in the core's terms: the Commands to send (generated types), a change to the
// pending state, or why it cannot be done. Pure; store.ts carries it out.

import type * as W from './generated';
import type { PendingAction } from './pending';
import type { UiCommand } from './protocol';
import { coreCardId, hasOwnText } from './canvasView';
import { COORDINATOR, variant } from './wire';

export interface Translated {
  core: W.Command[];
  pending?: PendingAction;
  /** Shown as a toast; nothing is sent. */
  refused?: string;
}

const only = (...core: W.Command[]): Translated => ({ core });
const refuse = (refused: string): Translated => ({ core: [], refused });

export function translate(cmd: UiCommand, core: W.Snapshot): Translated {
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
    case 'create_project':
      return only({ create_project: {
        name: cmd.name, account: cmd.account?.trim() || null, repo: cmd.repo_path?.trim() || null,
      } });
    case 'add_repo':
      return only({ add_repo: { project: cmd.project_id, path: cmd.path.trim(), name: cmd.name?.trim() || null } });
    case 'remove_repo': {
      const repo = core.projects.find(p => p.id === cmd.project_id)?.repos.find(r => r.name === cmd.repo);
      if (!repo) return { core: [], refused: `${cmd.project_id} has no repo ${cmd.repo}` };
      return only({ remove_repo: { project: cmd.project_id, repo: repo.id } });
    }
    case 'answer_inbox': return answer(cmd, core);
    case 'new_block': {
      const near = cmd.near ? { x: Math.round(cmd.near.x), y: Math.round(cmd.near.y) } : null;
      return only({ create_block: { effort: Number(cmd.effort_id), title: '', layout: cmd.layout, near } });
    }
    case 'place_card': return only({ place_card: { id: coreCardId(cmd.incoming_id), block: null } });
    case 'see_card': return only({ see_card: { id: coreCardId(cmd.card_id) } });
    case 'drop_incoming': return only({ drop_incoming: { id: coreCardId(cmd.incoming_id) } });
    case 'move_card': return only({ move_card: { id: coreCardId(cmd.card_id), x: cmd.x, y: cmd.y } });
    case 'delete_card': return only({ delete_card: { id: coreCardId(cmd.card_id) } });
    case 'add_card': {
      const block = core.blocks.find(b => String(b.id) === cmd.block_id);
      if (!block) return refuse('That block is gone.');
      if (!hasOwnText(cmd.kind)) return refuse(`A ${cmd.kind} card points at something that exists: research files it.`);
      const text = cmd.body ? `${cmd.title}\n${cmd.body}` : cmd.title;
      return only({
        add_card: {
          card: { effort: block.effort, block: block.id, kind: cmd.kind, reference: null, text },
          link_from: cmd.link_from ? coreCardId(cmd.link_from) : null,
        },
      });
    }
    case 'link_cards': return only({ link_cards: { from: coreCardId(cmd.from), to: coreCardId(cmd.to), label: '' } });
    case 'unlink_cards': return only({ unlink_cards: { from: coreCardId(cmd.from), to: coreCardId(cmd.to) } });
    case 'load_card': return only({ load_card: { id: coreCardId(cmd.card_id) } });
    case 'card_from_reply':
      return only({ card_from_reply: { agent: COORDINATOR, entry: Number(cmd.entry_id), effort: Number(cmd.effort_id) } });
    case 'cycle_permission_mode': return only({ cycle_permission_mode: { agent: cmd.agent_id } });
    case 'start_preview': return only({ start_preview: { agent: cmd.agent_id } });
    case 'restart_preview': return only({ restart_preview: { agent: cmd.agent_id } });
    case 'select_variant':
      return only({ select_variant: { agent: cmd.agent_id, slot: cmd.slot, option: cmd.option } });
    case 'keep_variants': return only({ keep_variants: { agent: cmd.agent_id } });
    case 'open_external': return only({ open_external: { url: cmd.url } });
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
