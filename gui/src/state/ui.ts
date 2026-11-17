// UI-only state: screen, zone in focus, mode, pending keys, selections, open view, modal, sidebar.
// Never part of the Snapshot (plan §3). Some of it round-trips through the URL hash (urlHash.ts).

import { useSyncExternalStore } from 'react';
import type { Id } from '../core/model';
import { createStore } from './observable';

export type Screen = 'home' | 'ws';
export type Zone = 'chat' | 'main' | 'inbox' | 'tree';
export type MainView = 'canvas' | 'viewer' | 'reading' | 'item' | 'agent';
export type Mode = 'normal' | 'insert';
export type PanelState = 'docked' | 'detached' | 'closed';
export type OverlayLayer = 'plain' | 'shift' | 'ctrl' | 'leader';

/** One row of the picker: plain data, drawn by the Picker component. */
export interface PickItem {
  key: string;
  /** What the filter matches against. */
  text: string;
  label: string;
  /** Dim text after the label. */
  detail?: string;
  /** Show this effort's coloured tag instead of the label. */
  effort?: Id;
  glyph?: string;
  glyphClass?: string;
  go: () => void;
}

export type Modal =
  | { type: 'picker'; title: string; items: PickItem[]; sel: number; q: string }
  | { type: 'prompt'; title: string; value: string; onDone: (value: string) => void }
  | { type: 'command'; value: string }
  | { type: 'form'; title: string; fields: FormField[]; sel: number; onDone: (values: string[]) => void };

/** One field of a form modal. The first field must be filled for the form to be sent. */
export interface FormField {
  label: string;
  value: string;
  /** Dim text in the empty field. */
  hint?: string;
}

export interface Overlay { scope: string; layer: OverlayLayer; leader: string[] }

export interface Ui {
  screen: Screen;
  homeSel: number;
  zone: Zone;
  /** Which sidebar part a move into the sidebar lands on. */
  side: 'inbox' | 'tree';
  main: MainView | null;
  mainItem: Id | null;
  /** The slice or subagent whose conversation the `agent` view shows. */
  agent: Id | null;
  /** What is typed in the agent view's own bar. */
  agentDraft: string;
  /** Cursor in the agent's conversation; -1 = following the bottom. */
  agentCur: number;
  mode: Mode;
  /** Keys typed so far of a multi-key sequence (`g` of `gg`). */
  pending: string;
  /** The Space-leader path typed so far, or null when the leader is not active. */
  leader: string[] | null;
  /** The effort in focus. */
  effort: Id | null;
  /** The slice the user opened in the viewer from the tree; null: the focused effort's slice. */
  viewerAgent: Id | null;
  /** Chat filter: every effort, or one effort's thread. */
  filter: 'all' | Id;
  /** The "where was I" pop-up is up. */
  wwi: boolean;
  /** Counts "bring it back" requests; the card flashes when it is above 0. */
  wwiFlash: number;
  inboxSel: number;
  treeSel: number;
  folded: Record<Id, boolean>;
  /** Cursor in the visible chat entries; -1 = following the bottom. */
  chatCur: number;
  openReports: Id[];
  draft: string;
  /** The effort the next message is tagged with. */
  tag: Id | null;
  cardSel: Id | null;
  /** A card whose body was asked for with `o`: the reading view opens when it arrives. */
  awaitCard: Id | null;
  /** The user just asked for a new card: the next one the core adds is selected. */
  awaitNewCard: boolean;
  /** The user just created an effort: the next one the core adds takes focus. */
  awaitEffort: boolean;
  /** The user just created a project on Home: the next one the core adds is selected. */
  awaitProject: boolean;
  canvasPart: 'cards' | 'incoming';
  incSel: number;
  zoom: number;
  slot: number;
  panel: PanelState;
  /** Bumped to reload the viewer's preview. */
  previewLoads: number;
  panelPos: { x: number; y: number };
  readHead: number;
  sideW: number;
  sideCollapsed: boolean;
  overlay: Overlay | null;
  modal: Modal | null;
  /** Plain text; `**x**` is shown bold. */
  toast: { text: string; n: number } | null;
}

export const initialUi: Ui = {
  screen: 'home', homeSel: 0,
  zone: 'chat', side: 'inbox', main: null, mainItem: null, agent: null, agentDraft: '', agentCur: -1,
  mode: 'normal', pending: '', leader: null,
  effort: null, viewerAgent: null, filter: 'all', wwi: false, wwiFlash: 0,
  inboxSel: 0, treeSel: 0, folded: { search: true, coupon: true },
  chatCur: -1, openReports: [],
  draft: '', tag: null,
  cardSel: 'Q3', awaitCard: null, awaitNewCard: false, awaitEffort: false, awaitProject: false, canvasPart: 'cards', incSel: 0, zoom: 0.85,
  slot: 2, panel: 'docked', previewLoads: 0, panelPos: { x: 0, y: 0 },
  readHead: 0, sideW: 330, sideCollapsed: false,
  overlay: null, modal: null, toast: null,
};

export const uiStore = createStore<Ui>(initialUi);

export function useUi(): Ui {
  return useSyncExternalStore(uiStore.subscribe, uiStore.get);
}

/** Merge a change into the UI state. */
export function setUi(patch: Partial<Ui> | ((ui: Ui) => Partial<Ui>)): void {
  uiStore.set(ui => ({ ...ui, ...(typeof patch === 'function' ? patch(ui) : patch) }));
}

let toastN = 0;
export function toast(text: string): void {
  setUi({ toast: { text, n: ++toastN } });
}

/** The keymap scope the keys resolve in now: the zone in focus, the open view, or Insert. */
export function scopeOf(ui: Ui): string {
  if (ui.mode === 'insert') return 'insert';
  if (ui.screen === 'home') return 'home';
  if (ui.zone === 'main' && ui.main) {
    if (ui.main === 'canvas') return ui.canvasPart === 'incoming' ? 'incoming' : 'canvas';
    return ui.main;
  }
  return ui.zone === 'main' ? 'chat' : ui.zone;
}

/** The agent the bar types to in Insert mode: the agent view's while it has focus; null for the
 * coordinator's chat bar. */
export function barAgent(u: Ui): Id | null {
  return u.zone === 'main' && u.main === 'agent' ? u.agent : null;
}

/** Focus effort `id`: the next message is tagged with it and its "where was I" comes up. */
export function focusEffort(id: Id): void {
  // The viewer goes back to following the effort: a slice picked in another effort is let go.
  setUi(u => ({ effort: id, tag: id, wwi: true, wwiFlash: 0, filter: u.filter === 'all' ? 'all' : id, viewerAgent: null }));
}
