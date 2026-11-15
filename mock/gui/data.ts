// Fake data for the Dovetail GUI mock. Nothing here is real.

type Kind = 'feature' | 'fix' | 'cleanup' | 'research' | 'experiment';
type TaskStatus = 'To do' | 'In progress' | 'Review' | 'Done';
interface Project { id: string; name: string; repos: string[]; account: string; efforts: number; running: number; inbox: number; opened: string; note: string }
interface Criterion { text: string; state: 'open' | 'review' | 'done' }
interface Agent { name: string; kind?: 'slice'; state: string; branch?: string; subs: Agent[] }
interface Task { title: string; status: TaskStatus; agents: Agent[] }
interface Effort { id: string; name: string; kind: Kind; hue: string; running: boolean; goal: string; criteria: Criterion[]; wwi: { last: string; waits: string; next: string }; tasks: Task[] }
interface InboxItem { id: string; kind: 'perm' | 'accept' | 'question' | 'done'; glyph: string; effort: string; from: string; title: string; time: string; body: string }
interface ChatEntry { who?: string; report?: boolean; from?: string; time: string; effort: string; text: string }
type CardType = 'question' | 'finding' | 'source' | 'idea' | 'decision' | 'map';
interface Card { id: string; type: CardType; x?: number; y?: number; title: string; body: string; src?: string; long?: boolean; under?: string }
interface VariantSlot { level: 'Global' | 'Component' | 'Content'; slot: string; options: string[] }

const PROJECTS: Project[] = [
  { id: 'shop', name: 'Shop', repos: ['shop-api', 'shop-web'], account: 'work', efforts: 5, running: 5, inbox: 4, opened: 'today 09:12', note: 'Payments, login fix, dark mode' },
  { id: 'dovetail', name: 'Dovetail', repos: ['dovetail'], account: 'personal', efforts: 3, running: 2, inbox: 1, opened: 'yesterday', note: 'GUI rework' },
  { id: 'garden', name: 'Garden planner', repos: ['garden-app', 'garden-data'], account: 'personal', efforts: 2, running: 0, inbox: 0, opened: '12 Nov', note: 'Planting calendar' },
  { id: 'thesis', name: 'Thesis notes', repos: [], account: 'personal', efforts: 4, running: 1, inbox: 0, opened: '9 Nov', note: 'Research only, no repo' },
  { id: 'homelab', name: 'Home lab', repos: ['infra', 'dotfiles', 'backups'], account: 'personal', efforts: 1, running: 0, inbox: 0, opened: '28 Oct', note: 'Backups to the NAS' },
];

const MACHINE_LIMIT: number = 4; // soft limit on running efforts on this machine

const KINDS: Record<Kind, { glyph: string; label: string }> = {
  feature: { glyph: '✦', label: 'feature' },
  fix: { glyph: '⚒', label: 'fix' },
  cleanup: { glyph: '⌫', label: 'cleanup' },
  research: { glyph: '⌕', label: 'research' },
  experiment: { glyph: '⚗', label: 'experiment' },
};

