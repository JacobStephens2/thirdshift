# Console UI: what to take or borrow from open-source factory UIs and session-log viewers

Research date: 2026-10-18.

**Question.** What can thirdshift take (code, under a licence compatible with its MIT licence) or borrow (ideas, layouts, interaction patterns) from open-source software-factory and agent-control UIs, and from agent session-log viewers? It is for the page ("console", name not final) planned in #711 and #574. A read-only-ish `thirdshift serve` on localhost, behind Caddy, would show:

- live Commands, each with its child Runs and the stage each is in (implement, review, Repair, CI watch, Self-merge);
- the full history of Commands on the host;
- their Command logs and Session logs;
- a Stop action.

Starting runs and charts come later. The hard rule is that **the page may do nothing the CLI can't do**. The page ships inside the Rust binary. The host has 4 cores and 8 GB, and no Docker.

This builds on [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md) (the "earlier survey") and [openhands-fit.md](openhands-fit.md). Those two notes judged the products as replacements for thirdshift. This one looks only at their UIs, and at log viewers.

**Method.**

- Source was read from shallow clones made on 2026-10-18. Code citations link to a file at a fixed commit:
  - `warpdotdev/docs@756982e`, and `warpdotdev/warp@1e6ef37` read through `gh api`
  - `OpenHands/OpenHands@852132d`
  - `openai/symphony@be10a1b`
  - `BloopAI/vibe-kanban@d5cbb53`
  - `gastownhall/gastown@649b832`
  - `agent-of-empires/agent-of-empires@7086f81`
  - `daaain/claude-code-log@9834d25`
  - `simonw/claude-code-transcripts@316fd09`
  - `ccusage/ccusage@b090428`
  - `jhlee0409/claude-code-history-viewer@7f48907`
  - `Brads3290/cclogviewer@951bad6`
- Licences and repository metadata come from `gh api repos/<owner>/<repo>` and each repo's LICENSE file, read on 2026-10-18. Package sizes and licences come from `npm view` and `npm pack`.
- Warp docs pages are cited by their live URL. The same text and screenshots are in `warpdotdev/docs` at the commit above.
- thirdshift's own Session log formats were checked against real files under `~/.thirdshift/logs/` on this host.
- **[interpretation]** marks my reading. **[unverified]** marks a claim I could not confirm from a primary source.
- Nothing was installed or run, apart from `npm pack` to measure packages.

