# Round 2 (answered)

Part of the [#698 grilling](README.md).

**Q2.1 - Session logs and Traces.** This follows from your Q1.2 answer, and it also settles #700, which asks whether some logs should be renamed traces. The options:
- (a) **Keep Session logs, and add a Trace.** A Trace is one OpenTelemetry trace per command, which thirdshift builds from the command's Session logs and what it knows about its Runs. Session logs stay the record, so a Trace can always be rebuilt from them. Each Trace records the GenAI conventions version it uses.
- (b) **Replace Session logs with Traces written live.** Anything a convention has no field for is lost. The Model evidence and review-report collection would have to be rewritten, since they read each Harness's own stream.
- (c) **Turn on each Harness's own export instead.** Claude Code's beta export has no token counts and redacts content by default. Codex's has no prompts, outputs or model name. The other four vary (eval-tools.md §3.3), and none of them knows about Runs.

*Recommended:* (a), built when the trace-viewing work starts, since replays and Run scores don't need it. For #700 this means no rename: a Session log stays a Session log, and Trace becomes a new term for the tree made from it.

*Answer:* b - check though if we would be losing any useful info by limiting ourselves to tracking what the convention defines, though maybe it's okay to drop that depending on what it is.

*Checked (2026-10-18):* with the conventions alone, a lot of useful information would be lost, mostly from Claude and Grok:
- cost, the cache-time split and rate-limit use;
- permission denials, interrupted work, and the sub-agent and background-task lifecycle;
- edit diffs, push and pull-request events;
- thinking duration and signatures;
- the CLI's own setup.

Muse would lose its task lifecycle, policy decisions and todos, and OpenCode its cost. Codex and agy map almost entirely. Most of what's lost matters for evals: diffs, denials and killed work for accuracy, cost and rate limits for efficiency.

Nothing need be lost if the Trace adds `thirdshift.*` attributes, which OpenTelemetry's naming rules allow, and keeps each raw Harness event on its span. Four catches remain:
- Spans are written when they end, so a killed Run could lose its open ones.
- The default is 128 events per span, and the median session has 248.
- Raindrop Workshop drops structured attribute values.
- The OTLP file format is still Development, and Rust has no file exporter.

This branch's code never reads a Session log back: it writes the log as it streams and passes on only its path. Resume uses the Harness's own session store. Q3.14–Q3.18 follow from this.

*Corrected (2026-10-19):* that last part is out of date on `main`, which is 228 commits ahead of this branch. #711 shipped `show`, which reads the last 64 KiB of a live Session log through its Harness's interpretation to report the latest event (`src/logs/reading/selection.rs:332-354` on `main`). The Console Spec, #776, reads Session logs far more:
- #819 renders Claude's and Codex's logs as a conversation, sending lines appended since a byte offset.
- #815 marks a Run Silent when its Session log and Command log both stop growing.
- #827, the Waterfall, times each tool call by matching it to a Command log line, because Session log lines carry no time.

All three Tickets are open and `ready-for-agent`. Q3.19 asks whether this changes your answer. Confirmed by a structure-only scan of 880 live and 220 replay Session logs, Security sessions skipped, and by the specs and code cited under "Research results (2026-10-18): Traces".

**Q2.2 - Where eval data may live.** This is Q1.2's hosting question again. Your answer asked about formats instead, which Q2.1 now covers. Hosting gates only a trace viewer or a score UI; replays and Run scores need no service. Security data stays off any hosted service whatever you choose. The options:
- (a) **Only on machines you run.**
- (b) **A hosted service may hold public repositories' non-Security logs.**
- (c) **As (b), with private repositories' logs too.**
- (d) **Decide when the viewer work starts.**

*Recommended:* (a), reopened only if a local viewer trial (Workshop, MLflow, Phoenix) falls short. With Q2.1's standard export, moving to a hosted tool later stays cheap.

*Answer:* agree

**Q2.3 - What the first replay set covers.** A replay needs past situations with a known right answer. #429 left three kinds already graded. The options:
- (a) **Review sessions only, from #429.** The 9 bugs with written catch criteria test the code review. The 40 plans, with their stops graded right or wrong, test the plan check. The 30 Spec branches, with findings graded real or wrong, test the Spec review. The right answers exist, so the first set costs only sessions.
- (b) **(a), plus implement sessions.** Re-run a past issue from the commit before its fix, and grade the result against the merged fix. This covers the sessions that write the code, but each one runs far longer and needs its own grading criteria.
- (c) **(a), plus #549's Security review cases.** These would stay private under ADR-0015, and they count only if their findings have since been graded.

*Recommended:* (a). Implement replays can follow as a second set once the runner exists. Nine bugs is a small sample, so a bar on them will be coarse.

*Answer:* agree

*Checked:* (c) was out of date. Codex graded #549's findings on 2026-10-09, but its sample had no confirmed vulnerabilities, so the grades can't show how many real issues a review catches. That makes them weak Tasks, and doesn't change (a). Confirmed by `docs/research/security-review-mode-trial.md`.

*Checked (2026-10-18):* two of (a)'s three suites don't match how production runs:
- **The code review isn't a session of its own.** It runs as sub-agents inside the implement session, so #429's standalone review differs from what a Run does. Q3.9 asks what the code-review suite replays.
- **There is no plan check in production.** It exists only in `replay.py`'s prompt and waits on #429, which is still open. Q3.10 asks what to do with that suite.

The Spec review is its own session, but its prompt commits fixes and updates the pull request, which can't work in a Trial's repository. Q3.11 covers that. Confirmed by `src/prompt.rs:81-170`, `src/spec_run.rs:139-143` and the promptfoo research below.

**Q2.4 - Where the replay runner lives.** You'd replay before changing a Factory skill, a Session prompt, or a Harness, Model or Effort default. All of those live in thirdshift's code. The options:
- (a) **A `thirdshift` subcommand in Rust,** which starts sessions through thirdshift's own Harness and prompt code. A replay then tests exactly what a Run would do: the same flags, the Session prompts from `src/prompt.rs`, the sub-agent rules and the Model evidence.
- (b) **Grow #429's `replay.py` as a separate Python script.** It's quicker to change, but it builds its own command line for each Harness. A change to a Session prompt or a Harness flag reaches it only if someone copies the change by hand.
- (c) **promptfoo, around a thin wrapper that calls thirdshift.** It adds assertions, repeats and a UI. It also adds a Node dependency, and the wrapper still has to build each historical repository.

*Recommended:* (a). A replay that bypasses thirdshift's code can pass while the real Runs regress.

*Answer:* c I think, as I'd prefer to not handroll and I'd like to get experience with the tools already out there which could do this well. I'm working on adding a web ui dashboard type view for seeing the thirdshift activity on a host in https://github.com/JacobStephens2/thirdshift/issues/711. Alternatively though to avoid using another language, maybe a could work but inspired by promptfoo, but probably the Fred Brooks style outsourcing the software engineering to the promptfoo community is worth it, and then maybe I'll get more experience contributing to open source. But ultimately I want the decision here to be oriented towards making thirdshift the highest quality and the evals giving the highest degree of truth and accuracy.

*Checked:* (c) keeps (a)'s accuracy if the wrapper calls thirdshift itself, rather than its own command lines as `replay.py` does. promptfoo then handles the Tasks, repeats, assertions, results and viewer, while thirdshift starts each session exactly as a Run would. The research on how promptfoo would drive thirdshift is back, under "Research results (2026-10-18)", and Q3.8–Q3.13 follow from it. Two options from the landscape note weren't listed: Inspect, a Python framework that would wrap thirdshift the same way, and the native `claude plugin eval`, which tests a single Claude skill.

**Q2.5 - Where cases and results live.** Some of #429's cases come from chart35, which is private, or from bugs that are still unfixed. The options:
- (a) **All private, under `~/.thirdshift/replays/`,** as #429's are. The repository holds the runner and the case format, plus research notes that give counts only.
- (b) **Public repositories' cases in the thirdshift repository,** and the rest kept locally.

*Recommended:* (a). It keeps one place to look, and a case's catch criterion describes its bug, which may still be open.

*Answer:* agree, and consider that I'm working https://github.com/JacobStephens2/thirdshift/issues/711 too to add a web ui here

*Checked:* the `web-ui` branch has no commits beyond `main` yet. promptfoo has its own local viewer, so whether Replay results show there, in #711's web UI or in both is a question for when the promptfoo research is back.

**Q2.6 - What a replay compares against.** A replay runs a candidate setup, which holds the change you're weighing. It needs a baseline to compare with. The options:
- (a) **The current setup, run fresh in the same replay.** That doubles the sessions, but both sides run on the same Harness and Model versions. #429 ran Claude Code 2.1.296, and those versions change between replays.
- (b) **Stored results from the last replay of the current setup,** re-run only when a Harness or Model version changes.
- (c) **The recorded production sessions,** as #429's Spec replay did. This is free, but those sessions ran older skills and versions, so a difference could come from any of them.

*Recommended:* (a). Otherwise you can't tell a change's effect from a Harness update.

*Answer:* agree

**Q2.7 - Names.** `CONTEXT.md` has no terms for any of this yet. "Run" is already taken: it means thirdshift working an issue, so a replay's sessions need another name. Proposed terms:
- **Replay:** running a set of past situations against two or more setups, to compare them with a bar fixed in advance.
- **Case:** one past situation in a Replay. It is a repository state, the kind of session to run on it, and its known right answer, such as a catch criterion, a right stop or a real finding.
- **Arm:** one setup being compared: Harness, Model, Effort, skills and Session prompts.
- **Trial:** one session of one Arm on one Case.
- **Grade:** a judgment of one Trial against its Case's right answer.
- **Trace:** as in Q2.1 (a), if you choose it.

"Eval" stays everyday speech rather than a term, since it covers all three jobs.

*Recommended:* these terms, written into `CONTEXT.md` as soon as you agree.

*Answer:* agree, assuming these are the industry standard terms for agent evals. /research if necessary here, or read the research that was done for this.

*Checked:* only partly. No single standard exists, but Anthropic's "Demystifying evals for AI agents" (9 January 2026), which `eval-tools.md` §5 already draws on, comes closest for agent evals. It defines Eval, Task ("a.k.a problem or test case"), Trial, Grader, Transcript ("also called a trace or trajectory"), Outcome, Evaluation suite, Evaluation harness and Agent harness. It doesn't use Case, Arm or Replay, and it has "grader" but no noun for the result. Other tools differ: SWE-bench and Harbor use task and trial, Inspect uses Task for a whole eval and Sample for one item, and promptfoo uses test case, provider and assertion. Q3.1 revises the names. Confirmed by the article and `eval-tools.md` §5.
