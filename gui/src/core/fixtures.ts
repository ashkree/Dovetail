// The fake data MockCore serves, ported from mock/gui/data.ts. Nothing here is real.
// Only the mock imports this file; components never do.
//
// It is written in the view's terms (model.ts), as the screens show it, and turned into the
// core's terms at the bottom: a core Snapshot (generated types) for what dovetail-api serves,
// and the pending state for what it does not serve yet (variants, reports).

import type * as W from './generated';
import type {
  Agent, ChatEntry, Effort, InboxItem, ProjectSummary, Report, Task, VariantSet,
} from './model';
import { cardViewId } from './canvasView';
import { emptyPending, type Pending } from './pending';
import { COORDINATOR, UNSORTED_EFFORT, emptyCoreSnapshot } from './wire';

/** Home's projects; `days` ago it was last opened. */
interface FixtureProject extends Omit<ProjectSummary, 'opened'> { days: number }
export const PROJECTS: FixtureProject[] = [
  { id: 'shop', name: 'Shop', repos: ['shop-api', 'shop-web'], account: 'work', efforts: 5, running: 5, needs_you: 4, days: 0 },
  { id: 'dovetail', name: 'Dovetail', repos: ['dovetail'], account: 'personal', efforts: 3, running: 2, needs_you: 1, days: 1 },
  { id: 'garden', name: 'Garden planner', repos: ['garden-app', 'garden-data'], account: 'personal', efforts: 2, running: 0, needs_you: 0, days: 4 },
  { id: 'thesis', name: 'Thesis notes', repos: [], account: 'personal', efforts: 4, running: 1, needs_you: 0, days: 7 },
  { id: 'homelab', name: 'Home lab', repos: ['infra', 'dotfiles', 'backups'], account: 'personal', efforts: 1, running: 0, needs_you: 0, days: 19 },
];

const EFFORTS: Effort[] = [
  {
    id: 'pay', name: 'payment-providers', kind: 'research', colour: 'blue', running: true,
    question_or_goal: 'Which payment provider should Shop use for the EU and the US?',
    criteria: [
      { id: 'pay-c1', text: 'At least three providers compared on fees, payouts and SCA', state: 'met' },
      { id: 'pay-c2', text: 'A recommendation with reasons and the risks', state: 'open' },
      { id: 'pay-c3', text: 'Checkout prototype with the chosen provider in shop-web', state: 'open' },
    ],
    where_was_i: {
      last: 'Fees for Stripe, Adyen and Mollie collected; SCA comparison report written.',
      waits: 'Question from **research-1**: include Adyen even though pricing needs a sales call?',
      next: 'Answer the question, then read the SCA report and pick a direction on the canvas.',
    },
  },
  {
    id: 'login', name: 'login-bug', kind: 'fix', colour: 'coral', running: true,
    question_or_goal: 'Users on Safari are logged out after about five minutes.',
    criteria: [
      { id: 'login-c1', text: 'A test reproduces the logout and fails before the fix', state: 'met' },
      { id: 'login-c2', text: 'Fix passes in Safari and Chrome', state: 'claimed' },
      { id: 'login-c3', text: 'No regressions in the session tests', state: 'open' },
    ],
    where_was_i: {
      last: 'login-fix set `SameSite=None; Secure` on the refresh cookie; e2e green in Chrome.',
      waits: 'Approve **npm run e2e -- --browser safari**, and review the criterion “Fix passes in Safari and Chrome”.',
      next: 'Once Safari passes, ask login-fix to open the PR on shop-api.',
    },
  },
  {
    id: 'dark', name: 'dark-mode', kind: 'feature', colour: 'lilac', running: true,
    question_or_goal: 'A dark theme for shop-web that users can switch on from the header.',
    criteria: [
      { id: 'dark-c1', text: 'Toggle in the header, choice kept across visits', state: 'open' },
      { id: 'dark-c2', text: 'Every page passes 4.5:1 contrast', state: 'open' },
      { id: 'dark-c3', text: 'No light flash on load', state: 'open' },
    ],
    where_was_i: {
      last: 'Theme tokens merged; header-toggle built three header variants and two hero lines.',
      waits: 'header-toggle finished a turn: pick a header and hero combination in the viewer.',
      next: 'Choose a combination, “Keep this combination”, then header-toggle audits the pages.',
    },
  },
  {
    id: 'search', name: 'search-index', kind: 'cleanup', colour: 'mint', running: true,
    question_or_goal: 'Drop the old search index code from shop-api.',
    criteria: [{ id: 'search-c1', text: 'Old index module removed, build and tests green', state: 'open' }],
    where_was_i: { last: 'search-gc removed the writer side.', waits: 'Nothing.', next: 'Remove the reader side and the feature flag.' },
  },
  {
    id: 'coupon', name: 'coupon-ab', kind: 'experiment', colour: 'gold', running: true,
    question_or_goal: 'Does a 10 % first-order coupon raise sign-ups?',
    criteria: [{ id: 'coupon-c1', text: 'Flagged experiment ships to 10 % of visitors', state: 'open' }],
    where_was_i: { last: 'Effort created; no agents yet.', waits: 'Nothing.', next: 'Ask the coordinator to plan the experiment.' },
  },
];

