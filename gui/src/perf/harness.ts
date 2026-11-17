// Perf instrumentation, part 2: scripted scenarios run inside the real window (WebKitGTK in the
// app, or any browser) against a synthetic core holding a realistic amount of data, with results
// logged through the shell (`perf_log`). Only built with VITE_PERF=1; see docs/plan/gui-perf.md.

import { invoke } from '@tauri-apps/api/core';
import { apply } from '../core/apply';
import { coreSnapshot, pendingFor } from '../core/fixtures';
import type * as W from '../core/generated';
import type { CoreLink } from '../core/link';
import { emptyPending } from '../core/pending';
import { toView } from '../core/view';
import { COORDINATOR } from '../core/wire';
import { dispatch } from '../keys/actions';
import { viewport } from '../keys/viewport';
import { connect, coreStore, snapshotStore } from '../state/store';
import { setUi } from '../state/ui';
import { renderStats } from './hook';

const log = (o: unknown) => {
  const line = JSON.stringify(o);
  console.log(line);
  void invoke('perf_log', { line }).catch(() => {});
};
const sleep = (ms: number) => new Promise<void>(r => setTimeout(r, ms));
/** Resolves just after the next frame was produced (rAF runs before paint; the message after it). */
const nextPaint = () => new Promise<void>(r => requestAnimationFrame(() => {
  const c = new MessageChannel();
  c.port1.onmessage = () => r();
  c.port2.postMessage(0);
}));

const MD = 'Here is **what I found** about the *session cookie*:\n\n- `SameSite=None; Secure` is needed on the refresh cookie\n- Safari drops cookies after **7 days** without interaction\n- see [[F3 SCA report]] and [S2, S3]\n\n```ts\nconst cookie = { sameSite: "none", secure: true };\nawait refresh(cookie);\n```\n\n| provider | fee |\n|---|---|\n| Stripe | 1.4% |\n| Adyen | 0.6% |\n\nNext I will open the PR.';

/** A snapshot with the amount of data a few hours of work leaves: long chats, many cards. */
function bigSnapshot(chat: number, cards: number): W.Snapshot {
  const s = coreSnapshot(0, 'shop');
  const block = s.blocks[0];
  if (!block) throw new Error('no block in fixtures');
  const entries: W.ConversationEntry[] = [];
  const base = Date.now() - 3 * 3600_000;
  for (let i = 0; i < chat; i++) {
    const kind = i % 4 === 0 ? 'user' : i % 7 === 3 ? 'tool_summary' : 'assistant';
    entries.push({
      id: 1000 + i, at_ms: base + i * 20_000, kind,
      text: kind === 'user' ? 'Please look at the next thing and report.' : kind === 'tool_summary' ? 'Read 3 files' : MD,
      effort: s.efforts[1 + (i % 3)]?.id ?? null,
    });
  }
  const conversations: W.Snapshot['conversations'] = { [COORDINATOR]: { entries, has_more: true } };
  for (const a of s.agents) if (a.id !== COORDINATOR) conversations[a.id] = { entries: entries.slice(0, Math.floor(chat / 2)), has_more: true };
  const extra: W.Card[] = [];
  for (let i = 0; i < cards; i++) {
    extra.push({
      id: 5000 + i, effort: block.effort, block: block.id, kind: i % 3 ? 'idea' : 'finding',
      x: 40 + (i % 8) * 250, y: 40 + Math.floor(i / 8) * 170, title: `Card ${i} with a reasonably long title`,
      text: `Card ${i}\nbody text of the card, a line or two long`, missing: false, added_by: 'user', created_ms: base, seen: true, reference: null,
    });
  }
  const links: W.CardLink[] = extra.slice(1).map((c, i) => ({ from: extra[Math.floor(i / 2)]?.id ?? c.id, to: c.id, label: "" }));
  return { ...s, conversations, cards: [...s.cards, ...extra], card_links: [...s.card_links, ...links] };
}

