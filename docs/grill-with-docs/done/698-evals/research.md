# Research results

Part of the [#698 grilling](README.md).

## 2026-10-16

- Since the Activity logs began on 3 October, 274 commands have ended across the eight repositories:
  - 246 (90%) ended with their pull request merged by a Self-merge, 218 of them from Weeding.
  - 5 were Architect runs that published a plan or filed an idea, with nothing merged.
  - 23 failed. 13 of those were interruptions, GitHub errors, Codex safeguard refusals, or a pull request found merged rather than open. 4 were a Declined CI fix, 3 had Tickets not done, and 3 had other causes.

  Confirmed by counting the `ended:` lines in each `activity.log`.
- 7 of the 8 repositories the factory works on are public. chart35 is private. Confirmed by `gh repo view`.
- No Session log holds a GitHub, Anthropic, OpenAI, Resend or AWS key in its usual format. Confirmed by a count-only `grep` of all 888 `.jsonl` files under `~/.thirdshift/logs/`; nothing was printed.
- #429's graded material is private, under `~/.thirdshift/replays/2026-10-09/grading/`, because some of its bugs are in private repositories or still unfixed. All its graders were Claude agents. Confirmed by `docs/research/second-opinion-replays.md`.
- Open issues that overlap: #700 (are some logs traces?), #711 (see every live Run; it gathers #368 and #574), #348 (autoresearch), #322 (`thirdshift retro`) and #321 (update the skills from mattpocock/skills). Confirmed by `gh issue list`.
- **Langfuse** has belonged to ClickHouse since January 2026. Everything outside its `ee/` folders is MIT, and every eval feature is in the free edition: scores, LLM-as-a-judge, annotation queues, datasets and experiments. The licence key unlocks only administration, such as RBAC and retention policies. Confirmed by its LICENSE, its "Langfuse joins ClickHouse" post and its docs (note §2.1).
- **Self-hosted Langfuse doesn't fit this host.** It runs as web and worker containers with Postgres, ClickHouse, Redis or Valkey and S3-compatible storage, by Docker Compose or Kubernetes; there is no documented path without Docker. Its docs ask for "at least 4 cores and 16 GiB of memory", and ClickHouse warns of memory exceptions below 16 GB. This host has 8 GB, a 4 GiB swap file and no Docker. Confirmed by Langfuse's and ClickHouse's docs, and `free` (note §2.2).
- **What needs an API key.** In Langfuse: the managed LLM-as-a-judge, the Playground, and experiments started from the UI. Tracing, scores sent by API, annotation queues, datasets and SDK experiments need none, and your own grading script can call `claude -p`. MLflow's and Phoenix's built-in judges need API keys too. Confirmed by their docs (note §2.1, §2.5, §4.1).
- **What the Harnesses export.** Claude Code can export traces (in beta) under `claude -p`, but it "doesn't set `gen_ai.usage.*` attributes". In Langfuse they'd show timings and the model, not tokens or cost, and their content is redacted by default. Codex's traces carry token counts but no prompts, outputs or model name. Langfuse takes traces only and drops metrics. Confirmed by Claude Code's monitoring docs and Codex's source (note §3.1, §3.2).
- **No tool reads thirdshift's Session logs as they are.** None of the 688 Claude Session logs holds its Session prompt, since thirdshift passes it as an argument, and none of the 166 Codex ones holds timestamps or the model. A viewer would need an importer thirdshift writes. That importer could cover all six Harnesses and the whole history, and skip Security sessions by folder and file name. Confirmed by a script over the logs that skipped the Security ones (note §1.1, §3.4).
- **What fits this host.** MLflow (Apache-2.0) and Phoenix (Elastic License 2.0, not an OSI licence) are the only trace platforms that run here: one process on SQLite, no Docker. Their memory use here is untested. Confirmed by their source and docs (note §4.2).
- **For replays, no tool does the hard part.** None rebuilds a past commit without its later fix and drives the CLIs on your logins without containers:
  - Inspect AI (MIT) adds trials, pass^k and a viewer, if thirdshift writes its solver.
  - promptfoo (MIT, part of OpenAI since March) runs Claude and Codex on the logins, but its workspaces keep later commits reachable.
  - Harbor needs containers.

  The note recommends growing #429's `replay.py` into a kept tool. Confirmed by each project's source and docs (note §4, §6.2).

## 2026-10-17

- **Session logs aren't traces, and the trace standards differ in role.** OpenTelemetry is the data model, OTLP its transport, the GenAI conventions (Development status) and OpenInference its attribute names, and ATIF a separate trajectory JSON. MLflow and Workshop accept OTLP as JSON, while Phoenix requires protobuf. Confirmed by `docs/research/eval-tools-landscape.md` §3.7.
- **thirdshift reads Session logs as they're written.** Recording consumes each transcript to checkpoint Model evidence, and a review session's reports are kept beside its Session log. Confirmed by `src/session/recording.rs` and `CONTEXT.md`.
- **`replay.py` builds its own command lines,** for example `claude -p … --permission-mode auto --model … --effort …` and `codex exec --json …`, rather than going through thirdshift's Harness code. Confirmed by `docs/research/second-opinion-replays/replay.py`, lines 334–354.
- **The wider landscape** adds Raindrop Workshop as a local viewer, the native `claude plugin eval` for skill changes, and Braintrust and LangSmith if hosting is allowed. For replays it still recommends maintaining #429's runner. Confirmed by `docs/research/eval-tools-landscape.md`, TL;DR and §4.

- **Anthropic's eval terms:** Eval, Task, Trial, Grader, Transcript (trace, trajectory), Outcome, Evaluation suite, Evaluation harness and Agent harness, plus the split between capability and regression evals. Case, Arm and Replay don't appear. Confirmed by reading "Demystifying evals for AI agents".
- **#549's Security findings were graded by Codex on 2026-10-09,** against no confirmed vulnerabilities. Confirmed by `docs/research/security-review-mode-trial.md`.
- **#711's `web-ui` branch is still at `main`.** Node v26.10.0 and npx 12.2.0 are installed here, so promptfoo can run without new system packages. Confirmed by `git log` and `node --version`.

## 2026-10-18: promptfoo with thirdshift

Read at promptfoo 0.124.1 (commit `421e795`, MIT, npm's `latest`) and in thirdshift's source. Nothing was installed or run.

- **thirdshift has no one-session command.** Its commands are Help, Version, Update, Setup, ReleaseSummary, EmailTest, Architect, Pickup, Secure and Run (`src/args.rs:20-33`). `Sessions::within(...).run` already holds what a faithful Trial needs: skills, the Codex fresh-sub-agent rule, review reports, Resume and Model evidence (`src/session.rs:120-235`). A Trial entry point also needs a `Logs` that takes an explicit directory (`src/logs.rs:183-189`).
- **thirdshift records no tokens or cost.** A session's end holds only its session id, killed work and final message (`src/harness/interpretation.rs:55-59`). Efficiency would have to come from the Session log or a Trace.
- **`replay.py` differs from production** in several flags (`--add-dir`, `--disallowedTools WebFetch WebSearch`, `--strict-mcp-config`), an environment variable and its skill setup, and it has no Resume. Confirmed by `replay.py:334-407` and `src/harness/claude.rs:90-104`.
- **promptfoo's JavaScript provider fits; `exec:` doesn't.** The provider receives `repeatIndex`, the test and a `traceparent`, can return metadata, tokens, cost and errors, and gets a cancel signal on timeout. Confirmed by promptfoo's source: `src/types/providers.ts`, `src/contracts/providers.ts`, `src/providers/scriptCompletion.ts`.
- **Graders.** `llm-rubric` takes any custom provider, which must return `{pass, score, reason}`. By default it sees only the output and the rubric, so blind grading is up to the provider stripping identifying lines. `select-best` doesn't shuffle. Confirmed by `src/providers/defaults.ts` and `src/matchers/`.
- **Tracing.** With tracing on, promptfoo receives OTLP over HTTP, JSON or protobuf, at `127.0.0.1:4318`. It passes a W3C `traceparent` to each Trial and stores matching spans with the result, for trajectory and span assertions. Confirmed by `src/tracing/otlpReceiver.ts` and `site/docs/tracing.md`.
- **Pitfalls on long Trials:** silent retries (`src/scheduler/retryPolicy.ts`), a 14-day cache, concurrency 4, no default per-Trial timeout, telemetry that sends one event even when disabled (`src/telemetry.ts:116-178` and its test), and a viewer on all interfaces (`src/server/server.ts:548`).

## 2026-10-18: Traces

A structure-only scan, with event types and key names but no content: 880 live Session logs and 220 from #429's replays, Security sessions skipped.

- **Claude Code** (688 files) streams assistant, user, thinking, task, rate-limit, push and result events. Its usage carries the cache-time split, service tier and thinking tokens. Its results carry `total_cost_usd`, per-model usage, permission denials and sub-agent statistics. Tool results carry edit diffs and git operations. It has no compaction, hook or todo events.
- **Codex** (186 files) has threads, turns and items: commands, messages, file changes without diff text, web searches and sub-agent calls. Its usage includes cached and reasoning tokens. It has no timestamps, model or reasoning items.
- **Muse** records task lifecycles, policy decisions, todos and the workspace commit, with no token usage. **Grok** is a subset of Claude's format, with no per-event timestamps. **OpenCode** has steps with tokens and cost, and tools. **agy** has steps with usage, tools and sub-agent details.
- **The GenAI conventions** (at `547934a`, Development, no releases) cover chat, agent, tool and skill spans, models, conversation IDs, token usage including cache and reasoning, tool calls, and opt-in message content. **OpenInference** adds cost and thinking signatures, and free-form metadata for the rest.
- **OpenTelemetry allows prefixed custom attributes** such as `thirdshift.*`, with map, array or JSON-string values. Attribute length is unlimited by default. Events per span default to 128, while a session's events have a median of 248, a 90th percentile of 421 and a maximum of 2,672.
- **Viewers treat custom data differently.** Phoenix and MLflow show attributes and events. Workshop keeps only scalar attributes. Langfuse puts them under metadata, where they can't be queried.
- **thirdshift writes Session logs and never reads them back.** `follow` copies each line to the file and feeds the live interpretation (`src/session/recording.rs:227-253`). The Model evidence and review reports are named from the log's path (`recording.rs:137`, `src/session/review_reports.rs:36-49`). Resume passes the live session ID to the Harness (`src/session.rs:212-223`). Changing the format touches the path scheme, those two sidecars, the README's Logs section and tests that read logs back, such as `tests/codex_sessions.rs:304`.

## 2026-10-19: the Console

- **#711 is closed** (2026-10-18), having shipped the Command record, `status`, `show`, `logs` and `stop`. **#776, the Console's Spec,** is open and `ready-for-agent`, with Tickets #814–#829. Confirmed by `gh issue view`.
- **What the Console reads.** It reads Command records, Activity logs, Command logs and Session logs through the same code as the CLI. It renders Claude's `stream-json` and Codex's `exec --json` logs, with per-tool renderers and a plain fallback, and only the current log layout. Its Out of Scope excludes any HTTP API, other machines and an SQLite index. Confirmed by #776's Implementation Decisions.
- **Silent, Slow and the Usual range.** A Run is Silent when its Session log and Command log have both stopped growing for `inbox.silent_minutes`. The Usual range is the 25th to 75th percentile of a stage's last 30 times in a repository on a machine. Confirmed by #776 and `CONTEXT.md` on `main`.
- **#813, charts over time,** is open for grilling. One of its candidates is tokens per day by Harness and Model, which thirdshift doesn't record today. Confirmed by `gh issue view 813`.
- **`show` reads Session log content** on `main`: the last 64 KiB of a live log, through the Harness's interpretation. Confirmed by `src/logs/reading/selection.rs:332-354` at `origin/main`.

## 2026-10-20

- **v0.13.0 shipped the Console today.** #776 and its Tickets #814–#829 are closed. `main` was 411 commits ahead of this branch until it was merged in that day. Confirmed by `gh release list`, `gh issue view` and `git rev-list`.
- **What on `main` reads Session logs.** The Console's conversation renderer (`src/console/conversation.rs`, 174 lines) parses Claude's and Codex's events and shows any other line plain. The Waterfall's matcher (`src/logs/status/waterfall/matching.rs`, 188 lines) times tool calls from the Command log's timed lines. `show`'s latest event is in `src/logs/reading/selection.rs`. Confirmed by reading them at `origin/main`.
- **The Command record** holds the command's Harness, Model and Effort, each stage's name, start and Session log, the outcome, the pull request, a link to its parent record, and its Tickets (`src/logs/command_record.rs:30-106` on `main`). Confirmed by source.
- **Model evidence and review reports** are still named from the Session log's path: `models.json` beside it (`src/session/recording.rs:144`), and the reports beside it (`src/session/review_reports.rs:49`). Confirmed by source at `origin/main`.
- **ADR 0016** keeps async code inside the Console and the rest of thirdshift blocking, and it limits the Console's actions to Stop and its own look. Confirmed by `docs/adr/0016-the-console-is-a-long-running-reader.md` on `main`.
- **#776 shows Security sessions and findings behind the sign-in.** Confirmed by #776's body.
- **`CONTEXT.md` is now `GLOSSARY.md` on `main`,** and the ADRs ran to 0018 until #920 added 0019. Glossary terms go in during grilling, before the code: Command record, Silent and the Usual range came in "Grill #711" commits. Confirmed by `git log -S`.
- **Old Session logs.** There are 924 from production, not counting 71 Security files, and 215 from #429's replays. No line of Codex's carries a time, some of Claude's do, and every one of OpenCode's does. Confirmed by a count-only scan of the logs' keys, with Security sessions skipped and nothing printed.
- **Bugs and reverts.** thirdshift has no revert in its history or its pull requests. 35 of its issues opened since 3 October are labelled `bug`, out of 50 in all. Confirmed by `gh issue list`, `gh pr list` and `git log --grep`.
- **No Rust eval framework fits.**
  - TensorZero's repository is archived, and its site says it is "no longer maintained".
  - Laminar's evals are Python or TypeScript, need a project API key, and self-host on Docker with Postgres, ClickHouse and Quickwit.
  - Rig deleted its evals module in 0.40.0, and Goose removed its bench crate. Zed's eval tool is GPL, unpublished and runs in containers.
  - The Langfuse, Braintrust and Phoenix crates only send traces to their servers.
  - The nearest, Mira 0.5.0 (MIT, one maintainer), runs any CLI with trials, arms and a judge you write, but has no paired statistics.

  Confirmed by crates.io, the GitHub API and each project's docs and source.
- **OpenTelemetry in Rust is at 0.33.1.**
  - Logs are Stable and traces Beta.
  - No crate writes the OTLP file format: opentelemetry-rust#2602 is open and blocked on the specification, whose file exporter is still Development. `opentelemetry-stdout` prints text for people. *Out of date by 2026-10-21:* #3763 is in review, and Arti publishes a traces-only crate (see that day's results and Q5.8).
  - `opentelemetry-otlp` can send HTTP and JSON over reqwest's blocking client, without tonic, for 20 more crates. The SDK with the official types adds 17, and writing OTLP JSON with serde adds none.

  Confirmed by each crate's README and changelog, and by `cargo tree` on a copy of `main`'s manifest.
- **The GenAI conventions are unchanged,** still at `547934a`, Development, with no releases. Their only events are `gen_ai.client.inference.operation.details`, `gen_ai.evaluation.result` and `gen_ai.client.operation.exception`. Tool calls are `execute_tool` spans, and messages are opt-in attributes. Confirmed by the repository.
- **The statistics.**
  - Miller's "Adding Error Bars to Evals" is arXiv 2411.00640, from 1 November 2024. It recommends clustered standard errors (§2.2), averaging each question's samples (§3.1), paired differences (§4.2) and power analysis (§5).
  - Its formulas, with its example variances, put the smallest difference 9 Eval tasks of 3 Trials can reliably detect at about 44 points. That number was worked out from the paper, and isn't in it.
  - Bowyer et al. (arXiv 2503.01747, ICML 2025) find that normal-approximation intervals fail below a few hundred items. They recommend Bayesian intervals for paired comparisons.

  Confirmed by both papers.
- **promptfoo isn't silent with its switches off.** The latest release is still 0.124.1. With a custom provider and grader, and its telemetry, update, sharing and remote-generation switches off, it still:
  - POSTs once per `eval` or `view` to `r.promptfoo.app`, with a persistent user ID and any stored email (`src/telemetry.ts:116-180`);
  - loads PostHog in the viewer's page, which reports the user ID, email, page URLs and clicked text to `a.promptfoo.app` and can load scripts from there. Its off switch is fixed when the package is built, and the published build has it on (`src/app/vite.config.ts:179-181` and the shipped bundle);
  - probes Google Cloud's metadata server on the first model-graded assertion when no provider key is set, unless `METADATA_SERVER_DETECTION=none` and an empty `CLOUDSDK_CONFIG` are set;
  - sends each eval's providers and prompts to Promptfoo Cloud if its config folder was ever signed in.

  The custom provider and grader run inside promptfoo's own Node process, so a `--require` guard on outgoing sockets covers them. `NODE_OPTIONS` would carry that guard into the Harnesses, so it goes to `node` directly instead. The viewer serves its own files, has no sign-in, and accepts requests from any origin. Confirmed by reading 0.124.1's source and its published package, without running either.
- **Issues filed today that bear on Round 4.**
  - #914 asks whether thirdshift should feed Grafana, Prometheus, Tempo, Mimir, Loki and Alertmanager (Q4.12).
  - #919 asks for a way to dismiss or acknowledge what Needs you lists (Q4.8).
  - #854 asks whether to capture Codex's patches. Codex's log gives each edit's path and kind but no patch, so the Console shows "Patch unavailable in this log." for it. A Session trace has room for a captured patch in a `thirdshift.*` attribute, but old Codex logs can't get theirs back (Q4.11).

  Confirmed by `gh issue list`, `gh issue view` and `src/console/conversation.rs:115`.
- **#921 is on `main`.** A Spec with no Startable Ticket isn't a Ready issue, so a Pickup run passes over it instead of starting a Spec run that would fail (Q4.13). Confirmed by `GLOSSARY.md` and `src/ready.rs` at `origin/main`.
- **Grafana's stores take OTLP.** Tempo is "OpenTelemetry compatible", and Loki "natively supports ingesting OpenTelemetry logs over HTTP". Confirmed by Tempo's README and Loki's docs.
- **The OpenTelemetry Collector can read Session traces from disk.** Its OTLP JSON file receiver reads the File Exporter format, at alpha for traces and logs. It re-reads a whole file whenever the file changes, so it suits finished sessions rather than live ones. Confirmed by the receiver's README.

## 2026-10-21

- **The two hosts' crontabs.** The `thirdshift` host runs Pickup runs on thirdshift, cascade, keeplore, chart35, muxboard, clave and vaulted-agent, and a Security run and an Architect run on thirdshift. `stephens` runs Pickup runs and Architect runs on bountywell, clave, fareloch, horarium, stephens.page and voice-agent-scheduling. They share only clave. Confirmed by `crontab -l` on each host.
- **`stephens`** has 4 cores, 15,993 MiB of memory, 4.4 GiB of swap with 2.7 GiB in use, 34 GB of disk free, and Docker 29.1.3. Confirmed by `free`, `nproc`, `df` and `docker --version` over SSH.
- **Every factory pull request names what built it,** in a closing line such as "Built with claude · claude-opus-5-5 · medium". Confirmed by #921's body.
- **Closed `bug` issues:** 41 in thirdshift, 7 in chart35, 7 in vaulted-agent, 3 in keeplore and 1 in clave, 59 in all. cascade, muxboard and `stephens`' other five repositories have none. Confirmed by `gh issue list --label bug --state closed`.
- **The glossary limits Grade and Grader to Trials,** and Grade's entry tells agents to avoid "score". Confirmed by `GLOSSARY.md`.
- **Publishing Specs.** `docs/agents/triage-labels.md` overrides `to-spec`'s and `to-tickets`' readiness defaults. A Spec of several sessions is published `needs-triage`, and turns `ready-for-agent` only once its Tickets, sub-issue links and blocking links are all in place. Confirmed by the file.
- **Workshop, Braintrust and LangSmith,** as the landscape note found them. Raindrop Workshop is an MIT trace viewer, one native binary on SQLite, that takes OTLP over HTTP. Braintrust is proprietary and hosted, and its self-hosted data plane is too large for these hosts. LangSmith is a proprietary service that self-hosts only as an Enterprise Kubernetes deployment. Confirmed by `docs/research/eval-tools-landscape.md` §3.5–3.6 and its comparison table.

- **The viewers** (`docs/research/eval-viewers.md`).
  - Only Grafana, through Loki, keeps OTLP log records. MLflow, Phoenix, Langfuse and Workshop take spans only.
  - No viewer computes a paired verdict. Phoenix's compare view comes closest to an Experiment page.
  - MLflow fits best for traces across Runs. Its telemetry and a price-list fetch are on by default, and two variables stop them. Its only memory report is 1.8 GiB idle at four workers.
  - The Console listens on 127.0.0.1:8711 behind Caddy, and its strict CSP already blocks other sites. Its `__Host-thirdshift-session` cookie reaches only `console.thirdshift.app`.
  - Matching the viewers in the Console would take about 2,100–4,000 lines, an estimate.

  Confirmed by each project's source and docs at pinned release commits, and by `src/console/auth.rs` and `src/console.rs`.
- **The runners** (`docs/research/eval-runners.md`).
  - In every option thirdshift writes about 1,400–2,300 lines that decide accuracy. A framework replaces a middle of about 300–550, with glue of about the same size. These figures are estimates.
  - promptfoo enforces nothing by hand and computes no statistics. Its derived metrics see one Arm at a time.
  - Inspect sends nothing by default. MLflow reports usage by default, and four of its defaults would change Trials.
  - thirdshift reads Claude's usage-limit errors today, but no Harness's warnings, and not Codex's rate limits.
  - Bowyer et al.'s paired method takes one Trial per Eval task per Arm. In the research's own simulations, a Dirichlet with prior weight 1/7 per value covered 93–98%.

  Confirmed by source at pinned commits; the simulations are the research's own.
- **The OTLP writer** (`docs/research/otlp-file-writer.md`).
  - opentelemetry-rust#3763, "add `opentelemetry-otlp-file` crate", has been open since 2026-09-28, with 17 review comments and no approval.
  - The specification's file exporter is still Development, and says "Files must contain exactly one type of data".
  - Arti's `otlp-file-exporter` 0.2.0 writes traces only.
  - `otlp-jsonl` and `otlp-file` are free on crates.io.

  Confirmed by the GitHub API, the specification and crates.io.
- **Codex's patches.**
  - Codex's rollout file, `~/.codex/sessions/YYYY/MM/DD/rollout-*-<thread_id>.jsonl`, keeps a `FileChange` item for each `apply_patch` edit, with each file's unified diff. Added and deleted files carry their whole content.
  - Edits made by shell commands aren't recorded, and `codex exec --json` drops the diffs.
  - thirdshift keeps the thread ID, which names the file. The format is internal to Codex, with two recording modes and optional compression.

  Confirmed by Codex's source at `rust-v0.163.0` (`8308618`), without opening any rollout file.
- **Sandboxes for Trials** (`docs/research/eval-sandboxes.md`).
  - **Q3.6's bwrap leaves three gaps:**
    - Without a PID namespace, a Trial can read every host process's command line, where thirdshift passes Session prompts.
    - Host loopback holds the Console, Caddy's admin API and a dozen other listeners.
    - Any network that reaches the internet reaches GitHub, where most repositories' later fixes are public.
  - **Closing them.** `--unshare-pid` closes the first gap. An allowlisting proxy over a Unix socket, as Anthropic's sandbox-runtime uses, closes the other two, if each Harness honours `HTTPS_PROXY`. pasta's defaults map the host's loopback in.
  - **Other sandboxes fit worse.**
    - Docker's group is root-equivalent.
    - Rootless Podman blocks loopback, but needs an image per repository holding six Harnesses.
    - nspawn, rootless gVisor and Firecracker fit worse still.
    - The Harnesses' own sandboxes confine commands, not the session.
  - **Logins.** A Trial gets the one real login file bound in, never a copy, since Codex rotates refresh tokens in place. A fresh home per Trial keeps session histories, and Harness memory, out.
  - **Memory.** Per-Trial memory caps need `loginctl enable-linger jacob`, since cron gives the factory no user manager. Until then, a raised `oom_score_adj` makes Trials the OOM killer's first choice.
  - **Containers.** They would make Harbor runnable, but:
    - four of its six matching agents need API keys;
    - none of them runs a Trial through thirdshift;
    - its telemetry is on by default.

    So Q5.6 (a) stands. The implement suite's images would also need disk the host lacks: 32 GB is free, and SWE-bench asks for 120 GB.
  - **`infra_ok`** is a score in Phoenix's Harbor plugin, 0 for any exception. The note proposes **void** Trials instead: rules fixed with the bar decide them, and they're re-run, shown, and kept out of Q4.4's interval.

  Confirmed by each tool's docs and source at pinned commits, and by read-only checks on this host. Nothing was installed or run.

## 2026-10-21: merging and releasing

- **Production runs the installed binary.** Factory skills and Session prompts are built into it (ADR 0001). `thirdshift update` replaces it with the latest stable GitHub Release, and "A Run never checks for updates or updates itself". Confirmed by `src/update.rs` and the guide's `update` section.
- **Since 1 October,** `main` took 230 merges, 228 of them pull requests, and 16 of the 230 changed `skills/` or `src/prompt.rs`. 26 releases went out, and 10 of them carried such a change. Confirmed by `git log --first-parent --merges` on `origin/main`, and by diffing each release tag against the one before.
- **Releases go out through `scripts/release.sh <version>`.** It opens a pull request that bumps the version, waits for its checks, merges it, tags it and waits for the release workflow. `--review` asks before it opens the pull request. Confirmed by the guide's "Releasing" section.
- **What decides a Self-merge.** `merge.always` makes every Run a Merge run unless its command says `merge` or `no-merge` (`Asks::of_run` in `src/asks.rs`). A Pass's words go to the Spec run or Run it dispatches, Security fixes included (`Asks::of_ready_issue`, called from `src/pass.rs`). A Ticket's Run is always a Merge run into its Spec branch (`Asks::of_child_run`). Confirmed by reading the code.
- **The `thirdshift` host's crontab** has one Architect line with `--plan-only`, seven Pickup lines and one Security line, none with merge words. Its User config has `merge.always`, `security.fix` and `security.review` on. Confirmed by `crontab -l` and the config's keys.
- **ADR 0016** limits the Console's actions to Stop and its own look, and pairs every view and action with a CLI command with `--json`. #919, labelled `ready-for-human`, asks for a way to dismiss items in Needs you.
- **Releases are cut on the `thirdshift` host.** Its git identity is the one on the release commits, its shell history runs `scripts/release.sh`, and its checkout's debug build is from just before 0.13.3 went out. Confirmed by `git config`, a count of the history's matching lines, and the build's date.
- **Harnesses update themselves.** On the `thirdshift` host, Claude Code installed 2.1.302 on 16 October and 2.1.306 on 20 October, five versions in five days. The host also runs Codex 0.163.0, Grok 1.0.50, Muse Code 1.4.10, agy 1.3.3 and OpenCode 2.0.24. thirdshift doesn't record a Harness's version itself. Confirmed by the dates of Claude's installed versions, each CLI's `--version`, and a search of `src/`.

## 2026-10-21: the Harbor talk

- **The Harbor talk doesn't reopen Q6.3–Q6.7.** Confirmed by the Granola transcript of Xander Song's AI Engineer NYC talk on 12 October ("Harbor and Phoenix integration", `not_vdFn60dpPv8THu`). The API returned it as one untimed segment, and it starts partway through the talk, so the opening slides may be missing.
  - **Network (Q6.3).** A second speaker says that controlling the environment means asking whether the agent "work[s] well with network turned off, or does it actually need specific endpoints". Their example is cutting off all network access for sensitive data or a cyber-defense test. That is (a)'s allowlist.
  - **Fresh environments (Q6.4, Q6.7).** Each trial gets its own container, "identical in the beginning". Logins, tokens and refresh never come up.
  - **Oracles (Q6.7).** Harbor's oracles are reference solutions that show a verifier passes on a known-good answer. Q6.7 (a) already does this when it checks that the fix's tests fail at the base and pass at the fix.
  - **Services (Q6.7).** For a system that doesn't fit in one container, the talk offers only Harbor's multiple images, and warns against mocking. That's Q6.7's escape hatch for a repository that needs services.
  - **Void Trials and memory (Q6.5, Q6.6).** The transcript never mentions `infra_ok`, timeouts, errors, memory or credentials. `eval-sandboxes.md` says the talk covered `infra_ok`. If it did, that part wasn't captured, and the note's summary doesn't mention it either. That note's finding that `infra_ok` is Phoenix's, not Harbor's, rests on source, not on the talk.
- **The talk backs outcome grading (Q3.5) and Run metrics (Q4.5).** Verifiers that peek at the trajectory suit optimizing a metric, but the speakers called such tests "unstable": a one-line change broke them. For regressions they grade the outcome "in a certain level of efficiency". Their example was a new tool that raised cost by 50%, and catching such regressions in CI is why they built controlled environments. Confirmed by the same transcript.
