// Dovetail GUI mock: state, rendering and key dispatch. Plain JS, no build.

const $ = (s: string): any => document.querySelector(s);
const esc = (s: unknown): string => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const effortById = (id: string): Effort => EFFORTS.find(e => e.id === id);

const S: any = {
  screen: 'home', homeSel: 0,
  zone: 'chat', side: 'inbox', main: null, mainItem: null,
  mode: 'normal', pending: '', leader: null,
  effort: 'login', filter: 'all', wwi: true,
  inbox: INBOX.slice(), inboxSel: 0,
  treeSel: 0, folded: { search: true, coupon: true },
  chat: CHAT.slice(), chatCur: -1, openReports: new Set(),
  draft: '', tag: 'login',
  cards: CARDS.map(c => ({ ...c })), edges: EDGES.slice(), incoming: INCOMING.slice(),
  cardSel: 'Q3', canvasPart: 'cards', incSel: 0, zoom: 0.85,
  slot: 2, choice: [0, 0, 0, 0, 0], kept: [0, 0, 0, 0, 0], panel: 'docked', panelPos: { x: 0, y: 0 },
  readHead: 0, sideW: 330, sideCollapsed: false,
  overlay: false, ovScope: 'chat', ovLayer: 'plain', ovLeader: [],
  modal: null,
};