const TASKS: Task[] = [
  { id: 'pay-t1', effort_id: 'pay', title: 'Gather fee data', status: 'done' },
  { id: 'pay-t2', effort_id: 'pay', title: 'Compare SCA support', status: 'review' },
  { id: 'pay-t3', effort_id: 'pay', title: 'Write recommendation', status: 'todo' },
  { id: 'login-t1', effort_id: 'login', title: 'Reproduce on Safari', status: 'done' },
  { id: 'login-t2', effort_id: 'login', title: 'Fix refresh cookie', status: 'in_progress' },
  { id: 'login-t3', effort_id: 'login', title: 'Regression test', status: 'todo' },
  { id: 'dark-t1', effort_id: 'dark', title: 'Theme tokens', status: 'done' },
  { id: 'dark-t2', effort_id: 'dark', title: 'Header toggle', status: 'in_progress' },
  { id: 'dark-t3', effort_id: 'dark', title: 'Audit pages', status: 'todo' },
  { id: 'search-t1', effort_id: 'search', title: 'Remove old index', status: 'in_progress' },
  { id: 'coupon-t1', effort_id: 'coupon', title: 'Plan the experiment', status: 'todo' },
];

const AGENTS: Agent[] = [
  { id: 'research-1', task_id: 'pay-t2', kind: 'slice', name: 'research-1', state: 'needs_you' },
  { id: 'fees-web', task_id: 'pay-t2', parent_id: 'research-1', kind: 'subagent', name: 'fees-web', state: 'done' },
  { id: 'login-fix', task_id: 'login-t2', kind: 'slice', name: 'login-fix', state: 'needs_you', branch: 'fix/login-safari' },
  { id: 'run-e2e', task_id: 'login-t2', parent_id: 'login-fix', kind: 'subagent', name: 'run e2e', state: 'working' },
  { id: 'header-toggle', task_id: 'dark-t2', kind: 'slice', name: 'header-toggle', state: 'working', branch: 'feat/dark-mode' },
  { id: 'search-gc', task_id: 'search-t1', kind: 'slice', name: 'search-gc', state: 'working', branch: 'chore/drop-old-index' },
];

const INBOX: InboxItem[] = [
  {
    id: 'i1', kind: 'permission', effort_id: 'login', agent_id: 'login-fix', title: 'Bash · npm run e2e -- --browser safari', at: '2m',
    body: '**login-fix** wants to run a command in `~/…/worktrees/shop-api/fix-login-safari`:\n\n```\nnpm run e2e -- --browser safari\n```\n\nIt runs the e2e suite against a local Safari WebDriver. No network beyond localhost.',
  },
  {
    id: 'i2', kind: 'criterion', effort_id: 'login', agent_id: 'login-fix', criterion_id: 'login-c2', title: 'Criterion · Fix passes in Safari and Chrome', at: '2m',
    body: '**Acceptance criterion:** Fix passes in Safari and Chrome\n\n**Evidence from login-fix:**\n\n- Chrome e2e: 48 passed, 0 failed (`e2e/chrome-2026-11-15.log`)\n- Safari e2e: *waiting for the run you are asked to approve*\n- Change: `src/auth/refresh.rs` sets `SameSite=None; Secure`\n\nApprove only when you are satisfied. Dovetail ticks the criterion on approval.',
  },
  {
    id: 'i3', kind: 'question', effort_id: 'pay', agent_id: 'research-1', title: 'Include Adyen? Its pricing needs a sales call', at: '14m',
    body: '**research-1 asks:** Adyen publishes no price list for our volume; getting one needs a sales call. Include it in the comparison with *estimated* fees, or drop it?',
  },
  {
    id: 'i4', kind: 'finished', effort_id: 'dark', agent_id: 'header-toggle', title: 'Header variants ready to view', at: '31m',
    body: 'header-toggle built three header variants, two footers and three hero lines. Open the viewer to mix and match.',
  },
];

