// What components ask for: UI commands, in the view's terms (model.ts). store.ts turns each into
// the core's Commands (translate.ts) or a change to the pending state (pending.ts). The wire
// itself carries the generated types in ./generated, from crates/dovetail-api.

import type { CardKind, EffortKind, Id } from './model';
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
  /** A slice in `repo` (by name; may be left out when the project has one). */
  | { type: 'new_slice'; name: string; brief: string; task_id?: Id; repo?: string }
  /** Ask for a fresh snapshot (after a gap). */
  | { type: 'resync' }
  | PendingAction;

export type { CardKind };
