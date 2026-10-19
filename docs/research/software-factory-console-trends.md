# Trends in software-factory consoles: how 2025–2026 agent products show many agents at once

Research date: 2026-10-18, for issue #711 (the Console) and its Q27 UI prototypes in `.scratch/prototypes/711-console/`. It builds on [console-run-monitoring-ui.md](console-run-monitoring-ui.md), which covers CI, orchestrators, trace UIs and the SRE material. This note covers products that run many coding agents unattended.

**Question.** Products that run many autonomous coding agents in parallel on issues and deliver pull requests now ship their own observation surfaces: mission controls, agent panels, agent views, task boards. In 2025–2026, how do they show many agents at once? How do they show what needs a human, hand work over for review, let you steer or stop it, show logs and transcripts, and report cost? Which trends fit thirdshift's Console? Its constraints: one machine; **Commands**, **Runs**, **Spec runs**, **Tickets** and **Passes**; the page does nothing the CLI can't; phone first; no Start buttons, charts or cross-machine view for now.

**Method.**

- Read each vendor's own docs, changelogs and launch posts, and READMEs for open-source tools, cited inline. A claim that rests only on a secondary source, or that no primary page confirmed, is marked **[unverified]**.
- **Vendors:**
  - GitHub (Agent HQ, mission control, the Copilot agents panel and page);
  - OpenAI (the Codex app, its review pane and automations inbox);
  - Cursor (the Agents Window, 3.0 and 3.1);
  - Google Jules;
  - Devin (session status, Session Insights);
  - Claude Code (cloud sessions, agent view, Projects);
  - Amp (the Agents Panel);
  - Linear (agent sessions, delegation, agent plans);
  - Factory (Desktop app, Missions);
  - Warp (Agent Management Panel, notifications);
  - Replit (Agent task board).
- **Open source:** Vibe Kanban, Claude Squad, Conductor and Sculptor.
- Mapped each trend onto [CONTEXT.md](../../CONTEXT.md) vocabulary and onto the existing variants: `garden`, `plain`, `accent`, `press`, `pressplain`, `pixel`, `briefing`, `briefingwide`, `trace`, `grid`, `lanes`.

**Out of scope.**

- **Pricing, model quality and vendor comparison:** only how each product's observation UI is structured is covered.
- **Anything that starts work:** dispatch boxes, plan approval and auto-approve, which thirdshift's Console leaves to the CLI and the scheduler.
- **Two products:** Terragon (no primary docs found) and Sourcegraph's own Amp page (it only points to the manual).

## TL;DR

