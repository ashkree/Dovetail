// Every action the GUI can take, and the one dispatcher that runs them. Keys (through the keymap),
// leader leaves, `:` commands and mouse handlers all call dispatch(); nothing is mouse-only.
// Actions change UI state with setUi() and shared state only by send(command).

import { hasOwnText } from '../core/canvasView';
import { COORDINATOR, UNSORTED_EFFORT } from '../core/wire';
import type { Agent, CardKind, EffortKind, Framework, Id, InboxItem, Preview, Snapshot, VariantSet } from '../core/model';
import type { InboxAnswer } from '../core/protocol';
import { send, snapshotStore } from '../state/store';
import { effortById, INBOX_GLYPH, KIND_GLYPH, readingReport, toKebab, treeRows, visibleChat } from '../state/select';
import { focusEffort, scopeOf, setUi, toast, uiStore, type MainView, type PickItem, type Ui, type Zone } from '../state/ui';
import { readingHeadings, viewport } from './viewport';

export type Action =
  | 'none'
  // everywhere
  | 'zoneLeft' | 'zoneRight' | 'zoneDown' | 'zoneUp' | 'insert' | 'command' | 'keys' | 'escape'
  | 'nextEffort' | 'prevEffort' | 'search'
  // lists (home, inbox, tree) and scrolling views
  | 'down' | 'up' | 'top' | 'bottom'
  // home
  | 'openProject' | 'newProject' | 'addRepo' | 'removeRepo'
  // inbox and inbox item
  | 'openItem' | 'allow' | 'allowAlways' | 'deny' | 'denyReason' | 'reply' | 'dismiss' | 'itemEffort' | 'openAgent'
  // efforts tree
  | 'fold' | 'openRow' | 'wwi' | 'addTask' | 'rename' | 'moveDown' | 'moveUp' | 'archive'
  // chat
  | 'scrollDown' | 'scrollUp' | 'nextTurn' | 'prevTurn' | 'unfold' | 'foldEntry' | 'cycleFilter' | 'pickFilter'
  // canvas and incoming strip
  | 'cardLeft' | 'cardDown' | 'cardUp' | 'cardRight' | 'follow' | 'followBack' | 'openCard' | 'refCard' | 'newCard'
  | 'moveCardL' | 'moveCardD' | 'moveCardU' | 'moveCardR' | 'toIncoming' | 'deleteCard' | 'zoomIn' | 'zoomOut' | 'zoomFit'
  | 'incLeft' | 'incRight' | 'accept' | 'drop' | 'toCanvas'
  // viewer
  | 'slotDown' | 'slotUp' | 'optNext' | 'optPrev' | 'opt1' | 'opt2' | 'opt3' | 'keepCombo' | 'resetCombo'
  | 'togglePanel' | 'detachPanel' | 'closePanel' | 'reload' | 'browser'
  // reading view
  | 'nextHeading' | 'prevHeading' | 'refReport'
  // insert (chat bar)
  | 'send' | 'newline' | 'cycleTag' | 'leaveBar' | 'normal' | 'deleteWord' | 'deleteLine' | 'paste'
  // Space leader
  | 'picker' | 'focusChat' | 'focusInbox' | 'focusTree' | 'toggleSide' | 'pickEffort' | 'newEffort' | 'criteria'
  | 'openCanvas' | 'openViewer' | 'openReading' | 'closeMain' | 'pickProject' | 'home' | 'pickAgent'
  | 'interrupt' | 'permMode'
  // mouse (the same dispatcher; each has a key elsewhere)
  | 'showView' | 'selectHome' | 'selectInbox' | 'selectTree' | 'selectCard' | 'selectIncoming' | 'selectSlot'
  | 'pickOption' | 'focusZone' | 'toggleReport' | 'refCardId' | 'switchEffort' | 'pickFramework'
  | 'framework1' | 'framework2' | 'framework3';

/** Actions whose key the browser should still handle (paste into the chat bar). */
export const PASS_THROUGH: ReadonlySet<Action> = new Set<Action>(['paste']);

/** The kinds the user makes by typing; the others point at something research or the coordinator filed. */
const CARD_KINDS: CardKind[] = ['idea', 'question'];
const EFFORT_KINDS: EffortKind[] = ['feature', 'fix', 'cleanup', 'research', 'experiment'];
const MOVE_STEP = 40;

const snap = (): Snapshot => snapshotStore.get();
const ui = (): Ui => uiStore.get();
const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n));

/** What the viewer's choice of slice depends on: the focused effort and the slice the user picked. */
type ViewerFocus = Pick<Ui, 'effort' | 'viewerAgent'>;

/** The slice the viewer shows: the one the user picked in the tree while it is still a slice;
 * else, in the focused effort, the one with variants or a preview already, else its first slice
 * with a worktree. With no effort focused, any effort's. */
