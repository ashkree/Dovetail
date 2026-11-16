// What crosses the CoreLink seam: Commands from the UI, ServerMsgs from the core.
// Mirrors crates/dovetail-api (protocol.rs); only what the GUI needs now (plan §3).

import type {
  Card, CardKind, CardLink, ChatEntry, CriterionState, Effort, EffortKind, Id, Snapshot, Status, Task,
} from './model';

/** How the user answers an inbox item. */
export type InboxAnswer = 'allow' | 'allow_always' | 'deny' | 'approve' | 'send_back' | 'reply' | 'dismiss';

export type Command =
  | { type: 'open_project'; project_id: Id }
  | { type: 'send_message'; text: string; effort_id: Id }
  /** `reason`: why it was denied or sent back; for `reply`, the reply's text. */
  | { type: 'answer_inbox'; id: Id; answer: InboxAnswer; reason?: string }
  /** Place an incoming card on its canvas at a position the user's client chose. */
  | { type: 'place_card'; incoming_id: Id; x: number; y: number }
  | { type: 'drop_incoming'; incoming_id: Id }
  /** Only the user moves cards; agents never do (design v2 §6). */
  | { type: 'move_card'; card_id: Id; x: number; y: number }
  | { type: 'delete_card'; card_id: Id }
  | { type: 'add_card'; canvas_id: Id; kind: CardKind; x: number; y: number; title: string; body: string; link_from?: Id }
  | { type: 'add_task'; effort_id: Id; title: string }
  | { type: 'rename_effort'; effort_id: Id; name: string }
  | { type: 'new_effort'; name: string; kind: EffortKind; question_or_goal: string }
  /** Tell the slice to make this combination permanent and delete the rest. */
  | { type: 'keep_variants'; agent_id: Id; choice: number[] }
  /** Ask for a fresh snapshot (after a gap). */
  | { type: 'resync' };

export type Event =
  | { type: 'chat_entry_added'; entry: ChatEntry }
  | { type: 'inbox_resolved'; id: Id }
  | { type: 'criterion_changed'; effort_id: Id; criterion_id: Id; state: CriterionState }
  | { type: 'card_added'; card: Card }
  | { type: 'card_link_added'; link: CardLink }
  | { type: 'incoming_removed'; id: Id }
  | { type: 'card_moved'; id: Id; x: number; y: number }
  /** Also removes the card's links. */
  | { type: 'card_deleted'; id: Id }
  | { type: 'task_added'; task: Task }
  | { type: 'effort_renamed'; id: Id; name: string }
  | { type: 'effort_added'; effort: Effort }
  | { type: 'variants_kept'; agent_id: Id; kept: number[] }
  | { type: 'status_changed'; status: Status };

/** One message from the core: a whole snapshot, or one event with its sequence number. */
export type ServerMsg =
  | { type: 'snapshot'; snapshot: Snapshot }
  | { type: 'event'; seq: number; event: Event };
