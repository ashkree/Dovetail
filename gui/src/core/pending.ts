// What the GUI shows that the core does not serve yet: canvases and cards (ADR 0025), the
// viewer's variants (design v2 §7) and reports in the reading view. It is held in the frontend and changed by the frontend's own reducer below.
// With the real core it starts empty for every project; MockCore seeds it with the fake data.
// Each part moves into the core's Snapshot when the core grows it, and leaves this file.

import type {
  Card, CardKind, CardLink, Canvas, Id, IncomingCard, Report, Status, TagColour, VariantSet,
} from './model';

export interface Pending {
  canvases: Canvas[];
  cards: Card[];
  card_links: CardLink[];
  incoming: IncomingCard[];
  variants: VariantSet[];
  reports: Report[];
  /** The bottom bar's machine-wide effort count and limit (design v2 open point 9). */
  status: Pick<Status, 'effort_limit' | 'machine_running'>;
  /** Tag colours, by effort id; an effort not listed gets one from its id. */
  colours: Record<Id, TagColour>;
}

export function emptyPending(): Pending {
  return {
    canvases: [], cards: [], card_links: [], incoming: [], variants: [], reports: [],
    status: { effort_limit: 0, machine_running: 0 }, colours: {},
  };
}

/** A change to the pending state, from a UI command. */
export type PendingAction =
  /** Place an incoming card on its canvas at a position the user's client chose. */
  | { type: 'place_card'; incoming_id: Id; x: number; y: number }
  | { type: 'drop_incoming'; incoming_id: Id }
  /** Only the user moves cards; agents never do (ADR 0025). */
  | { type: 'move_card'; card_id: Id; x: number; y: number }
  /** Also removes the card's links. */
  | { type: 'delete_card'; card_id: Id }
  | { type: 'add_card'; canvas_id: Id; kind: CardKind; x: number; y: number; title: string; body: string; link_from?: Id }
  | { type: 'keep_variants'; agent_id: Id; choice: number[] };

/** Folds one action into the pending state. Pure. */
export function applyPending(p: Pending, a: PendingAction): Pending {
  switch (a.type) {
    case 'place_card': {
      const inc = p.incoming.find(c => c.id === a.incoming_id);
      if (!inc) return p;
      return {
        ...p,
        incoming: p.incoming.filter(c => c.id !== inc.id),
        cards: [...p.cards, { id: inc.id, canvas_id: inc.canvas_id, kind: inc.kind, x: a.x, y: a.y, title: inc.title, body: inc.body }],
        card_links: [...p.card_links, { from: inc.under, to: inc.id }],
      };
    }
    case 'drop_incoming':
      return { ...p, incoming: p.incoming.filter(c => c.id !== a.incoming_id) };
    case 'move_card':
      return { ...p, cards: p.cards.map(c => (c.id === a.card_id ? { ...c, x: a.x, y: a.y } : c)) };
    case 'delete_card':
      return {
        ...p,
        cards: p.cards.filter(c => c.id !== a.card_id),
        card_links: p.card_links.filter(l => l.from !== a.card_id && l.to !== a.card_id),
      };
    case 'add_card': {
      const prefix = a.kind.charAt(0).toUpperCase();
      let n = p.cards.length + 1;
      while (p.cards.some(c => c.id === prefix + n)) n++;
      const id = prefix + n;
      const card: Card = { id, canvas_id: a.canvas_id, kind: a.kind, x: a.x, y: a.y, title: a.title, body: a.body };
      const links = a.link_from ? [...p.card_links, { from: a.link_from, to: id }] : p.card_links;
      return { ...p, cards: [...p.cards, card], card_links: links };
    }
    case 'keep_variants':
      return { ...p, variants: p.variants.map(v => (v.agent_id === a.agent_id ? { ...v, kept: a.choice } : v)) };
  }
}
