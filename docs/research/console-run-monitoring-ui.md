# What a run-monitoring Console should show: lessons from CI, orchestrators, agent and trace UIs

Research date: 2026-10-18, for issue #711 (the Console) and its Q27 UI prototypes in `.scratch/prototypes/711-console/` (kept on the branch `prototype/711-console`, commit `05b1b0a`).

**Question.** The Console is one machine's page of its own **Commands**. It shows live **Runs** with their stage and how long they have been in it, a **Pickup run** → **Spec run** → **Tickets** nesting, outcomes, Commands whose process died, a Stop with a confirm step, **Session logs** with grouped tool calls, the **Command log**, and **Security findings**. The **Day shift** reads it, often on a phone. What do mature tools in neighbouring fields show about running and finished work, and why? Which of those features fit thirdshift's rules: the page does nothing the CLI can't; no Start buttons, charts or cross-machine view for now; phone first? Which should go into the prototypes, or into new variants?

**Method.**

- Read the first-party docs, product pages and changelogs of each tool, cited inline. Where a page gave nothing on a point, the point is marked **[unverified]** or left out.
- **CI/CD:** GitHub Actions (run graph, logs, workflow commands, cancellation, job summaries, timeouts), Buildkite (build page, log groups, annotations, waterfall) and GitLab pipelines.
- **Orchestrators:** Temporal Web UI and its Activity heartbeats, the Airflow UI, the Dagster webserver and Argo Workflows (`stop` against `terminate`).
- **Agent and LLM UIs:** Langfuse (timeline, agent graphs), LangSmith (trace tree, cost), the GitHub Copilot coding agent's session pages, Devin's session tools, Session Insights and usage, the Cursor agent and its Cloud Agents API, and Codex cloud.
- **Tracing:** Jaeger and Grafana's trace view (critical path, minimap).
- **Principles:** the Google SRE book's chapters on monitoring and on-call; Kubernetes' progress deadline as a primary source on detecting a stall.
- **UX:** Nielsen Norman Group on progressive disclosure, on dashboards and on status trackers; GOV.UK Design System tags and task lists; Android's list-detail layout.
- Mapped each finding onto the thirdshift vocabulary in [CONTEXT.md](../../CONTEXT.md) and onto the seven prototype variants: `garden`, `plain`, `accent`, `press`, `pixel`, `briefing` and `lanes`.

**Out of scope.** Building any of it; the server, htmx and auth design; cross-machine views; anything that starts work. Apple's notification guidance was in scope, but its HIG page didn't render for the fetcher, so it isn't cited.

## TL;DR

