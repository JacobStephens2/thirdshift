# Eval tools for thirdshift: Langfuse and the open-source alternatives

Research date: 2026-10-16.

**Question.** Issue [#698](https://github.com/JacobStephens2/thirdshift/issues/698) asks: "add evals -integrate with langfuse / oss eval tool?". This note gathers the facts for its grilling: what Langfuse and the open-source alternatives offer, what the Harnesses can export themselves, and how each fits thirdshift. "Evals" can mean three things here, and tools differ on each:

- **(a) Offline change checks**, run before changing a Factory skill, a Session prompt, or a Harness, Model or Effort default. The [#429 replays](second-opinion-replays.md) were one.
- **(b) Scoring production Runs** by what happened afterwards: merged, repaired, reverted, a later bug.
- **(c) A trace viewer** for Session logs.

The criteria are the earlier notes' ([openhands-fit.md](openhands-fit.md), [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md)): 1. works with subscription auth; 2. open source and self-hosted, preferred; 3. buy over build. Two more apply here: Security sessions stay private ([ADR-0015](../adr/0015-security-findings-stay-private-until-fixed.md)), and the factory host (hostname `thirdshift`) has 4 cores, 8 GB RAM, a 4 GiB swap file and no Docker.

**Method.**

- Source code was read from shallow clones made on 2026-10-16. Code citations link to a file at a fixed commit:
  - `langfuse/langfuse@e3a2bee`, `langfuse/langfuse-docs@fdd3537` (sparse), `langfuse/langfuse-python@35f8e7e`, `langfuse/claude-observability-plugin@b5b7cc3`, `langfuse/codex-observability-plugin@586910c`
  - `openai/codex@5009a99`, `anomalyco/opencode@07bbd0f`, `google-antigravity/antigravity-cli@e8a4004`, `SWE-bench/SWE-bench@02e7a74`
  - `Arize-ai/phoenix@23162f5`, `Arize-ai/coding-harness-tracing@20b0635`, `mlflow/mlflow@6d3ff85`, `comet-ml/opik@535e611`, `comet-ml/opik-claude-code-plugin@56616eb`, `lmnr-ai/lmnr@ae5c7d1`, `openlit/openlit@d1156bd`, `Agenta-AI/agenta@a0fb9dd`, `wandb/weave@5b9bf0d`
  - `UKGovernmentBEIS/inspect_ai@a699376`, `meridianlabs-ai/inspect_swe@963523e`, `meridianlabs-ai/inspect_scout@8c8e789`, `harbor-framework/harbor@62db0c1`, `promptfoo/promptfoo@7901170`, `confident-ai/deepeval@ec979c1`, `pydantic/pydantic-ai@bbe2790`
- Repository metadata and releases come from `gh api`, run on 2026-10-16.
- Docs were fetched live on 2026-10-16: langfuse.com, code.claude.com, learn.chatgpt.com, clickhouse.com, and each alternative's own site. Langfuse pages are cited by live URL; the same text is in `langfuse-docs` at the commit above. Grok Build's monitoring guide was read from the copy its CLI installs (`~/.grok/docs/user-guide/24-monitoring-usage.md`).
- thirdshift's Session logs were sampled read-only by scripts kept outside the log folders, for field names and counts only. `audits/`, `security-reviews/`, `reproductions/` and every `security-*` Session log were skipped.
- Nothing was installed, no service was started or signed up for, and no data was sent anywhere. One side effect: `muse --version`, run without `MUSE_NO_AUTO_UPDATE=1`, let Muse Code update itself from 1.4.4 to 1.4.10 on the host.
- **[unverified]** marks a claim I could not confirm from a primary source. **[3rd-party]** marks a claim from a secondary source. **[interpretation]** marks my reading. **[needs live check]** marks something only a real run would settle.

**Out of scope.** Which evals to build first, which is the grilling's job; model quality; building anything.

## TL;DR

- **The data is already there; two gaps matter.** There are 863 Session logs since 2026-09-25: 688 Claude Code, 166 Codex, 8 Muse. They come to about 46 a day, beside Command logs, the Activity log and Model evidence. Claude's Session logs lack the Session prompt; Codex's lack timestamps and the model. The #429 and #549 evals were hand-built scripts. (§1)
- **Langfuse now belongs to ClickHouse, and every eval feature is free.** It is MIT except its `ee/` folders. The licence key unlocks only administration: RBAC, retention policies, audit logs, SCIM and the like. Scores, LLM-as-a-judge, annotation queues, datasets, experiments and the Playground are MIT, with no limits. (§2.1)
- **What needs an API key.** Langfuse's managed LLM-as-a-judge, Playground, UI experiments and decision-model evaluators need an LLM connection. Tracing, scores by API, annotation queues, datasets and SDK experiments don't, and an external pipeline can judge with `claude -p`. (§2.1, §2.5)
- **Self-hosted Langfuse doesn't fit the factory host.** v4 is web and worker containers plus Postgres, ClickHouse 25.12 or later, Redis or Valkey, and S3-compatible storage, deployed by Docker Compose (for testing) or Kubernetes. There is no documented path without Docker. The docs ask for 4 cores and 16 GiB for a VM, or 11 CPUs and 25.5 GiB at production minimums, with ClickHouse alone at 8 GiB. ClickHouse warns of memory exceptions below 16 GB. So: a separate machine, or Cloud. (§2.2)
- **Langfuse Cloud.** Hobby is free for 50k units a month; the factory would send about 140k, roughly $33 a month on Core. It would put private repositories' code with a third party, and Security sessions must stay off it. (§2.3)
- **Ingestion.** OTLP/HTTP traces, as JSON or protobuf, but not gRPC; metrics are discarded. There is no Rust SDK, but the `reqwest` and `serde_json` thirdshift already links can send OTLP JSON and scores. (§2.4)
- **Claude Code exports** metrics and events, and traces in beta (`CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1`). The traces work under `-p` and read `TRACEPARENT`. Only the traces would land in Langfuse, with tokens left unmapped (they aren't `gen_ai.usage.*`), no cost, and content redacted by default. Langfuse's own integration is a Stop hook that reads Claude's transcript instead. (§3.1)
- **Codex exports** logs, traces and metrics under `[otel]`. Its traces carry `gen_ai.usage.*` tokens but no prompts, outputs or model: good for timing and tokens, not as transcripts. Langfuse's Codex plugin reads rollouts. Of the other four, Grok exports logs and metrics in alpha, OpenCode undocumented app spans, Muse an undocumented exporter, agy nothing. (§3.2, §3.3)
- **Only an importer reaches all six Harnesses and the history.** No existing tool reads thirdshift's Session logs as they are. Langfuse's Claude plugin would emit nothing, for want of the opening prompt row. thirdshift already decodes all six formats. (§3.4)
- **The alternatives that fit.**
  - **MLflow** (Apache-2.0) and **Phoenix** (ELv2, not OSI) are the only platforms that run on the factory host: one process on SQLite, no Docker. Phoenix's hooks cover 4 of the 6 Harnesses, MLflow's 3.
  - **Inspect AI** (MIT) is the most rigorous replay framework, with pass^k, but reaches the subscriptions only through a solver thirdshift would write.
  - **promptfoo** (MIT, now owned by OpenAI) runs the Claude Agent SDK and the Codex SDK on the logins.
  - **Harbor** needs containers. Opik, Laminar, OpenLIT and Agenta need Docker with ClickHouse or MySQL. (§4)
- **Method.** Tasks are issues at their base commit. Graders come in three kinds: code, model and human. Pass^k suits a factory that merges unattended. Leakage has to be checked: no later commit, no shared memory. (§5)
- **Recommendation [interpretation].** (§6)
  - (a) Grow `replay.py` into a kept tool, borrowing pass^k, SWE-bench's leakage check and a train/test split.
  - (b) Script code graders over the logs and GitHub, and run model graders through the CLIs.
  - (c) Write an importer from Session logs to OTLP in the GenAI conventions, viewed in MLflow, or Phoenix, on the factory host. Consider Langfuse only with a second machine, or Cloud for non-Security sessions.

## 1. What thirdshift already has to evaluate with

### 1.1 The logs

Every log sits under `~/.thirdshift/logs/<owner>/<repo>/` ([README.md](../../README.md) "Logs"). On 2026-10-16 there were 863 Session logs besides the `security-*` ones, from 2026-09-25 to 2026-10-16, about 46 a day in October. They split 688 Claude Code, 166 Codex, 8 Muse Code and one empty file. agy, Grok Build and OpenCode have run only in the #429 replays, whose output is under `~/.thirdshift/replays/`. A session holds a median of about 100 model calls and tool calls, the things a trace viewer shows as observations.

| Harness, flag | Tokens | Cost | Timings | Tool calls | Model | The Harness's own copy |
|---|---|---|---|---|---|---|
| Claude Code, `--output-format stream-json --verbose` ([claude.rs](../../src/harness/claude.rs)) | `message.usage` on each assistant event; `usage` and `modelUsage` on `result` | `result.total_cost_usd`, the API-price equivalent, not a charge ([README.md](../../README.md) "Usage and cost") | `timestamp` on each assistant and user event; `duration_ms`, `duration_api_ms`, `ttft_ms` on `result` | `tool_use` blocks, results in `user` events; sub-agents by `parent_tool_use_id` and `system/task_*` events with their own usage | `message.model`, `system/init.model` | `~/.claude/projects/<cwd>/<session>.jsonl`, deleted after `cleanupPeriodDays`, "The default is 30 days" ([.claude directory](https://code.claude.com/docs/en/claude-directory)). 480 exist for factory worktrees |
| Codex, `exec --json` | `turn.completed.usage`: input, cached, cache-write, output, reasoning | none | none: the events carry no timestamps | `item.*`: `command_execution` with exit code, `file_change`, `web_search`, `collab_tool_call` | none in the stream | rollouts in `~/.codex/sessions/`, timestamped: 813 since 2026-10-04, 1.1 GB |
| Muse Code, `exec --json` | read from Muse's `session.jsonl` after exit ([muse/stream.rs](../../src/harness/muse/stream.rs)) | none | `recorded_at` on each record | `task.lifecycle.*` records | `run.model.configured` | `~/.local/share/muse/sessions/…/session.jsonl` |
| Grok Build, `--output-format streaming-messages-json` | `usage`, `modelUsage` on `result` | `result.total_cost_usd` | `result.duration_ms` | Claude-like `assistant` and `user` events | `system/init.model` | `~/.grok/sessions/<cwd>/<id>/`; `grok usage <id>` |
| OpenCode, `run --format json` | `tokens` on each `step_finish` | `cost` on each `step_finish` | `timestamp` on each event | `tool_use` events | none | `~/.local/share/opencode/opencode.db`; `opencode session export` ([opencode/export.rs](../../src/harness/opencode/export.rs)) |
| agy, `--output-format stream-json` | `usage` on `step_update` events; thirdshift reads the cumulative one on `result` ([agy/stream.rs](../../src/harness/agy/stream.rs)) | none | `duration_seconds` on steps and on `result` | `step_update` events with `step_type` `tool` | `init.model` | one SQLite file per conversation in `~/.gemini/antigravity-cli/conversations/`, protobuf blobs |

Beside the Session logs:

- **Model evidence**, `*.models.json`: the Harness, requested Model and Effort, and observed Models. Only 21 exist so far.
- **Reviewer reports**, `*.spec` and `*.standards`: 67 of each.
- **The Command log** and **the Activity log**. The Activity logs recorded 274 command endings from 2026-10-03 to 2026-10-16: 249 merged, 20 failed, 5 other.

Two gaps matter for any importer:

- **No Session prompt in Claude's logs.** All 688 Claude Session logs start with an assistant event. thirdshift passes the Session prompt as an argument ([claude.rs](../../src/harness/claude.rs) `claude_args`), and `stream-json` doesn't echo it.
- **No time or model in Codex's logs.** Only the rollout in `~/.codex/sessions/` has them.

### 1.2 The evals already run by hand

- **#429, Second opinion replays** ([second-opinion-replays.md](second-opinion-replays.md)): 183 read-only review sessions, plus 24 follow-ups, over historical commits, in seven arms across all six Harnesses, each graded blind by fresh agents against a bar fixed in advance. [`replay.py`](second-opinion-replays/replay.py) builds each unit's throwaway repository with `git init` and a fetch of only the head and base commits, with no origin and no tags. It copies the skill in, signs `gh` out, runs one session at a time, and stops a stage at a plan-limit warning. Grading files stay private.
- **#549, Security review mode trial** ([security-review-mode-trial.md](security-review-mode-trial.md)): 12 sessions, two at a time, each in a fresh clone with a 45-minute timeout. Cost came from Claude's `total_cost_usd`, tokens from Codex's rollouts, and the findings went to a private file with mode 600.

What a tool would have to do to repeat them: build tasks from git and GitHub at a past commit; run several arms on the subscription CLIs; run one or two at a time and stop at plan limits; keep raw output private; grade blind; compare against a bar. The last two are what eval tools sell. The first four are what #429's script mostly is.

## 2. Langfuse

### 2.1 Ownership and licence

- **ClickHouse owns it.** "ClickHouse has acquired Langfuse", announced on 2026-01-16, with "no planned changes to licensing" ([blog](https://langfuse.com/blog/joining-clickhouse)). The LICENSE now reads "Copyright (c) 2023-2026 ClickHouse, Inc." ([LICENSE](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/LICENSE)). The repository has 35.9k stars and released v4.63.0 on 2026-10-16, its fifth release in three days (`gh api`).
- **The split.** Everything is MIT except the `ee/`, `web/src/ee/` and `worker/src/ee/` folders, which fall under the commercial [Enterprise licence](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/ee/LICENSE).
- **The licence key unlocks only administration.** That means project-level RBAC roles, protected prompt labels, data retention policies, audit logs, server-side data masking, UI customization, organization creators, the org management API and SCIM, and the instance management API ([license key](https://langfuse.com/self-hosting/license-key)).
- **Every evaluation feature is in the free edition.** "All product capabilities—tracing, evaluations, prompt management, experiments, annotation, the playground, and more—are MIT licensed without any usage limits" ([open source](https://langfuse.com/handbook/chapters/open-source)). The code agrees: the `oss` plan has no limit on annotation queues, model-based evaluators, prompts or data access ([entitlements.ts L139-L180](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/web/src/features/entitlements/constants/entitlements.ts#L139-L180)).

| Feature | Free self-hosted | Needs | Cloud Hobby |
|---|---|---|---|
| Tracing, sessions, OTLP ingestion, scores by API, score configs | Yes | nothing | yes |
| Datasets, experiments via the SDK, batch evaluation with your own code | Yes | nothing; your code calls whatever it likes | yes |
| Annotation queues (human grading) | Yes, unlimited | nothing | 1 queue |
| LLM-as-a-judge, Playground, experiments via the UI | Yes | an **LLM connection: an API key** ([LLM API](https://langfuse.com/self-hosting/deployment/infrastructure/llm-api)) | yes |
| Decision-model evaluators | Yes | an OpenAI or TypeSafe key ([decision models](https://langfuse.com/docs/evaluation/evaluation-methods/decision-models)) | yes |
| Code evaluators (Python or TypeScript) | Yes | a dispatcher: AWS Lambda, or `insecure-local`, which runs JavaScript only, in the worker ([code evaluators](https://langfuse.com/self-hosting/configuration/code-evaluators)) | yes |
| Data retention policies | Licence key | | Pro and up |
| Langfuse Assistant, Ask AI | Yes | an instance-wide model you configure | on Bedrock |

### 2.2 Self-hosting

- **Version.** v4 is current and "generally available for self-hosted deployments" ([v4](https://langfuse.com/docs/v4)). Its components are v3's, at newer versions.
- **Components.** Two Node.js application containers, web and worker, plus:
  - Postgres: at least 15, with 16 recommended ([postgres](https://langfuse.com/self-hosting/deployment/infrastructure/postgres));
  - ClickHouse: "Langfuse v4 requires ClickHouse >= 25.12 (26.4 recommended)" ([clickhouse](https://langfuse.com/self-hosting/deployment/infrastructure/clickhouse));
  - Redis or Valkey: at least 7, with `maxmemory-policy=noeviction` ([cache](https://langfuse.com/self-hosting/deployment/infrastructure/cache));
  - S3-compatible blob storage, which can be SeaweedFS or MinIO when self-hosted.
- **Deployment.** Docker Compose is for "Local use and testing", a "Single VM without high availability, scaling, or backups". Production means Kubernetes with Helm, AWS, Azure or GCP, or community Render and Railway templates ([self-hosting](https://langfuse.com/self-hosting)). The Compose file starts six containers, with no memory limits ([docker-compose.yml](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/docker-compose.yml)).
- **Resources.** For a Compose VM, "we recommend that you use at least 4 cores and 16 GiB of memory", and about 100 GiB of disk ([Docker Compose](https://langfuse.com/self-hosting/deployment/docker-compose)). The production minimums add up to 11 CPUs and 25.5 GiB ([scaling](https://langfuse.com/self-hosting/configuration/scaling)):

  | Web | Worker | Postgres | Redis/Valkey | ClickHouse | SeaweedFS/MinIO |
  |---|---|---|---|---|---|
  | 2 CPU, 4 GiB | 2 CPU, 4 GiB | 2 CPU, 4 GiB | 1 CPU, 1.5 GiB | 2 CPU, 8 GiB | 2 CPU, 4 GiB |

- **ClickHouse's own guidance.** "The recommended amount of RAM is 32 GB or more", and "If your system has less than 16 GB of RAM, you may experience various memory exceptions". It can run in "as low as 2 GB" with extra tuning and a low ingestion rate ([usage recommendations](https://clickhouse.com/docs/operations/tips)). For low data volumes, "total memory shouldn't be below 8GB" ([sizing](https://clickhouse.com/docs/guides/sizing-and-hardware-recommendations)).
- **Without Docker.** No documented path. Building from source needs pnpm, Node 24 and a Rust toolchain, and the contributing guide runs the databases in Docker ([CONTRIBUTING.md L132-L136](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/CONTRIBUTING.md#L132-L136)). Running ClickHouse, Postgres, Valkey and SeaweedFS as host services, with the two Node apps beside them, may work, but nobody supports it **[interpretation]**.
- **On the factory host: no [interpretation].** ClickHouse's 8 GB floor is the whole host's RAM. Langfuse's own VM advice is twice the host. The 4 GiB swap file doesn't close that gap: two building Runs plus ClickHouse would page heavily or invite the OOM killer. Docker isn't installed. It would fit a separate machine with 16 GB or more, or Langfuse Cloud.

### 2.3 Langfuse Cloud

- **Plans** ([pricing](https://langfuse.com/pricing)):
  - Hobby: free; 50k units a month, 30 days of data access, 2 users, 1 annotation queue.
  - Core: $29 a month; 100k units, then $8 per 100k; 90 days; 3 queues.
  - Pro: $199 a month; 3 years; unlimited queues; data retention management.
  - Enterprise: $2,499 a month.
- **Units.** A unit is "any tracing data point": traces, observations and scores each count. At about 46 sessions a day and about 100 observations each, the factory would send roughly 140k units a month: over Hobby, and about $33 a month on Core **[interpretation]**.
- **Access is not retention.** Data past the access window is "Hidden, but still stored". Retention policies that delete data exist only on Pro, Enterprise and self-hosted Enterprise ([data access vs retention](https://langfuse.com/faq/all/data-access-vs-data-retention)). Deleting traces by API works on every plan ([openapi.yml L6108-L6158](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/web/public/generated/api/openapi.yml#L6108-L6158)).
- **Regions.** US (Oregon), EU (Ireland), Japan (Tokyo) and HIPAA (Oregon), on AWS, "partly managed by Clickhouse" ([data regions](https://langfuse.com/security/data-regions)).
- **A third party holds the transcripts.** Session logs include code from private repositories and, for Security sessions, vulnerability evidence. Langfuse says customer traces "are never used to train internal or third-party ML models" ([security FAQ](https://langfuse.com/security/security-faq)). Under ADR-0015's reasoning (a Run notification passing through Resend carries only a finding's severity, title and link), Security sessions would have to be kept out of it entirely **[interpretation]**.

### 2.4 Ingestion

- **OTLP.** Spans go to `/api/public/otel`, or `/api/public/otel/v1/traces` for signal-specific exporters. "Langfuse currently supports OTLP over HTTP with both `HTTP/JSON` and `HTTP/protobuf`. `gRPC` is not supported yet." Auth is HTTP Basic with the project's public and secret keys. Exporters should send `x-langfuse-ingestion-version: 4`, or data can lag "up to 10 minutes" ([OpenTelemetry](https://langfuse.com/integrations/native/opentelemetry); [openapi.yml L3940-L3961](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/web/public/generated/api/openapi.yml#L3940-L3961)).
- **Traces only.** The `/otel/v1/metrics` route accepts and discards: `fn: async () => {}` ([metrics/index.ts L11-L20](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/web/src/pages/api/public/otel/v1/metrics/index.ts#L11-L20)). There is no logs route.
- **Attribute mapping** ([attribute mapping](https://langfuse.com/integrations/native/opentelemetry#property-mapping)):
  - `langfuse.*` attributes set the trace name, session, user, tags and metadata, and each observation's type, input, output, model, usage and cost.
  - GenAI conventions are read too: `gen_ai.request.model`, `gen_ai.usage.*`, `gen_ai.prompt` and `gen_ai.completion`, and tool-call arguments and results.
  - So are OpenInference's `input.value` and `llm.token_count.*`, and MLflow's span inputs and outputs.
  - Token usage is read only from `gen_ai.usage.*` or `llm.token_count.*` keys ([OtelIngestionProcessor.ts L3312-L3320](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/packages/shared/src/server/otel/OtelIngestionProcessor.ts#L3312-L3320)) or `langfuse.observation.usage_details`.
- **REST.** The public API covers v2 observations, v3 scores, `POST /api/public/scores`, datasets and dataset items, experiments and experiment items, annotation queues and their items, score configs, and evaluators and evaluation rules ([openapi.yml](https://github.com/langfuse/langfuse/blob/e3a2bee2a2c048adf29b8d8a18b29bd4a8bf8ac3/web/public/generated/api/openapi.yml); [public API](https://langfuse.com/docs/api-and-data-platform/features/public-api)).
- **SDKs.** Python (MIT) and JS/TS. There is no official Rust SDK: for other languages the docs point to each language's OpenTelemetry SDK, Rust included ([SDK overview](https://langfuse.com/docs/observability/sdk/overview)). Unofficial crates exist, the largest with 15 stars **[3rd-party]**.
- **From thirdshift.** The binary already links `reqwest` (blocking, rustls, json) and `serde_json` ([Cargo.toml](../../Cargo.toml)). It could POST OTLP/HTTP JSON spans with explicit start and end times, and POST scores, with no new crate, or use the `opentelemetry-otlp` crate over HTTP **[interpretation]**. A script outside the binary could use the Python SDK instead.

### 2.5 Evaluation features

- **Scores.** NUMERIC, CATEGORICAL, BOOLEAN or TEXT. "Each Score references **exactly one** of `Trace`, `Observation`, `Session`, or `DatasetRun`", with source `API`, `EVAL` or `ANNOTATION`. The score's ID doubles as an idempotency key ([scores data model](https://langfuse.com/docs/evaluation/scores/data-model)).
- **Managed LLM-as-a-judge.** The worker runs it on matching observations or experiment items; trace-level evaluators are deprecated in v4 ([LLM-as-a-judge](https://langfuse.com/docs/evaluation/evaluation-methods/llm-as-a-judge)). It needs an LLM connection: OpenAI, Azure OpenAI, Anthropic, Google AI Studio or Vertex, Bedrock, or an OpenAI-compatible base URL that handles tool calls. Private and loopback addresses are blocked unless a self-hosted instance allowlists them ([LLM connections](https://langfuse.com/docs/administration/llm-connection)). **So managed judges need an API key**, or a gateway that has one.
- **External evaluation pipelines.** Fetch observations by API, score them with your own code, and push the scores back ([cookbook](https://langfuse.com/guides/cookbook/example_external_evaluation_pipelines)). The Python SDK wraps this as `run_batched_evaluation(mapper, filter, evaluators, …)` ([client.py L3097-L3113](https://github.com/langfuse/langfuse-python/blob/35f8e7ee0c9c450cecf059563edbab14656a9fd1/langfuse/_client/client.py#L3097-L3113)). The evaluator can be a `claude -p` call on a subscription.
- **Annotation queues.** Human grading of traces, observations or sessions against a score config. Items can be queued by API ([annotation queues](https://langfuse.com/docs/evaluation/evaluation-methods/annotation-queues)).
- **Datasets and experiments.** An item has an input, an expected output and metadata. `run_experiment(name, data, task, evaluators, run_evaluators, max_concurrency=50, …)` runs any task function over local or hosted data and traces each call ([client.py L2421-L2435](https://github.com/langfuse/langfuse-python/blob/35f8e7ee0c9c450cecf059563edbab14656a9fd1/langfuse/_client/client.py#L2421-L2435)).
  - The task can be "check out the base commit, run `claude -p` on the issue, return the diff" **[interpretation]**.
  - There are no built-in trials or pass@k. Repeats are separate runs, or duplicated items **[interpretation]**.
  - Without an SDK, a run can be built from OTLP spans that carry `langfuse.experiment.*` attributes ([experiments over OTel](https://langfuse.com/integrations/native/opentelemetry/experiments)).

### 2.6 Integrations for the Harnesses

Langfuse lists nine coding-agent integrations ([tracing coding agents](https://langfuse.com/resources/engineering/coding-agent-tracing)). Three are thirdshift Harnesses:

- **Claude Code** ([docs](https://langfuse.com/integrations/developer-tools/claude-code)). The MIT plugin registers `Stop` and `SessionEnd` hooks ([hooks.json](https://github.com/langfuse/claude-observability-plugin/blob/b5b7cc364996467e856dc17575b5674f4799ad87/hooks/hooks.json)): "The hook reads Claude Code's generated conversation transcripts". It turns each turn into a trace with backdated generations and tool spans, and nests sub-agents from their own transcripts.
  - Tracing is opt-in, with `TRACE_TO_LANGFUSE=true`.
  - It needs Python and `langfuse>=4.7,<5`, run through `uv` or `python3` ([langfuse_hook.py L1-L7](https://github.com/langfuse/claude-observability-plugin/blob/b5b7cc364996467e856dc17575b5674f4799ad87/hooks/langfuse_hook.py#L1-L7)).
  - `CC_LANGFUSE_TRACEPARENT` attaches a session's traces to a caller's trace ([L192-L200](https://github.com/langfuse/claude-observability-plugin/blob/b5b7cc364996467e856dc17575b5674f4799ad87/hooks/langfuse_hook.py#L192-L200)), so thirdshift could hang each session under a Run.
- **Codex** ([docs](https://langfuse.com/integrations/developer-tools/codex)). The MIT plugin's `Stop` hook reads the rollout file and uploads through the TypeScript SDK. It needs Node 22+ and Codex 0.143+. It captures prompts, system prompts, tool calls, tokens and sub-agents "in full and are never truncated".
- **OpenCode** ([docs](https://langfuse.com/integrations/developer-tools/opencode)): the npm plugin `@langfuse/opencode-observability-plugin`.

There is nothing for agy, Grok Build or Muse Code.

## 3. What the Harnesses can export themselves

### 3.1 Claude Code

Source: [monitoring](https://code.claude.com/docs/en/monitoring-usage).

- **Metrics and events.** Metrics cover sessions, lines of code, pull requests, commits, `cost.usage`, `token.usage` and active time. Events go over the logs signal, among them `user_prompt`, `tool_result`, `api_request` (`model`, `cost_usd`, tokens, `duration_ms`, `effort`), `api_error` and `tool_decision`.
- **Traces, in beta.** Set `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1` and `OTEL_TRACES_EXPORTER=otlp`. The protocol can be `grpc`, `http/json` or `http/protobuf`.
  - `claude_code.interaction` is one span per prompt. Under it sit `claude_code.llm_request` and `claude_code.tool`, which has a `claude_code.tool.execution` child.
  - A sub-agent's spans nest under its Agent tool span.
  - `llm_request` carries `model`, `gen_ai.request.model`, `effort`, `duration_ms` and `ttft_ms`. Its tokens are plain `input_tokens`, `output_tokens`, `cache_read_tokens` and `cache_creation_tokens`. "The span has no cost attribute", and "Claude Code doesn't set `gen_ai.usage.*` attributes".
- **Redaction.** Prompts, responses, tool details and tool content are all off by default. `OTEL_LOG_USER_PROMPTS`, `OTEL_LOG_ASSISTANT_RESPONSES`, `OTEL_LOG_TOOL_DETAILS` and `OTEL_LOG_TOOL_CONTENT` turn each on. Detailed beta tracing (`ENABLE_BETA_TRACING_DETAILED=1` with `BETA_TRACING_ENDPOINT`) adds content attributes, and "Agent SDK and `-p` sessions don't" need an allowlist for it.
- **Under `claude -p`.** "In Agent SDK and non-interactive sessions started with `-p`, Claude Code also reads `TRACEPARENT` and `TRACESTATE` from its own environment", so each interaction becomes a child of thirdshift's span. Spans are batched every 5 s. Flushing at exit isn't documented **[needs live check]**.
- **In Langfuse.** Only the traces would land: metrics are discarded and logs have no route. The spans arrive with timings and the model. Tokens wouldn't map (they aren't `gen_ai.usage.*`), cost is absent, and content attributes such as `user_prompt` aren't Langfuse's input and output keys, so they would show only as metadata **[interpretation; needs live check]**. An OpenTelemetry Collector could rename them on the way. Langfuse's own integration uses the hook route instead.

### 3.2 Codex

- **Config.** `[otel]` lives in user-level config; project-local `.codex/config.toml` ignores it ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference)). Its keys are:
  - `exporter` (logs), `trace_exporter` and `metrics_exporter`, each `none`, `otlp-http` (`binary` or `json`) or `otlp-grpc`; `metrics_exporter` defaults to `statsig`;
  - `log_user_prompt`, false by default;
  - `environment` and `span_attributes` ([types.rs L600-L684](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/config/src/types.rs#L600-L684)).
- **Events.** `codex.conversation_starts`, `codex.api_request`, `codex.sse_event` (tokens on `response.completed`), `codex.user_prompt` (redacted by default), `codex.tool_decision` and `codex.tool_result` with an output snippet ([advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced)). Opt-in `codex.agent_response` carries final answers ([otel README](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/otel/README.md)).
- **Traces.** `codex exec` opens a `codex.exec` root span and continues a `TRACEPARENT` from its environment ([exec/src/lib.rs L250-L256](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/exec/src/lib.rs#L250-L256); [trace_context.rs L148-L158](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/otel/src/trace_context.rs#L148-L158)). Its per-response `handle_responses` spans carry `gen_ai.usage.input_tokens`, `…cache_read.input_tokens`, `…cache_write.input_tokens` and `…output_tokens` ([turn.rs L2715-L2728](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/core/src/session/turn.rs#L2715-L2728)). No span holds prompt or response text.
- **Docs and source disagree twice.**
  - The advanced-config page documents only `exporter` and `log_user_prompt`; the config reference adds `trace_exporter`.
  - The docs say `analytics.enabled = false` "also disables custom OTLP metrics exporters". The source says "Custom OTLP exporters are independent of `analytics.enabled`" ([types.rs L645-L647](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/config/src/types.rs#L645-L647)).
- **In Langfuse.** The traces would land with token usage mapped, as Codex's internal span tree, with no prompts, outputs or model name on the spans **[interpretation; needs live check]**. Langfuse's own integration reads the rollout instead. Opik's docs say Codex's trace export "never includes … token counts" **[3rd-party]**, but the source records them when a response completes ([session_telemetry.rs L653-L672](https://github.com/openai/codex/blob/5009a99dfc07830ddf7464c26f310a2b8532b55d/codex-rs/otel/src/events/session_telemetry.rs#L653-L672)).

### 3.3 The other four

- **Grok Build** has a documented OTLP export, "Status: alpha". It sends logs and metrics only ("no customer-facing traces exporter") after a double opt-in, `GROK_EXTERNAL_OTEL=1` plus an `OTEL_*_EXPORTER`. Separately, `[telemetry] trace_upload` sends session traces to xAI and "follows telemetry when unset" **[needs live check whether it is on for factory sessions]**.
- **OpenCode** v2 exports logs and app spans such as `Session.generate` to `OTEL_EXPORTER_OTLP_ENDPOINT` when it is set, with no `gen_ai.*` attributes and no user docs ([index.ts L129-L135](https://github.com/anomalyco/opencode/blob/07bbd0fd166c82d87e79f053dc70e6fdb66f88ac/packages/cli/src/index.ts#L129-L135); [otlp.ts L63-L106](https://github.com/anomalyco/opencode/blob/07bbd0fd166c82d87e79f053dc70e6fdb66f88ac/packages/util/src/observability/otlp.ts#L63-L106)).
- **Muse Code**'s binary has `settings.telemetry.*` exporter keys, but no docs **[needs live check]**.
- **agy**: no OTLP export found. Its telemetry goes to Google, and a request for OTLP token export is open ([#366](https://github.com/google-antigravity/antigravity-cli/issues/366)).

### 3.4 Importing Session logs after the fact

- **Live capture covers four Harnesses at most**: Claude Code and Codex by their own export or by hooks, OpenCode by plugin, agy by Arize's hook (§4.2). It starts only on the day it is switched on.
- **Existing importers are hook-driven transcript converters.**
  - Langfuse's Claude plugin reads a `{session_id, transcript_path}` payload on stdin, so it could be pointed at a saved transcript **[interpretation]**.
  - Langfuse's Codex plugin reads rollouts.
  - MLflow's Claude plugin exports `processTranscript(transcriptPath, sessionId?)` ([tracing.ts L531](https://github.com/mlflow/mlflow/blob/6d3ff8585e55714b22b653cd389b1b4070e5ef8e/libs/typescript/integrations/claude-code/src/tracing.ts#L531)), but it traces only from the last user message on ([transcript.ts L96-L106](https://github.com/mlflow/mlflow/blob/6d3ff8585e55714b22b653cd389b1b4070e5ef8e/libs/typescript/integrations/claude-code/src/transcript.ts#L96-L106)).
  - Arize's `coding-harness-tracing` has hooks for Claude Code, Codex, agy and OpenCode, and no backfill command ([core/cli.py](https://github.com/Arize-ai/coding-harness-tracing/blob/20b0635d55925dd7394171999df1cc25de85ec3f/core/cli.py)).
  - Harbor converts the native logs of all six Harnesses to its ATIF trajectory format. Its OpenCode converter reads `run --format=json` output, the format of thirdshift's own OpenCode Session logs ([opencode.py L527-L537](https://github.com/harbor-framework/harbor/blob/62db0c1fd003807ec2c64cea91f474f9bacc67d6/src/harbor/agents/installed/opencode.py#L527-L537)). `harbor-atif2otel` turns ATIF into OTel spans, and Phoenix uploads ATIF directly. The converters are methods on Harbor's agent classes, so using them alone is **[needs live check]**.
- **None of them reads thirdshift's Session logs as they are.**
  - The Claude plugin ignores "assistant rows until we see a user message" ([langfuse_hook.py L1392-L1395](https://github.com/langfuse/claude-observability-plugin/blob/b5b7cc364996467e856dc17575b5674f4799ad87/hooks/langfuse_hook.py#L1392-L1395)). No Claude Session log has that user row (§1.1), so it would emit nothing **[interpretation; needs live check]**.
  - The Harnesses' own copies could be imported instead. But they are short-lived (Claude's last 30 days), and they hold every session on the machine, Security ones included.
- **An importer thirdshift owns would cover all six Harnesses and the whole history.** It would read thirdshift's own Session logs, `*.models.json`, Command logs and Activity log, and could skip Security sessions by kind. It needs one parser per format. thirdshift already decodes all six streams ([src/harness/](../../src/harness/)). Codex timings would come from the rollouts **[interpretation]**.

## 4. Open-source alternatives

### 4.1 Triage

Health is from `gh api` on 2026-10-16. Unless a row cites otherwise, facts come from each project's README, LICENSE and docs at the commits in Method.

**Observability platforms with evals**, the kind of tool Langfuse is:

| Tool | Licence | On the factory host without Docker? | OTLP in | Trials | Judges need | Human grading | Harness hooks | Health |
|---|---|---|---|---|---|---|---|---|
| **Arize Phoenix** | Elastic License 2.0: **not OSI**, but self-hosting is "free and fully permitted" with "no feature gates" ([license.mdx](https://github.com/Arize-ai/phoenix/blob/23162f5e972652baba9e86a170198e914dda987d/docs/phoenix/self-hosting/license.mdx)) | **Yes**: `phoenix serve`, one Python process on SQLite | HTTP `/v1/traces`, protobuf only; gRPC on 4317 | `repetitions` | API keys | annotations in the UI and by REST; no queue found | Claude Code, Codex, agy, OpenCode | 11.9k★, v20.23.0 on 2026-10-16 |
| **MLflow** | Apache-2.0 (Databricks) | **Yes**: `mlflow server`, SQLite by default | HTTP `/v1/traces`, JSON or protobuf; no gRPC | none: duplicate the rows | API keys | review queues, "experimental" | Claude Code, Codex, OpenCode | 28.4k★, v3.17.1 on 2026-10-13 |
| Comet Opik | Apache-2.0 | No: MySQL, Redis, ClickHouse, ZooKeeper, MinIO and two backends, in Docker | HTTP, JSON or protobuf | `trial_count` | API keys | annotation queues; in the free edition **[unverified]** | Claude Code; Codex through its own OTel | 22.7k★ |
| Laminar | Apache-2.0 | No: Postgres, ClickHouse and Quickwit, in Docker | gRPC, or HTTP | an eval runner; trials not checked | not checked | labeling queues | Claude Code, Codex, OpenCode | 3.4k★ |
| OpenLIT | Apache-2.0 | No: ClickHouse | gRPC and HTTP | none found | API key | none found | Claude Code, Codex, OpenCode | 2.9k★; 45 commits in 30 days |
| Agenta | MIT, plus an `ee/` folder since 2025-10-14 | No: about a dozen services in Docker | HTTP protobuf | – | – | – | none | 4.8k★ |
| W&B Weave | the SDK is Apache-2.0; the server is W&B's SaaS, or self-managed with a W&B licence | No | – | – | – | – | – | CoreWeave bought W&B on 2025-05-05 |

**Eval frameworks and agent harnesses**, which run tasks rather than store production traces:

| Tool | Licence | Footprint | Runs the CLIs on a subscription? | Trials | Graders | Viewer | Health |
|---|---|---|---|---|---|---|---|
| **Inspect AI**, with inspect_swe and Inspect Scout | MIT, UK AI Security Institute; the two add-ons are MIT, Meridian Labs | A Python library writing log files; no server, no database | **Its packaged agents can't**: inspect_swe proxies "model API calls … back to Inspect" ([index.qmd L27-L29](https://github.com/meridianlabs-ai/inspect_swe/blob/963523e73f8657535fdaf7a2bcb1fdb5ba3d566d/docs/index.qmd#L27-L29)), so API keys. A custom solver could | `epochs`, with `pass_at_{k}` and `pass_k_{k}`, "all `k` epoch attempts succeed" ([metrics.qmd L380-L392](https://github.com/UKGovernmentBEIS/inspect_ai/blob/a699376315c95b6410f5aec01af37c8dec452aea/docs/metrics.qmd#L380-L392)) | code; model-graded (API key); human through `edit_score()` | `inspect view` | 3.0k★, 0.3.281 on 2026-10-15 |
| **promptfoo** | MIT. It agreed to join OpenAI on 2026-03-09 ([blog](https://www.promptfoo.dev/blog/promptfoo-joining-openai/)): "Promptfoo is now part of OpenAI. Promptfoo remains open source and MIT licensed" ([README L23](https://github.com/promptfoo/promptfoo/blob/79011700b4cb544ddd9267565571a1d42d3a3051/README.md#L23)) | A Node CLI on SQLite | **Yes**: the Claude Agent SDK provider on "a local Claude Code binary with an active session" ([claude-agent-sdk.md L46-L55](https://github.com/promptfoo/promptfoo/blob/79011700b4cb544ddd9267565571a1d42d3a3051/site/docs/providers/claude-agent-sdk.md#L46-L55)), the Codex SDK on the ChatGPT login, and `exec:` for any command | `--repeat` | code assertions; rubric judges | `promptfoo view` | 26.1k★, 0.124.1 on 2026-10-08 |
| **Harbor**, the Terminal-Bench harness | Apache-2.0 | Containers or cloud sandboxes only ([environment_type.py L4-L37](https://github.com/harbor-framework/harbor/blob/62db0c1fd003807ec2c64cea91f474f9bacc67d6/src/harbor/models/environment_type.py#L4-L37)); its SWE-bench tasks default to 1 CPU and 4 GB each | Claude Code and Codex, **inside a container**, on `CLAUDE_CODE_OAUTH_TOKEN` and a copied `auth.json` ([claude_code.py L136-L152](https://github.com/harbor-framework/harbor/blob/62db0c1fd003807ec2c64cea91f474f9bacc67d6/src/harbor/agents/installed/claude_code.py#L136-L152)); the other four take API keys | `n_attempts`, pass@k | verifier scripts; LLM judges; agent judges | `harbor view` | 6.2k★, v0.24.0 on 2026-10-05 |
| DeepEval | Apache-2.0 | A Python library; monitoring and annotation live in the Confident AI cloud | no runner | – | API keys | cloud | 18.9k★ |
| Pydantic Evals | MIT, inside pydantic-ai | A Python library | no runner | `repeat=N` | `LLMJudge` takes API keys, except an `openai-codex:` provider that reuses the Codex CLI's client ID, the pattern [openhands-fit.md](openhands-fit.md) §2.4 says to avoid | terminal | 20.7k★ |

**Gateways** (Helicone, LiteLLM) could see Claude Code's subscription traffic. Claude Code's docs say that setting only `ANTHROPIC_BASE_URL` "doesn't replace the subscription. Requests still route through the gateway, but a saved claude.ai login remains the active credential" ([LLM gateways](https://code.claude.com/docs/en/llm-gateway)). LiteLLM documents forwarding such a token ([tutorial](https://docs.litellm.ai/docs/tutorials/claude_code_max_subscription.md)). Anthropic's legal page still says developers "may not collect, store, or intermediate Claude.ai credentials or session tokens" ([legal](https://code.claude.com/docs/en/legal-and-compliance)). Codex's ChatGPT login can go through a gateway only as an enterprise-workspace setup ([gateway guide](https://learn.chatgpt.com/docs/enterprise/sign-in-with-chatgpt-through-a-gateway)). A gateway sees model calls, not tool runs. Not pursued.

**Closed reference points.** Braintrust: SaaS, free Starter tier with 14-day retention, self-hosting on Enterprise only ([pricing](https://www.braintrust.dev/pricing)). LangSmith: SaaS, free Developer tier of 5k traces a month, self-hosting on Enterprise only ([pricing](https://www.langchain.com/pricing)).

### 4.2 The ones that fit

**Arize Phoenix**: the lightest full platform, for (c) and (b).

- **Runs as a plain process.** `pip install arize-phoenix`, then `phoenix serve`. SQLite in `~/.phoenix/` is meant for "single-user deployments", and it needs no Redis, ClickHouse or object store ([architecture](https://github.com/Arize-ai/phoenix/blob/23162f5e972652baba9e86a170198e914dda987d/docs/phoenix/self-hosting/architecture.mdx)). Its Helm chart sizes the pod at 1 to 2 GiB ([values.yaml L782-L788](https://github.com/Arize-ai/phoenix/blob/23162f5e972652baba9e86a170198e914dda987d/helm/values.yaml#L782-L788)). Real use on the factory host is **[needs live check]**.
- **Ingestion.** OTLP/HTTP takes "only `application/x-protobuf`" ([traces.py L466-L470](https://github.com/Arize-ai/phoenix/blob/23162f5e972652baba9e86a170198e914dda987d/src/phoenix/server/api/routers/v1/traces.py#L466-L470)). A Rust sender would need the `opentelemetry-otlp` crate, not hand-built JSON.
- **Coding CLIs.** Arize's Apache-2.0 [coding-harness-tracing](https://github.com/Arize-ai/coding-harness-tracing/blob/20b0635d55925dd7394171999df1cc25de85ec3f/README.md#L58-L71) has hooks for Claude Code, Codex, agy and OpenCode, 4 of the 6 Harnesses. It installs in user-level settings, so thirdshift would set `ARIZE_TRACE_ENABLED=false` on Security sessions. It reconstructs only "One foreground subagent" ([claude_code/README L25-L29](https://github.com/Arize-ai/coding-harness-tracing/blob/20b0635d55925dd7394171999df1cc25de85ec3f/tracing/claude_code/README.md#L25-L29)). Codex hooks need a one-time `/hooks` trust in interactive `codex`.
- **History.** It can upload ATIF trajectories as traces ([helper L55](https://github.com/Arize-ai/phoenix/blob/23162f5e972652baba9e86a170198e914dda987d/packages/phoenix-client/src/phoenix/client/helpers/atif/__init__.py#L55)), and a Harbor plugin records a Harbor job as a dataset with one experiment per agent configuration.
- **Evals.** Experiments take "any function/process that returns a JSON serializable output", with `repetitions`. Trace, span and session annotations go over REST. Judges need API keys.
- **Against it.** ELv2 isn't an OSI licence, which criterion 2 prefers. Its usage analytics are on until `PHOENIX_TELEMETRY_ENABLED=false` ([privacy.mdx L37-L38](https://github.com/Arize-ai/phoenix/blob/23162f5e972652baba9e86a170198e914dda987d/docs/phoenix/self-hosting/security/privacy.mdx#L37-L38)).

**MLflow**: the same footprint, Apache-2.0, for (c) and (b).

- **Runs as a plain process.** `mlflow server` "defaults to sqlite:///mlflow.db" ([cli/__init__.py L577](https://github.com/mlflow/mlflow/blob/6d3ff8585e55714b22b653cd389b1b4070e5ef8e/mlflow/cli/__init__.py#L577)).
- **Ingestion.** OTLP/HTTP takes JSON or protobuf, with an `x-mlflow-experiment-id` header ([otel_api.py L104-L156](https://github.com/mlflow/mlflow/blob/6d3ff8585e55714b22b653cd389b1b4070e5ef8e/mlflow/server/otel_api.py#L104-L156)). thirdshift's `reqwest` could send it as is.
- **Evals.** `mlflow.genai.evaluate(data, scorers, predict_fn)` takes any callable, or traces already stored ([base.py L62-L67](https://github.com/mlflow/mlflow/blob/6d3ff8585e55714b22b653cd389b1b4070e5ef8e/mlflow/genai/evaluation/base.py#L62-L67)). Feedback can be added later from the CLI (`mlflow traces log-feedback`). Review queues are "experimental (added in MLflow 3.14.0)". Judges need API keys.
- **Coding CLIs.** Plugins exist for Claude Code, Codex and OpenCode, 3 of 6. Its docs say `mlflow autolog claude` writes hooks into `.claude/settings.json`, but the source now installs a marketplace plugin instead ([plugin.py L22-L55](https://github.com/mlflow/mlflow/blob/6d3ff8585e55714b22b653cd389b1b4070e5ef8e/mlflow/claude_code/plugin.py#L22-L55)).
- **Against it.** Anonymous usage telemetry is on until `MLFLOW_DISABLE_TELEMETRY=true`. It's a large ML platform, of which GenAI tracing is one part.

**Inspect AI**: the most rigorous framework for (a), if thirdshift writes the solver.

- **No server.** Logs are files, read by `inspect view`.
- **Runs without Docker.** Its `local` sandbox is "Local file system (no sandbox)", running as the current user ([sandboxing.qmd L96-L110](https://github.com/UKGovernmentBEIS/inspect_ai/blob/a699376315c95b6410f5aec01af37c8dec452aea/docs/sandboxing.qmd#L96-L110)).
- **Runs without a model.** `inspect eval … --model none` is documented ([models.qmd L36-L44](https://github.com/UKGovernmentBEIS/inspect_ai/blob/a699376315c95b6410f5aec01af37c8dec452aea/docs/models.qmd#L36-L44)). So a solver that builds `replay.py`'s workspace and runs the host's own `claude -p` or `codex exec` could stay on the subscriptions **[interpretation; needs live check]**.
- **What it adds.** Epochs with pass@k and pass^k, clustered standard errors, score provenance, and a viewer.
- **What it doesn't.** inspect_swe's packaged agents force API keys, and model-graded scorers go through an Inspect provider. Inspect Scout imports Claude's `~/.claude/projects` sessions, not `stream-json` Session logs.

**promptfoo**: the lightest way to run arms on subscriptions, for (a).

- **Arms.** Its Agent SDK and Codex SDK providers run on the logins, and `exec:` runs any CLI, a `replay.py` wrapper included. `--repeat` gives trials.
- **Results.** SQLite and a local viewer.
- **Three caveats.**
  - The Claude path goes through the Agent SDK, not `claude -p`, a slightly less plain case under Anthropic's terms ([openhands-fit.md](openhands-fit.md) §2.4).
  - Its git workspaces use `git clone --shared` and keep the source's objects and tags ([agentWorkspace.ts L683-L688](https://github.com/promptfoo/promptfoo/blob/79011700b4cb544ddd9267565571a1d42d3a3051/src/providers/agentWorkspace.ts#L683-L688)), so later fixes stay reachable unless tasks point at repositories built the `replay.py` way **[interpretation]**.
  - OpenAI now owns it.

**Harbor** fits only if a container runtime comes to the host. Claude Code and Codex would then run unmodified on subscription tokens inside containers, but the other four Harnesses need API keys. It sends telemetry to PostHog unless `HARBOR_TELEMETRY=off`.

## 5. Method: what maps onto thirdshift

The sources are Anthropic's [Demystifying evals for AI agents](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents) (9 January 2026), Lance Martin's [Automating eval design and hillclimbing with Claude](https://claude.dev/blog/automating-eval-design-and-hillclimbing/) (claude.dev, 28 September 2026), OpenAI's [Evaluation best practices](https://developers.openai.com/api/docs/guides/evaluation-best-practices), and SWE-bench ([arXiv:2310.06770](https://arxiv.org/abs/2310.06770); harness at `02e7a74`). The right-hand column is **[interpretation]**.

| Idea | What thirdshift already has |
|---|---|
| **Task**: "a single test with defined inputs and success criteria". SWE-bench's is an issue at its pull request's `base_commit` ([types.py L9-L21](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/types.py#L9-L21); [build_dataset.py L21-L64](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/collect/build_dataset.py#L21-L64)) | An issue at its Base branch commit. `replay.py` uses the issue body as first published, much as SWE-bench keeps only hints written before the pull request's first commit ([utils.py L285-L325](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/collect/utils.py#L285-L325)) |
| **Reference and tests**: SWE-bench splits the gold patch from the test patch by path, and a task is resolved when FAIL_TO_PASS and PASS_TO_PASS both pass ([utils.py L328-L349](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/collect/utils.py#L328-L349); [grading.py L309-L326](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/harness/grading.py#L309-L326)) | The merged PR is the gold patch, and its `tests/` and `*_tests.rs` changes give FAIL_TO_PASS candidates. Inline `#[cfg(test)]` blocks would escape a path split. Running the tests means a build at the base and at the head, about two at a time on this host |
| **Trials**: pass@k "measures the likelihood that an agent gets at least one correct solution in k attempts"; pass^k "measures the probability that all k trials succeed"; "pass^k for agents where consistency is essential" | #429 ran one trial per cell. A Merge run's unattended Self-merge is the consistency case, so a changed default wants pass^k |
| **Graders**: code is "Fast," "Cheap," "Objective"; a model grader "Requires calibration with human graders"; a human grader is the "Gold standard". The judge "should not be the model you are testing" (claude.dev) | Code graders: CI green at the exact head, Self-merge done, the Repair count, Inherited failure or own, a later bug traced back to the PR. Model graders: #429's blind, shuffled fresh agents. Human graders: the Day shift merging, editing or closing; a Security finding's triage "is also its grade, real or not" ([GLOSSARY.md](../../GLOSSARY.md)) |
| **Outcomes and transcripts**: "it's often better to grade what the agent produced, not the path it took", but "it's often useful to also grade the transcript" | The outcomes are the PR, CI and the Activity log line. The transcripts are the Session logs |
| **Leakage**: "Claude gaining an unfair advantage on some tasks by examining the git history from previous trials". SWE-bench's `git_clone_timesafe` removes `origin`, later tags and the reflog, and fails if a later commit survives ([docker_utils.py L58-L78](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/image_builder/docker_utils.py#L58-L78)) | `replay.py` fetches only two commits, with no origin and no tags, and signs `gh` out. It doesn't assert that no later commit exists. Harness memory is a second channel: Claude's auto-memory, Grok's memory, Muse's reminders |
| **Size and noise**: "20-50 simple tasks drawn from real failures is a great start". claude.dev "splits the evaluation set at random into test and train", keeps a patch only "If train and test sets improve", and checks that noise is smaller than the smallest gain worth acting on | #429's 6, 3, 20 and 30 items fit the first part, but they show only large effects ([model-strength-for-review.md](model-strength-for-review.md) §3.3) |
| **Read transcripts**: CORE-Bench rose from 42% to 95% once grading bugs and task ambiguities were fixed; "0% pass@100 is most often a signal of a broken task" | Any trace viewer helps here; this is (c) |
| **What to hill-climb**: "Your system prompt / Skills or instruction files / Tool descriptions / Model choice, effort level … / Your harness code". A hill-climber that reads failing transcripts "should never paste the failure content into the prompt" (claude.dev) | Factory skills, Session prompts, and Harness, Model and Effort defaults: exactly (a) |

Two more points:

- **OpenAI's own eval platform is closing.** "Evals will become read-only for existing users on October 31, 2026", and "the platform is scheduled to shut down on November 30, 2026". Its advice still holds: "Log everything", "create a randomized, blinded test", "Use pairwise comparison or pass/fail for more reliability".
- **claude.dev's pattern needs no platform.** `/claude-api build-eval` returns "the cases, the grader, the runner, one JSON line and one full transcript per case, and a plain page that lists each case's score with a link to its transcript". The pages "are static files that open locally and load nothing from the network". That is close to what #429's scripts already are. Whether its runner can drive `claude -p` on a subscription is **[unverified]**.

## 6. Fit

### 6.1 The serious candidates against the criteria

Columns 1 to 3 are the earlier notes' criteria; 4 to 7 are this issue's. Cells are **[interpretation]** where they go beyond §2 to §4.

| | 1. Subscription auth | 2. Open source, self-hosted | 3. What thirdshift still writes | 4. Security sessions kept off | 5. Integration cost | 6. Harnesses | 7. Replays like #429 |
|---|---|---|---|---|---|---|---|
| **Langfuse, self-hosted** | Tracing, scores, queues, datasets and SDK experiments need no model key. Managed judges, the Playground and UI experiments need one | MIT for every eval feature, but Docker and 16 GB: **another machine** | An importer or hooks; outcome scoring; the replay runner | Yes: thirdshift never sends them (per-session env, filter by folder and kind) | Rust: OTLP/HTTP JSON and REST scores over the existing `reqwest`. Or the Python SDK in a script | Hooks for Claude Code, Codex, OpenCode; an importer for all six | Stores and compares runs; no trials |
| **Langfuse Cloud** | Same | Not self-hosted: a third party holds private repositories' code | Same | Only by filtering before sending; kept until deleted below Pro | Same; about $33 a month on Core | Same | Same |
| **Phoenix** | Tracing yes; judges need keys | ELv2, not OSI; **fits the factory host** | An importer (OTLP protobuf, or ATIF); scoring; the runner | Per-session `ARIZE_TRACE_ENABLED=false`; importer filter | Rust needs `opentelemetry-otlp`, since HTTP takes protobuf only | Hooks for 4: not Grok, not Muse | Experiments over any task, with `repetitions` |
| **MLflow** | Tracing yes; judges need keys | Apache-2.0; **fits the factory host** | Same as Phoenix | Per-session `MLFLOW_CLAUDE_TRACING_ENABLED=false`; importer filter | OTLP JSON over the existing `reqwest` | Plugins for 3 | `genai.evaluate` over any callable; no repetitions |
| **Inspect AI** | Only through a custom solver under `--model none` **[needs live check]** | MIT; a library, fits | The solver: workspace, CLI call, transcript capture; scorers | Not applicable: it runs only what it's given | A Python script outside the binary | Any, through the solver | Yes: epochs, pass@k, pass^k, `inspect view` |
| **promptfoo** | Yes: Agent SDK and Codex SDK providers on the logins; `exec:` for the rest | MIT, owned by OpenAI; fits (Node, SQLite) | Leak-free task repositories; graders | Not applicable | Config plus a wrapper script | Claude Code, Codex and OpenCode by SDK; all six by `exec:` | Yes: `--repeat`, assertions, viewer |
| **Harbor** | Claude Code and Codex in containers; the rest on API keys | Apache-2.0; **needs a container runtime** | An "issue at its base commit" adapter | Not applicable | Python | All six, four of them on API keys | Yes, with containers |
| **Owned scripts**, the #429 way | Yes: `claude -p` and `codex exec`, as today | Own code | Everything: importer, scores, runner, report page | Yes, by folder and kind | None new | All six, and the whole history | Yes: #429 did it |

### 6.2 Options and recommendation [interpretation]

**(a) Offline change checks.**

- **Options.**
  1. Grow `replay.py` into a kept tool, with task manifests, arms, k trials, code graders, blind model graders on the CLIs, and one static page per run, in claude.dev's style.
  2. Inspect AI, with a thirdshift solver in the `local` sandbox.
  3. promptfoo, with `exec:` wrappers.
  4. Harbor, once the host has a container runtime.
- **Recommendation: option 1.** The hard parts are the ones thirdshift already wrote and that no tool does on subscriptions without containers:
  - leak-free repositories at a past commit;
  - six CLIs on their logins;
  - one or two sessions at a time, stopping at plan limits;
  - private output.

  The tools would add storage and a viewer. Borrow from the method instead:
  - pass^k, and clustered standard errors over items that share a PR (§5);
  - SWE-bench's check that no later commit survives;
  - a train and test split for any hill-climbing.

  Try Inspect (option 2) if the bookkeeping outgrows a script, after its live check.

**(b) Scoring production Runs.**

- **The data is already there.** Activity log outcomes, Repairs, Declined CI fixes, Inherited failures, Self-merges, reverts, later bug issues, Unaddressed findings, reviewer reports and the Day shift's triage (§1.1, §5).
- **Recommendation.**
  - Compute these as code graders in a script over the logs and GitHub first. No platform is needed.
  - Run any model grader through `claude -p` or `codex exec` on the subscription, never a platform's managed judge, which needs an API key.
  - If a viewer is adopted for (c), push the results as scores onto that session's trace: Langfuse `POST /api/public/scores`, MLflow `log-feedback`, Phoenix annotations.

**(c) A trace viewer.**

- **Every choice needs an importer for thirdshift's own Session logs.** No tool reads them as they are, and hooks cover at most four Harnesses, only from the day they are installed (§3.4). Writing the importer's spans with the GenAI conventions keeps the backend swappable, since Langfuse, MLflow and Phoenix all read `gen_ai.*`.
- **On the factory host: MLflow first, Phoenix second.**
  - MLflow is Apache-2.0, one process on SQLite, and takes OTLP JSON that the binary can already send.
  - Phoenix has better coding-agent hooks and ATIF import, but ELv2 and protobuf only.
  - A static HTML page generated from the logs is the no-service alternative.
- **Langfuse** has the most complete evaluation features, all of them free, but needs a 16 GB machine with Docker, or its Cloud. Consider it if a second machine appears, or if Cloud is acceptable for non-Security sessions on public repositories.

## 7. Uncertainties and live checks

- **What Claude Code's beta traces look like in a backend [needs live check].** Whether spans are flushed when `claude -p` exits, and whether tokens, which aren't `gen_ai.usage.*`, show anywhere but metadata (§3.1).
- **What Codex's traces look like in a backend [needs live check].** The source records `gen_ai.usage.*` on `handle_responses`, and Opik's docs say there are no token counts (§3.2).
- **Langfuse's Claude plugin on thirdshift's Session logs [needs live check].** Read from its code, it would emit nothing, because none of the logs has the opening user row (§3.4). Pointed at Claude's own transcripts, it should work.
- **Hooks in headless sessions [needs live check].**
  - Claude Code hooks under `claude -p --permission-mode auto`.
  - Codex hooks, which need a one-time `/hooks` trust in interactive `codex`, under `codex exec --dangerously-bypass-approvals-and-sandbox`.
  - agy's Stop hook under `agy -p`.
- **Memory use of `phoenix serve` and `mlflow server --workers 1` on the factory host [needs live check]**, idle and while ingesting.
- **Inspect AI on the subscriptions [needs live check].** A custom solver under `--model none` in the `local` sandbox, running the host's `claude -p` and `codex exec`.
- **Harbor's ATIF converters used alone [needs live check].** Run on thirdshift's OpenCode Session logs, and on Claude's transcripts and Codex's rollouts.
- **promptfoo on the plan [needs live check].** Its Agent SDK provider with `apiKeyRequired: false` and `permission_mode: auto`, and which allowance it draws. Also whether its default grader picks up the Codex login.
- **Grok's `trace_upload` [needs live check].** Whether factory Grok sessions send traces to xAI; it "follows telemetry when unset".
- **Muse Code's and OpenCode's exporters [needs live check].** What Muse's undocumented exporter sends, and what OpenCode v2 sends to an OTLP endpoint.
- **Opik's annotation queues in the free edition [unverified]**, which has no user management.
- **claude.dev's `build-eval` runner on a subscription [unverified].**
- **Langfuse without Docker [interpretation].** Running its services natively is possible in principle and supported by no one.
- **The volume estimate [interpretation].** Units and cost in §2.3 assume one observation per model call and per tool call, at October's rate.

## Sources

thirdshift: [README.md](../../README.md) ("Logs", "Usage and cost"), [GLOSSARY.md](../../GLOSSARY.md), [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md), [ADR-0011](../adr/0011-thirdshift-keeps-a-per-repository-activity-log.md), [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md), [ADR-0015](../adr/0015-security-findings-stay-private-until-fixed.md), [second-opinion-replays.md](second-opinion-replays.md), [security-review-mode-trial.md](security-review-mode-trial.md), [model-strength-for-review.md](model-strength-for-review.md), [openhands-fit.md](openhands-fit.md), [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md), [src/harness/](../../src/harness/), [Cargo.toml](../../Cargo.toml).

Langfuse (code at the commits in Method; docs fetched 2026-10-16):
- https://langfuse.com/blog/joining-clickhouse
- https://langfuse.com/handbook/chapters/open-source
- https://langfuse.com/self-hosting/license-key
- https://langfuse.com/docs/v4
- https://langfuse.com/self-hosting
- https://langfuse.com/self-hosting/deployment/docker-compose
- https://langfuse.com/self-hosting/configuration/scaling
- https://langfuse.com/self-hosting/deployment/infrastructure/clickhouse, …/postgres, …/cache, …/llm-api
- https://langfuse.com/self-hosting/configuration/code-evaluators
- https://langfuse.com/pricing
- https://langfuse.com/faq/all/data-access-vs-data-retention
- https://langfuse.com/security/data-regions
- https://langfuse.com/security/security-faq
- https://langfuse.com/integrations/native/opentelemetry, …/opentelemetry/experiments
- https://langfuse.com/docs/api-and-data-platform/features/public-api
- https://langfuse.com/docs/observability/sdk/overview
- https://langfuse.com/docs/evaluation/scores/data-model
- https://langfuse.com/docs/evaluation/evaluation-methods/llm-as-a-judge, …/decision-models, …/annotation-queues
- https://langfuse.com/docs/administration/llm-connection
- https://langfuse.com/guides/cookbook/example_external_evaluation_pipelines
- https://langfuse.com/integrations/developer-tools/claude-code, …/codex, …/opencode
- https://langfuse.com/resources/engineering/coding-agent-tracing

ClickHouse:
- https://clickhouse.com/docs/operations/tips
- https://clickhouse.com/docs/guides/sizing-and-hardware-recommendations

Harnesses:
- https://code.claude.com/docs/en/monitoring-usage
- https://code.claude.com/docs/en/claude-directory
- https://code.claude.com/docs/en/llm-gateway
- https://code.claude.com/docs/en/legal-and-compliance
- https://learn.chatgpt.com/docs/config-file/config-reference
- https://learn.chatgpt.com/docs/config-file/config-advanced
- https://learn.chatgpt.com/docs/enterprise/sign-in-with-chatgpt-through-a-gateway
- https://github.com/google-antigravity/antigravity-cli/issues/366
- Grok Build's installed guide, `~/.grok/docs/user-guide/24-monitoring-usage.md` and `05-configuration.md`

Alternatives (code at the commits in Method, each project's docs read in its repository; pages fetched 2026-10-16):
- Repositories: [phoenix](https://github.com/Arize-ai/phoenix), [coding-harness-tracing](https://github.com/Arize-ai/coding-harness-tracing), [mlflow](https://github.com/mlflow/mlflow), [opik](https://github.com/comet-ml/opik), [lmnr](https://github.com/lmnr-ai/lmnr), [openlit](https://github.com/openlit/openlit), [agenta](https://github.com/Agenta-AI/agenta), [weave](https://github.com/wandb/weave), [inspect_ai](https://github.com/UKGovernmentBEIS/inspect_ai), [inspect_swe](https://github.com/meridianlabs-ai/inspect_swe), [inspect_scout](https://github.com/meridianlabs-ai/inspect_scout), [harbor](https://github.com/harbor-framework/harbor), [promptfoo](https://github.com/promptfoo/promptfoo), [deepeval](https://github.com/confident-ai/deepeval), [pydantic-ai](https://github.com/pydantic/pydantic-ai)
- https://www.promptfoo.dev/blog/promptfoo-joining-openai/
- https://investors.coreweave.com/news/news-details/2025/CoreWeave-Completes-Acquisition-of-Weights--Biases/default.aspx
- https://docs.litellm.ai/docs/tutorials/claude_code_max_subscription.md
- https://www.braintrust.dev/pricing, https://www.langchain.com/pricing

Method:
- Anthropic, "Demystifying evals for AI agents" (9 January 2026): https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
- Lance Martin, "Automating eval design and hillclimbing with Claude" (28 September 2026): https://claude.dev/blog/automating-eval-design-and-hillclimbing/
- OpenAI, "Evaluation best practices": https://developers.openai.com/api/docs/guides/evaluation-best-practices
- Jimenez et al., "SWE-bench" (ICLR 2024): https://arxiv.org/abs/2310.06770
- The SWE-bench harness: https://github.com/SWE-bench/SWE-bench