const CHAT: ChatEntry[] = [
  { id: 'm1', who: 'you', report: false, at: '09:14', effort_id: 'login', text: 'What is left on the Safari logout?' },
  { id: 'm2', who: 'coordinator', report: false, at: '09:14', effort_id: 'login', text: '**login-fix** found it: the refresh cookie had `SameSite=Lax`, so Safari dropped it on the cross-site token refresh.\n\n- Fix is in `src/auth/refresh.rs`\n- Chrome e2e is green\n- Safari run is waiting for your approval in the inbox' },
  { id: 'm3', who: 'research-1', report: true, at: '09:20', effort_id: 'pay', text: 'SCA comparison written · 3 providers · 9 sources · report ready to read' },
  { id: 'm4', who: 'you', report: false, at: '09:31', effort_id: 'pay', text: 'Summarise the SCA findings in two lines.' },
  { id: 'm5', who: 'coordinator', report: false, at: '09:31', effort_id: 'pay', text: '**Stripe and Adyen** handle 3DS2 and exemptions for us; **Mollie** does too but has *no US acquiring*, so it fails the US half of the question.\n\nThe full report is on the canvas as card **F7**.' },
  { id: 'm6', who: 'header-toggle', report: true, at: '09:44', effort_id: 'dark', text: 'Header variants ready · 3 headers · 2 footers · 3 hero lines' },
  { id: 'm7', who: 'search-gc', report: true, at: '09:50', effort_id: 'search', text: 'Writer side removed · 14 files · tests green' },
  { id: 'm8', who: 'you', report: false, at: '09:52', effort_id: 'dark', text: 'Keep the toggle as an icon, not a text button.' },
  { id: 'm9', who: 'coordinator', report: false, at: '09:52', effort_id: 'dark', text: 'Told **header-toggle**. Variant *B* of the header already uses an icon; the viewer has it.' },
];

// Question-tree canvas for "payment-providers", in the core's terms (ADR 0025): the core's card
// number is the number of the view id (`Q1`, `F5`), so the fixture says it once. Cards that point
// at something carry a made-up reference; their bodies are what `load_card` answers. Canvas
// coordinates are px.
const PAY_EFFORT = 2; // EFFORT_ID.get('pay')
interface FixtureCard {
  id: number; kind: W.CardKind; x?: number; y?: number; title: string; body: string;
  /** Opens this report in the reading view. */
  report?: string;
}
const FIXTURE_CARDS: FixtureCard[] = [
  { id: 1, kind: 'question', x: 360, y: 20, title: 'Which provider for the EU and the US?', body: 'Root question of the effort.' },
  { id: 2, kind: 'question', x: 40, y: 170, title: 'Fees at our volume?', body: '~2 400 orders a month, €38 average.' },
  { id: 3, kind: 'question', x: 360, y: 170, title: 'SCA / 3DS handled for us?', body: 'Who runs the challenge flow and exemptions.' },
  { id: 4, kind: 'question', x: 680, y: 170, title: 'Payouts and currencies?', body: 'EUR and USD payouts, timing.' },
  { id: 5, kind: 'finding', x: 40, y: 330, title: 'Stripe 1.5 % + €0.25 EU cards', body: 'Cheapest published rate for EU cards at our volume.' },
  { id: 6, kind: 'source', x: 40, y: 470, title: 'stripe.com/pricing', body: 'Fetched 15 Nov.' },
  { id: 7, kind: 'finding', x: 360, y: 330, title: 'SCA comparison (report)', body: 'Stripe and Adyen handle 3DS2 and exemptions; Mollie has no US acquiring.', report: 'r-sca' },
  { id: 8, kind: 'idea', x: 680, y: 330, title: 'Use Stripe for both, revisit Adyen at 10k orders', body: 'Keeps one integration now.' },
  { id: 9, kind: 'decision', x: 360, y: 470, title: 'Decision (open)', body: 'Not decided yet. Needs the recommendation criterion.' },
  { id: 10, kind: 'map', x: 680, y: 470, title: 'Checkout flow map', body: 'Where the provider plugs into shop-web.' },
  // Incoming: no position.
  { id: 11, kind: 'finding', title: 'Mollie: no US acquiring', body: 'From research-1, 2 sources.' },
  { id: 12, kind: 'source', title: 'adyen.com/pricing (no list price)', body: 'Fetched 15 Nov.' },
  { id: 13, kind: 'question', title: 'Chargeback fees?', body: 'Proposed by the coordinator.' },
];
const FIXTURE_LINKS: [number, number][] = [
  [1, 2], [1, 3], [1, 4], [2, 5], [5, 6], [3, 7], [4, 8], [7, 9], [8, 10],
  // Where the incoming cards would hang.
  [3, 11], [2, 12], [2, 13],
];
const isIncoming = (c: FixtureCard) => c.x === undefined;