- **The common shape is a list grouped by attention state, not by process state.** Claude Code's agent view groups sessions as Pinned, Ready for review, Needs input, Working and Completed ([agent view](https://code.claude.com/docs/en/agent-view)). Its Projects Overview has Ready for review, Waiting on you, Working, Landing, Idle and Resolved ([Projects](https://code.claude.com/docs/en/claude-projects)). Warp, Devin, Linear and Replit all carry a "blocked on you" state alongside working, failed and done. thirdshift's "live first, then history" order is still a process view. A grouping by what the **Day shift** must do would match the field.
- **Each row is one line: what it's doing now, or the question it's asking.** You "peek" without opening it. Agent view rows show a one-line summary of current activity or the question, and `Space` opens a peek of the latest output ([agent view](https://code.claude.com/docs/en/agent-view)). Amp's `Space` expands a thread's last message ([Amp Agents Panel](https://ampcode.com/news/agents-panel)).
- **Review is the destination, and the row carries its size.** Claude Code shows `+42 -18` on each session ([cloud sessions](https://code.claude.com/docs/en/claude-code-on-the-web)). Codex's review pane and inbox, Conductor's review → PR → merge → archive, and Replit's Ready column all end at a diff. thirdshift can show the diffstat, checks and PR link; reviewing happens on GitHub.
- **Coordinator and workers, with milestones and checklists as the unit of progress.**
  - Claude Code Projects: a coordinator conversation whose threads "report back".
  - Factory Missions: an orchestrator, workers, milestones and validation criteria.
  - Linear: agent plans with pending, inProgress, completed and canceled steps.
  - Warp: parent and child runs.

  A **Pickup run** → **Spec run** → **Ticket** Runs is the same shape, and the **Tickets checklist** is thirdshift's plan.
- **Attention leaves the page.** Agent view puts "2 awaiting input" in the terminal tab title. Warp has toasts, a mailbox and an attention badge. The Projects Overview button shows a dot. The Codex inbox has unread indicators. A page title count and a favicon dot are read-only and fit; browser notifications are a later question.
- **Spend and session size sit on the row or the overview.** Copilot shows token usage and session length, Factory shows credits in the top bar, and Devin rates sessions XS–XL and flags L and XL as unhealthy ([Session Insights](https://docs.devin.ai/product-guides/session-insights)).
- **Steering mid-run is everywhere, but it's a write the thirdshift CLI doesn't have.** Copilot, Claude Code, Jules and Factory all let you steer a running session. It's ruled out for now; Stop stays the only write, with its CLI hint.
- **New variant:** `inbox`, a keyboard-first, attention-grouped list with a peek pane and tiled live logs. Its brief is in [§5](#5-a-new-variant-inbox).

## 1. Vendors

### 1.1 GitHub: Agent HQ, mission control, the agents panel

- **Mission control:** announced as "a consistent interface across GitHub, VS Code, mobile, and the CLI" to direct, monitor and manage agent tasks. You pick agents, "assign work in parallel" and follow progress from any device. Third-party agents (Anthropic, OpenAI, Google, Cognition, xAI) arrive in the same place ([Agent HQ, 2025-10-28](https://github.blog/news-insights/company-news/welcome-home-agents/)).
- **The how-to post:** mission control is "a unified interface for managing GitHub Copilot coding agent tasks". It replaces "bouncing between pages", and "session logs show reasoning, not just actions". You can stop a drifting agent and restart it with refined instructions ([mission control, 2025-12-01](https://github.blog/ai-and-ml/github-copilot/how-to-orchestrate-agents-using-mission-control/)).
- **The agents panel:** available "from any page on GitHub". It lists your sessions and those others started; repositories have an "All sessions" tab.
  - A session opens a log and overview with "progress, token usage, and session length".
  - Steering is a prompt box under the log, applied "after it finishes its current tool call", and each steer costs credits.
  - **Stop session** ends the run and keeps pushed commits. Sessions can be archived but not deleted.
  - ([manage and track agents](https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/manage-and-track-agents), [track sessions](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/track-copilot-sessions))
- **Not documented:** no layout or status names for the list **[unverified]**.

### 1.2 OpenAI: the Codex app

- **Launch:** OpenAI framed the app as a command center for overseeing many agents at once, each in its own thread grouped by project, with worktrees isolating changes ([Introducing the Codex app](https://openai.com/index/introducing-the-codex-app/); the page returned 403 to the fetcher, so details are **[unverified]** beyond search snippets).
- **Review pane:** "reflects the state of your Git repository, not just what Codex edited".
  - Scopes: Unstaged, Staged, Commit, Branch, Last turn.
  - You can stage or revert per file and per hunk, and leave inline comments the agent then addresses.
  - The PR sidebar has an inbox (Assigned to me / my team / Authored by me) and Summary and Changes panes, with **Fix** attaching failing checks to a chat ([Codex code review](https://learn.chatgpt.com/docs/code-review?surface=app)).
- **Automations:** "The Scheduled view acts as your inbox", and "an unread indicator shows when a run needs your attention" ([Codex automations](https://learn.chatgpt.com/docs/automations?surface=app)).

### 1.3 Cursor: the Agents Window

- **Agents Window:** the 3.0 release (2026-04-02) made it the headline feature, to "run many agents in parallel across repos and environments": local, worktree, cloud and SSH. Chats can be shown "side-by-side or in a grid" ([Cursor 3.0](https://cursor.com/changelog/3-0)).
- **Tiled panes:** 3.1 (2026-04-13) split the view into tiled panes to manage several agents at once ([Cursor 3.1](https://cursor.com/ko/changelog/3-1)).
- **Help page:** you manage every agent from the sidebar. Agents are reachable "from the web, mobile, Slack, GitHub, and Linear", and artifacts like screenshots and demo videos help review "so you don't have to read every line" ([multi-agent help](https://cursor.com/help/ai-features/multi-agent)).

### 1.4 Google Jules

- **Plan before code:** "Once the plan is approved, Jules will start coding."
- **Activity feed:** it updates per step with "inline explanations of each change" and "a mini diff preview for each file". **Pause** waits for new instructions ([running tasks](https://jules.google/docs/running-tasks/)).
- **Notifications:** for "when a plan is ready or a task completes" ([FAQ](https://jules.google/docs/faq/)).
- **Planning Critic:** a second agent reviews plans that are auto-approved, credited with "a 9.5% reduction in task failure rates" ([changelog, 2026-01-26](https://jules.google/docs/changelog/2026-01-26-1/)).

### 1.5 Devin

- **Session status:**
  - `running` carries a `status_detail` of `working`, `waiting_for_user`, `waiting_for_approval` or `finished`.
  - `suspended` carries a reason, such as `inactivity`, `usage_limit_exceeded` or `out_of_credits`.
  - Sessions also list `pull_requests` with their state, `acus_consumed` and `child_session_ids` ([session API](https://docs.devin.ai/api-reference/v3/sessions/get-organizations-session.md)).
- **Session Insights:** after the fact, an Issue Timeline groups problems "by label and impact level". It also gives actionable feedback, including a rewritten prompt, and a size class XS–XL by ACUs, with L and XL "flagged as unhealthy" ([Session Insights](https://docs.devin.ai/product-guides/session-insights)).

### 1.6 Claude Code: cloud sessions, agent view, Projects

- **Cloud sessions:** they sit in the claude.ai/code sidebar. Each shows "a diff indicator with lines added and removed, like `+42 -18`", which opens a diff view with inline comments.
  - Follow-ups can be queued from any CLI with `claude -p "…" --cloud <id>`, and Auto-fix watches a PR's CI and review comments.
  - Sessions are watched "from anywhere including the Claude mobile app" ([cloud sessions](https://code.claude.com/docs/en/claude-code-on-the-web)).
- **Agent view (`claude agents`, research preview):** "a grouped list, not a grid".
  - **Groups:** Pinned, **Ready for review**, **Needs input**, **Working**, **Completed**.
  - **Rows:** "a one-line summary of its current activity or question", plus its age. "Sessions that need you rise to the top", and the tab title shows "`2 awaiting input`".
  - **Peek:** `Space` shows the latest output or the exact question, plus linked PRs, and you can reply in place.
  - **Notifications:** sent when a session "needs input, finishes, or fails" ([agent view](https://code.claude.com/docs/en/agent-view)).
- **Projects (public beta):** a coordinator conversation starts parallel threads that "report back".
  - The **Overview** pane groups threads as **Ready for review**, **Waiting on you** (including failed), **Working**, **Landing** (approved or queued to merge), **Idle** and **Resolved** (auto after "a week with no activity").
  - The Overview button "shows a dot when a thread is waiting on you" ([Projects](https://code.claude.com/docs/en/claude-projects)).
- **Overview of parallel work:** compares subagents, agent view, agent teams, workflows and projects. `/tasks` and `/workflows` list running work and its phase ([Run agents in parallel](https://code.claude.com/docs/en/agents)).

### 1.7 Amp

- **Agents Panel (2026-01-09):** lists all active threads, with `j`/`k` to move, `Space` to expand "its last message or tool result", and `e` to archive, so the panel "shows your current working set" ([Agents Panel](https://ampcode.com/news/agents-panel)).
- **Agent-to-agent:** agents spawn agents that report back to the main thread ([agent-to-agent](https://ampcode.com/docs/orbs/agent-to-agent)).

### 1.8 Linear

- **Delegation, not assignment:** assigning an issue to an agent makes it the **delegate**, "so humans maintain ownership while agents act on their behalf" ([agent interaction](https://linear.app/developers/agent-interaction)).
- **Session states:** `pending`, `active`, `error`, `awaitingInput`, `complete` and `stale`. They are derived "based on the last emitted activity", so state comes from the activity stream, not from the agent's say-so.
- **Activity types:** thought, action, elicitation, response and error.
- **Agent plans:** "a session-level checklist of tasks", each step pending, inProgress, completed or canceled, in technology preview (same page).
- **Delegate filter:** delegated issues stay in **My issues** and can be filtered by **Delegate** ([agents in Linear](https://linear.app/docs/agents-in-linear)).

### 1.9 Factory

- **Desktop app (2026-04-08):** "each session lives in the sidebar with its own context, progress, and history". Droid Computers get "status, logs, configuration, in one place" ([Factory Desktop](https://factory.com/news/factory-desktop)).
- **Missions:**
  - **Top bar:** elapsed time and credits used.
  - **Left sidebar:** workers.
  - **Main view:** the active milestone, with each feature's validation criteria and commits.
  - **Right sidebar:** a live progress log of "every worker and milestone" ([running in the app](https://docs.factory.com/missions/running-app.md)).
  - **Human role:** "Missions are not fire-and-forget". You pause the orchestrator and re-plan, and a warning appears when a feature hits its retry limit ([Missions](https://docs.factory.com/missions/overview)).
- **Framing:** the "software factory" phrase isn't on these pages; Factory frames it as Droids across "every part of your software business".

### 1.10 Warp

- **Agent Management Panel and Oz Runs page (works on mobile):**
  - **Statuses:** **Working**, **Blocked** ("waiting on user input or a required step"), Canceled, Failed and Success.
  - **Filters:** by source (interactive, API, CLI, Slack or Linear, scheduled), day, creator and status.
  - **Cloud runs:** show their duration.
  - **Orchestrated runs:** appear as parent and child runs ([managing agents](https://docs.warp.dev/agents/using-agents/managing-agents)).
- **Notifications:** three kinds, **Complete**, **Request** ("blocked and needs your input") and **Error**. They arrive as toasts, at most two at once, and in a mailbox with All tabs, Unread and Errors filters. Tabs carry "an attention badge" ([agent notifications](https://docs.warp.dev/agents/capabilities/agent-notifications/)).

### 1.11 Replit

- **Agent task board columns:**
  - **Drafts:** not started.
  - **Active:** including Queued, when a task depends on another or hits the limit.
  - **Ready:** waiting for your review.
  - **Done:** applied, archived or cancelled.
- **Cards:** a status label with time, such as "Active • 12m".
- **Isolation:** each task works in its own copy, and "nothing changes in your main version until you apply the work" ([task board](https://docs.replit.com/references/agent/task-board)).

### 1.12 Open-source orchestrators

- **Vibe Kanban:** issues on "a kanban board". Each workspace gets "a branch, a terminal, and a dev server", and inline diff comments go back to the agent. The README now says the project is sunsetting ([BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban)).
- **Claude Squad:** one terminal lists instances. `tab` switches preview and diff; `c` commits and pauses, `r` resumes and `D` kills ([smtg-ai/claude-squad](https://github.com/smtg-ai/claude-squad)).
- **Conductor:** one workspace per task, each its own branch and worktree. You "archive, merge, or discard each result independently" ([Conductor parallel agents](https://www.conductor.build/docs/core/parallel-agents)). The sidebar's status model isn't documented **[unverified]**.
- **Sculptor:** "Every active agent is visible in Sculptor". Each agent has its own diff, branch and terminal, with Pairing Mode to sync one into your IDE ([Sculptor](https://www.imbue.com/product/sculptor), [docs](https://docs.imbue.com)).

## 2. Trends across tools

| # | Trend | Shown by | Why it matters |
|---|---|---|---|
| T1 | **Group by attention state.** Ready for review, needs you, working, landing, done. | Claude Code agent view and Projects; Warp; Devin `waiting_for_user`; Linear `awaitingInput`; Replit Ready | Many agents only stay manageable when the list sorts itself by what the human must do next. |
| T2 | **One-line now-line plus peek.** Each row says what it is doing or asking, and a peek shows the latest output without opening it. | Agent view summary and peek; Amp `Space`; Linear ephemeral activities | Reading many rows beats opening many transcripts. |
| T3 | **Review is the destination, sized on the row.** Diffstat, checks, PR state; review panes; a Ready column. | Claude Code `+42 -18`; Codex review pane and inbox; Conductor; Replit; Vibe Kanban | The human's job is review, so the row says how big the review is. |
| T4 | **Coordinator, workers and a plan.** Parent/child sessions, milestones, validation criteria, checklists. | Claude Code Projects; Factory Missions; Linear agent plans; Warp parent/child; Devin `child_session_ids`; Jules plans | Progress reads as steps done out of steps planned, not as log volume. |
| T5 | **State derived from activity.** "Stale" or "inactivity" comes from the event stream. | Linear (state from last activity; `stale`); Devin `suspended: inactivity`; agent view idle sessions | It agrees with [console-run-monitoring-ui.md](console-run-monitoring-ui.md) §2 and the `health()` rule: silence is the stall. |
| T6 | **Attention leaves the page.** Tab-title counts, dots, badges, toasts, mailbox, OS notifications. | Agent view tab title; Projects dot; Warp toasts, mailbox and badge; Codex unread; Jules notifications | Nobody watches the page; the page has to call. |
| T7 | **Spend and size per session.** Tokens, session length, credits, XS–XL. | Copilot overview; Factory top bar; Devin ACUs and size class; Warp usage | Cost and "this was too big" are review signals. |
| T8 | **Keyboard and tiles for the desk; the same list on the phone.** `j`/`k`, `Space`, `Enter`; side-by-side tiles; mobile surfaces. | Agent view; Amp; Claude Squad; Cursor 3.0/3.1 grid and tiles; mission control "any device"; Warp Runs on mobile | Power users scan with keys and watch several live logs at once; phones get the list. |
| T9 | **Steer mid-run.** Follow-up box, pause, refine, restart. | Copilot; Claude Code; Jules pause; Factory pause and re-plan; Claude Squad pause and resume | Saves a failed Run, but needs a write path. |
| T10 | **Working set by archiving or resolving.** | Amp `e`; Copilot archive; Projects auto-Resolved after a week; Conductor archive | Keeps "now" short. |

## 3. Fit with thirdshift's rules

| Trend | Fits? | How, or why not |
|---|---|---|
| T1 attention groups | **Yes** | Read-only regrouping of the same Commands. Map: **Needs you** = died, silent, failed, `needs-triage`, Security finding waiting (`needsYou()`); **Ready for review** = outcome `ready` with an open PR; **Working** = live and healthy; **Landing** = a live **Merge run** at CI watch or **Self-merge**; **Done** = merged, skipped, plus history. The settled "live first" order survives inside Working. |
| T2 now-line + peek | **Yes** | The now-line is the latest event (or the question, cause or silence); the peek shows the last few Session log entries via `mountLog`. A reply box doesn't fit (T9). |
| T3 review destination | **Partly** | Show diffstat (`summaryText`), checks and a PR link. Reviewing, staging, commenting and merging stay on GitHub, because the CLI can't do them. |
| T4 coordinator/workers/plan | **Yes** | Pickup run → Spec run → Ticket Runs is the hierarchy. Show a Spec run's Tickets as a plan checklist (merged / running / waiting on / not started), which is the **Tickets checklist** the Spec PR already carries. |
| T5 state from activity | **Yes** | It's already the `health()` rule (silence ≥ `SILENT_MIN` = possibly stuck). |
| T6 attention leaves the page | **Yes, read-only** | Put the count in the page title ("2 need you · thirdshift Console") and a dot on the favicon. OS or browser notifications need a permission prompt and are better left to the existing **Run notification** email for now. |
| T7 spend and size | **Yes** | `usageText()` per session and Command. A size class could come from session length against `usual()`. Text only, no charts. |
| T8 keyboard, tiles, phone | **Yes** | Keys and tiles are view-only. On the phone it's the same list with the peek as a sheet. |
| T9 steer | **No, for now** | There is no `thirdshift` command to message or pause a running session. Stop stays the only write: `thirdshift stop <issue>` (shown as a hint; requeue is the prototype's invented `--requeue`). A future `thirdshift resume`/`steer` would be a CLI decision first. |
| T10 archive/resolve | **Partly** | Manual archive is a write, so it's ruled out. An automatic "Resolved" fold works, as in Projects: ended Commands with nothing waiting fold away after a day. |

## 4. What this adds to the earlier note

[console-run-monitoring-ui.md](console-run-monitoring-ui.md) argued for a "needs you" strip, silence as the stall rule, usual ranges, at most two fold levels, and what Stop keeps. Agent products agree on all of that. They add four things:

- **Group the whole page by attention state** (T1), not just a strip above a process-ordered body.
- **The row's now-line is the question when there is one** (T2): for thirdshift, the cause, the silence, or what a Ticket waits on.
- **A "Landing" state** between Ready for review and Done (T1, Projects): a Merge run that is green and merging.
- **Attention outside the page** (T6) via the title and favicon.

## 5. A new variant: `inbox`

**Idea.** An agent-view-style console: one keyboard-first list grouped by what the Day shift must do, a peek pane that shows the newest lines without opening anything, and tiles that show several live Session logs side by side. It sits closest to `accent` (list and detail) and `briefing` (needs first). It differs from both in three ways: the groups are attention states rather than live and history; the detail is a light *peek* (now-line, last entries, PR, Stop) with the full log one key away; and live logs can tile.

**Sections, top to bottom.** Each section has an `h2`, so it folds like the rest:

1. **Header:** host name, the stephens Console link, sign out. A small key legend (`j k` move · `space` peek · `enter` open · `t` tile).
2. **Needs you** (from `needsYou()`), urgent first. The now-line is the problem in words: "no output for 14 min at CI watch", "died at implement 2 h 51 min ago", the `cause`, or "low Security finding waits".
3. **Ready for review:** Commands with outcome `ready`. The now-line is "PR chart35#227 · +112 −15 in 7 files · 2 Unaddressed findings" (`summaryText`).
4. **Landing:** live Merge runs at CI watch or Self-merge. The now-line is the CI state, with `heartbeat()`.
5. **Working:** live and healthy Runs, live first. The now-line is the latest event, then stage · age, `usualText`, `heartbeat`. A Spec run shows its Tickets as a plan checklist under the row (✓ merged, ▸ running with stage · age, ⏸ waiting on #239, ○ not started). Each Ticket is a row of its own, so two levels at most.
6. **Done:** merged, interrupted and skipped Commands from the last day, one line each with `summaryText`. Older ones fold into **Resolved**, closed by default.

**Rows.** Each row has:
- a state glyph and a state word;
- the kind, then the issue and title (`chart35#239 Queue edits made while offline`);
- the now-line, kept to one line;
- the age on the right;
- for live Commands, a quiet **Stop**, shown only when `canStop`.

`data-latest` stays on the now-line of the running Run, so it updates live.

**Peek pane.** Desktop puts it on the right, about 40%; on a phone it is a bottom sheet over the list. `Space`, a tap or a click opens it on the selected row. It shows:
- the row's title and state;
- health in words (`health()`), stage · age with `usualText`, and `heartbeat`;
- `usageText` for the row's sessions;
- the PR and `summaryText`;
- the cause, for a failure;
- the **last 8 entries** of the open Session log (a compact `mountLog` with folds closed), plus links to the full Session logs and the Command log;
- the Stop confirm with `stopEffectsHTML(c)`, the needs-triage sentence, "Requeue instead", Cancel, and `stopHint` and `requeueHint`.

`Enter` opens the full log in the pane.

**Tiles.** `t` pins the selected live Run's Session log as a tile. Up to four tiles sit in a 2 × 2 grid under the list on wide screens, with each tile's heartbeat in its bar, as in Cursor's tiled Agents Window. Tiles don't show on phones.

**Attention outside the page.** The document title becomes "‹n› need you · thirdshift Console", and the favicon gets a dot while `needsYou()` has urgent items, as agent view does in the tab title.

**Tone and look.** Quiet and dense, close to a terminal UI but in the browser: a monospace now-line, a sans-serif title, and one accent per state with a word beside every colour. Light and dark.

**Keyboard.** `j`/`k` or arrows move, `space` peeks, `enter` opens, `t` tiles, `esc` closes the peek, and `g` then `n`/`r`/`w` jumps to Needs you, Ready or Working. Keys never fire inside the search box or the Stop confirm. The prototype bar's ← and → keep cycling variants, so the inbox uses `j`/`k`, not the arrows.

**Phone (390 px).** One column: the groups in order, rows of two lines (title; now-line and age), and a bottom sheet for the peek. No tiles and no page-level horizontal scroll.

**Shared helpers.** `needsYou`, `health`, `usualText`, `heartbeat`, `usageText`/`sessionsOf`, `summaryText`, `canStop`, `status` (for "stopping…"), `stopEffectsHTML`, `stopHint`/`requeueHint`, `mountLog`, `commandLogText`, `openSession`/`openCommandLog`, and `data-fold`/`data-fold-default="open"` for the plan checklist and Resolved.

## 6. Small revisions the trends support

- **All variants, in `app.js`:** put the "‹n› need you" count in the document title and a dot on the favicon (T6). It's read-only and works whatever the variant.
- **All variants:** add a **Ready for review** group or strip for `ready` Commands with diffstat (T3). Today they only appear in history, and `briefing` alone lists them.
- **`lanes`, `press`, `pressplain`, `accent`:** mark a live Merge run at CI watch or Self-merge as **Landing**, not just "live" (T1).
- **`plain`, `accent`, `garden`:** show a Spec run's Tickets as a plan checklist with ✓ / ▸ / ⏸ / ○ and words, as Linear agent plans and Factory milestones do (T4).
- **History in every variant:** fold Commands that ended more than a day ago, with nothing waiting, into a closed **Resolved** group (T10), rather than adding an archive action.
