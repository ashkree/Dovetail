// A minimal observable value: the one pattern every store uses.

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
