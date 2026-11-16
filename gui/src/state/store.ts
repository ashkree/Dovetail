// Holds the client's Snapshot and the one CoreLink. Components read with useSnapshot() and change
// shared state only through send(command); every change arrives back as an event folded in by
// apply(). A gap asks the core for a fresh snapshot.

import { useSyncExternalStore } from 'react';
import { apply } from '../core/apply';
import type { CoreLink } from '../core/link';
import { emptySnapshot, type Snapshot } from '../core/model';
import type { Command, ServerMsg } from '../core/protocol';

/** A minimal observable value: the one pattern both stores use. */
export interface Store<T> {
  get(): T;
  set(next: T | ((prev: T) => T)): void;
  subscribe(fn: () => void): () => void;
}

export function createStore<T>(initial: T): Store<T> {
  let value = initial;
  const subs = new Set<() => void>();
  return {
    get: () => value,
    set(next) {
      const v = typeof next === 'function' ? (next as (prev: T) => T)(value) : next;
      if (v === value) return;
      value = v;
      for (const fn of subs) fn();
    },
    subscribe(fn) {
      subs.add(fn);
      return () => { subs.delete(fn); };
    },
  };
}

export const snapshotStore = createStore<Snapshot>(emptySnapshot());
let link: CoreLink | null = null;

/** Attach the core. Called once, from main.tsx. */
export function connect(core: CoreLink): () => void {
  link = core;
  return core.subscribe(receive);
}

export function send(cmd: Command): void {
  link?.send(cmd);
}

function receive(msg: ServerMsg): void {
  if (msg.type === 'snapshot') {
    snapshotStore.set(msg.snapshot);
    return;
  }
  const r = apply(snapshotStore.get(), msg.seq, msg.event);
  if (r.kind === 'applied') snapshotStore.set(r.snapshot);
  else if (r.kind === 'gap') send({ type: 'resync' });
}

export function useSnapshot(): Snapshot {
  return useSyncExternalStore(snapshotStore.subscribe, snapshotStore.get);
}
