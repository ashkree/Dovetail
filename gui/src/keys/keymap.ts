// THE keymap: the only place a key is named (plan §4). Ported from mock/gui/keymap.ts.
// The dispatcher runs keys from it AND the which-key pop-up and the `?` keyboard overlay are drawn
// from it, so what you see is exactly what the keys do.
//
// STATUS: a proposal the user has not reviewed (docs/gui-layout-and-keys.md "Keys"). Keep it pure
// data: changing a key is a one-line edit here and nowhere else.
//
// Each binding: [keys, action, label, cat]. cat colours the key in the overlay:
//   move = movement, act = act on the thing in focus, view = open or switch a view,
//   mode = change mode. Scope bindings win over global ones.
// Key tokens: a printable character, `C-x` (Ctrl), `<CR>` `<A-CR>` `<Esc>` `<Tab>` `<S-Tab>` `<Space>`.
// A sequence is tokens written one after another (`gg`, `]e`, `dd`).

import type { Action } from './actions';

export type Cat = 'move' | 'act' | 'view' | 'mode' | '';
export type Binding = [keys: string, action: Action, label: string, cat: Cat];
export type LeaderLeaf = [action: Action, label: string];
export interface LeaderGroup { name: string; keys: Record<string, LeaderLeaf> }
export type LeaderNode = LeaderLeaf | LeaderGroup;

export const SCOPES: Record<string, { name: string; hint: string }> = {
  home: { name: 'Home', hint: 'project list' },
  inbox: { name: 'Inbox', hint: 'sidebar, top' },
  tree: { name: 'Efforts tree', hint: 'sidebar, bottom' },
  chat: { name: 'Chat', hint: 'coordinator, left' },
  canvas: { name: 'Canvas', hint: 'main view' },
  incoming: { name: 'Incoming strip', hint: 'under the canvas' },
  viewer: { name: 'Viewer', hint: 'main view' },
  reading: { name: 'Reading view', hint: 'main view' },
  item: { name: 'Inbox item', hint: 'main view' },
  insert: { name: 'Insert (chat bar)', hint: 'typing' },
};

