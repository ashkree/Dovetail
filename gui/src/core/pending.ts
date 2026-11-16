// What the GUI shows that the core does not serve (yet): reports in the reading view, and the bodies of cards the core has sent on request (ADR 0025).
// It is held in the frontend and changed by the frontend's own reducer below. With the real core
// it starts empty for every project; MockCore seeds it with the fake data. Each part moves into
// the core's Snapshot when the core grows it, and leaves this file. (Canvases and cards left it
// with ADR 0025, Home's project details with ADR 0021: they are core state now.)

import type { Id, Report, Status, TagColour } from './model';

export interface Pending {
  reports: Report[];
  /** The bottom bar's machine-wide effort count and limit (design v2 open point 9). */
  status: Pick<Status, 'effort_limit' | 'machine_running'>;
  /** Tag colours, by effort id; an effort not listed gets one from its id. */
  colours: Record<Id, TagColour>;
  /** Bodies of cards that point at something, by view card id, as `load_card` brought them. */
  bodies: Record<Id, string>;
  /** Cards that open a report of `reports`, by view card id (MockCore's fake reports). */
  card_reports: Record<Id, Id>;
}

export function emptyPending(): Pending {
  return {
    reports: [], status: { effort_limit: 0, machine_running: 0 },
    colours: {}, bodies: {}, card_reports: {},
  };
}

/** A change to the pending state, from a UI command. */
export type PendingAction =
  /** A card's body arrived from the core. */
  | { type: 'card_body'; card_id: Id; body: string };

/** Folds one action into the pending state. Pure. */
export function applyPending(p: Pending, a: PendingAction): Pending {
  switch (a.type) {
    case 'card_body':
      return { ...p, bodies: { ...p.bodies, [a.card_id]: a.body } };
  }
}