**Out of scope.** Visual design (a prototype session), the live-record format (#711 Q3), and how the page is authenticated (#711 Q4).

## TL;DR

- **Warp: no Factories UI code is public.** The Factories dashboard is a closed web app at `platform.warp.dev`. The desktop Agent Management Panel is in the public client, but it is AGPL-3.0 Rust drawn with Warp's own GPU UI framework, so it is neither copyable into an MIT project nor usable in a browser. Worth borrowing:
  - runs grouped under their parent, with a **Sub-agents** tab;
  - a small status set (Working, Blocked, Failed, Succeeded, Cancelled);
  - **Stop run** shown only while a run can be cancelled;
  - an Inbox of what needs a human. (§1)
- **OpenHands Agent Canvas: MIT, but too heavy to embed.** It is React 19 + React Router 7 + Vite + Tailwind 4 + HeroUI + TanStack Query, with xterm, Monaco and i18next. The prebuilt frontend in the npm package is 10.3 MB, 6.5 MB of it JavaScript. Its components render OpenHands' own event schema, not Claude or Codex JSONL. Borrow the ideas:
  - a run's **phase plus how long it has been in that phase**, which tells a moving run from a stalled one;
  - cancel only when cancellable;
  - consecutive tool calls folded into one group;
  - a per-tool "visualizer" registry with a generic fallback. (§2)
- **Symphony: yes, a dashboard, but no Stop.** The SPEC makes the HTTP dashboard optional and requires it to be an observability surface only. The Elixir reference has a Phoenix LiveView page (metric cards, running, blocked and retry tables) and a terminal status view. Borrow:
  - the read-only JSON API (`GET /api/v1/state`, `GET /api/v1/<issue>`), with the page reading that API rather than keeping its own state logic;
  - binding to loopback by default. (§3)
- **Others.** Two are close in shape to thirdshift's page:
  - **Gas Town** (MIT, Go): a server-rendered dashboard embedded in the binary. It uses htmx with idiomorph, and SSE nudges a morph re-render. Its actions are an **allowlist of CLI commands** with Safe or Confirm flags. That is the closest existing match to "the page may do nothing the CLI can't do".
  - **Vibe Kanban** (Apache-2.0, Rust, sunsetting) and **agent-of-empires** (MIT, Rust): a React bundle built by npm and embedded with `rust-embed`. That is a working precedent, but it puts Node into the release build.

  Claude Squad and opcode are AGPL. Superset is ELv2. claude_code_agent_farm is MIT with an OpenAI/Anthropic rider. (§4)
- **Log viewers.** thirdshift's Session logs are Claude `--output-format stream-json` stdout and `codex exec --json` stdout. Most viewers parse something else: Claude's on-disk transcripts (`~/.claude/projects`) and Codex rollouts (`~/.codex/sessions`). The content blocks are the same Anthropic shape, so their *rendering* carries over. The parts worth taking or porting:
  - Vibe Kanban's `ClaudeJson` serde types parse stream-json directly in Rust (Apache-2.0).
  - simonw's Jinja macros (Apache-2.0) are about 190 lines.
  - claude-code-log (MIT) has a complete tool-by-tool formatter set, and it measured that re-rendering a whole session per append doesn't scale. (§5)
- **Recommendation.**
  - **Stack.** Server-rendered HTML from Rust, plus htmx and idiomorph (both 0BSD, about 20 KB gzipped together, vendored) and one small hand-written ES module. Compile it in with the `include_dir` crate thirdshift already depends on. SSE carries both the "something changed" signal and Session-log tails. No npm in the build, matching how `site/` is already built.
  - **Order of work.** Add `thirdshift status --json` and `thirdshift stop` first, then build the page on them, Symphony- and Gas-Town-style.
  - **Avoid.** Lifting the OpenHands or React-based UIs, and anything AGPL or ELv2. (§6)

## 1. Warp

### 1.1 Is any Factories, Oz or Automation Platform UI code public?

**No.** The evidence:

- **The dashboard is a hosted web app.** "The factory dashboard is the web app for operating a single factory". Its URL is `https://platform.warp.dev` ([factory dashboard](https://docs.warp.dev/factories/factory-dashboard/); `VARS.FACTORY_WEB_APP_URL` in [vars.ts](https://github.com/warpdotdev/docs/blob/756982e49a28058e6657616d262660b333aed779/src/data/vars.ts)).
- **None of the 100+ `warpdotdev` repositories holds that web app** (`gh api orgs/warpdotdev/repos`, 2026-10-18).
  - The client repo `warpdotdev/warp` is 67 MB of Rust and 3.9 KB of TypeScript (`gh api repos/warpdotdev/warp/languages`).
  - A code search for `platform.warp.dev` in it returns nothing.
- **What the client does have** is the desktop **Agent Management Panel**, under `app/src/ai/agent_management/` (`view.rs` alone is 97 KB). It is written against Warp's own `warpui` GPU UI framework.
  - **Licence:** "Warp's UI framework (the `warpui_core` and `warpui` crates) are licensed under the MIT license. The rest of the code in this repository is licensed under the AGPL v3" ([README](https://github.com/warpdotdev/warp/blob/1e6ef379bc9f51257c4243e8c4396a046cede8af/README.md)).
  - So the panel's code is AGPL-3.0. Copying it into thirdshift would make the combined work AGPL, which conflicts with thirdshift's MIT licence ([Cargo.toml](../../Cargo.toml) `license = "MIT"`) **[interpretation]**.
  - The MIT `warpui` framework draws native windows, not HTML, so it has no use in a browser page either.
- **Other public pieces with UI:**
  - `oz-workspace` (Apache-2.0) is a *sample* chat-room-and-Kanban app on the Oz API. It is built on Next.js 16, Prisma with SQLite, NextAuth, Tailwind with shadcn/ui, Zustand and SSE ([README](https://github.com/warpdotdev/oz-workspace)). It is not the Factories UI, and its stack is far heavier than this page needs.
  - `session-sharing-protocol` is AGPL-3.0 (its README).
  - `power-fixer-server` (MIT, Rust) is an API server for an issue-triage demo, backed by Postgres ([README](https://github.com/warpdotdev/power-fixer-server)).
- **Nothing to take as code.** The earlier survey's §3.6 licence list still holds.

### 1.2 What the Factories UI shows

From the docs pages and their screenshots:

- **Layout.**
  - A sidebar holds team-wide pages (Runs, MCPs and apps, Secrets, Integrations) above a factory list. A global **Inbox** sits beside them.
  - A factory's pages are **Dashboard** (metrics), Runs, Agents, Automations (with a Webhooks tab), Factory definition, Scorers, Self-improvement, Benchmarks and Settings ([factory dashboard](https://docs.warp.dev/factories/factory-dashboard/)).
- **Work items and stages.**
  - "A work item is a single request the factory acts on … It keeps its identity from intake to handoff, however many agents contribute to it along the way."
  - The stages are Intake, Triage, Planning, Building, Reviewing, Human handoff, then Complete or Cancelled ([how factories work](https://docs.warp.dev/factories/how-factories-work/)).
  - The system diagram lists "Work items by stage" and "Runs and costs" as dashboard contents ([diagram](https://github.com/warpdotdev/docs/blob/756982e49a28058e6657616d262660b333aed779/src/assets/agent-platform/factories-work-item-flow.png)).
  - I found no screenshot of a by-stage view in the docs. Its layout is **[unverified]**.
- **Runs.**
  - "A single request can span several runs as different agents pick it up, and related runs group under their parent in the list."
  - Opening a run shows "its timeline and cost, plus a **Sub-agents** tab for child runs". From there you can "view the agent's full session, stop or score the run" ([factory dashboard](https://docs.warp.dev/factories/factory-dashboard/#inspect-runs)).
  - Run fields are Status, Title, Environment, Creator, Source, Artifacts (PRs, files) and Cost or credits.
  - Quick filters are All, Mine, Active, Failed and Recurring, plus search ([web app Runs](https://docs.warp.dev/platform/oz-web-app/)).
- **Status set.** Working, Blocked, Cancelled, Failed/Errored and Success, each with its own icon ([managing cloud agents](https://docs.warp.dev/platform/managing-cloud-agents/)).
  - "The parent's own status reflects only its work - a parent can finish successfully while a child is still running or has failed" (same page).
  - The panel screenshot shows one row per run: a status icon, a title, then "Source · Run time · Credits used", a relative time and the creator's avatar, with a filter bar across the top ([screenshot](https://github.com/warpdotdev/docs/blob/756982e49a28058e6657616d262660b333aed779/src/assets/agent-platform/management-view-scannable-list.png)).
- **Logs.**
  - The run's "shared session" shows "the prompt, plan, commands, logs, outputs, and follow-up messages", live while the run is going and as a transcript afterwards ([session sharing](https://docs.warp.dev/platform/viewing-cloud-agent-runs/)).
  - Clicking a row opens it: "find the thing you care about, click once, and you're in the right context" ([managing cloud agents](https://docs.warp.dev/platform/managing-cloud-agents/)).
- **Stop.**
  - "Open the run on the factory's Runs page and click **Stop run**. It takes effect immediately, with no confirmation prompt" ([troubleshooting](https://docs.warp.dev/factories/troubleshooting/)).
  - The API is `POST /agent/runs/{runId}/cancel`. "Cancelling the parent does **not** automatically cancel its children." "Self-hosted, local, and GitHub Action runs cannot be cancelled through this endpoint" ([multi-agent runs](https://docs.warp.dev/platform/orchestration/multi-agent-runs/)).
  - In the client, the Cancel button is configured only "if `display_status.is_cancellable()`" ([details_action_buttons.rs L42-L55](https://github.com/warpdotdev/warp/blob/1e6ef379bc9f51257c4243e8c4396a046cede8af/app/src/ai/agent_management/details_action_buttons.rs#L42-L55)).
- **Stale states.** A Warp spec plans to reconcile a "stale raw `InProgress` task with terminal matching conversation" into Done or Failed by a derived display status ([spec APP-4382, increment 3](https://github.com/warpdotdev/warp/blob/1e6ef379bc9f51257c4243e8c4396a046cede8af/specs/APP-4382/INCREMENT-3-agent-management-and-details.md)). That is the same problem as #711's "died at review".
- **Inbox.** Asked question, Answered question, Spec review, PR review, Blocked and Failed. "Items clear on their own once they stop being relevant", with read, unread and resolved states ([factory inbox](https://docs.warp.dev/factories/factory-inbox/)).

### 1.3 What thirdshift can borrow

| Warp idea | thirdshift equivalent | Note |
|---|---|---|
| Runs nested under a parent, with a Sub-agents tab | Command → Spec run → Ticket Runs, and Base fix | Show the parent's own outcome apart from its children's, as Warp does |
| One scannable row per run, with a filter bar | Command history | Filters: repo, kind (Run, Spec run, Pickup, Architect, Security), outcome, date |
| Work item keeps its identity across agents | An Issue keeps one row across implement, review, Repair and Self-merge | Stage as a column, not a new row per session |
| Stop only when cancellable; no confirm | Stop only on a live Command whose process is alive | thirdshift's Stop discards less than Warp's (the work is pushed on failure, per [README.md](../../README.md) "Failed runs"), so no confirm is defensible **[interpretation]** |
| Derived display status that fixes stale "in progress" | "Died" when the live record's process is gone (#711 Q3) | Same pattern |
| Inbox of what needs a human | Failed runs, Unaddressed findings, offered Base fixes | Later; out of scope for v1 |
| Cost per PR, autonomy, cycle time | Later charts | Session logs already carry usage (§5.1) |

## 2. OpenHands Agent Canvas

- **Repository facts.** `OpenHands/OpenHands` is MIT ([LICENSE](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/LICENSE)). It is published as npm `@openhands/agent-canvas` 1.27.1 and needs Node ≥ 24 ([package.json](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/package.json)).
- **Stack** (same package.json):
  - React 19.3 and React Router 7, built with Vite 8.
  - Tailwind 4 and `@heroui/react`.
  - TanStack Query and Zustand.
  - `@xterm/xterm`, `monaco-editor`, `react-markdown` with remark and rehype, `react-syntax-highlighter`.
  - i18next, `framer-motion`, and `posthog-js` for telemetry.
- **Size.** 665 `.tsx` files, about 176k lines of TypeScript under `src/`.
  - The npm tarball unpacks to 77 MB in 11,654 files.
  - Its prebuilt `build/` frontend is 10.3 MB: 6.5 MB of JavaScript in 309 files, plus a 440 KB CSS file. The largest chunks are vendor bundles of 0.5–0.6 MB each, a 472 KB markdown chunk and a 341 KB terminal chunk (`npm pack @openhands/agent-canvas@1.27.1`, measured 2026-10-18).

### 2.1 What it shows

- **Running automations and history.**
  - The home page lists running automations. Each row has a health dot, an activity sparkline, its trigger (cron or event), a status tooltip, and a menu with Run now, **Cancel run**, View and Edit ([running-automations-list.tsx](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/home/featured-automations/running-automations-list.tsx); [build-home-automation-menu-items.tsx L60-L82](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/home/featured-automations/build-home-automation-menu-items.tsx#L60-L82)).
  - Cancel appears only when `canCancel` is true. "Run now" is disabled while a run can be cancelled (same file).
  - The backend call is `POST /api/automation/v1/runs/{run_id}/cancel` ([automation-service.api.ts L621-L625](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/api/automation-service/automation-service.api.ts#L621-L625)).
- **Run history.**
  - An automation's detail page has an activity log. Each row shows a timestamp, a summary, a cost (`$0.0000`, with a real zero shown and an unknown cost hidden), a status badge, the phase, and a terminal icon that opens a stdout/stderr **logs modal** ([activity-log-item.tsx](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/automations/detail/activity-log-item.tsx); [run-logs-modal.tsx](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/automations/detail/run-logs-modal.tsx)).
  - Run statuses are PENDING, RUNNING, COMPLETED, FAILED, CANCELLED and SKIPPED ([types/automation.ts L161-L168](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/types/automation.ts#L161-L168)).
- **Phase and phase age.** [run-phase.tsx](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/automations/detail/run-phase.tsx) is the most transferable idea in the repo:
  - A phase is shown only for pending, running or failed runs. "It answers 'what is it doing now' or 'where did it stop'" (L17).
  - Its age is shown only while the run is in flight. "Only its age says whether it entered that phase seconds ago or forty minutes ago" (L35, L114).
  - Known phase codes are translated. Unknown ones fall back to the author's label, then to the raw code (L91).
- **Conversations (sessions).**
  - Events are rendered by a registry of per-tool "visualizers": bash, file editor with a diff view, search and task. Anything else falls back to markdown ([dispatcher.tsx](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/chat/tool-visualizers/dispatcher.tsx)).
  - Runs of two or more consecutive tool calls fold into one collapsible group. Finish, think, error and message events break a group ([group-events.ts L14-L40](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/conversation-events/chat/group-events.ts#L14-L40)).
  - Thinking is collapsible (`collapsible-thinking.tsx`).
  - Live events arrive over a WebSocket (`src/hooks/use-websocket.ts`).
- **Stop.** A conversation's stop button *pauses* the conversation (`usePauseConversation`, [chat-input-actions.tsx](https://github.com/OpenHands/OpenHands/blob/852132d38dbc15e03d6825bbacbeb971c72575be/src/components/features/chat/components/chat-input-actions.tsx)). An automation run is *cancelled*. Two verbs for two kinds of object.

### 2.2 Could components be lifted?

- **Licence: fine.** MIT into MIT needs only the copyright notice kept.
- **Weight: no.**
  - Lifting even one component brings React, HeroUI, Tailwind 4's Vite plugin, i18next and the `#/` path aliases. The release build would then need Node and npm, which thirdshift's `dist` release build doesn't have today ([ADR-0003](../adr/0003-static-binaries-through-github-releases.md)).
  - The full bundle would add about 10 MB to the binary, or roughly 2–3 MB compressed **[unverified: not measured compressed]**.
- **Schema mismatch.** The visualizers take OpenHands `ActionEvent` and `ObservationEvent` types (`#/types/agent-server/core`), not Claude stream-json or Codex `exec --json`. Every component would need an adapter layer first.
- **[interpretation]** Port the *behaviour* (phase plus age, cancel-when-cancellable, grouping, a visualizer registry with fallback) into server-rendered Rust templates instead. Each of those is a few dozen lines of logic.

## 3. OpenAI Symphony

- **Licence.** Apache-2.0, with a NOTICE file ("Copyright 2025 OpenAI") ([LICENSE](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/LICENSE), [NOTICE](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/NOTICE)).
- **What the SPEC says** ([SPEC.md §13](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/SPEC.md)):
  - A status surface is optional. It "SHOULD draw from orchestrator state/metrics only and MUST NOT be REQUIRED for correctness" (§13.4).
  - The HTTP extension "MAY serve server-rendered HTML or a client-side application". "The dashboard/API MUST be observability/control surfaces only" (§13.7).
  - "Implementations SHOULD bind loopback by default" (§13.7).
  - Endpoints "SHOULD be read-only except for operational triggers like `/refresh`". "If the dashboard is a client-side app, it SHOULD consume this API rather than duplicating state logic" (§13.7.2).
  - The API is `GET /api/v1/state`, `GET /api/v1/<issue_identifier>` (per issue: workspace path, attempts, running session, log file paths, recent events, last error) and `POST /api/v1/refresh`, with suggested JSON shapes (§13.7.2).
  - Token accounting should "prefer absolute thread totals" and "ignore delta-style payloads" (§13.5).
- **The reference implementation has two status views.**
  - A **terminal dashboard** ([status_dashboard.ex](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/elixir/lib/symphony_elixir/status_dashboard.ex), 1,941 lines). Its README screenshot shows a header (agents running out of the maximum, throughput, runtime, tokens in, out and total, rate limits, next refresh). Below it is a Running table with columns ID, STAGE, PID, AGE / TURN, TOKENS, SESSION and EVENT, and then a "Backoff queue" ([screenshot](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/.github/media/elixir-screenshot.png)). The EVENT column is a humanized last event, such as "command output streaming: …" or "turn diff updated (152 lines)".
  - A **Phoenix LiveView web page** at `/` ([dashboard_live.ex](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/elixir/lib/symphony_elixir_web/live/dashboard_live.ex), 446 lines, plus a 471-line [dashboard.css](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/elixir/priv/static/dashboard.css)).
    - Metric cards: Running, Retrying, Blocked, Total tokens, Runtime.
    - Then rate limits, a "Running sessions" table (Issue, State, Session, Runtime / turns, Codex update, Tokens) with a copy-session-ID button, a "Blocked sessions" table and a "Retry queue" (L81-L292).
    - It updates by PubSub plus a once-a-second runtime tick (L19-L33). The router serves the Phoenix JS from the app itself, not a CDN ([router.ex](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/elixir/lib/symphony_elixir_web/router.ex)).
- **No Stop.** Neither view nor the API has a stop, cancel or kill action (no matches in `dashboard_live.ex` or the controllers). The only write is `/refresh`.
- **What to take.**
  - The API-first shape, and its JSON shapes, as the model for `thirdshift status --json` (§6).
  - The humanized last-event column (a stalled Run reads as an old event).
  - Code is portable under Apache-2.0 with NOTICE kept, but LiveView is Elixir-specific, so there is little to copy beyond CSS **[interpretation]**.

## 4. Other open-source agent orchestration UIs

All metadata from `gh api repos/<owner>/<repo>` and the LICENSE file, 2026-10-18. "Copyable" means the licence allows copying into an MIT project with attribution.

| Project | Licence | Copyable | UI and stack | What's worth a look |
|---|---|---|---|---|
| **Gas Town** (`gastownhall/gastown`, Go, 18k★) | MIT | Yes | `gt dashboard`: a Go `html/template` page with CSS and JS embedded by `//go:embed` ([handler.go L19](https://github.com/gastownhall/gastown/blob/649b832b7672bc7a2dbef26f5983aba6198b819b/internal/web/handler.go#L19)). Uses htmx 1.9 and idiomorph from unpkg. SSE `dashboard-update` triggers `hx-get="/"` with `hx-swap="morph:outerHTML"`, and it polls every 30 s only when SSE is down ([convoy.html L8-L13](https://github.com/gastownhall/gastown/blob/649b832b7672bc7a2dbef26f5983aba6198b819b/internal/web/templates/convoy.html#L8-L13); [dashboard.js L33-L46](https://github.com/gastownhall/gastown/blob/649b832b7672bc7a2dbef26f5983aba6198b819b/internal/web/static/dashboard.js#L33-L46)) | **Actions are allowlisted CLI commands.** `AllowedCommands` maps `gt` subcommands to `Safe` or `Confirm`. `BlockedPatterns` refuses `--force`, `kill`, `rm`, `reset` and so on, which "require terminal access" ([commands.go L27-L110](https://github.com/gastownhall/gastown/blob/649b832b7672bc7a2dbef26f5983aba6198b819b/internal/web/commands.go#L27-L110)). The page is 58 KB of template, 96 KB of CSS and 133 KB of JS: big, but with no build step |
| **Vibe Kanban** (`BloopAI/vibe-kanban`, Rust + React, 28k★) | Apache-2.0 | Yes, with NOTICE | Axum serves a Vite-built React SPA embedded with `rust-embed` ([frontend.rs L9-L11](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/crates/server/src/routes/frontend.rs#L9-L11)) | "**Vibe Kanban is sunsetting**" ([README](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/README.md)). Its Rust log normalizers are the useful part (§5.2) |
| **agent-of-empires** (`agent-of-empires/agent-of-empires`, Rust, 3.4k★) | MIT | Yes | `aoe serve`: Axum plus a React 19 SPA (Tailwind, shiki, `@pierre/diffs`, assistant-ui) behind a `web` cargo feature. `build.rs` runs `npm ci` and `npm run build`, then embeds `web/dist` with `rust-embed`. It panics if npm is missing ([build.rs L135-L205](https://github.com/agent-of-empires/agent-of-empires/blob/7086f811915af7df9005b952691bc377ba906aa2/build.rs#L135-L205); [Cargo.toml L154, L221](https://github.com/agent-of-empires/agent-of-empires/blob/7086f811915af7df9005b952691bc377ba906aa2/Cargo.toml#L154)) | The closest stack precedent (a Rust CLI with an embedded web dashboard usable on a phone), and a clear picture of the Node-in-the-build cost. Its "structured view" renders sessions over ACP: plan, tool-call cards, diffs ([structured-view.md](https://github.com/agent-of-empires/agent-of-empires/blob/7086f811915af7df9005b952691bc377ba906aa2/docs/structured-view.md)) |
| **agent-orchestrator** (`OrchestratorInc/agent-orchestrator`, Go, 13k★) | Apache-2.0 | Yes, with NOTICE | Desktop app plus a local daemon. "Follow every worker, pull request, CI run, and review in a live Kanban" (README) | Columns by live status, with PR, CI and review on the card. Desktop-only |
| **Cline Kanban** (`cline/kanban`, TS) | Apache-2.0 | Yes | `npx kanban`, "a local web app that runs CLI agents in parallel"; research preview (README) | A Kanban of agent tasks |
| **Paperclip** (`paperclipai/paperclip`, TS, 104k★) | MIT | Yes | "A Node.js server and React UI"; "track work and costs from one dashboard" (README) | Cost tracking per task; heavy |
| **Terragon** (`terragon-labs/terragon-oss`, TS) | Apache-2.0 | Yes | Next.js dashboard. "Task status and agent progress stream to your browser in real-time" (README). Shut down; needs Postgres, Redis and more (earlier survey §4.1) | Screenshots only |
| **Jean** (`coollabsio/jean`) | Apache-2.0 | Yes | Tauri 2 + React 19 + shadcn, plus a browser mode (README) | Desktop-first |
| **Emdash** (`generalaction/emdash`) | Apache-2.0 | Yes | Desktop ADE | Desktop-only |
| **Sculptor** (`imbue-ai/sculptor`) | MIT | Yes | Desktop app, "experimental research preview" (README) | Desktop-only |
| **ccmanager** (`kbwo/ccmanager`) | MIT | Yes | TUI session manager | Terminal, not web |
| **Claude Code UI / CloudCLI** (`siteboon/claudecodeui`) | AGPL-3.0 | **No** | Web and mobile UI for Claude Code, Codex and others | Ideas only |
| **opcode** (`winfunc/opcode`, formerly getAsterisk) | AGPL-3.0 | **No** | GUI for Claude Code | Ideas only |
| **Claude Squad** (`smtg-ai/claude-squad`) | AGPL-3.0 | **No** | TUI | Ideas only |
| **Superset** (`superset-sh/superset`) | ELv2 ([LICENSE.md](https://github.com/superset-sh/superset/blob/main/LICENSE.md)) | **No** (not open source) | Desktop agentic IDE | Avoid |
| **claude_code_agent_farm** (`Dicklesworthstone/...`) | "MIT License (with OpenAI/Anthropic Rider)" | **No** without reading the rider | tmux "real-time dashboard" | Avoid |
| **humanlayer / CodeLayer** | Apache-2.0 (LICENSE text; `gh api` reports NOASSERTION) | Yes | Desktop | Not looked at further |
| **coder/agentapi** | MIT | Yes | HTTP API over agent TUIs | **Archived** |

Conductor is closed source (earlier survey §4.1). `wandb/catnip` returned 404.

## 5. Session-log viewers

### 5.1 First, what thirdshift's Session logs actually are

Checked against files in `~/.thirdshift/logs/<owner>/<repo>/sessions/` on 2026-10-18:

- **Claude Code.** The stdout of `claude -p --output-format stream-json --verbose` ([src/harness/claude.rs L97](../../src/harness/claude.rs)).
  - Top-level `type` values in one Architecture review log: `system` (161, `init` among them), `assistant` (202), `user` (110), `tool_progress` (18), `rate_limit_event` (2) and `result` (1).
  - Assistant content blocks: `thinking` (75), `tool_use` (108) and `text` (19).
  - The `result` line carries `duration_ms`, `num_turns`, `total_cost_usd` and `usage`.
- **Codex.** The stdout of `codex exec --json` ([src/harness/codex.rs L91](../../src/harness/codex.rs)): `thread.started`, `turn.started`, `item.started` and `item.completed` (`command_execution`, `agent_message`), and `turn.completed` with `usage`.
- **Muse.** A different, thirdshift-specific envelope (`schema_version`, `stream`, `payload_type`, …). agy and Grok also write stream-json ([src/harness/agy.rs](../../src/harness/agy.rs), [src/harness/grok.rs](../../src/harness/grok.rs)).
- **Size.** Session logs run to about 5 MB each, and the host holds 696 MB of logs (`du`).
- **What this means for reuse.**
  - Most viewers parse Claude's on-disk transcripts (`~/.claude/projects/*.jsonl`, whose lines carry `uuid`, `parentUuid` and `timestamp`) or Codex *rollout* files (`~/.codex/sessions`). Neither is the stream thirdshift keeps.
  - The Claude `message.content` blocks are the same Anthropic shape in both, so rendering logic carries over but loaders don't **[interpretation]**.
  - Codex `exec --json` and rollouts are different schemas.

### 5.2 Viewers and parsers

| Project | Licence | Language | Reads | Renders | Port or embed? |
|---|---|---|---|---|---|
| **claude-code-log** (`daaain/claude-code-log`, 1.2k★) | MIT | Python (Jinja2, Pygments) | Claude transcripts. Codex rollouts (beta) and agy (alpha) for single sessions ([README](https://github.com/daaain/claude-code-log/blob/9834d25c88dc4dbe8f8ade40ae4b74602ae7669d/README.md); [providers/codex.py](https://github.com/daaain/claude-code-log/blob/9834d25c88dc4dbe8f8ade40ae4b74602ae7669d/claude_code_log/providers/codex.py)) | User and assistant messages, tool use and results, thinking, images, token usage, sub-agents and forks as foldable branches, filter by message type, timeline, search. `serve --watch` makes an open session page grow | The most complete per-tool formatter set: Bash, Edit and MultiEdit diffs, Read, Write, Grep, TodoWrite, Task, WebFetch, AskUserQuestion, ExitPlanMode, Monitor and more ([tool_formatters.py](https://github.com/daaain/claude-code-log/blob/9834d25c88dc4dbe8f8ade40ae4b74602ae7669d/claude_code_log/html/tool_formatters.py)). Port the formatter list to Rust, not the code. Its Codex provider calls the rollout format "an implementation detail of Codex rather than a stable file-format API" |
| **claude-code-transcripts** (`simonw/...`, 1.7k★) | Apache-2.0 | Python (Jinja2) | Claude transcripts | Paginated, mobile-friendly pages. Write, Edit and Bash cards, thinking, tool results, TodoWrite, commit cards linked to GitHub, an index of prompts with tool stats ([macros.html](https://github.com/simonw/claude-code-transcripts/blob/316fd093aca3c9c0ad8aa70092711aee3adacc6e/src/claude_code_transcripts/templates/macros.html), 192 lines) | **Small enough to port**: the macros map almost one-to-one onto Rust templates. Keep the Apache NOTICE and attribution if copied |
| **Vibe Kanban executors** | Apache-2.0 | Rust (serde) | **Claude stream-json** (it launches `claude` with `--output-format=stream-json`, [claude.rs L190](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/crates/executors/src/executors/claude.rs#L190)), and Codex via `app-server` ([codex.rs L453](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/crates/executors/src/executors/codex.rs#L453)), not `exec --json` | A normalized entry model: UserMessage, AssistantMessage, ToolUse with an ActionType (FileRead, FileEdit with a diff, CommandRun with exit status, Search, WebFetch, TodoManagement, …), Thinking, ErrorMessage, TokenUsageInfo ([logs/mod.rs L73-L250](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/crates/executors/src/logs/mod.rs#L73)) | **The one directly reusable parser.** The `ClaudeJson` enum ([claude.rs L2258](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/crates/executors/src/executors/claude.rs#L2258)) deserializes the exact stream thirdshift keeps. The normalized model is a good template for one renderer over every Harness. The files are large (claude.rs is 3,284 lines), so take the types and the mapping idea, not the file |
| **cclogviewer** (`Brads3290/cclogviewer`) | MIT | Go | Claude transcripts | Interactive HTML from one binary. Templates, CSS and about 190 lines of JS embedded with `//go:embed` ([templates.go L13](https://github.com/Brads3290/cclogviewer/blob/951bad64d7f9847981da6d676706a2f38c403e6a/internal/renderer/templates.go#L13)), about 5k lines in total | A stack model rather than code to take; last pushed 2025-08 |
| **claude-code-history-viewer** (`jhlee0409/...`, 2.3k★) | MIT | Rust (Tauri) + React | Claude, Codex rollouts and 30-odd other tools (`src-tauri/src/providers/`) | History, search, analytics, token stats. A headless `cchv-server --serve` mode uses Axum and `rust-embed` behind a `webui-server` feature ([Cargo.toml L19, L67](https://github.com/jhlee0409/claude-code-history-viewer/blob/7f48907081d93ad209b30e10ff640ea0b8ed0e24/src-tauri/Cargo.toml#L19)) | Another embedded-SPA precedent. Its Codex provider (3,918 lines) reads rollouts |
| **ccusage** (`ccusage/ccusage`, 19k★) | MIT (LICENSE; `gh api` reports NOASSERTION) | Rust (formerly TypeScript) | Claude, Codex and about 18 other CLIs' local data | Token and cost reports only: daily, weekly, monthly, session ([README](https://github.com/ccusage/ccusage/blob/b0904280a2c45972e594f01a44a5de154522efec/README.md)) | No rendering. A reference for the later charts. Its Codex adapter handles replayed and forked history and dedupe ([adapters/codex/README.md](https://github.com/ccusage/ccusage/blob/b0904280a2c45972e594f01a44a5de154522efec/rust/adapters/codex/README.md)) |
| **claude-run** (`nilbuild/claude-run`) | MIT | TS (Hono + React) | Claude transcripts | History browser | React; ideas only |
| **claude-code-viewer** (`d-kimuson/...`) | MIT | TS | Claude | A full web client (it sends messages, which is now opt-in because of Anthropic's terms, per its README) | Beyond scope |
| **yepanywhere** (`kzahel/yepanywhere`) | MIT | TS | Claude and Codex, including `codex exec --json` (test fixtures `packages/server/test/fixtures/codex/*.jsonl`) | A self-hosted web UI that drives sessions | Large; its Codex fixtures are useful test data for thirdshift's `exec --json` renderer |
| **raine/claude-history** | MIT | Rust | Claude transcripts | Fuzzy-search TUI (from the description; not read) | No |

### 5.3 Lessons from claude-code-log's live mode

- **Render the append, not the file.** On a 53 MB session, a one-line append took 15–27 s per tick, "against a documented target of 0.26 s". The causes were "three separate costs, each O(entries in the file), none O(append)" ([watch-live-sessions.md L95-L137](https://github.com/daaain/claude-code-log/blob/9834d25c88dc4dbe8f8ade40ae4b74602ae7669d/work/watch-live-sessions.md#L95-L137)).
  - For thirdshift: tail a growing Session log by byte offset, render only the new lines, and append them to the page.
- **Watch only what's on screen.** Its live registry follows what is on screen: "the page polls every 1 s and stops when hidden" (same file, L208-L211).
  - For thirdshift: tail only the Session logs someone is viewing.

## 6. Recommendation

### 6.1 Take (code, licence-compatible)

- **Vendored JS, 0BSD.** Copy these into the repo and embed them; no CDN at runtime.
  - htmx 2.0.7: 51 KB minified, 16 KB gzipped, published 2025-09-08.
  - `htmx-ext-sse` 2.2.3: 2.8 KB minified, published 2025-02-13.
  - idiomorph 0.7.4: 9.7 KB minified, 3.4 KB gzipped, published 2025-09-29.
  - All from `npm pack`. 0BSD needs no attribution.
- **Optional:** diff2html 3.4.52 (MIT): 80 KB minified, 20 KB gzipped, plus a 17 KB CSS file. Use it only if Edit diffs need side-by-side rendering. A unified diff in `<pre>` with line classes covers v1 **[interpretation]**.
- **Claude stream-json types.** Base them on Vibe Kanban's `ClaudeJson` (Apache-2.0). Keep the attribution and NOTICE in a header comment, or rewrite from the observed shapes in §5.1. Either way, test against real Session logs.
- **Small templates.** Port simonw's macro set (Apache-2.0, with attribution) as Rust templates for message, tool-use, tool-result, thinking and commit cards.

### 6.2 Borrow (ideas, no code)

| Idea | From | For the console |
|---|---|---|
| The page reads a JSON API, and the CLI serves the same data | Symphony SPEC §13.7.2 | Build `thirdshift status --json` first. The page renders what it returns. "Nothing the CLI can't do" then holds by construction |
| Actions are an allowlist of CLI commands, with Safe or Confirm | Gas Town `commands.go` | Stop is `thirdshift stop <command>` run by the server. No other write path. Later "start runs" adds allowlisted `thirdshift <issue>` and `thirdshift pickup` |
| Stop only while cancellable; children not cancelled implicitly | Warp, OpenHands | Show Stop only when the live record's process is alive. Decide whether stopping a Spec run stops its Ticket Runs. Today SIGINT, SIGTERM and SIGHUP are recorded as an interrupt and the Command ends interrupted ([src/interrupt.rs](../../src/interrupt.rs)), so Stop is SIGTERM to the Command's process **[interpretation]** |
| Phase plus time in phase | OpenHands `run-phase.tsx` | "review · 4 min" vs "review · 52 min": the stage column carries its age, so stalls show without logs |
| Derived status that fixes stale "running" | Warp spec APP-4382 | "Died at review" when the process is gone (#711 Q3) |
| Children nested under the parent, parent status separate | Warp Runs page | Pickup run → Spec run → Ticket Runs and Base fix |
| Humanized last event | Symphony EVENT column | One-line latest tool call per live Run (`Bash: cargo test`, `Edit src/ci.rs`) |
| Fold consecutive tool calls; collapsible thinking | OpenHands, claude-code-log | Session-log view |
| Per-tool renderer registry with a generic JSON fallback | OpenHands visualizers, claude-code-log formatters | One registry keyed on tool name, shared by Claude and normalized Codex items |
| SSE nudge plus morph re-render, polling fallback | Gas Town | The Commands list. Session-log tails use SSE appends instead (§5.3) |
| Loopback bind by default | Symphony SPEC §13.7 | `thirdshift serve` binds 127.0.0.1. Caddy fronts it |
| Cost and usage per Run | Warp Dashboard, OpenHands run cost, ccusage | Later charts, from the `result` and `turn.completed` usage in the Session logs |

### 6.3 Avoid

- **Copying from AGPL-3.0 code**: Warp's client including the Agent Management Panel, Claude Squad, opcode, claudecodeui. **ELv2** (Superset). The **rider-modified MIT** of claude_code_agent_farm.
- **Lifting the OpenHands Canvas UI, or any React SPA** (Vibe Kanban, agent-of-empires, CCHV). The licences are fine. The cost is Node and npm in every release build (agent-of-empires' `build.rs` shows what that takes), a 1–10 MB bundle, and adapters from each project's own event schema.
- **Loading scripts from a CDN at runtime**, as Gas Town does with unpkg. The console should work with no internet and leak nothing. Vendor the files.
- **Re-rendering whole Session logs per update** (§5.3).

### 6.4 Stack that fits a page embedded in the Rust binary

- **Server.** Rust renders HTML. thirdshift already has tokio and hyper in its lockfile through reqwest (`Cargo.lock`), so a small hyper or axum server adds little. A sync server like `tiny_http` (Apache-2.0, last pushed 2024-08) is the lighter alternative. Choosing between them is for the design **[interpretation]**.
- **Embedding.** Use `include_dir`, already a dependency for the Factory skills ([Cargo.toml](../../Cargo.toml) line 23; [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md)). `rust-embed`, as Vibe Kanban, agent-of-empires and CCHV use it, would be a second crate doing the same job.
- **Client.**
  - htmx, `htmx-ext-sse` and idiomorph, about 20 KB gzipped together.
  - One hand-written ES module for the few interactions htmx doesn't cover: copy, collapse state, follow-tail.
  - This matches the site's existing "plain HTML + CSS + one vanilla ES module, no build step" choice ([site-stack-for-communication.md](site-stack-for-communication.md) TL;DR).
- **Live updates.** One SSE stream for "Commands changed", which triggers an htmx morph of the list. One SSE stream per open Session log that sends rendered HTML for new lines, reading from a byte offset.
- **Why not a prebuilt SPA bundle via `include_dir`.**
  - It would work technically.
  - But it brings a JS toolchain into the repo, a bundle to rebuild whenever markup changes, and client-side state logic that Symphony's SPEC warns against duplicating.
  - Every comparable dashboard that stays small (Gas Town, cclogviewer, Symphony's LiveView) renders on the server.

## 7. Uncertainties

- **Warp's by-stage view.** The docs list "Work items by stage" on the dashboard, but no screenshot or text shows its layout **[unverified]**.
- **OpenHands bundle when compressed.** The 10.3 MB is uncompressed. Compressed size and what Monaco loads at runtime were not measured **[unverified]**.
- **Stop semantics for nested Commands.** Whether stopping a Spec run should stop its running Ticket Runs is a design question. Warp deliberately doesn't cascade cancels. thirdshift's Ticket Runs and Base fixes are child `thirdshift` processes of the Command ([src/child_run.rs](../../src/child_run.rs)), so a signal to the parent could cascade **[interpretation]**. Also, the crate publishes only `/src`, `/skills`, `README.md` and `LICENSE` ([Cargo.toml](../../Cargo.toml) `include`), so embedded page assets must live under one of those paths.
- **Vibe Kanban's sunset.** It may stop being maintained. Take types, not a dependency.
- **Codex `exec --json` schema stability.** Codex documents neither the rollout format nor this stream as stable; claude-code-log says so of rollouts. Renderers should ignore unknown item types.
- **Licence metadata.** `gh api` reports NOASSERTION for ccusage, humanlayer and claude_code_agent_farm. The LICENSE files were read directly. ccusage and humanlayer are plain MIT and Apache-2.0. The agent farm adds a rider.