export const KEYMAP: Record<string, Binding[]> = {
  global: [
    ['h', 'zoneLeft', 'zone ←', 'move'],
    ['l', 'zoneRight', 'zone →', 'move'],
    ['C-h', 'zoneLeft', 'zone ← (always)', 'move'],
    ['C-l', 'zoneRight', 'zone → (always)', 'move'],
    ['C-j', 'zoneDown', 'zone ↓', 'move'],
    ['C-k', 'zoneUp', 'zone ↑', 'move'],
    ['i', 'insert', 'chat bar', 'mode'],
    [':', 'command', 'command', 'mode'],
    ['?', 'keys', 'keyboard map', 'view'],
    ['<Esc>', 'escape', 'back / close', 'mode'],
    [']e', 'nextEffort', 'next effort', 'move'],
    ['[e', 'prevEffort', 'prev effort', 'move'],
    ['/', 'search', 'search', 'act'],
  ],
  home: [
    ['j', 'down', 'next project', 'move'],
    ['k', 'up', 'prev project', 'move'],
    ['gg', 'top', 'first', 'move'],
    ['G', 'bottom', 'last', 'move'],
    ['<CR>', 'openProject', 'open project', 'view'],
    ['n', 'newProject', 'new project', 'act'],
    ['r', 'addRepo', 'add repo', 'act'],
    ['h', 'none', '', ''],
    ['l', 'none', '', ''],
  ],
  inbox: [
    ['j', 'down', 'next item', 'move'],
    ['k', 'up', 'prev item', 'move'],
    ['<CR>', 'openItem', 'open in full', 'view'],
    ['y', 'allow', 'allow / approve', 'act'],
    ['A', 'allowAlways', 'always allow', 'act'],
    ['d', 'deny', 'deny / send back', 'act'],
    ['D', 'denyReason', 'deny with reason', 'act'],
    ['r', 'reply', 'reply', 'act'],
    ['x', 'dismiss', 'dismiss', 'act'],
    ['e', 'itemEffort', "item's effort", 'view'],
  ],
  tree: [
    ['j', 'down', 'next row', 'move'],
    ['k', 'up', 'prev row', 'move'],
    ['o', 'fold', 'fold / unfold', 'act'],
    ['<CR>', 'openRow', 'open (a slice: its preview)', 'view'],
    ['w', 'wwi', 'where was I', 'view'],
    ['a', 'addTask', 'add task', 'act'],
    ['e', 'rename', 'rename', 'act'],
    ['J', 'moveDown', 'move down', 'act'],
    ['K', 'moveUp', 'move up', 'act'],
    ['dd', 'archive', 'archive', 'act'],
    ['<S-Tab>', 'permMode', "agent's permission mode", 'mode'],
  ],
  chat: [
    ['j', 'scrollDown', 'scroll ↓', 'move'],
    ['k', 'scrollUp', 'scroll ↑', 'move'],
    ['gg', 'top', 'top', 'move'],
    ['G', 'bottom', 'bottom', 'move'],
    [']]', 'nextTurn', 'next turn', 'move'],
    ['[[', 'prevTurn', 'prev turn', 'move'],
    ['zo', 'unfold', 'open report', 'act'],
    ['zc', 'foldEntry', 'fold', 'act'],
    ['f', 'cycleFilter', 'filter: All ⇄ effort', 'view'],
    ['<S-Tab>', 'permMode', "coordinator's permission mode", 'mode'],
    ['C-j', 'insert', 'chat bar', 'mode'],
  ],
  canvas: [
    ['h', 'cardLeft', 'card ←', 'move'],
    ['j', 'cardDown', 'card ↓', 'move'],
    ['k', 'cardUp', 'card ↑', 'move'],
    ['l', 'cardRight', 'card →', 'move'],
    ['<Tab>', 'follow', 'follow link', 'move'],
    ['<S-Tab>', 'followBack', 'link back', 'move'],
    ['<CR>', 'openCard', 'open card', 'view'],
    ['r', 'refCard', 'ref → chat', 'act'],
    ['n', 'newCard', 'new card', 'act'],
    ['H', 'moveCardL', 'move card ←', 'act'],
    ['J', 'moveCardD', 'move card ↓', 'act'],
    ['K', 'moveCardU', 'move card ↑', 'act'],
    ['L', 'moveCardR', 'move card →', 'act'],
    ['gi', 'toIncoming', 'go to incoming', 'move'],
    ['dd', 'deleteCard', 'delete card', 'act'],
    ['+', 'zoomIn', 'zoom in', 'view'],
    ['-', 'zoomOut', 'zoom out', 'view'],
    ['0', 'zoomFit', 'fit', 'view'],
    ['1', 'framework1', 'no canvas yet: question tree', 'act'],
    ['2', 'framework2', 'no canvas yet: options compare', 'act'],
    ['3', 'framework3', 'no canvas yet: free board', 'act'],
  ],
  incoming: [
    ['h', 'incLeft', 'prev card', 'move'],
    ['l', 'incRight', 'next card', 'move'],
    ['a', 'accept', 'place on canvas', 'act'],
    ['x', 'drop', 'drop', 'act'],
    ['r', 'refCard', 'ref → chat', 'act'],
    ['k', 'toCanvas', 'back to canvas', 'move'],
    ['<Esc>', 'toCanvas', 'back to canvas', 'move'],
  ],
  viewer: [
    ['j', 'slotDown', 'next slot', 'move'],
    ['k', 'slotUp', 'prev slot', 'move'],
    ['<Tab>', 'optNext', 'next variant', 'act'],
    ['<S-Tab>', 'optPrev', 'prev variant', 'act'],
    ['1', 'opt1', 'variant 1', 'act'],
    ['2', 'opt2', 'variant 2', 'act'],
    ['3', 'opt3', 'variant 3', 'act'],
    ['y', 'keepCombo', 'keep this combination', 'act'],
    ['u', 'resetCombo', 'back to kept', 'act'],
    ['p', 'togglePanel', 'variant panel', 'view'],
    ['D', 'detachPanel', 'detach / dock panel', 'view'],
    ['q', 'closePanel', 'close panel', 'view'],
    ['R', 'reload', 'reload', 'act'],
    ['gx', 'browser', 'open in browser', 'view'],
  ],
  reading: [
    ['j', 'scrollDown', 'scroll ↓', 'move'],
    ['k', 'scrollUp', 'scroll ↑', 'move'],
    ['gg', 'top', 'top', 'move'],
    ['G', 'bottom', 'bottom', 'move'],
    [']]', 'nextHeading', 'next heading', 'move'],
    ['[[', 'prevHeading', 'prev heading', 'move'],
    ['r', 'refReport', 'ref → chat', 'act'],
    ['gx', 'browser', 'open source', 'view'],
  ],
  item: [
    ['y', 'allow', 'allow / approve', 'act'],
    ['A', 'allowAlways', 'always allow', 'act'],
    ['d', 'deny', 'deny / send back', 'act'],
    ['D', 'denyReason', 'deny with reason', 'act'],
    ['r', 'reply', 'reply', 'act'],
    ['x', 'dismiss', 'dismiss', 'act'],
    ['<CR>', 'openAgent', 'open the agent', 'view'],
  ],
  insert: [
    ['<CR>', 'send', 'send', 'act'],
    ['<A-CR>', 'newline', 'new line', 'act'],
    ['C-t', 'cycleTag', 'change effort tag', 'act'],
    ['<S-Tab>', 'permMode', "coordinator's permission mode", 'mode'],
    ['C-k', 'leaveBar', 'leave bar (keep draft)', 'mode'],
    ['<Esc>', 'normal', 'Normal mode', 'mode'],
    ['C-w', 'deleteWord', 'delete word', 'act'],
    ['C-u', 'deleteLine', 'delete line', 'act'],
    ['C-v', 'paste', 'paste / image', 'act'],
  ],
};

