import { describe, expect, it } from 'vitest';
import { dropMissing } from './geometry';

describe('dropMissing', () => {
  it('removes the entries whose id is no longer live and keeps the rest', () => {
    const cache = new Map([['Q1', 1], ['F2', 2], ['I3', 3]]);
    dropMissing(cache, ['Q1', 'I3', 'D9']);
    expect([...cache.keys()]).toEqual(['Q1', 'I3']);
  });
});
