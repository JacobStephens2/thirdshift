# Grilling: #583, View available subscription usage and route work across Harnesses

Issue: https://github.com/JacobStephens2/thirdshift/issues/583
Related: #969 (coding subscription monitor in Console, `needs-info`), #1546 (fallback Model/Harness for provider capacity, auth or refusals), #584 (bounded retry for temporary Harness 503/529), #587 and #588 (closed: Claude terminal limit diagnostics and stopping-stage Recovery)
Research: [ADR 0009](../adr/0009-the-operating-system-schedules-thirdshift.md), [ADR 0016](../adr/0016-the-console-is-a-long-running-reader.md), [ADR 0021](../adr/0021-a-command-may-run-its-security-review-on-another-harness.md), [ADR 0026 (Console API)](../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md), [ADR 0026 (Failure limit)](../adr/0026-pickup-counts-failures-and-passes-share-a-repository-up-to-a-command-limit.md), [ADR 0030](../adr/0030-a-full-disk-is-the-hosts-failure-not-the-issues.md), [ADR 0031](../adr/0031-plan-usage-routes-steps-and-pauses-without-failing-the-issue.md), [run-capacity.md](run-capacity.md) Q2.6

## Round 1 (answered)

**Q1.1 - Domain vocabulary (`Plan usage` and `Plan window` vs `Usage` or `Quota wait`).** #583 asks to *"view available usage in console"* and *"route requests depending on available subscription usage"*. Three terms collide with `GLOSSARY.md` today:
1. **Session usage** already means the token counts and API-priced cost reported in one **Session trace** (`src/session/usage.rs`), so bare *"usage"* is ambiguous between tokens spent and subscription allowance left.
2. **Quota wait** already means *GitHub's* API rate-limit wait (`src/github/quota.rs`), where Passes skip and running Commands sleep in-process until a <1-hour reset. Meanwhile **Paused Command** already calls provider subscription limits *"a plan-limit warning or rejection"*, with *"window, utilization and reset time"*.
3. *"Requests"* is not a thirdshift noun: a **Harness** owns individual model turns inside a session, while thirdshift starts **Commands** and agent sessions (**Steps**).

The options:
- (a) **Adopt `Plan usage` (made up of `Plan windows`, such as a 5-hour window, a 7-day window or a spend cap, each with utilization, status and reset time when reported), keep `Session usage` for per-Session token/cost counts, keep `Quota wait` for GitHub's API quota, and speak of routing `Commands` or session `Steps`, never "requests".**
- (b) Broaden **Quota** / **Quota wait** to cover both GitHub API limits and Harness subscriptions.
- (c) Another term (say which).

*Recommended:* (a). It matches **Paused Command**'s existing *"plan-limit"* language in `GLOSSARY.md`, keeps token accounting (**Session usage**) separate from subscription headroom (**Plan usage**), and avoids conflating GitHub's sub-hour API wait with multi-day subscription windows.

*Answer:* agreed

*Checked:* Added **Plan usage** and **Plan window** under `### Agent sessions` in `GLOSSARY.md`, and added `_Avoid_` cross-references on **Quota wait** and **Session usage**.