class SyntheticCore implements CoreLink {
  state: W.Snapshot;
  private subs = new Set<(m: W.ServerMsg) => void>();
  constructor(chat: number, cards: number) { this.state = bigSnapshot(chat, cards); }
  subscribe(fn: (m: W.ServerMsg) => void) {
    this.subs.add(fn);
    fn({ snapshot: this.state });
    return () => { this.subs.delete(fn); };
  }
  send() {}
  emit(event: W.Event) {
    const seq = this.state.seq + 1;
    const r = apply(this.state, seq, event);
    if (r.kind === 'applied') this.state = r.snapshot;
    for (const f of this.subs) f({ event: { seq, event } });
  }
}

const key = (k: string, o: KeyboardEventInit = {}) => window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...o }));

const mean = (a: number[]) => +(a.reduce((x, y) => x + y, 0) / Math.max(1, a.length)).toFixed(2);
const pct = (a: number[], p: number) => +([...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor(a.length * p))] ?? 0).toFixed(2);

/** Fire n times; time to the end of the React work of the handler, and to just after the next paint. */
async function latency(fire: () => void, n = 30) {
  const sync: number[] = [], paint: number[] = [];
  for (let i = 0; i < n; i++) {
    await nextPaint();
    const t0 = performance.now();
    fire();
    await Promise.resolve();
    sync.push(performance.now() - t0);
    await nextPaint();
    paint.push(performance.now() - t0);
    await sleep(5);
  }
  return { n, sync_ms: mean(sync), paint_ms: mean(paint), p95_paint_ms: pct(paint, 0.95) };
}

async function counted(name: string, run: () => Promise<unknown>): Promise<void> {
  renderStats.renders.clear();
  renderStats.commits = 0;
  renderStats.counting = true;
  const result = await run();
  renderStats.counting = false;
  const top = [...renderStats.renders.entries()].sort((a, b) => b[1] - a[1]).slice(0, 24);
  log({ scenario: name, result, commits: renderStats.commits, renders: Object.fromEntries(top) });
}

