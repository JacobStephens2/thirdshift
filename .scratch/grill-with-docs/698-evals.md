# Grilling: #698, evals

Issue: https://github.com/JacobStephens2/thirdshift/issues/698
Research: `docs/research/eval-tools.md`

## Round 1 (open)

**Q1 - What #698's evals are for.** "Evals" covers three different jobs, and tools lean toward different ones. Langfuse is built around traces and scoring them, (b) and (c). For (a) it can store and compare runs. The part that rebuilds a past commit without its later fix and drives the CLIs on your logins would still be thirdshift's: the research found no tool that does it without containers. The options:
- (a) **Replays before a change.** A standing set of past situations with a known right answer. You re-run it on demand before changing a Factory skill (such as #321's update), a Session prompt, or a Harness, Model or Effort default, and compare the result with the current setup against a bar set in advance. #429 and #549 did this by hand, each with a throwaway script. #429's graded material would seed the set: 9 bugs with written catch criteria, the plans its arms stopped, and the Spec findings graded real or wrong. Autoresearch (#348) would need something like it to measure against.
- (b) **Run scores.** What each Run already tells us, recorded when it ends: outcome, Repairs and CI rounds used, Unaddressed findings, minutes and cost-equivalent. Later, what became of its pull request: reverted, or a bug traced to it. Shown as trends by Harness, Model, Effort and kind of session.
- (c) **A trace viewer.** Session logs browsable in a UI: each session's tool calls, tokens and timings, across Runs. Any viewer needs an importer that thirdshift writes, since no tool reads its Session logs as they are.

*Recommended:* (a) first, seeded from #429. Then (b), limited to what thirdshift knows when a Run ends. Leave (c) to #700 and #711, unless the tool chosen for (a) shows traces anyway. (a) answers the decisions you keep making, and twice now you've paid for it with a one-off script. (b) is cheap but blunt: 90% of commands since 3 October ended in a Self-merge, so its signals measure friction more than quality. (c) shows sessions but grades nothing.

*Answer:* agree. I generally want a way to measure if changes to the agents make them better or worse. I want to be able to know this before putting the changes into production, and then I want to analyze the traces of production to understand how well things are running largely in terms of accuracy and quality, though I'm somewhat concerned about efficiency too and other good things for evals to consider.

**Q2 - Where eval data may live.** Evals keep Session logs, or traces made from them, and their grades. A Session log holds the repository's code, the agent's reasoning, tool output and paths from this machine. Anthropic and OpenAI already see every session, since they run it. A hosted eval service would be one more company holding all of it. The options:
- (a) **Only on machines you run.** No hosted service. On this host that means MLflow or Phoenix, each one process on SQLite. Self-hosted Langfuse needs Docker and about 16 GiB, so a second machine.
- (b) **A hosted service may hold public repositories' logs,** but never a Security session's. Those are easy to keep out: they sit under `commands/secure/`, with the Runs a Security run starts, or in `security-*` files, and a fix you start by hand carries the `security-fix` label. chart35's logs stay local. Langfuse Cloud would cost about $33 a month on its Core plan: the factory would send about 140k units a month, and the free Hobby plan covers 50k.
- (c) **As (b), with private repositories' logs too.**

*Recommended:* (a). It matches the preference for self-hosted in the Warp and OpenHands research, and replays and Run scores need no service at all. On Langfuse Cloud, data past your plan's access window is hidden but still stored until you delete it by API; only the Pro plan, at $199 a month, deletes it on a schedule. Choose (b) if you want Langfuse's grading tools without running a second machine.

*Answer:* are session logs traces? if their data is basically the data of traces and there is a different standard format for traces, I think I want to switch to that. or are the traces formats different depending on the eval framework?****

## Settled by earlier answers (shout if any is wrong)

- No write-up of a Security finding goes to a new third party. That's why a Security run's Run notification, which passes through Resend, carries only severity, title and link ([ADR-0015](../../docs/adr/0015-security-findings-stay-private-until-fixed.md)). So Security sessions' logs and grades stay off any hosted eval service, whatever Q2 decides.
- Eval sessions run through the Harnesses on your subscriptions, as Runs do, not on API keys: subscription auth came first in the Warp and OpenHands research. A tool feature that needs an API key, such as a hosted LLM judge, is out unless you choose to pay for API use.

## Research results (2026-10-16)

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

## Waiting on research

- Nothing is running. The note's live checks (§7) wait until a question needs one, such as whether Claude Code's beta traces arrive when `claude -p` exits, or how much memory MLflow and Phoenix use here.
