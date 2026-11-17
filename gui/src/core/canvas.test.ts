// Blocks and cards through the view, the translation and MockCore (ADR 0025, ADR 0027).

import { describe, expect, it } from 'vitest';
import { apply } from './apply';
import { cardViewId, coreCardId } from './canvasView';
import { coreSnapshot, pendingFor } from './fixtures';
import type * as W from './generated';
import { MockCore } from './mock';
import { applyPending } from './pending';
import { translate } from './translate';
import { toView } from './view';
import { emptyCoreSnapshot, variant } from './wire';

const core = coreSnapshot(0, 'shop');
const pending = pendingFor('shop');
const view = toView(core, pending);

describe('the canvas view', () => {
  it('shows the placed cards by kind letter and number, and the incoming ones apart', () => {
    expect(view.blocks).toEqual([{ id: '1', effort_id: '2', layout: 'question_tree', title: 'Which provider?', x: 20, y: -20 }]);
    expect(view.cards.map(c => c.id)).toEqual(['Q1', 'Q2', 'Q3', 'Q4', 'F5', 'S6', 'F7', 'I8', 'D9', 'M10']);
    expect(view.incoming.map(c => c.id)).toEqual(['F11', 'S12', 'Q13']);
    expect(view.incoming[0]).toMatchObject({ effort_id: '2', under: 'Q3' });
  });

  it("keeps an idea's or question's first line as its title and the rest as its body", () => {
    expect(view.cards.find(c => c.id === 'Q2')).toMatchObject({ title: 'Fees at our volume?', body: '~2 400 orders a month, €38 average.' });
  });

  it('shows no body for a card that points at something until it is loaded', () => {
    expect(view.cards.find(c => c.id === 'F5')?.body).toBe('');
    const loaded = toView(core, applyPending(pending, { type: 'card_body', card_id: 'F5', body: 'Cheapest published rate.' }));
    expect(loaded.cards.find(c => c.id === 'F5')?.body).toBe('Cheapest published rate.');
  });

  it('makes a long loaded body a report the card opens', () => {
    const long = `# Heading\n\n${'word '.repeat(80)}`;
    const v = toView(core, applyPending(pending, { type: 'card_body', card_id: 'S6', body: long }));
    expect(v.cards.find(c => c.id === 'S6')?.report_id).toBe('card-S6');
    expect(v.reports.find(r => r.id === 'card-S6')?.markdown).toBe(long);
    // The mock's own report stays on its card.
    expect(v.cards.find(c => c.id === 'F7')?.report_id).toBe('r-sca');
  });

  it('round-trips a card id', () => {
    expect(cardViewId({ id: 90, kind: 'finding' })).toBe('F90');
    expect(coreCardId('F90')).toBe(90);
  });

  it('marks a card an agent put into a block until it is seen', () => {
    const agentCard: W.Card = { ...core.cards[0]!, id: 40, seen: false, added_by: 'research-1' };
    const v = toView({ ...core, cards: [...core.cards, agentCard] }, pending);
    expect(v.cards.find(c => c.id === 'Q40')?.unseen).toBe(true);
    expect(v.cards.find(c => c.id === 'Q1')?.unseen).toBeUndefined();
  });

  it('has nothing for a project without blocks', () => {
    const empty = toView(emptyCoreSnapshot(), pendingFor(null));
    expect([empty.blocks, empty.cards, empty.incoming, empty.card_links]).toEqual([[], [], [], []]);
  });
});

describe('translating card commands', () => {
  it('leaves where a placed card goes to the core', () => {
    const t = translate({ type: 'place_card', incoming_id: 'F11' }, core);
    expect(t.core).toEqual([{ place_card: { id: 11, block: null } }]);
  });

  it('asks the core for a new block of a layout', () => {
    expect(translate({ type: 'new_block', effort_id: '3', layout: 'options_compare' }, core).core)
      .toEqual([{ create_block: { effort: 3, title: '', layout: 'options_compare' } }]);
  });

  it('sends a new idea with its title as the first line', () => {
    const t = translate({ type: 'add_card', block_id: '1', kind: 'idea', title: 'Ask Mollie', body: '', link_from: 'Q2' }, core);
    expect(t.core).toEqual([{
      add_card: { card: { effort: 2, block: 1, kind: 'idea', reference: null, text: 'Ask Mollie' }, link_from: 2 },
    }]);
  });

  it('refuses to type a finding into existence', () => {
    const t = translate({ type: 'add_card', block_id: '1', kind: 'finding', title: 'x', body: '' }, core);
    expect(t.core).toEqual([]);
    expect(t.refused).toMatch(/points at something/);
  });
});

