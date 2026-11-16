// The GUI's reducer against the shared cases Rust writes (crates/dovetail-api/tests/apply_cases.rs):
// the same snapshot and events must give the same snapshot here as in Rust (ADR 0023, point 5).

import { describe, expect, it } from 'vitest';
import { apply } from './apply';
import type * as W from './generated';


interface Case {
  snapshot: W.Snapshot;
  events: { seq: number; event: W.Event }[];
  results: ('applied' | 'gap' | 'stale')[];
  expected: W.Snapshot;
}

const CASES = import.meta.glob<Case>('../../../crates/dovetail-api/tests/fixtures/apply/*.json', {
  eager: true, import: 'default',
});
const files = Object.keys(CASES).sort();

describe('apply matches Rust on the shared cases', () => {
  it('has cases to run', () => expect(files.length).toBeGreaterThan(0));
  for (const file of files) {
    it(file.split('/').pop() ?? file, () => {
      const c = structuredClone(CASES[file]) as Case;
      const before = JSON.stringify(c.snapshot);
      let state = c.snapshot;
      const results = c.events.map(({ seq, event }) => {
        const r = apply(state, seq, event);
        if (r.kind === 'applied') state = r.snapshot;
        return r.kind;
      });
      expect(results).toEqual(c.results);
      expect(state).toEqual(c.expected);
      // Pure: the starting snapshot is untouched.
      expect(JSON.stringify(c.snapshot)).toBe(before);
    });
  }
});
