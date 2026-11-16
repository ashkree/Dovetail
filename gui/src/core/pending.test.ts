import { describe, expect, it } from 'vitest';
import { applyPending, emptyPending } from './pending';

describe('applyPending', () => {
  it('keeps a card body that arrived', () => {
    const base = emptyPending();
    const p = applyPending(base, { type: 'card_body', card_id: 'F5', body: 'text' });
    expect(p.bodies).toEqual({ F5: 'text' });
    expect(base.bodies).toEqual({});
  });
});
