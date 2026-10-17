# Warp Factories, and what else could replace or underpin thirdshift

Research date: 2026-10-14.

**Question.** Could Warp's factory product replace thirdshift, given a preference to buy rather than build? What other "software factory" products, primitives or open-source projects could replace thirdshift, or sit under it so less of it is hand-rolled? The options are judged on three things, in this order:

1. **Subscription auth.** Can it run on the Claude Pro/Max and ChatGPT Plus/Pro subscriptions Jacob already pays for, rather than per-token API keys or the vendor's own credits? How does it do that, and do Anthropic's and OpenAI's terms allow it?
2. **Open source and self-hosted**, preferred but not required.
3. **Buy over build.**

**Method.**

- Product facts come from primary sources: vendor docs, pricing pages, changelogs and blog posts, the vendors' own GitHub repositories, and LICENSE files. Repository metadata comes from `gh api repos/<owner>/<repo>`, run on 2026-10-14.
- **[3rd-party]** marks anything taken from a secondary write-up. **[unverified]** marks claims I could not confirm from a primary source. **[needs live check]** marks things only a real run would settle.
- Facts about thirdshift cite files in this repository.
- No product was signed up for or run. Two `openai.com` pages (Terms of Use and the DevDay 2026 recap) and one `help.openai.com` article returned 403, so text from them is cited via search-index snippets and flagged.

**Out of scope.** Model quality, and the full design of any migration.

## TL;DR

