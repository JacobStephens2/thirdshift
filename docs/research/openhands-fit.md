# OpenHands as a replacement for thirdshift, or a primitive under it

Research date: 2026-10-16.

**Question.** How well would OpenHands (All Hands AI) fit as a replacement for thirdshift: fully, or as a primitive that thirdshift sits on top of? This builds on [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md) (the "earlier survey"). It reuses that note's §1 list of what a replacement must cover and its §2 reading of Anthropic's and OpenAI's subscription terms. It settles the open question the earlier survey left: can OpenHands automations, or its "Software Factory", run agents billed to a subscription? The criteria, in order:

1. **Subscription auth.** Can it run on Claude Pro/Max (the `claude` CLI) and ChatGPT (the `codex` CLI) rather than per-token API keys, while staying within both vendors' terms?
2. **Open source and self-hosted**, preferred.
3. **Buy over build**, preferred.

**Method.**

- Source code was read from shallow clones made on 2026-10-16. Code citations link to a file at a fixed commit:
  - `OpenHands/OpenHands@0452907`
  - `OpenHands/software-agent-sdk@bf9b605`
  - `OpenHands/automation@bc4cca7`
  - `OpenHands/extensions@5aeba9b`
  - `OpenHands/docs@bc442c6`
  - `OpenHands/OpenHands-Cloud@1a2edb8`
- Repository metadata and releases come from `gh api`. npm package facts come from `npm view`.
- Vendor terms were fetched again on 2026-10-16.
- Docs pages are cited by their live URL on `docs.openhands.dev`. The same text is in the `OpenHands/docs` repo at the commit above.
- **[unverified]** marks a claim I could not confirm from a primary source. **[3rd-party]** marks a claim from a secondary source. **[interpretation]** marks my reading of the terms. **[needs live check]** marks something only a real run would settle.
- Nothing was installed or run. The factory host has no Docker or `uv` installed (`which docker uv` on 2026-10-16).

**Out of scope.** Model quality, and the full design of any migration.

## TL;DR

- **OpenHands today is "Agent Canvas".** The flagship MIT repo `OpenHands/OpenHands` is now a TypeScript control centre: a UI, a launcher and an ingress. Two separate MIT repos sit behind it:
  - the Python **Agent Server / SDK** (`software-agent-sdk`);
  - an **Automation service** that runs cron and webhook automations (`automation`).

  Canvas and the automation service are both labelled **beta**. The old Python GUI and the old CLI sit under "Deprecated Projects" in the docs. The SaaS and Enterprise backends are source-available under the PolyForm Free Trial licence, which is **not open source**. (§1)
- **Subscription auth: yes on a self-hosted backend, through ACP agents. The documented Software Factory recipe uses API keys.**
  - **How it works.** An **ACP agent profile** makes the Agent Server spawn `@agentclientprotocol/claude-agent-acp` or `@agentclientprotocol/codex-acp`. The first wraps Anthropic's Claude Agent SDK, which runs the bundled native Claude Code binary. The second launches the official `codex app-server`. On the same machine, either one "reuses that login automatically": `~/.claude/.credentials.json` or `~/.codex/auth.json` ([ACP_AGENTS.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/ACP_AGENTS.md#authentication)).
  - **What unattended work can use it.** The script-bundle automations (issue-to-PR, PR reviewer, triage) start "profile-backed" conversations by `agent_profile_id`. The Agent Server resolves that ID to the profile's agent, ACP included, and the shipped workers' tests cover ACP-backed conversations.
  - **What can't.** The simple prompt and plugin automations are hard-wired to the built-in OpenHands agent and an LLM. So are the Software Factory docs, which say "configure your model and API key".
  - **Status.** ACP inside unattended automations is supported by the code but **not documented** **[needs live check]**. (§2)
- **On terms.** Running ACP Claude Code or Codex on your own box, under your own login, is close to thirdshift's current route (a). The Claude side goes through the Agent SDK rather than `claude -p`. Anthropic currently says the Agent SDK "and third-party apps" may use subscription limits, but its legal page tells *developers* building on the Agent SDK to use API keys. For one person running his own instance this is **[interpretation]**, and a little less clear-cut than `claude -p`.
  - **Avoid OpenHands' own ChatGPT OAuth** (`LLM.subscription_login`). It reuses the Codex CLI's OAuth client ID against `chatgpt.com/backend-api` instead of OpenAI's registered Sign in with ChatGPT flow.
  - **Avoid pasting a Claude token into OpenHands Cloud.** That has a third party hold it. (§2.4)
- **As a replacement it falls well short.** Its Software Factory is four loosely coupled cron automations:
  - triage, which applies `ready-for-dev`;
  - an issue-to-PR scanner;
  - an agent reviewer, which posts `software-factory/*` commit statuses;
  - a deterministic watchdog, which **squash**-merges an exact accepted head.

  It has no Repair loop with budgets, no Inherited-failure or Check re-run logic, and no Base fix. No code repairs merge conflicts. It has no Spec branch or Spec PR over sub-issues, no Claim labels, no Architect runs, no private Security runs and no email notifications. CI repair is left to the agents: a red head pauses review, and revisions are triggered only by a failed review status. (§3)
