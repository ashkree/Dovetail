import { describe, expect, it } from 'vitest';
import { applyPending, emptyPending, type Pending } from './pending';

const base: Pending = {
  ...emptyPending(),
  variants: [{ agent_id: 'a', effort_id: '1', url: 'u', slots: [], kept: [0] }],
};

describe('applyPending', () => {
  it('keeps a combination of variants', () => {
    const p = applyPending(base, { type: 'keep_variants', agent_id: 'a', choice: [2] });
    expect(p.variants[0]?.kept).toEqual([2]);
    expect(base.variants[0]?.kept).toEqual([0]); // pure
  });

  it('keeps a card body that arrived', () => {
    const p = applyPending(base, { type: 'card_body', card_id: 'F5', body: 'text' });
    expect(p.bodies).toEqual({ F5: 'text' });
    expect(base.bodies).toEqual({});
  });
});