/** The mock's card bodies, by core card id: what `load_card` answers for a card that points at something. */
export const CARD_BODIES: Record<number, string> = Object.fromEntries(
  FIXTURE_CARDS.filter(c => !['idea', 'question'].includes(c.kind)).map(c => [c.id, c.body]),
);

function coreCanvasState(): Pick<W.Snapshot, 'canvases' | 'cards' | 'card_links'> {
  return {
    canvases: [{ id: 1, effort: PAY_EFFORT, title: 'Which provider?', framework: 'question_tree' }],
    cards: FIXTURE_CARDS.map((c): W.Card => {
      const own = c.kind === 'idea' || c.kind === 'question';
      return {
        id: c.id, effort: PAY_EFFORT, ...(isIncoming(c) ? {} : { canvas: 1 }), kind: c.kind, x: c.x ?? 0, y: c.y ?? 0,
        ...(own ? {} : { reference: { kind: c.kind as W.RefKind, id: String(c.id) } }),
        title: c.title, text: own ? `${c.title}\n${c.body}` : '', missing: false,
        added_by: isIncoming(c) ? 'research-1' : 'user', created_ms: NOW,
      };
    }),
    card_links: FIXTURE_LINKS.map(([from, to]) => ({ from, to, label: '' })),
  };
}

// Viewer variants for "dark-mode", three levels.
const VARIANTS: VariantSet[] = [{
  agent_id: 'header-toggle', effort_id: 'dark', url: 'localhost:5173/',
  slots: [
    { id: 'theme', level: 'global', name: 'Theme', options: ['Dusk', 'Midnight', 'Soft dark'] },
    { id: 'layout', level: 'global', name: 'Layout', options: ['Grid', 'List'] },
    { id: 'header', level: 'component', name: 'Header', options: ['A · centred, text toggle', 'B · left logo, icon toggle', 'C · compact'] },
    { id: 'footer', level: 'component', name: 'Footer', options: ['Slim', 'Full'] },
    { id: 'hero', level: 'content', name: 'Hero line', options: ['Find something good.', 'New in: winter.', 'Made to last.'] },
  ],
  kept: [0, 0, 0, 0, 0],
}];

const REPORT = `# SCA support compared

**Question:** which of Stripe, Adyen and Mollie run Strong Customer Authentication for us, in the EU and the US? [F3]

## Short answer

**Stripe and Adyen** run the 3DS2 challenge flow and apply exemptions (low value, trusted beneficiary) themselves. **Mollie** does the same in the EU but has *no US acquiring*, so it cannot be the only provider. [S2, S3]

## Stripe

- 3DS2 through Payment Intents; the challenge opens in Stripe's own frame
- Exemptions requested automatically under *Radar* rules
- Works with \`shop-web/src/checkout/pay.tsx\` as it is today

## Adyen

- 3DS2 native and redirect flows
- Exemptions on request; *pricing needs a sales call* (question in your inbox)

## Mollie

- 3DS2 handled; **EU only**
- Cheapest for iDEAL and Bancontact

## Risks

1. Adyen's real fees are unknown until the call.
2. A two-provider setup doubles the webhook code in \`shop-api/src/payments/\`.
`;
const REPORTS: Report[] = [{ id: 'r-sca', effort_id: 'pay', title: 'SCA support compared', markdown: REPORT }];