**Q1.2 - Tracker scope (#583, #969, #1546, #584).** #583 spans two halves—viewing Plan usage in the Console and routing work around subscription limits—and sits beside three open issues:
- **#969** (`needs-info`, *"add coding subscription monitor to thirdshift console web ui"*) is the Console half of #583.
- **#1546** (`ready-for-human`, *"add fallback model setting for provider issues, like codex model at capacity"*) covers fallback for model capacity/overload, expired OAuth and cybersecurity safeguard refusals.
- **#584** (`ready-for-human`) covers bounded same-Harness retries on temporary HTTP 503/529/overload failures.

The options:
- (a) **Keep both viewing and subscription-aware routing together in #583 (splitting into a Spec with ordered Tickets once settled so the view and shared Plan-usage state land first), close #969 into #583, and keep #584 and #1546 separate—while shaping #583's `User config` route syntax so #1546 can reuse rather than replace it.**
- (b) Split #583 now: move the Console/CLI view into #969 and narrow #583 to routing only.
- (c) Fold #1546 (capacity/overload and safeguard-refusal fallback) into #583 now as one combined fallback Spec.

*Recommended:* (a). Viewing and routing read the exact same machine/Fleet Plan-usage facts, so designing their state model together avoids building it twice, while keeping #1546's distinct failure classes (expired auth, safeguard refusals, model-at-capacity) out of #583 keeps this Spec focused.

*Answer:* agree

**Q1.3 - Which Harnesses, and how thirdshift learns their Plan usage.** Across the seven registered Harnesses (`claude`, `codex`, `agy`, `grok`, `muse`, `opencode`, `kimi`), local inspection shows three distinct tiers of observable truth:
- **Codex (`codex`)**: emits live `rate_limits` (`primary`/`secondary` `used_percent` and `resets_at`) in `~/.codex/sessions/...` during `codex exec`, **and** ships a non-billed `codex app-server` protocol RPC (`account/rateLimits/read`) returning live `usedPercent`, `resetsAt`, `windowDurationMins`, `planType`, `accountId`, `credits`, `individualLimit` and `ordinaryUsageAllowed` without starting a turn.
- **Grok (`grok`)**: ships non-billed `grok usage --json` for the plan's usage limit (via its local leader service).
- **Claude (`claude`)**: emits structured `rate_limit_event` windows (`five_hour`, `seven_day`, `utilization`, `resetsAt`, `status`, plus `overageStatus` / `overageDisabledReason` on limit errors) in `--output-format stream-json` during every session (today `limits.rs` ignores `status == "allowed"` and `< 85%`), and `claude auth status --json` reports `subscriptionType` (`"max"`), `planTier`, `authMethod`, `orgId` and `email`—but Claude CLI has **no** non-session command for window utilization. Reading live Claude utilization on an idle host would mean calling undocumented `claude.ai` / OAuth endpoints with Claude Code's private keychain credentials.
- **Antigravity (`agy`), Muse (`muse`), OpenCode (`opencode`), Kimi (`kimi`)**: ship no plan-window CLI command and report only terminal `UsageLimit` errors in session streams when exhausted (`opencode stats --json` is local historical token/cost stats only).

The options:
- (a) **Record passive session-stream Plan-usage facts for every Harness (`claude` `rate_limit_event` at all utilization levels, `codex` `token_count`, and terminal `UsageLimit` rejections on all seven), combine with official non-billed CLI reads where supported (`codex` app-server rate-limits read, `grok usage --json`, and `claude auth status --json` for plan/account identity), and never scrape undocumented web/OAuth endpoints or touch Harnesses' private auth tokens.**
- (b) Passive session-stream capture only: never run `codex` app-server or `grok usage --json` outside a session, accepting stale/empty numbers on idle hosts and after window resets.
- (c) Also query Claude's internal web/OAuth usage endpoint directly with Claude Code's stored credentials so Claude refreshes without a session.

*Recommended:* (a). Widening Claude's stream decoder from `>= 85%` to every `rate_limit_event` captures live Claude windows on every session for free, official CLI reads keep Codex and Grok fresh even after an idle period or reset, and staying strictly on documented CLI/stream surfaces avoids credential extraction and ToS/breakage risk.

*Answer:* agreed, though include muse and agy in there too if possible. I don't run any subscriptions on opencode or kimi - but use those by API, so they don't need to be included here in this way.

*Checked:* Inspected `muse`, `agy`, `codex` and `grok` more deeply (wire schemas, ACP/stdio servers, docs and live zero-token calls):
- **Muse (`muse`) DOES have a non-billed RPC!** `muse serve` (stable MSP stdio JSON-RPC) serves `usage/read`, returning the host's last-observed `SubscriptionUsage` (`tier`, `observedAtMs`, `window { usedPercent, resetsAtMs, windowDurationMins }` and `weekly { usedPercent, resetsAtMs }`, or `{}` when nothing has been observed yet on that host) with zero model calls, plus `account/read` (`state`, `label` account identity, `credentialRequired`).
- **Codex (`codex`) confirmed live:** `codex app-server` `account/rateLimits/read` returned in <1s with `planType: "pro"`, `ordinaryUsageAllowed: true`, `primary: { usedPercent: 87, windowDurationMins: 10080, resetsAt: ... }`, `rateLimitsByLimitId`, and `credits: { hasCredits: true, balance: "28719.46..." }`.
- **Antigravity (`agy`):** `/usage` (`/quota`) and `/credits` exist only inside the interactive TUI; neither `agy --help` nor `agy acp` exposes a headless quota-read RPC (`agy acp` serves only `session/*` and `auth/logout`). In headless runs, `agy` reports terminal `UsageLimit` errors (with any reset text from the backend), and its `~/.gemini/antigravity-cli/settings.json` has `useG1Credits` controlling fallback to personal Google AI credits (asked in Q2.5).
- **Grok (`grok`):** Although `grok usage --help` advertises plan usage, running `grok usage --json` today exits 1 with ``Error: `grok usage` can't show your plan's usage here yet. Run /usage inside grok instead.`` So `grok`, like `agy`, starts with passive terminal `UsageLimit` capture (and can adopt `grok usage --json` once Grok implements it).
- **OpenCode (`opencode`) and Kimi (`kimi`):** Left out of subscription **Plan usage** monitoring as API-only Harnesses.

**Q1.4 - Where Plan usage lives in the Console and CLI, and how Fleet machines share logins.** Every Console view has a CLI twin with `--json` (ADR 0016, ADR 0026). Today `9 Resources` (`/resources` + the header pill) shows *one machine's* RAM, CPU load and disk space, whereas your six Fleet machines share the same Claude and Codex subscription logins ([run-capacity.md](run-capacity.md) Q2.6)—while an ETA or secondary OS account could use a different login. Consider: if `orchestrate` burns Claude's 5-hour window to 92% at 03:00 while `thirdshift.app` sits idle, `thirdshift.app`'s local Claude observation from midnight is stale; conversely, showing six ungrouped rows for one shared Max account repeats the same subscription six times. The options:
- (a) **Give Plan usage its own Console page and CLI command (with `--json` and `--fleet`, defaulting to Fleet when Fleet peers are configured, like `Metrics`), group shared subscriptions across machines by account identity when known (showing the freshest observation and naming the machines using that login, while keeping distinct accounts or unknown-identity machines separate), and surface near-limit / exhausted warnings in the Console header.** Keep `9 Resources` for per-host RAM/CPU/disk.
- (b) Fold Plan usage into `9 Resources` (`/resources`) instead of adding a new Console nav item, adding `--fleet` there.
- (c) Machine-local view only (each Console shows only what its own sessions/CLI observed, with no `--fleet` fan-out or shared-login grouping).

*Recommended:* (a). Host RAM/CPU/disk is per-machine hardware; a coding subscription is a shared pool keyed by login across machines. Keeping them on separate pages stays crisp, and Fleet fan-out solves Claude's passive-only gap whenever *any* paired host ran a Claude session recently.

*Answer:* agreed

**Q1.5 - Proactive routing (with a Day shift reserve) vs reactive fallback, or both.** Routing around subscription limits can act at two different moments:
- **Proactive (before starting a session)**: When a Harness is already in a recorded limit cooldown—or at/above a configured utilization ceiling (for instance 85–90%, holding back the last slice of your subscription for interactive Day shift work, the way GitHub **Quota wait** holds 500 requests in reserve and **Paused Command** stops Experiments at 85%)—pick the next configured route before starting.
- **Reactive (after a session hits a confirmed usage/spend limit)**: As in `chart35#239`, a Spec run starts with plenty of room, lands 17 Tickets, and hits `usage_limit_reached` at `Spec review` (or hits exhaustion on a host whose passive Claude reading was stale).

The options:
- (a) **Both:** route **proactively** before starting a session when a Harness is in a recorded limit cooldown or at/above an optional reserve ceiling in `User config`, **and** route **reactively** when a session fails on a confirmed usage/spend limit (never on bare HTTP 429 transient throttling).
- (b) **Reactive only:** always run the primary Harness until a session actually fails with a confirmed usage/spend limit, then fall back and record the cooldown.
- (c) **Proactive only:** choose before starting from known utilization/cooldowns, and never switch after a session fails.

*Recommended:* (a). Reactive-only drains 100% of your subscription before routing, starving your own interactive Day shift terminal, while proactive-only fails `chart35#239` whenever a long Spec run crosses 100% mid-flight or Claude's passive utilization was last seen hours earlier.

*Answer:* agreed

**Q1.6 - Routing unit and pinning (per-Step vs per-Command, explicit flags, and Evals).** Three boundaries decide what may switch Harnesses/Models:
1. **Mid-Command (`chart35#239`)**: All 17 Tickets landed on Claude, then `Spec review` hit `usage_limit_reached`. A **Resume** (`--resume <id>`) cannot cross Harnesses, so switching to Codex means running the unfinished **Step** (`Spec review`) as a fresh session on Codex over the preserved git branch, leaving finished Tickets untouched.
2. **Explicit choices**: A person runs `thirdshift <url> --harness claude`, a **Custom process** Step names `harness = "claude"`, or `security.harness` chooses the **Security review** (ADR 0021).
3. **Evals**: **Experiments** and **Scoring runs** evaluate a fixed **Arm** / **Grader** and already become a **Paused Command** at plan limits (`GLOSSARY.md`).

The options:
- (a) **Route per session `Step` on factory Commands using `User config` defaults, never route `Experiments` or `Scoring runs` (keep `Paused Command`), and treat an explicit CLI `harness`/`model` flag or explicit Step `harness` as pinned (no silent switch to another Harness unless a fallback covers it).** Finished Steps and landed Spec Tickets stay done; a Step that switches Harnesses starts a fresh session on the new Harness (never a cross-Harness **Resume**), and `.models.json` / notifications record what actually ran.
- (b) Route only once at **Command** startup before the first Step; if a limit hits mid-Command (such as `Spec review` in `chart35#239`), stop the Command rather than running later Steps on another Harness.
- (c) Let subscription routing override explicit CLI `harness` flags and Step `harness` pins too.

*Recommended:* (a). Per-Step routing solves `chart35#239` directly without throwing away 17 landed Tickets, fits ADR 0021's multi-Harness direction, preserves Eval validity, and keeps `--harness` honest when you explicitly ask for one CLI.

*Answer:* agreed

**Q1.7 - Spending boundary, and what happens when every allowed route is exhausted.** Two safety rules govern the end of a configured route list:
1. **Paid/API spend**: Installed Harnesses or credentials on a machine can include usage-billed API keys or overage. Should thirdshift ever fall back to an installed Harness or credential you did not list in the routing config?
2. **All allowed routes unavailable**: Suppose your route list is `[claude, codex]` and *both* hit their limit/reserve ceiling (or you configured no fallback). Unlike GitHub's <1-hour **Quota wait**, a `seven_day` window reset or monthly spend cap can last days, so sleeping inside a process breaks ADR 0009; at the same time, subscription exhaustion is an account limit rather than a broken issue, so branding Ready issues `agent-failed` would trip `pickup.failure_limit` for nothing (the exact problem ADR 0030 fixed for full disks).

The options:
- (a) **Route only across explicitly configured choices (never falling back to an unlisted Harness or paid credential); when no allowed route is available, scheduled `Passes` skip quietly (logging once on reason change), and a live/hand-started `Command` stops without sleeping in-process—preserving any branch work, never counting account quota exhaustion as `agent-failed` toward `pickup.failure_limit` (releasing an untouched Claim, or leaving preserved work waiting/paused with honest provider reset/cap facts).**
- (b) Same Pass skip and opt-in boundary, except a live Command that already pushed branch work still gets `agent-failed` and counts toward `pickup.failure_limit`.
- (c) Same as (a), except when a live Command's earliest confirmed `resets_at` is short (for instance ≤ 1 hour, with no spend-cap rejection), let the live Command wait it out like GitHub **Quota wait**, and stop immediately when the reset is longer, absent, or blocked by a spend cap.

*Recommended:* (a). Strict opt-in prevents surprise API spend, skipping Passes stops cron from failing Ready issues in a loop, stopping without multi-hour sleeps preserves ADR 0009, and keeping account exhaustion out of `agent-failed` prevents an empty subscription window from locking repository Pickup after the window resets.

*Answer:* agreed

## Round 2 (answered)

**Q2.1 - CLI command, Console route, and NATted Fleet reporting (`plan-usage` / `/plan-usage`).** Building on Q1.1 and Q1.4, **Plan usage** needs an exact CLI command, Console route/nav entry, and Fleet transport. On your Fleet ([2036-console-metrics.md](2036-console-metrics.md) Q2.1), `lima-thirdshift` runs behind NAT on your MacBook: `thirdshift.app` cannot pull inbound HTTPS from `lima-thirdshift`, which is why `console report` pushes a machine's `Metrics` summary outward after each Command. If you run Claude or Muse on `lima-thirdshift` and open `console.thirdshift.app` from your phone, `thirdshift.app` only sees `lima-thirdshift`'s fresher **Plan window** observation if `console report` pushes local **Plan usage** too. The options:
- (a) **Name the command `thirdshift plan-usage [--fleet | --machine <name>] [--json]` and the Console page `/plan-usage` (`Plan usage` in `nav.html`, `/api/v1/plan-usage` in OpenAPI, plus a header warning pill beside host resources when a configured route is exhausted or at/above its reserve ceiling), cache last-seen peer snapshots for offline Fleet hosts, and include the machine's local `Plan usage` snapshot in `console report`'s best-effort post-Command push.**
- (b) Same `thirdshift plan-usage` / `/plan-usage` command, page and header pill, using live outbound Fleet pull only (no `console report` push or offline peer cache).
- (c) Different command or page name (say which).

*Recommended:* (a). `plan-usage` matches the glossary term word-for-word, avoids colliding with `opencode`/`grok`'s subcommands or **Session usage**, and piggybacking on `console report` keeps `thirdshift.app`'s Fleet view accurate when your laptop VM spends Claude or Muse allowance.

*Answer:* agreed

**Q2.2 - When active non-billed reads (`codex`, `muse`, `claude auth status`) run.** Q1.3 gives three non-billed local/RPC readers (`codex app-server` `account/rateLimits/read`, `muse serve` `usage/read` + `account/read`, and `claude auth status --json`), alongside passive session-stream writes on every Harness. ADR 0016 keeps `thirdshift console` near-zero CPU/network when idle. When should thirdshift run those active CLI/RPC readers?
- (a) **On demand with a short TTL cache (e.g. 60s), never on an idle background timer:** run them when `thirdshift plan-usage` or `/plan-usage` is read (if the machine's saved snapshot is older than 60s) and before a Pass/Step chooses a route when a candidate Harness's saved snapshot is missing, past `resets_at`, or older than a short freshness window (e.g. 5 minutes); meanwhile every running session updates the machine's saved **Plan usage** file passively as stream events arrive.
- (b) Poll on a recurring background timer inside `thirdshift console` (e.g. every 5 minutes) even when nobody is viewing the Console and no Commands are running.
- (c) Run active CLI/RPC reads only when viewing `plan-usage` / `/plan-usage`; routing reads only the passive file written by sessions.

*Recommended:* (a). Zero idle background polling preserves ADR 0016, a 60s cache keeps browser refreshes snappy, and refreshing stale/post-reset Codex and Muse snapshots right before a routing decision prevents routing on stale percentages after a window resets.

*Answer:* agreed

**Q2.3 - `User config` syntax for routes and Day shift reserves (shared with #1546).** Today `[harness]` has `default = "claude"` and `[harness.<name>]` (`model`, `effort`), with strict key checking (`src/config.rs:1283`). Q1.2, Q1.5 and Q1.7 ask for an explicit ordered route list with optional reserve ceilings that #1546 can later reuse for capacity/refusal fallbacks—and both Codex (`rateLimitsByLimitId`) and Antigravity (per-model-group quotas) can track separate pools for different Models on the *same* Harness. The options:
- (a) **Under `[harness]`, add an optional ordered `routes` list and an optional `max_utilization` percentage (1–100):** each entry in `routes` may be either a Harness name string (`"claude"`, inheriting `[harness.claude]`'s `model` and `effort`) or an inline table (`{ harness = "agy", model = "gemini-3.8-flash-high", effort = "high", max_utilization = 90 }`) so two Models on the same Harness or per-route reserve ceilings work cleanly. With `routes` omitted or empty, thirdshift keeps today's single-Harness behavior (`harness.default`, no fallback). When `max_utilization` is unset, proactive switching happens only on confirmed exhaustion (`100%` / `rejected`), so a Day shift reserve (e.g. `max_utilization = 85`) is explicit rather than a hidden surprise.
- (b) Same `routes` syntax, except default `max_utilization` to `85` whenever `routes` has two or more entries (matching Evals' 85% stop unless overridden).
- (c) Restrict routes to one entry per Harness (`routes = ["claude", "codex", "muse"]` strings only, always using `[harness.<name>]` settings and one shared `max_utilization`).

*Recommended:* (a), or (b) if you want an 85% Day shift reserve out of the box whenever multiple routes are listed. Allowing both `"claude"` shorthand and `{ harness = ..., model = ... }` inline tables keeps the common case a one-liner (`routes = ["claude", "codex", "muse", "agy"]`) while supporting same-Harness model pools and #1546's model-at-capacity fallbacks without a second config format.

*Answer:* agreed

*Checked:* `src/config/settings.rs` inventories every leaf in `DEFAULTS` into `thirdshift config` and Console `/config` automatically (`routes = []`, `max_utilization = 100`, `security.routes = []`), so `100` cleanly represents the default "switch proactively only at 100% / confirmed exhaustion" without a `null` special case.

**Q2.4 - How `security.harness` (ADR 0021) behaves when plan-limited.** Today a **Security review** or **Security run** uses `security.harness` when set, and if `security.harness` fails *before first turn*, `src/session.rs:539` falls back only to `harness.default`—never on a cybersecurity **SafeguardRefusal** (which holds **Self-merge**). Under #583, `security.harness` itself may hit its **Plan window** ceiling, *or* `harness.default` may already be exhausted when `security.harness` falls back. The options:
- (a) **Add an optional `security.routes` list (same entry format as `harness.routes`); when `security.routes` is unset and `security.harness` is unavailable due to `Plan usage` (or a startup error before first turn), walk `harness.routes` (falling back to `harness.default` when `harness.routes` is unset). Keep ADR 0021's rule that a cybersecurity `SafeguardRefusal` never routes to another Harness and still holds `Self-merge`.**
- (b) Keep `security.harness` pinned with fallback only to `harness.default`; if both are plan-limited, pause/stop even when `harness.routes` has other healthy Harnesses.
- (c) Walk `harness.routes` on a cybersecurity `SafeguardRefusal` too (note: this overlaps #1546/#1972).

*Recommended:* (a). Without walking `harness.routes`, an exhausted `claude` (`harness.default`) combined with an exhausted `codex` (`security.harness`) would block Security reviews even when `muse` or `agy` is configured in `harness.routes`, while keeping `SafeguardRefusal` out of #583 preserves ADR 0021's safety rule and leaves refusal policy to #1546/#1972.

*Answer:* agreed

**Q2.5 - Provider credit and overage pools (`Codex` credits and `agy` `useG1Credits`).** Live testing on this machine showed your Codex Pro account at `primary.usedPercent: 87` alongside `credits: { hasCredits: true, balance: "28719.46..." }`, while Antigravity CLI has `useG1Credits` in `~/.gemini/antigravity-cli/settings.json` (which spends personal Google AI credits once subscription baseline quota runs out). If Codex's included 7-day window reaches `100%`, `ordinaryUsageAllowed` or credits may still permit turns by burning your credit balance; similarly, an interactive `useG1Credits: true` in `~/.gemini/antigravity-cli/settings.json` would spend Google AI credits headlessly before `agy` ever reports exhaustion. Under Q1.7's spending boundary, how should included windows vs credit balances work?
- (a) **Treat included `Plan windows` (`usedPercent` / `utilization`) as the routing boundary by default—routing away once an included window reaches `max_utilization` or `100%` before burning paid/credit balances, and passing `--settings '{"useG1Credits":false}'` on headless `agy` launches—unless a route explicitly opts in with `allow_credits = true` in `User config`.**
- (b) Leave credit/overage behavior entirely to each Harness CLI's own account/settings (if Codex has credits or `agy` has `useG1Credits: true` in its own config, let it spend them and route only when the CLI itself rejects).

*Recommended:* (a). Otherwise having a Codex credit balance or interactive `useG1Credits: true` silently defeats subscription routing by draining credits on unattended Pickup work before moving to your other paid-for subscriptions (`claude`, `muse`, `agy`).

*Answer:* agreed

**Q2.6 - Reactive mid-Step handoff when a session hits a limit *after* doing work (`before_first_turn == false`).** `SessionFailure.before_first_turn` (`src/harness/interpretation.rs:196`) already distinguishes an immediate startup rejection from a session that ran tool calls, edited files, or made commits before hitting a limit (like `chart35#239`, which hit Claude's limit at record 810 of `Spec review`). Because the next Harness cannot `--resume` the prior Harness's conversation ID, how should reactive fallback hand off the worktree when `before_first_turn == false`?
- (a) **Keep all commits and uncommitted worktree changes intact (never `git reset --hard`), prepend a factory-owned handoff preamble to that Step's `Session prompt` telling the new Harness that the prior session stopped at a Plan-usage limit mid-Step and instructing it to inspect `git status` and `git log` against the base before finishing, and try each configured route at most once per Step.** (Read-only Steps like `Triage` simply re-run cleanly on the next route.)
- (b) Keep local commits, discard uncommitted edits (`git checkout -- . && git clean -fd`), and start the Step on the next Harness.
- (c) Switch Harnesses reactively **only** when `before_first_turn == true`; if a session hit a limit mid-Step after doing work, stop/pause the Command and let the next **Continuation** take over.

*Recommended:* (a). In `chart35#239`, `Spec review` may already be 80% through fixing review findings when the limit hits; discarding uncommitted edits throws away paid work, stopping mid-Command fails to finish the PR on Codex, and thirdshift's prompts already reconcile existing branch state cleanly on Continuations.

*Answer:* I think b, as I'm wary to edit the prompt without an eval / experiment - I don't want the agent to get worried about running out

*Checked:* Keeping zero new prompt text is clean; cross-checking `src/prompt.rs` and `src/delivery.rs` surfaced two worktree/prompt edge cases (`Implement` after local commits and merge-conflict `Repair` with `MERGE_HEAD`), asked in Q3.3.

**Q2.7 - Clearing cooldowns when `resets_at` is absent or a monthly spend cap outlives a weekly reset (`chart35#239`).** In `chart35#239`, Claude's structured error reported `rateLimitType: seven_day, resetsAt: Oct 10 02:00 UTC` alongside `overageStatus: rejected` and prose `"You've hit your monthly spend limit"`, and Codex's schema states *"clients must not infer recovery from percentages or reset times"*. Meanwhile `agy`, `grok`, or a pure spend-cap rejection may omit `resets_at` altogether. What clears a Harness's exhaustion cooldown?
- (a) **Hold the cooldown until `resets_at` when reported, or for an initial 1-hour revalidation backoff (doubling on repeated `before_first_turn` limit rejections up to 6 hours) when `resets_at` is absent. Once that time passes, mark the Harness *revalidating* rather than unconditionally healthy: re-probe via `codex` / `muse` where supported (requiring `ordinaryUsageAllowed != false` and `spendControlReached == false` on Codex), and for passive-only Harnesses (`claude`, `agy`, `grok`) let the next Step try once—if it rejects again `before_first_turn` (e.g. a monthly spend cap still blocks after the weekly reset), re-arm the backoff cooldown and immediately fall back to the next route. Also provide `thirdshift plan-usage --clear [<harness>]` for manual reset after raising a spend cap.**
- (b) Trust `resets_at` unconditionally when present, and when `resets_at` is absent, hold the Harness unavailable until a person runs `thirdshift plan-usage --clear` (never auto-revalidating).
- (c) Trust `resets_at` unconditionally when present, and use a fixed 15-minute cooldown (like GitHub **Quota wait**'s missing-reset fallback) when `resets_at` is absent.

*Recommended:* (a). Fixed 15-minute retries waste turns every quarter hour against a multi-week monthly spend cap, while never auto-revalidating locks out `agy`/`grok` until human intervention; exponential revalidation (1h → 2h → 4h → 6h cap) plus `--clear` handles both missing timestamps and `chart35#239`'s overlapping weekly/monthly caps safely.

*Answer:* agreed

**Q2.8 - Does routing consult Fleet peers, or stay machine-local?** For **viewing**, `thirdshift plan-usage --fleet` and `/plan-usage` aggregate across the Fleet (Q1.4, Q2.1). What about **routing** inside a Pass or Command on host `dev` when `orchestrate` just exhausted a shared Claude Max login two minutes ago? Claude has no non-billed window probe, so `dev`'s local Claude file doesn't know yet. Compare:
- (a) **Keep routing decisions strictly machine-local (local saved `Plan usage` + local non-billed CLI probe for `codex`/`muse`, zero cross-machine HTTP calls during Pass/Step admission).** When `dev` tries Claude, Claude rejects `before_first_turn` in ~1–2 seconds, `dev` records the `resetsAt` cooldown locally, and Q1.5's reactive fallback immediately runs that Step on the next route (`codex`/`muse`) without failing the Command.
- (b) Have Passes and Commands also query paired Fleet peers' `/api/v1/plan-usage` (with a short timeout, failing open to local state) when a passive-only Harness (`claude`, `agy`, `grok`) has no fresh local observation, matching accounts by `orgId`/`email`.

*Recommended:* (a). Keeping CLI execution independent of peer reachability avoids N×M Fleet HTTP calls across frequent cron Passes, and because a depleted subscription rejects `before_first_turn` in ~1s with `resetsAt` and immediately falls back in-process, `dev` learns the shared cooldown on its first attempt with zero lost work.

*Answer:* agreed

**Q2.9 - Exact ending of a live Command (and sibling `Spec run` Tickets) when every allowed route is exhausted, plus `Built with` attribution.** Two details close out Q1.6 and Q1.7:
1. **Outcome, Claim, Inbox and sibling Spec Tickets**: Today **Paused Command** (`outcome: "paused"`, `NeedsYouKind::Paused`) covers only `Experiment` and `Scoring run`, while `Claim::Ending::OutOfDisk` (`src/claim.rs:67`) releases an issue's Claim (`in-progress` off, `ready-for-agent` restored if the Claim took it off, no `agent-failed`, work preserved on `origin`). In a `Spec run`, several Tickets may be queued or running in parallel when all routes run dry.
2. **`Built with` line (`src/harness/built_with.rs`)**: Scoring's reader parses `Built with <harness> <version> · <model> · <effort> <!-- thirdshift:built-with -->` as a single Harness/Model/Effort tuple, while `.models.json` beside each **Session trace** and the **Run notification**'s Session models list already record every session's actual Harness and Model (ADR 0021).

The options:
- (a) **Broaden `Paused Command` to cover any factory Command that stops because every allowed route's `Plan usage` is unavailable (`outcome = "paused"`): in a `Spec run`, stop starting queued Tickets and let active Tickets finish/salvage; release the issue's `Claim` without `agent-failed` (putting `ready-for-agent` back if the Claim took it off, so the next Pickup Pass after a window reset continues it automatically and clears the `Paused` Inbox row via `"taken up by a later Command"`, while hand-started/unqueued work waits in `Needs you` as `Paused` with its Recovery/resume command). For `Built with`, keep the single-tuple format naming the Harness/Model/Effort that ran the primary implementation session (or the Command's resolved first route if rerouted before implementation), while `.models.json`, `CommandRecord` and the Run notification list every Step's actual Harness and Model.**
- (b) Keep `Paused Command` for Evals only; record an all-routes-exhausted factory Run as `outcome = "failed"` (still releasing its Claim without `agent-failed`).
- (c) Change the `Built with` HTML-comment format to list multiple Harnesses when a mid-Command fallback occurred (updating `built_with::FromStr` and Scoring's reader).

*Recommended:* (a). Recording `outcome = "paused"` puts subscription stops into `/metrics`'s existing `Paused` stack instead of inflating `Failed` (#2036 Q2.4), releasing `ready-for-agent` lets Pickup self-heal automatically once a window resets, and keeping `Built with` a single parseable tuple preserves ADR 0021 and `Scoring` compatibility while `.models.json` and notifications show exact per-Step attribution.

*Answer:* agreed

*Checked:* Updated **Paused Command**, **Claim** and **Failure limit** in `GLOSSARY.md`. Cross-checking `src/claim.rs` (`hand_back`) and [ADR 0030](../adr/0030-a-full-disk-is-the-hosts-failure-not-the-issues.md) confirmed two security invariants that carry over directly: (1) putting `ready-for-agent` back checks `untrusted_edit` first (falling back to `needs-triage` if an untrusted account edited the issue since its last Trusted `ready-for-agent` labelling, and keeping `agent-failed` on Security fixes); and (2) reactive fallback and `Paused` Claim release match only structured Harness wire-frame limit events (`rate_limit_event`, `ClaudeApiError`, Codex `token_count` / RPC, `ErrorKind::UsageLimit`), never agent-writable text.

## Round 3 (answered)

**Q3.1 - `Needs you` and `Run notifications` when a `Paused Command` will self-heal via `Pickup` vs when it waits on a person.** Under Q2.9, a factory Command that stops because every allowed route's **Plan usage** is unavailable ends as `outcome = "paused"` and releases its `Claim` without `agent-failed`—putting `ready-for-agent` back when the Claim took it off. `GLOSSARY.md` defines **Needs you** as *"what won't come right unless the Day shift acts."* Compare two concrete 02:00 AM stops:
1. A scheduled `Pickup run` pauses across `[claude, codex]` and puts `ready-for-agent` back on `#1234`. At 04:00 AM Claude's 5-hour window resets, the next `Pickup run` takes `#1234` automatically, finishes it, and self-merges it.
2. You started `thirdshift <url>` by hand on an unlabelled issue, ran a PR `Fix run` / `Review run`, or ran an `Experiment` / `Scoring run`—none of which `Pickup` will retake automatically.

The options:
- (a) **Put a `Paused Command` in `Needs you` (and email its `Run notification` when notifications are configured) only when `Pickup` will not retake it automatically** (`Experiment`, `Scoring run`, PR `Fix` / `Review`, Security fix, untrusted-edit `needs-triage`, or a Run whose Claim did not put `ready-for-agent` back). When a `Paused Command` puts `ready-for-agent` back so `Pickup` will continue it after a window reset, place it in `Done` / history as `Paused`, and when `only_unmerged` is set, save its Run notification without emailing (still emailing if the user asked for every Run notification regardless of outcome).
- (b) **Put every `Paused Command` in `Needs you` and email its `Run notification`**, relying on the post-reset `Pickup run` to clear the `Needs you` row via `"taken up by a later Command"`.

*Recommended:* (a). It keeps `Needs you` strictly to items that require human action and avoids middle-of-the-night emails for paused `Pickup` work that finishes and self-merges unattended two hours later.

*Answer:* agreed

*Checked:* Updated **Paused Command**, **Needs you** and **Harness** in `GLOSSARY.md`.

**Q3.2 - Which rows show on `plan-usage` / `/plan-usage`, and where `Pass` exhaustion warns (`1 Inbox` / `status` vs Console header pill).** Five subscription Harnesses are in scope (`claude`, `codex`, `muse`, `agy`, `grok`), while a Linux server may configure only `["claude", "codex"]` and not even have `agy` or `grok` installed. Meanwhile, GitHub **Quota wait** renders a status banner at the top of `1 Inbox` (`src/console/inbox.html:68`) and `thirdshift status` (`src/logs/status.rs:277`) because Passes are held back. With multi-Harness routing, having route 1 (`claude`) at 90% while route 2 (`codex`) still has room means Passes are **not** held back—Passes skip only when *every* configured route is unavailable. The options:
- (a) **On `thirdshift plan-usage` and `/plan-usage` (`data-jump="p"` in `nav.html`), list every Harness named in `harness.default`, `harness.routes`, `security.harness` or `security.routes` (in route order, marking the currently active route and any `max_utilization` ceiling) plus any other subscription Harness (`claude`, `codex`, `muse`, `agy`, `grok`) that has a saved observation or a signed-in CLI on the queried machine/Fleet—omitting unconfigured, uninstalled, unobserved Harnesses. Show the Console header warning pill whenever *any* configured route is exhausted or at/above `max_utilization`, and show a status banner on `1 Inbox` and `thirdshift status` (like GitHub `Quota wait`) only when *every* configured default route on the machine is unavailable so Passes are skipping.**
- (b) Always list all five subscription Harnesses (`claude`, `codex`, `muse`, `agy`, `grok`) on every machine (showing `"not installed"` / `"not signed in"` rows where absent), with the same header pill and `1 Inbox` / `status` banner rules.
- (c) Same row filter as (a), except omit the `1 Inbox` and `thirdshift status` banner and keep Plan-usage warnings strictly on `/plan-usage` and the Console header pill.

*Recommended:* (a). Omitting unused/uninstalled Harnesses keeps headless servers clean while surfacing any broken entry in `harness.routes`, and reserving the `Inbox` / `status` banner for total route exhaustion avoids false alarms during normal fallback.

*Answer:* agreed

**Q3.3 - Mid-Step clean reset details (`Implement` after local commits, merge-conflict `Repair`, and `Factory skills`), building on Q2.6(b).** Cross-checking `src/prompt.rs` and `src/delivery.rs` against Q2.6(b) (*"keep local commits, discard uncommitted edits, and don't edit prompts with limit text"*) turned up two concrete cases when a session hits a limit mid-Step (`before_first_turn == false`):
1. **`Implement` on a fresh Issue branch that already made commits (or opened a PR) before hitting the limit:** `src/prompt.rs` already has two existing, eval-tested prompts—`prompt::fresh` (for a brand-new branch with no commits or PR) and `prompt::continuation` (for an Issue branch that already has commits or an open PR, with zero mention of limits or running out).
2. **Merge-conflict `Repair` (`conflict_repair`):** `Delivery` intentionally enters a conflict `Repair` with an unmerged `MERGE_HEAD` (`PendingMerge`) in the worktree, where `git checkout -- .` fails on unmerged index entries.

The options:
- (a) **Keep zero new prompt text: if `Implement` made commits or opened a PR before the mid-Step limit, run the next Harness with the existing `prompt::continuation` (staying on `prompt::fresh` when no commit or PR was made); for ordinary Steps, discard uncommitted edits (`git reset --hard HEAD` and `git clean -fd`, preserving `.git/info/exclude` factory handover files); for a merge-conflict `Repair`, `git merge --abort` and re-merge the pinned `PendingMerge` commit so the next Harness starts from the same clean conflict state; and link every configured route's `Factory skills` into the worktree.**
- (b) Re-run the exact same `prompt::fresh` string even when the earlier session already committed or opened a PR on the branch.
- (c) Switch Harnesses mid-Step only when `HEAD` has not moved and no merge is in progress; if commits were made or a conflict `Repair` is in progress when the limit hits, pause the Command and let the next `Continuation` handle it.

*Recommended:* (a). Every prompt stays 100% unchanged and free of quota-anxiety language, `prompt::continuation` prevents the second Harness from fighting commits or a PR the first Harness already created, and conflict `Repair` resets cleanly.

*Answer:* agreed

**Q3.4 - Offer `ADR 0031`, and confirm how `#583` splits into Spec Tickets.** Under `domain-modeling`, an ADR is offered when a decision is hard to reverse, surprising without context, and the result of real trade-offs. #583 amends [ADR 0021](../adr/0021-a-command-may-run-its-security-review-on-another-harness.md) (per-Step ordered `routes` beyond `Security review`) and [ADR 0029](../adr/0029-in-progress-marks-only-a-live-run.md) / [ADR 0030](../adr/0030-a-full-disk-is-the-hosts-failure-not-the-issues.md) (releasing a Claim with work on `origin` without `agent-failed` on structured provider Plan-usage exhaustion), and locks in three non-obvious choices: machine-local routing vs Fleet-wide viewing, discarding uncommitted edits instead of adding un-eval'd handoff prompt text, and pausing immediately instead of sleeping in-process. For Spec `#583` (closing `#969`), the natural dependency-ordered Ticket split is:
1. **Plan-usage state, passive stream capture (`claude`, `codex`, `muse`, `agy`, `grok`), active non-billed readers (`codex`, `muse`, `claude auth status`), and headless `agy` `--settings '{"useG1Credits":false}'` default.**
2. **`thirdshift plan-usage` CLI (`--fleet`, `--machine`, `--json`, `--clear`), Console `/plan-usage` + `/api/v1/plan-usage` + header warning pill, offline peer cache, and `console report` push** (closes `#969`).
3. **`User config` `[harness]` `routes` / `max_utilization` / `allow_credits` and `[security]` `routes`, plus proactive and reactive per-Step routing** (machine-local selection, exponential revalidation backoff, mid-Step clean reset, multi-Harness Factory skill linking).
4. **All-routes-exhausted endings:** Pass skip gate + `1 Inbox` / `status` banner, broadened `Paused Command` (`outcome = "paused"`) across factory Commands and `Spec run` sibling Tickets, and `Claim` release without `agent-failed`.
5. **User manual (`docs/guide.md` + regenerated `site/docs/index.html`) and hand-written site pages (`site/`).**

The options:
- (a) **Write `docs/adr/0031-plan-usage-routes-steps-and-pauses-without-failing-the-issue.md` alongside `GLOSSARY.md` and this grilling doc, and use this 5-Ticket Spec split when publishing `#583` (closing `#969` into `#583`).**
- (b) Skip the ADR (`GLOSSARY.md`, this grilling doc, and Spec `#583` are sufficient), keeping the 5-Ticket Spec split.
- (c) Adjust the ADR or Ticket split (say how).

*Recommended:* (a). Recording a concise ADR 0031 explains the amendments to ADR 0021/0029/0030 and stops future contributors from "fixing" deliberate trade-offs like discarding uncommitted edits or keeping routing machine-local.

*Answer:* agreed

*Checked:* Recorded [ADR 0031](../adr/0031-plan-usage-routes-steps-and-pauses-without-failing-the-issue.md).

## Settled by earlier answers (shout if any is wrong)

- **Vocabulary (`GLOSSARY.md` updated):** **Plan usage** names how much of a coding subscription's allowance is used across its **Plan windows** and any spend cap, and whether included work can still run on its **Harness**. **Session usage** stays per-Session token/cost totals, **Quota wait** stays GitHub's API quota wait, and routing applies to **Commands** and session **Steps** (Q1.1).
- **Tracker scope:** #583 covers both the Console/CLI **Plan usage** view and subscription-aware routing (to be published as a Spec with ordered Tickets so shared state and viewing land first). #969 closes into #583; #584 (temporary 503/529 retries) and #1546 (capacity/auth/refusal fallback) stay separate, with #583's `User config` route syntax shaped so #1546 can reuse it (Q1.2).
- **Harnesses in scope:** Subscription **Plan usage** covers the five subscription Harnesses—`claude`, `codex`, `muse`, `agy` and `grok`—leaving out `opencode` and `kimi` (used via API keys, not subscriptions). Sources are passive session-stream capture plus official non-billed CLI/RPC reads (`codex` app-server, `muse serve`, `claude auth status`, and `grok usage --json` if/when Grok implements it), never scraping undocumented web/OAuth endpoints or reading private auth tokens (Q1.3).
- **Console and CLI surface:** **Plan usage** gets its own Console page and CLI twin with `--json` and `--fleet` (defaulting to Fleet when peers exist), grouping shared subscriptions across machines by account identity when known and showing near-limit / exhausted warnings in the Console header, while `9 Resources` stays per-host RAM/CPU/disk (Q1.4).
- **Proactive + reactive routing:** Routing acts both proactively before a session starts (on a recorded cooldown or optional reserve ceiling in `User config`) and reactively when a session fails on a confirmed usage/spend limit, never on bare HTTP 429 transient throttling (Q1.5).
- **Per-Step routing and pinning:** Factory Commands using `User config` routing can switch per session **Step** (starting a fresh session on the new Harness, never a cross-Harness **Resume**, keeping finished Steps and landed Spec Tickets intact). **Experiments** and **Scoring runs** never route (they stay **Paused Commands**), and explicit CLI `harness`/`model` flags and explicit Step `harness` settings stay pinned (Q1.6).
- **Spending boundary and exhaustion:** Routing only ever uses explicitly configured routes (never unlisted Harnesses or paid API credentials). When every allowed route is unavailable, scheduled **Passes** skip quietly (one Activity log line on reason change), and live/hand-started **Commands** stop without sleeping in-process, preserving branch work and never marking the issue `agent-failed` toward `pickup.failure_limit` (Q1.7).
- **CLI, Console, and NATted Fleet reporting:** `thirdshift plan-usage [--fleet | --machine <name>] [--json]` and `/plan-usage` (`/api/v1/plan-usage`, header warning pill beside host resources), with last-seen peer snapshot caching for offline Fleet hosts and post-Command local `Plan usage` pushes in `console report` (Q2.1).
- **Refresh cadence:** Active non-billed readers (`codex` app-server, `muse serve`, `claude auth status`) run on demand with short TTL caches (~60s when viewing, ~5m or past `resets_at` before routing), never on an idle Console background timer; running sessions update the local `Plan usage` file passively as stream events arrive (Q2.2).
- **`User config` route syntax (shared with #1546):** `[harness]` adds `routes = [...]` (strings `"claude"` or inline tables `{ harness, model, effort, max_utilization, allow_credits }`) and `max_utilization` (`100` by default so Day shift reserves like `85` are explicit); `[security]` adds optional `security.routes` and otherwise walks `harness.routes` when `security.harness` is plan-limited or fails before first turn, while cybersecurity `SafeguardRefusal` never routes and still holds `Self-merge` (Q2.3, Q2.4, Q2.5).
- **Credit/overage pools:** Included **Plan windows** are the default routing boundary—routing away before burning Codex credits or `agy` Google AI credits (passing `--settings '{"useG1Credits":false}'` on headless `agy` launches) unless a route sets `allow_credits = true` (Q2.5).
- **Mid-Step reactive fallback without prompt edits:** When a session hits a confirmed Plan-usage limit mid-Step (`before_first_turn == false`), thirdshift keeps local commits, discards uncommitted edits, and runs the Step on the next configured route without adding limit/handoff text to the prompt (trying each route at most once per Step) (Q2.6).
- **Cooldown revalidation and manual clear:** Cooldowns hold until `resets_at` when reported, or use exponential revalidation backoff (1h → 2h → 4h → 6h cap) when `resets_at` is absent or a spend cap still blocks after a window reset, with `thirdshift plan-usage --clear [<harness>]` for manual reset (Q2.7).
- **Machine-local routing:** Routing reads only machine-local state + local `codex`/`muse` non-billed probes (zero Fleet HTTP calls during Pass/Step admission); passive-only Harnesses (`claude`, `agy`, `grok`) learn shared-login exhaustion in ~1s via `before_first_turn` rejection and immediately fall back (Q2.8).
- **Broadened `Paused Command` (`GLOSSARY.md` updated) and `Built with`:** All-routes-exhausted factory Commands end as `outcome = "paused"` (stopping queued sibling Tickets in a `Spec run` while active ones finish/salvage), releasing the issue's `Claim` without `agent-failed` (honoring Trusted-account hand-back and structured-only limit evidence), and keeping `Built with` a single parseable primary tuple while `.models.json`, `CommandRecord` and notifications record per-Step attribution (Q2.9).
- **`Needs you` and `Run notification` on `Paused Commands`:** A `Paused Command` enters **Needs you** (and emails its **Run notification** under `only_unmerged`) only when `Pickup` will not retake it automatically; one whose `Claim` put `ready-for-agent` back sits in Done/history as `Paused` and saves its Run notification without emailing when `only_unmerged` is set (Q3.1).
- **Row filter and `Inbox` / `status` banner:** `thirdshift plan-usage` and `/plan-usage` (`data-jump="p"`) list configured routes (`harness.default`, `harness.routes`, `security.harness`, `security.routes`) plus any other subscription Harness (`claude`, `codex`, `muse`, `agy`, `grok`) with a saved observation or signed-in CLI, omitting unused/uninstalled ones. The Console header warning pill shows when any configured route is exhausted or at/above `max_utilization`; `1 Inbox` and `thirdshift status` show a status banner only when every configured default route on the machine is unavailable so Passes are skipping (Q3.2).
- **Mid-Step reset edge cases and `Factory skills`:** Zero new prompt text is added: if `Implement` made commits or opened a PR before a mid-Step limit, the next Harness runs with the existing `prompt::continuation` (staying on `prompt::fresh` otherwise); ordinary Steps discard uncommitted edits (`git reset --hard HEAD` and `git clean -fd`, preserving `.git/info/exclude` handover files); merge-conflict `Repair` aborts and re-merges the pinned `PendingMerge` commit; and every configured route's `Factory skills` are linked into the worktree (Q3.3).
- **ADR and Spec shape:** Recorded [ADR 0031](../adr/0031-plan-usage-routes-steps-and-pauses-without-failing-the-issue.md); Spec `#583` will publish with the 5 dependency-ordered Tickets above and close `#969` into `#583` (Q3.4).
- **Nothing is left open: the design tree is complete.**

## Research results (2026-11-13)

- **Existing plan-limit code in `thirdshift`:** `src/harness/interpretation/limits.rs` already parses Claude `rate_limit_event` (`status` in `rejected | allowed_warning`, `rateLimitType`, `utilization >= 0.85`, `resetsAt`, `unifiedWindows`) and Codex `token_count` (`rate_limits.primary`/`secondary` `used_percent >= 85`, `resets_at`, `rate_limit_reached_type`). `src/harness/codex/limits.rs` tails Codex's verified native `~/.codex/sessions/...` file during `codex exec`. Today only Eval callers (`Experiment` and `Scoring run`, via `src/session/limits.rs`) act on those events, turning the Command into a **Paused Command**; ordinary Runs ignore `PlanLimit` stream evidence (`plan_limit_stream_evidence_is_live_and_does_not_fail_ordinary_sessions`), confirmed by source and tests.
- **Claude terminal limit diagnostics (#587) and stopping-stage Recovery (#588):** Both are closed on `main`. `src/harness/interpretation/stream.rs` (`ClaudeApiError::facts`) already extracts `api_error`, `api_error_status`, `status`, `rateLimitType`, `overageStatus`, `overageDisabledReason`, `resetsAt` and `overageResetsAt` onto failed Claude turns, and `src/recovery.rs` reports stopping stages and Continuation commands, confirmed by `gh issue view 587` / `588` and source.
- **GitHub `Quota wait` (#1994/#1995/#1997):** `src/github/quota.rs` stores per-machine GitHub rate-limit waits keyed by `(host, account, resource)` with a 500-request reserve; Passes skip via `pass_quota_wait` and admitted Commands sleep up to a 2-hour cap, confirmed by source and `GLOSSARY.md`.
- **What each subscription Harness CLI exposes without spending model tokens (verified live):**
  - `codex` (`/opt/homebrew/bin/codex`): `codex app-server` `account/rateLimits/read` works live on stdio in <1s without starting a turn, returning `accountId`, `ordinaryUsageAllowed`, `planType` (`"pro"`), `rateLimits` / `rateLimitsByLimitId` (`usedPercent`, `windowDurationMins`, `resetsAt`, `credits`, `individualLimit`, `spendControlReached`), plus `rateLimitResetCredits`. Its schema notes for `ordinaryUsageAllowed`: *"clients must not infer recovery from percentages or reset times"*, confirmed by `codex app-server` live stdio RPC and `generate-json-schema`.
  - `muse` (`/Users/jacob/.local/bin/muse`): `muse serve` (stable MSP stdio JSON-RPC) serves `usage/read`, returning `UsageReadResult { usage?: SubscriptionUsage { observedAtMs, tier, window: { usedPercent, resetsAtMs, windowDurationMins }, weekly: { usedPercent, resetsAtMs } } }` without a model call (omitting `usage` when unobserved on that host), and `account/read` (`capabilities.experimentalApi = true`) returning `AccountState { state, label, credentialRequired }`, confirmed by `muse schema generate-json-schema` and live `muse serve` stdio RPC.
  - `claude` (`/Users/jacob/.local/bin/claude`): `claude auth status --json` returns `loggedIn`, `authMethod` (`"claude.ai"`), `apiProvider`, `subscriptionType` (`"max"`), `planTier`, `orgId`, `orgName` and `email`, with no window utilization or reset fields. `claude --help` has no `usage` subcommand, and `~/.claude/stats-cache.json` holds only historical daily token/message totals, confirmed by CLI `--help`, `claude auth status --json | jq keys` and `~/.claude/stats-cache.json` keys.
  - `agy` (`/Users/jacob/.local/bin/agy`): `/usage` (`/quota`) and `/credits` are TUI-only panels; `agy --help` and `agy acp` expose no headless quota-read subcommand or ACP RPC (`agy acp` exposes `session/*` and `auth/logout`). `~/.gemini/antigravity-cli/settings.json` supports `useG1Credits` (per-session override via `--settings`), confirmed by `agy` docs, `agy acp` `initialize` and binary RPC table inspection.
  - `grok` (`/Users/jacob/.grok/bin/grok`): `grok usage --json` currently exits 1 with ``Error: `grok usage` can't show your plan's usage here yet. Run /usage inside grok instead``, confirmed by live CLI execution.
- **Claim endings, Inbox `Paused`, and `Built with`:** `Claim::Ending::OutOfDisk` (`src/claim.rs:67`) already releases a Claim (`in-progress` off, `ready-for-agent` restored when taken off, no `agent-failed`, work kept on `origin`), while `Claimed::hand_back` (`src/claim.rs:548`) checks `untrusted_edit` before putting `ready-for-agent` back (`needs-triage` on untrusted edits, `agent-failed` on Security fixes). `NeedsYouKind::Paused` (`src/logs/reading/inbox.rs:91`) already leaves **Needs you** automatically via `"taken up by a later Command"` when a later Command starts on the same issue or pull request. `BuiltWith` (`src/harness/built_with.rs:58`) parses a single `Harness · Model · Effort` line for Scoring, while ADR 0021 keeps `Built with` on the Command's primary choice when a Step runs on another Harness, confirmed by source.
- **Config inventory, status banners, and structured limit evidence:** `src/config/settings.rs` inventories `DEFAULTS` automatically into `thirdshift config` and Console `/config`. GitHub `Quota wait` renders a status banner in `src/console/inbox.html:68` and `src/logs/status.rs:277` when Passes are held back. Across all Harnesses, `ErrorKind::UsageLimit` (`src/harness/interpretation/event.rs:155`), `rate_limit_event`, `ClaudeApiError`, and Codex `token_count` / RPC come strictly from Harness wire frames rather than agent-writable text, matching ADR 0030's security rule, confirmed by source.
- **Related open issues:** #969 (`needs-info`, *"add coding subscription monitor to thirdshift console web ui"*), #1546 (`ready-for-human`, *"add fallback model setting for provider issues, like codex model at capacity"*), and #584 (`ready-for-human`, *"Recover from temporary Harness service failures within a bounded retry budget"*), confirmed by `gh issue list` and `gh issue view`.

## Waiting on research

- Nothing.