/** Frame rate and worst frame while `during` runs for `ms`. */
async function frames(ms: number, during: (stop: () => boolean) => Promise<void>) {
  const ts: number[] = [];
  let on = true;
  const tick = (t: number) => { ts.push(t); if (on) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  const started = performance.now();
  const end = started + ms;
  await during(() => performance.now() >= end);
  on = false;
  await nextPaint();
  const gaps = ts.slice(1).map((t, i) => t - (ts[i] ?? t));
  return { fps: +(gaps.length / ((performance.now() - started) / 1000)).toFixed(1), worst_frame_ms: +Math.max(...gaps).toFixed(1), frames: gaps.length };
}

/** Runs every scenario; the numbers go to the shell's log, one JSON line each. */
export async function runPerf(chat: number, cards: number): Promise<void> {
  const nav = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming | undefined;
  log({ startup: { dom_content_loaded_ms: nav?.domContentLoadedEventEnd, load_ms: nav?.loadEventEnd, first_contentful_paint_ms: performance.getEntriesByName('first-contentful-paint')[0]?.startTime, script_ready_ms: performance.now() } });
  const core = new SyntheticCore(chat, cards);
  connect(core, pendingFor);
  const json = JSON.stringify(core.state);
  const t0 = performance.now();
  for (let i = 0; i < 20; i++) toView(coreStore.get(), emptyPending());
  log({ snapshot: { json_kb: Math.round(json.length / 1024), chat_entries: chat, cards, to_view_ms: +((performance.now() - t0) / 20).toFixed(2) } });
  await sleep(500);
  const effort = core.state.efforts[1]?.id;
  setUi({ screen: 'ws', zone: 'tree', mode: 'normal', main: 'canvas', effort: effort === undefined ? null : String(effort) });
  await sleep(500);

  await counted('key j/k in tree', async () => { let i = 0; return latency(() => key(i++ % 2 ? 'k' : 'j')); });
  await counted('Space leader open then Esc', async () => latency(() => { key(' '); setTimeout(() => key('Escape'), 0); }));
  await counted('mode i / Esc', async () => { let i = 0; return latency(() => key(i++ % 2 ? 'Escape' : 'i')); });
  setUi({ mode: 'normal', zone: 'main' });
  await counted('view switch canvas<->viewer', async () => { let i = 0; return latency(() => dispatch(i++ % 2 ? 'openCanvas' : 'openViewer')); });
  dispatch('openCanvas');
  await sleep(300);
  await counted('typing in chat bar', async () => {
    setUi({ zone: 'chat', mode: 'insert' });
    await sleep(100);
    const ta = document.querySelector<HTMLTextAreaElement>('.chat-zone textarea');
    if (!ta) return { error: 'no textarea' };
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
    let n = 0;
    const r = await latency(() => { set?.call(ta, 'x'.repeat(++n)); ta.dispatchEvent(new Event('input', { bubbles: true })); }, 40);
    setUi({ mode: 'normal', draft: '' });
    return r;
  });

  // Chat streaming: an assistant reply grows by `rate` deltas a second for 5 s.
  for (const rate of [30, 100]) {
    await counted(`streaming ${rate} deltas/s, chat in view`, async () => {
      setUi({ zone: 'chat', mode: 'normal', main: 'canvas' });
      const entry = 90000 + rate;
      core.emit({ conversation_appended: { agent: COORDINATOR, entry: { id: entry, at_ms: Date.now(), kind: 'assistant', text: '' } } });
      const handler: number[] = [];
      const f = await frames(5000, async stop => {
        while (!stop()) {
          const t = performance.now();
          core.emit({ assistant_delta: { agent: COORDINATOR, entry, text: 'word ' } });
          handler.push(performance.now() - t);
          await sleep(1000 / rate);
        }
      });
      return { ...f, deltas: handler.length, handler_ms_mean: mean(handler), handler_ms_p95: pct(handler, 0.95) };
    });
  }

  setUi({ zone: 'main', main: 'canvas', mode: 'normal' });
  await sleep(500);
  await counted(`canvas zoom steps (${cards} cards)`, async () => {
    let i = 0;
    return latency(() => { if (i++ % 2) viewport.canvas?.zoomOut(); else viewport.canvas?.zoomIn(); }, 20);
  });
  await counted('canvas wheel zoom 3 s', async () => {
    const pane = document.querySelector('.react-flow__pane');
    if (!pane) return { error: 'no pane' };
    let i = 0;
    return frames(3000, async stop => {
      while (!stop()) {
        pane.dispatchEvent(new WheelEvent('wheel', { deltaY: i++ % 20 < 10 ? -30 : 30, ctrlKey: true, clientX: 700, clientY: 400, bubbles: true, cancelable: true }));
        await nextPaint();
      }
    });
  });
  await counted('idle 5 s', async () => { await sleep(5000); return {}; });
  // The script samples CPU and memory in each of these windows.
  // Each phase hides one more part of the page, to find what the idle CPU is spent on.
  const phases: [string, string][] = [
    ['idle', ''],
    ['idle-spinner-hidden', '.spin { display: none !important; }'],
  ];
  if (import.meta.env.VITE_PERF_BISECT) {
    phases.push(
      ['spin-contain-strict', '.status .spin { contain: strict !important; }'],
      ['spin-steps-4', '.status .spin { animation: spin 1s steps(4) infinite !important; }'],
      ['spin-opacity-blink', '@keyframes blink { 50% { opacity: .3; } } .status .spin { animation: blink 1.2s steps(1) infinite !important; }'],
      ['spin-translatez', '.status .spin { transform: translateZ(0); will-change: transform, opacity; animation: spin 1s linear infinite !important; }'],
      ['spin-opacity-only-composited', '.status .spin { will-change: opacity; animation: blink 1.2s linear infinite !important; }'],
    );
  }
  const style = document.createElement('style');
  document.head.append(style);
  for (const [name, css] of phases) {
    style.textContent = css;
    log({ phase: name, busy: snapshotStore.get().status.busy });
    await sleep(8000);
  }
  log({ done: true });
}