- **Host fit is acceptable without Docker, poor with it.**
  - Without Docker, the default runtime (`OH_CONVERSATION_RUNTIME=local`) runs the agents on the host with full filesystem access. That is the same trust model as thirdshift ([ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). It needs Node 24+, `uv` and SQLite, and no Postgres or Redis.
  - With Docker, per-conversation containers default to 4 GB and 2 CPUs each, so 8 GB holds one or two at most, and Docker isn't installed on the host.
  - OpenHands has no bwrap-style sandbox. (§4)
- **Recommendation: borrow ideas only, and don't adopt it as a primitive now.** Putting the Agent Server between thirdshift and `claude`/`codex` would add a Python server, Node adapters, a moving beta API and a less direct subscription path. In return thirdshift would get things it doesn't need (a UI, Docker isolation, conversation persistence). Things worth borrowing:
  - the ACP adapters, as a possible future common Harness protocol;
  - the watchdog's exact-head, fail-closed acceptance gate;
  - "profile-backed" idempotent delivery of revisions to a stable conversation per subject.

  Revisit when OpenHands documents ACP profiles in automations and the API leaves beta. (§7)

## 1. What OpenHands is now

| Component | Repo / where | Licence | Status | What it does |
|---|---|---|---|---|
| **Agent Canvas** | `OpenHands/OpenHands` (npm `@openhands/agent-canvas`, image `ghcr.io/openhands/agent-canvas`) | MIT ([LICENSE](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/LICENSE)) | "status-beta" badge; v1.27.1 on 2026-10-12 ([README](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/README.md)) | "The self-hosted developer control center for coding agents and automations". UI, `agent-canvas` launcher, backend switching. "Agent Canvas runs the open source OpenHands agent out-of-the-box, but can use third-party agents such as Claude Code, Codex, Gemini CLI, Pi, and OpenCode" (same README) |
| **Agent Server + SDK** | `OpenHands/software-agent-sdk` | MIT (`gh api`) | v1.55.1 on 2026-10-16 (`gh api …/releases`) | "Python SDK, Agent Server, agents, tools, conversations, workspaces, events, and the canonical server API" ([README, Repository boundaries](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/README.md#repository-boundaries)) |
| **Automation service** | `OpenHands/automation` | MIT ([LICENSE](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/LICENSE)) | "⚠️ **Beta**: … APIs and features may change without notice" ([README](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/README.md)) | "Automation definitions, cron scheduling, webhooks, run history, and dispatching". FastAPI app, with Postgres or SQLite (same README; [scheduler.py](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/openhands/automation/scheduler.py#L8)) |
| **Extensions** (automation catalog, skills, plugins) | `OpenHands/extensions` | MIT (`gh api`) | Active | The prebuilt automations, the Software Factory's four automations among them, are "script bundles" in [`automations/catalog/`](https://github.com/OpenHands/extensions/tree/5aeba9b5c279bd54cac1ae4946bb867685490c55/automations/catalog) |
| **OpenHands Cloud** (SaaS) | app.all-hands.dev | Proprietary SaaS. Its Helm charts are PolyForm Free Trial: "**NOT** an open source license. Usage is limited to 30 days per calendar year" ([OpenHands-Cloud README](https://github.com/OpenHands/OpenHands-Cloud/blob/1a2edb87b8bc0b758173a0cbfe336378db3550d0/README.md)) | GA | Hosted backends. GitHub `openhands` label / `@openhands` resolver ([GitHub integration](https://docs.openhands.dev/openhands/usage/cloud/github-installation)). GitHub *event* automations need the OpenHands GitHub App plus a claimed org ([event automations](https://docs.openhands.dev/openhands/usage/automations/event-automations)) |
| **OpenHands Enterprise** | `OpenHands/enterprise` (public, created 2026-07-27) | PolyForm Free Trial, "commercially licensed … **not** an open source license" (repo README via `gh api`) | Commercial | The "source-available backend" with auth, billing and integrations. Self-hosted in a VPC ([pricing](https://www.openhands.dev/pricing): Enterprise, "Contact us") |
| **Old Local GUI, OpenHands CLI, V0 runtimes** | `OpenHands-CLI` (MIT) etc. | MIT | Listed under "Deprecated Projects" in the docs navigation ([docs.json](https://github.com/OpenHands/docs/blob/bc442c6bc18826f2fe70cb357690ff89b02edea3/docs.json)) | Superseded by Canvas |
| **`agent-canvas` repo** | `OpenHands/agent-canvas` | – | Archived (`gh api`) | Merged into `OpenHands/OpenHands` around 2026-07-27 **[interpretation]**, from the archive date and the enterprise repo's creation date |

Pricing ([openhands.dev/pricing](https://www.openhands.dev/pricing), fetched 2026-10-16):

- Local open source: free, "designed for a single user", bring your own LLM key.
- Individual SaaS: free, 10 conversations a day. Bring your own key, or use OpenHands' LLM provider "at cost".
- Enterprise: custom price; SaaS or self-hosted.

The pricing page does not mention Claude or ChatGPT subscriptions.

## 2. Subscription auth: the concrete answer

### 2.1 The ways an OpenHands agent can reach a model

| Path | What it is | Claude Pro/Max | ChatGPT |
|---|---|---|---|
| **A. ACP agent on a local or self-hosted backend** | "The Agent Server spawns the agent's own CLI as a subprocess and relays each turn to it. The external agent manages its own LLM, tools, and execution" ([ACP_AGENTS.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/ACP_AGENTS.md)). Default commands: `npx -y @agentclientprotocol/claude-agent-acp` and `npx -y @agentclientprotocol/codex-acp` | **Yes**: "A Claude Code login (Pro/Max), from Claude Code's own credential store: … `~/.claude/.credentials.json` on Linux". "The login takes priority over an API key" (same doc, Authentication) | **Yes**: "A ChatGPT login (`codex login`) cached at `~/.codex/auth.json`" (same) |
| **A′. ACP agent in a container or on Cloud** | A fresh container has no login, so Canvas collects `CLAUDE_CODE_OAUTH_TOKEN` ("a Pro/Max OAuth token") or `CODEX_AUTH_JSON` (the contents of `auth.json`). It saves them as Agent Server secrets and injects them at spawn (same doc, "Running ACP agents in a Docker container") | Token via `claude setup-token` | `auth.json` copied in |
| **B. SDK-native ChatGPT OAuth** | `LLM.subscription_login(vendor="openai", …)`: "OAuth PKCE … credentials are cached locally in `~/.openhands/auth/`" ([LLM Subscriptions](https://docs.openhands.dev/sdk/guides/llm-subscriptions)). "OpenAI subscription is the first provider we support" | No (OpenAI only) | Yes, but see §2.4 |
| **C. LiteLLM with an API key** | The built-in OpenHands agent on an LLM profile ([agent profiles](https://docs.openhands.dev/openhands/usage/agent-canvas/agent-profiles)) | No, per-token | No, per-token |

What the ACP adapters actually run, from the npm registry and the adapter repos:

- **`claude-agent-acp`** is "An ACP-compatible coding agent powered by the Claude Agent SDK (TypeScript)", Apache-2.0. It depends on `@anthropic-ai/claude-agent-sdk` (`npm view @agentclientprotocol/claude-agent-acp`). That SDK ships a per-platform "Native CLI binary for @anthropic-ai/claude-agent-sdk on linux-x64" of about 267 MB (`npm view @anthropic-ai/claude-agent-sdk-linux-x64@0.3.300`).
  - So the code path is OpenHands → ACP adapter → Agent SDK → **Anthropic's own Claude Code binary**, which reads the user's own login.
  - The SDK pins the adapter at `0.63.0` for container images ([acp_install_catalog.py L105](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-sdk/openhands/sdk/settings/acp_install_catalog.py#L105)). Locally the `npx -y` default takes the latest (0.93.0 on 2026-10-16).
- **`codex-acp`**: "starts the Codex App Server, translates ACP requests into Codex operations". "The npm package includes a compatible `@openai/codex` dependency" ([codex-acp README](https://github.com/agentclientprotocol/codex-acp)). Version 1.10.0, the one OpenHands pins, depends on `@openai/codex ^0.153.3` (`npm view`).
  - So it is the official Codex binary in `app-server` mode, under the user's `auth.json`.
- **Permissions are bypassed.** "Permission requests from the server are automatically granted" ([ACP Agent guide](https://docs.openhands.dev/sdk/guides/agent-acp)). The session mode is set to `bypassPermissions` (Claude) or `agent-full-access` (Codex) ([acp_providers.py L203-L208](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-sdk/openhands/sdk/settings/acp_providers.py#L203-L208)).
  - Compare thirdshift: it runs Claude in `--permission-mode auto`, with a classifier, not in bypass ([README.md](../../README.md) "Unattended sessions").

### 2.2 Does path A reach unattended automations?

There are two kinds of automation, and they differ.

1. **Prompt and plugin presets** (`POST /v1/preset/prompt`, `/v1/preset/plugin`) always build the built-in agent from an LLM:
   - `llm = workspace.get_llm(...)`, then `agent = get_default_agent(llm=llm, cli_mode=True, …)` ([plugin/sdk_main.py L478](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/openhands/automation/presets/plugin/sdk_main.py#L478); the same in [prompt/sdk_main.py L447](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/openhands/automation/presets/prompt/sdk_main.py#L447)).
   - **No ACP**, so these automations are API-key only (path C, or path B in custom code).
2. **Script-bundle automations**, which include the Software Factory's triage, issue-to-PR and reviewer, can use ACP:
   - **The dispatcher passes the profile.** It exports the automation's `agent_profile_id` as `AUTOMATION_AGENT_PROFILE_ID` ([dispatcher.py L356-L357](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/openhands/automation/dispatcher.py#L356-L357)).
   - **The bundle starts a conversation with it.** The shared `AgentConversationDispatcher` reads that variable and creates the conversation with `StartConversationRequest(agent_profile_id=self.profile_id, …)` ([agent_conversation.py L180, L405-L420](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github/scripts/agent_conversation.py#L180)).
   - **The Agent Server resolves the profile.** `launch_source()` resolves `request.agent_profile_id` to the stored profile's settings ([launch.py L146-L149](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-agent-server/openhands/agent_server/launch.py#L146-L149)). Those settings include `ACPAgentSettings`, which has Codex-specific handling ([L221-L225](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-agent-server/openhands/agent_server/launch.py#L221-L225)).
   - **The docs allow it.** "Agent Profiles define which agent a new Agent Canvas conversation runs with. A profile can use the built-in OpenHands agent or an ACP agent such as Claude Code, Codex, or Gemini CLI" ([agent profiles](https://docs.openhands.dev/openhands/usage/agent-canvas/agent-profiles)). Automations can "use the selected agent profile instead of an LLM profile" ([prebuilt automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations)).
   - **The tests cover it.** The shipped workers' tests include an ACP-backed conversation (`agent={"kind": "ACPAgent", …}`, [test_automation_llm_provenance.py L494-L504](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/tests/test_automation_llm_provenance.py#L494)).

**But the documented recipe doesn't use ACP.**

- The Software Factory walkthrough says: "choose OpenHands, then configure your model and API key". Its three agent profiles each "Select your saved model" ([Software Factory](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)).
- No page documents an ACP agent profile driving an automation (searched the docs repo for "acp" under the automation pages and release notes).

**Answer.** On a self-hosted backend, the Software Factory's agent automations *should* run on your Claude or ChatGPT subscription if you give them ACP agent profiles: the code path is in place end to end. It is undocumented and untested by me **[needs live check]**. Known rough edges:

- **Shared CLI state.** "Concurrent same-provider conversations in one container share a HOME, so they can race on the CLI's auth/config/lock files". Per-conversation isolation (`acp_isolate_data_dir`) is not yet exposed by Canvas ([ACP_AGENTS.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/ACP_AGENTS.md#per-conversation-isolation)). On a local backend, every conversation shares `~/.claude` and `~/.codex`, as thirdshift's Runs do today.
- **No OAuth token beside a base URL.** "Do not set `ANTHROPIC_BASE_URL` alongside the Claude OAuth token" (same doc).
- **Some features are off under ACP.** `tools`, `mcp_config`, `condenser` and `critic` raise `NotImplementedError` on `ACPAgent`. Skills reach it only as text appended to the prompt ([ACP Agent guide](https://docs.openhands.dev/sdk/guides/agent-acp)).

### 2.3 Cloud

On OpenHands Cloud, "OpenHands Cloud runs only Claude Code, Codex, and Gemini CLI" as ACP agents. "On a clean cloud sandbox there's no stored login", so you supply an API key or a token ([ACP_AGENTS.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/ACP_AGENTS.md)). Subscription use on Cloud therefore means handing All Hands a `claude setup-token` token or your `auth.json`. That is route (a′)/(b) territory (§2.4).

### 2.4 Against the vendors' terms

The earlier survey's §2 citations still hold. The Anthropic legal page and support article were re-fetched on 2026-10-16.

| Path | Anthropic | OpenAI |
|---|---|---|
| **A, self-hosted on your own box** | Probably fine **[interpretation]**. See the three points below the table. | Fine on "trusted private infrastructure". `codex-acp` runs the official `codex app-server` with your `auth.json`, the same login `codex exec` uses in thirdshift today ([CI/CD auth](https://learn.chatgpt.com/docs/auth/ci-cd-auth), per the earlier survey §2.2). OpenAI's documented app-server pattern has the *app* own an OAuth token and pass `ACCESS_TOKEN` ([codex app-server](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)), but reusing your own CLI login on your own box is the CLI's own use. |
| **A′, OpenHands Cloud holding your token** | Risky. Developers "may not collect, store, or intermediate Claude.ai credentials or session tokens" ([legal](https://code.claude.com/docs/en/legal-and-compliance)). A hosted Agent-SDK-based adapter holding your token sits closer to that line than the "unmodified Claude Code binary … where a platform hosts Claude Code" carve-out **[interpretation]**. | Not addressed. OpenAI says "Don't expose Codex execution in untrusted or public environments" ([auth](https://learn.chatgpt.com/docs/auth), per the earlier survey). |
| **B, `LLM.subscription_login`** | n/a | **Avoid.** The SDK hard-codes `CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"` and posts to `https://chatgpt.com/backend-api/codex/responses` ([auth/openai.py L122-L125](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-sdk/openhands/sdk/llm/auth/openai.py#L122-L125)). That is the Codex CLI's own client ID **[3rd-party: widely documented in OpenCode and other clients; not confirmed from OpenAI]**. OpenAI's Sign in with ChatGPT flow instead has "OpenAI issue[] a separate client ID during sign-in" and sends inference "to the public Responses API" ([cookbook](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt)). OpenHands is not among the listed open-source integrations (OpenClaw, OpenCode, Pi, T3) ([Sign in with ChatGPT](https://learn.chatgpt.com/docs/sign-in-with-chatgpt)). |
| **C, API key** | Allowed, pay per token | Allowed, pay per token |

Three points behind the Anthropic cell for path A:

- **What the user runs.** The unmodified native binary, under his own login, for his own use, on his own machine.
- **What Anthropic says now.** "You can still use the Claude Agent SDK, `claude -p`, and third-party apps with your subscription limits." The 2026-10-07 notice adds that Max and Team plans "now include monthly API credits" covering "the Claude Agent SDK, claude -p, the Claude API, and Claude Managed Agents" ([support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)).
- **The tension.** The legal page says "Developers building products or services … including those using the Agent SDK, should use API key authentication", and limits assume "ordinary, individual usage of Claude Code and the Agent SDK" ([legal](https://code.claude.com/docs/en/legal-and-compliance)). One person running his own OpenHands instance is not a developer offering login to users, but `claude -p` (thirdshift today) is the plainer case.

**Bottom line for criterion 1.** Self-hosted OpenHands with ACP agents can stay on both subscriptions within the terms, at about the same risk as thirdshift for Codex and slightly more for Claude, since it goes through the Agent SDK rather than `claude -p` **[interpretation]**. Don't use path B, and don't put subscription tokens into OpenHands Cloud.

## 3. Capability map

The thirdshift terms are from [CONTEXT.md](../../CONTEXT.md). "Software Factory" means the four catalog automations in `OpenHands/extensions`: triage, issue-to-PR, PR reviewer and delivery watchdog ([Software Factory doc](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)).

| thirdshift capability | OpenHands equivalent | Gap |
|---|---|---|
| **Run**: Issue URL in, ready PR out, with self-review and Unaddressed findings ([README.md](../../README.md) "What a Run does") | **Issue-to-PR** scanner: polls on cron (default `*/15`) for the trigger label, then dispatches one profile-backed conversation per ready issue. The agent clones, implements, pushes and opens the PR, then adds `openhands-review` ([worker.py](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-issue-to-pr/scripts/worker.py); [manifest](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/automations/catalog/github-issue-to-pr/manifest.json)). A separate **reviewer** agent posts a native review and `software-factory/review` / `software-factory/tests` statuses | Close in shape. Review is a separate automation and agent, not in-session Standards/Spec axes. No Unaddressed-findings convention. Prompts live in Python strings in each bundle, not in a prompt module like `src/prompt.rs` |
| **Pickup run** / **Ready issue** / **Claim limit** | The trigger label (default `openhands`, or `ready-for-dev` in the factory recipe) marks ready work. `priority:high` goes first, then issue number. `Depends on: #n` lines in the body gate it ([github_client.py L352-L362](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-issue-to-pr/scripts/github_client.py#L352-L362)). Dedup is per subject and head, held in automation state | No `in-progress` **Claim** label: progress shows as issue comments and automation state. No Claim limit: the profile-backed worker submits *every* ready issue each poll (worker.py `run()`), capped only by the Agent Server's `max_concurrent_runs` (default 10; beyond it, HTTP 429) ([config.py L344-L353](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-agent-server/openhands/agent_server/config.py#L344-L353)). No Sweep |
| **Triage labels** (`needs-triage`, `ready-for-agent`, …) ([docs/agents/triage-labels.md](../agents/triage-labels.md)) | **Triage** automation: an agent "prioritizes issues, checks dependencies, and establishes acceptance criteria before applying `ready-for-dev`" ([Software Factory](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)) | Agent triage, where thirdshift leaves triage to the Day shift. The label vocabulary is configurable, so the five roles could be mapped **[unverified]** |
| **Repair loop**: merge the Base branch in, watch CI, at most 5 Repairs; conflict Repairs ([README.md](../../README.md) step 6) | Not deterministic. The reviewer won't review a red head: "Review paused: current-head checks did not pass … Fix the checks above and push" ([pr-reviewer worker.py L898-L912](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-pr-reviewer/scripts/worker.py#L898)). Issue-to-PR re-dispatches a revision **only** when `software-factory/review` is `failure`/`error` ([issue-to-pr worker.py L141-L145](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-issue-to-pr/scripts/worker.py#L141)). The docs say "Failed checks lead to revisions" | **Large gap.** No Repair budget, no deterministic CI watch, and no code path that turns a red head without a failed review status into a revision (as far as I read; **[needs live check]**). No conflict Repair: the watchdog calls GitHub's `update-branch` when the PR is behind, and a conflict there is just logged ([watchdog worker.py L63-L92](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-delivery-watchdog/scripts/worker.py#L63)) |
| **Inherited failure**, **Check re-run**, **Declined CI fix**, **Base fix** ([ADR-0008](../adr/0008-inherited-failures-fail-the-run.md), [ADR-0010](../adr/0010-one-check-re-run-before-a-declined-ci-fix.md)) | None. The watchdog needs *every* latest workflow run on the head to be `success`, failing closed ([watchdog worker.py L12-L41](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-delivery-watchdog/scripts/worker.py#L12)) | **Missing.** A red Base branch would stall every PR, with no diagnosis |
| **Self-merge**: `gh pr merge --merge --match-head-commit`, never auto-merge ([ADR-0004](../adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)) | **Delivery Watchdog**, deterministic and with no agent. It requires the PR to be open, non-draft, `mergeable`, on the expected branch prefix and base, with both factory statuses `success`, all statuses green and Actions green. It then calls `PUT /pulls/{n}/merge {"sha": head, "merge_method": "squash"}` (same file L43-L80) | Good and philosophically aligned (exact head, fail closed), but **squash**, not a merge commit, and hard-coded. No issue close, Claim removal or branch delete. Merges only branches matching its own prefix |
| **Foreign commits** in a Merge run | `update-branch` puts the head through a fresh exact-head review; nothing else | Partial |
| **Spec run**: Tickets as sub-issues, blocked-by order, Spec branch, Spec PR, Tickets checklist ([ADR-0006](../adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md)) | `Depends on:` lines order independent PRs into the default branch | **Missing.** No Spec branch, Spec PR, Spec review or sub-issue graph |
| **Architect run** / **Weeding** | Nothing comparable. The catalog has `github-agents-md-maintainer`, `upstream-fork-sync` and `github-stale-ci-pr-closer` ([catalog](https://github.com/OpenHands/extensions/tree/5aeba9b5c279bd54cac1ae4946bb867685490c55/automations/catalog)) | Missing |
| **Security run** / **Fencing** / **Security review**; private advisories ([ADR-0015](../adr/0015-security-findings-stay-private-until-fixed.md)) | A "Vulnerability Remediation" use case and plugin: "Scan dependencies for known CVEs, find hardcoded secrets, and alert your team on a schedule" ([use case](https://docs.openhands.dev/openhands/usage/use-cases/vulnerability-remediation)) | Missing as designed: no reproduction in a throwaway worktree, no draft advisories, no gating of merges on Security findings **[unverified beyond the use-case page]** |
| **Harness, Model, Effort** (Claude Code, Codex, agy, Grok, Muse, OpenCode) | ACP presets for Claude Code, Codex, Gemini CLI, Pi and OpenCode, plus "any stdio ACP server" via Custom. Models are reported by the agent ([ACP_AGENTS.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/ACP_AGENTS.md)). Codex reasoning effort is mapped ([acp_providers.py ~L413](https://github.com/OpenHands/software-agent-sdk/blob/bf9b6057b00e1b2b532ecff508c0d92d818adced/openhands-sdk/openhands/sdk/settings/acp_providers.py#L413)) | agy, Grok and Muse would need ACP servers **[unverified that they have them]**. Claude effort selection over ACP is unclear **[unverified]** |
| **Factory skills** linked into the worktree ([ADR-0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)) | `AgentContext.skills`, appended to the ACP prompt as text ([ACP Agent guide](https://docs.openhands.dev/sdk/guides/agent-acp)); a skills/plugins registry in `extensions` | Different mechanism. Whether `claude-agent-acp` also loads the repo's `.claude/skills/` natively is **[unverified]** |
| **Failed run**: push the work, PR back to draft, Claim kept or released; **Continuation** ([ADR-0002](../adr/0002-existing-issue-branch-means-continue.md)) | The legacy issue-to-PR path commits and pushes what's left and opens the PR itself when the agent didn't. It comments on failure and abandons conversations after 2 h ([main.py](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-issue-to-pr/scripts/main.py) L1-L22, L185). The profile-backed path keeps "a stable conversation for each subject" ([agent_conversation.py](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github/scripts/agent_conversation.py#L171)) | Comparable salvage idea. Draft/ready semantics differ |
| **Scheduling**: the OS's cron starts Passes; one Pass per repo per machine ([ADR-0009](../adr/0009-the-operating-system-schedules-thirdshift.md)) | The Automation service's own scheduler and dispatcher, with cron per automation. GitHub *events* need the Cloud GitHub App and org claiming ([event automations](https://docs.openhands.dev/openhands/usage/automations/event-automations)), so a self-hosted setup polls | Equivalent for polling. Adds a long-running service in place of cron |
| **Run notifications** (Resend email), **Activity log**, **Command/Session logs** ([ADR-0005](../adr/0005-run-notifications-through-resend.md), [ADR-0011](../adr/0011-thirdshift-keeps-a-per-repository-activity-log.md)) | Run history and conversation event streams in Canvas. Slack integrations in the catalog | No email. History lives in the Canvas DB/UI rather than per-repo log files |
| **Unattended and fully trusted** ([ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md)) | ACP auto-approves everything (§2.1). The local runtime "runs the agent-server directly on the machine … full access to your filesystem" ([README](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/README.md#option-1-without-a-sandbox)) | Same trust model. Docker per conversation is an opt-in upgrade (§4) |

**Coverage summary.** OpenHands covers the shape of label → PR → independent review → exact-head merge, and its UI and observability are better than thirdshift's. It lacks the parts thirdshift is mostly made of: the Repair loop and its CI semantics, Spec runs, Architect and Security runs, and Claims. That matches the earlier survey's conclusion (§5.3 there).

## 4. Execution, sandboxing and host fit

- **Runtimes.**
  - `OH_CONVERSATION_RUNTIME=local` is the default: agents run as host processes.
  - `docker` gives each conversation "a dedicated Docker container running a full Agent Server". Containers run as the host UID with `--cap-drop ALL` and `no-new-privileges`, limited by `OH_CONVERSATION_CONTAINER_MEMORY` (default `4g`), `…_CPUS` (default `2.0`) and `…_PIDS_LIMIT` (512) ([docker execution](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/docker-execution)).
  - Remote options are VM, Kubernetes, Modal and DigitalOcean backends ([backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends)), plus an SDK Apptainer sandbox ([apptainer](https://docs.openhands.dev/sdk/guides/agent-server/apptainer-sandbox)).
  - There is **no bwrap, Landlock or nsjail option** (no matches in the SDK, Canvas or automation sources).
- **Minimum host.** "2 vCPU / 4 GB RAM is plenty for a single user" ([SELF_HOSTING.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/SELF_HOSTING.md)). The launcher needs Node 24+ and `uv` (README). The local automation DB is SQLite: `AUTOMATION_DB_URL: sqlite+aiosqlite:///…/automations.db` ([dev-static.mjs L371](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/scripts/dev-static.mjs#L371)). Postgres is only for multi-process deployments ([scheduler.py](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/openhands/automation/scheduler.py#L8)). No Redis.
- **On the 4-core / 8 GB / no-swap host.** It has about two building Runs today (project memory `tracewake-machine-capacity`), Node 26 and no Docker or `uv`.
  - **Local runtime.** It adds a resident Agent Server (Python), the automation service (Python), the Canvas ingress/static server (Node) and one Node ACP adapter per conversation, on top of the same `claude`/`codex` processes thirdshift runs. My estimate of the extra memory is roughly 0.5–1 GB **[unverified; not measured]**. Concurrency would still be bound by builds and tests, so about 2 conversations, enforced by setting `max_concurrent_runs`, not by the factory's own logic.
  - **Docker runtime.** Not installed. At the default 4 GB per container, at most one or two containers fit. The recipe's "six developers ran concurrently in Docker" ([Software Factory](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)) is far beyond this host.
  - **Workspace collisions.** On the local runtime, the profile-backed issue-to-PR worker tells the agent "The workspace starts empty. Clone the repository into it with `gh repo clone … .`" ([issue-to-pr worker.py L26-L33](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-issue-to-pr/scripts/worker.py#L26)). Every conversation is created on the run's `WORKSPACE_BASE` ([agent_conversation.py L189-L193](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github/scripts/agent_conversation.py#L189)). Whether two issues dispatched by one poll get separate directories is unclear. The README warns that "Conversations using the same host workspace still share those files" **[needs live check]**.
  - **Sandboxing.** Security reviews' bwrap sandbox (project memory `tracewake-security-sandbox`) has no counterpart. OpenHands' isolation story is Docker, which this host doesn't run.

## 5. Self-hosting and operational burden

- **Single user, local.** `npm install -g @openhands/agent-canvas && agent-canvas` "starts the full local stack": frontend, Agent Server, automation backend and ingress, bound to 127.0.0.1 with an auto-injected session key ([README](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/README.md)).
- **On a server.** The self-hosting guide asks for firewalling, `LOCAL_BACKEND_API_KEY`, and optionally TLS or Tailscale ([SELF_HOSTING.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/SELF_HOSTING.md)). A systemd unit for an always-on service would be yours to write **[unverified whether one is shipped]**.
- **Multi-user and governance** (SSO, orgs, GitHub App event routing, unlimited concurrency) are Cloud or Enterprise. The Enterprise backend is PolyForm Free Trial licensed (§1). None of that is needed for one user.
- **Burden compared with thirdshift.** One static Rust binary plus cron ([ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md), [ADR-0003](../adr/0003-static-binaries-through-github-releases.md), [ADR-0009](../adr/0009-the-operating-system-schedules-thirdshift.md)) becomes three long-running services across Python and Node, plus `npx`-fetched ACP adapters that float to the latest version on local backends unless pinned. Upgrades must keep Canvas, the Agent Server, the automation service and the extensions in step: Canvas has a "minimumAgentServer" ([ACP_AGENTS.md](https://github.com/OpenHands/OpenHands/blob/045290739f0af2ae4cc89d2a209c6bd930bb81ee/docs/ACP_AGENTS.md)), and release notes list bundled dependency bumps ([v1.24.0 notes](https://docs.openhands.dev/openhands/usage/agent-canvas/release-notes/v1.24.0)).

## 6. Maturity

- **Release cadence.**
  - Canvas: 25 tagged releases, v1.6.1 on 2026-07-28 to v1.27.1 on 2026-10-12, so roughly two minor versions a week (`gh api repos/OpenHands/OpenHands/releases`; [release notes](https://docs.openhands.dev/openhands/usage/agent-canvas/release-notes/v1.27.0)).
  - SDK and Agent Server: v1.51.0 on 2026-10-03 to v1.55.1 on 2026-10-16, five releases in two weeks.
- **Activity.** `OpenHands/OpenHands` has 91k stars and was pushed to on 2026-10-16. `software-agent-sdk` has 1.3k stars and `automation` 41 (`gh api`).
- **Churn and breaking changes.**
  - The product pivoted in mid-2026: the `agent-canvas` repo was archived 2026-07-27, and the old GUI and CLI were deprecated (§1).
  - The automation service says its APIs "may change without notice".
  - The SDK removes fields on a schedule ("`acp_env` … removed in 1.29.0", [ACP Agent guide](https://docs.openhands.dev/sdk/guides/agent-acp)).
  - Canvas features gate on Agent Server versions, for example "appears only when the connected backend reports agent-server `1.31.0` or later" ([agent profiles](https://docs.openhands.dev/openhands/usage/agent-canvas/agent-profiles)).
- **Verdict.** Very active and well staffed, but a **moving target**, especially the automation and ACP-in-automation path this use would depend on.

## 7. Options and recommendation

| Option | What thirdshift would gain | What it would cost or lose | Verdict |
|---|---|---|---|
| **Replace thirdshift with the Agent Canvas Software Factory**, on ACP profiles | A UI, run history, Slack, a reviewer agent and an exact-head merge gate, with no Rust to maintain for those parts | Loses the Repair loop semantics, Inherited failures, Check re-run, Base fix, Spec runs, Architect/Weeding, Security/Fencing, Claims and email. Merge commits become squash. ACP in automations is undocumented. Three beta services. The repair gap lands exactly where a merging factory needs rigour (§3) | **No** |
| **Adopt the Agent Server as a primitive**, a new thirdshift Harness that drives conversations over its REST API | One API across Claude Code, Codex, Gemini, Pi and OpenCode. Persisted, viewable conversations. Optional Docker isolation | thirdshift already drives six Harnesses directly. It adds a Python server and Node adapters between thirdshift and `claude`, and moves Claude from `claude -p` (auto mode, the plainest terms reading) to the Agent SDK in `bypassPermissions`. Session logs, Resume, Model/Effort and Factory-skill wiring would all need rework. Moving beta API | **Not now** |
| **Use the Automation service as the scheduler** for `thirdshift pickup` etc. (a custom-tarball automation can run any `entrypoint`: [schemas.py L511-L520](https://github.com/OpenHands/automation/blob/bc4cca7c7e2eab5cb7d0ff671bec5dd113d3e2c2/openhands/automation/schemas.py#L511)) | A UI over pass history | Replaces a cron line with a service. Thirdshift's Activity log already covers this ([ADR-0011](../adr/0011-thirdshift-keeps-a-per-repository-activity-log.md)) | **No** |
| **Use the ACP adapters directly** as a future Harness transport, without OpenHands | One protocol (JSON-RPC over stdio) for many CLIs; the adapters wrap the official binaries (§2.1) | Must be weighed against `claude -p` / `codex exec`, which thirdshift already parses. The Claude path moves to the Agent SDK | **Watch.** Worth a note in [harness-candidates.md](harness-candidates.md) if Harness count keeps growing |
| **Borrow ideas** | (1) The watchdog's fail-closed exact-head gate: all latest workflow runs per (workflow, event, branch), plus independent named statuses ([watchdog worker.py](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github-delivery-watchdog/scripts/worker.py)). (2) "Deliver one revision at a time to a stable conversation for each subject", deduplicated by head ([agent_conversation.py](https://github.com/OpenHands/extensions/blob/5aeba9b5c279bd54cac1ae4946bb867685490c55/skills/github/scripts/agent_conversation.py#L171)). (3) Per-role, repository-scoped fine-grained PATs for triage, developer and reviewer ([Software Factory](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)). (4) The reviewer as an independent agent publishing commit statuses | None | **Yes** |

**Recommendation: borrow ideas only. Keep thirdshift as the orchestrator and keep calling `claude` and `codex` directly.**

- **Criterion 1.** OpenHands *can* run on both subscriptions when self-hosted, through ACP. That is real progress since the earlier survey. But it reaches them through more layers than thirdshift does, and the unattended part is undocumented.
- **Criterion 2.** It passes for the MIT core. Cloud and Enterprise fail.
- **Criterion 3.** "Buy" would only cover the parts thirdshift finds easy (pickup, PR, merge gate). The rest would still have to be built, in someone else's beta codebase.

**Revisit when:**

- OpenHands documents ACP agent profiles in automations, and the automation service drops its beta label;
- the Software Factory gains a deterministic CI/conflict repair loop;
- or thirdshift's Harness maintenance becomes the bottleneck, in which case ACP, with or without OpenHands, is the thing to evaluate.

## 8. Uncertainties and live checks

- **ACP in an automation, end to end [needs live check].** Create an ACP Claude Code agent profile on a local Canvas, then point the issue-to-PR bundle at it. Check two things: the conversation runs on the `~/.claude` login, and `claude` shows plan usage, not API billing. Repeat with Codex.
- **Which Claude allowance it draws [unverified].** Since 2026-10-07, Max plans include monthly API credits that "cover the Claude Agent SDK, claude -p". The support article does not say how those credits and the subscription limits interact ([support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)). The same ambiguity applies to thirdshift's `claude -p`.
- **Agent SDK and "developers" [interpretation].** The legal page's line that Agent SDK developers "should use API key authentication" is read here as aimed at products serving other users, not one person's self-hosted instance. Anthropic does not say this explicitly.
- **CI-repair behaviour of the factory [needs live check].** The docs say "Failed checks lead to revisions". The code I read re-dispatches only on a failed `software-factory/review` status, and the reviewer pauses on red heads.
- **Local workspace sharing [needs live check].** It is unclear whether two issues dispatched by one poll on the local runtime share a working directory (§4).
- **Extra memory of the OpenHands services [unverified].** Not measured.
- **Codex OAuth client ID [3rd-party].** That `app_EMoamEEZ73f0CkXaXp7hrann` is the Codex CLI's client is widely reported, but I did not confirm it in OpenAI's code or docs.
- **Merge-into-default after the pivot [interpretation].** That the old Python OpenHands app now lives on as the PolyForm `enterprise` backend is inferred from repo dates and READMEs, not from an announcement.