export function viewerAgent(s: Snapshot, u: ViewerFocus): Agent | undefined {
  const picked = s.agents.find(a => a.id === u.viewerAgent && a.kind === 'slice');
  if (picked) return picked;
  const taskEffort = new Map(s.tasks.map(t => [t.id, t.effort_id]));
  const slices = s.agents.filter(a =>
    a.kind === 'slice' && (u.effort === null || taskEffort.get(a.task_id) === u.effort));
  return slices.find(a => s.variants.some(v => v.agent_id === a.id))
    ?? slices.find(a => s.previews.some(p => p.agent_id === a.id && p.state !== 'not_started'))
    ?? slices.find(a => a.branch)
    ?? slices[0];
}

/** The variant set of the slice the viewer shows. */
export function viewerSet(s: Snapshot, u: ViewerFocus): VariantSet | undefined {
  const agent = viewerAgent(s, u);
  return agent && s.variants.find(v => v.agent_id === agent.id);
}

/** The preview state of the slice the viewer shows; `not_started` until the core says more. */
export function viewerPreview(s: Snapshot, u: ViewerFocus): Preview | undefined {
  const agent = viewerAgent(s, u);
  const effort = agent ? s.tasks.find(t => t.id === agent.task_id)?.effort_id : undefined;
  return agent && (s.previews.find(p => p.agent_id === agent.id)
    ?? { agent_id: agent.id, effort_id: effort ?? u.effort ?? '', state: 'not_started' });
}

/** The canvas the main view shows: the focused effort's first canvas. */
export function effortCanvas(s: Snapshot, effort: Id | null) {
  return s.canvases.find(c => c.effort_id === effort);
}

/** The agent whose permission mode `Shift-Tab` and `Space s m` change: the one under the tree
 * cursor while the tree has focus, else the coordinator. A subagent shares its parent's mode. */
export function modeTarget(s: Snapshot, u: Ui): { id: Id; name: string; mode: string } {
  const row = u.zone === 'tree' && u.mode === 'normal' ? treeRows(s, u.folded)[u.treeSel] : undefined;
  if (row?.type === 'agent') {
    const a = row.agent.parent_id ? s.agents.find(x => x.id === row.agent.parent_id) ?? row.agent : row.agent;
    return { id: a.id, name: a.name, mode: a.mode };
  }
  return { id: COORDINATOR, name: 'coordinator', mode: s.status.permission_mode };
}

/** The slot the viewer's keys act on: the remembered one, kept inside this set's slots (another
 * slice's set may have fewer). */
export function slotIndex(set: VariantSet | undefined, slot: number): number {
  return set ? clamp(slot, 0, Math.max(0, set.slots.length - 1)) : slot;
}

/** The user picks option `j` of slot `i`: the core keeps it and the viewer posts it to the page. */
function pick(set: VariantSet, i: number, j: number): void {
  const slot = set.slots[i];
  const option = slot?.options[j];
  if (slot && option !== undefined) send({ type: 'select_variant', agent_id: set.agent_id, slot: slot.name, option });
}

/** The frameworks an empty canvas offers, in their key order (`1 2 3`). */
export const FRAMEWORK_KEY = { framework1: 'question_tree', framework2: 'options_compare', framework3: 'free_board' } as const satisfies Record<string, Framework>;

function switchEffort(id: Id, quiet = false): void {
  const e = effortById(snap(), id);
  if (!e) return;
  focusEffort(id);
  if (!quiet) toast(`Effort: **${e.name}** · next message is tagged with it`);
}

function setZone(z: Zone | 'side'): void {
  setUi(u => {
    const zone = z === 'side' ? u.side : z;
    return zone === 'inbox' || zone === 'tree' ? { zone, side: zone, sideCollapsed: false } : { zone };
  });
}

function openMain(view: MainView): void {
  setUi(view === 'reading' ? { main: view, zone: 'main', readHead: 0 } : { main: view, zone: 'main' });
}

function prompt(title: string, onDone: (v: string) => void, value = ''): void {
  setUi({ modal: { type: 'prompt', title, value, onDone } });
}

function picker(title: string, items: PickItem[], q = ''): void {
  setUi({ modal: { type: 'picker', title, items, sel: 0, q } });
}

function insertRef(text: string): void {
  setUi(u => ({ draft: (u.draft ? u.draft.trimEnd() + ' ' : '') + text + ' ', mode: 'insert', zone: 'chat' }));
}

function cardsOnCanvas(s: Snapshot, u: Ui) {
  const canvas = effortCanvas(s, u.effort);
  return canvas ? s.cards.filter(c => c.canvas_id === canvas.id) : [];
}

function incomingOnCanvas(s: Snapshot, u: Ui) {
  const canvas = effortCanvas(s, u.effort);
  return canvas ? s.incoming.filter(c => c.canvas_id === canvas.id) : [];
}

/** The nearest card in a direction: along the direction, with sideways distance counted double. */
export function nearestCard<T extends { id: Id; x: number; y: number }>(cards: T[], fromId: Id | null, dx: number, dy: number): T | undefined {
  const c = cards.find(x => x.id === fromId);
  if (!c) return cards[0];
  let best: T | undefined;
  let bestD = Infinity;
  for (const o of cards) {
    if (o === c) continue;
    const vx = o.x - c.x, vy = o.y - c.y;
    const along = vx * dx + vy * dy;
    if (along <= 0) continue;
    const across = Math.abs(vx * dy - vy * dx);
    const d = along + across * 2;
    if (d < bestD) { bestD = d; best = o; }
  }
  return best;
}

