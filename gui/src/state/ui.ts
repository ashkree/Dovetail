// UI-only state: screen, zone in focus, mode, pending keys, selections, open view, modal, sidebar.
// Never part of the Snapshot (plan §3). Some of it round-trips through the URL hash (urlHash.ts).

import { useSyncExternalStore } from 'react';
import type { Id } from '../core/model';
import { createStore } from './store';

export type Screen = 'home' | 'ws';
export type Zone = 'chat' | 'main' | 'inbox' | 'tree';
export type MainView = 'canvas' | 'viewer' | 'reading' | 'item';
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
  | { type: 'form'; title: string; fields: [label: string, value: string][] };

export interface Overlay { scope: string; layer: OverlayLayer; leader: string[] }

export interface Ui {
  screen: Screen;
  homeSel: number;
  zone: Zone;
  /** Which sidebar part a move into the sidebar lands on. */
  side: 'inbox' | 'tree';
  main: MainView | null;
  mainItem: Id | null;
  mode: Mode;
  /** Keys typed so far of a multi-key sequence (`g` of `gg`). */
  pending: string;
  /** The Space-leader path typed so far, or null when the leader is not active. */
  leader: string[] | null;
  /** The effort in focus. */
  effort: Id | null;
  /** Chat filter: every effort, or one effort's thread. */
  filter: 'all' | Id;
  /** The "where was I" pop-up is up. */
  wwi: boolean;
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
  canvasPart: 'cards' | 'incoming';
  incSel: number;
  zoom: number;
  slot: number;
  /** The live variant mix in the viewer, option index per slot. */
  choice: number[];
  panel: PanelState;
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
  zone: 'chat', side: 'inbox', main: null, mainItem: null,
  mode: 'normal', pending: '', leader: null,
  effort: null, filter: 'all', wwi: false,
  inboxSel: 0, treeSel: 0, folded: { search: true, coupon: true },
  chatCur: -1, openReports: [],
  draft: '', tag: null,
  cardSel: 'Q3', canvasPart: 'cards', incSel: 0, zoom: 0.85,
  slot: 2, choice: [0, 0, 0, 0, 0], panel: 'docked', panelPos: { x: 0, y: 0 },
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