const MACHINE_LIMIT = 4; // soft limit on running efforts on this machine

// ---- the core's terms -------------------------------------------------------------------------

/** The mock's clock: "now" is when the page loaded. */
const NOW = Date.now();
const MIN = 60_000;

/** Today at "09:14", in Unix ms. */
function today(hhmm: string): number {
  const [h, m] = hhmm.split(':').map(Number);
  const d = new Date(NOW);
  d.setHours(h ?? 0, m ?? 0, 0, 0);
  return d.getTime();
}

/** Core ids for Shop: Unsorted is effort 1, the fixtures' efforts follow in order. */
const EFFORT_ID = new Map(EFFORTS.map((e, i) => [e.id, i + 2]));
const TASK_ID = new Map(TASKS.map((t, i) => [t.id, i + 1]));
const CRITERION_ID = new Map(EFFORTS.flatMap(e => e.criteria).map((c, i) => [c.id, i + 1]));
const num = (map: Map<string, number>, id: string): number => {
  const n = map.get(id);
  if (n === undefined) throw new Error(`fixture id ${id} has no core id`);
  return n;
};

const coreProject = (p: FixtureProject, i: number): W.ProjectSummary => ({
  id: p.id, name: p.name,
  repo_path: p.repos.length ? `/home/you/code/${p.repos[0]}` : `/home/you/.local/share/dovetail/projects/${p.id}`,
  kind: p.repos.length ? 'code' : 'research',
  repos: p.repos.map((name, j) => ({ id: (i + 1) * 10 + j, name, path: `/home/you/code/${name}` })),
  account: p.account || null,
  last_opened_ms: NOW - p.days * 86_400_000 - 60_000,
  counts: { efforts: p.efforts, running: p.running, needs_you: p.needs_you },
});

function coreAgent(id: string, name: string, kind: W.AgentKind, task: number | null, status: W.AgentStatus): W.Agent {
  return {
    id, name, kind, parent: null, task, worktree: null, status, status_line: '', memory_mb: null, cost_usd: 0,
    permission_mode: 'auto', context_pct: null, progress: null, last_activity_ms: NOW, lock: 'headless',
    subagent_tokens: 0,
  };
}

const CORE_STATUS: Record<Agent['state'], W.AgentStatus> = {
  working: 'working', needs_you: 'needs_you', done: 'done', idle: 'idle',
};

function coordinator(project: FixtureProject, busy: boolean): W.Agent {
  return {
    ...coreAgent(COORDINATOR, 'coordinator', 'coordinator', null, busy ? 'working' : 'idle'),
    context_pct: 62, model: 'claude-opus-5-5', effort: 'medium',
    ...(project.id === 'shop' ? {} : { context_pct: 0 }),
  };
}

const UNSORTED: W.Effort = {
  id: UNSORTED_EFFORT, name: 'unsorted', kind: 'cleanup', question_or_goal: 'Tasks not sorted into an effort yet',
  status: 'open', where_was_i: { last_done: '', waits_on_you: '', next_step: '' }, criteria: [],
};

function coreEfforts(): W.Effort[] {
  // The criterion waiting for review (inbox item i2) is claimed, with its evidence.
  const evidence = INBOX.find(i => i.kind === 'criterion')?.body.split('\n\n').slice(1, 3).join('\n\n') ?? null;
  return [UNSORTED, ...EFFORTS.map((e): W.Effort => {
    const id = num(EFFORT_ID, e.id);
    return {
      id, name: e.name, kind: e.kind, question_or_goal: e.question_or_goal, status: 'open',
      where_was_i: { last_done: e.where_was_i.last, waits_on_you: e.where_was_i.waits, next_step: e.where_was_i.next },
      criteria: e.criteria.map(c => ({
        id: num(CRITERION_ID, c.id), effort: id, text: c.text, state: c.state,
        ...(c.state === 'claimed' ? { evidence } : {}),
      })),
    };
  })];
}

function coreTasks(): W.Task[] {
  return TASKS.map(t => ({
    id: num(TASK_ID, t.id), title: t.title, status: t.status, p_level: null, bucket: null, order: 0, deps: [],
    notes: '', pr: null, follow_up_of: null, coordinator_created: false, related: [],
    effort: num(EFFORT_ID, t.effort_id),
  }));
}