describe('MockCore speaks the canvas protocol', () => {
  /** A client of the mock: the snapshot, then each event through apply(), as store.ts does. */
  function client() {
    const mock = new MockCore();
    const c = { state: emptyCoreSnapshot(), failures: [] as string[] };
    mock.subscribe(msg => {
      const [tag, data] = variant(msg);
      if (tag === 'snapshot') c.state = data as W.Snapshot;
      else if (tag === 'event') {
        const { seq, event } = data as { seq: number; event: W.Event };
        const [kind, payload] = variant(event);
        if (kind === 'command_failed') c.failures.push((payload as { message: string }).message);
        const r = apply(c.state, seq, event);
        if (r.kind === 'applied') c.state = r.snapshot;
      }
    });
    mock.send({ open_project: { id: 'shop' } });
    return { mock, c };
  }

  it('places, moves, links and deletes, and refuses what makes no sense', () => {
    const { mock, c } = client();
    expect(c.state.cards).toHaveLength(13);
    mock.send({ move_card: { id: 11, x: 1, y: 1 } }); // incoming: refused
    mock.send({ place_card: { id: 11, block: null } });
    // Under Q3, which links to it.
    const q3 = c.state.cards.find(x => x.id === 3)!;
    expect(c.state.cards.find(x => x.id === 11)).toMatchObject({ block: 1, x: q3.x });
    mock.send({ move_card: { id: 11, x: 120, y: 220 } });
    expect(c.state.cards.find(x => x.id === 11)).toMatchObject({ block: 1, x: 120, y: 220 });
    expect(c.failures).toEqual(['card 11 is incoming: place it first']);

    mock.send({ add_card: { card: { effort: 2, block: 1, kind: 'idea', reference: null, text: 'Ask Mollie' }, link_from: 1 } });
    const made = c.state.cards.at(-1);
    expect(made).toMatchObject({ id: 14, title: 'Ask Mollie', added_by: 'user', block: 1, seen: true });
    expect(c.state.card_links).toContainEqual({ from: 1, to: 14, label: '' });

    mock.send({ delete_card: { id: 14 } });
    expect(c.state.cards.some(x => x.id === 14)).toBe(false);
    expect(c.state.card_links.some(l => l.to === 14)).toBe(false);
    mock.send({ drop_incoming: { id: 12 } });
    expect(c.state.cards.some(x => x.id === 12)).toBe(false);
    mock.send({ drop_incoming: { id: 1 } }); // placed: refused
    expect(c.failures.at(-1)).toMatch(/on the canvas/);
  });

  it('adds a block right of the others, and makes one for a card placed where there is none', () => {
    const { mock, c } = client();
    mock.send({ create_block: { effort: 2, title: '', layout: 'free_area' } });
    const [first, second] = c.state.blocks;
    expect(second).toMatchObject({ id: 2, effort: 2, title: 'Free area', layout: 'free_area', y: 0 });
    expect(second!.x).toBeGreaterThan(first!.x + 600);
    mock.send({ add_card: { card: { effort: 3, block: null, kind: 'idea', reference: null, text: 'Loose' }, link_from: null } });
    mock.send({ place_card: { id: 14, block: null } });
    expect(c.state.blocks.at(-1)).toMatchObject({ id: 3, effort: 3, layout: 'free_area' });
    expect(c.state.cards.find(x => x.id === 14)).toMatchObject({ block: 3 });
    mock.send({ see_card: { id: 14 } });
    mock.send({ load_card: { id: 5 } });
    expect(c.failures).toEqual([]);
  });
});