/* ---------------- markdown (just enough) ---------------- */
function inline(s: string): string {
  s = esc(s);
  s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>');
  s = s.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  s = s.replace(/\[\[([A-Z]\d+)([^\]]*)\]\]/g, '<span class="ref">↗ $1$2</span>');
  s = s.replace(/\[([FS]\d+(?:, [FS]\d+)*)\]/g, '<span class="cite">[$1]</span>');
  s = s.replace(/&lt;b&gt;(.*?)&lt;\/b&gt;/g, '<b>$1</b>');
  return s;
}
function md(text: string): string {
  const out = []; let list = null; let code = null; let para = [];
  const flush = () => { if (para.length) { out.push('<p>' + inline(para.join(' ')) + '</p>'); para = []; } if (list) { out.push(`</${list}>`); list = null; } };
  for (const line of text.split('\n')) {
    if (code !== null) { if (line.startsWith('```')) { out.push('<pre><code>' + esc(code.join('\n')) + '</code></pre>'); code = null; } else code.push(line); continue; }
    if (line.startsWith('```')) { flush(); code = []; continue; }
    let m;
    if ((m = line.match(/^(#{1,3}) (.*)/))) { flush(); out.push(`<h${m[1].length} data-h>${inline(m[2])}</h${m[1].length}>`); continue; }
    if ((m = line.match(/^(-|\d+\.) (.*)/))) { if (para.length) { out.push('<p>' + inline(para.join(' ')) + '</p>'); para = []; } const t = m[1] === '-' ? 'ul' : 'ol'; if (list !== t) { if (list) out.push(`</${list}>`); out.push(`<${t}>`); list = t; } out.push('<li>' + inline(m[2]) + '</li>'); continue; }
    if (!line.trim()) { flush(); continue; }
    if (list) { out.push(`</${list}>`); list = null; }
    para.push(line);
  }
  flush();
  return out.join('');
}

/* ---------------- helpers ---------------- */
function tagDot(id: string): string {
  const e = effortById(id);
  return e ? `<span class="tagdot" title="${esc(e.name)}"><span class="dot" style="background:${e.hue}"></span><span class="tagname">${esc(e.name)}</span></span>` : '';
}
function tagHtml(id, small) {
  const e = effortById(id); if (!e) return '';
  return `<span class="tag" style="color:${e.hue}">${KINDS[e.kind].glyph} ${esc(e.name)}</span>`;
}
function visibleChat() { return S.chat.map((m, i) => ({ m, i })).filter(({ m }) => S.filter === 'all' || m.effort === S.filter); }
function scopeNow() {
  if (S.mode === 'insert') return 'insert';
  if (S.screen === 'home') return 'home';
  if (S.zone === 'main') {
    if (S.main === 'canvas') return S.canvasPart === 'incoming' ? 'incoming' : 'canvas';
    return S.main;
  }
  return S.zone;
}
function toast(msg) { const t = $('#toast'); t.innerHTML = msg; t.hidden = false; clearTimeout(toast.h); toast.h = setTimeout(() => t.hidden = true, 2600); }
function running() { return EFFORTS.filter(e => e.running).length; }
function treeRows() {
  const rows = [];
  for (const e of EFFORTS) {
    rows.push({ type: 'effort', e });
    if (S.folded[e.id]) continue;
    for (const t of e.tasks) {
      rows.push({ type: 'task', e, t });
      for (const a of t.agents) {
        rows.push({ type: 'agent', e, a });
        for (const s of a.subs) rows.push({ type: 'sub', e, a: s });
      }
    }
  }
  return rows;
}

/* ---------------- render ---------------- */
function render() {
  const app = $('#app');
  applyDesign();
  const pop = S.screen === 'ws' && S.wwi && D('wwi') === 'popup' ? `<div class="wwi-pop">${wwiCard(effortById(S.effort))}</div>` : '';
  app.innerHTML = (S.screen === 'home' ? renderHomeScreen() : renderWorkspace()) + pop + renderDesignPanel();
  renderOverlay(); renderWhichKey(); renderModal();
  afterRender();
}

function limitText(n: number, over: boolean): string {
  const v = D('limit');
  if (v === 'b') return `${n} running · limit ${MACHINE_LIMIT}`;
  if (v === 'c') return `${over ? '▲ ' : ''}${n} of ${MACHINE_LIMIT} efforts running`;
  return `efforts ${n} / ${MACHINE_LIMIT}${over ? ' ▲' : ''}`;
}

function header(inWs) {
  return `<div class="header">
    <span class="brand">◆ Dovetail</span>
    ${inWs ? `<span class="crumb"><button data-a="home" class="dim">Projects</button><span class="sep">/</span>Shop</span>
      <span class="chip repo">shop-api</span><span class="chip repo">shop-web</span>
      <nav class="viewtabs">${[[null, 'Chat', 'Space v x'], ['canvas', 'Canvas', 'Space v c'], ['viewer', 'Viewer', 'Space v w'], ['reading', 'Reading', 'Space v r']].concat(S.main === 'item' ? [['item', 'Inbox item', 'Enter in the inbox']] : [])
        .map(([k, n, key]) => `<button class="vtab ${S.main === k ? 'on' : ''}" ${k ? `data-a="main" data-m="${k}"` : 'data-a="closeMain"'} title="${key}">${n}</button>`).join('')}</nav>` : `<span class="crumb">Projects</span>`}
    <span class="spacer"></span>
  </div>`;
}

function statusBar() {
  const m = S.mode === 'insert' ? 'INSERT' : (S.leader ? 'SPACE' : 'NORMAL');
  const cls = S.mode === 'insert' ? 'insert' : (S.leader ? 'leader' : '');
  const scope = SCOPES[scopeNow()];
  const over = running() > MACHINE_LIMIT;
  const ws = S.screen === 'ws';
  return `<div class="status">
    <span class="mode ${cls}">${m}</span>
    <span class="pending">${esc(S.leader ? 'Space ' + S.leader.join(' ') : S.pending)}</span>
    <span class="st">${scope ? esc(scope.name) : ''}</span>
    ${ws && S.inbox.length ? `<span class="st warn">⚠ ${S.inbox.length} need you</span>` : ''}
    <span class="grow"></span>
    <span class="st ${over ? 'warn' : ''}" title="Running efforts on this machine against the soft limit">${ws ? limitText(running(), over) : `8 / ${MACHINE_LIMIT} efforts running ▲`}</span>
    ${ws ? `<span class="st" title="Claude profile of this project">work</span>
    <span class="st" title="Coordinator context fill"><span class="ctx-bar"><i style="width:62%"></i></span>ctx 62%</span>
    <span class="st" title="Usage limits of this profile">5h 34% · week 18%</span>
    <span class="st"><b>Opus 5.5</b><span class="lvl">Medium</span></span>
    <span class="st">Auto</span>
    <span class="st" title="A turn is running"><span class="spin"></span></span>` : ''}
    <button class="chip" data-a="designToggle" title="Design variants for this mock (F2)">◐ design</button><span class="hint"><kbd>?</kbd> keys</span>
  </div>`;
}

function renderHomeScreen() {
  const recent = PROJECTS.slice(0, 4).map((p, i) => `
    <button class="tile ${S.homeSel === i ? 'sel' : ''}" data-a="openProject" data-i="${i}">
      <div class="thumb"><i style="width:70%"></i><i style="width:90%"></i><i style="width:50%"></i><i style="width:80%"></i></div>
      <div class="meta"><b>${esc(p.name)}</b><span class="dim">${esc(p.opened)}</span></div>
    </button>`).join('');
  const rows = PROJECTS.map((p, i) => `
    <tr class="${S.homeSel === i ? 'sel' : ''}" data-a="openProject" data-i="${i}" style="cursor:pointer">
      <td><span class="pname">${esc(p.name)}</span><div class="dim" style="font-size:.88em">${esc(p.note)}</div></td>
      <td>${p.repos.length ? p.repos.map(r => `<span class="chip repo">${esc(r)}</span>`).join(' ') : '<span class="dim">no repo · research</span>'}</td>
      <td>${esc(p.account)}</td>
      <td>${p.running} running / ${p.efforts}</td>
      <td>${p.inbox ? `<span class="badge-n">${p.inbox} ⚠</span>` : '<span class="dim">—</span>'}</td>
      <td class="dim">${esc(p.opened)}</td>
    </tr>`).join('');
  return header(false) + `<div class="home">
      <h2>Recent projects</h2>
      <div class="recent">${recent}</div>
      <h2>All projects</h2>
      <table class="projects"><thead><tr><th>Name</th><th>Repos</th><th>Account</th><th>Efforts</th><th>Needs you</th><th>Opened</th></tr></thead><tbody>${rows}</tbody></table>
      <p class="dim" style="margin-top:20px"><kbd>j</kbd>/<kbd>k</kbd> move · <kbd>Enter</kbd> open · <kbd>n</kbd> new project · <kbd>?</kbd> keyboard map</p>
    </div>` + statusBar();
}

function renderWorkspace() {
  const nomain = !S.main;
  return header(true) + `<div class="ws ${nomain ? 'nomain' : ''} ${S.sideCollapsed ? 'railed"' : `" style="--side-w:${S.sideW}px"`}>
    ${renderChat()}
    ${nomain ? '' : renderMain()}
    ${renderSidebar()}
  </div>` + statusBar();
}

function renderSidebar() {
  const inboxRows = S.inbox.map((it, i) => `
    <div class="row ${S.inboxSel === i ? 'sel' : ''}" data-a="selInbox" data-i="${i}">
      <span class="g-${it.kind}">${it.glyph}</span>
      <span class="dot" style="background:${effortById(it.effort).hue}" title="${esc(effortById(it.effort).name)}"></span>
      <span class="t">${esc(it.title)}</span><span class="tm">${it.time}</span>
    </div>`).join('') || '<div class="row dim">Nothing needs you.</div>';
  const rows = treeRows().map((r, i) => {
    const sel = S.treeSel === i ? 'sel' : '';
    if (r.type === 'effort') {
      const done = r.e.criteria.filter(c => c.state === 'done').length;
      return `<div class="row effort-row ${sel} ${r.e.id === S.effort ? 'current' : ''}" data-a="selTree" data-i="${i}">
        <span class="dim">${S.folded[r.e.id] ? '▸' : '▾'}</span><span class="dot" style="background:${r.e.hue}"></span>
        <span class="t">${esc(r.e.name)}</span><span class="kind" title="${r.e.kind}">${KINDS[r.e.kind].glyph}</span><span class="tm">☑ ${done}/${r.e.criteria.length}</span></div>${S.wwi && D('wwi') === 'tree' && r.e.id === S.effort ? `<div class="wwi-tree">${wwiCard(r.e, true)}</div>` : ''}`;
    }
    if (r.type === 'task') return `<div class="row indent1 ${sel}" data-a="selTree" data-i="${i}"><span class="t">${esc(r.t.title)}</span><span class="status-pill st-${r.t.status.split(' ')[0]}">${r.t.status}</span></div>`;
    if (r.type === 'agent') return `<div class="row indent2 ${sel}" data-a="selTree" data-i="${i}"><span class="${r.a.state === 'needs' ? 'needs' : 'agent-g'}">${r.a.state === 'needs' ? '⚠' : '◐'}</span><span>◆ ${esc(r.a.name)}</span><span class="branch t">${esc(r.a.branch)}</span></div>`;
    return `<div class="row indent3 ${sel}" data-a="selTree" data-i="${i}"><span class="${r.a.state === 'done' ? 'g-accept' : 'agent-g'}">${r.a.state === 'done' ? '✓' : '◐'}</span><span class="t">◇ ${esc(r.a.name)}</span></div>`;
  }).join('');
  const fz = z => S.zone === z ? 'focus' : '';
  if (S.sideCollapsed) {
    return `<div class="zone sidebar rail">
      <button class="rail-inbox" data-a="focusInbox" title="Inbox (Space n)">${S.inbox.length ? `<span class="needs">⚠</span> ${S.inbox.length}` : '✓'}</button>
      ${EFFORTS.map(e => `<button class="rail-effort ${e.id === S.effort ? 'current' : ''}" data-a="railEffort" data-id="${e.id}" title="${esc(e.name)}"><span class="dot" style="background:${e.hue}"></span></button>`).join('')}
      <span class="spacer"></span><button class="dim" data-a="toggleSide" title="Expand the sidebar (Space z)">⇤</button></div>`;
  }
  return `<div class="zone sidebar"><div class="side-handle" id="sidehandle" title="Drag to widen"></div>
    <div class="part ${fz('inbox')}"><div class="zone-title ${fz('inbox')}" data-a="zone" data-z="inbox">Inbox · ${S.inbox.length}<span class="spacer"></span><button class="dim collapse-btn" data-a="toggleSide" title="Collapse the sidebar (Space z)">⇥</button></div><div class="list">${inboxRows}</div></div>
    <div class="part ${fz('tree')}"><div class="zone-title ${fz('tree')}" data-a="zone" data-z="tree">Efforts</div><div class="list">${rows}</div></div>
  </div>`;
}

function renderMain() {
  const tabs = [['canvas', 'Canvas'], ['viewer', 'Viewer'], ['reading', 'Reading']];
  if (S.main === 'item') tabs.push(['item', 'Inbox item']);
  let body = '';
  if (S.main === 'canvas') body = renderCanvas();
  else if (S.main === 'viewer') body = renderViewer();
  else if (S.main === 'reading') body = `<div class="reading" id="reading"><article>${md(REPORT)}</article></div>`;
  else if (S.main === 'item') body = renderItem();
  return `<div class="zone ${S.zone === 'main' ? 'focus' : ''}">
    <div class="zone-title" data-a="zone" data-z="main"><span>${tabs.find(x => x[0] === S.main)[1]}</span><span class="spacer"></span><button class="dim" data-a="closeMain" title="Close the view; the chat takes the space">✕ close <kbd>Space v x</kbd></button></div>
    ${S.wwi && D('wwi') === 'main' ? `<div class="wwi-main">${wwiCard(effortById(S.effort), true)}</div>` : ''}
    <div class="main-body">${body}</div></div>`;
}

function renderCanvas() {
  if (S.effort !== 'pay') {
    const e = effortById(S.effort);
    return `<div class="empty-main"><div style="text-align:center">
      <div style="font-size:1.1em;margin-bottom:12px">${esc(e.name)} has no canvas yet. Start from a framework:</div>
      <div class="answers" style="justify-content:center">
        <button data-a="fw">1 · Question tree</button><button data-a="fw">2 · Options compare</button><button data-a="fw">3 · Free board</button></div>
      <p class="dim">The mock's filled canvas is on <b>payment-providers</b> (<kbd>]e</kbd> / <kbd>[e</kbd>).</p></div></div>`;
  }
  const byId = Object.fromEntries(S.cards.map(c => [c.id, c]));
  const lines = S.edges.map(([a, b]) => {
    const A = byId[a], B = byId[b]; if (!A || !B) return '';
    const x1 = A.x + 125, y1 = A.y + 70, x2 = B.x + 125, y2 = B.y;
    return `<path d="M${x1},${y1} C${x1},${(y1 + y2) / 2} ${x2},${(y1 + y2) / 2} ${x2},${y2}" style="stroke:var(--edge-hi)" stroke-width="2" fill="none"/>`;
  }).join('');
  const cardHtml = (c, inc) => `<div class="card ${c.type} ${(!inc && S.canvasPart === 'cards' && S.cardSel === c.id) || (inc && S.canvasPart === 'incoming' && S.incoming[S.incSel] === c) ? 'sel' : ''}"
      ${inc ? '' : `style="left:${c.x}px;top:${c.y}px"`} data-a="${inc ? 'selInc' : 'selCard'}" data-id="${c.id}">
      <div class="ct">${c.type} · ${c.id}<button class="refbtn" data-a="refCardBtn" data-id="${c.id}" title="Insert this card's reference into the chat (r)">↗ chat</button></div>
      <div class="ttl">${esc(c.title)}</div><div class="bd" ${inc ? `data-under="goes under ${c.under}"` : ''}>${esc(c.body)}${c.long ? ' <span class="dim">· Enter reads it</span>' : ''}</div></div>`;
  return `<div class="framework">framework: <b>question tree</b> · ${S.cards.length} cards · zoom ${Math.round(S.zoom * 100)} %</div>
    <div class="canvas-wrap"><div class="canvas" style="transform:scale(${S.zoom})"><svg width="960" height="600">${lines}</svg>${S.cards.map(c => cardHtml(c, false)).join('')}</div></div>
    <div class="incoming ${S.zone === 'main' && S.canvasPart === 'incoming' ? 'focus' : ''}"><span class="lbl">incoming · ${S.incoming.length}</span>
      ${S.incoming.map(c => cardHtml(c, true)).join('') || '<span class="dim" style="align-self:center">Nothing new. New cards from agents land here first.</span>'}</div>`;
}

function renderViewer() {
  const c = S.choice;
  const site = `<div class="site t${c[0]}">
      <header class="h${c[2]}"><span class="logo">SHOP</span><nav><span>New</span><span>Women</span><span>Men</span><span>Home</span></nav>
        <span class="tog">${c[2] === 0 ? 'Dark mode: on' : '☾'}</span></header>
      <div class="hero">${esc(VARIANTS[4].options[c[4]])}</div>
      <div class="grid ${c[1] === 1 ? 'list' : ''}">${['Wool scarf', 'Desk lamp', 'Canvas bag', 'Mug set', 'Boots', 'Throw'].map(p => `<div class="prod">${p}</div>`).join('')}</div>
      <footer class="${c[3] === 1 ? 'full' : ''}">${c[3] === 1 ? '<span>About</span><span>Shipping</span><span>Returns</span><span>Contact</span><span>Careers</span><span>© Shop</span>' : '© Shop · Shipping · Returns'}</footer></div>`;
  let last = '';
  const slots = VARIANTS.map((v, i) => {
    const lvl = v.level !== last ? `<div class="lvl">${v.level}</div>` : ''; last = v.level;
    return lvl + `<div class="slot ${S.slot === i ? 'sel' : ''}" data-a="selSlot" data-i="${i}"><div class="sn">${v.slot}</div><div class="opts">${v.options.map((o, j) =>
      `<button class="opt ${c[i] === j ? 'on' : ''} ${S.kept[i] === j ? 'kept' : ''}" data-a="pickOpt" data-i="${i}" data-j="${j}"><span class="n">${j + 1}</span>${esc(o)}</button>`).join('')}</div></div>`;
  }).join('');
  const same = c.every((v, i) => v === S.kept[i]);
  const det = S.panel === 'detached';
  const panel = S.panel === 'closed' ? `<button class="chip panel-closed" data-a="togglePanel">variants closed · <kbd>p</kbd> opens</button>` :
    `<div class="vpanel ${det ? 'detached' : ''}" id="vpanel" ${det ? `style="left:${S.panelPos.x}px;top:${S.panelPos.y}px"` : ''}>
      <div class="vh" data-drag>Variants<span class="spacer"></span><button data-a="detachPanel">${det ? '⇲ dock' : '⇱ detach'}</button><button data-a="closePanel">✕</button></div>
      ${slots}
      <button class="keep ${same ? 'same' : ''}" data-a="keepCombo">${same ? 'This is the kept combination' : 'Keep this combination  (y)'}</button></div>`;
  return `<div class="viewer"><div class="preview"><div class="urlbar"><span>◆ header-toggle · feat/dark-mode</span><span class="mono">localhost:5173/</span><span class="spacer"></span><span>⟳ R · ↗ gx</span></div>${site}</div>${panel}</div>`;
}

function renderItem() {
  const it = S.inbox.find(x => x.id === S.mainItem);
  if (!it) return '<div class="empty-main">Answered.</div>';
  const isAccept = it.kind === 'accept';
  const btns = it.kind === 'perm' ? [['allow', 'y · allow once', 1], ['allowAlways', 'A · always allow'], ['deny', 'd · deny'], ['denyReason', 'D · deny with reason']]
    : isAccept ? [['allow', 'y · approve, tick the criterion', 1], ['deny', 'd · send back'], ['denyReason', 'D · send back with reason']]
    : it.kind === 'question' ? [['reply', 'r · reply', 1], ['dismiss', 'x · dismiss']] : [['openAgent', 'Enter · open', 1], ['dismiss', 'x · dismiss']];
  return `<div class="itemview"><article>
    <div style="display:flex;gap:10px;align-items:center"><span class="g-${it.kind}" style="font-size:1.3em">${it.glyph}</span>${tagHtml(it.effort)}<span class="dim">from ${esc(it.from)} · ${it.time} ago</span></div>
    ${md(it.body)}
    <div class="answers">${btns.map(([a, l, p]) => `<button class="${p ? 'primary' : ''}" data-a="${a}">${l}</button>`).join('')}<button data-a="escape">Esc · back</button></div>
  </article></div>`;
}

function wwiCard(e: Effort, compact = false): string {
  return `<div class="wwi" id="wwi">
      <h3><span class="dot" style="background:${e.hue}"></span>Where was I · ${esc(e.name)} <span class="kind">${KINDS[e.kind].glyph} ${e.kind}</span></h3>
      ${compact ? '' : `<div class="goal">${esc(e.goal)}</div>`}
      <dl><dt>Last done</dt><dd>${e.wwi.last}</dd>
        <dt>Waits on you</dt><dd>${e.wwi.waits}</dd>
        <dt>Next step</dt><dd>${e.wwi.next}</dd>
        ${compact ? '' : `<dt>Criteria</dt><dd><ul class="crit">${e.criteria.map(c => `<li>${c.state === 'done' ? '<span class="g-accept">☑</span>' : c.state === 'review' ? '<span class="g-question">◔</span>' : '<span class="dim">☐</span>'} ${esc(c.text)}${c.state === 'review' ? ' <span class="dim">· in your inbox</span>' : ''}</li>`).join('')}</ul></dd>`}</dl>
      <div class="foot"><kbd>Esc</kbd> hide · <kbd>w</kbd> in the tree or <kbd>Space e w</kbd> brings it back</div></div>`;
}

function renderChat() {
  const e = effortById(S.effort);
  const vis = visibleChat();
  const msgs = vis.map(({ m, i }, vi) => {
    const cur = S.zone === 'chat' && S.chatCur === vi ? 'cur' : '';
    if (m.report) {
      const open = S.openReports.has(i);
      return `<div class="msg ${cur}" data-vi="${vi}"><div class="report" data-a="toggleReport" data-i="${i}"><span>${open ? '▾' : '▸'}</span>${tagHtml(m.effort)}<span class="from">${esc(m.from)}</span><span class="t">${esc(m.text)}</span><span class="spacer"></span><span>${m.time}</span></div>
        ${open ? `<div class="report-open">${md('**' + m.from + ' reports:** ' + m.text + '.\n\nFull report: `Enter` on its card, or `Space v r`.')}</div>` : ''}</div>`;
    }
    return `<div class="msg ${m.who === 'You' ? 'by-you' : 'by-agent'} ${cur}" data-vi="${vi}"><div class="msg-h"><span class="who ${m.who === 'You' ? 'you' : ''}">${esc(m.who)}</span>${S.filter === 'all' ? (D('density') !== 'full' ? tagDot(m.effort) : tagHtml(m.effort)) : ''}<span class="rule"></span><span class="time">${m.time}</span></div><div class="msg-body">${md(m.text)}</div></div>`;
  }).join('');
  const wwi = S.wwi && (D('wwi') === 'chat' || (D('wwi') === 'main' && !S.main)) ? wwiCard(e) : '';
  const filters = [['all', '<b style="color:inherit">All</b>']].concat(EFFORTS.map(x => [x.id, `<span class="dot" style="background:${x.hue}"></span>${esc(x.name)}`]))
    .map(([id, l]) => `<button class="fbtn ${S.filter === id ? 'on' : ''}" data-a="setFilter" data-f="${id}">${l}</button>`).join('');
  return `<div class="zone ${S.zone === 'chat' ? 'focus' : ''}">
    <div class="zone-title" data-a="zone" data-z="chat">Coordinator · Shop</div>
    ${D('density') !== 'full' ? `<div class="filter-min"><button data-a="pickFilter">${S.filter === 'all' ? 'All efforts' : `<span class="dot" style="background:${effortById(S.filter).hue}"></span> ${esc(effortById(S.filter).name)}`} ▾</button><span class="dim">f</span></div>` : `<div class="filter-row">${filters}</div>`}
    <div class="chat-scroll" id="chatscroll">${msgs}${wwi}</div>
    <div class="chatbar ${S.mode === 'insert' ? 'active' : ''}"><div class="chatbar-inner">
      <div class="to">${D('density') !== 'full' ? tagDot(S.tag) : `to coordinator, tagged ${tagHtml(S.tag)}`} <span class="spacer"></span><span class="tag-hint"><kbd>Ctrl-t</kbd> change tag</span></div>
      <textarea id="input" rows="2" placeholder="${S.mode === 'insert' ? 'Type a message · Enter sends · Alt-Enter new line' : 'i to type'}">${esc(S.draft)}</textarea></div></div>
  </div>`;
}

/* ---------------- overlay: the drawn keyboard ---------------- */
const KB_ROWS = [
  [['<Esc>', 'Esc', null, 'w15'], ['1', '1', '!'], ['2', '2', '@'], ['3', '3', '#'], ['4', '4', '$'], ['5', '5', '%'], ['6', '6', '^'], ['7', '7', '&'], ['8', '8', '*'], ['9', '9', '('], ['0', '0', ')'], ['-', '-', '_'], ['=', '=', '+'], [null, '⌫', null, 'w15']],
  [['<Tab>', 'Tab', '<S-Tab>', 'w15'], ['q'], ['w'], ['e'], ['r'], ['t'], ['y'], ['u'], ['i'], ['o'], ['p'], ['[', '[', '{'], [']', ']', '}'], ['\\', '\\', '|']],
  [[null, 'Caps', null, 'w2'], ['a'], ['s'], ['d'], ['f'], ['g'], ['h'], ['j'], ['k'], ['l'], [';', ';', ':'], ["'", "'", '"'], ['<CR>', 'Enter', null, 'w2']],
  [[null, 'Shift', null, 'w25'], ['z'], ['x'], ['c'], ['v'], ['b'], ['n'], ['m'], [',', ',', '<'], ['.', '.', '>'], ['/', '/', '?'], [null, 'Shift', null, 'w25']],
  [['<Space>', 'Space', null, 'space']],
];
function leaderNode(path) { let n = { keys: LEADER }; for (const k of path) n = n.keys[k]; return n; }
function keyInfo(def, scope, layer) {
  let [base, cap, shifted, w] = def; cap = cap || base; w = w || '';
  if (base && base.length === 1 && /[a-z]/.test(base)) shifted = base.toUpperCase();
  let token = null, shownCap = cap;
  if (layer === 'plain') token = base;
  else if (layer === 'shift') { token = shifted; if (shifted && shifted.length === 1) shownCap = shifted; }
  else if (layer === 'ctrl') token = base && /^[a-z]$/.test(base) ? 'C-' + base : null;
  else if (layer === 'leader') {
    const node = leaderNode(S.ovLeader); let k = base;
    if (base === '<Space>' && S.ovLeader.length === 0) k = '<Space>';
    const v = k && node.keys[k];
    if (base === '<Esc>') return { w, cap: shownCap, label: S.ovLeader.length ? 'up a level' : 'cancel', cls: 'c-mode' };
    if (!v) return { w, cap: shownCap, label: '', cls: '' };
    if (Array.isArray(v)) return { w, cap: shownCap, label: v[1], cls: 'c-view' };
    return { w, cap: shownCap, label: v.name + ' ▸', cls: 'c-group', group: k };
  }
  if (!token) return { w, cap: shownCap, label: '', cls: '' };
  if (layer === 'plain' && token === '<Space>' && scope !== 'insert') return { w, cap: shownCap, label: 'leader ▸ (see Space layer)', cls: 'c-group' };
  const binds = bindingsFor(scope);
  const local = new Set((KEYMAP[scope] || []).map(b => b[0]));
  const exact = binds.find(b => b[0] === token && b[2]);
  const pre = binds.filter(b => b[0].length > token.length && b[0].startsWith(token) && !token.startsWith('<') && b[2]);
  if (exact) return { w, cap: shownCap, label: exact[2] + (pre.length ? ' · ' + pre.map(p => p[0] + ' ' + p[2]).join(', ') : ''), cls: 'c-' + exact[3] + (local.has(token) ? ' c-local' : '') };
  if (pre.length) return { w, cap: shownCap, label: pre.map(p => `<b style="color:var(--ink)">${esc(p[0])}</b> ${esc(p[2])}`).join('<br>'), cls: 'c-pre', raw: true };
  return { w, cap: shownCap, label: '', cls: '' };
}
function renderOverlay() {
  const ov = $('#overlay');
  if (!S.overlay) { ov.hidden = true; return; }
  ov.hidden = false;
  const sc = S.ovScope, ly = S.ovLayer;
  const scopes = Object.entries(SCOPES).map(([k, v]) => `<button class="${sc === k ? 'on' : ''}" data-a="ovScope" data-s="${k}">${v.name}</button>`).join('');
  const layers = [['plain', 'Keys'], ['shift', 'Shift'], ['ctrl', 'Ctrl'], ['leader', 'Space leader']].map(([k, n]) => `<button class="${ly === k ? 'on' : ''}" data-a="ovLayer" data-l="${k}">${n}</button>`).join('');
  const kb = KB_ROWS.map(row => `<div class="kb-row">${row.map(def => {
    const k = keyInfo(def, sc, ly);
    return `<div class="key ${k.w} ${k.cls}" ${k.group ? `data-a="ovDrill" data-k="${k.group}" style="cursor:pointer"` : ''}><span class="cap">${esc(k.cap)}</span><span class="lab">${k.raw ? k.label : esc(k.label)}</span></div>`;
  }).join('')}</div>`).join('');
  const seqs = bindingsFor(sc).filter(b => b[2] && (b[0].length > 1 && !b[0].startsWith('<') && !b[0].startsWith('C-') || b[0].startsWith('<')));
  const leaderTree = Object.entries(LEADER).map(([k, v]) => Array.isArray(v)
    ? `<div class="grp"><kbd>Space ${k === '<Space>' ? 'Space' : k}</kbd> ${esc(v[1])}</div>`
    : `<div class="grp"><div><kbd>Space ${k}</kbd> <b>${v.name}</b></div>${Object.entries(v.keys).map(([k2, l]) => `<div style="padding-left:18px"><kbd>${k2}</kbd> ${esc(l[1])}</div>`).join('')}</div>`).join('');
  const crumb = ly === 'leader' ? `<div style="margin:4px 0">Showing: <kbd>Space</kbd>${S.ovLeader.map(k => ` <kbd>${k}</kbd>`).join('')} ${S.ovLeader.length ? '<button class="chip" data-a="ovUp">↑ up a level</button>' : '<span class="dim">click a group key to see inside it</span>'}</div>` : '';
  ov.innerHTML = `<div class="ov-head"><h2>Keyboard · ${esc(SCOPES[sc].name)}</h2><span class="dim">${esc(SCOPES[sc].hint)}</span><span class="spacer"></span>
      <span class="dim"><kbd>[</kbd> <kbd>]</kbd> view · <kbd>Tab</kbd> layer · <kbd>Esc</kbd> close</span><button class="chip" data-a="keys">✕ close</button></div>
    <div class="ov-head"><div class="seg">${scopes}</div><div class="seg">${layers}</div></div>
    <div class="legend"><span class="lg-move">move</span><span class="lg-act">act on it</span><span class="lg-view">open / switch view</span><span class="lg-mode">mode</span><span class="lg-group">leader group</span><span>● = only in this view; the rest work everywhere</span></div>
    ${crumb}
    <div class="kb">${kb}</div>
    <div class="ov-cols">
      <div class="ov-box"><h3>Sequences and special keys here</h3>${seqs.map(b => `<div><kbd>${esc(b[0])}</kbd> ${esc(b[2])}</div>`).join('') || '<div class="dim">none</div>'}</div>
      <div class="ov-box"><h3>Space leader</h3><div class="leader-tree">${leaderTree}</div></div>
      <div class="ov-box"><h3>Zones</h3>
        <div><kbd>h</kbd>/<kbd>l</kbd> move between chat, main view and sidebar (left to right) where the view has no use for them; <kbd>Ctrl-h</kbd>/<kbd>Ctrl-l</kbd> always do (also in the canvas and while typing).</div>
        <div><kbd>Ctrl-j</kbd>/<kbd>Ctrl-k</kbd> inbox ⇄ efforts tree, canvas ⇄ incoming strip.</div>
        <div><kbd>]e</kbd>/<kbd>[e</kbd> next or previous effort from anywhere; the "where was I" card comes up.</div>
        <div>Mouse: everything is clickable, nothing needs it.</div></div>
    </div>`;
}
function renderWhichKey() {
  const w = $('#whichkey');
  if (!S.leader) { w.hidden = true; return; }
  const node = leaderNode(S.leader);
  w.hidden = false;
  w.innerHTML = `<h4>Space${S.leader.map(k => ' ' + k).join('')}${S.leader.length ? ' · ' + node.name : ''}</h4><div class="wk">${Object.entries(node.keys).map(([k, v]) =>
    `<kbd>${k === '<Space>' ? 'Space' : k}</kbd><span class="${Array.isArray(v) ? '' : 'grp-n'}">${Array.isArray(v) ? esc(v[1]) : v.name + ' ▸'}</span>`).join('')}</div>`;
}

/* ---------------- modal: picker / prompt / form ---------------- */
function picker(title, items, onPick) { S.modal = { type: 'picker', title, items, sel: 0, q: '', onPick }; }
function prompt(title, onDone, value = '') { S.modal = { type: 'prompt', title, q: value, onDone }; }
function renderModal() {
  let m = document.querySelector('.modal'); if (m) m.remove();
  if (!S.modal) return;
  const d = document.createElement('div'); d.className = 'modal';
  if (S.modal.type === 'picker') {
    const items = pickerItems();
    d.innerHTML = `<div class="ttl">${esc(S.modal.title)}</div><input id="pq" value="${esc(S.modal.q)}" placeholder="type to filter · Ctrl-j/Ctrl-k or arrows · Enter · Esc">
      ${items.map((it, i) => `<div class="row ${i === S.modal.sel ? 'sel' : ''}" data-a="pickRow" data-i="${i}">${it.html}</div>`).join('')}`;
  } else if (S.modal.type === 'prompt') {
    d.innerHTML = `<div class="ttl">${esc(S.modal.title)}</div><input id="pq" value="${esc(S.modal.q)}">`;
  } else if (S.modal.type === 'form') {
    d.innerHTML = S.modal.html;
  }
  document.body.appendChild(d);
  const q = $('#pq'); if (q) { q.focus(); q.setSelectionRange(q.value.length, q.value.length); }
}
function pickerItems() { const q = S.modal.q.toLowerCase(); return S.modal.items.filter(it => it.text.toLowerCase().includes(q)); }

/* ---------------- after render: focus, scroll ---------------- */
function afterRender() {
  const ta = $('#input');
  if (ta) {
    if (S.mode === 'insert' && !S.modal && !S.overlay) { ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); }
    ta.oninput = () => { S.draft = ta.value; };
    ta.onfocus = () => { if (S.mode !== 'insert') { S.mode = 'insert'; S.zone = 'chat'; render(); } };
  }
  const cs = $('#chatscroll');
  if (cs) {
    if (S.chatCur >= 0) { const el = cs.querySelector(`[data-vi="${S.chatCur}"]`); if (el) el.scrollIntoView({ block: 'nearest' }); }
    else cs.scrollTop = cs.scrollHeight;
  }
  const r = $('#reading');
  if (r) { const hs = r.querySelectorAll('[data-h]'); hs.forEach((h, i) => h.classList.toggle('cur', i === S.readHead)); if (hs[S.readHead] && S.readHead > 0) hs[S.readHead].scrollIntoView({ block: 'start' }); }
  const sel = document.querySelector('.canvas .card.sel'); if (sel) sel.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  document.querySelectorAll('.list .row.sel').forEach(el => el.scrollIntoView({ block: 'nearest' }));
  const sh = $('#sidehandle');
  if (sh) sh.onmousedown = ev => {
    ev.preventDefault(); sh.classList.add('drag');
    const ws = $('.ws'); const right = ws.getBoundingClientRect().right;
    const mv = e2 => { S.sideW = Math.max(240, Math.min(700, Math.round(right - e2.clientX - 8))); ws.style.setProperty('--side-w', S.sideW + 'px'); };
    const up = () => { sh.classList.remove('drag'); removeEventListener('mousemove', mv); removeEventListener('mouseup', up); };
    addEventListener('mousemove', mv); addEventListener('mouseup', up);
  };
  const dp = $('#designpanel [data-dragdesign]');
  if (dp) dp.onmousedown = ev => {
    const p = $('#designpanel'); const r = p.getBoundingClientRect(); const sx = ev.clientX - r.left, sy = ev.clientY - r.top;
    const mv = e2 => { DS.pos = { x: e2.clientX - sx, y: e2.clientY - sy }; p.style.left = DS.pos.x + 'px'; p.style.top = DS.pos.y + 'px'; p.style.bottom = 'auto'; };
    const up = () => { saveDesign(); removeEventListener('mousemove', mv); removeEventListener('mouseup', up); };
    addEventListener('mousemove', mv); addEventListener('mouseup', up);
  };
  const vp = $('#vpanel [data-drag]');
  if (vp && S.panel === 'detached') vp.onmousedown = ev => {
    const sx = ev.clientX - S.panelPos.x, sy = ev.clientY - S.panelPos.y;
    const mv = e2 => { S.panelPos = { x: e2.clientX - sx, y: e2.clientY - sy }; const p = $('#vpanel'); p.style.left = S.panelPos.x + 'px'; p.style.top = S.panelPos.y + 'px'; };
    const up = () => { removeEventListener('mousemove', mv); removeEventListener('mouseup', up); };
    addEventListener('mousemove', mv); addEventListener('mouseup', up);
  };
}

/* ---------------- actions ---------------- */
const ZONES_H = () => S.main ? ['chat', 'main', 'side'] : ['chat', 'side'];
function zoneKey() { return S.zone === 'inbox' || S.zone === 'tree' ? 'side' : S.zone; }
function setZone(z) { if (z === 'side') z = S.side; if (z === 'inbox' || z === 'tree') { S.side = z; S.sideCollapsed = false; } S.zone = z; S.chatCur = z === 'chat' ? S.chatCur : S.chatCur; }

function switchEffort(id, quiet) {
  S.effort = id; S.tag = id; S.wwi = true;
  if (S.filter !== 'all') S.filter = id;
  if (!quiet) toast(`Effort: ${tagHtml(id)} · next message is tagged with it`);
}
function nearestCard(dx, dy) {
  const c = S.cards.find(x => x.id === S.cardSel); if (!c) return;
  let best = null, bd = 1e9;
  for (const o of S.cards) {
    if (o === c) continue;
    const vx = o.x - c.x, vy = o.y - c.y;
    const along = vx * dx + vy * dy; if (along <= 0) continue;
    const across = Math.abs(vx * dy - vy * dx);
    const d = along + across * 2; if (d < bd) { bd = d; best = o; }
  }
  if (best) S.cardSel = best.id;
}
function refInsert(id) {
  const c = S.cards.find(x => x.id === id) || S.incoming.find(x => x.id === id);
  if (!c) return;
  S.draft = (S.draft ? S.draft.trimEnd() + ' ' : '') + `[[${c.id} ${c.title}]] `;
  S.mode = 'insert'; S.zone = 'chat';
  toast(`Reference to <b>${c.id}</b> put in the chat bar`);
}
function answerItem(kind) {
  const it = S.zone === 'main' && S.main === 'item' ? S.inbox.find(x => x.id === S.mainItem) : S.inbox[S.inboxSel];
  if (!it) return;
  const e = effortById(it.effort);
  const remove = () => { S.inbox = S.inbox.filter(x => x !== it); S.inboxSel = Math.min(S.inboxSel, Math.max(0, S.inbox.length - 1)); if (S.main === 'item' && S.mainItem === it.id) { S.main = null; S.zone = S.side; } };
  if (kind === 'allow' && it.kind === 'accept') {
    const c = e.criteria.find(c => c.state === 'review'); if (c) c.state = 'done';
    remove(); toast(`Approved · criterion ticked in ${tagHtml(e.id)}`);
  } else if (kind === 'allow' || kind === 'allowAlways') {
    if (it.kind !== 'perm') return toast('Not a permission request');
    remove(); S.chat.push({ report: true, effort: it.effort, from: it.from, time: '10:02', text: 'Safari e2e running (approved' + (kind === 'allowAlways' ? ', always' : ' once') + ')' });
    toast(kind === 'allow' ? 'Allowed once' : 'Always allowed for this project');
  } else if (kind === 'deny' || kind === 'denyReason') {
    const label = it.kind === 'accept' ? 'Sent back' : 'Denied';
    if (kind === 'denyReason') prompt(`${label} with reason, to ${it.from}:`, v => { remove(); toast(`${label}: “${esc(v)}”`); });
    else { remove(); toast(label); }
  } else if (kind === 'reply') {
    if (it.kind !== 'question') return toast('Only questions take a reply');
    prompt(`Reply to ${it.from}:`, v => { remove(); S.chat.push({ who: 'You', time: '10:03', effort: it.effort, text: `@${it.from} ${v}` }); toast('Reply sent'); });
  } else if (kind === 'dismiss') { remove(); toast('Dismissed'); }
}

function act(a: string, arg?: unknown): void {
  const sc = scopeNow();
  switch (a) {
    case 'none': return;
    case 'home': S.screen = 'home'; S.main = null; break;
    case 'openProject': {
      const i = arg != null ? +arg : S.homeSel; S.homeSel = i;
      if (PROJECTS[i].id !== 'shop') { toast(`The mock only has <b>Shop</b> filled in. Opening Shop.`); }
      S.screen = 'ws'; S.zone = 'chat'; break;
    }
    case 'newProject': S.modal = { type: 'form', html: formHtml('New project', [['Name', 'Shop'], ['Repos', 'shop-api, shop-web  (any number; none for research)'], ['Claude account', 'work']]) }; break;
    case 'addRepo': toast('Add repo: folder picker, the repo joins this project'); break;
    case 'down': case 'up': {
      const d = a === 'down' ? 1 : -1;
      if (sc === 'home') S.homeSel = Math.max(0, Math.min(PROJECTS.length - 1, S.homeSel + d));
      else if (sc === 'inbox') S.inboxSel = Math.max(0, Math.min(S.inbox.length - 1, S.inboxSel + d));
      else if (sc === 'tree') S.treeSel = Math.max(0, Math.min(treeRows().length - 1, S.treeSel + d));
      break;
    }
    case 'top': if (sc === 'home') S.homeSel = 0; else if (sc === 'chat') S.chatCur = 0; else if (sc === 'reading') S.readHead = 0, $('#reading').scrollTop = 0; break;
    case 'bottom': if (sc === 'home') S.homeSel = PROJECTS.length - 1; else if (sc === 'chat') S.chatCur = -1; else if (sc === 'reading') { const r = $('#reading'); r.scrollTop = r.scrollHeight; return; } break;
    case 'zoneLeft': case 'zoneRight': {
      const zs = ZONES_H(); let i = zs.indexOf(zoneKey());
      i = Math.max(0, Math.min(zs.length - 1, i + (a === 'zoneLeft' ? -1 : 1)));
      setZone(zs[i]); break;
    }
    case 'zoneDown': case 'zoneUp':
      if (S.zone === 'inbox' || S.zone === 'tree') setZone(a === 'zoneDown' ? 'tree' : 'inbox');
      else if (S.zone === 'main' && S.main === 'canvas') S.canvasPart = a === 'zoneDown' ? 'incoming' : 'cards';
      break;
    case 'insert': S.mode = 'insert'; S.zone = 'chat'; break;
    case 'normal': S.mode = 'normal'; break;
    case 'leaveBar': S.mode = 'normal'; S.zone = 'chat'; break;
    case 'escape':
      if (S.wwi && D('wwi') !== 'chat') { S.wwi = false; break; }
      if (S.zone === 'main' && S.main === 'item') { S.main = null; S.zone = 'inbox'; }
      else if (S.zone === 'main' && S.main === 'reading') { S.main = 'canvas'; }
      else if (S.wwi && S.zone === 'chat') S.wwi = false;
      else if (S.zone === 'chat') S.chatCur = -1;
      break;
    case 'command': prompt(':', v => runCommand(v)); break;
    case 'search': toast('Search in this view (not mocked)'); break;
    case 'keys': S.overlay = !S.overlay; S.ovScope = scopeNow(); S.ovLayer = 'plain'; S.ovLeader = []; break;
    case 'nextEffort': case 'prevEffort': {
      const i = EFFORTS.findIndex(e => e.id === S.effort);
      switchEffort(EFFORTS[(i + (a === 'nextEffort' ? 1 : EFFORTS.length - 1)) % EFFORTS.length].id); break;
    }
    // inbox
    case 'openItem': { const it = S.inbox[S.inboxSel]; if (!it) break;
      if (it.kind === 'done') { S.main = 'viewer'; S.zone = 'main'; switchEffort(it.effort, true); S.inbox.splice(S.inboxSel, 1); toast('Finished turn opened in the viewer; the item is cleared'); }
      else { S.main = 'item'; S.mainItem = it.id; S.zone = 'main'; }
      break; }
    case 'allow': case 'allowAlways': case 'deny': case 'denyReason': case 'reply': case 'dismiss': answerItem(a); break;
    case 'itemEffort': { const it = S.inbox[S.inboxSel]; if (it) switchEffort(it.effort); break; }
    case 'openAgent': { const it = S.inbox.find(x => x.id === S.mainItem); if (it && it.kind === 'done') { S.main = 'viewer'; } else toast(`Opens ${it ? it.from : 'the agent'}'s conversation in the chat zone (not mocked)`); break; }
    // tree
    case 'fold': { const r = treeRows()[S.treeSel]; if (r) S.folded[r.e.id] = !S.folded[r.e.id]; S.treeSel = treeRows().findIndex(x => x.type === 'effort' && x.e === r.e); break; }
    case 'openRow': { const r = treeRows()[S.treeSel]; if (!r) break;
      if (r.type === 'effort') { switchEffort(r.e.id); S.folded[r.e.id] = false; }
      else if (r.type === 'task') toast(`Task “${esc(r.t.title)}” · status is set by Dovetail: ${r.t.status}`);
      else toast(`Opens ${esc(r.a.name)}'s conversation in the chat zone (not mocked yet)`);
      break; }
    case 'wwi': S.wwi = true; S.zone = S.zone === 'tree' ? S.zone : 'chat'; setTimeout(() => { const w = $('#wwi'); if (w) { w.classList.add('flash'); w.scrollIntoView({ block: 'nearest' }); } }, 0); break;
    case 'addTask': prompt('New task in ' + effortById(treeRows()[S.treeSel].e.id).name + ':', v => { treeRows()[S.treeSel].e.tasks.push({ title: v, status: 'To do', agents: [] }); }); break;
    case 'rename': toast('Rename (inline edit, not mocked)'); break;
    case 'moveDown': case 'moveUp': toast('Reorder (not mocked)'); break;
    case 'archive': toast('Archive, asks first (not mocked)'); break;
    // chat
    case 'scrollDown': case 'scrollUp':
      if (sc === 'chat') { const n = visibleChat().length; let c = S.chatCur < 0 ? n - 1 : S.chatCur; c += a === 'scrollDown' ? 1 : -1; S.chatCur = c >= n ? -1 : Math.max(0, c); }
      else if (sc === 'reading') { $('#reading').scrollBy(0, a === 'scrollDown' ? 60 : -60); return; }
      break;
    case 'nextTurn': case 'prevTurn': {
      const v = visibleChat(); let c = S.chatCur < 0 ? v.length : S.chatCur; const d = a === 'nextTurn' ? 1 : -1;
      for (c += d; c >= 0 && c < v.length; c += d) if (v[c].m.who === 'You') break;
      S.chatCur = c >= v.length ? -1 : Math.max(0, c); break;
    }
    case 'unfold': case 'foldEntry': { const v = visibleChat()[S.chatCur]; if (v && v.m.report) { if (a === 'unfold') S.openReports.add(v.i); else S.openReports.delete(v.i); } break; }
    case 'pickFilter': picker('Show in the chat', [{ text: 'All efforts', html: '<span class="t">All efforts</span>', id: 'all' }, ...EFFORTS.map(e => ({ text: e.name, html: `${tagHtml(e.id)}`, id: e.id }))], it => { S.filter = it.id; if (it.id !== 'all') switchEffort(it.id, true); S.chatCur = -1; }); break;
    case 'cycleFilter': S.filter = S.filter === 'all' ? S.effort : 'all'; S.chatCur = -1; break;
    // canvas
    case 'cardLeft': nearestCard(-1, 0); break; case 'cardRight': nearestCard(1, 0); break;
    case 'cardUp': nearestCard(0, -1); break; case 'cardDown': nearestCard(0, 1); break;
    case 'follow': case 'followBack': {
      const nb = S.edges.filter(e => a === 'follow' ? e[0] === S.cardSel : e[1] === S.cardSel).map(e => a === 'follow' ? e[1] : e[0]);
      if (nb.length) { S._fi = ((S._fi || 0) + 1) % nb.length; S.cardSel = nb[a === 'follow' ? S._fi % nb.length : 0]; } break;
    }
    case 'openCard': { const c = S.cards.find(x => x.id === S.cardSel); if (c && c.long) { S.main = 'reading'; S.readHead = 0; } else if (c) toast(`<b>${c.id}</b> · ${esc(c.title)} — ${esc(c.body)}`); break; }
    case 'refCard': refInsert(sc === 'incoming' ? (S.incoming[S.incSel] || {}).id : S.cardSel); break;
    case 'refReport': S.draft = (S.draft ? S.draft + ' ' : '') + '[[F3 SCA comparison]] '; S.mode = 'insert'; S.zone = 'chat'; break;
    case 'newCard':
      picker('New card · kind', ['finding', 'source', 'idea', 'decision', 'question', 'map'].map(k => ({ text: k, html: `<span class="t">${k}</span>`, k })), it => {
        const c = S.cards.find(x => x.id === S.cardSel) || { x: 40, y: 20 };
        const id = it.k[0].toUpperCase() + (S.cards.length + 1);
        S.cards.push({ id, type: it.k, x: c.x + 30, y: c.y + 90, title: 'New ' + it.k, body: 'Type the text (not mocked)' }); S.edges.push([S.cardSel, id]); S.cardSel = id;
      }); break;
    case 'moveCardL': case 'moveCardR': case 'moveCardU': case 'moveCardD': {
      const c = S.cards.find(x => x.id === S.cardSel); if (!c) break;
      const m = { moveCardL: [-40, 0], moveCardR: [40, 0], moveCardU: [0, -40], moveCardD: [0, 40] }[a]; c.x += m[0]; c.y += m[1]; break;
    }
    case 'toIncoming': S.canvasPart = 'incoming'; break;
    case 'toCanvas': S.canvasPart = 'cards'; break;
    case 'deleteCard': S.cards = S.cards.filter(c => c.id !== S.cardSel); S.edges = S.edges.filter(e => !e.includes(S.cardSel)); S.cardSel = (S.cards[0] || {}).id; break;
    case 'zoomIn': S.zoom = Math.min(1.6, S.zoom + .1); break;
    case 'zoomOut': S.zoom = Math.max(.5, S.zoom - .1); break;
    case 'zoomFit': S.zoom = .8; break;
    case 'incLeft': S.incSel = Math.max(0, S.incSel - 1); break;
    case 'incRight': S.incSel = Math.min(S.incoming.length - 1, S.incSel + 1); break;
    case 'accept': {
      const c = S.incoming[S.incSel]; if (!c) break;
      const p = S.cards.find(x => x.id === c.under);
      const kids = S.edges.filter(e => e[0] === c.under).length;
      S.cards.push({ ...c, x: (p ? p.x : 40) + 30 + kids * 18, y: (p ? p.y : 20) + 150 + kids * 26 });
      S.edges.push([c.under, c.id]); S.incoming.splice(S.incSel, 1); S.incSel = Math.max(0, Math.min(S.incSel, S.incoming.length - 1));
      S.cardSel = c.id; if (!S.incoming.length) S.canvasPart = 'cards';
      toast(`Placed <b>${c.id}</b> under ${c.under}`); break;
    }
    case 'drop': S.incoming.splice(S.incSel, 1); S.incSel = Math.max(0, S.incSel - 1); if (!S.incoming.length) S.canvasPart = 'cards'; break;
    // viewer
    case 'slotDown': S.slot = Math.min(VARIANTS.length - 1, S.slot + 1); break;
    case 'slotUp': S.slot = Math.max(0, S.slot - 1); break;
    case 'optNext': case 'optPrev': { const n = VARIANTS[S.slot].options.length; S.choice[S.slot] = (S.choice[S.slot] + (a === 'optNext' ? 1 : n - 1)) % n; break; }
    case 'opt1': case 'opt2': case 'opt3': { const j = +a.slice(3) - 1; if (j < VARIANTS[S.slot].options.length) S.choice[S.slot] = j; break; }
    case 'keepCombo': {
      S.kept = S.choice.slice();
      const desc = VARIANTS.map((v, i) => `${v.slot}: ${v.options[S.choice[i]]}`).join(' · ');
      S.chat.push({ who: 'You', time: '10:05', effort: 'dark', text: `Keep this combination for **header-toggle**: ${desc}` });
      toast('Combination kept and sent to header-toggle through the chat'); break;
    }
    case 'resetCombo': S.choice = S.kept.slice(); break;
    case 'togglePanel': S.panel = S.panel === 'closed' ? 'docked' : 'closed'; break;
    case 'closePanel': S.panel = 'closed'; break;
    case 'detachPanel':
      if (S.main !== 'viewer') { S.main = 'viewer'; S.zone = 'main'; }
      if (S.panel === 'detached') S.panel = 'docked'; else { S.panel = 'detached'; S.panelPos = { x: innerWidth - 820, y: 120 }; }
      break;
    case 'reload': toast('Preview reloaded'); break;
    case 'browser': toast('Opens in your browser (not mocked)'); break;
    case 'nextHeading': case 'prevHeading': { const n = document.querySelectorAll('#reading [data-h]').length; S.readHead = Math.max(0, Math.min(n - 1, S.readHead + (a === 'nextHeading' ? 1 : -1))); break; }
    // leader leaves
    case 'picker': picker('Everything · efforts, agents, inbox, cards', [
      ...EFFORTS.map(e => ({ text: 'effort ' + e.name, html: `${tagHtml(e.id)}<span class="t dim">effort</span>`, go: () => switchEffort(e.id) })),
      ...EFFORTS.flatMap(e => e.tasks.flatMap(t => t.agents.map(ag => ({ text: 'agent ' + ag.name, html: `<span>◆ ${esc(ag.name)}</span><span class="t dim">${esc(ag.branch)} · ${esc(e.name)}</span>`, go: () => toast(`Opens ${ag.name} (not mocked)`) })))),
      ...S.inbox.map(it => ({ text: 'inbox ' + it.title, html: `<span class="g-${it.kind}">${it.glyph}</span><span class="t">${esc(it.title)}</span>`, go: () => { S.inboxSel = S.inbox.indexOf(it); S.zone = 'inbox'; S.side = 'inbox'; } })),
      ...S.cards.map(c => ({ text: 'card ' + c.id + ' ' + c.title, html: `<span class="dim">${c.id}</span><span class="t">${esc(c.title)}</span><span class="dim">${c.type}</span>`, go: () => { switchEffort('pay', true); S.main = 'canvas'; S.zone = 'main'; S.cardSel = c.id; } })),
    ], it => it.go()); break;
    case 'focusChat': S.zone = 'chat'; break;
    case 'focusInbox': setZone('inbox'); break;
    case 'toggleSide': S.sideCollapsed = !S.sideCollapsed; if (S.sideCollapsed && (S.zone === 'inbox' || S.zone === 'tree')) S.zone = 'chat'; break;
    case 'focusTree': setZone('tree'); break;
    case 'pickEffort': picker('Switch effort', EFFORTS.map(e => ({ text: e.name + ' ' + e.kind, html: `${tagHtml(e.id)}<span class="t dim">${esc(e.goal)}</span>`, id: e.id })), it => switchEffort(it.id)); break;
    case 'newEffort': S.modal = { type: 'form', html: formHtml('New effort in Shop', [['Kind', 'feature · fix · cleanup · research · experiment'], ['Question or goal', 'e.g. Does a 10 % first-order coupon raise sign-ups?'], ['Acceptance criteria', 'one per line; ticked only after you approve them in the inbox'], ['Canvas framework', 'question tree · options compare · free board · none']]) }; break;
    case 'criteria': act('wwi'); return;
    case 'openCanvas': S.main = 'canvas'; S.zone = 'main'; break;
    case 'openViewer': S.main = 'viewer'; S.zone = 'main'; break;
    case 'openReading': S.main = 'reading'; S.zone = 'main'; S.readHead = 0; break;
    case 'closeMain': S.main = null; if (S.zone === 'main') S.zone = 'chat'; break;
    case 'pickProject': picker('Switch project', PROJECTS.map((p, i) => ({ text: p.name, html: `<span class="pname">${esc(p.name)}</span><span class="t dim">${esc(p.repos.join(', ') || 'research')}</span><span class="dim">${esc(p.account)}</span>`, i })), it => { S.homeSel = it.i; act('openProject', it.i); }); break;
    case 'pickAgent': act('picker'); S.modal.q = 'agent '; break;
    case 'newSlice': toast('New slice: picks repo and task, makes the worktree (not mocked)'); break;
    case 'interrupt': toast('Interrupt the agent in focus (not mocked)'); break;
    case 'permMode': toast('Permission mode: auto → acceptEdits → manual'); break;
    // insert
    case 'send': {
      const t = S.draft.trim(); if (!t) break;
      S.chat.push({ who: 'You', time: '10:04', effort: S.tag, text: t });
      S.chat.push({ who: 'coordinator', time: '10:04', effort: S.tag, text: `Noted for **${effortById(S.tag).name}**. *(The mock's coordinator only echoes.)*` });
      S.draft = ''; S.wwi = false; S.chatCur = -1; break;
    }
    case 'newline': S.draft += '\n'; break;
    case 'cycleTag': { const i = EFFORTS.findIndex(e => e.id === S.tag); S.tag = EFFORTS[(i + 1) % EFFORTS.length].id; break; }
    // mouse-only helpers
    case 'fw': toast('Starts the canvas from that framework (not mocked)'); break;
    default: toast('Not mocked: ' + a);
  }
  render();
}

function runCommand(v) {
  v = v.trim();
  if (v === 'q' || v === 'home') act('home');
  else if (v.startsWith('effort ')) { const e = EFFORTS.find(x => x.name.toLowerCase().startsWith(v.slice(7).toLowerCase())); if (e) switchEffort(e.id); }
  else if (v === 'canvas' || v === 'viewer' || v === 'reading') act('open' + v[0].toUpperCase() + v.slice(1));
  else toast(`:${esc(v)} — commands in the mock: q, home, effort &lt;name&gt;, canvas, viewer, reading`);
}

function formHtml(title, fields) {
  return `<div class="ttl" style="font-size:1em;color:var(--accent);padding-bottom:8px">${esc(title)}</div>
    ${fields.map(([k, v]) => `<div class="row" style="display:block;white-space:normal"><div class="dim" style="font-size:.85em">${esc(k)}</div><div>${esc(v)}</div></div>`).join('')}
    <div class="row dim" style="padding:12px 16px">A form in the real app; <kbd>Esc</kbd> closes this sketch.</div>`;
}

/* ---------------- key dispatch ---------------- */
function tok(e: KeyboardEvent): string | null {
  let k = e.key;
  const arrows = { ArrowDown: 'j', ArrowUp: 'k', ArrowLeft: 'h', ArrowRight: 'l' };
  if (k === ' ') k = '<Space>';
  else if (k === 'Enter') k = e.altKey ? '<A-CR>' : '<CR>';
  else if (k === 'Escape') k = '<Esc>';
  else if (k === 'Tab') k = e.shiftKey ? '<S-Tab>' : '<Tab>';
  else if (arrows[k] && S.mode === 'normal') k = arrows[k];
  else if (k.length > 1) return null;
  if (e.ctrlKey && k.length === 1) k = 'C-' + k.toLowerCase();
  return k;
}

function onKey(e: KeyboardEvent): void {
  if (e.key === 'F2') { e.preventDefault(); DS.open = !DS.open; saveDesign(); render(); return; }
  const k = tok(e); if (!k) return;
  // modal
  if (S.modal) {
    if (k === '<Esc>') { S.modal = null; render(); e.preventDefault(); return; }
    if (S.modal.type === 'form') { e.preventDefault(); return; }
    if (S.modal.type === 'picker') {
      const items = pickerItems();
      if (k === 'C-j' || e.key === 'ArrowDown') { S.modal.sel = Math.min(items.length - 1, S.modal.sel + 1); renderModal(); e.preventDefault(); return; }
      if (k === 'C-k' || e.key === 'ArrowUp') { S.modal.sel = Math.max(0, S.modal.sel - 1); renderModal(); e.preventDefault(); return; }
      if (k === '<CR>') { const it = items[S.modal.sel]; const f = S.modal.onPick; S.modal = null; if (it) f(it); render(); e.preventDefault(); return; }
      setTimeout(() => { const q = $('#pq'); if (q && q.value !== S.modal?.q) { S.modal.q = q.value; S.modal.sel = 0; renderModal(); } }, 0);
      return;
    }
    if (S.modal.type === 'prompt') {
      if (k === '<CR>') { const v = $('#pq').value; const f = S.modal.onDone; S.modal = null; if (v.trim()) f(v); render(); e.preventDefault(); }
      return;
    }
  }
  // overlay
  if (S.overlay) {
    e.preventDefault();
    const names = Object.keys(SCOPES), layers = ['plain', 'shift', 'ctrl', 'leader'];
    if (k === '<Esc>' && S.ovLayer === 'leader' && S.ovLeader.length) S.ovLeader.pop();
    else if (k === '<Esc>' || k === '?') S.overlay = false;
    else if (k === ']') S.ovScope = names[(names.indexOf(S.ovScope) + 1) % names.length];
    else if (k === '[') S.ovScope = names[(names.indexOf(S.ovScope) + names.length - 1) % names.length];
    else if (k === '<Tab>' || k === '<S-Tab>') { S.ovLayer = layers[(layers.indexOf(S.ovLayer) + (k === '<Tab>' ? 1 : 3)) % 4]; S.ovLeader = []; }
    else if (S.ovLayer === 'leader') { const n = leaderNode(S.ovLeader).keys[k]; if (n && !Array.isArray(n)) S.ovLeader.push(k); }
    render(); return;
  }
  // insert mode
  if (S.mode === 'insert') {
    const b = KEYMAP.insert.find(x => x[0] === k && x[1] !== 'none');
    if (b) { e.preventDefault(); const ta = $('#input'); if (ta) S.draft = ta.value; act(b[1]); return; }
    if (k === 'C-h' || k === 'C-l') { e.preventDefault(); S.mode = 'normal'; act(k === 'C-h' ? 'zoneLeft' : 'zoneRight'); }
    if (k === 'C-w' || k === 'C-u') { e.preventDefault(); const ta = $('#input'); S.draft = k === 'C-u' ? '' : ta.value.replace(/\S+\s*$/, ''); ta.value = S.draft; }
    return; // let the textarea type
  }
  // normal mode
  if (e.target && e.target.tagName === 'TEXTAREA') e.target.blur();
  e.preventDefault();
  if (S.leader) {
    if (k === '<Esc>') { S.leader = S.leader.length ? null : null; render(); return; }
    const n = leaderNode(S.leader).keys[k];
    if (!n) { S.leader = null; toast(`Space … ${esc(k)} is not bound`); render(); return; }
    if (Array.isArray(n)) { S.leader = null; act(n[0]); return; }
    S.leader.push(k); render(); return;
  }
  if (k === '<Space>' && !S.pending && S.screen === 'ws') { S.leader = []; render(); return; }
  const scope = scopeNow();
  const binds = bindingsFor(scope);
  const tryKey = seq => {
    const exact = binds.find(b => b[0] === seq);
    const prefix = binds.some(b => b[0] !== seq && b[0].startsWith(seq) && b[1] !== 'none');
    if (prefix) { S.pending = seq; render(); return true; }
    if (exact) { S.pending = ''; act(exact[1]); return true; }
    return false;
  };
  if (tryKey(S.pending + k)) return;
  const had = S.pending; S.pending = '';
  if (had && tryKey(k)) return;
  render();
}

/* ---------------- mouse ---------------- */
document.addEventListener('click', e => {
  const el = e.target.closest('[data-a]'); if (!el) return;
  const a = el.dataset.a; e.stopPropagation();
  switch (a) {
    case 'zone': setZone(el.dataset.z); S.mode = 'normal'; break;
    case 'selInbox': S.inboxSel = +el.dataset.i; setZone('inbox'); if (e.detail === 2) return act('openItem'); break;
    case 'selTree': S.treeSel = +el.dataset.i; setZone('tree'); if (e.detail === 2) return act('openRow'); break;
    case 'selCard': S.cardSel = el.dataset.id; S.canvasPart = 'cards'; S.zone = 'main'; if (e.detail === 2) return act('openCard'); break;
    case 'selInc': S.incSel = S.incoming.findIndex(c => c.id === el.dataset.id); S.canvasPart = 'incoming'; S.zone = 'main'; break;
    case 'refCardBtn': return refInsert(el.dataset.id), render();
    case 'selSlot': S.slot = +el.dataset.i; S.zone = 'main'; break;
    case 'pickOpt': S.slot = +el.dataset.i; S.choice[S.slot] = +el.dataset.j; S.zone = 'main'; break;
    case 'main': S.main = el.dataset.m; S.zone = 'main'; break;
    case 'pickFilter': return act('pickFilter');
    case 'railEffort': switchEffort(el.dataset.id); break;
    case 'setFilter': S.filter = el.dataset.f; if (S.filter !== 'all') switchEffort(S.filter, true); S.chatCur = -1; break;
    case 'toggleReport': { const i = +el.dataset.i; S.openReports.has(i) ? S.openReports.delete(i) : S.openReports.add(i); break; }
    case 'ovScope': S.ovScope = el.dataset.s; break;
    case 'ovLayer': S.ovLayer = el.dataset.l; S.ovLeader = []; break;
    case 'ovDrill': S.ovLeader.push(el.dataset.k); break;
    case 'ovUp': S.ovLeader.pop(); break;
    case 'pickRow': { const it = pickerItems()[+el.dataset.i]; const f = S.modal.onPick; S.modal = null; f(it); break; }
    case 'openProject': return act('openProject', el.dataset.i);
    case 'designPick': DS.choice[el.dataset.s] = el.dataset.o; saveDesign(); break;
    case 'designClose': case 'designToggle': DS.open = !DS.open; saveDesign(); break;
    case 'designKeep': { const t = designCombo(); navigator.clipboard?.writeText(t).catch(() => {}); toast('Copied: paste it in the chat to keep this combination'); return; }
    default: return act(a);
  }
  render();
});
document.addEventListener('keydown', onKey);

// Keep where you were across the dev server's auto-reload (per tab, best effort).
const KEEP = ['screen', 'zone', 'side', 'main', 'mainItem', 'effort', 'filter', 'tag', 'wwi', 'cardSel', 'zoom', 'slot', 'choice', 'kept', 'panel', 'panelPos', 'homeSel', 'sideW', 'sideCollapsed'];
try { const saved = JSON.parse(sessionStorage.getItem('dovetail-mock') || '{}'); for (const k of KEEP) if (k in saved) S[k] = saved[k]; } catch { /* no storage */ }
addEventListener('beforeunload', () => { try { sessionStorage.setItem('dovetail-mock', JSON.stringify(Object.fromEntries(KEEP.map(k => [k, S[k]])))); } catch { /* no storage */ } });
render();