- **Warp's product is "Warp Factories"**, launched in Early Access on 2026-08-27 ([changelog](https://docs.warp.dev/changelog/2026/)). It runs teams of cloud agents (a Foreman routing to triage, spec, implement and review agents) attached to a set of repositories. Factories are defined as files in a repo ([docs](https://docs.warp.dev/factories/), [blog](https://www.warp.dev/blog/software-factory-as-code)).
- **Warp Factories cannot run on your Claude or ChatGPT subscriptions.** The Claude Code and Codex harnesses take only API keys:
  - Claude Code takes an Anthropic API key or Bedrock credentials.
  - Codex takes an OpenAI API key. Warp says: "Codex runs are billed against your OpenAI API balance, not a ChatGPT subscription" ([harness auth](https://docs.warp.dev/platform/harnesses/authentication/)).
  - Warp's cloud-agent FAQ says a connected ChatGPT plan "applies to Warp Agent conversations in the Warp app, not cloud agent runs", and "Warp also doesn't support connecting a Claude consumer subscription" ([cloud agent FAQ](https://docs.warp.dev/agent-platform/cloud-agents/faqs/)).
  - Warp also charges per agent-hour for every cloud run, Claude Code and Codex runs included ([platform credits](https://docs.warp.dev/support-and-community/plans-and-billing/platform-credits/)).
  - Self-hosted workers require the Enterprise plan ([self-hosting](https://docs.warp.dev/factories/self-hosting/)).
  - The Warp *client* is AGPL-3.0 open source. The Factories control plane is closed SaaS ([warpdotdev/warp README](https://github.com/warpdotdev/warp)).
  - **It fails the first criterion, and mostly fails the second.**
- **Vendor terms back thirdshift's current design.**
  - Anthropic: third-party developers may not "offer Claude.ai login" or "route requests through Free, Pro, or Max plan credentials on behalf of their users". But this "does not … prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription" ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)). `claude setup-token` exists "for CI pipelines and scripts" ([authentication](https://code.claude.com/docs/en/authentication)).
  - OpenAI: ChatGPT-login `auth.json` on "trusted private infrastructure" is a documented, if "advanced", path ([CI/CD auth](https://learn.chatgpt.com/docs/auth/ci-cd-auth)).
  - A tool that shells out to your own logged-in `claude` and `codex` CLIs, which is exactly what thirdshift does, is the safest route there is.
- **No product or open-source project covers thirdshift end to end on subscriptions.** The strongest candidates:
  1. **Claude Code GitHub Actions** (`anthropics/claude-code-action`, MIT) with `CLAUDE_CODE_OAUTH_TOKEN`. It is officially subscription-billed and can fire on any GitHub event, including `issues: labeled`, `workflow_run` and cron ([docs](https://code.claude.com/docs/en/github-actions)). It is a primitive, though: the repair loop, the merge and the specs are still yours to build.
  2. **OpenHands Agent Canvas** (MIT). It ships an open-source "Software Factory" of label-driven triage, issue-to-PR, review and a merge watchdog, all on cron ([docs](https://docs.openhands.dev/openhands/usage/use-cases/software-factory.md)). It can drive the logged-in `claude`/`codex` CLIs through ACP ([ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents.md)). But the factory recipe documents API-key models and per-conversation Docker. That the two combine is **[unverified]**, and the recipe is heavy on 4 cores and 8 GB.
  3. **OpenAI Symphony** (Apache-2.0, "low-key engineering preview"). It is a spec plus an Elixir reference daemon that polls a tracker (Linear, GitHub Issues, Jira) and runs `codex app-server`. It is the closest in concept to thirdshift, but Codex-only ([repo](https://github.com/openai/symphony)).
  - Anthropic's **Routines + auto-fix + Projects** run on your plan but are not issue-driven. Their GitHub triggers cover pull requests and releases only, schedules run at most hourly, and auto-fix can't see merge conflicts ([routines](https://code.claude.com/docs/en/routines), [cloud](https://code.claude.com/docs/en/claude-code-on-the-web)).
- **Recommendation: hybrid, and keep thirdshift.**
  - Its core (label pickup → worktree → `claude -p`/`codex exec` → CI- and conflict-Repair loop → Self-merge → Spec runs over sub-issues) is the part nobody sells on subscriptions.
  - Stop hand-rolling the side parts: use Claude routines or claude-code-action for one-off PR reviews and fixes, and keep watching OpenHands and Symphony as candidates to take over orchestration.
  - Revisit this if Anthropic changes its subscription terms or Warp adds subscription auth for cloud harnesses.

## 1. What thirdshift does that a replacement must cover

thirdshift is a single Rust binary (with the Factory skills compiled in) that "turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it" ([README.md](../../README.md) line 3; [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md)). A replacement would need:

| Capability | thirdshift today | Source |
|---|---|---|
| Issue in, PR out | `thirdshift <Issue URL>`: pre-flight checks, Claim (`in-progress` label), git worktree, headless session with Factory skills, self-review against the Base branch, PR listing Unaddressed findings | [README.md](../../README.md) "What a Run does" |
| Unattended sessions on your own logins | `claude -p --permission-mode auto`. Codex, agy, Grok, Muse and OpenCode run with approvals and their sandbox off | [README.md](../../README.md) "Unattended sessions"; [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md) |
| Subscription billing | "When `claude` is logged in with a claude.ai subscription, Runs use the plan's usage allowance, not API billing" | [README.md](../../README.md) "Usage and cost" |
| Multiple harnesses, with a Model and Effort per command | Claude Code (default), Codex, Antigravity, Grok Build, Muse Code, OpenCode | [README.md](../../README.md) "Harness, Model and Effort"; [harness-candidates.md](harness-candidates.md) |
| CI and conflict Repair loop | Merges the Base branch in, watches CI, at most 5 Repairs per Run. Separates Inherited failures from the branch's own, does one Check re-run, optionally a Base fix | [README.md](../../README.md) step 6; [ADR-0008](../adr/0008-inherited-failures-fail-the-run.md), [ADR-0010](../adr/0010-one-check-re-run-before-a-declined-ci-fix.md) |
| Self-merge | `gh pr merge --merge --match-head-commit`, never GitHub auto-merge | [ADR-0004](../adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md) |
| Spec runs | Sub-issues worked as Tickets in dependency order into a Spec branch, ending in one Spec PR | [ADR-0006](../adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md) |
| Passes on a schedule | `pickup` (lowest-numbered `ready-for-agent`), `architect` (Weeding), `secure` (Fencing). Started by cron; one Pass per repo per machine | [ADR-0009](../adr/0009-the-operating-system-schedules-thirdshift.md); [docs/agents/triage-labels.md](../agents/triage-labels.md) |
| Security work | Private draft advisories, reproduction in a throwaway worktree, optional single fix, security review of a Run's change | [README.md](../../README.md) "Security runs"; [ADR-0015](../adr/0015-security-findings-stay-private-until-fixed.md) |
| Failure handling | Failed run pushes the work and sets the PR back to draft. The Claim is released, kept or removed. Continuation of an existing Issue branch | [README.md](../../README.md) "Failed runs", "Continuation"; [ADR-0002](../adr/0002-existing-issue-branch-means-continue.md) |
| Notifications and logs | Resend email, per-repo Activity log, Command and Session logs | [ADR-0005](../adr/0005-run-notifications-through-resend.md), [ADR-0011](../adr/0011-thirdshift-keeps-a-per-repository-activity-log.md) |

**Host constraints.**

- The factory host has 4 cores, about 8 GB RAM and no swap. About two building Runs at once is the practical ceiling, and the load comes from concurrent builds and tests, not from the agent processes (project memory `tracewake-machine-capacity`). So a replacement that adds a Docker container, Postgres, Redis or Kubernetes per task costs real headroom.
- Ubuntu 24.04 sets `kernel.apparmor_restrict_unprivileged_userns = 1`, so CLI sandboxes built on bubblewrap fail out of the box ([ADR-0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md), [codex-headless-harness.md](codex-headless-harness.md) §8). The host now has bwrap plus a custom AppArmor profile for Security reviews (project memory `tracewake-security-sandbox`), but Runs still have only the worktree as a boundary.

## 2. Subscription auth: what Anthropic and OpenAI allow

This is the first filter. A tool reaches a subscription in one of four ways:

- **(a)** it runs the official, unmodified `claude`/`codex` CLI under your login;
- **(b)** it does the OAuth itself, or lifts the tokens;
- **(c)** you bring your own API key;
- **(d)** you buy the vendor's own credits.

### 2.1 Anthropic (Claude Pro/Max)

- **Ordinary use.** OAuth is "intended exclusively for purchasers of Claude Free, Pro, Max, Team, and Enterprise subscription plans and is designed to support ordinary use of Claude Code and other native Anthropic applications" ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)).
- **Third-party developers.** They "should use API key authentication". Anthropic "does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, or Max plan credentials on behalf of their users". Developers also "may not collect, store, or intermediate Claude.ai credentials or session tokens" ([same page](https://code.claude.com/docs/en/legal-and-compliance)). **Route (b) is out for Claude.**
- **Your own unmodified binary is allowed.** "Nor does it prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code" ([same page](https://code.claude.com/docs/en/legal-and-compliance)). Hosts that ship Claude Code "may not remove, disable, or restrict any authentication method built into it (including methods that permit signing in with a Claude account…)" ([same page](https://code.claude.com/docs/en/legal-and-compliance)). **Route (a) is explicitly fine.**
- **Usage-limit caveat.** "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK" ([same page](https://code.claude.com/docs/en/legal-and-compliance)).
- **Scripts and CI are a documented use.** `claude setup-token` makes a one-year OAuth token "for CI pipelines, scripts, or other environments where interactive browser login isn't available". It "authenticates with your Claude subscription and requires a Pro, Max, Team, or Enterprise plan" ([authentication](https://code.claude.com/docs/en/authentication)). `--bare` mode does not read it ([headless](https://code.claude.com/docs/en/headless)).
- **Consumer Terms.** They bar access "through automated or non-human means, whether through a bot, script, or otherwise", "except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it" (effective 2025-10-08, [consumer terms](https://www.anthropic.com/legal/consumer-terms)). The Claude Code docs above (setup-token for scripts, `claude -p`, claude-code-action with OAuth, routines) are that explicit permission for the official tooling.
- **Current help-centre stance.** "You can still use the Claude Agent SDK, `claude -p`, and third-party apps with your subscription limits." The announced June change to Agent SDK usage is paused (notices dated 2026-06-15 and 2026-10-07, [support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)).
  - "Third-party apps" there sits in tension with the legal page's ban on intermediating credentials. My reading is that it means apps running the unmodified binary or SDK under the user's own login.
  - [3rd-party] reports describe enforcement in 2026 against tools doing their own Claude OAuth, OpenCode among them ([gigazine](https://gigazine.net/gsc_news/en/20260220-anthropic-third-party-block)). OpenCode's own docs now say "Anthropic explicitly prohibits this" ([opencode providers](https://opencode.ai/docs/providers/)).
- **Agent SDK.** "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK" ([Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview)).

### 2.2 OpenAI (ChatGPT Plus/Pro)

- **Two sign-in modes.** Codex offers "Sign in with ChatGPT for subscription access" or "an API key for usage-based access". Codex cloud requires ChatGPT sign-in ([auth](https://learn.chatgpt.com/docs/auth)).
- **Automation guidance.** "Use API key authentication for programmatic Codex CLI workflows, such as CI/CD jobs." But "Access tokens are intended for trusted scripts, schedulers, and private CI runners", and "Don't expose Codex execution in untrusted or public environments" ([auth](https://learn.chatgpt.com/docs/auth)).
- **ChatGPT login in CI is documented.** Reusing the ChatGPT `auth.json` in CI is "an advanced workflow for enterprise and other trusted private automation". It requires that "the runner is trusted private infrastructure", and says "Do not use this workflow for public or open-source repositories" ([CI/CD auth](https://learn.chatgpt.com/docs/auth/ci-cd-auth)). thirdshift on a private box is that case. The public-repo caveat applies to thirdshift's own public repository **[interpretation]**.
- **"Sign in with ChatGPT" for other apps.** OpenAI has an official program: "ChatGPT plan usage is available to open-source projects, personal projects that run locally, and selected private apps" ([cookbook](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt)).
  - It covers apps that drive `codex app-server` with a plan-authorized token ([Codex app-server](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)) and remote VMs ([self-hosted VMs](https://developers.openai.com/siwc/token-sharing-open-source/self-hosted-vms)).
  - The listed open-source integrations are OpenClaw, OpenCode, Pi and T3 ([Sign in with ChatGPT](https://learn.chatgpt.com/docs/sign-in-with-chatgpt)).
  - Plan usage is for "eligible ChatGPT Plus and Pro subscribers" and "counts toward your existing plan limits" ([same](https://learn.chatgpt.com/docs/sign-in-with-chatgpt)).
  - **For ChatGPT, route (b) is sanctioned.**
- **Terms of Use.** They bar "automatically or programmatically extract data or Output". I could not fetch the live page (403); the text comes from search snippets of [openai.com/policies/terms-of-use](https://openai.com/policies/terms-of-use/) **[unverified live text]**. OpenAI's own docs for `codex exec` under ChatGPT auth show it does not treat agentic CLI automation as extraction.

### 2.3 What this means for the comparison

| Route | Claude Pro/Max | ChatGPT Plus/Pro |
|---|---|---|
| (a) unmodified official CLI under your login, on your machine | Allowed, and documented for scripts and CI | Allowed (trusted private runner) |
| (a′) a hosted platform runs the unmodified CLI with *your* token | Allowed by the legal page's "including where a platform hosts Claude Code" | Not addressed for hosted third parties, apart from the SIWC program |
| (b) tool does its own OAuth or intermediates tokens | **Prohibited** | Sanctioned via the SIWC program |
| (c) BYO API key | Pay per token | Pay per token |

## 3. Warp Factories

### 3.1 What it is

- **Name and status.** "Warp Factories". "Warp Factories is now live in Early Access" (2026.08.27 entry, [changelog](https://docs.warp.dev/changelog/2026/)). TechCrunch covered the launch on 2026-08-18 [3rd-party] ([TechCrunch](https://techcrunch.com/2026/08/18/warps-new-system-is-an-out-of-the-box-software-factory-for-ai-development/)).
- **Definition.** "A factory is a team of cloud agents attached to a set of repositories." Work comes from Slack, Teams, GitHub, GitLab, Azure DevOps, Linear or Jira, moves through Triage → Planning → Building → Reviewing, and "usually" returns a PR ([Factories](https://docs.warp.dev/factories/)).
- **Foreman.** "The foreman coordinates every work item." By default it pauses for plan approval, for unclear requirements, and for merging ([how factories work](https://docs.warp.dev/factories/how-factories-work/)).
- **Merging.** "The factory never merges on your behalf" ([Factories](https://docs.warp.dev/factories/)). That conflicts with thirdshift's Merge run.
- **Factory as code.** `factory.yaml`, plus directories for `agents/`, `automations/`, `runners/`, `skills/`, `webhooks/`, `scorers/` and `benchmarks/`, reviewed through PRs ([blog, 2026-09-30](https://www.warp.dev/blog/software-factory-as-code)).
- **Shared platform.** It runs on the "Automation Platform", formerly "Oz". "Every agent run in a factory is an ordinary cloud agent run on the Automation Platform" ([Factories](https://docs.warp.dev/factories/); [platform overview](https://docs.warp.dev/platform/overview/)).
- **Dogfooding.** Warp drives its own open-source repository with a factory, using `ready-to-spec` and `ready-to-implement` labels ([warpdotdev/warp README](https://github.com/warpdotdev/warp)).

### 3.2 Triggers

- **GitHub.** Issues created, labelled, assigned or mentioned. Pull request events. Reviews. Pushes, and check suites or workflow runs completing.
- **Default automations** start work when the factory's label is applied, or when someone mentions @warp ([GitHub integration](https://docs.warp.dev/factories/integrations/github/)).
- **CI.** There is no built-in loop for fixing CI on the factory's own PRs. The example automation handles failed CI on the default branch by opening a fix PR or an issue. Merge-conflict handling is not documented ([same](https://docs.warp.dev/factories/integrations/github/)).
- **Other triggers.** Cron schedules, verified webhooks, Slack, Teams, Linear and Jira ([blog](https://www.warp.dev/blog/software-factory-as-code)).

### 3.3 Execution environment

- **Default.** "Runs execute on Warp-hosted compute by default" ([Factories](https://docs.warp.dev/factories/)). An environment is a Docker image, repos and setup commands ([platform overview](https://docs.warp.dev/platform/overview/)).
- **Self-hosting.** "Self-hosting requires an Enterprise plan." Even then, "Agent context and model requests still travel through Warp's services" ([self-hosting](https://docs.warp.dev/factories/self-hosting/)).
  - The worker, `oz-agent-worker`, is MIT. It is a daemon that connects to Warp over WebSocket and runs tasks on Docker, Kubernetes or directly on the host ([repo](https://github.com/warpdotdev/oz-agent-worker)).
- **Unmanaged mode.** `oz agent run` from your own CI or VM, which needs a `WARP_API_KEY`. Whether the Claude Code and Codex harnesses work in unmanaged mode is not documented ([unmanaged execution](https://docs.warp.dev/platform/unmanaged-execution/)).

### 3.4 Models and auth (criterion 1)

- **Harnesses.** Warp Agent (`oz`, the default), Claude Code (`claude`) and Codex (`codex`). "Third-party harnesses require a paid plan" ([harnesses](https://docs.warp.dev/platform/harnesses/)).
- **Claude Code.** It "calls Anthropic directly using credentials your team provides". The supported credentials are an Anthropic API key, a Bedrock API key, or a Bedrock access key. Subscription and OAuth tokens are not mentioned ([Claude Code harness](https://docs.warp.dev/platform/harnesses/claude-code/), [harness auth](https://docs.warp.dev/platform/harnesses/authentication/)).
- **Codex.** "Codex runs are billed against your OpenAI API balance, not a ChatGPT subscription." "A ChatGPT subscription (Plus, Pro, Team) does not include API access." An OpenAI API key is the only credential type ([harness auth](https://docs.warp.dev/platform/harnesses/authentication/), [Codex harness](https://docs.warp.dev/platform/harnesses/codex/)).
- **Secrets.** "Cloud runs of Claude Code and Codex always use Warp-managed secrets." "BYOK configured in the desktop app applies only to local agent runs" ([harness auth](https://docs.warp.dev/platform/harnesses/authentication/)).
- **ChatGPT sign-in does not reach Factories.**
  - Warp added "Sign in with ChatGPT" on 2026-09-23 / 2026-09-29 for the Warp Agent in the terminal and the Warp Agent CLI ([changelog](https://docs.warp.dev/changelog/2026/), [blog](https://www.warp.dev/blog/sign-in-to-warp-with-chatgpt)).
  - The cloud-agent FAQ is explicit: "No. A connected ChatGPT subscription applies to Warp Agent conversations in the Warp app, not cloud agent runs." Also: "Warp also doesn't support connecting a Claude consumer subscription" ([cloud agent FAQ](https://docs.warp.dev/agent-platform/cloud-agents/faqs/)).
- **Is Warp's choice forced by the terms?** Partly.
  - For Claude, the legal page would let a platform host the unmodified binary with the user's own subscription (§2.1), so the API-key-only design is Warp's choice, not strictly forced **[interpretation]**.
  - For Codex, OpenAI's CI guidance says not to use ChatGPT auth on infrastructure you don't trust (§2.2).

### 3.5 Pricing and limits

- **Factories plans** ([pricing](https://www.warp.dev/pricing)):
  - Pay-as-you-go: $0/month, usage at 20% above API rates.
  - Build: $20/month, usage at API rates, and includes the Claude Code and Codex harnesses.
  - Business: $50/user/month, adds BYO API keys and custom endpoints.
  - Enterprise: custom, adds self-hosted workers and BYO provider.
- **Platform charges on every cloud run.** Cloud runs bill "billable agent time" in agent-hours. For Claude Code and Codex, "active runtime is billable, including command execution and waiting on the model provider." Self-hosted compute avoids the compute charge but "still incur[s] platform charges" ([platform credits](https://docs.warp.dev/support-and-community/plans-and-billing/platform-credits/)). "Bringing your own inference does not remove platform charges" ([cloud agent FAQ](https://docs.warp.dev/agent-platform/cloud-agents/faqs/)).
- **No published compute rates.** "Concurrency limits and credit allocation details are still being finalized" ([cloud agent FAQ](https://docs.warp.dev/agent-platform/cloud-agents/faqs/)).
- **Starter credit.** Eligible teams can get up to $10,000 of factory usage ([warp README](https://github.com/warpdotdev/warp), [blog](https://www.warp.dev/blog/software-factory-as-code)).

### 3.6 Open source

- **Client.** The Warp client went open source on 2026-04-27 ([changelog](https://docs.warp.dev/changelog/2026/)). It is AGPL-3.0, with the UI framework under MIT ([README](https://github.com/warpdotdev/warp)).
- **MIT pieces.** `oz-agent-worker`, `oz-skills`, `oz-for-oss` (a Vercel-hosted webhook control plane for issue triage, spec and implementation on a *Warp-hosted* Oz agent), `warp-factory-examples` and `docs` (`gh api`).
- **SDKs.** `oz-sdk-*` are Apache-2.0.
- **Closed.** The Factories / Automation Platform server is not in any public repo. Self-hosted workers still need Warp's service.

### 3.7 Verdict on Warp Factories

Against thirdshift, Warp Factories is more polished on intake (Slack, Linear, Jira), on evals and benchmarks, and on team features. It is weaker on the things thirdshift is built around:

- No subscription inference.
- A per-hour platform fee on top of API bills.
- No self-merge.
- No documented loop for repairing CI or merge conflicts on its own PRs.
- Self-hosting only on Enterprise.
- A closed control plane.

**Not a fit for Jacob's priorities.** It is worth re-checking only if Warp adds subscription auth for cloud harnesses. Its FAQ says Warp is "exploring" that, subject to provider policy ([Warp docs search result](https://docs.warp.dev/agent-platform/cloud-agents/faqs/)).

## 4. Alternatives

### 4.1 Comparison table

The thirdshift-fit columns: **Pick** = label or issue pickup; **CI** = CI-repair loop; **Cfl** = merge-conflict repair; **Mrg** = self-merge; **Sched** = runs on a schedule; **Spec** = specs over sub-issues. "DIY" means possible, but you write the glue.

| Option | License / self-host | Subscription auth (how) | Pick | CI | Cfl | Mrg | Sched | Spec | Maturity | Notes |
|---|---|---|---|---|---|---|---|---|---|---|
| **thirdshift** (today) | Own code, runs on own box | Claude and ChatGPT, (a) | ✔ | ✔ | ✔ | ✔ | ✔ cron | ✔ | Working | The baseline |
| **Warp Factories** | Closed SaaS; self-hosted workers Enterprise only | **No.** API keys only for Claude Code/Codex; Warp per-hour fee | ✔ label/@warp | ✗ own PRs | ✗ | ✗ by design | ✔ | Spec stage, human-approved | Early Access since 2026-08 | §3 |
| **claude-code-action** | MIT; GitHub-hosted runners (self-hosted runners not covered in its docs) | **Claude yes**: `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`, "runs use your Claude subscription" ([docs](https://code.claude.com/docs/en/github-actions)) | ✔ any GH event | DIY via `workflow_run` | DIY | DIY | ✔ cron | DIY | v1, very active | Uses Actions minutes; OAuth token is per person |
| **Claude Code cloud + Routines + Projects** | Anthropic-hosted; self-hosted environments are Team/Enterprise only ([docs](https://code.claude.com/docs/en/self-hosted-environments)) | **Claude yes**: "draw down subscription usage the same way interactive sessions do" ([routines](https://code.claude.com/docs/en/routines)) | ✗ GitHub triggers are PR and release only; issues need hourly cron or `/fire` | ✔ auto-fix | ✗ "auto-fix can't react to conflicts" ([cloud](https://code.claude.com/docs/en/claude-code-on-the-web)) | Projects "Merge it" button | Hourly at most | Projects threads from a spec | Research preview / beta | Routines are research preview; Projects is Pro/Max beta ([projects](https://code.claude.com/docs/en/claude-projects)) |
| **Claude Managed Agents** | Anthropic-hosted, or self-hosted sandbox | **No**: API key, tokens plus $0.08/session-hour ([overview](https://platform.claude.com/docs/en/managed-agents/overview)) | DIY | DIY | DIY | DIY | ✔ | DIY | Beta | Primitive only |
| **OpenAI Codex cloud / `@codex`** | OpenAI-hosted | **ChatGPT yes**; cloud requires ChatGPT sign-in ([auth](https://learn.chatgpt.com/docs/auth)) | Linear only; GitHub issue trigger not found | `@codex fix the CI failures` in a PR | not found | not found | ChatGPT scheduled tasks | – | GA | Linear: you create the PR ([linear](https://learn.chatgpt.com/docs/third-party/linear.md)) |
| **openai/codex-action** | Apache-2.0 [unverified license] | **No**: API key only ([repo](https://github.com/openai/codex-action)) | DIY | DIY | DIY | DIY | ✔ | DIY | v1 | |
| **GitHub Copilot cloud agent, incl. Claude/Codex agents** | GitHub-hosted | **No**: AI credits plus Actions minutes; third-party agents bill as Copilot ([3rd-party agents](https://docs.github.com/en/copilot/concepts/agents/about-third-party-agents)) | ✔ assign issue | not stated | – | – | – | – | GA / preview | |
| **GitHub Agentic Workflows (gh-aw)** | MIT, runs in Actions | **No**: Claude engine needs `ANTHROPIC_API_KEY` ([engines](https://github.github.com/gh-aw/reference/engines/)) | ✔ | DIY | DIY | DIY | ✔ | DIY | Preview | |
| **Cursor cloud agents + Automations** | Cursor-hosted | **No**: "charged at API pricing" ([help](https://cursor.com/help/ai-features/cloud-agents)) | ✔ label trigger | Teams-only autofix | – | – | ✔ | – | GA | ([automations](https://cursor.com/docs/cloud-agent/automations)) |
| **Devin** | Cognition-hosted; Enterprise dedicated | **No**: own ACUs and plans ([billing](https://docs.devin.ai/admin/billing/self-serve)) | ✔ labelled | ✔ check_run trigger | – | not stated | ✔ | – | GA | Best feature match among the SaaS options, but separate spend ([automations](https://docs.devin.ai/product-guides/automations)) |
| **Factory (Droids)** | SaaS; Enterprise on-prem | **No**: BYOK only in CLI and desktop, not hosted ([byok](https://docs.factory.com/cli/byok/overview)) | Linear "Ticket to PR" every 15 min | Triage (preview) | – | – | ✔ | – | GA | ([automations](https://factory.com/product/automations)) |
| **Tembo** | SaaS; VPC on Enterprise | **Both, yes**: Claude subscription connected by pasting an auth code, plus ChatGPT plans; "Subscription-backed inference costs $0 through Tembo"; VM billed ([models](https://docs.tembo.io/features/models)) | Linear/Jira; GitHub issue triggers undocumented | unclear | unclear | unclear | ✔ | – | Active | Claude connection is a hosted platform holding your token; whether that stays within Anthropic's "unmodified binary" carve-out is **[unverified]** |
| **Google Jules** | Google-hosted | Google AI Pro/Ultra only ([limits](https://jules.google/docs/usage-limits/)) | ✔ `jules` label | not found | – | – | ✔ | – | GA | Different subscription |
| **OpenHands Agent Canvas** | **MIT**, self-host | **Partly**: ACP agents spawn the logged-in `claude`/`codex` CLIs, (a); containers take `CLAUDE_CODE_OAUTH_TOKEN` / `CODEX_AUTH_JSON` ([ACP](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents.md)) | ✔ label poll every 15 min ([issue-to-PR](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt/issue-to-pr.md)) | ✔ "failed checks lead to revisions" | watchdog refreshes stale branches | ✔ watchdog merges on green | ✔ `*/5` | – | v1.27.1; Canvas beta | The Software Factory recipe documents API-key models and per-conversation Docker ([recipe](https://docs.openhands.dev/openhands/usage/use-cases/software-factory.md)); combining it with ACP is **[unverified]** |
| **OpenAI Symphony** | **Apache-2.0**, self-host | **ChatGPT yes**: launches `codex app-server`, (a) ([SPEC](https://github.com/openai/symphony/blob/main/SPEC.md)) | ✔ `required_labels`; Linear, GitHub Issues, Jira | demo shows CI + landing | – | "land the PR" | daemon polls | parent/blocker links | v0.0.3 "engineering preview" | Codex-only; a community fork adds Claude |
| **Cyrus** | **Apache-2.0**, self-host daemon | **Claude yes**: "OAuth Token (for Max subscription users)" via `setup-token`, plus Codex ([README](https://github.com/cyrusagents/cyrus)) | ✔ assigned issues: Linear, GitHub, GitLab | – | – | – | – | – | 856★, active | Needs a public webhook URL ([self-hosting](https://github.com/cyrusagents/cyrus/blob/main/docs/SELF_HOSTING.md)) |
| **zeroshot** | **MIT**, local CLI | Yes: "reuse the harness's existing login, including subscription-backed sessions" | per task | ✔ `--ship` through PR, CI, merge | – | ✔ | – | – | 2k★, active | ([README](https://github.com/the-open-engine/zeroshot)) |
| **agent-orchestrator** (OrchestratorInc, ex-Composio) | **Apache-2.0**, desktop app + local daemon | Yes, (a): wraps `claude auth login` / `codex login` | GitHub/GitLab adapters | feeds CI back to the agent | tracks conflicts | – | `automation create --cron` | – | 13k★ | Headless server use undocumented ([README](https://github.com/OrchestratorInc/agent-orchestrator)) |
| **Jean** | **Apache-2.0**; headless `jean-server` on Linux | Yes, (a) | "Mr. Robot (beta)" polls GitHub issues | – | – | – | ✔ | – | 1.3k★ | Plans need human approval ([README](https://github.com/coollabsio/jean)) |
| **goose** | Apache-2.0 | Yes, (a): `claude-acp`/`codex-acp` providers ([docs](https://goose-docs.ai/docs/guides/acp-providers)) | ✗ | – | – | – | `goose schedule` (open bug [#11164](https://github.com/aaif-goose/goose/issues/11164)) | – | Mature | A runtime, not a factory |
| **opencode** | MIT | ChatGPT yes, (b); **Claude no** ("Anthropic explicitly prohibits this", [providers](https://opencode.ai/docs/providers/)) | GitHub agent in Actions with API keys ([github](https://opencode.ai/docs/github/)) | – | – | – | ✔ | – | Very mature | Already a thirdshift Harness |
| **Cline CLI / Kanban** | Apache-2.0 | ChatGPT, (b); Claude Code provider, (a), in the IDE (CLI undocumented) | ✗ (Linear via MCP) | – | – | – | `cline schedule` | – | Mature | ([CLI README](https://github.com/cline/cline/blob/main/apps/cli/README.md)) |
| **Paperclip** | MIT | Claude, (a): `claude_local` adapter ([docs](https://docs.paperclip.ing/adapters/claude-local/)) | its own issue system | – | – | – | ✔ | – | 102k★ | A "run a business" platform, not GitHub-native |
| **Gas Town** | MIT | (a), tmux | Beads, not GitHub | – | – | merge queue | – | – | 18k★ | Built for 20–30 agents; too heavy for 8 GB |
| **Conductor / Emdash / Superset / Claude Squad / Sculptor / Vibe Kanban** | Closed Mac app / Apache-2.0 / ELv2 / AGPL-3.0 / MIT / Apache-2.0 (sunsetting) | All (a) | interactive | – | – | – | Superset Pro only | – | Various | Interactive desktop tools, not unattended factories |
| **SWE-agent, Aider, Sweep, Roo Code** | MIT / Apache-2.0 / source-available / archived | (c) API keys | SWE-agent issue mode | – | – | – | – | – | Stalled or dead | Not candidates |
| **Terragon** | Apache-2.0 snapshot | was (a)/(b) for both | ✔ | ? | ? | ? | ✔ | – | **Shut down 2026-01-16** ([repo](https://github.com/terragon-labs/terragon-oss)) | Needs Postgres, Redis, Stripe and R2 to self-host |

Unless a citation is given in the row, details for OSS projects come from each project's README and repository metadata (`gh api`, 2026-10-14). SaaS details come from the vendor pages linked in the row.

### 4.2 Notes on the top candidates

**Claude Code GitHub Actions** ([docs](https://code.claude.com/docs/en/github-actions), [repo](https://github.com/anthropics/claude-code-action), MIT)

- **Auth.** `claude_code_oauth_token` is a first-class input. Under costs, the docs say: "If you authenticate with an OAuth token, runs use your Claude subscription instead of API billing."
- **Triggers.** It runs "on any GitHub event, including a cron schedule".
- **Caveats.**
  - It runs on GitHub-hosted runners and uses Actions minutes.
  - "An OAuth token is tied to the subscription of the person who ran `claude setup-token`."
  - Commits pushed with the default `GITHUB_TOKEN` don't trigger CI.
  - Bots can't trigger runs unless listed in `allowed_bots`.
- **What it replaces in thirdshift.** The trigger plumbing and the runner, but not the Run's state machine: Claim, Repair budgets, Inherited-failure logic, Self-merge on an exact head, Spec ordering. Those would become workflow YAML plus scripts, which is still building.

**Anthropic Routines / cloud sessions / Projects**

- **Billing.** Plan-billed, and nothing to host.
- **Gaps against thirdshift.**
  - GitHub triggers cover "Pull request" and "Release" only.
  - "The minimum interval is one hour."
  - "A green status… does not mean the task… succeeded" ([routines](https://code.claude.com/docs/en/routines)).
  - Auto-fix handles CI failures and review comments but not merge conflicts ([cloud](https://code.claude.com/docs/en/claude-code-on-the-web)).
  - Projects (Pro/Max beta) adds thread fan-out from a spec and a "Merge it" button ([projects](https://code.claude.com/docs/en/claude-projects)).
- **Best use.** A useful *complement*, for example PR review or nightly architecture notes, but not a replacement for scheduled, label-driven Runs that end in a merge.

**OpenHands Agent Canvas** ([repo](https://github.com/OpenHands/OpenHands), MIT, 90.9k★, v1.27.1)

- **Closest packaged OSS factory.** Triage → `ready-for-dev` → issue-to-PR → `openhands-review` → a deterministic "Delivery Watchdog" that "merges only a current branch with passing independent acceptance statuses and passing Actions runs", each on `*/5` cron ([recipe](https://docs.openhands.dev/openhands/usage/use-cases/software-factory.md)).
- **Doesn't fit as documented.**
  - The recipe asks for "A model with an API key" and per-conversation Docker workspaces.
  - The ACP path that would reuse Claude and Codex logins isn't documented for automations.
  - Six concurrent Docker developer agents in the walkthrough is well beyond this host.
- **Worth a [needs live check]:** can an Automation run an ACP Claude Code agent with `CLAUDE_CODE_OAUTH_TOKEN` in a container?

**OpenAI Symphony** ([repo](https://github.com/openai/symphony), Apache-2.0, 27.7k★)

- **Design.** A language-agnostic SPEC plus an experimental Elixir implementation. It polls a tracker for issues with `required_labels`, runs bounded concurrency with retry and backoff, gives each issue its own workspace, and launches `codex app-server` per issue ([SPEC](https://github.com/openai/symphony/blob/main/SPEC.md)).
- **Status.** "A low-key engineering preview for testing in trusted environments."
- **Fit.** Shaped like thirdshift (a poller plus a per-issue workspace plus an agent), but Codex-only and pre-1.0. Its SPEC is worth reading for ideas even if thirdshift stays.

**Cyrus** ([repo](https://github.com/cyrusagents/cyrus), Apache-2.0)

- **Design.** A self-hosted daemon that takes issues assigned to it (Linear, GitHub, GitLab, Slack), makes a worktree per issue, and runs Claude Code with a `setup-token` OAuth token. That is route (a′) on your own box.
- **Gaps.** No documented CI-repair loop, merge or schedule, and it needs a public webhook endpoint.

## 5. Recommendation

**Hybrid: keep thirdshift, keep it thin, and buy the edges.**

1. **Don't move to Warp Factories.** It bills API tokens plus a per-hour platform fee, can't use either subscription for cloud runs, self-hosts only on Enterprise, has a closed control plane, and never merges (§3).
2. **No hosted product runs the whole loop on subscriptions.**
   - The hosted products that use subscriptions are Anthropic's own (Routines, cloud, Projects, claude-code-action) and OpenAI's own (Codex cloud, `@codex`). Each covers a slice.
   - The third parties that bill your subscriptions are Tembo (both vendors, VM-billed) and, before it shut down, Terragon. Tembo's Claude connection is a hosted service holding your token, closer to the line Anthropic draws than running the binary on your own box **[interpretation]**.
3. **thirdshift's architecture is already the one the terms favour.** It runs the unmodified `claude` and `codex` binaries under Jacob's own logins on his own machine (§2.3). The pieces nobody offers on subscriptions are what thirdshift's code is mostly about: the Repair loop with Inherited-failure logic, the Self-merge on an exact head, Spec runs, and private Security runs ([README.md](../../README.md), ADRs 0004/0006/0008/0010/0015).
4. **Ways to stop hand-rolling the edges:**
   - **PR review and fix-on-comment:** claude-code-action with `CLAUDE_CODE_OAUTH_TOKEN`, or a Claude routine on `pull_request.opened`, instead of new thirdshift features for those. Both run on the Claude plan.
   - **Intake from Slack or Linear, if ever wanted:** Codex's Linear integration or a Claude routine `/fire` endpoint feeding `ready-for-agent` labels, rather than building intake.
   - **Ideas to borrow rather than code to adopt:** Symphony's SPEC (tracker-adapter abstraction, retry and backoff) and OpenHands' Delivery Watchdog (a deterministic merge gate, separate from the agent) both map onto thirdshift concepts.
5. **Revisit when any of these happens:**
   - Warp documents subscription auth for cloud harnesses.
   - OpenHands documents its automations running ACP agents on subscription tokens with a light (non-Docker) workspace.
   - Symphony reaches 1.0 with Claude support.
   - Anthropic changes how `claude -p` subscription usage is billed. A change was announced for June 2026 and then paused ([support](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)). It is the biggest risk to the whole subscription-backed approach, whoever builds the factory.

## 6. Uncertainties

- **Anthropic's help article vs. its legal page.** The help article says third-party apps can use subscription limits; the legal page forbids third parties intermediating credentials. My reading (unmodified binary under your own login: fine; tool-held OAuth: not) is an interpretation. [3rd-party] reports of 2026 enforcement against tools that did their own OAuth support it.
- **OpenAI's public-repo caveat.** OpenAI's CI/CD guidance says not to use ChatGPT auth "for public or open-source repositories". thirdshift's own repository is public on GitHub, and Runs on it execute repository-controlled code under the ChatGPT login. Whether that caveat is about secrets in shared CI or a usage rule is **[unverified]**.
- **Warp's "any model or harness" marketing** ([warp.dev/factories](https://www.warp.dev/factories)) is broader than its docs. The docs list only Warp Agent, Claude Code and Codex, all on API keys. The docs are taken as authoritative.
- **Unclear capabilities.** Tembo's GitHub label triggers and CI iteration, Conductor Cloud's subscription support, Devin's per-ACU price, whether claude-code-action is supported on self-hosted runners, and agent-orchestrator running headless on Linux are all undocumented or conflicting.
- **OpenAI pages not fetched.** The OpenAI Terms of Use and DevDay recap returned 403, so their wording comes from search snippets.