/** The inbox item an answer applies to: the one open in the main view, else the selected row. */
function itemInFocus(s: Snapshot, u: Ui): InboxItem | undefined {
  return u.zone === 'main' && u.main === 'item' ? s.inbox.find(x => x.id === u.mainItem) : s.inbox[u.inboxSel];
}

function answer(item: InboxItem, a: InboxAnswer, reason?: string): void {
  send({ type: 'answer_inbox', id: item.id, answer: a, ...(reason !== undefined ? { reason } : {}) });
  setUi(u => {
    const n = snap().inbox.length;
    const patch: Partial<Ui> = { inboxSel: clamp(u.inboxSel, 0, Math.max(0, n - 1)) };
    if (u.main === 'item' && u.mainItem === item.id) { patch.main = null; patch.mainItem = null; patch.zone = u.side; }
    return patch;
  });
}

function answerItem(a: Action): void {
  const it = itemInFocus(snap(), ui());
  if (!it) return;
  const criterion = it.kind === 'criterion';
  switch (a) {
    case 'allow':
      if (criterion) { answer(it, 'approve'); toast(`Approved · criterion ticked in **${effortById(snap(), it.effort_id)?.name ?? ''}**`); }
      else if (it.kind === 'permission') { answer(it, 'allow'); toast('Allowed once'); }
      else toast('Not a permission request or a criterion');
      return;
    case 'allowAlways':
      if (it.kind !== 'permission') { toast('Not a permission request'); return; }
      answer(it, 'allow_always'); toast('Always allowed for this project');
      return;
    case 'deny':
    case 'denyReason': {
      if (it.kind !== 'permission' && !criterion) { toast('Not a permission request or a criterion'); return; }
      const label = criterion ? 'Sent back' : 'Denied';
      const ans: InboxAnswer = criterion ? 'send_back' : 'deny';
      if (a === 'denyReason') prompt(`${label} with reason, to ${it.agent_id}:`, v => { answer(it, ans, v); toast(`${label}: “${v}”`); });
      else { answer(it, ans); toast(label); }
      return;
    }
    case 'reply':
      if (it.kind !== 'question') { toast('Only questions take a reply'); return; }
      prompt(`Reply to ${it.agent_id}:`, v => { answer(it, 'reply', v); toast('Reply sent'); });
      return;
    case 'dismiss':
      answer(it, 'dismiss'); toast('Dismissed');
      return;
  }
}

function moveCard(dx: number, dy: number): void {
  const c = snap().cards.find(x => x.id === ui().cardSel);
  if (c) send({ type: 'move_card', card_id: c.id, x: c.x + dx, y: c.y + dy });
}

function selectChatCursor(n: number): void {
  setUi({ chatCur: n });
}

export function runCommand(line: string): void {
  const v = line.trim();
  const s = snap();
  if (v === 'q' || v === 'home') dispatch('home');
  else if (v.startsWith('effort ')) {
    const want = toKebab(v.slice(7));
    const e = s.efforts.find(x => x.name.startsWith(want));
    if (e) switchEffort(e.id); else toast(`No effort named **${want}**`);
  } else if (v === 'canvas') dispatch('openCanvas');
  else if (v === 'viewer') dispatch('openViewer');
  else if (v === 'reading') dispatch('openReading');
  else if (v === 'inbox') dispatch('focusInbox');
  else if (v === 'tree') dispatch('focusTree');
  else if (v === 'keys') dispatch('keys');
  else if (v) toast(`:${v} — commands: q, home, effort <name>, canvas, viewer, reading, inbox, tree, keys`);
}