const EFFORTS: Effort[] = [
  {
    id: 'pay', name: 'Payment providers', kind: 'research', hue: 'var(--tag-blue)', running: true,
    goal: 'Which payment provider should Shop use for the EU and the US?',
    criteria: [
      { text: 'At least three providers compared on fees, payouts and SCA', state: 'done' },
      { text: 'A recommendation with reasons and the risks', state: 'open' },
      { text: 'Checkout prototype with the chosen provider in shop-web', state: 'open' },
    ],
    wwi: {
      last: 'Fees for Stripe, Adyen and Mollie collected; SCA comparison report written.',
      waits: 'Question from <b>research-1</b>: include Adyen even though pricing needs a sales call?',
      next: 'Answer the question, then read the SCA report and pick a direction on the canvas.',
    },
    tasks: [
      { title: 'Gather fee data', status: 'Done', agents: [] },
      { title: 'Compare SCA support', status: 'Review', agents: [{ name: 'research-1', kind: 'slice', state: 'needs', branch: '' , subs: [{ name: 'fees-web', state: 'done' }] }] },
      { title: 'Write recommendation', status: 'To do', agents: [] },
    ],
  },
  {
    id: 'login', name: 'Login bug', kind: 'fix', hue: 'var(--tag-coral)', running: true,
    goal: 'Users on Safari are logged out after about five minutes.',
    criteria: [
      { text: 'A test reproduces the logout and fails before the fix', state: 'done' },
      { text: 'Fix passes in Safari and Chrome', state: 'review' },
      { text: 'No regressions in the session tests', state: 'open' },
    ],
    wwi: {
      last: 'login-fix set SameSite=None; Secure on the refresh cookie; e2e green in Chrome.',
      waits: 'Approve <b>npm run e2e -- --browser safari</b>, and review the criterion “Fix passes in Safari and Chrome”.',
      next: 'Once Safari passes, ask login-fix to open the PR on shop-api.',
    },
    tasks: [
      { title: 'Reproduce on Safari', status: 'Done', agents: [] },
      { title: 'Fix refresh cookie', status: 'In progress', agents: [{ name: 'login-fix', kind: 'slice', state: 'needs', branch: 'fix/login-safari', subs: [{ name: 'run e2e', state: 'working' }] }] },
      { title: 'Regression test', status: 'To do', agents: [] },
    ],
  },
  {
    id: 'dark', name: 'Dark mode', kind: 'feature', hue: 'var(--tag-lilac)', running: true,
    goal: 'A dark theme for shop-web that users can switch on from the header.',
    criteria: [
      { text: 'Toggle in the header, choice kept across visits', state: 'open' },
      { text: 'Every page passes 4.5:1 contrast', state: 'open' },
      { text: 'No light flash on load', state: 'open' },
    ],
    wwi: {
      last: 'Theme tokens merged; dark-mode built three header variants and two hero lines.',
      waits: 'dark-mode finished a turn: pick a header and hero combination in the viewer.',
      next: 'Choose a combination, “Keep this combination”, then dark-mode audits the pages.',
    },
    tasks: [
      { title: 'Theme tokens', status: 'Done', agents: [] },
      { title: 'Header toggle', status: 'In progress', agents: [{ name: 'dark-mode', kind: 'slice', state: 'working', branch: 'feat/dark-mode', subs: [] }] },
      { title: 'Audit pages', status: 'To do', agents: [] },
    ],
  },
  {
    id: 'search', name: 'Search index', kind: 'cleanup', hue: 'var(--tag-mint)', running: true,
    goal: 'Drop the old search index code from shop-api.',
    criteria: [{ text: 'Old index module removed, build and tests green', state: 'open' }],
    wwi: { last: 'search-gc removed the writer side.', waits: 'Nothing.', next: 'Remove the reader side and the feature flag.' },
    tasks: [{ title: 'Remove old index', status: 'In progress', agents: [{ name: 'search-gc', kind: 'slice', state: 'working', branch: 'chore/drop-old-index', subs: [] }] }],
  },
  {
    id: 'coupon', name: 'Coupon A/B', kind: 'experiment', hue: 'var(--tag-gold)', running: true,
    goal: 'Does a 10 % first-order coupon raise sign-ups?',
    criteria: [{ text: 'Flagged experiment ships to 10 % of visitors', state: 'open' }],
    wwi: { last: 'Effort created; no agents yet.', waits: 'Nothing.', next: 'Ask the coordinator to plan the experiment.' },
    tasks: [{ title: 'Plan the experiment', status: 'To do', agents: [] }],
  },
];

const INBOX: InboxItem[] = [
  { id: 'i1', kind: 'perm', glyph: '⚠', effort: 'login', from: 'login-fix', title: 'Bash · npm run e2e -- --browser safari', time: '2m',
    body: '**login-fix** wants to run a command in `~/…/worktrees/shop-api/fix-login-safari`:\n\n```\nnpm run e2e -- --browser safari\n```\n\nIt runs the e2e suite against a local Safari WebDriver. No network beyond localhost.' },
  { id: 'i2', kind: 'accept', glyph: '☑', effort: 'login', from: 'login-fix', title: 'Criterion · Fix passes in Safari and Chrome', time: '2m',
    body: '**Acceptance criterion:** Fix passes in Safari and Chrome\n\n**Evidence from login-fix:**\n\n- Chrome e2e: 48 passed, 0 failed (`e2e/chrome-2026-11-15.log`)\n- Safari e2e: *waiting for the run you are asked to approve*\n- Change: `src/auth/refresh.rs` sets `SameSite=None; Secure`\n\nApprove only when you are satisfied. Dovetail ticks the criterion on approval.' },
  { id: 'i3', kind: 'question', glyph: '?', effort: 'pay', from: 'research-1', title: 'Include Adyen? Its pricing needs a sales call', time: '14m',
    body: '**research-1 asks:** Adyen publishes no price list for our volume; getting one needs a sales call. Include it in the comparison with *estimated* fees, or drop it?' },
  { id: 'i4', kind: 'done', glyph: '✓', effort: 'dark', from: 'dark-mode', title: 'Header variants ready to view', time: '31m',
    body: 'dark-mode built three header variants, two footers and three hero lines. Open the viewer to mix and match.' },
];

