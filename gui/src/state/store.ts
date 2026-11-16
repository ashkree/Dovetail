// Holds the client's copy of the core's Snapshot, the pending state (what the core does not
// serve yet), and the one CoreLink. Components read the view made of both with useSnapshot(),
// and change shared state only through send(command): the command goes to the core, and the
// change comes back as an event folded in by apply(). A gap asks the core for a fresh snapshot.

import { useSyncExternalStore } from 'react';
import { apply } from '../core/apply';
import type * as W from '../core/generated';
import type { CoreLink } from '../core/link';
import type { Snapshot } from '../core/model';
import { cardViewId } from '../core/canvasView';
import { applyPending, emptyPending, type Pending } from '../core/pending';
import type { UiCommand } from '../core/protocol';
import { translate } from '../core/translate';
import { toView } from '../core/view';
import { emptyCoreSnapshot, variant } from '../core/wire';
import { createStore, type Store } from './observable';
import { focusEffort, setUi, toast, uiStore } from './ui';

export { createStore, type Store };

/** The core's Snapshot as last folded. */
export const coreStore = createStore<W.Snapshot>(emptyCoreSnapshot());
/** What the GUI shows that the core does not serve yet, for the open project. */
export const pendingStore = createStore<Pending>(emptyPending());

/** The view the components read: made from the two stores, again only when one changed. */
export const snapshotStore = derived();

let link: CoreLink | null = null;
let seed: (project: string | null) => Pending = () => emptyPending();
/** Whether a snapshot has arrived: events before the first one are dropped. */
let started = false;

/** Attach the core; `pendingFor` gives each project's pending state (empty for the real core). */
export function connect(core: CoreLink, pendingFor?: (project: string | null) => Pending): () => void {
  link = core;
  if (pendingFor) seed = pendingFor;
  started = false;
  return core.subscribe(receive);
}

export function send(cmd: UiCommand): void {
  const t = translate(cmd, coreStore.get());
  if (t.refused) toast(t.refused);
  if (t.pending) pendingStore.set(p => applyPending(p, t.pending!));
  for (const c of t.core) link?.send(c);
}

function receive(msg: W.ServerMsg): void {
  const [tag, data] = variant(msg);
  if (tag === 'snapshot') {
    const next = data as W.Snapshot;
    const before = coreStore.get().project?.id ?? null;
    const after = next.project?.id ?? null;
    if (!started || before !== after) pendingStore.set(seed(after));
    started = true;
    setUi({ awaitEffort: false });
    coreStore.set(next);
    if (before !== after) focusOpenedProject();
    return;
  }
  if (tag !== 'event' || !started) return;
  const { seq, event } = data as { seq: number; event: W.Event };
  const [kind, payload] = variant(event);
  if (kind === 'command_failed') {
    toast((payload as { message: string }).message);
    // A refused create_effort must not leave a later effort to take focus unasked.
    setUi({ awaitEffort: false });
  }
  const before = coreStore.get();
  const r = apply(before, seq, event);
  if (r.kind === 'applied') {
    coreStore.set(r.snapshot);
    cardEvent(before, kind, payload);
    effortEvent(before, kind, payload);
  }
  else if (r.kind === 'gap') link?.send('resync');
}

/** What an event about a card means for the UI: a body to keep, a new card to select (ADR 0025). */
function cardEvent(before: W.Snapshot, kind: string, payload: unknown): void {
  if (kind === 'card_loaded') {
    const { id, body } = payload as { id: number; body: string };
    const card = coreStore.get().cards.find(c => c.id === id);
    if (!card) return;
    const viewId = cardViewId(card);
    pendingStore.set(p => applyPending(p, { type: 'card_body', card_id: viewId, body }));
    if (uiStore.get().awaitCard === viewId) {
      // The reading view shows a body long enough to be a document; a short one is a toast.
      const report = snapshotStore.get().cards.find(c => c.id === viewId)?.report_id;
      setUi(report ? { awaitCard: null, main: 'reading', zone: 'main', readHead: 0 } : { awaitCard: null });
      if (!report) toast(`**${viewId}** · ${card.title} — ${body}`);
    }
  } else if (kind === 'card_upserted' && uiStore.get().awaitNewCard) {
    const card = payload as W.Card;
    if (!before.cards.some(c => c.id === card.id)) {
      setUi({ awaitNewCard: false, cardSel: cardViewId(card) });
    }
  }
}

/** A new effort the user just created takes focus, as switching to it does. */
function effortEvent(before: W.Snapshot, kind: string, payload: unknown): void {
  if (kind !== 'effort_upserted') return;
  const effort = payload as W.Effort;
  if (!uiStore.get().awaitEffort || before.efforts.some(e => e.id === effort.id)) return;
  setUi({ awaitEffort: false });
  focusEffort(String(effort.id));
}

/** A project's snapshot just arrived: focus an effort of it, as opening it from Home does. */
function focusOpenedProject(): void {
  const s = snapshotStore.get();
  const u = uiStore.get();
  if (s.open_project === null) return;
  if (s.efforts.some(e => e.id === u.effort)) return;
  const effort = s.inbox[0]?.effort_id ?? s.efforts[0]?.id ?? null;
  setUi({ effort, tag: effort, wwi: effort !== null && u.screen === 'ws' });
}

function derived(): Store<Snapshot> {
  let core: W.Snapshot | null = null;
  let pending: Pending | null = null;
  let view: Snapshot | null = null;
  const get = (): Snapshot => {
    const c = coreStore.get();
    const p = pendingStore.get();
    if (view === null || c !== core || p !== pending) {
      core = c;
      pending = p;
      view = toView(c, p);
    }
    return view;
  };
  return {
    get,
    set() { throw new Error('the view is derived: send a command instead'); },
    subscribe(fn) {
      const a = coreStore.subscribe(fn);
      const b = pendingStore.subscribe(fn);
      return () => { a(); b(); };
    },
  };
}

export function useSnapshot(): Snapshot {
  return useSyncExternalStore(snapshotStore.subscribe, snapshotStore.get);
}