export function dispatch(a: Action, arg?: unknown): void {
  const s = snap();
  const u = ui();
  const sc = scopeOf(u);
  switch (a) {
    case 'none': return;
    case 'paste': return;

    // ---- screens and projects
    case 'home': setUi({ screen: 'home', main: null, mode: 'normal', wwi: false }); return;
    case 'selectHome': setUi({ homeSel: Number(arg) }); return;
    case 'openProject': {
      const i = arg != null ? Number(arg) : u.homeSel;
      const p = s.projects[i];
      if (!p) return;
      send({ type: 'open_project', project_id: p.id });
      const after = snap();
      const keep = effortById(after, u.effort);
      const effort = keep?.id ?? after.inbox[0]?.effort_id ?? after.efforts[0]?.id ?? null;
      setUi({ homeSel: i, screen: 'ws', zone: 'chat', mode: 'normal', effort, tag: effort, wwi: effort !== null });
      return;
    }
    case 'newProject':
      setUi({ modal: { type: 'form', title: 'New project', sel: 0, fields: [
        { label: 'Name', value: '', hint: 'Shop' },
        { label: 'Claude account', value: '', hint: 'a profile from config.toml; empty for the default' },
        { label: 'First repo', value: '', hint: '~/code/shop-api — a folder in a git repo; empty for research only' },
      ], onDone: ([name = '', account = '', repo = '']) => {
        setUi({ awaitProject: true });
        send({ type: 'create_project', name, account, repo_path: repo });
      } } });
      return;
    case 'addRepo': {
      // Home: the selected project; a workspace: the open one.
      const p = u.screen === 'home' ? s.projects[u.homeSel] : s.projects.find(x => x.id === s.open_project);
      if (!p) return;
      setUi({ modal: { type: 'form', title: `Add a repo to ${p.name}`, sel: 0, fields: [
        { label: 'Folder', value: '', hint: '~/code/shop-web — a folder in a git repo' },
        { label: 'Name', value: '', hint: "what agents call it; empty for the folder's name" },
      ], onDone: ([path = '', name = '']) => {
        send({ type: 'add_repo', project_id: p.id, path, name });
      } } });
      return;
    }
    case 'removeRepo': {
      const p = s.projects[u.homeSel];
      if (!p) return;
      if (!p.repos.length) { toast(`**${p.name}** has no repo`); return; }
      const confirm = (repo: string) => picker(`Remove ${repo} from ${p.name}? Its files stay where they are.`, [
        { key: 'keep', text: 'keep', label: 'Keep it', go: () => {} },
        { key: 'remove', text: 'remove', label: `Remove ${repo}`, go: () => send({ type: 'remove_repo', project_id: p.id, repo }) },
      ]);
      picker(`Remove a repo from ${p.name}`, p.repos.map(r => ({ key: r, text: r, label: r, glyph: '▣', glyphClass: 'dim', go: () => confirm(r) })));
      return;
    }
    case 'pickProject':
      picker('Switch project', s.projects.map((p, i) => ({
        key: p.id, text: p.name, label: p.name, detail: `${p.repos.join(', ') || 'research'} · ${p.account}`, go: () => dispatch('openProject', i),
      })));
      return;

    // ---- zones and modes
    case 'zoneLeft': case 'zoneRight': {
      const zones: (Zone | 'side')[] = u.main ? ['chat', 'main', 'side'] : ['chat', 'side'];
      const cur = u.zone === 'inbox' || u.zone === 'tree' ? 'side' : u.zone;
      const i = clamp(zones.indexOf(cur) + (a === 'zoneLeft' ? -1 : 1), 0, zones.length - 1);
      const to = zones[i] ?? 'chat';
      if (to === 'side' && u.sideCollapsed) setUi({ sideCollapsed: false });
      setUi({ mode: 'normal' });
      setZone(to);
      return;
    }
    case 'zoneDown': case 'zoneUp':
      if (u.zone === 'inbox' || u.zone === 'tree') setZone(a === 'zoneDown' ? 'tree' : 'inbox');
      else if (u.zone === 'main' && u.main === 'canvas') setUi({ canvasPart: a === 'zoneDown' ? 'incoming' : 'cards' });
      else if (u.zone === 'chat' && a === 'zoneDown') setUi({ mode: 'insert' });
      return;
    case 'focusZone': setZone(arg as Zone); setUi({ mode: 'normal' }); return;
    case 'focusChat': setUi({ zone: 'chat' }); return;
    case 'focusInbox': setZone('inbox'); return;
    case 'focusTree': setZone('tree'); return;
    case 'toggleSide':
      setUi(x => ({ sideCollapsed: !x.sideCollapsed, zone: !x.sideCollapsed && (x.zone === 'inbox' || x.zone === 'tree') ? 'chat' : x.zone }));
      return;
    case 'insert': setUi({ mode: 'insert', zone: 'chat' }); return;
    case 'normal': setUi({ mode: 'normal' }); return;
    case 'leaveBar': setUi({ mode: 'normal', zone: 'chat' }); return;
    case 'escape':
      if (u.wwi) setUi({ wwi: false });
      else if (u.zone === 'main' && u.main === 'item') setUi({ main: null, mainItem: null, zone: 'inbox' });
      else if (u.zone === 'main' && u.main === 'reading' && effortCanvas(s, u.effort)) setUi({ main: 'canvas' });
      else if (u.zone === 'chat') setUi({ chatCur: -1 });
      return;
    case 'command': setUi({ modal: { type: 'command', value: '' } }); return;
    case 'search': toast('Search in this view (not in this build)'); return;
    case 'keys': setUi(x => ({ overlay: x.overlay ? null : { scope: scopeOf(x), layer: 'plain', leader: [] } })); return;

    // ---- efforts
    case 'nextEffort': case 'prevEffort': {
      const n = s.efforts.length;
      if (!n) return;
      const i = s.efforts.findIndex(e => e.id === u.effort);
      const next = s.efforts[(i + (a === 'nextEffort' ? 1 : n - 1) + n) % n];
      if (next) switchEffort(next.id);
      return;
    }
    case 'switchEffort': switchEffort(String(arg)); return;
    case 'pickEffort':
      picker('Switch effort', s.efforts.map(e => ({
        key: e.id, text: `${e.name} ${e.kind}`, label: e.name, effort: e.id, detail: e.question_or_goal, go: () => switchEffort(e.id),
      })));
      return;
    case 'newEffort':
      prompt('New effort · name (saved in lower kebab case)', name => {
        const kebab = toKebab(name);
        if (!kebab) return;
        picker(`New effort ${kebab} · kind`, EFFORT_KINDS.map(k => ({
          key: k, text: k, label: k, glyph: KIND_GLYPH[k],
          go: () => prompt(`New effort ${kebab} · question or goal`, goal => {
            // The core answers with the effort; it takes focus then (store.ts, effortEvent).
            setUi({ awaitEffort: true });
            send({ type: 'new_effort', name: kebab, kind: k, question_or_goal: goal });
            toast(`New effort saved as **${kebab}**`);
          }),
        })));
      });
      return;
    case 'wwi': case 'criteria': setUi(x => ({ wwi: true, wwiFlash: x.wwiFlash + 1 })); return;

    // ---- lists
    case 'down': case 'up': {
      const d = a === 'down' ? 1 : -1;
      if (sc === 'home') setUi({ homeSel: clamp(u.homeSel + d, 0, s.projects.length - 1) });
      else if (sc === 'inbox') setUi({ inboxSel: clamp(u.inboxSel + d, 0, Math.max(0, s.inbox.length - 1)) });
      else if (sc === 'tree') setUi({ treeSel: clamp(u.treeSel + d, 0, treeRows(s, u.folded).length - 1) });
      return;
    }
    case 'top':
      if (sc === 'home') setUi({ homeSel: 0 });
      else if (sc === 'chat') selectChatCursor(0);
      else if (sc === 'reading') { setUi({ readHead: 0 }); viewport.reading?.scrollTo({ top: 0 }); }
      return;
    case 'bottom':
      if (sc === 'home') setUi({ homeSel: s.projects.length - 1 });
      else if (sc === 'chat') selectChatCursor(-1);
      else if (sc === 'reading') viewport.reading?.scrollTo({ top: viewport.reading.scrollHeight });
      return;

    // ---- inbox
    case 'selectInbox': setUi({ inboxSel: Number(arg) }); setZone('inbox'); return;
    case 'openItem': {
      const it = s.inbox[u.inboxSel];
      if (!it) return;
      if (it.kind === 'finished') {
        switchEffort(it.effort_id, true);
        setUi({ wwi: false });
        openMain('viewer');
        answer(it, 'dismiss');
        toast('Finished turn opened in the viewer; the item is cleared');
      } else setUi({ main: 'item', mainItem: it.id, zone: 'main' });
      return;
    }
    case 'allow': case 'allowAlways': case 'deny': case 'denyReason': case 'reply': case 'dismiss':
      answerItem(a);
      return;
    case 'itemEffort': { const it = s.inbox[u.inboxSel]; if (it) switchEffort(it.effort_id); return; }
    case 'openAgent': {
      const it = s.inbox.find(x => x.id === u.mainItem);
      if (it?.kind === 'finished') openMain('viewer');
      else toast(`Opens **${it?.agent_id ?? 'the agent'}**'s conversation (not in this build)`);
      return;
    }

    // ---- efforts tree
    case 'selectTree': setUi({ treeSel: Number(arg) }); setZone('tree'); return;
    case 'fold': {
      const r = treeRows(s, u.folded)[u.treeSel];
      if (!r) return;
      const folded = { ...u.folded, [r.effort.id]: !u.folded[r.effort.id] };
      setUi({ folded, treeSel: treeRows(s, folded).findIndex(x => x.type === 'effort' && x.effort.id === r.effort.id) });
      return;
    }
    case 'openRow': {
      const r = treeRows(s, u.folded)[u.treeSel];
      if (!r) return;
      if (r.type === 'effort') { switchEffort(r.effort.id); setUi(x => ({ folded: { ...x.folded, [r.effort.id]: false } })); }
      else if (r.type === 'task') toast(`Task “${r.task.title}” · its status is set by Dovetail`);
      else if (r.agent.kind === 'slice') { setUi({ viewerAgent: r.agent.id }); openMain('viewer'); }
      else toast(`Opens **${r.agent.name}**'s conversation (not in this build)`);
      return;
    }
    case 'addTask': {
      const r = treeRows(s, u.folded)[u.treeSel];
      const e = r?.effort ?? effortById(s, u.effort);
      if (e) prompt(`New task in ${e.name}:`, v => send({ type: 'add_task', effort_id: e.id, title: v.trim() }));
      return;
    }
    case 'rename': {
      const r = treeRows(s, u.folded)[u.treeSel];
      if (!r || r.type !== 'effort') { toast('Rename works on an effort row'); return; }
      prompt(`Rename ${r.effort.name} (saved in lower kebab case):`, v => {
        const name = toKebab(v);
        if (name) { send({ type: 'rename_effort', effort_id: r.effort.id, name }); toast(`Renamed to **${name}**`); }
      }, r.effort.name);
      return;
    }
    case 'moveDown': case 'moveUp': toast('Reorder (not in this build)'); return;
    case 'archive': toast('Archive, asks first (not in this build)'); return;

    // ---- chat
    case 'scrollDown': case 'scrollUp':
      if (sc === 'chat') {
        const n = visibleChat(s, u.filter).length;
        let c = u.chatCur < 0 ? n - 1 : u.chatCur;
        c += a === 'scrollDown' ? 1 : -1;
        selectChatCursor(c >= n ? -1 : Math.max(0, c));
      } else if (sc === 'reading') viewport.reading?.scrollBy({ top: a === 'scrollDown' ? 60 : -60 });
      return;
    case 'nextTurn': case 'prevTurn':
      if (sc === 'reading') { dispatch(a === 'nextTurn' ? 'nextHeading' : 'prevHeading'); return; }
      {
        const v = visibleChat(s, u.filter);
        const d = a === 'nextTurn' ? 1 : -1;
        let c = u.chatCur < 0 ? v.length : u.chatCur;
        for (c += d; c >= 0 && c < v.length; c += d) if (v[c]?.who === 'you') break;
        selectChatCursor(c >= v.length ? -1 : Math.max(0, c));
      }
      return;
    case 'unfold': case 'foldEntry': {
      const m = visibleChat(s, u.filter)[u.chatCur];
      if (m?.report) setUi(x => ({ openReports: a === 'unfold' ? [...x.openReports.filter(i => i !== m.id), m.id] : x.openReports.filter(i => i !== m.id) }));
      return;
    }
    case 'toggleReport': {
      const id = String(arg);
      setUi(x => ({ openReports: x.openReports.includes(id) ? x.openReports.filter(i => i !== id) : [...x.openReports, id] }));
      return;
    }
    case 'cycleFilter': setUi(x => ({ filter: x.filter === 'all' ? (x.effort ?? 'all') : 'all', chatCur: -1 })); return;
    case 'pickFilter':
      picker('Show in the chat', [
        { key: 'all', text: 'All efforts', label: 'All efforts', go: () => setUi({ filter: 'all', chatCur: -1 }) },
        ...s.efforts.map(e => ({ key: e.id, text: e.name, label: e.name, effort: e.id, go: () => { setUi({ filter: e.id, chatCur: -1 }); switchEffort(e.id, true); setUi({ wwi: false }); } })),
      ]);
      return;

    // ---- insert (chat bar)
    case 'send': {
      const text = u.draft.trim();
      // With no effort in focus (a new project has none) the message goes to Unsorted.
      const tag = u.tag ?? u.effort ?? String(UNSORTED_EFFORT);
      if (!text) return;
      send({ type: 'send_message', text, effort_id: tag });
      setUi({ draft: '', wwi: false, chatCur: -1 });
      return;
    }
    case 'newline': setUi(x => ({ draft: x.draft + '\n' })); return;
    case 'deleteWord': setUi(x => ({ draft: x.draft.replace(/\S+\s*$/, '') })); return;
    case 'deleteLine': setUi(x => ({ draft: x.draft.replace(/[^\n]*$/, '') })); return;
    case 'cycleTag': {
      const n = s.efforts.length;
      if (!n) return;
      const i = s.efforts.findIndex(e => e.id === u.tag);
      setUi({ tag: s.efforts[(i + 1) % n]?.id ?? null });
      return;
    }

    // ---- views
    case 'showView': {
      const v = arg as MainView | null;
      if (v) openMain(v); else dispatch('closeMain');
      return;
    }
    case 'openCanvas': openMain('canvas'); return;
    case 'openViewer': openMain('viewer'); return;
    case 'openReading': openMain('reading'); return;
    case 'closeMain': setUi(x => ({ main: null, mainItem: null, zone: x.zone === 'main' ? 'chat' : x.zone })); return;

    // ---- canvas
    case 'cardLeft': case 'cardRight': case 'cardUp': case 'cardDown': {
      const [dx, dy] = { cardLeft: [-1, 0], cardRight: [1, 0], cardUp: [0, -1], cardDown: [0, 1] }[a];
      const next = nearestCard(cardsOnCanvas(s, u), u.cardSel, dx ?? 0, dy ?? 0);
      if (next) setUi({ cardSel: next.id });
      return;
    }
    case 'selectCard': setUi({ cardSel: String(arg), canvasPart: 'cards', zone: 'main' }); return;
    case 'follow': case 'followBack': {
      // Tab: to the first card this one links to; Shift-Tab: back to the card that links to it.
      const l = s.card_links.find(x => a === 'follow' ? x.from === u.cardSel : x.to === u.cardSel);
      if (l) setUi({ cardSel: a === 'follow' ? l.to : l.from });
      return;
    }
    case 'openCard': {
      const c = s.cards.find(x => x.id === u.cardSel);
      if (c?.report_id) openMain('reading');
      else if (c && hasOwnText(c.kind)) toast(`**${c.id}** · ${c.title}${c.body ? ` — ${c.body}` : ''}`);
      else if (c) { setUi({ awaitCard: c.id }); send({ type: 'load_card', card_id: c.id }); }
      return;
    }
    case 'refCard': {
      const id = sc === 'incoming' ? incomingOnCanvas(s, u)[u.incSel]?.id : u.cardSel;
      dispatch('refCardId', id);
      return;
    }
    case 'refCardId': {
      const c = s.cards.find(x => x.id === arg) ?? s.incoming.find(x => x.id === arg);
      if (!c) return;
      insertRef(`[[${c.id} ${c.title}]]`);
      toast(`Reference to **${c.id}** put in the chat bar`);
      return;
    }
    case 'newCard': {
      const canvas = effortCanvas(s, u.effort);
      if (!canvas) return;
      const from = s.cards.find(x => x.id === u.cardSel);
      picker('New card · kind', CARD_KINDS.map(k => ({
        key: k, text: k, label: k,
        go: () => prompt(`New ${k} card · title`, title => {
          setUi({ awaitNewCard: true });
          send({
            type: 'add_card', canvas_id: canvas.id, kind: k, x: (from?.x ?? 40) + 30, y: (from?.y ?? 20) + 150,
            title: title.trim(), body: '', ...(from ? { link_from: from.id } : {}),
          });
        }),
      })));
      return;
    }
    case 'moveCardL': moveCard(-MOVE_STEP, 0); return;
    case 'moveCardR': moveCard(MOVE_STEP, 0); return;
    case 'moveCardU': moveCard(0, -MOVE_STEP); return;
    case 'moveCardD': moveCard(0, MOVE_STEP); return;
    case 'deleteCard': {
      const id = u.cardSel;
      if (!id) return;
      const rest = cardsOnCanvas(s, u).filter(c => c.id !== id);
      send({ type: 'delete_card', card_id: id });
      setUi({ cardSel: nearestCard(rest, rest[0]?.id ?? null, 0, 0)?.id ?? rest[0]?.id ?? null });
      toast(`Deleted **${id}**`);
      return;
    }
    case 'zoomIn': viewport.canvas?.zoomIn(); return;
    case 'zoomOut': viewport.canvas?.zoomOut(); return;
    case 'zoomFit': viewport.canvas?.fit(); return;
    case 'toIncoming': if (incomingOnCanvas(s, u).length) setUi({ canvasPart: 'incoming', incSel: 0 }); return;
    case 'toCanvas': setUi({ canvasPart: 'cards' }); return;
    case 'selectIncoming': setUi({ incSel: Number(arg), canvasPart: 'incoming', zone: 'main' }); return;
    case 'incLeft': setUi({ incSel: Math.max(0, u.incSel - 1) }); return;
    case 'incRight': setUi({ incSel: clamp(u.incSel + 1, 0, Math.max(0, incomingOnCanvas(s, u).length - 1)) }); return;
    case 'accept': {
      const c = incomingOnCanvas(s, u)[u.incSel];
      if (!c) return;
      const p = s.cards.find(x => x.id === c.under);
      const kids = s.card_links.filter(l => l.from === c.under).length;
      send({ type: 'place_card', incoming_id: c.id, x: (p?.x ?? 40) + 30 + kids * 18, y: (p?.y ?? 20) + 150 + kids * 26 });
      const left = incomingOnCanvas(snap(), u).length;
      setUi({ cardSel: c.id, incSel: clamp(u.incSel, 0, Math.max(0, left - 1)), canvasPart: left ? 'incoming' : 'cards' });
      toast(c.under ? `Placed **${c.id}** under ${c.under}` : `Placed **${c.id}**`);
      return;
    }
    case 'drop': {
      const c = incomingOnCanvas(s, u)[u.incSel];
      if (!c) return;
      send({ type: 'drop_incoming', incoming_id: c.id });
      const left = incomingOnCanvas(snap(), u).length;
      setUi({ incSel: clamp(u.incSel - 1, 0, Math.max(0, left - 1)), canvasPart: left ? 'incoming' : 'cards' });
      toast(`Dropped **${c.id}**`);
      return;
    }
    // The empty canvas offers its frameworks as 1 2 3; with a canvas the keys do nothing.
    case 'framework1': case 'framework2': case 'framework3': {
      if (effortCanvas(s, u.effort)) return;
      dispatch('pickFramework', FRAMEWORK_KEY[a]);
      return;
    }
    case 'pickFramework': {
      if (u.effort) send({ type: 'new_canvas', effort_id: u.effort, framework: (arg as Framework) ?? 'question_tree' });
      return;
    }

    // ---- viewer
    case 'slotDown': case 'slotUp': {
      const set = viewerSet(s, u);
      if (set) setUi({ slot: clamp(slotIndex(set, u.slot) + (a === 'slotDown' ? 1 : -1), 0, set.slots.length - 1) });
      return;
    }
    case 'selectSlot': setUi({ slot: Number(arg), zone: 'main' }); return;
    case 'optNext': case 'optPrev': case 'opt1': case 'opt2': case 'opt3': {
      const set = viewerSet(s, u);
      const i = slotIndex(set, u.slot);
      const slot = set?.slots[i];
      if (!set || !slot) return;
      const n = slot.options.length;
      const cur = set.choice[i] ?? 0;
      const j = a === 'optNext' ? (cur + 1) % n : a === 'optPrev' ? (cur + n - 1) % n : Number(a.slice(3)) - 1;
      if (j >= n) return;
      pick(set, i, j);
      return;
    }
    case 'pickOption': {
      const [slot, j] = arg as [number, number];
      const set = viewerSet(s, u);
      setUi({ slot, zone: 'main' });
      if (set) pick(set, slot, j);
      return;
    }
    case 'keepCombo': {
      const set = viewerSet(s, u);
      if (!set) return;
      send({ type: 'keep_variants', agent_id: set.agent_id });
      toast(`Combination kept and sent to **${set.agent_id}**`);
      return;
    }
    case 'resetCombo': {
      const set = viewerSet(s, u);
      set?.slots.forEach((_, i) => { if ((set.choice[i] ?? 0) !== (set.kept[i] ?? 0)) pick(set, i, set.kept[i] ?? 0); });
      return;
    }
    case 'togglePanel': setUi(x => ({ panel: x.panel === 'closed' ? 'docked' : 'closed' })); return;
    case 'closePanel': setUi({ panel: 'closed' }); return;
    case 'detachPanel':
      if (u.main !== 'viewer') openMain('viewer');
      setUi(x => x.panel === 'detached' ? { panel: 'docked' } : { panel: 'detached', panelPos: { x: Math.max(16, window.innerWidth - 820), y: 120 } });
      return;
    case 'reload': {
      const p = viewerPreview(s, u);
      if (p && (p.state === 'failed' || p.state === 'no_command')) {
        send({ type: 'restart_preview', agent_id: p.agent_id });
        toast('Preview restarting');
        return;
      }
      setUi(x => ({ previewLoads: x.previewLoads + 1 }));
      toast('Preview reloaded');
      return;
    }
    case 'browser': {
      const url = viewerPreview(s, u)?.url;
      if (url) send({ type: 'open_external', url });
      else toast('The preview is not running');
      return;
    }

    // ---- reading view
    case 'nextHeading': case 'prevHeading': {
      const n = readingHeadings().length;
      setUi({ readHead: clamp(u.readHead + (a === 'nextHeading' ? 1 : -1), 0, Math.max(0, n - 1)) });
      return;
    }
    case 'refReport': {
      const report = readingReport(s, u.cardSel, u.effort);
      const card = s.cards.find(c => c.report_id === report?.id);
      if (card) { insertRef(`[[${card.id} ${card.title}]]`); toast(`Reference to **${card.id}** put in the chat bar`); }
      return;
    }

    // ---- leader leaves with no view of their own
    case 'picker':
      picker('Everything · efforts, agents, inbox, cards', [
        ...s.efforts.map(e => ({ key: 'e' + e.id, text: 'effort ' + e.name, label: e.name, effort: e.id, detail: 'effort', go: () => switchEffort(e.id) })),
        ...s.agents.map(ag => ({
          key: 'a' + ag.id, text: 'agent ' + ag.name, label: ag.name, glyph: ag.kind === 'slice' ? '◆' : '◇', glyphClass: 'agent-g',
          detail: [ag.branch, effortById(s, s.tasks.find(t => t.id === ag.task_id)?.effort_id)?.name].filter(Boolean).join(' · '),
          go: () => toast(`Opens **${ag.name}** (not in this build)`),
        })),
        ...s.inbox.map((it, i) => ({
          key: 'i' + it.id, text: 'inbox ' + it.title, label: it.title, glyph: INBOX_GLYPH[it.kind], glyphClass: 'g-' + it.kind,
          go: () => { setUi({ inboxSel: i }); setZone('inbox'); },
        })),
        ...s.cards.map(c => ({
          key: 'c' + c.id, text: `card ${c.id} ${c.title}`, label: c.title, glyph: c.id, glyphClass: 'dim', detail: c.kind,
          go: () => {
            const canvas = s.canvases.find(x => x.id === c.canvas_id);
            if (canvas) switchEffort(canvas.effort_id, true);
            setUi({ wwi: false, cardSel: c.id, canvasPart: 'cards' });
            openMain('canvas');
          },
        })),
      ]);
      return;
    case 'pickAgent': dispatch('picker'); setUi(x => x.modal?.type === 'picker' ? { modal: { ...x.modal, q: 'agent ' } } : {}); return;
    case 'interrupt': toast('Interrupt the agent in focus (not in this build)'); return;
    // The coordinator's session moves to its next mode (auto → acceptEdits → manual); the
    // bottom bar shows the new one when the core reports it.
    case 'permMode': send({ type: 'cycle_permission_mode', agent_id: modeTarget(s, u).id }); return;
  }
}
