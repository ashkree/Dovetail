// The fake data MockCore serves, ported from mock/gui/data.ts. Nothing here is real.
// Only MockCore imports this file; components never do.

import type {
  Agent, Canvas, Card, CardLink, ChatEntry, Effort, IncomingCard, InboxItem, ProjectSummary, Report,
  Snapshot, Status, Task, VariantSet,
} from './model';

export const PROJECTS: ProjectSummary[] = [
  { id: 'shop', name: 'Shop', repos: ['shop-api', 'shop-web'], account: 'work', efforts: 5, running: 5, needs_you: 4, opened: 'today 09:12', note: 'Payments, login fix, dark mode' },
  { id: 'dovetail', name: 'Dovetail', repos: ['dovetail'], account: 'personal', efforts: 3, running: 2, needs_you: 1, opened: 'yesterday', note: 'GUI rework' },
  { id: 'garden', name: 'Garden planner', repos: ['garden-app', 'garden-data'], account: 'personal', efforts: 2, running: 0, needs_you: 0, opened: '12 Nov', note: 'Planting calendar' },
  { id: 'thesis', name: 'Thesis notes', repos: [], account: 'personal', efforts: 4, running: 1, needs_you: 0, opened: '9 Nov', note: 'Research only, no repo' },
  { id: 'homelab', name: 'Home lab', repos: ['infra', 'dotfiles', 'backups'], account: 'personal', efforts: 1, running: 0, needs_you: 0, opened: '28 Oct', note: 'Backups to the NAS' },
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
  { id: 'm5', who: 'coordinator', report: false, at: '09:31', effort_id: 'pay', text: '**Stripe and Adyen** handle 3DS2 and exemptions for us; **Mollie** does too but has *no US acquiring*, so it fails the US half of the question.\n\nThe full report is on the canvas as card **F3**.' },
  { id: 'm6', who: 'header-toggle', report: true, at: '09:44', effort_id: 'dark', text: 'Header variants ready · 3 headers · 2 footers · 3 hero lines' },
  { id: 'm7', who: 'search-gc', report: true, at: '09:50', effort_id: 'search', text: 'Writer side removed · 14 files · tests green' },
  { id: 'm8', who: 'you', report: false, at: '09:52', effort_id: 'dark', text: 'Keep the toggle as an icon, not a text button.' },
  { id: 'm9', who: 'coordinator', report: false, at: '09:52', effort_id: 'dark', text: 'Told **header-toggle**. Variant *B* of the header already uses an icon; the viewer has it.' },
];

const CANVASES: Canvas[] = [{ id: 'c-pay', effort_id: 'pay', framework: 'question_tree', title: 'Which provider?' }];

// Question-tree canvas for "payment-providers". Coordinates are in canvas px.
const card = (id: string, kind: Card['kind'], x: number, y: number, title: string, body: string, more: Partial<Card> = {}): Card =>
  ({ id, canvas_id: 'c-pay', kind, x, y, title, body, ...more });
const CARDS: Card[] = [
  card('Q1', 'question', 360, 20, 'Which provider for the EU and the US?', 'Root question of the effort.'),
  card('Q2', 'question', 40, 170, 'Fees at our volume?', '~2 400 orders a month, €38 average.'),
  card('Q3', 'question', 360, 170, 'SCA / 3DS handled for us?', 'Who runs the challenge flow and exemptions.'),
  card('Q4', 'question', 680, 170, 'Payouts and currencies?', 'EUR and USD payouts, timing.'),
  card('F1', 'finding', 40, 330, 'Stripe 1.5 % + €0.25 EU cards', 'Cheapest published rate for EU cards at our volume.', { ref: 'S1' }),
  card('S1', 'source', 40, 470, 'stripe.com/pricing', 'Fetched 15 Nov.'),
  card('F3', 'finding', 360, 330, 'SCA comparison (report)', 'Stripe and Adyen handle 3DS2 and exemptions; Mollie has no US acquiring.', { report_id: 'r-sca' }),
  card('I1', 'idea', 680, 330, 'Use Stripe for both, revisit Adyen at 10k orders', 'Keeps one integration now.'),
  card('D1', 'decision', 360, 470, 'Decision (open)', 'Not decided yet. Needs the recommendation criterion.'),
  card('M1', 'map', 680, 470, 'Checkout flow map', 'Where the provider plugs into shop-web.'),
];
const LINKS: CardLink[] = [['Q1', 'Q2'], ['Q1', 'Q3'], ['Q1', 'Q4'], ['Q2', 'F1'], ['F1', 'S1'], ['Q3', 'F3'], ['Q4', 'I1'], ['F3', 'D1'], ['I1', 'M1']]
  .map(([from, to]) => ({ from: from as string, to: to as string }));
const INCOMING: IncomingCard[] = [
  { id: 'F4', canvas_id: 'c-pay', kind: 'finding', title: 'Mollie: no US acquiring', body: 'From research-1, 2 sources.', under: 'Q3' },
  { id: 'S5', canvas_id: 'c-pay', kind: 'source', title: 'adyen.com/pricing (no list price)', body: 'Fetched 15 Nov.', under: 'Q2' },
  { id: 'Q5', canvas_id: 'c-pay', kind: 'question', title: 'Chargeback fees?', body: 'Proposed by the coordinator.', under: 'Q2' },
];

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

export const STATUS: Status = {
  effort_limit: MACHINE_LIMIT,
  machine_running: PROJECTS.reduce((n, p) => n + p.running, 0),
  account: 'work', context_fill: 0.62, usage: { five_hour: 0.34, week: 0.18 },
  model: 'Opus 5.5', effort_level: 'Medium', permission_mode: 'Auto', busy: true,
};

/** The snapshot for the home screen: every project, none open. */
export function homeSnapshot(seq: number): Snapshot {
  return {
    seq, projects: PROJECTS, open_project: null, efforts: [], tasks: [], agents: [], inbox: [], chat: [],
    canvases: [], cards: [], card_links: [], incoming: [], variants: [], reports: [], status: STATUS,
  };
}

/** The snapshot with one project open. Only Shop has data; the others open empty. */
export function projectSnapshot(seq: number, projectId: string): Snapshot {
  const project = PROJECTS.find(p => p.id === projectId);
  const base = homeSnapshot(seq);
  if (!project) return base;
  if (project.id !== 'shop') return { ...base, open_project: project.id, status: { ...STATUS, account: project.account, busy: false } };
  return {
    ...base, open_project: project.id,
    efforts: EFFORTS, tasks: TASKS, agents: AGENTS, inbox: INBOX, chat: CHAT,
    canvases: CANVASES, cards: CARDS, card_links: LINKS, incoming: INCOMING, variants: VARIANTS, reports: REPORTS,
  };
}
