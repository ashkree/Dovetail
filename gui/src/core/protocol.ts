// What components ask for: UI commands, in the view's terms (model.ts). store.ts turns each into
// the core's Commands (translate.ts) or a change to the pending state (pending.ts). The wire
// itself carries the generated types in ./generated, from crates/dovetail-api.

import type { CardKind, EffortKind, Framework, Id } from './model';
import type { PendingAction } from './pending';

/** How the user answers an inbox item. */
export type InboxAnswer = 'allow' | 'allow_always' | 'deny' | 'approve' | 'send_back' | 'reply' | 'dismiss';

export type UiCommand =
  | { type: 'open_project'; project_id: Id }
  | { type: 'send_message'; text: string; effort_id: Id }
  /** `reason`: why it was denied or sent back; for `reply`, the reply's text. */
  | { type: 'answer_inbox'; id: Id; answer: InboxAnswer; reason?: string }
  | { type: 'add_task'; effort_id: Id; title: string }
  | { type: 'rename_effort'; effort_id: Id; name: string }
  | { type: 'new_effort'; name: string; kind: EffortKind; question_or_goal: string }
  /** ADR 0021: a name, an optional account and an optional first repo (a path inside it). */
  | { type: 'create_project'; name: string; account?: string; repo_path?: string }
  /** A repo by a path inside it; `name` defaults to its folder's. */
  | { type: 'add_repo'; project_id: Id; path: string; name?: string }
  | { type: 'remove_repo'; project_id: Id; repo: string }
  /** A slice in `repo` (by name; may be left out when the project has one). */
  /** The canvas and its cards (ADR 0025). Positions are canvas pixels. */
  | { type: 'new_canvas'; effort_id: Id; framework: Framework }
  /** Place an incoming card on its effort's canvas. */
  | { type: 'place_card'; incoming_id: Id; x: number; y: number }
  | { type: 'drop_incoming'; incoming_id: Id }
  /** Only the user moves cards; agents never do (ADR 0025). */
  | { type: 'move_card'; card_id: Id; x: number; y: number }
  | { type: 'delete_card'; card_id: Id }
  /** An idea or question card, its title the first line of its text. */
  | { type: 'add_card'; canvas_id: Id; kind: CardKind; x: number; y: number; title: string; body: string; link_from?: Id }
  | { type: 'link_cards'; from: Id; to: Id }
  | { type: 'unlink_cards'; from: Id; to: Id }
  /** Ask the core for a card's body. */
  | { type: 'load_card'; card_id: Id }
  /** Keep a chat reply as an idea card in the effort's incoming strip, by copying its text. */
  | { type: 'card_from_reply'; entry_id: Id; effort_id: Id }
  /** The viewer opened on a slice: start its preview unless it runs (ADR 0026). */
  | { type: 'start_preview'; agent_id: Id }
  | { type: 'restart_preview'; agent_id: Id }
  /** Pick an option (by its name) of a slot (by its name). */
  | { type: 'select_variant'; agent_id: Id; slot: string; option: string }
  /** "Keep this combination": the core marks the picks and prompts the slice. */
  | { type: 'keep_variants'; agent_id: Id }
  /** Open a web page in the user's browser, through the core. */
  | { type: 'open_external'; url: string }
  /** Move a session to its next permission mode. */
  | { type: 'cycle_permission_mode'; agent_id: Id }
  /** Ask for a fresh snapshot (after a gap). */
  | { type: 'resync' }
  | PendingAction;

export type { CardKind };
