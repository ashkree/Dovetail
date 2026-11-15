// Design variants for iterating on the mock itself (not part of Dovetail's keymap or UI).
// Each open design question is a slot with 2–4 options at one of three levels. Pick live,
// mix and match, then "Keep this combination" copies the choice for the chat; the chosen
// options become permanent and the rest are deleted.
//
// F2 or the "◐ design" button in the status bar shows the panel. Choices are kept per browser.

interface DesignOption { id: string; label: string }
interface DesignSlot { level: 'Global' | 'Component' | 'Content'; id: string; name: string; options: DesignOption[] }

const DESIGN: DesignSlot[] = [
  { level: 'Global', id: 'theme', name: 'Theme', options: [
    { id: 'iris', label: 'Iris · deep purple, gold' },
    { id: 'iceberg', label: 'Iceberg · blue-grey, pastels' },
    { id: 'iris-hc', label: 'Iris high contrast' },
  ] },
  { level: 'Global', id: 'fs', name: 'Text size', options: [
    { id: '16', label: '16 px' },
    { id: '17', label: '17 px' },
    { id: '18', label: '18 px' },
  ] },
  { level: 'Component', id: 'wwi', name: '“Where was I” card', options: [
    { id: 'chat', label: 'Bottom of the chat, above the input' },
    { id: 'main', label: 'Banner on top of the main view' },
    { id: 'popup', label: 'Pop-up in the middle' },
    { id: 'tree', label: 'Under the effort in the tree' },
  ] },
  { level: 'Component', id: 'chatw', name: 'Chat width (view open)', options: [
    { id: '440', label: '440 px' },
    { id: '360', label: '360 px' },
    { id: '520', label: '520 px' },
  ] },
  { level: 'Component', id: 'inbox', name: 'Inbox height', options: [
    { id: 'fixed', label: 'Fixed, 38 % of the sidebar' },
    { id: 'fit', label: 'Fits its items (max 40 %)' },
  ] },
  { level: 'Content', id: 'limit', name: 'Running-efforts count', options: [
    { id: 'a', label: 'efforts 5 / 4 ▲' },
    { id: 'b', label: '5 running · limit 4' },
    { id: 'c', label: '▲ 5 of 4 efforts running' },
  ] },
];

const DS: { choice: Record<string, string>; open: boolean; pos: { x: number; y: number } | null } = { choice: {}, open: true, pos: null };
try { Object.assign(DS, JSON.parse(localStorage.getItem('dovetail-design') || '{}')); } catch { /* no storage */ }
function saveDesign(): void { try { localStorage.setItem('dovetail-design', JSON.stringify(DS)); } catch { /* no storage */ } }

function D(id: string): string {
  const slot = DESIGN.find(s => s.id === id);
  if (!slot) return '';
  const v = DS.choice[id];
  return slot.options.some(o => o.id === v) ? v : slot.options[0].id;
}
function applyDesign(): void {
  for (const s of DESIGN) document.documentElement.dataset[s.id] = D(s.id);
}
function designCombo(): string {
  return 'Keep: ' + DESIGN.map(s => `${s.name} = ${s.options.find(o => o.id === D(s.id))!.label}`).join(' · ');
}
function renderDesignPanel(): string {
  if (!DS.open || !DESIGN.length) return '';
  let last = '';
  const slots = DESIGN.map(s => {
    const lvl = s.level !== last ? `<div class="lvl">${s.level}</div>` : ''; last = s.level;
    return lvl + `<div class="slot"><div class="sn">${s.name}</div><div class="opts">${s.options.map((o, j) =>
      `<button class="opt ${D(s.id) === o.id ? 'on' : ''}" data-a="designPick" data-s="${s.id}" data-o="${o.id}"><span class="n">${j + 1}</span>${o.label}</button>`).join('')}</div></div>`;
  }).join('');
  const pos = DS.pos ? `left:${DS.pos.x}px;top:${DS.pos.y}px` : 'left:16px;bottom:44px';
  return `<div class="vpanel detached design" id="designpanel" style="${pos}">
    <div class="vh" data-dragdesign>◐ Design variants<span class="spacer"></span><button data-a="designClose" title="F2">✕</button></div>
    ${slots}
    <button class="keep" data-a="designKeep">Keep this combination</button></div>`;
}
