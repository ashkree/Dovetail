# Terms of use (S0)

Status: decided, see Decision.

- **Date researched:** 2026-11-01
- **Plan and auth method:** claude.ai Pro, signed in with the claude.ai login (no API key)
- **Claude Code version:** 2.1.316

Evidence comes only from Anthropic's own pages. Unless a line is marked **Inference**, it reports what the linked page says.

## Summary

- No Anthropic page clearly allows or clearly bans this exact setup: a personal app that drives the unmodified `claude` binary on a Pro login.

- For: Anthropic says `claude -p` and the Agent SDK [draw from subscription limits when signed in with a plan](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan), and it documents running the CLI as a subprocess from other languages.

- Against: the [Consumer Terms](https://www.anthropic.com/legal/consumer-terms) ban scripted access unless explicitly permitted, and Anthropic [prefers API keys for third-party tools](https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account).

- No page states a limit on concurrent sessions for Pro. Parallel sessions share one usage pool, and that pool has a five-hour and a weekly limit.

- A subscription falls under the Consumer Terms, where training is a setting you control. An API key falls under the Commercial Terms, with no training on your content, per-token billing, and no monthly credit for Pro.

## Findings

### 1. Is headless use through a personal app allowed on a subscription login?

**What points toward allowed**

The [support article on the Agent SDK](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) says that, when signed in with a plan, "the Claude Agent SDK and claude -p still draw from your subscription limits." That update is dated October 7, 2026.

The same article, in an update dated June 15, 2026, says "third-party app usage" also still draws from subscription limits. It says changes announced earlier are paused, and it opens that statement with "For now".

The [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) says its restrictions don't "prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription". The sentence goes on to mention platforms that host Claude Code, and it isn't limited to them.

The same page says advertised plan limits "assume ordinary, individual usage of Claude Code and the Agent SDK." It doesn't define ordinary. The [login article](https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account) says plan usage is designed for "ordinary use of native Anthropic applications".

The [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) tells developers using other languages to run the CLI as a subprocess with `-p`. That page doesn't address which login may be used.

The [authentication page](https://code.claude.com/docs/en/authentication) documents `claude setup-token` for "CI pipelines, scripts, or other environments" without a browser login. The token authenticates with your subscription.

**What points toward not allowed, or toward caution**

The [Consumer Terms](https://www.anthropic.com/legal/consumer-terms) forbid access "through automated or non-human means, whether through a bot, script, or otherwise", except via an API key "or where we otherwise explicitly permit it". The terms don't say what counts as explicit permission.

The [login article](https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account) says the "preferred way" to use third-party tools, "including open-source projects", is API key authentication. It adds that Anthropic may, at its discretion, allow certain tools for paid subscribers who have enabled usage credits, and may charge that use to credits.

The same article prohibits tools that misrepresent their identity to Anthropic's servers or "attempt to route third-party traffic against subscription limits". It doesn't define third-party traffic.

The [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) says OAuth is designed to support "ordinary use of Claude Code and other native Anthropic applications". It says developers building products, including with the Agent SDK, should use API keys, and that Anthropic may enforce "without prior notice". It sends questions about permitted methods to sales.

It also bars developers from offering Claude.ai login in their own apps, routing requests through consumer plan credentials on behalf of their users, or collecting, storing or intermediating Claude.ai credentials.

The [Usage Policy](https://www.anthropic.com/legal/aup) bars providing access through "services that route requests through consumer subscriptions or misrepresent the product or client being used". The version on the page takes effect on Nov 12, 2026, so see Open points.

**How those points apply to Dovetail**

**Inference:** The developer restrictions above are written around serving other users. Dovetail has no other users, keeps the normal login in Claude Code's own hands, and doesn't handle credentials. Those clauses seem aimed elsewhere, but no page says a personal tool is exempt.

**Inference:** The closest fit to the Consumer Terms exception is the support statement that `claude -p` draws from plan limits. That statement is about billing, though, so reading it as explicit permission is a stretch the page doesn't make.

**Inference:** Dovetail's size matters as much as its method. A supervisor running many long sessions is further from "ordinary, individual usage" than one person running one session. The sources give no threshold.

**Conflicts between pages**

- **Which terms cover the Agent SDK.** The [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) says the Commercial Terms govern it. The [headless page](https://code.claude.com/docs/en/headless) calls `claude -p` the Agent SDK via the CLI. The [legal page](https://code.claude.com/docs/en/legal-and-compliance) puts Pro users under the Consumer Terms.

- **Agent SDK on a subscription.** The [support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) describes it working on plan limits. The [overview](https://code.claude.com/docs/en/agent-sdk/overview) and [quickstart](https://code.claude.com/docs/en/agent-sdk/quickstart) tell builders to use API keys, though their note concerns third-party developers offering claude.ai login or rate limits for their products.

- **Third-party apps.** The [support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) says their usage draws from subscription limits. The [login article](https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account) says Anthropic may charge it to usage credits instead.

### 2. Are many concurrent sessions allowed, and what limits apply on Pro?

**Concurrency.** I found no page that sets a number of concurrent sessions on Pro, and none that bans running several.

Anthropic's own [agent view](https://code.claude.com/docs/en/agent-view) runs many sessions from one screen. It's a research preview and says ten parallel agents use quota "roughly ten times as fast as running one." The [Agent SDK hosting page](https://code.claude.com/docs/en/agent-sdk/hosting) treats concurrency as limited by host memory.

The [Projects page](https://code.claude.com/docs/en/claude-projects) warns that a Pro user should expect to reach limits sooner on days they run a project. Its cap of 500 new threads a day applies to that feature, and nothing says it applies to `claude -p`.

**Limits on Pro.** Pro has [a five-hour session limit and a weekly limit](https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan). The [Pro plan page](https://support.claude.com/en/articles/8325606-what-is-the-pro-plan) gives no message or token numbers.

The limits are [shared across Claude and Claude Code](https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan), and IDE use counts too.

Anthropic [may adjust limits at its discretion](https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work), and that page says it doesn't guarantee uninterrupted service. A [May 6, 2026 post](https://www.anthropic.com/news/higher-limits-spacex) says Claude Code's five-hour limits were doubled for Pro. It gives no base figure.

Subagents [draw on your usage too](https://code.claude.com/docs/en/costs). A long session [sends the full conversation with each request](https://code.claude.com/docs/en/costs), so long-lived slices draw more than their activity suggests.

**Going past the limits.** Pro users can enable [usage credits](https://support.claude.com/en/articles/12429409-manage-extra-usage-for-paid-claude-plans), billed separately at standard API rates, up to a spend limit you set.

The [Claude Code cost page](https://code.claude.com/docs/en/costs) says credits are managed with `/usage-credits`. The credits article doesn't mention `claude -p`, so credit behaviour in a headless session is unconfirmed.

**Inference:** The practical limit for Dovetail on Pro is likely the shared usage pool, not a session count. The sources give no figures to size this.

### 3. What differs between subscription and API-key auth?

| | Pro subscription login | API key (Console) |
| --- | --- | --- |
| **Terms** | [Consumer Terms](https://www.anthropic.com/legal/consumer-terms) | [Commercial Terms](https://www.anthropic.com/legal/commercial-terms) |
| **Scripted access** | Banned unless explicitly permitted | Named exception in the Consumer Terms ban |
| **Billing** | Plan limits, then optional [usage credits](https://support.claude.com/en/articles/12429409-manage-extra-usage-for-paid-claude-plans) | [Per token](https://code.claude.com/docs/en/costs), to the Console organization |
| **Training** | [A setting you control](https://code.claude.com/docs/en/data-usage) | [Not trained on your content](https://www.anthropic.com/legal/commercial-terms) by default |
| **Retention** | [5 years with training on, 30 days with it off](https://code.claude.com/docs/en/data-usage) | [30 days standard](https://code.claude.com/docs/en/data-usage) |

**Terms.** The [legal page](https://code.claude.com/docs/en/legal-and-compliance) puts Free, Pro, Max and Ultra under the Consumer Terms, and Team, Enterprise and API under the Commercial Terms.

The Commercial Terms say services under them "are not for consumer use." They don't say how that applies to one person buying API access for personal work.

**Precedence.** The [authentication page](https://code.claude.com/docs/en/authentication) ranks credentials in this order, highest first:

1. Cloud provider credentials
2. `ANTHROPIC_AUTH_TOKEN`
3. `ANTHROPIC_API_KEY`
4. `apiKeyHelper`
5. `CLAUDE_CODE_OAUTH_TOKEN`
6. Anthropic profile and federation credentials
7. Subscription OAuth from `/login`

In `-p` mode, "the key is always used when present." So a stray `ANTHROPIC_API_KEY` in Dovetail's environment would [bill the API instead of the plan](https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan), with no approval prompt.

**Monthly API credits.** These [exclude Pro](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers). Only Max, Ultra and Team get them. On a Max plan, the [support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) says they cover `claude -p` only with an API key from the linked Console organization.

**Data detail.** Even with training off, the [Consumer Terms](https://www.anthropic.com/legal/consumer-terms) allow training on material you give feedback on, or that's flagged for safety review.

Claude Code also [keeps plaintext transcripts locally](https://code.claude.com/docs/en/data-usage) for 30 days by default. [Transcripts shared via `/feedback`](https://code.claude.com/docs/en/data-usage) are kept for 5 years. Error reporting is [on by default for Pro sign-ins](https://code.claude.com/docs/en/data-usage).

## Open points

**Is a personal supervisor "ordinary, individual usage"?** No page defines the phrase or says whether a local app driving the unmodified binary counts as a third-party tool. Anthropic support could settle it, and the [legal page](https://code.claude.com/docs/en/legal-and-compliance) sends such questions to sales. A written answer would be stronger than any reading here.

**Does the support article count as explicit permission under the Consumer Terms?** It's written as a billing note, not a permission. Only Anthropic can settle this.

**The paused changes.** The [support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) doesn't say what the changes were or when they might resume. The word "For now" suggests they could return. The user should watch that page.

**Usage Policy date.** The [page](https://www.anthropic.com/legal/aup) shows Nov 12, 2026, which is 11 days after this research. I didn't read the version in force today, so I can't say whether the consumer subscriptions clause is new. The page has a version history link, which the user can check.

**Unconfirmed, secondhand only.** Press and search summaries report a February 2026 wording that banned consumer OAuth tokens in any other product, including the Agent SDK, and blocks on third-party harnesses from April 2026. I found neither on a current Anthropic page, and I'm not citing them as evidence. The current legal page doesn't contain that wording.

**Enforcement.** I found no page tying account action to automated use. The [error reference](https://code.claude.com/docs/en/errors) lists an account-on-hold message with an appeal link, but gives no causes. The risk from a wrong reading is therefore unquantified.

**The public repo.** If others run Dovetail, each would sign in to their own binary. No page addresses publishing such a tool's source. The [legal page](https://code.claude.com/docs/en/legal-and-compliance) rules for products would need a second look if it were ever offered to others.

**Driver crates in S1.** The evidence favouring this setup concerns the unmodified `claude` binary. Whether a community crate such as `claude-agent-sdk-rs` or `rusty_claw` stays within that is for S1 to check. It should run the real binary, use its login, and not touch credentials.

**`--bare` default (technical risk).** The [headless page](https://code.claude.com/docs/en/headless) says `--bare` "will become the default for `-p` in a future release", and bare mode never reads OAuth credentials. If that happens with no opt-out, subscription login could stop working in `-p`, and `AGENTS.md` forbids bare mode. The same page says bare mode also turns off background subagents, which Dovetail tracks. The page gives no date or opt-out details.

**What each outcome would mean, in the plan's terms**

- **Pass:** the user is satisfied this is allowed, stays on the Pro login, and accepts the open points above. S1 to S3 can start.

- **Fail, move to API-key billing:** Dovetail's sessions use a Console key under the Commercial Terms, which fits the Consumer Terms exception. Cost becomes per token, and Pro gets no monthly credit toward it.

- **Fail, rethink:** the design changes, for example away from a many-session headless supervisor. The plan says to stop and rethink before any other work.

## Pages read

Support pages show only relative dates, listed as shown on 2026-11-01.

| Page | Link | Date shown |
| --- | --- | --- |
| Legal and compliance | https://code.claude.com/docs/en/legal-and-compliance | Not shown |
| Authentication | https://code.claude.com/docs/en/authentication | Not shown |
| Run Claude Code programmatically | https://code.claude.com/docs/en/headless | Not shown |
| Agent SDK overview | https://code.claude.com/docs/en/agent-sdk/overview | Not shown |
| Agent SDK quickstart | https://code.claude.com/docs/en/agent-sdk/quickstart | Not shown |
| Agent SDK hosting (keyword search only) | https://code.claude.com/docs/en/agent-sdk/hosting | Not shown |
| Manage costs effectively | https://code.claude.com/docs/en/costs | Not shown |
| Data usage | https://code.claude.com/docs/en/data-usage | Not shown |
| Agent view (keyword search only) | https://code.claude.com/docs/en/agent-view | Not shown |
| Projects (keyword search only) | https://code.claude.com/docs/en/claude-projects | Not shown |
| Error reference (keyword search only) | https://code.claude.com/docs/en/errors | Not shown |
| Consumer Terms | https://www.anthropic.com/legal/consumer-terms | Effective Oct 8, 2025 |
| Commercial Terms | https://www.anthropic.com/legal/commercial-terms | Effective Jun 17, 2025 |
| Usage Policy | https://www.anthropic.com/legal/aup | Effective Nov 12, 2026 |
| Privacy Policy (read for scope; no claim above relies on it) | https://www.anthropic.com/legal/privacy | Effective Sep 10, 2026 |
| Use the Claude Agent SDK with your Claude plan | https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan | "Updated yesterday"; updates dated Oct 7 and Jun 15, 2026 |
| Log in to your Claude account | https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account | "Updated this week" |
| Use Claude Code with your Pro, Max, or Ultra plan | https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan | "Updated yesterday" |
| How do usage and length limits work? | https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work | "Updated this week" |
| What is the Pro plan? | https://support.claude.com/en/articles/8325606-what-is-the-pro-plan | "Updated this week" |
| Manage usage credits for paid Claude plans | https://support.claude.com/en/articles/12429409-manage-extra-usage-for-paid-claude-plans | "Updated this week" |
| Monthly API credits for Max, Ultra, and Team plans | https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-ultra-and-team-plans | "Updated this week" |
| API credits for Max, Ultra, and Team plans | https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers | Not shown |
| Higher usage limits and a compute deal with SpaceX | https://www.anthropic.com/news/higher-limits-spacex | Published May 6, 2026 |
| Plans and pricing (keyword search only; nothing cited) | https://claude.com/pricing | Not shown |

## Decision

**Decision:** Pass, decided by the user on 2026-11-01. Dovetail stays on the Pro login, accepting the open points above.