const CHAT: ChatEntry[] = [
  { who: 'You', time: '09:14', effort: 'login', text: 'What is left on the Safari logout?' },
  { who: 'coordinator', time: '09:14', effort: 'login', text: '**login-fix** found it: the refresh cookie had `SameSite=Lax`, so Safari dropped it on the cross-site token refresh.\n\n- Fix is in `src/auth/refresh.rs`\n- Chrome e2e is green\n- Safari run is waiting for your approval in the inbox' },
  { report: true, effort: 'pay', from: 'research-1', time: '09:20', text: 'SCA comparison written · 3 providers · 9 sources · report ready to read' },
  { who: 'You', time: '09:31', effort: 'pay', text: 'Summarise the SCA findings in two lines.' },
  { who: 'coordinator', time: '09:31', effort: 'pay', text: '**Stripe and Adyen** handle 3DS2 and exemptions for us; **Mollie** does too but has *no US acquiring*, so it fails the US half of the question.\n\nThe full report is on the canvas as card **F3**.' },
  { report: true, effort: 'dark', from: 'dark-mode', time: '09:44', text: 'Header variants ready · 3 headers · 2 footers · 3 hero lines' },
  { report: true, effort: 'search', from: 'search-gc', time: '09:50', text: 'Writer side removed · 14 files · tests green' },
  { who: 'You', time: '09:52', effort: 'dark', text: 'Keep the toggle as an icon, not a text button.' },
  { who: 'coordinator', time: '09:52', effort: 'dark', text: 'Told **dark-mode**. Variant *B* of the header already uses an icon; the viewer has it.' },
];

// Question-tree canvas for "Payment providers". Coordinates are in canvas px.
const CARDS: Card[] = [
  { id: 'Q1', type: 'question', x: 360, y: 20, title: 'Which provider for the EU and the US?', body: 'Root question of the effort.' },
  { id: 'Q2', type: 'question', x: 40, y: 170, title: 'Fees at our volume?', body: '~2 400 orders a month, €38 average.' },
  { id: 'Q3', type: 'question', x: 360, y: 170, title: 'SCA / 3DS handled for us?', body: 'Who runs the challenge flow and exemptions.' },
  { id: 'Q4', type: 'question', x: 680, y: 170, title: 'Payouts and currencies?', body: 'EUR and USD payouts, timing.' },
  { id: 'F1', type: 'finding', x: 40, y: 330, title: 'Stripe 1.5 % + €0.25 EU cards', body: 'Cheapest published rate for EU cards at our volume.', src: 'S1' },
  { id: 'S1', type: 'source', x: 40, y: 470, title: 'stripe.com/pricing', body: 'Fetched 15 Nov.' },
  { id: 'F3', type: 'finding', x: 360, y: 330, title: 'SCA comparison (report)', body: 'Stripe and Adyen handle 3DS2 and exemptions; Mollie has no US acquiring.', long: true },
  { id: 'I1', type: 'idea', x: 680, y: 330, title: 'Use Stripe for both, revisit Adyen at 10k orders', body: 'Keeps one integration now.' },
  { id: 'D1', type: 'decision', x: 360, y: 470, title: 'Decision (open)', body: 'Not decided yet. Needs the recommendation criterion.' },
  { id: 'M1', type: 'map', x: 680, y: 470, title: 'Checkout flow map', body: 'Where the provider plugs into shop-web.' },
];
const EDGES: [string, string][] = [['Q1', 'Q2'], ['Q1', 'Q3'], ['Q1', 'Q4'], ['Q2', 'F1'], ['F1', 'S1'], ['Q3', 'F3'], ['Q4', 'I1'], ['F3', 'D1'], ['I1', 'M1']];
const INCOMING: Card[] = [
  { id: 'F4', type: 'finding', title: 'Mollie: no US acquiring', body: 'From research-1, 2 sources.', under: 'Q3' },
  { id: 'S5', type: 'source', title: 'adyen.com/pricing (no list price)', body: 'Fetched 15 Nov.', under: 'Q2' },
  { id: 'Q5', type: 'question', title: 'Chargeback fees?', body: 'Proposed by the coordinator.', under: 'Q2' },
];

// Viewer variants for "Dark mode", three levels.
const VARIANTS: VariantSlot[] = [
  { level: 'Global', slot: 'Theme', options: ['Dusk', 'Midnight', 'Soft dark'] },
  { level: 'Global', slot: 'Layout', options: ['Grid', 'List'] },
  { level: 'Component', slot: 'Header', options: ['A · centred, text toggle', 'B · left logo, icon toggle', 'C · compact'] },
  { level: 'Component', slot: 'Footer', options: ['Slim', 'Full'] },
  { level: 'Content', slot: 'Hero line', options: ['Find something good.', 'New in: winter.', 'Made to last.'] },
];

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