- **Lead with what needs a human, not with what is running.** The SRE book says pages must be *actionable*, should *require intelligence*, and should be driven by symptoms ("what's broken") rather than causes ([SRE: Monitoring](https://sre.google/sre-book/monitoring-distributed-systems/), [SRE: Being on-call](https://sre.google/sre-book/being-on-call/)). Buildkite's build page can group steps by state, which puts failed steps and steps "Waiting for input" at the top, and the `f` key jumps to the next failure ([Buildkite build page](https://buildkite.com/docs/pipelines/build-page)). Every variant should open on a small "needs you" strip: died, stalled, failed, findings waiting.
- **"Stalled" should mean *no progress for a while*, not *a long time in one stage*.**
  - Kubernetes reports a Deployment as stalled when it makes no progress within `progressDeadlineSeconds`. It "takes no action … other than to report a status condition" ([Kubernetes Deployment](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)).
  - Temporal's Heartbeat Timeout is "the maximum time between Activity Heartbeats", and each heartbeat says the Worker "has not crashed" ([Temporal: detecting Activity failures](https://docs.temporal.io/encyclopedia/detecting-activity-failures)).
  - The Session log is thirdshift's heartbeat. Show "last event 40 s ago" next to "review · 52 min", and flag a stall on the gap since the last event, not on stage age alone.
- **Compare against what is normal here.** Airflow shows task durations across recent runs so you can "spot recurring issues or performance trends at a glance" ([Airflow UI](https://airflow.apache.org/docs/apache-airflow/stable/ui.html)). NN/g's status-tracker guidance says to "state exactly what the wait time will be" where possible ([NN/g: status trackers](https://www.nngroup.com/articles/status-tracker-progress-update/)). Write "review · 52 min (usually 10–20)", taking the typical time from earlier Command logs. That is text, not a chart.
- **Open failures by default; fold the rest.** GitHub expands failed steps automatically ([GitHub: workflow run logs](https://docs.github.com/en/actions/how-tos/monitor-workflows/use-workflow-run-logs)). Buildkite has `^^^ +++` to reopen the group that failed ([Buildkite: log output](https://buildkite.com/docs/pipelines/configure/managing-log-output)). The prototypes now fold everything, so make an exception for the failing tool call and for where a died Run stopped.
- **No more than two disclosure levels.** NN/g: "designs that go beyond 2 disclosure levels typically have low usability because users often get lost" ([NN/g: progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/)). The drill-down added today goes Command → Spec run → Ticket → details: four levels. Flatten it by showing Tickets inline as status chips under the Command, so one unfold reaches the details.
- **Say what Stop keeps.** Copilot's Stop session "preserves any commits already pushed" ([GitHub Copilot: manage agents](https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/manage-and-track-agents)). Argo separates `stop`, which lets exit handlers run, from `terminate`, which doesn't ([argo stop](https://github.com/argoproj/argo-workflows/blob/main/docs/cli/argo_stop.md), [argo terminate](https://argo-workflows.readthedocs.io/en/latest/cli/argo_terminate/)). thirdshift's interruption is the gentle kind: Failed run salvage, confirmed Self-merge completion, Claim ending and Spec accounting still finish ([CONTEXT.md: Command](../../CONTEXT.md)). The confirm step should say so.
- **New variant concepts the research points to:**
  - **Trace waterfall:** one Command as nested spans down to tool calls, with a critical path.
  - **Stage grid:** the Airflow-style stages × recent Runs table of durations.
  - **Parcel tracker:** one Run at a time as a plain-language, phone-first status tracker.

## 1. CI/CD run views

### 1.1 GitHub Actions

- **Run graph.** Each run gets a "real-time" graph of its jobs, with lines for dependencies and a status icon for each job. Clicking a job opens its log ([GitHub: visualization graph](https://docs.github.com/en/actions/how-tos/monitor-workflows/use-the-visualization-graph)).
  - *thirdshift:* the Pickup → Spec run → Ticket tree, with "waiting on #239" edges. The Spec run's Tickets checklist already holds the "blocked by" links. Fits the rules: it is read-only.
- **Logs.**
  - "Any failed steps are automatically expanded."
  - Each step shows how long it took.
  - A line number gives a shareable link to that line.
  - **Search logs** searches only expanded steps.
  - **Download log archive.**
  - The CLI equivalents are `gh run view --log` and `--log-failed`.

  Source: [GitHub: workflow run logs](https://docs.github.com/en/actions/how-tos/monitor-workflows/use-workflow-run-logs).
  - *thirdshift:* auto-open what failed, give each Session log line an anchor, and add a search box. The page/CLI parity rule is met because the Session log files are on disk and grep-able.
- **Groups and annotations.** `::group::` "Creates an expandable group in the log." `::error` / `::warning` / `::notice` create annotations that can point at a file and line ([GitHub: workflow commands](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands)). Job summaries, Markdown written to `GITHUB_STEP_SUMMARY`, show on the run's summary page so that "someone viewing the result of a workflow run doesn't need to go into the logs to see important information" (same page).
  - *thirdshift:* this is the case for a **Run summary** at the end of each Run: PR, outcome, cause, Unaddressed findings count, tokens. All of it is already in the Command log or the pull request body, so a Run summary is display only.
- **Cancel.** **Cancel workflow** is a single click with no confirm step, according to the docs ([GitHub: cancel a run](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/cancel-a-workflow-run)). The cancellation sequence re-evaluates `if: always()` and `cancelled()` steps, sends SIGINT, then SIGTERM after 7500 ms, then kills the process tree after 2500 ms more. After a 5-minute timeout the server forces the rest ([GitHub: workflow cancellation](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-cancellation)).
  - *thirdshift:* "Stopping…" is a real state that can take a while because salvage finishes. Show it, rather than flipping straight to interrupted.
- **Timeouts.** A job's `timeout-minutes` is "the maximum number of minutes to let a job run before GitHub automatically cancels it", 360 by default ([GitHub: workflow syntax](https://docs.github.com/actions/using-workflows/workflow-syntax-for-github-actions)). thirdshift has no automatic stall cancel, and the Console shouldn't add one, because the page does nothing the CLI can't. Showing a threshold is fine.

### 1.2 Buildkite

- **Log groups.**
  - `---` makes a collapsed group, `+++` an expanded one, and `~~~` a de-emphasised one; consecutive de-emphasised groups "fold into expander rows".
  - `^^^ +++` after a failing command reopens the previous group so the failure is visible.
  - Search steps through matches, including matches inside folded groups.
  - Each line carries a timestamp.

  Source: [Buildkite: managing log output](https://buildkite.com/docs/pipelines/configure/managing-log-output).
  - *thirdshift:* the Session log should **de-emphasise** routine groups (reads, greps), not only collapse them, and **reopen the failing group**. Search must reach into folded groups.
- **Annotations.** Markdown blocks on the build page with styles `success`, `info`, `warning` and `error`. A `--context` key updates the same annotation in place, and a priority sets the order ([Buildkite: annotate](https://buildkite.com/docs/agent/v3/cli-annotate)).
  - *thirdshift:* a Run's summary and its Security findings are annotations in this sense: one styled block per subject, updated in place.
- **Build page.**
  - The sidebar is a hierarchical step list with collapsible groups. Selecting a step opens a docked step panel with **Logs**, **Artifacts** and **Environment** tabs.
  - Skipped steps show their reason without opening a log, and retried steps get an attempt selector.
  - **Group by state** puts failed steps and "Waiting for input" at the top.
  - `f` jumps to the next failure and `j` follows active steps.
  - The **Waterfall** view is for timing and "is hidden on mobile".

  Source: [Buildkite: build page](https://buildkite.com/docs/pipelines/build-page).
  - *thirdshift:* show the skipped reason inline, as the Architect run already does. The Resume and each Repair are the attempts. Like Buildkite, the waterfall- or lanes-style timing view needn't be the phone view.
- **Waterfall.** Each bar splits into time waiting for an agent, time from assignment to start, and run time. Parallel steps nest under a parent row, and a canceled job is a blank row ([Buildkite: waterfall](https://buildkite.com/docs/pipelines/insights/waterfall)).
  - *thirdshift:* the lanes variant is this view. Its bars should separate **waiting** (a Ticket blocked, or a Base fix awaited) from **working**.

### 1.3 GitLab pipelines

The pipeline page draws every job, grouped by stage or by `needs` dependencies, and lets you "cancel a running pipeline, retry failed jobs, or delete a pipeline" ([GitLab: pipelines](https://docs.gitlab.com/ci/pipelines/)). Its total running time leaves out queue time and the first runs of retried jobs (same page). For thirdshift, "how long" should be stated as one of two things: time working, or wall clock including waiting.

## 2. Workflow orchestrators

- **Temporal Web UI.**
  - The Workflow page has history views: **Timeline** ("chronological or reverse-chronological order of events with a summary"), **All**, **Compact** ("a logical grouping of Activities, Signals and Timers") and **JSON**.
  - **Pending Activities** gives a summary of active or pending Activity Executions.
  - **Call stack** shows "each location where Workflow code is waiting".
  - **Relationships** "displays the full hierarchy … with all parent and child nodes displayed in a tree".
  - From the UI you can request Cancellation, or Reset and Terminate.

  Source: [Temporal Web UI](https://docs.temporal.io/web-ui).
  - *thirdshift:* the **Compact** idea matches the folded tool-call groups. **Where it is waiting** matches "waiting on #239", or "waiting on CI for 4f1c2a9", or Resume. A **Relationships** tree is the Pickup → Spec run → Tickets → Base fix nesting.
- **Heartbeats as stall detection.** "An Activity Heartbeat is a ping from the Worker … Each ping informs the Temporal Service that the Activity Execution is making progress and the Worker has not crashed." "A Heartbeat Timeout is the maximum time between Activity Heartbeats" ([Temporal: detecting Activity failures](https://docs.temporal.io/encyclopedia/detecting-activity-failures)).
  - *thirdshift:* the time since the Session log's last event, or the Command log's last line during CI watch, is the heartbeat. A Run whose log still moves is slow; a Run whose log has stopped is suspect; a Command with no process is "died at <stage>".
- **Airflow.**
  - The **Grid** view has a row per task and a column per run, coloured by state, with a sidebar of task durations across recent runs to "spot recurring issues or performance trends at a glance".
  - **Graph** nodes show duration.
  - The Task Instances tab has a "mini Gantt-style timeline" in each row.
  - The Dag Run **Details** tab shows "Queued At" beside the start and end times.

  Source: [Airflow UI](https://airflow.apache.org/docs/apache-airflow/stable/ui.html).
  - *thirdshift:* a **stage × recent Runs** grid of durations, as text cells, answers "is review always this slow on chart35?". It also gives the "usually" figure in the TL;DR.
- **Dagster.** The run page has a Gantt-style timeline of "how long each asset or op took", plus a log pane that switches between **structured event logs** ("enriched and categorized with metadata") and **raw compute logs** (stdout and stderr, downloadable) ([Dagster webserver](https://dagster.io/docs/guides/operate/webserver)).
  - *thirdshift:* the same toggle on a Session log, between folded tool calls and raw JSONL. The Command log is already the raw stderr.
- **Argo Workflows.** `argo stop` stops "allowing all exit handlers to run". `argo terminate` will "immediately stop a workflow and do not run any exit handlers" ([argo stop](https://github.com/argoproj/argo-workflows/blob/main/docs/cli/argo_stop.md), [argo terminate](https://argo-workflows.readthedocs.io/en/latest/cli/argo_terminate/)).
  - *thirdshift:* only the gentle stop exists, and the confirm should name what still runs: salvage, Claim ending and Spec accounting ([CONTEXT.md: Command, Failed run](../../CONTEXT.md)).

## 3. Agent and LLM trace UIs

- **GitHub Copilot coding agent.**
  - The session log shows "Copilot's internal reasoning and the tools it used".
  - The session overview lets you "monitor the agent's progress, token usage, and session length".
  - **Stop session** "ends the GitHub Actions run and preserves any commits already pushed".
  - You can steer mid-session from a prompt box. The steer applies "after it finishes its current tool call" and costs AI credits.

  Source: [GitHub: manage and track agents](https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/manage-and-track-agents).
  - The CLI has `gh agent-task view --log --follow` to stream a session ([GitHub Enterprise: tracking sessions](https://docs.github.com/en/enterprise-cloud@latest/copilot/using-github-copilot/using-copilot-coding-agent-to-work-on-tasks/using-the-copilot-coding-agent-logs)).
  - Each session has a 59-minute limit ([GitHub: troubleshoot cloud agent](https://docs.github.com/en/enterprise-cloud@latest/copilot/how-tos/use-copilot-agents/cloud-agent/troubleshoot-cloud-agent)).
  - *thirdshift:* **tokens and session length per Session log** are expected. thirdshift already prints "session ended after 26m 2s: 7416316 input tokens (7285760 cached), 20664 output tokens" (as quoted on [thirdshift.app](../../site/index.html)), so the Console can show those numbers. Steering would break the "nothing the CLI can't" rule, since thirdshift has no command for it. Leave it out.
- **Cursor agents.**
  - Status is `ACTIVE` ("a turn is running, waiting on background work, or about to start") or `IDLE` ("the last turn finished and follow-ups are accepted") ([Cursor Cloud Agents API](https://cursor.com/docs/cloud-agent/api/endpoints)).
  - Stop "stops Agent mid-task" ([Cursor: Agent mode](https://cursor.com/help/ai-features/agent)).
  - Stopping a subagent also stops its nested subagents ([Cursor release notes](https://cursor.com/docs/release-notes)).
  - *thirdshift:* "waiting on background work" is exactly when a **Resume** is needed ([CONTEXT.md: Resume](../../CONTEXT.md)), so show background tasks still running in a session. Stopping the Pickup run stops its children, and the confirm step should list them.
- **Devin.**
  - The Progress tab collects shell commands, edits and browser activity into one view, and any step opens to its details ([Devin session tools](https://docs.devin.ai/work-with-devin/devin-session-tools)).
  - **Session Insights** draws a colour-coded timeline of "where Devin made progress, where it hit obstacles, and how it recovered" ([Devin Session Insights](https://docs.devin.ai/product-guides/session-insights)).
  - Idle sessions sleep after 30 minutes by default ([Devin usage](https://docs.devin.ai/admin/billing/usage)).
  - *thirdshift:* Repairs, Resumes, Check re-runs and upstream moves are the "obstacles and recoveries" of a Run. A history entry could list them in order: "CI red → Repair 1 → green".
- **Langfuse.**
  - The **timeline** view helps "identify latency bottlenecks, visualize parallelism, and understand multi-step reasoning in deeply nested chains" ([Langfuse: timeline view](https://langfuse.com/changelog/2024-06-12-timeline-view)). A 2026 update fits any trace on one screen, with zoom, pan and focus ([Langfuse: responsive timeline](https://langfuse.com/changelog/2026-08-28-responsive-timeline)).
  - **Agent graphs** come in two modes. **Aggregated** has one node per step name with a counter, "for grasping structure & complexity at a glance". **Expanded** has one node per call, "to walk through a single run" ([Langfuse: agent graphs](https://langfuse.com/docs/observability/features/agent-graphs)).
  - *thirdshift:* the folded tool-call group is the Aggregated mode ("6 tool calls · Bash ×3, Read, Edit ×2"). Unfolding it is the Expanded mode. Show counts per tool in the summary.
- **LangSmith.** The trace tree shows token usage and cost for the whole trace, an aggregate for each parent run, and a breakdown for each child run ([LangSmith: cost tracking](https://docs.langchain.com/langsmith/cost-tracking)). The Details view shows inputs, outputs, timing, token counts and errors for one run ([LangSmith: view traces](https://docs.langchain.com/langsmith/view-traces)).
  - *thirdshift:* add up tokens from Ticket Run to Spec run to Command. The Run notification already collects Model evidence per Command ([CONTEXT.md: Run notification](../../CONTEXT.md)).
- **Codex cloud.** When a task finishes it shows "its answer and a diff of any files it changed". `codex apply` applies the latest diff locally ([Codex Cloud (Legacy)](https://developers.openai.com/codex/cloud/environments)). Task cancelling was added in June 2025 ([Codex changelog](https://developers.openai.com/codex/changelog)).
  - *thirdshift:* the Run summary could include a diffstat ("3 files, +14 −21"), which can be read from git.

## 4. Trace timelines

- **Critical path.** Jaeger highlights "which operations are on the critical path of the overall trace duration", on by default ([Jaeger UI](https://www.jaegertracing.io/docs/latest/frontend-ui/)). In Grafana, the darker segment of each span bar marks the critical path, and the switch "helps identify performance bottlenecks and their impact on overall latency" ([Grafana traces panel](https://grafana.com/docs/grafana/latest/panels-visualizations/visualizations/traces/), [Grafana trace view](https://grafana.com/docs/grafana/latest/explore/trace-integration/)).
  - *thirdshift:* in a **Spec run**, the critical path is the chain of blocking Tickets that decides when the Spec PR can start: #239 → #240 → Spec review. Marking it answers "what is the Spec run waiting on?".
- **Minimap and collapse.** Grafana's trace view has a minimap you drag to zoom the timeline, and an **Expand children** control that "expands or collapses all the children spans of the selected span" ([Grafana trace view](https://grafana.com/docs/grafana/latest/explore/trace-integration/)).
  - *thirdshift:* "Unfold all" and "Fold all" controls belong with the drill-down.

## 5. Principles for what earns attention

- **Symptoms first.** Monitoring answers "what's broken" (the symptom) and "why" (the cause). Paging should follow symptoms, and causes are mostly debugging aids ([SRE: Monitoring](https://sre.google/sre-book/monitoring-distributed-systems/)). For the Console, the symptoms are "a Run has been silent for 30 min", "a process died", "a Claim is waiting on the Day shift". The causes, such as a Repair or a flaky check, sit one fold down.
- **Actionable, not robotic.** "Every page should be actionable … If a page merely merits a robotic response, it shouldn't be a page." Dashboards "should answer basic questions about your service", and the chapter warns against too many alerts, metrics and dashboards (same chapter). Alert fatigue makes "serious alerts … treated with less attention than necessary", and the aim is close to one alert per incident ([SRE: Being on-call](https://sre.google/sre-book/being-on-call/)).
  - *thirdshift:* one problem gets one "needs you" item. A failed Spec run with two failed Tickets is one item, not three.
- **Report a stall; don't act on it.** Kubernetes "takes no action on a stalled Deployment other than to report a status condition" ([Kubernetes Deployment](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)). Same for the Console: it marks a stall and offers Stop. It never stops anything by itself.

## 6. UX guidance

- **Progressive disclosure.** It "defers advanced or rarely used features to a secondary screen". What shows first signals importance. Beyond two levels, "users often get lost" ([NN/g: progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/)).
- **Dashboards.** They give "at-a-glance information on which users can act quickly". Position and length read best. Colour "should only be used to reinforce information that is already communicated in a different way" ([NN/g: dashboards](https://www.nngroup.com/articles/dashboards-preattentive/)).
  - *thirdshift:* every coloured or drawn state needs words next to it. This bears on the garden and pixel sprites, and on lanes' amber and red.
- **Status trackers.**
  - "Present the latest update prominently", in plain language, "free from jargon".
  - Keep earlier updates with their dates.
  - State expected waits.
  - Long silences make users "think perhaps something went wrong".

  Source: [NN/g: status trackers](https://www.nngroup.com/articles/status-tracker-progress-update/).
- **Status tags.**
  - Use adjectives, not verbs.
  - "Do not use colour alone."
  - "Do not make a tag interactive."
  - Each extra status is harder to remember.

  Source: [GOV.UK tag](https://design-system.service.gov.uk/components/tag/).
  - The GOV.UK task list adds **"Cannot start yet"**, in grey text with no link, for a task that depends on another ([GOV.UK: complete multiple tasks](https://design-system.service.gov.uk/patterns/complete-multiple-tasks/)). That is the "waiting on #239" Ticket.
- **List-detail on phones.** "In small windows, only one pane is visible at a time." Back returns to the list ([Android: list-detail](https://developer.android.com/develop/ui/compose/layouts/adaptive/list-detail)). The `accent` variant already works this way.

## 7. Ranked features for the Console prototypes

| # | Feature | Answers | Sources | Fits / contradicts |
|---|---|---|---|---|
| 1 | **"Needs you" strip above everything**: died, silent too long, failed with cause, findings waiting, needs-triage. One item per problem. | "Do I have to do anything?" | SRE monitoring and on-call; Buildkite group-by-state | `briefing` is built on it. Add a strip to `plain`, `accent`, `garden`, `press`, `pixel` and `lanes` without reordering the settled live-then-history body. |
| 2 | **Heartbeat age**: "last event 40 s ago" beside stage · age. Flag a stall on silence, not stage length. | "Slow or stuck?" | Temporal heartbeats; Kubernetes progress deadline | All variants. Changes the stall rule in `lanes`, `press`, `pixel`, `briefing` and `accent`, which today flag stage age past 45 min. |
| 3 | **"Usually" durations** from earlier Command logs: "review · 52 min (usually 10–20)". | "Is this abnormal for this repo?" | Airflow durations across runs; NN/g status trackers | Text only, so no chart. Fits all; best in `lanes` as a ghost bar and in `briefing` sentences. |
| 4 | **Failures open by default** in Session and Command logs: the failing tool call, the last group before a death, the error line. | "What went wrong?" without hunting | GitHub auto-expand; Buildkite `^^^ +++` | Amends today's "everything starts folded" in every variant. |
| 5 | **Two disclosure levels at most**: Tickets as inline chips (state + age) under their Command, so one unfold reaches details. | "Where is each Ticket?" at a glance | NN/g progressive disclosure | Flattens the four-level drill-down in `lanes`, `plain` and `garden`. |
| 6 | **Run summary card** for each ended Run: PR, outcome, cause, Unaddressed findings, Repairs/Resumes, diffstat, tokens. | "What came of it?" without logs | GitHub job summaries; Buildkite annotations; Codex diff | History in every variant; `press` proof cards and `garden` harvest log suit it best. |
| 7 | **Tokens and session length** per Session log, added up to Spec run and Command. Numbers, not charts. | "What did this cost?" | Copilot session overview; LangSmith trace tree | All. thirdshift already prints these at session end. |
| 8 | **Stop confirm says what continues**: pushed work kept; salvage, Claim ending and Spec accounting finish; child Runs listed; "Stopping…" state until the process ends. | "What happens if I press this?" | Copilot Stop; Argo stop vs terminate; GitHub cancellation; Cursor nested stop | All. Edit each confirm. |
| 9 | **Session log search, line anchors, structured/raw toggle.** | "Find the error; send someone the line" | GitHub logs; Buildkite search; Dagster toggle | All log views. A raw JSONL download is CLI parity. |
| 10 | **De-emphasised routine groups**: reads and greps quieter than edits, Bash failures and commits. Group summary counts per tool. | "What did it actually do?" | Buildkite `~~~`; Langfuse aggregated mode | All `.sl-*` styles. |
| 11 | **Follow live with a "N new lines ↓" pill** instead of silent auto-scroll. | "Is it still moving?" while reading old lines | Buildkite follow `j`; Copilot live logs | All log views. |
| 12 | **Critical path in a Spec run**: mark the blocking chain that gates the Spec PR. | "What is the Spec run waiting on?" | Jaeger and Grafana critical path; GitHub run graph | `lanes`, `press`, `accent`, `briefing`. |
| 13 | **Attempts and recoveries in order**: implement → CI red → Repair 1 → Check re-run → green. | "How hard was this Run?" | Devin Session Insights; Buildkite attempts | History in all variants; `lanes` and `press` as markers on the line. |
| 14 | **Waiting is not working**: draw or label blocked time (Ticket waiting, Base fix awaited, CI queue) apart from session time. | "Why did it take 3 h?" | Buildkite waterfall segments; GitLab running time | `lanes` (hatch exists), `press` (feeder). |
| 15 | **Words beside every colour and sprite.** Status tags are adjectives and not links; "Cannot start yet" for blocked Tickets. | Accessibility; glanceability | GOV.UK tag and task list; NN/g dashboards | Check `pixel`, `garden` and `lanes` (amber and red alone today). |

## 8. New variant concepts the research points to

1. **Trace waterfall: one Command, top to bottom.** Jaeger, Grafana, Langfuse and the Dagster run page all show a single execution as nested bars on a shared time axis. Choose a Command, and every **Session**, **Repair**, **Resume**, CI watch and **Base fix** becomes a span, with tool-call groups as child spans you can expand. A minimap scrubs the time range. The **critical path** is drawn darker, and silent gaps (no heartbeat) are hatched. `lanes` shows every Command against the wall clock; this shows one Command in depth, from Run down to tool call. It replaces the separate Session log panel with spans you can open in place. On a phone it degrades to the span list, without the bars, as Buildkite hides its waterfall on mobile.
2. **Stage grid: is this normal?** This is the Airflow Grid applied to thirdshift. Rows are the stages (implement, review, Delivery, CI watch, Repair, Self-merge, cleanup) and columns are the last 10–20 Runs on this machine, newest on the left. The live ones are pinned and outlined. Each cell is a coloured state with the duration as text ("14m"), so it stays a table, not a chart. Repairs and Resumes add a dot. A live cell more than twice its row's usual time is flagged. It answers "is review always slow on chart35?" and "did CI watch get slower this week?", questions no current variant can. Live-first holds because the live columns come first. On a phone the grid transposes into one Run per card, each card a list of stage durations.
3. **Parcel tracker: one Run, plain-language updates.** This follows NN/g's status-tracker guidance and GOV.UK's task list. Each Run is a tracking page: the latest update in a large sentence ("Reviewing its own change. Last activity 40 s ago; review usually takes 10–20 min here"), then a dated history of updates ("09:46 Tests passed · 09:31 Started review · 08:46 Started"), then what happens next ("Then Delivery: push, mark ready, watch CI"). Tickets that are blocked read "Cannot start yet: waiting on chart35#239". The home page is a short list of trackers, with "needs you" items first. It's built for the phone and one-handed reading, and differs from `briefing`, which summarises the whole machine rather than tracking one Run.

## 9. Revisions to existing variants

- **All variants.**
  - Add the "needs you" strip (#1) and heartbeat ages (#2).
  - Change the stall rule from stage age to silence.
  - Open failures by default (#4).
  - Show tokens and session length (#7).
  - Extend the Stop confirm (#8).
  - Add a "N new lines ↓" pill (#11).
  - Make routine tool groups quieter (#10).
- **`lanes`.**
  - Flatten the drill-down: Ticket chips on the Spec run lane (#5).
  - Draw a "usually" ghost bar behind the current stage (#3).
  - Mark the critical path (#12).
  - Add Repair markers on bars (#13).
  - On phones, consider showing the list instead of the axis, following Buildkite.
- **`plain`.** Show Tickets as inline chips under the Spec run row (#5). Add a Run summary line to each history row (#6).
- **`accent`.** Already list-detail, as Android recommends. Add tokens, attempts and the critical path to the detail pane (#7, #12, #13). Make the stepper show "usually" times (#3).
- **`garden` and `pixel`.** Pair every sprite state with words (#15). Base the stall bubble on silence (#2). Turn harvest or ledger entries into Run summary cards (#6).
- **`press`.** Make the misprint box show the last heartbeat and the last failing tool call open (#2, #4). Give the delivery pile's proof cards Run summaries (#6). Keep the feeder for waiting time (#14).
- **`briefing`.** Already closest to the SRE guidance. Add "usually" times and heartbeat ages to its sentences (#2, #3). Collapse a failed Spec run's Tickets into one item (one problem, one item).
