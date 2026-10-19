# Temporal as a base for a thirdshift-like factory

Research date: 2026-10-14.

**Question.** Does [Temporal](https://temporal.io/) offer primitives or products that could be used to build something like thirdshift? Specifically:

- Which of them could run coding agents on the user's own subscriptions?
- Which of them could carry a coding process, a software factory?

thirdshift is a Rust CLI. It turns an **Issue URL** into a ready-for-review pull request by running headless **Harness** sessions (`claude`, `codex`, `agy`, `grok`, `muse`, `opencode`). Each **Run** works in its own **Worktree** on the user's machine, with the user's own logins, not API keys. A Run goes through stages: the implement session, reviews, **Repairs** and the **Self-merge**. It resumes sessions by id (**Resume**). The operating system's scheduler starts each **Pass** ([ADR 0009](../adr/0009-the-operating-system-schedules-thirdshift.md)). The host has 4 cores and 8 GB, enough for about two building Runs at once.

**Method.**

- **Docs:** `temporalio/documentation` at commit [`c245e77`](https://github.com/temporalio/documentation/tree/c245e77084206d2ec3845a73a1d4ea778bc42e31) (2026-10-14), cloned and grepped. That repository is the source of docs.temporal.io, and every docs.temporal.io link below was checked live (each returned 200, where a made-up path returns 404). Each page's release-stage banner (`<ReleaseNoteHeader type=…>`) was read from that source.
- **Temporal's site:** the pricing page [temporal.io/pricing](https://temporal.io/pricing) and the [changelog](https://temporal.io/changelog), both read on 2026-10-14. The pricing page carries no "last updated" date.
- **GitHub, via the API:**
  - `temporalio/sdk-rust`: README at tag `v1.1.0`, and the release notes for `v1.0.0` and `v1.1.0`.
  - Latest releases: `temporalio/temporal` v1.32.1 (2026-10-08), `temporalio/cli` v1.9.1 (2026-09-14), sdk-python 1.34.0, sdk-typescript v1.24.0, sdk-go v1.49.0, sdk-java v1.40.0, sdk-dotnet 1.20.0, sdk-ruby v1.9.0 and sdk-php v2.19.
  - The READMEs of `temporalio/pi-temporal` (head `50a3700`, 2026-10-12), `temporalio/claude-temporal-plugin`, `temporalio/agent-poc-quickstart` and `temporalio/ai-cookbook`.
  - crates.io's API for `temporalio-sdk`.
- **Anthropic:** one primary source for Claude subscription terms, [Claude Code: Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance), read 2026-10-14.
- **No tests.** The `temporal` CLI is not installed on this machine, and nothing was downloaded or run. Resource use is therefore not measured.
- **Confirmation tags.** Each fact says how it was confirmed: **docs** (docs.temporal.io), **source** (repository code or README), **release notes**, **changelog**, **pricing page**, or **Anthropic docs**. Everything under "Assessment" is the author's judgment, not a cited fact.

**Out of scope.**

- Building a prototype, and whether to adopt Temporal at all. The assessment in §7 is advice only.
- Other durable-execution engines (Restate, Inngest, DBOS, Hatchet and others).
- OpenAI's and other vendors' terms for headless use of their subscriptions. Only Anthropic's are cited, briefly.

## TL;DR

- **The primitives fit, but no product does the work.** Temporal's pieces cover what a factory needs:
  - a durable Workflow per Run, with Activities for each session and each `git`/`gh` step
  - Child Workflows for a **Spec run**'s **Tickets**
  - Schedules for Passes
  - Signals and Updates for **Day shift** decisions
  - Worker slot limits as a concurrency cap
  - heartbeats and retries for crash recovery
  - a Web UI

  Temporal sells durable execution, not a coding factory. Nothing first-party turns an issue into a pull request, and nothing runs Claude Code or Codex CLIs (§3).
- **Subscriptions are not Temporal's concern.** Temporal bills only Actions, storage and support (pricing page), and "does not run or manage your applications or Workers" (docs). A self-hosted Worker on the user's machine can run an Activity that execs `claude -p` or `codex exec` under the user's login. Temporal adds nothing that forces API keys.

  The agent-framework integrations are different. OpenAI Agents SDK, the Claude cookbook loop, Pi and the others call model APIs with API keys. Anthropic's docs say developers building with the Agent SDK "should use API key authentication". They also say that running the unmodified Claude Code binary signed in with the user's own subscription is not restricted (§2.4). So to keep subscriptions, a Temporal-based factory must wrap the CLIs, as thirdshift does, not an agent framework.
- **What exists for AI agents** (§3):
  - a "Durable AI" docs hub and the AI Cookbook (samples)
  - about a dozen agent-framework integrations. OpenAI Agents SDK for Python has been in public preview since 2025-07-30, and most of the others are public preview or pre-release.
  - Temporal skills and a Claude Code / Codex / Cursor plugin for agents that *write* Temporal code (public preview)
  - Serverless Workers. Bedrock AgentCore support is pre-release.
  - `temporalio/pi-temporal`, a template that runs the Pi coding agent's loop on Temporal, with no release stage stated. It is the nearest thing to "a coding agent on Temporal", and it uses provider API keys.
- **There is a GA Rust SDK.** `temporalio-sdk` 1.0.0 was released on 2026-09-04 and 1.1.0 on 2026-10-06, with MSRV 1.92. It covers Workflows, Activities, Workers, Signals, Queries, Updates, Timers, Child Workflows, Continue-As-New, heartbeats, patching, cancellation and replay testing. Defining Nexus handlers is "added later", and there is no Worker Session API (Go only). The Core SDK under the TypeScript, Python, .NET and Ruby SDKs is the same Rust repository.
- **Deployment options:**
  - **Dev server:** `temporal server start-dev` is one process with the Web UI. It keeps state in memory, or in SQLite with `--db-filename`. SQLite "is meant only for development and testing, not production usage".
  - **Production self-hosting:** needs PostgreSQL, MySQL or Cassandra. The Docker Compose example adds Elasticsearch.
  - **Footprint:** Temporal publishes no hardware minimum, and none was measured here.
  - **Temporal Cloud:** pay-as-you-go from $50 per million Actions, with no base fee or minimum. Storage is $0.042 per GB-hour active and $0.00105 per GB-hour retained. A trial gives $150 in credits for 90 days. Each Activity heartbeat that reaches the server is an Action.
- **Assessment:** not worth adopting now.
  - **What Temporal would add:** automatic recovery of a Run after a crash or reboot (today a `kill -9` leaves the issue `in-progress` for a human), a UI, and coordination across machines.
  - **What it would cost:**
    - a long-lived Worker daemon, which reverses ADR 0009
    - a server to run, or Cloud payloads to encrypt so that a **Security finding** never leaves the machine in clear text
    - determinism and versioning discipline across every `thirdshift update`
    - a rewrite of the Delivery and Repair loop as Workflow code

  Revisit if crash recovery or a multi-machine fleet becomes a goal (§7).

## 1. Temporal's primitives, and where each would sit in thirdshift

Temporal's model: application code runs in **Workers** on your own machines. Workers poll **Task Queues** on a Temporal Service and send results back ([Workers](https://docs.temporal.io/workers), [Task Queues](https://docs.temporal.io/task-queue): "Workers poll for Tasks in Task Queues via synchronous RPC"; docs). Polling is outbound, so a Worker on a home machine needs no open inbound port. That was the reason ADR 0009 gave for rejecting webhooks (inference from the polling model).

| Primitive | What it is (cited) | Where it would sit in thirdshift |
|---|---|---|
| **Workflow** | Durable, replayed code. It "must be deterministic": no direct I/O, threads, random numbers or system time in Workflow code ([Workflow Definition](https://docs.temporal.io/workflow-definition), docs; [Rust SDK README](https://github.com/temporalio/sdk-rust/blob/v1.1.0/crates/sdk/README.md), source). Workflows have no Retry Policy by default ([Retry Policies](https://docs.temporal.io/encyclopedia/retry-policies), docs). | One Workflow per **Command**: a Run, Spec run, Architect run, Pickup run or Security run. Its code is the decision logic only: which session next, whether to Repair, whether to Self-merge. |
| **Activity** | Code that may do I/O and may run more than once. The default Retry Policy is a 1 s initial interval, backoff 2.0, a maximum interval of 100 × initial, and unlimited attempts ([Retry Policies](https://docs.temporal.io/encyclopedia/retry-policies), docs). | Each agent session (implement, review, Repair, Security review), and each `git`, `gh`, CI poll and email send. |
| **Activity timeouts** | Schedule-To-Start and Schedule-To-Close default to ∞. Start-To-Close "has no default value of its own", and one of Start-To-Close or Schedule-To-Close must be set ([Detecting Activity failures](https://docs.temporal.io/encyclopedia/detecting-activity-failures), docs). | A session can run for hours, so set Start-To-Close well above the longest session and rely on a short Heartbeat Timeout to catch a dead Worker. |
| **Heartbeat** | Tells the service the Activity is alive. It can carry progress details "that can be used by the Activity Execution should a retry occur". Cancellation is delivered only through heartbeats: "Activities that don't Heartbeat can't receive a Cancellation". Heartbeats are throttled to 0.8 × Heartbeat Timeout, else 30 s by default and 60 s at most (same page, docs). | Heartbeat while `claude -p` streams events, and put the **session id** in the heartbeat details. A retried Activity can then resume the session (`--resume <id>`) rather than start over. That is a crash-recovery Resume, broader than thirdshift's **Resume**, which is only for killed background work. One caveat: details survive only if the heartbeat reached the server before the crash (same page). |
| **Task Queue routing** | A dedicated Task Queue sends Activities to a dedicated host ([Task Routing](https://docs.temporal.io/task-routing), docs). The [Worker-Specific Task Queues](https://docs.temporal.io/design-patterns/worker-specific-taskqueue) pattern has the first Activity return a host-specific queue name for later Activities that need local files (docs). A Worker Session API exists only in Go ([Go sessions](https://docs.temporal.io/develop/go/workers/sessions), docs). | **Worktree affinity.** A Run's worktree is on one machine, so every Activity of that Run must go to that machine's queue. On a single host this is trivial. Across hosts, use the worker-specific queue pattern. |
| **Sticky execution** | Caches *Workflow* state on the Worker that last ran it, through an automatic per-Worker queue ([Sticky Execution](https://docs.temporal.io/sticky-execution), docs). It is a Workflow-task cache, not Activity affinity. | Not what pins a worktree. Use explicit Task Queues as in the row above. |
| **Worker slots / tuner** | `MaxConcurrentActivityExecutionSize` caps the Activity tasks one Worker runs at once ([Worker tuning reference](https://docs.temporal.io/develop/worker-tuning-reference), docs). Fixed-size slot suppliers are recommended for "tasks that have variable, or very high, per-task resource needs" ([Runtime tuning](https://docs.temporal.io/develop/worker-performance/runtime-tuning), docs). Rust's `WorkerOptions` takes a `tuner` ([Rust worker process](https://docs.temporal.io/develop/rust/workers/worker-process), docs). There is also Task Queue rate limiting and [priority and fairness](https://docs.temporal.io/develop/task-queue-priority-fairness) (docs). | The **~2 building Runs** cap becomes a session Task Queue whose Worker has a fixed two Activity slots. Work beyond that waits in the queue rather than being skipped. |
| **Signals / Queries / Updates** | Signals are asynchronous writes, Queries are reads, and Updates are synchronous writes that return a result ([Message passing](https://docs.temporal.io/encyclopedia/workflow-message-passing), docs). The [Approval](https://docs.temporal.io/design-patterns/approval) and [Resumable Activity](https://docs.temporal.io/design-patterns/resumable-activity) patterns park a Workflow until a human signals (docs). | Day shift decisions: triaging an **Architect idea**, allowing a **Base fix**, a fix for a **Security finding**. thirdshift keeps these as GitHub labels today. Signals would need a bridge from GitHub, or would duplicate that state. |
| **Timers** | Durable sleeps ([Timers](https://docs.temporal.io/workflow-execution/timers-delays), docs). | The Ready issue settling window and CI-watch intervals. Each Timer is a billable Action on Cloud ([Actions](https://docs.temporal.io/evaluate/cloud/actions), docs). |
| **Child Workflows** | Workflows started by a Workflow, with a [Parent Close Policy](https://docs.temporal.io/parent-close-policy) ([Child Workflows](https://docs.temporal.io/child-workflows), docs). | A **Spec run** starts one child per **Ticket** as its blockers close. A Run starts a **Base fix** as a child. |
| **Workflow Id policies** | Workflow Id uniqueness within a Namespace, governed by Reuse and Conflict policies. A conflicting start fails with "Workflow execution already started" by default ([Workflow Id](https://docs.temporal.io/workflow-execution/workflowid-runid), docs). | A Workflow Id like `run/<owner>/<repo>/<issue>` gives at most one open Run per issue on every machine that shares a server. Today that job falls to the per-machine Pass lock plus the **Claim**. |
| **Schedules** | Cron-like starts with an Overlap Policy, Pause and Backfill ([Schedules](https://docs.temporal.io/schedule), docs). The Rust client has schedule APIs (`crates/client/src/schedules.rs`, source). | They would replace the crontab lines for Pickup, Architect and Security passes. The Overlap Policy would take over the Pass lock's "skip if one is running". |
| **Continue-As-New** | Starts a fresh Event History under the same Workflow Id. History is capped at 51,200 events or 50 MB ([Continue-As-New](https://docs.temporal.io/workflow-execution/continue-as-new), [limits](https://docs.temporal.io/workflow-execution/limits), docs). | Only for long-lived loops, for example a Workflow per repository that keeps picking up work. A single Run stays far below the cap. |
| **Versioning** | Patching, and Worker Versioning with pinned or auto-upgrade routing ([Worker Versioning](https://docs.temporal.io/worker-versioning), docs). Rust 1.1.0 adds `versioning_override`, which needs Server ≥ 1.28 or 1.32 ([release notes](https://github.com/temporalio/sdk-rust/releases/tag/v1.1.0)). | Needed whenever `thirdshift update` changes Workflow code while a Run is open. That hazard does not exist today, because each Pass is a fresh process. |
| **Nexus** | Connects Temporal applications across Namespaces, through endpoints and service contracts ([Nexus](https://docs.temporal.io/nexus), docs). | No use for one user on one Namespace. |
| **Standalone Activities** | Activities started without a Workflow. GA per the [changelog](https://temporal.io/changelog) (2026-09-15); some operator commands are still public preview ([Standalone Activity](https://docs.temporal.io/standalone-activity), docs). | A one-off session job with retries and visibility, without writing a Workflow. |
| **Visibility / Web UI** | The dev server bundles the Web UI ([dev server](https://docs.temporal.io/develop/run-a-development-server), docs). It shows Event History, pending Activities and Search Attributes ([Visibility](https://docs.temporal.io/visibility), docs). | Overlaps the **Command log** and **Activity log**, and adds live state. **Session logs** stay on disk. |
| **Limits** | 2 MB per request payload, 4 MB per gRPC message, and 2,000 incomplete Activities per Workflow ([Cloud limits](https://docs.temporal.io/evaluate/cloud/limits), docs). | Session transcripts must not be Activity results. Return paths and summaries instead. |

## 2. Running coding agents on the user's subscriptions

### 2.1 Temporal has no notion of LLM billing

- The Cloud pricing model is Actions, storage and a support plan, nothing else ([pricing page](https://temporal.io/pricing); [Cloud pricing](https://docs.temporal.io/cloud/pricing), docs). Actions are Workflow, Activity, Timer, Signal, Query and Schedule operations (pricing page).
- Temporal Cloud "does not run or manage your applications or Workers. Applications written with a Temporal SDK run in your computing environment" ([Cloud security](https://docs.temporal.io/evaluate/cloud/security), docs).
- The docs' own guidance for LLM calls is "you make LLM calls in your Activities" ([OpenAI Agents SDK integration](https://docs.temporal.io/develop/python/integrations/openai-agents), docs). Which credentials those calls use is up to the Activity.

### 2.2 Shelling out to `claude -p` or `codex exec` from an Activity

Nothing in Temporal stops it. An Activity is ordinary code on the user's Worker. A Rust Activity is an `async fn` on Tokio ([Rust SDK README](https://github.com/temporalio/sdk-rust/blob/v1.1.0/crates/sdk/README.md), source), so it can spawn a process exactly as thirdshift's `session` module does today. It runs as the Worker's OS user, so `claude` and `codex` find that user's login. Constraints to design for:

- **Determinism.** The subprocess, its stream parsing, `git` and `gh` must all be Activities. Workflow code may not do I/O, read the clock or use `tokio::select!`/`tokio::spawn`; it must use the SDK's `select!`, `join!` and `join_all` ([Rust SDK README](https://github.com/temporalio/sdk-rust/blob/v1.1.0/crates/sdk/README.md), source). The [GA announcement](https://temporal.io/blog/build-durable-applications-rust-temporal-rust-sdk-now-generally-available) adds that the nondeterminism detector cannot catch `SystemTime::now()` or `rand::random()`.
- **Hour-long sessions.** Set a Start-To-Close longer than any session, a short Heartbeat Timeout, and heartbeat from the stream reader (§1). Without heartbeats a dead Worker is noticed only when Start-To-Close expires, and cancellation never reaches the Activity ([Detecting Activity failures](https://docs.temporal.io/encyclopedia/detecting-activity-failures), docs). On cancellation the Activity itself must kill the `claude` process tree. Temporal does not know the child exists (inference).
- **Retries re-run the session.** The default Retry Policy has unlimited attempts (§1). A session that fails for a model or plan reason would be re-run forever unless the error is marked non-retryable or attempts are capped. That matches thirdshift's fixed Repair budgets only if configured to.
- **Worker-local state.** A worktree and the agent CLI's own session store (`~/.claude`, `~/.codex`) live on one host, so all of a Run's Activities must stay on that host's Task Queue (§1). This is the Worker-Specific Task Queues pattern.
- **Payload size.** Each request payload is capped at 2 MB (§1). Keep the transcript on disk, as the **Session log** is today, and return only the session id, the final message and usage.

### 2.3 Does Temporal Cloud push you towards API keys?

Only for Temporal itself. A Worker authenticates to Cloud with a Namespace API key or mTLS certificate ([Cloud security](https://docs.temporal.io/evaluate/cloud/security), docs). That is a Temporal credential, not an LLM one.

The real concern is data. Workflow inputs, Activity results and heartbeat details are stored by the service. "Data conversion runs on your Workers", and a Payload Codec can encrypt them so "Temporal Cloud cannot see or decrypt your data" (same page; [Payload Codec](https://docs.temporal.io/payload-codec), docs). A factory that handles **Security findings**, which [ADR 0015](../adr/0015-security-findings-stay-private-until-fixed.md) keeps private until fixed, would need that codec on Cloud.

### 2.4 Anthropic's terms

[Claude Code: Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance) (Anthropic docs, read 2026-10-14) says:

- **OAuth is for ordinary use.** OAuth "is intended exclusively for purchasers of Claude Free, Pro, Max, Team, and Enterprise subscription plans and is designed to support ordinary use of Claude Code and other native Anthropic applications."
- **Products use API keys.** "Developers building products or services that interact with Claude's capabilities, including those using the Agent SDK, should use API key authentication."
- **The unmodified CLI is fine.** The restriction does not "prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription".
- **Usage limits assume individual use.** "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK."

So a Temporal Activity that execs the user's own `claude` binary is in the same position as thirdshift today. A Temporal *agent-framework* integration that calls Claude's API directly is in API-key territory. This is the distinction, not legal advice.

## 3. First-party AI and agent offerings

Nothing first-party is a coding factory, and nothing runs Claude Code or Codex CLI sessions. What does exist:

| Offering | What it is | Status |
|---|---|---|
| [Durable AI](https://docs.temporal.io/ai) docs hub | Patterns for agents, pipelines and "internal agent platforms", a featured video and Code Exchange picks (docs) | Docs |
| [AI Cookbook](https://github.com/temporalio/ai-cookbook) | Recipes: agentic loops for Claude and OpenAI, human-in-the-loop, guardrails, MCP, deep research, multi-agent with ADK (source). The Claude loop wraps the Anthropic Messages API with an `AsyncAnthropic` client, which means an API key. | Samples |
| OpenAI Agents SDK, Python | The agent loop runs in the Workflow and model calls run as Activities. Package `temporalio-openai-agents`, needs Python SDK ≥ 1.33.0 ([docs](https://docs.temporal.io/develop/python/integrations/openai-agents)). `SandboxAgent` gives an agent a shell and filesystem. `UnixLocalSandboxClient` runs commands on the Worker host, "only for local development with trusted prompts". `LocalShellTool` is not supported (same page). | Public preview since 2025-07-30 ([changelog](https://temporal.io/changelog/open-ai-agents-sdk-integration-pp)). Sandbox public preview 2026-04-16 ([changelog](https://temporal.io/changelog)). The page has no stage banner now, but no GA announcement was found. |
| OpenAI Agents SDK, TypeScript | Same idea ([docs](https://docs.temporal.io/develop/typescript/integrations/openai-agents)) | Pre-release banner (source) |
| Other framework integrations ([catalog](https://docs.temporal.io/integrations)) | Python: Google ADK, Deep Agents, Google GenAI, LangGraph, Strands, OpenRouter, LangSmith. TypeScript: Vercel AI SDK, Google ADK, Strands, LangSmith. Go: Google ADK. Java: Spring AI. Pydantic AI and Mastra are listed but documented by those projects themselves. | Pre-release: ADK (Python, TS), Deep Agents. Public preview: GenAI, LangGraph, Strands (Python), Vercel AI SDK, Spring AI, LangSmith. Experimental: Strands (TS). Per banners in source. |
| [Develop with AI](https://docs.temporal.io/with-ai) | Skills and a plugin for Claude Code, Codex and Cursor that help a coding agent *write and operate* Temporal code, plus a knowledge-base MCP server | Developer skill public preview 2026-03-23 ([changelog](https://temporal.io/changelog/product-area/python-sdk)). The [Claude plugin](https://github.com/temporalio/claude-temporal-plugin) README says "Public Preview". |
| [`temporalio/pi-temporal`](https://github.com/temporalio/pi-temporal) | "Temporal executor for the Pi coding agent: kill the worker mid-turn and another machine finishes it." A `piSession` Workflow per session, a `runStep` Activity per step (or one Activity per model call and tool call), Schedules for unattended turns, token and wall-clock budgets, and a `fleet` profile that ships project files between Workers. It recovers an interrupted tool call as an "unknown outcome" rather than re-running it. Provider is `openai` or `anthropic`, read from `<PROVIDER>_API_KEY` (source, README at `50a3700`). | Template repository, MIT. No release stage stated. Created 2026-08-18. |
| [`temporalio/agent-poc-quickstart`](https://github.com/temporalio/agent-poc-quickstart) | A URL you give your coding agent so it builds a Temporal proof of concept (ai-agents or data-pipelines, any SDK including Rust) | "Experimental" (source) |
| [Serverless Workers](https://docs.temporal.io/serverless-workers) | Workers on AWS Lambda, GCP Cloud Run or Amazon Bedrock AgentCore Runtime | Lambda and Cloud Run public preview, AgentCore pre-release (docs; [changelog](https://temporal.io/changelog)) |

pi-temporal is the closest analogue to thirdshift. It is useful as a reference design for recovering a coding agent's session on Temporal. It is not a factory: one agent loop, no issues, pull requests, reviews or CI. And it drives Pi through provider API keys, not a subscription CLI.

A code search of the `temporalio` organization for `claude -p` and `codex exec` found only:

- a Korean-language demo
- a plugin test script
- `temporalio/deputy`, an "early, experimental" dependency-management tool that execs `claude` for triage. Its `go.mod` has no Temporal dependency.

## 4. Deployment and cost

### 4.1 Self-hosted

- **Dev server.** `temporal server start-dev` starts the server and Web UI on `localhost:7233` / `:8233` with "an in-memory database". It is "a single process with zero runtime dependencies" that "supports persistence to disk and in-memory mode through SQLite" ([dev server](https://docs.temporal.io/develop/run-a-development-server), docs). `--db-filename <path>` keeps state across restarts; "By default, Workflow Executions are lost when the server process dies" ([CLI reference](https://docs.temporal.io/cli/command-reference/server), docs).
- **SQLite is dev-only.** "Temporal also supports SQLite v3.x persistence, but this is meant only for development and testing, not production usage." Production uses Cassandra, MySQL or PostgreSQL; PostgreSQL 13.18 to 16.6 is tested ([Persistence](https://docs.temporal.io/temporal-service/persistence), docs). The [embedded Go server](https://docs.temporal.io/self-hosted-guide/embedded-server) carries the same caution (docs).
- **Docker Compose** (`temporalio/samples-server`) runs the server with PostgreSQL and Elasticsearch, and the UI on `:8080`. Server binaries can also run under systemd ([Deployment](https://docs.temporal.io/self-hosted-guide/deployment), docs). Server and CLI are MIT-licensed (source).
- **Footprint.** No minimum CPU or memory for any of these appears in the docs grepped, and none was measured. On a 4-core / 8 GB host already running two agent sessions, a dev-server process is plausible. A PostgreSQL plus Elasticsearch stack competes for the same memory. That is an inference to check before relying on it.

### 4.2 Temporal Cloud

- **Pay-as-you-go:** "Starts at $50 per million Actions", with "No base monthly fee and no minimums or commitment". It includes developer support for 10% of usage. Self-service volume pricing goes down to $25 per million ([pricing page](https://temporal.io/pricing)). The docs give the tiers: $50 for the first 5M, $45 to 10M, $40 to 20M, down to $25 for 100M–200M ([Cloud pricing](https://docs.temporal.io/cloud/pricing), docs).
- **Storage:** $0.042 per GB-hour active (open Workflows), and $0.00105 per GB-hour retained (closed Workflows) (pricing page).
- **Trial:** $150 in credits for 90 days, with a credit card required. No permanent free tier is described (pricing page).
- **Billing details that matter here:**
  - Each Activity start or retry is an Action.
  - A heartbeat is an Action when it reaches the service, which with throttling is about every 0.8 × Heartbeat Timeout ([Actions](https://docs.temporal.io/evaluate/cloud/actions), docs).
  - "Each Heartbeat counts as one Action" ([Cost optimization](https://docs.temporal.io/best-practices/cost-optimization), docs).
  - The default Retention Period is 30 days, settable from 1 to 90 days ([Cloud limits](https://docs.temporal.io/evaluate/cloud/limits), docs).

**Rough estimate (assessment, not a quote).** Assume a Run with 3 hours of session time, heartbeating every 4 minutes. That is about 45 heartbeats, plus a few dozen Activities and Timers for `git`, `gh` and CI watching: on the order of 100 to 200 Actions. A Pickup pass every 5 minutes, at around 5 Actions each, is about 43k Actions a month. Thirty Runs a month plus those passes come to under 100k Actions, a few dollars a month, plus the 10% support charge. Cost is not the obstacle.

## 5. Languages: the Rust SDK

- **GA:** "Temporal's Rust SDK is now Generally Available" (changelog, 2026-09-04). Public preview was 2026-05-07 ([changelog](https://temporal.io/changelog)). The v1.0.0 release says "With this 1.0.0 release, the Rust SDK is now considered stable" ([release notes](https://github.com/temporalio/sdk-rust/releases/tag/v1.0.0)). The blog announcement is dated 2026-09-23 ([blog](https://temporal.io/blog/build-durable-applications-rust-temporal-rust-sdk-now-generally-available)).
- **Current version:** `temporalio-sdk` 1.1.0, released 2026-10-06, with `rust_version` 1.92.0 (crates.io API). The crates are `temporalio-sdk`, `temporalio-client`, `temporalio-macros` and `temporalio-common` (source). A `vendored-protox` feature builds without `protoc` (v1.0.0 release notes).
- **Covered:** Workflows and Activities through `#[workflow]`, `#[workflow_methods]` and `#[activities]` macros. Also Workers on Tokio, Signals, Queries, Updates, Timers, Child Workflows, Continue-As-New, heartbeats, Local Activities, cancellation, patching, interceptors, replay testing and schedules in the client (README at v1.1.0 and `crates/client/src/schedules.rs`, source). v1.1.0 adds experimental OpenTelemetry.
- **Gaps:**
  - "Defining Nexus handlers will be added later"; only starting Nexus operations works (README, source).
  - The docs' Rust guide has no separate pages yet for message passing, schedules or versioning; the README covers them ([Rust guide](https://docs.temporal.io/develop/rust), docs).
  - There is no Worker Session API, which exists in Go only.
- **Core SDK:** "Core SDK that can be used as a base for other Temporal SDKs. It is currently used as the base of" TypeScript, Python, .NET and Ruby (sdk-rust README, source).
- **Other SDKs:** the docs list eight SDKs: Go, Java, Python, TypeScript, .NET, Ruby, PHP and Rust ([Temporal SDKs](https://docs.temporal.io/encyclopedia/temporal-sdks), docs). That page states no stage per SDK. All eight have stable 1.x or later releases (versions under Method).

## 6. What thirdshift already does, side by side

Facts about thirdshift come from [GLOSSARY.md](../../GLOSSARY.md), [README.md](../../README.md) and the ADRs.

| Concern | thirdshift today | With Temporal |
|---|---|---|
| Starting work | The OS scheduler starts each Pass; there is no daemon ([ADR 0009](../adr/0009-the-operating-system-schedules-thirdshift.md)) | Schedules start Workflows, but a long-lived Worker process must be running for anything to happen |
| One at a time | An OS file lock per repository per machine, plus the **Claim** label on GitHub | Workflow Id uniqueness and the Schedule Overlap Policy, across every machine on the server |
| Concurrency cap | Spec run `parallel`, the **Claim limit**, and the Pass lock | Worker Activity slots per Task Queue; excess work queues |
| Retries and Repairs | A **Repair loop** with fixed budgets (5 Repairs), one **Check re-run** per head, one **Resume** for killed background work | Retry Policies per Activity. The budgets would still be Workflow logic. |
| Crash or reboot mid-Run | "A process killed outright, with `kill -9` or by a power cut, releases nothing: the issue stays `in-progress` for you to relabel" (README). A **Continuation** must be started again. | The Workflow carries on once a Worker is back. The interrupted session Activity times out on heartbeat and retries, and could resume the agent session by id. |
| Interruption | Command interruption finishes salvage, Self-merge completion and Claim ending (GLOSSARY.md) | Cancellation through heartbeats, plus cleanup Activities, which would have to be written |
| Observability | Command log, Activity log, Session logs, Run notifications | Web UI with live Event History and pending Activities; the logs would still be needed for transcripts |
| Deploy and update | One static binary; `thirdshift update` takes effect on the next Pass | A server (or Cloud), a Worker service, and Workflow versioning for in-flight Runs across updates |

## 7. Assessment

*This section is judgment, not cited fact.*

- **Temporal can carry a thirdshift-like factory.** A Workflow per Command, Activities that exec the Harness CLIs under the user's login, a host Task Queue with two Activity slots, Child Workflows for Tickets, and Schedules for Passes. The Rust SDK is GA, so it would not need a second language. Nothing about Temporal forces API keys. The integrations that do (OpenAI Agents SDK, the Cookbook's Claude loop, pi-temporal) are the wrong layer for a subscription-based factory anyway.
- **Temporal would add three things thirdshift lacks:**
  - automatic recovery of an in-flight Run after a crash or reboot
  - a live UI over every Run
  - a natural path to more than one machine, through host Task Queues and shared Workflow Ids
- **It would not replace the hard parts.** Worktree ownership, Delivery's captured pull request, Inherited failures, Self-merge checks and Security finding privacy are GitHub- and git-specific logic. That logic would move into Activities and Workflow code largely unchanged.
- **The costs:**
  - A daemon, against ADR 0009's reasons: kept alive, runs a stale binary after update, silent when it hangs.
  - A server process, or Cloud plus a payload codec to keep Security findings private.
  - Determinism rules over orchestration code that today freely runs subprocesses.
  - Workflow versioning for every change while Runs are open.
  - A rewrite of `delivery`, `spec_run` and the Pass modules.
- **Recommendation:** don't adopt Temporal now. If crash recovery of long Runs becomes painful, or Runs need to spread across machines, the cheapest test is a prototype. One Rust Workflow would run one session Activity (`claude -p` with heartbeated session id and `--resume` on retry) against `temporal server start-dev --db-filename`. It should measure the dev server's memory on the 4-core / 8 GB host and kill the Worker mid-session. [pi-temporal's guarantees doc](https://github.com/temporalio/pi-temporal/blob/50a3700/docs/guarantees.md) is the reference design to read first.