// Space leader: mnemonic groups. A group is {name, keys}; a leaf is [action, label].
export const LEADER: Record<string, LeaderNode> = {
  '<Space>': ['picker', 'picker: everything'],
  c: ['focusChat', 'chat (coordinator)'],
  n: ['focusInbox', 'inbox'],
  t: ['focusTree', 'efforts tree'],
  z: ['toggleSide', 'sidebar: collapse / expand'],
  e: { name: 'effort', keys: {
    e: ['pickEffort', 'switch effort'],
    n: ['newEffort', 'new effort'],
    w: ['wwi', 'where was I'],
    f: ['cycleFilter', 'chat filter: All ⇄ this'],
    c: ['criteria', 'acceptance criteria'],
  } },
  v: { name: 'view', keys: {
    c: ['openCanvas', 'canvas'],
    w: ['openViewer', 'web viewer'],
    r: ['openReading', 'reading view'],
    x: ['closeMain', 'close view (chat wide)'],
    d: ['detachPanel', 'detach variant panel'],
  } },
  p: { name: 'project', keys: {
    p: ['pickProject', 'switch project'],
    h: ['home', 'home (all projects)'],
    r: ['addRepo', 'add repo'],
  } },
  s: { name: 'session', keys: {
    s: ['pickAgent', 'pick agent'],
    x: ['interrupt', 'interrupt'],
    m: ['permMode', 'permission mode (tree: its agent)'],
  } },
};

/** Global keys Home does not take (it has no zones or efforts). */
export const HOME_SKIPS_GLOBAL = ['h', 'l', 'C-h', 'C-l', 'C-j', 'C-k', 'i', ']e', '[e'];

/** Keys inside the `?` keyboard overlay while it is open. */
export const OVERLAY_KEYS = {
  close: ['<Esc>', '?'],
  nextScope: [']'],
  prevScope: ['['],
  nextLayer: ['<Tab>'],
  prevLayer: ['<S-Tab>'],
};

/** Keys inside the picker, prompts and the `:` command line while one is open. */
export const MODAL_KEYS = {
  close: ['<Esc>'],
  accept: ['<CR>'],
  down: ['C-j', '<Down>'],
  up: ['C-k', '<Up>'],
  /** A form's next and previous field (as well as down and up). */
  nextField: ['<Tab>'],
  prevField: ['<S-Tab>'],
};