function coreAgents(project: FixtureProject): W.Agent[] {
  return [coordinator(project, true), ...AGENTS.map(a => ({
    ...coreAgent(a.id, a.name, a.kind === 'slice' ? 'slice' : { subagent: { background: false } },
      a.parent_id ? null : num(TASK_ID, a.task_id), CORE_STATUS[a.state]),
    parent: a.parent_id ?? null,
    worktree: a.branch ? { path: `/home/you/.local/share/dovetail/projects/shop/worktrees/${a.id}`, branch: a.branch } : null,
  }))];
}

/** The inbox as the core holds it; the criterion review is the claimed criterion instead. */
function coreInbox(): W.InboxItem[] {
  const ago = (m: number) => NOW - m * MIN;
  return [
    {
      id: 1, agent: 'login-fix', at_ms: ago(2), kind: { permission: {
        tool: 'Bash', input: { command: 'npm run e2e -- --browser safari' }, suggestions: [],
        reason: 'It runs the e2e suite against a local Safari WebDriver. No network beyond localhost.',
      } },
    },
    {
      id: 3, agent: 'research-1', at_ms: ago(14), kind: { question: {
        text: 'Adyen publishes no price list for our volume; getting one needs a sales call. Include it in the comparison with *estimated* fees, or drop it?',
      } },
    },
    {
      id: 4, agent: 'header-toggle', at_ms: ago(31), kind: { finished_turn: {
        preview: 'header-toggle built three header variants, two footers and three hero lines. Open the viewer to mix and match.',
      } },
    },
  ];
}

function coreChat(): W.ConversationEntry[] {
  return CHAT.map((m, i): W.ConversationEntry => {
    const effort = m.effort_id ? { effort: num(EFFORT_ID, m.effort_id) } : {};
    const base = { id: i + 1, at_ms: today(m.at), ...effort };
    if (m.who === 'you') return { ...base, kind: 'user', text: m.text };
    if (m.who === 'coordinator') return { ...base, kind: 'assistant', text: m.text };
    const name = AGENTS.find(a => a.id === m.who)?.name ?? m.who;
    return { ...base, kind: 'agent_message', text: `agent ${name}\n${m.text}` };
  });
}

/** The core's Snapshot: Home (no project) or one project. Only Shop has data. */
export function coreSnapshot(seq: number, projectId: string | null): W.Snapshot {
  const base: W.Snapshot = { ...emptyCoreSnapshot(), seq, now_ms: NOW, projects: PROJECTS.map(coreProject) };
  const index = PROJECTS.findIndex(p => p.id === projectId);
  const project = PROJECTS[index];
  if (!project) return base;
  const opened: W.Snapshot = {
    ...base, project: coreProject(project, index), profile: project.account,
    usage: {
      five_hour: { utilization: 0.34, resets_at_ms: null, status: 'allowed' },
      seven_day: { utilization: 0.18, resets_at_ms: null, status: 'allowed' },
    },
  };
  if (project.id !== 'shop') return { ...opened, efforts: [UNSORTED], agents: [coordinator(project, false)] };
  return {
    ...opened,
    efforts: coreEfforts(), ...coreCanvasState(), tasks: coreTasks(), agents: coreAgents(project), inbox: coreInbox(),
    conversations: { [COORDINATOR]: { entries: coreChat(), has_more: false } },
  };
}

/** The pending state for a project: Shop's canvas, cards, variants and report, everyone's Home
 * columns. Effort ids are the core's, as text. */
export function pendingFor(projectId: string | null): Pending {
  const effort = (id: string) => String(num(EFFORT_ID, id));
  const base: Pending = {
    ...emptyPending(),
    status: { effort_limit: MACHINE_LIMIT, machine_running: PROJECTS.reduce((n, p) => n + p.running, 0) },
  };
  if (projectId !== 'shop') return base;
  return {
    ...base,
    variants: VARIANTS.map(v => ({ ...v, effort_id: effort(v.effort_id) })),
    reports: REPORTS.map(r => ({ ...r, effort_id: effort(r.effort_id) })),
    colours: Object.fromEntries(EFFORTS.map(e => [effort(e.id), e.colour])),
    card_reports: Object.fromEntries(
      FIXTURE_CARDS.filter(c => c.report).map(c => [cardViewId(c), c.report as string]),
    ),
  };
}
