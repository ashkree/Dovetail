// The canvas part of the view (ADR 0025, ADR 0027): blocks, cards, links and the incoming strip,
// made from the core's Snapshot. Pure. A card's view id is its kind's letter and its core number
// (`Q4`, `F90`): short, shown, and what a chat reference names. Bodies of cards that point at
// something come with `load_card` and are kept in the pending state (`bodies`).

import type * as W from './generated';
import type { Block, Card, CardLink, Id, IncomingCard, Report } from './model';
import type { Pending } from './pending';

const LETTER: Record<W.CardKind, string> = {
  finding: 'F', source: 'S', idea: 'I', decision: 'D', question: 'Q', map: 'M', task: 'T',
};

/** The view id of a core card. */
export const cardViewId = (c: Pick<W.Card, 'id' | 'kind'>): Id => LETTER[c.kind] + c.id;

/** The core id a view card id stands for. */
export const coreCardId = (viewId: Id): number => Number(viewId.slice(1));

/** A card that holds its own words rather than pointing at something. */
export const hasOwnText = (kind: string): boolean => kind === 'idea' || kind === 'question';

/** A loaded body longer than this is a document: it opens in the reading view. */
const SHORT_BODY = 160;

export interface CanvasView {
  blocks: Block[];
  cards: Card[];
  card_links: CardLink[];
  incoming: IncomingCard[];
  /** The loaded bodies that are long enough to read as documents, one report each. */
  reports: Report[];
}

export function viewCanvas(core: W.Snapshot, pending: Pending): CanvasView {
  const blocks: Block[] = core.blocks.map(b => ({
    id: String(b.id), effort_id: String(b.effort), layout: b.layout, title: b.title, x: b.x, y: b.y,
  }));
  const viewId = new Map(core.cards.map(c => [c.id, cardViewId(c)]));
  const card_links: CardLink[] = core.card_links.flatMap(l => {
    const from = viewId.get(l.from);
    const to = viewId.get(l.to);
    return from && to ? [{ from, to }] : [];
  });
  const reports: Report[] = [];
  const base = (c: W.Card) => {
    const id = viewId.get(c.id) ?? cardViewId(c);
    const loaded = pending.bodies[id];
    const own = pending.card_reports[id];
    let report_id = own;
    if (!report_id && loaded !== undefined && loaded.length > SHORT_BODY) {
      report_id = `card-${id}`;
      reports.push({ id: report_id, effort_id: String(c.effort), title: c.title, markdown: loaded });
    }
    return {
      id, kind: c.kind, title: c.title, body: c.reference ? excerpt(loaded) : restOf(c.text),
      ...(report_id ? { report_id } : {}),
    };
  };
  const cards: Card[] = core.cards.flatMap(c => (c.block == null ? [] : [{
    ...base(c), block_id: String(c.block), x: c.x, y: c.y, ...(c.seen ? {} : { unseen: true }),
  }]));
  const incoming: IncomingCard[] = core.cards.flatMap(c => {
    if (c.block != null) return [];
    // The card it would hang under: the one that links to it, if it is placed.
    const parent = core.card_links.map(l => (l.to === c.id ? core.cards.find(p => p.id === l.from) : undefined))
      .find(p => p?.block != null);
    return [{
      ...base(c), effort_id: String(c.effort),
      ...(parent ? { under: cardViewId(parent) } : {}),
    }];
  });
  return { blocks, cards, card_links, incoming, reports };
}

/** What follows the first line: an idea's or question's title is its first line. */
function restOf(text: string): string {
  const lines = text.split('\n');
  const first = lines.findIndex(l => l.trim() !== '');
  return first < 0 ? '' : lines.slice(first + 1).join('\n').trim();
}

/** A loaded body, cut for the card. */
function excerpt(body: string | undefined): string {
  if (!body) return '';
  const first = body.trim().split(/\n\s*\n/)[0] ?? '';
  const flat = first.replace(/\s+/g, ' ');
  return flat.length > SHORT_BODY ? `${flat.slice(0, SHORT_BODY - 1)}…` : flat;
}
