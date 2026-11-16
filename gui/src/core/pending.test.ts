import { describe, expect, it } from 'vitest';
import { applyPending, emptyPending, type Pending } from './pending';

const base: Pending = {
  ...emptyPending(),
  cards: [
    { id: 'Q1', canvas_id: 'c', kind: 'question', x: 0, y: 0, title: 'q', body: '' },
    { id: 'F1', canvas_id: 'c', kind: 'finding', x: 0, y: 100, title: 'f', body: '' },
  ],
  card_links: [{ from: 'Q1', to: 'F1' }],
  incoming: [{ id: 'F2', canvas_id: 'c', kind: 'finding', title: 'new', body: '', under: 'Q1' }],
};

describe('applyPending', () => {
  it('deletes a card with its links', () => {
    const p = applyPending(base, { type: 'delete_card', card_id: 'F1' });
    expect(p.cards.map(c => c.id)).toEqual(['Q1']);
    expect(p.card_links).toEqual([]);
    expect(base.cards).toHaveLength(2); // pure
  });

  it('places an incoming card under its question', () => {
    const p = applyPending(base, { type: 'place_card', incoming_id: 'F2', x: 5, y: 6 });
    expect(p.incoming).toEqual([]);
    expect(p.cards.find(c => c.id === 'F2')).toMatchObject({ x: 5, y: 6 });
    expect(p.card_links).toContainEqual({ from: 'Q1', to: 'F2' });
  });

  it('numbers a new card after the ones its kind has', () => {
    const p = applyPending(base, { type: 'add_card', canvas_id: 'c', kind: 'finding', x: 1, y: 2, title: 't', body: '', link_from: 'Q1' });
    expect(p.cards.at(-1)?.id).toBe('F3');
    expect(p.card_links).toContainEqual({ from: 'Q1', to: 'F3' });
  });
});
