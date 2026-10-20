# Grilling: #698, evals

Issue: https://github.com/JacobStephens2/thirdshift/issues/698
Research: `docs/research/eval-tools.md`, `docs/research/eval-tools-landscape.md` (#741)

## Round 1 (answered)

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

*Checked:* Not quite. A Session log is the stream of events one Harness prints during one session, in that Harness's own format: six Harnesses, six formats. A trace is a tree of timed spans, each with an ID and a parent. One trace could cover a whole command: its Runs, each session in them, and each model call and tool call inside those. Most of a trace's content is already in the Session logs. What they lack is the tree, which thirdshift knows and they don't.

There is one dominant standard, OpenTelemetry, sent over OTLP, and most eval tools accept it. They disagree on the attribute names inside it. OpenTelemetry's own GenAI conventions are still marked Development. Phoenix reads OpenInference's names and takes OTLP only as protobuf. Harbor uses its own ATIF JSON. So one export reaches most tools, but each one shows it a little differently.

thirdshift can't switch the Session logs themselves to a trace format. The Harnesses write them, and thirdshift reads them as they arrive to collect Model evidence and the reviewers' reports. Q3 asks what to do instead.

## Round 2 (answered)

**Q3 - Session logs and Traces.** This follows from your Q2 answer, and it also settles #700, which asks whether some logs should be renamed traces. The options:
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

This branch's code never reads a Session log back: it writes the log as it streams and passes on only its path. Resume uses the Harness's own session store. Q23–Q27 follow from this.

*Corrected (2026-10-19):* that last part is out of date on `main`, which is 228 commits ahead of this branch. #711 shipped `show`, which reads the last 64 KiB of a live Session log through its Harness's interpretation to report the latest event (`src/logs/reading/selection.rs:332-354` on `main`). The Console Spec, #776, reads Session logs far more:
- #819 renders Claude's and Codex's logs as a conversation, sending lines appended since a byte offset.
- #815 marks a Run Silent when its Session log and Command log both stop growing.
- #827, the Waterfall, times each tool call by matching it to a Command log line, because Session log lines carry no time.

All three Tickets are open and `ready-for-agent`. Q28 asks whether this changes your answer. Confirmed by a structure-only scan of 880 live and 220 replay Session logs, Security sessions skipped, and by the specs and code cited under "Research results (2026-10-18): Traces".

**Q4 - Where eval data may live.** This is Q2's hosting question again. Your answer asked about formats instead, which Q3 now covers. Hosting gates only a trace viewer or a score UI; replays and Run scores need no service. Security data stays off any hosted service whatever you choose. The options:
- (a) **Only on machines you run.**
- (b) **A hosted service may hold public repositories' non-Security logs.**
- (c) **As (b), with private repositories' logs too.**
- (d) **Decide when the viewer work starts.**

*Recommended:* (a), reopened only if a local viewer trial (Workshop, MLflow, Phoenix) falls short. With Q3's standard export, moving to a hosted tool later stays cheap.

*Answer:* agree

**Q5 - What the first replay set covers.** A replay needs past situations with a known right answer. #429 left three kinds already graded. The options:
- (a) **Review sessions only, from #429.** The 9 bugs with written catch criteria test the code review. The 40 plans, with their stops graded right or wrong, test the plan check. The 30 Spec branches, with findings graded real or wrong, test the Spec review. The right answers exist, so the first set costs only sessions.
- (b) **(a), plus implement sessions.** Re-run a past issue from the commit before its fix, and grade the result against the merged fix. This covers the sessions that write the code, but each one runs far longer and needs its own grading criteria.
- (c) **(a), plus #549's Security review cases.** These would stay private under ADR-0015, and they count only if their findings have since been graded.

*Recommended:* (a). Implement replays can follow as a second set once the runner exists. Nine bugs is a small sample, so a bar on them will be coarse.

*Answer:* agree

*Checked:* (c) was out of date. Codex graded #549's findings on 2026-10-09, but its sample had no confirmed vulnerabilities, so the grades can't show how many real issues a review catches. That makes them weak Tasks, and doesn't change (a). Confirmed by `docs/research/security-review-mode-trial.md`.

*Checked (2026-10-18):* two of (a)'s three suites don't match how production runs:
- **The code review isn't a session of its own.** It runs as sub-agents inside the implement session, so #429's standalone review differs from what a Run does. Q18 asks what the code-review suite replays.
- **There is no plan check in production.** It exists only in `replay.py`'s prompt and waits on #429, which is still open. Q19 asks what to do with that suite.

The Spec review is its own session, but its prompt commits fixes and updates the pull request, which can't work in a Trial's repository. Q20 covers that. Confirmed by `src/prompt.rs:81-170`, `src/spec_run.rs:139-143` and the promptfoo research below.

**Q6 - Where the replay runner lives.** You'd replay before changing a Factory skill, a Session prompt, or a Harness, Model or Effort default. All of those live in thirdshift's code. The options:
- (a) **A `thirdshift` subcommand in Rust,** which starts sessions through thirdshift's own Harness and prompt code. A replay then tests exactly what a Run would do: the same flags, the Session prompts from `src/prompt.rs`, the sub-agent rules and the Model evidence.
- (b) **Grow #429's `replay.py` as a separate Python script.** It's quicker to change, but it builds its own command line for each Harness. A change to a Session prompt or a Harness flag reaches it only if someone copies the change by hand.
- (c) **promptfoo, around a thin wrapper that calls thirdshift.** It adds assertions, repeats and a UI. It also adds a Node dependency, and the wrapper still has to build each historical repository.

*Recommended:* (a). A replay that bypasses thirdshift's code can pass while the real Runs regress.

*Answer:* c I think, as I'd prefer to not handroll and I'd like to get experience with the tools already out there which could do this well. I'm working on adding a web ui dashboard type view for seeing the thirdshift activity on a host in https://github.com/JacobStephens2/thirdshift/issues/711. Alternatively though to avoid using another language, maybe a could work but inspired by promptfoo, but probably the Fred Brooks style outsourcing the software engineering to the promptfoo community is worth it, and then maybe I'll get more experience contributing to open source. But ultimately I want the decision here to be oriented towards making thirdshift the highest quality and the evals giving the highest degree of truth and accuracy.

*Checked:* (c) keeps (a)'s accuracy if the wrapper calls thirdshift itself, rather than its own command lines as `replay.py` does. promptfoo then handles the Tasks, repeats, assertions, results and viewer, while thirdshift starts each session exactly as a Run would. The research on how promptfoo would drive thirdshift is back, under "Research results (2026-10-18)", and Q17–Q22 follow from it. Two options from the landscape note weren't listed: Inspect, a Python framework that would wrap thirdshift the same way, and the native `claude plugin eval`, which tests a single Claude skill.

**Q7 - Where cases and results live.** Some of #429's cases come from chart35, which is private, or from bugs that are still unfixed. The options:
- (a) **All private, under `~/.thirdshift/replays/`,** as #429's are. The repository holds the runner and the case format, plus research notes that give counts only.
- (b) **Public repositories' cases in the thirdshift repository,** and the rest kept locally.

*Recommended:* (a). It keeps one place to look, and a case's catch criterion describes its bug, which may still be open.

*Answer:* agree, and consider that I'm working https://github.com/JacobStephens2/thirdshift/issues/711 too to add a web ui here

*Checked:* the `web-ui` branch has no commits beyond `main` yet. promptfoo has its own local viewer, so whether Replay results show there, in #711's web UI or in both is a question for when the promptfoo research is back.

**Q8 - What a replay compares against.** A replay runs a candidate setup, which holds the change you're weighing. It needs a baseline to compare with. The options:
- (a) **The current setup, run fresh in the same replay.** That doubles the sessions, but both sides run on the same Harness and Model versions. #429 ran Claude Code 2.1.296, and those versions change between replays.
- (b) **Stored results from the last replay of the current setup,** re-run only when a Harness or Model version changes.
- (c) **The recorded production sessions,** as #429's Spec replay did. This is free, but those sessions ran older skills and versions, so a difference could come from any of them.

*Recommended:* (a). Otherwise you can't tell a change's effect from a Harness update.

*Answer:* agree

**Q9 - Names.** `CONTEXT.md` has no terms for any of this yet. "Run" is already taken: it means thirdshift working an issue, so a replay's sessions need another name. Proposed terms:
- **Replay:** running a set of past situations against two or more setups, to compare them with a bar fixed in advance.
- **Case:** one past situation in a Replay. It is a repository state, the kind of session to run on it, and its known right answer, such as a catch criterion, a right stop or a real finding.
- **Arm:** one setup being compared: Harness, Model, Effort, skills and Session prompts.
- **Trial:** one session of one Arm on one Case.
- **Grade:** a judgment of one Trial against its Case's right answer.
- **Trace:** as in Q3 (a), if you choose it.

"Eval" stays everyday speech rather than a term, since it covers all three jobs.

*Recommended:* these terms, written into `CONTEXT.md` as soon as you agree.

*Answer:* agree, assuming these are the industry standard terms for agent evals. /research if necessary here, or read the research that was done for this.

*Checked:* only partly. No single standard exists, but Anthropic's "Demystifying evals for AI agents" (9 January 2026), which `eval-tools.md` §5 already draws on, comes closest for agent evals. It defines Eval, Task ("a.k.a problem or test case"), Trial, Grader, Transcript ("also called a trace or trajectory"), Outcome, Evaluation suite, Evaluation harness and Agent harness. It doesn't use Case, Arm or Replay, and it has "grader" but no noun for the result. Other tools differ: SWE-bench and Harbor use task and trial, Inspect uses Task for a whole eval and Sample for one item, and promptfoo uses test case, provider and assertion. Q10 revises the names. Confirmed by the article and `eval-tools.md` §5.

## Round 3 (answered)

**Q10 - Names, revised.** This builds on Q9's check. Anthropic's article is the nearest thing to a standard, and its "agent harness" already means what thirdshift calls a Harness. Proposed terms:
- **Eval:** a test of thirdshift's agents.
- **Eval suite:** a set of Tasks for one kind of session, such as the code review's 9 bugs.
- **Task** (replaces Case): one past situation, with its inputs (a repository state and the kind of session to run on it) and its success criteria (a catch criterion, a right stop or a real finding).
- **Arm:** one setup being compared. The **Baseline arm** is the current setup and the **Candidate arm** holds the change. No standard word fits: Anthropic has none, and promptfoo's "provider" is narrower. Arm is the usual word in experiments, and #429 used it.
- **Trial:** one attempt at a Task by one Arm.
- **Grader:** what scores a Trial: code, a model or you. Its result is a **Grade**, the word #429 and `CONTEXT.md`'s Security triage already use.
- **Replay** (kept): one run of an Eval suite on a Baseline arm and a Candidate arm against a bar fixed in advance. Anthropic has no word for this. "Eval" would be ambiguous, and "Run" is taken.

The article also separates *capability* evals, which start at a low pass rate, from *regression* evals, which "should have a nearly 100% pass rate". #429's Tasks are bugs that reviews missed, so they are capability-style. Q13's bar has to be relative to the Baseline arm for that reason. Transcript waits for the Q3 research.

*Recommended:* these terms, in `CONTEXT.md` once you agree.

*Answer:* agree

*Checked (2026-10-20):* `main` has renamed `CONTEXT.md` to `GLOSSARY.md`, which tells readers not to call the Usual range a "baseline". The Baseline arm is a different thing: the Arm that runs the current setup in a Replay, where the Usual range is about stage times in production. So the two don't clash. The terms go into `GLOSSARY.md` once `main` is merged into this branch; see the Settled list.

**Q11 - Trials per Task.** Models aren't deterministic, so one Trial per Arm can't tell a real gain from luck. #429 ran one. The article measures repeated Trials as pass@k (at least one of k succeeds) and pass^k (all k succeed). At #429's medians, a Claude max Trial takes about 20 minutes for a code review, 15 for a plan check and 19 for a Spec review, and a Codex xhigh one about 5 to 7. The options:
- (a) **3 Trials per Arm on each code-review Task, and 1 on each plan-check and Spec-review Task.** With 40 and 30 Tasks, the larger suites already average out much of the noise. A code-review Replay on Claude max is 9 × 3 × 2 = 54 Trials, about 9 hours two at a time.
- (b) **3 everywhere.**
- (c) **1 everywhere,** as #429 did.

*Recommended:* (a).

*Answer:* agree

**Q12 - Which Eval suites a Replay runs.** The options:
- (a) **Only the suites the change touches.** A code review skill change runs the code-review suite. A change to a Harness, Model or Effort default runs every suite whose sessions use that default.
- (b) **Every suite, every time.**

*Recommended:* (a). Running all three on Claude max is roughly a day of sessions.

*Answer:* agree

**Q13 - The bar.** Q8 settled that the Baseline arm runs fresh beside the Candidate. Because the Tasks are capability-style (Q10), an absolute pass rate tells you little. The options:
- (a) **A standing default bar, fixed before each Replay.** The Candidate passes if it is no worse than the Baseline on accuracy (bugs caught, right stops, real findings) and no worse on wrong findings. "Worse" means a gap bigger than the spread between the Baseline's own Trials. Efficiency is shown but isn't part of the bar, unless the change is meant to save time or cost. You may change the bar before a Replay starts, never after.
- (b) **A new bar written for each Replay,** as #429 did.
- (c) **Fixed thresholds,** such as a minimum number of bugs caught.

*Recommended:* (a). It keeps each Replay comparable with the last, and Q1 put accuracy ahead of efficiency.

*Answer:* agree

**Q14 - How Trials are graded.** The article says a model Grader "requires calibration with human graders", and claude.dev adds that the judge "should not be the model you are testing". The options:
- (a) **As #429, plus a human check.** Code Graders run first where a check is mechanical. For example, a Trial that changed its repository is void. Then fresh-session model Graders grade blind: Arm labels shuffled, identifying lines removed, and each Task's Trials in one packet, graded against its written success criteria. The Grader's Model belongs to neither Arm when another is available. You recheck every borderline Grade, and every Grade that decides the bar.
- (b) **You grade every Trial.**
- (c) **Model Graders only.**

*Recommended:* (a). (b) is the most accurate, but 54 Trials a Replay is too many to grade by hand, and (c) is never checked.

*Answer:* agree

**Q15 - Keeping Trials honest.** #429 ran each Trial in a throwaway repository holding only the history up to the commit, with no origin and no tags, `gh` signed out, and a prompt forbidding network use and builds. #549 used fresh clones with `origin` removed. Three gaps remain:
- `replay.py` doesn't assert that no later commit survives, as SWE-bench does.
- Harness memory carries over between sessions: Claude's auto-memory, Grok's memory and Muse's reminders.
- Each Trial runs with full access to the host. That host holds the answers: the grading files under `~/.thirdshift/replays/`, and the real checkouts that contain the later fixes. Only the prompt keeps a Trial out of them.

This host already has bwrap, with the AppArmor profile that Security reviews use. The options:
- (a) **#429's setup, plus the three fixes.** Assert that no later commit survives, turn Harness memory off, and run each Trial in bwrap that shows it only its throwaway repository and its Harness's login files.
- (b) **The first two fixes, without bwrap.**
- (c) **As #429.**

*Recommended:* (a), if the Harness logins work inside bwrap, which needs a live check. It differs from a production Run only in what the session can see on disk.

*Answer:* agree

**Q16 - When and where Replays run.** This host has 4 cores and 8 GB and handles about 2 building Runs at once. Review Trials don't build. Replays use the same subscriptions as production Runs, and `replay.py` stopped at a plan-limit warning. The options:
- (a) **On demand, at most 2 Trials at once beside the factory,** stopping at a plan-limit warning and resuming later.
- (b) **Pause the factory's cron during a Replay.**
- (c) **One Trial at a time,** as #429.

*Recommended:* (a). Stopping at the warning keeps a Replay from using up the plan the factory needs.

*Answer:* agree

*Checked (2026-10-19):* "this host" is the `thirdshift` host, formerly tracewake, which holds #429's Tasks. thirdshift also runs on `stephens`. Replays run on the `thirdshift` host, where the private Tasks are; see the Settled list.

**Q17 - How promptfoo starts a Trial.** This follows from Q6. thirdshift has no command that runs one session: every session runs inside a Run, a Spec run, an Architect run or a Security run. promptfoo's `exec:` provider truncates output at 1 MiB, returns no metadata and leaves sessions running after a timeout. Its JavaScript provider can stop a Trial when it's cancelled and return metadata. Skills are built into the thirdshift binary, so an Arm that changes a skill or a Session prompt needs its own build. The options:
- (a) **A hidden internal flag, like `--release-summary`,** such as `thirdshift --session <kind> --worktree D --logs D`, with the Harness, Model and Effort. It runs one session through the same code a Run uses: the Harness check, the production prompt, skills, sub-agent rules, review reports, Resume and Model evidence. It then prints one JSON line with the final message, log paths, HEAD before and after, and elapsed time. A small JavaScript provider spawns it and stops it on cancel. An Arm is a thirdshift build plus a Harness, Model and Effort.
- (b) **The same command, public and documented,** such as `thirdshift session`.

*Recommended:* (a). It exists for the Eval harness, as `--release-summary` exists for releases. It can go public later if #706's `thirdshift review` wants it.

*Answer:* agre

*Checked (2026-10-19), against the Console:* a Trial must stay out of the Command records. #776 works out each stage's Usual range from the last 30 Command records in a repository, and its Inbox lists every Command a machine's records hold. A Trial that wrote a record would skew the Usual ranges and appear as a Run. So the flag writes no Command record, and its `--logs` directory sits outside the factory's logs folder. The cost is that the Console doesn't show Trials while they run; Q22 covers where they are seen.

**Q18 - What the code-review suite replays.** Q5 chose #429's 9 bugs. In production the review runs as sub-agents inside the implement session, so a standalone review differs from what a Run does. The options:
- (a) **A review-only session,** which runs the production code review skill, with the implement session's sub-agent rules, on the merged pull request's diff. It tests the skill, the Harness, the Model and the Effort, but not how the implement session decides to call the review, or what its author's context adds.
- (b) **The whole implement session,** from the pull request's starting point, graded on whether the bug survives into the final diff. This is truest to production, but the session may write different code where the bug never arises, so the catch criterion may not apply. Each Trial is also far longer.
- (c) **(a) now, and (b) as the implement-session suite Q5 left for later.**

*Recommended:* (c). (a) is what a review skill change or a Model default change most affects, and its catch criteria already exist.

*Answer:* agree

**Q19 - The plan-check suite.** No plan check runs in production. It is #429's proposal, still open, and the Architecture review writes plans without one. A plan-check suite would protect nothing until a plan check is built. The options:
- (a) **Leave it out of the first suites,** and add it when the plan check is built, from #429's 40 graded plans.
- (b) **Keep it now,** to measure Model and Effort changes for a future plan check.

*Recommended:* (a). That leaves code review and Spec review to start with.

*Answer:* Agree. And I want a way to test changes to prompts and models, so for example if I change some wording in the code-review skill, I want to be able to test how that changes the run. And for changing the model too I want to be able to test impact of model changes on the processes run by thirdshift.

*Checked:* noted. The code-review suite (Q18 a) tests a wording change to the code review skill on the review itself. Its effect on a whole Run needs the implement-session suite (Q18 c), and a Model change touches every kind of session. Q29 asks which processes the suites must cover, and in what order.

**Q20 - The Spec review's commits.** The production Spec review prompt has the session commit its fixes and update the pull request through `gh`. A Trial's repository has no origin, and `gh` is signed out. The options:
- (a) **Give each Trial a local bare repository as its origin.** Commits and pushes then work as in production, the `gh` update fails harmlessly, and only the review reports are graded.
- (b) **Leave origin out and let the push fail,** grading the reports only. The session may spend effort working around the failure, which a Run never does.
- (c) **Add a prompt variant that stops after the review.** That isn't the production prompt.

*Recommended:* (a). It changes the least about what the session sees.

*Answer:* agree

**Q21 - promptfoo's defaults.** Several defaults would undermine Trials or send data off this host:
- **Silent retries.** Up to 3 retries fire when an error mentions "rate limit", "timeout", "502" and similar, and each one is a fresh session.
- **Caching.** Reruns return cached Trials unless caching is off.
- **Concurrency.** It defaults to 4.
- **The default grader.** With no API keys it falls back to the Codex SDK on your Codex login, unless a grader is set.
- **Telemetry.** It is on by default. Even `PROMPTFOO_DISABLE_TELEMETRY=1` sends one "telemetry disabled" event to `r.promptfoo.app`, which the source and its own test confirm.
- **The viewer.** `promptfoo view` listens on every network interface.

The options:
- (a) **A fixed setup for every Replay:**
  - retries off (`maxRetries: 0`), `--no-cache` and `-j 2`;
  - a Grader set explicitly;
  - a per-Trial timeout that stops the session;
  - the telemetry, update, sharing and remote-generation switches off, and promptfoo's hosts blocked at the network;
  - the viewer reached only through localhost or an SSH tunnel.

  Then send promptfoo a pull request that makes the telemetry switch fully silent.
- (b) **The same setup, accepting the single telemetry event** rather than blocking the hosts.

*Recommended:* (a). The upstream fix is a small, real first contribution, which you said you'd like.

*Answer:* I think agree. 

Can I use promptfoo with any pretty much any harness I want to and with the subscription offering of these? I want to be able to do that. Grok, codex, claude, opencode (the one api key option in this list), muse, antigravity. And yeah, I don't want to send data off the host in the use of prompt foo or whatever eval framework I choose. I'm not totally committed to promptfoo yet, and would prevent a lot of rework of mine. I'm curious if there are any rust options, but if not promptfoo could work. I just added a thirdshift console web ui in v0.13.0 of thirdshift, so I'm curious how any sort of promptfoo web ui could relate to that, if it even comes with web ui.

And but promptfoo seems like potentially a good option. 

I'm wondering how fitting it would be to setup OpenTelemetry with thirdshift.

*Checked (2026-10-20):*
- **Any Harness, on its subscription.** Yes, by Q17's design. promptfoo never talks to a Harness: its provider starts thirdshift, which runs whichever Harness the Arm names, on the same logins a Run uses. OpenCode runs on its API key, as it does in production. A Grader can run the same way, so it can be any Harness too. promptfoo's own Claude and Codex providers go unused.
- **Nothing off the host.** That now holds for whatever framework is chosen; see the Settled list. Only the Harness sessions, Trials and Graders alike, talk to their providers, as Runs do. An audit of every outbound call promptfoo makes is running.
- **Rust options.** Being researched. Q43 will put the framework choice once the research is back.
- **promptfoo's web UI.** It has one. `promptfoo view` is a local page that shows the Arms side by side from promptfoo's own SQLite database, and it has no sign-in. Q22 and Q30 cover how it relates to the Console.
- **OpenTelemetry.** Q3 and Q23–Q28 already have thirdshift write OpenTelemetry, since Session traces are OTLP. How to write it in Rust, with the OpenTelemetry SDK or directly, is being researched for Q44.

**Q22 - Where Replay results are shown.** promptfoo keeps results in its own SQLite database, shows two Arms as side-by-side columns in its viewer, and exports JSON.

*Updated 2026-10-19 for the Console.* #711 shipped the CLI half, and #776 specifies the Console. Three of the Console's rules bear on this:
- Every page is also a CLI command with `--json` (ADR 0016).
- Nothing is loaded from another site.
- It is served on public addresses behind sign-in.

`promptfoo view` has no sign-in and listens on every interface, so it can't sit behind the Console's address. The options:
- (a) **promptfoo's viewer, on localhost or through an SSH tunnel.** The Console doesn't show Replays for now.
- (b) **A Replays page on the Console,** behind a `thirdshift` command with `--json` that reads promptfoo's exported results, as the CLI-first rule requires. This becomes a Ticket under #776 or a follow-up.
- (c) **Both.** The Console gives a summary per Replay, and promptfoo's viewer gives the Trial-by-Trial detail.

*Recommended:* (a) now, and (c) once Replays run often enough that you want them on your phone. #776's Tickets are already cut, and a Replays page would need its CLI command first.

*Answer:* I'm leaning towards c for integration / preferring one UI to use.

*Checked (2026-10-20):* the Console shipped in v0.13.0 today, and #776 and all its Tickets are closed. Recorded as (c). Q30 asks whether one UI means the Console shows each Trial in full too.

**Q23 - What a Trace holds beyond the conventions.** This follows from Q3 (b) and its check. You said dropping some things might be fine, depending on what they are. The options:
- (a) **Conventions, plus `thirdshift.*` attributes, plus each raw Harness event.** The attributes cover the gaps that matter for evals: cost, the cache split, rate limits, permission denials, interrupted or killed work, sub-agent lifecycle, diffs, push and pull-request events, the CLI's setup and the Session prompt, which no Claude log holds today. Each raw Harness event is kept as a span event on the model or tool span it belongs to, so nothing is lost. After a round-trip check shows (b) misses nothing you use, trim to (b).
- (b) **Conventions plus those `thirdshift.*` attributes, dropping the rest.** What goes is mostly low-value: thinking signatures, streaming deltas, progress heartbeats and idempotency keys.
- (c) **Conventions only.**

*Recommended:* (a), trimmed to (b) after the check. Values are written as JSON strings, which every viewer keeps, even Workshop.

*Answer:* agree

**Q24 - A Trace when a Run is killed.** *Depends on Q28; answer it only if Q28 is (b) or (c).* Today each Session log line is on disk as soon as it arrives. A span is written only when it ends, so a killed Run would lose its open session and Run spans.

*Updated 2026-10-19 for the Console:* a span also isn't written while it is open. So during a long tool call, such as a 20-minute build, a spans-only file stops growing, and #815 would mark a working Run Silent. Today Claude's progress events keep the log growing. The options:
- (a) **Write each span as soon as it ends,** and close the open ones with an error status when thirdshift is interrupted. A model call or tool call ends within seconds, so a hard kill loses only the open parents. The children carry their parent IDs, so the parents can be rebuilt. Long tool calls still need a heartbeat to stay clear of Silent.
- (b) **Buffer the raw stream in a temporary file,** convert it to the Trace when the session ends, and convert any leftover on the next start. Nothing is lost, but the Trace isn't live, and the Console's live log and Silent would have to read the buffer.

*Recommended:* Q28 (b) settles this as a matter of course: its log records are written as they arrive. Choose here only if you pick Q28 (c), and then (a).

*Answer:* agree

**Q25 - Where Traces are written.** The OTLP file format is JSON Lines, ideally `.jsonl`, and still Development. Rust has no exporter for it, so thirdshift would write its own. During a Replay, promptfoo can receive the spans over OTLP and grade on them. Phoenix takes protobuf only. The options:
- (a) **One file per session, where its Session log is today, with all of a command's sessions sharing the command's trace ID.** Spans also go to an OTLP endpoint when one is set: promptfoo during a Replay, or a viewer later.
- (b) **One file per command.** This holds the whole tree, but a command's child Runs would all write to the same file.

*Recommended:* (a). It keeps today's layout and sort order. The shared trace ID ties the files together, and no two sessions write one file.

*Answer:* agree

**Q26 - The terms for Traces.** *If Q28 is (a), Session log keeps its name, and Trace is the new file beside it.* This settles #700's rename. ADR-0011 established the Command log and the Activity log on purpose, and they aren't Harness streams, so they stay logs. The options:
- (a) **Trace** for one command's whole tree, its sessions and their model and tool calls. **Session trace** replaces Session log, for one session's part, kept in its own file. Anthropic's "Transcript" isn't added: the article treats it as a synonym of trace.
- (b) **Keep the name Session log** for the file, though it holds OTLP now.

*Recommended:* (a), with an ADR. Replacing Session logs with OpenTelemetry traces is hard to reverse and surprising without context, and it came out of a real trade-off. If you agree, I'll draft the ADR.

*Answer:* agree

**Q27 - Traces before or after the first Replay.** Traces touch every production session, while the Replay work is mostly new code beside them. thirdshift records no tokens or cost today, so a Replay's efficiency numbers would come from Traces, or from parsing each Harness's Session log. The options:
- (a) **The first Replay comes first.** It grades accuracy and reports minutes only. Tokens and cost come once Traces land, with no throwaway parser.
- (b) **Traces come first,** so the first Replay has tokens, cost and trajectory assertions from the start.
- (c) **Both at once.**

*Recommended:* (a). Q1 put accuracy first, and the first Replay doesn't wait on a change to every session.

*Updated 2026-10-19 for the Console:* the order that matters more is Traces against #819 and #827, not against the first Replay. If Q28 is (b), building #819's Claude and Codex renderers, and #827's Command-log matcher, on today's Session logs means building both twice. The recommendation stays (a) for the Replay. Q28 asks about the Console's order.

*Answer:* Agree - I don't care either way which come first here as long as we get both as part of this spec (or specs if the scope needs multiple)

**Q28 - Session logs, Traces and the Console.** This reopens Q3 because of new facts. The Console needs a file that grows as the session streams: for Silent (#815), for the live conversation sent by byte offset (#819), and for `show`'s latest event. A spans-only Trace grows only when a span ends. OpenTelemetry has a second signal that fits: **log records**, each tied to its trace and span, and written the moment they happen. OpenTelemetry's events are log records with an event name. The options:
- (a) **Keep Session logs as they are, and write a Trace beside each one** (Q3's original (a)). The Console's Tickets go ahead unchanged. #827 can read real span times from the Trace instead of matching Command log lines.
- (b) **Replace Session logs with a Session trace of OTLP log records plus spans.** Each Harness event becomes a log record as it arrives, with `thirdshift.*` attributes and the raw event, so nothing is lost and the file grows live. Spans are written as they end. Two Console Tickets change before they're built:
  - #819 renders the log records, which works the same for all six Harnesses, where #776 covers only Claude and Codex;
  - #827 reads span times, so it needs no Command-log matcher.

  #815's file-growth rule still works. Traces then come before #819 and #827.
- (c) **Replace Session logs with spans only,** as Q24 assumed, with a heartbeat span for long tool calls. The Console reads span events, and the 128-events-per-span limit has to be raised.

*Recommended:* (b). It keeps your Q3 choice, of one standard format and nothing lost. It also makes the Console better: six Harnesses rendered rather than two, and real times for the Waterfall. The cost is ordering: #819 and #827 wait for the Session trace, while #814–#818, #820–#826, #828 and #829 go ahead. Two checks are needed:
- whether Phoenix, MLflow and Workshop show OTLP log records, or only spans;
- whether `show`'s latest-event parser can read them.

*Answer:* agree, no problem on the ordering waiting if it makes a better system. 

*Checked (2026-10-20):* #819 and #827 shipped in v0.13.0 today, so (b) now means reworking them rather than waiting for them. The rework is small:
- the conversation renderer, `src/console/conversation.rs` on `main`, 174 lines, which reads Claude's and Codex's events;
- the Waterfall's Command-log matcher, `src/logs/status/waterfall/matching.rs`, 188 lines, which the records' own times replace;
- `show`'s latest event, in `src/logs/reading/selection.rs`.

Each log record keeps its raw Harness event (Q23), so the renderer can first unwrap the record and keep its Claude and Codex code, then grow to all six Harnesses. #815's Silent rule reads file growth, so it doesn't change. Recorded as (b); shout if the rework changes your answer.

**Q29 - Which processes the suites cover, and in what order.** This builds on your Q19 note: a wording change to a skill, or a Model change, should be tested on what thirdshift actually runs. Q5 and Q18 set the first suites: a review-only code review, and the Spec review. The kinds of session thirdshift starts are:
- the implement session, with its review sub-agents;
- Repairs and Resumes;
- the Base fix;
- the Spec review;
- the Architecture review;
- the Security review and the Security fix.

The release summary also calls a Harness, but it isn't a session.

The options:
- (a) **Code review and Spec review first. Then the implement session (Q18 c), graded against the merged fix and its tests, SWE-bench style. Then the Architecture review, once its plans have graded Tasks.** A Model change runs every suite that exists, and Security stays separate under ADR-0015.
- (b) **The implement session first,** since it is what a Run mostly is, before widening the review suites.
- (c) **Everything at once.**

*Recommended:* (a). The first two suites have Tasks with right answers today, while the implement suite needs grading criteria written for each Task.

*Answer:*  agree

## Round 4 (open)

**Q30 - How much of a Replay the Console shows.** This follows from Q22, where you leaned to (c) and said you prefer one UI. With Session traces (Q28), a Trial writes the same format as a Run, so the Console can show a Trial's session with the code it already uses for a Run's. The options:
- (a) **The Console shows Replays in full.** A Replays page lists each Replay with its verdict against its bar. Each Replay's own page shows every Task by Arm, each Trial's Grades with the Grader's reasons, and each Trial's Session trace. Every page has its CLI command with `--json` (ADR 0016), which reads the Replays folder, outside the factory's logs (Q17). promptfoo's viewer, if promptfoo stays (Q43), is left for debugging promptfoo itself.
- (b) **The Console shows a summary,** each Replay's verdict and its numbers by suite, and promptfoo's viewer shows the Trial-by-Trial detail. This is Q22 (c) as written.
- (c) **A summary now, and the detail later.**

*Recommended:* (a). It's the one UI you asked for, and it doesn't tie a Replay's detail to promptfoo's viewer, which Q43 may drop. The `thirdshift` host's Console would show the private Tasks behind its sign-in, as it already shows Security sessions.

*Answer:* 

**Q31 - How a Replay is started.** Several settled answers need one place that enforces them: the bar is fixed before a Replay starts (Q13), each Task's throwaway repository is built and checked (Q15), promptfoo's settings apply every time (Q21), and a CLI command must list Replays for the Console (Q30). The options:
- (a) **A `thirdshift replay` command.** It takes the suites (Q12), the Candidate (Q32) and the bar. It records the bar with the Replay before the first Trial, builds both Arms, prepares the Tasks, runs the Trials and Graders, applies the bar and writes the result. `thirdshift replays` lists them, with `--json` for the Console. promptfoo, if it stays, runs underneath with Q21's settings.
- (b) **Run promptfoo by hand,** with a config kept in the repository. thirdshift only reads the results.
- (c) **A script beside the config,** as #429's `replay.py` was.

*Recommended:* (a). A bar the command has recorded can't change after the first Trial, and no setting can be forgotten.

*Answer:* 

**Q32 - What the Baseline arm is built from.** Q8 runs the Baseline fresh beside the Candidate, and Q17 makes an Arm a thirdshift build plus a Harness, Model and Effort. Skills and Session prompts are built into the binary. The options:
- (a) **The point where the Candidate's branch left `main`.** Both Arms are built the same way, the Candidate from its branch and the Baseline from that point, so the change is the only difference between them. A change to a Harness, Model or Effort default that needs no code uses one build for both Arms, with different flags.
- (b) **The installed release,** which is what production runs. Anything merged to `main` since that release then counts as part of the change.
- (c) **`main` as it is when the Replay starts.** Anything merged since the branch left `main` then counts against the change.

*Recommended:* (a).

*Answer:* 

**Q33 - How "worse" is measured.** Q13 fails the Candidate when it is worse than the Baseline by more than "the spread between the Baseline's own Trials". That needs a method. With 9 Tasks of 3 Trials each, small differences can't be told from luck, whatever the method. Anthropic's "Adding Error Bars to Evals" (Miller, 2024) recommends comparing two models question by question, and working out in advance how many questions it takes to detect a difference of a given size. The options:
- (a) **Paired by Task, with an interval and three verdicts.** For each Task, take the Candidate's share of passing Trials minus the Baseline's, and give the mean over Tasks a 95% interval.
  - The Candidate **fails** if the whole interval is below zero.
  - It **passes** if the whole interval is above a margin you fix with the bar, such as 10 points worse.
  - Anything else is **inconclusive**, and the report says how many more Tasks would settle it.

  Wrong findings are judged the same way. For the code-review suite, the report also gives pass^3, the share of Tasks where all three Trials pass, as a measure of consistency.
- (b) **Fail only when clearly worse.** As (a), but anything not clearly worse passes, so a real regression can hide in a small suite.
- (c) **Q13's words as they stand.** The Candidate fails if its total is below the lowest of the Baseline's three totals, one per repeat. It's simple, but it ignores the Candidate's own luck, and with 9 Tasks a single Trial can decide it.

*Recommended:* (a). It never reports a pass the data can't support. With 9 Tasks, expect most Replays to come out inconclusive unless a change is large. That is the honest result, and the reason for Q38.

*Answer:* 

**Q34 - What a Run score records.** Run scores open now that grading is settled (Q14). A Run score attaches to its Run's Command record, and its trends go to #813's charts (Settled). The options:
- (a) **Everything the Run and its Session traces show, as separate signals and never one number:**
  - from the Run: its outcome, Repairs, CI rounds, Unaddressed findings, the review's findings by kind, and each stage's time against its Usual range;
  - from its Session traces: tokens by Model, cost where the Harness reports one, tool errors, permission denials, killed work, repeated tool calls, and destructive commands a code Grader can spot, such as deleting files outside the worktree.

  thirdshift keeps no price table, so a Harness that reports no cost shows tokens only. A model Grade is Q35's question.
- (b) **Only what the Run knows,** leaving out the Session-trace signals.
- (c) **(a), plus a price table,** so that every Harness gets a cost-equivalent.

*Recommended:* (a). A price table goes stale, and cost on a subscription is notional. Tokens and minutes compare well within one Model.

*Answer:* 

**Q35 - Grading production Runs.** Q1 asked for production to be analyzed for accuracy and quality, and a Run's own facts measure friction more than quality. From 3 to 16 October the factory ended about 140 Commands a week. The options:
- (a) **A model Grader grades a random sample of finished Runs,** about 10 a week, spread across the Harnesses and Models in use. It sees the issue, the merged diff and the pull request, but not the Harness, Model or Effort, and it grades against Q36's rubric. Its Model differs from the Run's (Q14). You grade some of the same Runs, and its Grades show as uncalibrated until they agree with yours. It runs on your subscriptions and stops at a plan-limit warning, as Replays do.
- (b) **Grade every Run,** each in a session of its own.
- (c) **No model Grades in production.** Rely on Q34's signals and Q37's bugs.

*Recommended:* (a). Production Grades watch for drift, and Replays decide changes, because production Runs work different issues and can't be compared like for like.

*Answer:* 

**Q36 - What a production Grade judges.** Q1 put accuracy and quality first, and asked what else evals should consider. The options:
- (a) **Five things, each graded and shown on its own:**
  - **Correct:** the diff does what the issue asked, with no bug the Grader can name.
  - **Scoped:** it changes nothing the issue didn't ask for.
  - **Tested:** tests cover the change, and none was weakened to make it pass.
  - **Honest:** the pull request and the final message claim only what the diff and CI show.
  - **Standards:** it follows the repository's coding standards and reads like the code around it.
- (b) **Correct and Tested only.**
- (c) **Correct only.**

*Recommended:* (a). Safety stays with Q34's code Graders, which check every Run rather than a sample.

*Answer:* 

**Q37 - Tracing bugs back to the Runs that caused them.** Q1 (b) counted what became of a Run's pull request: reverted, or a bug traced to it. thirdshift has never had a revert, but 35 of its issues opened since 3 October are labelled `bug`. Nothing links a bug to the pull request that caused it, yet those links are the nearest thing production has to a right answer. The options:
- (a) **A short session proposes each link, and you confirm it.** When a Run on a `bug` issue merges, a session reads the fix and the history and names the commit that introduced the bug, if the history shows one. thirdshift maps that commit to its pull request, and to that pull request's Command record if a Run made it. Once you confirm or correct the link from the CLI, it counts against that Run's score and feeds Q38. Confirming from the Console as well would amend ADR 0016, which limits the Console's actions to Stop and its own look.
- (b) **You record the links by hand,** for example with a line on the bug's issue that names the pull request.
- (c) **No links:** reverts only.

*Recommended:* (a). It leaves the implement session's prompt alone, so it needs no Replay of its own, and nothing counts until you confirm it.

*Answer:* 

**Q38 - New Tasks from bugs found in production.** Under Q33, 9 Tasks will rarely give a clear verdict. #429's Tasks are bugs that reviews missed, and each link confirmed under Q37 is another: a bug the code review let through. The options:
- (a) **Each confirmed link proposes a Task** for the code-review suite: the pull request's diff as it was reviewed, and a draft catch criterion. It joins the suite once you approve it, and it stays private under `~/.thirdshift/replays/` (Q7).
- (b) **Tasks come only from deliberate studies,** as #429's did.
- (c) **(a), plus Spec findings you judge real or wrong in production,** for the Spec-review suite.

*Recommended:* (a). It grows the suite from the failures that matter most. Spec findings can follow once there's a way to record your judgment of them.

*Answer:* 

**Q39 - The command and its Runs in a Trace.** Q26 made a Trace one command's whole tree, and Q25 put each session's part in a Session trace of its own. The levels above the sessions need a home too: the command, its Runs and their stages. The Command record already holds each Run's stages and when each began, and the Waterfall draws those levels from it. The options:
- (a) **Derive them from the Command record.** Each Session trace carries IDs computed from its Command record, so a whole Trace can be put together without guessing. The Console does that, and an export command writes the whole Trace as OTLP for any other tool.
- (b) **Write them as spans in one more file beside the Command record,** so the files on disk make a whole standard Trace by themselves. Stage times are then kept twice, in the record and in the file.
- (c) **Copy the command's and Run's spans into each Session trace.**

*Recommended:* (a). The Command record stays the one place those times are kept, so nothing has to be kept in step.

*Answer:* 

**Q40 - Session logs from before the switch.** There are 924 Session logs from production, not counting Security sessions, and 215 from #429's replays. The Console reads them today. The options:
- (a) **Convert them once into Session traces,** with the same code that writes Session traces live, and keep the originals until the conversion is checked. Codex's lines carry no time and only some of Claude's do, so a record without a time gets one estimated from the Command log, as the Waterfall does now, and marked as estimated. The Console then needs one renderer, and Run scores and #813's charts start with history.
- (b) **Leave them as they are.** The Console keeps its Claude and Codex code for them, and the Session-trace signals start at the switch.
- (c) **Convert them on demand,** when a chart or a Run score needs the history.

*Recommended:* (a). The conversion also serves as Q23's round-trip check, over every log there is. Security sessions' logs are converted on the machine like the rest.

*Answer:* 

**Q41 - The Console as the trace viewer.** Q4 kept a local OpenTelemetry viewer as the fallback, and Q28 left a check open on whether Phoenix, MLflow and Workshop show log records. With Session traces, the Console shows any session from any Harness. The options:
- (a) **The Console is the trace viewer,** with no other viewer for now. Session traces are standard, so one can be added later without changing them, and the check waits until then.
- (b) **Also run Phoenix or MLflow on the `thirdshift` host,** for searching and comparing spans across Runs, which the Console doesn't do. This needs the check first, and memory on an 8 GB host.

*Recommended:* (a). It's the one UI you prefer, and #813's charts will cover trends across Runs.

*Answer:* 

**Q42 - How this becomes Specs.** Q27 keeps Replays and Session traces both in this work, in one Spec or several. They touch different code. Session traces change every session, the Console's log view and Waterfall, and `show`. Replays add new commands and the Eval harness. Run scores need both. The options:
- (a) **Three Specs, for Session traces, Replays and Run scores,** each with its own Tickets, and all linked from #698. Session traces and Replays can be built in parallel, and Run scores follow both.
- (b) **One Spec for all of it.**
- (c) **Two Specs:** Session traces, and then Replays with Run scores.

*Recommended:* (a). Each can be reviewed and merged on its own, and the factory can work on the first two at once.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- No write-up of a Security finding goes to a new third party. That's why a Security run's Run notification, which passes through Resend, carries only severity, title and link ([ADR-0015](../../docs/adr/0015-security-findings-stay-private-until-fixed.md)). So Security sessions' logs and grades stay off any hosted eval service, whatever Q2 decides.
- **Order.** Replays and Session traces both belong to this work, in either order (Q27). Session traces come before the Console's log view and Waterfall are reworked (Q28). Run scores follow both, since they need Session-trace signals and Graders calibrated in Replays.
- Accuracy and quality come first. Efficiency (minutes, tokens, cost-equivalent) is measured and shown beside them, never folded into one score (Q1).
- A Run's outcome, its Repairs and its CI rounds don't count as evidence of quality on their own (Q1, and the triage notes on the issue).
- **Tools.** Packaged tools come before handrolling, but accuracy outranks that preference: Trials start sessions through thirdshift's own code (Q6). promptfoo leads but isn't committed to. Q21 asked about Rust options, and Q43 will put the choice. Langfuse is out for now (Q4).
- **Nothing leaves the host but the sessions.** Eval data, Session traces included, stays on machines you run, and the eval framework, whichever it is, sends nothing anywhere (Q4, Q21). Only the Harness sessions talk to their providers: Trials and Graders do so just as Runs do. Hosted viewers are out unless a local one falls short (Q4).
- Eval sessions run through the Harnesses on your subscriptions, as Runs do, not on API keys: subscription auth came first in the Warp and OpenHands research. A tool feature that needs an API key, such as a hosted LLM judge, is out unless you choose to pay for API use.
- **Names (Q10):** Eval, Eval suite, Task, Arm (the Baseline arm and the Candidate arm), Trial, Grader, Grade and Replay.
- **Suites.** The first Eval suites are code review, which replays #429's 9 bugs in a review-only session (Q18), and Spec review, #429's 30 Spec branches (Q5). The plan-check suite waits for a plan check to exist (Q19). The implement session comes next, graded against the merged fix and its tests, then the Architecture review once its plans have graded Tasks. A Model change runs every suite that exists, and Security stays separate under ADR-0015 (Q29).
- **Trials.** Three per Arm on each code-review Task, and one on each Spec-review Task (Q11). A Replay runs only the suites its change touches, and a change to a Harness, Model or Effort default runs every suite that uses it (Q12). Every Replay runs the Baseline arm fresh beside the Candidate arm, which doubles its Trials (Q8).
- **The bar** is a standing default, fixed before each Replay and never changed after it starts. The Candidate must be no worse than the Baseline on accuracy and on wrong findings. Efficiency is shown, but kept out of the bar unless the change is meant to save time or cost (Q13). Q33 asks how "worse" is measured.
- **Grading.** Code Graders run first, where a check is mechanical. Then blind, fresh-session model Graders grade one packet per Task against its written success criteria, on a Model from neither Arm when another is available. You recheck borderline Grades and the ones that decide the bar (Q14).
- **Keeping Trials honest.** Each Trial gets a throwaway repository that is checked for later commits, with Harness memory off, inside bwrap that shows it only that repository and its Harness's login files (Q15).
- **Running.** On demand, at most two Trials at once beside the factory, stopping at a plan-limit warning and resuming later (Q16). Replays run on the `thirdshift` host, where #429's private Tasks are; `stephens` runs none.
- **Starting a Trial.** A hidden flag runs one session through the code a Run uses and prints one JSON line, and a small JavaScript provider starts and stops it. An Arm is a thirdshift build plus a Harness, Model and Effort (Q17). Trials never write a Command record, and their logs sit outside the factory's logs folder, so the Console's Inbox, Usual ranges and Silent ignore them.
- **Spec-review Trials** get a local bare repository as their origin, and only their review reports are graded (Q20).
- **promptfoo, if it stays,** runs with retries off, no cache, two Trials at a time, a Grader set explicitly, and a per-Trial timeout that stops the session. Its telemetry, update, sharing and remote-generation switches are off, its hosts are blocked, and its viewer is reached only on localhost or through an SSH tunnel. Fixing its telemetry switch upstream is a first contribution (Q21).
- **Replay results** show in the Console, which you'd like to be the one UI, with promptfoo's viewer for the detail as things stand (Q22). Q30 asks how far that goes.
- Tasks, Trials and Grades stay private under `~/.thirdshift/replays/`. The repository holds the runner and the Task format, and research notes give counts only (Q7).
- **Session traces replace Session logs** (Q3, Q28). Each session writes one file where its Session log is today, and all of a command's sessions share its trace ID (Q25). The file holds an OTLP log record for each Harness event as it arrives, and spans written as they end. Spans also go to an OTLP endpoint when one is set. An interruption closes the open spans with an error status, and after a hard kill their records are already on disk (Q24).
- **What a Session trace holds** (Q23): the GenAI conventions' fields, `thirdshift.*` attributes for what those lack, and each raw Harness event. It is trimmed only after a round-trip check shows that nothing you use is lost, and it records the conventions version it follows. Model evidence and review reports stay beside each session's file, named from its path as now.
- **Terms** (Q26): a Trace is one command's whole tree, and a Session trace is one session's part, in place of Session log. That answers #700. Command logs and Activity logs stay logs. An ADR records the switch.
- **The Console's rework** (Q28): its log view renders Session traces, first by unwrapping each record's raw event for its Claude and Codex code, then for all six Harnesses. The Waterfall takes tool times from the records and drops its Command-log matcher, and `show` reads the latest record. Silent's file-growth rule doesn't change.
- **Security sessions and their Session traces stay on the machine.** The Console shows them behind its sign-in, on that same machine, as #776 specifies. That is consistent with Q4.
- **Run scores attach to Command records.** Their trends belong with #813's charts, under its rules: a CLI command with `--json` first, drawn from the Command records and logs, and nothing loaded from another site. The stage times and Usual ranges the Console already computes are the start of Run-score efficiency. A production Grader is calibrated against your Grades, as in Replays (Q14).
- **This branch is 411 commits behind `main`,** where `CONTEXT.md` is now `GLOSSARY.md` and the ADRs run to 0018. Q10's terms, Q26's rename and its ADR go in once `main` is merged into this branch. The code facts in this file need rechecking against `main` before anything is built.

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

## Research results (2026-10-17)

- **Session logs aren't traces, and the trace standards differ in role.** OpenTelemetry is the data model, OTLP its transport, the GenAI conventions (Development status) and OpenInference its attribute names, and ATIF a separate trajectory JSON. MLflow and Workshop accept OTLP as JSON, while Phoenix requires protobuf. Confirmed by `docs/research/eval-tools-landscape.md` §3.7.
- **thirdshift reads Session logs as they're written.** Recording consumes each transcript to checkpoint Model evidence, and a review session's reports are kept beside its Session log. Confirmed by `src/session/recording.rs` and `CONTEXT.md`.
- **`replay.py` builds its own command lines,** for example `claude -p … --permission-mode auto --model … --effort …` and `codex exec --json …`, rather than going through thirdshift's Harness code. Confirmed by `docs/research/second-opinion-replays/replay.py`, lines 334–354.
- **The wider landscape** adds Raindrop Workshop as a local viewer, the native `claude plugin eval` for skill changes, and Braintrust and LangSmith if hosting is allowed. For replays it still recommends maintaining #429's runner. Confirmed by `docs/research/eval-tools-landscape.md`, TL;DR and §4.

- **Anthropic's eval terms:** Eval, Task, Trial, Grader, Transcript (trace, trajectory), Outcome, Evaluation suite, Evaluation harness and Agent harness, plus the split between capability and regression evals. Case, Arm and Replay don't appear. Confirmed by reading "Demystifying evals for AI agents".
- **#549's Security findings were graded by Codex on 2026-10-09,** against no confirmed vulnerabilities. Confirmed by `docs/research/security-review-mode-trial.md`.
- **#711's `web-ui` branch is still at `main`.** Node v26.10.0 and npx 12.2.0 are installed here, so promptfoo can run without new system packages. Confirmed by `git log` and `node --version`.

## Research results (2026-10-18): promptfoo with thirdshift

Read at promptfoo 0.124.1 (commit `421e795`, MIT, npm's `latest`) and in thirdshift's source. Nothing was installed or run.

- **thirdshift has no one-session command.** Its commands are Help, Version, Update, Setup, ReleaseSummary, EmailTest, Architect, Pickup, Secure and Run (`src/args.rs:20-33`). `Sessions::within(...).run` already holds what a faithful Trial needs: skills, the Codex fresh-sub-agent rule, review reports, Resume and Model evidence (`src/session.rs:120-235`). A Trial entry point also needs a `Logs` that takes an explicit directory (`src/logs.rs:183-189`).
- **thirdshift records no tokens or cost.** A session's end holds only its session id, killed work and final message (`src/harness/interpretation.rs:55-59`). Efficiency would have to come from the Session log or a Trace.
- **`replay.py` differs from production** in several flags (`--add-dir`, `--disallowedTools WebFetch WebSearch`, `--strict-mcp-config`), an environment variable and its skill setup, and it has no Resume. Confirmed by `replay.py:334-407` and `src/harness/claude.rs:90-104`.
- **promptfoo's JavaScript provider fits; `exec:` doesn't.** The provider receives `repeatIndex`, the test and a `traceparent`, can return metadata, tokens, cost and errors, and gets a cancel signal on timeout. Confirmed by promptfoo's source: `src/types/providers.ts`, `src/contracts/providers.ts`, `src/providers/scriptCompletion.ts`.
- **Graders.** `llm-rubric` takes any custom provider, which must return `{pass, score, reason}`. By default it sees only the output and the rubric, so blind grading is up to the provider stripping identifying lines. `select-best` doesn't shuffle. Confirmed by `src/providers/defaults.ts` and `src/matchers/`.
- **Tracing.** With tracing on, promptfoo receives OTLP over HTTP, JSON or protobuf, at `127.0.0.1:4318`. It passes a W3C `traceparent` to each Trial and stores matching spans with the result, for trajectory and span assertions. Confirmed by `src/tracing/otlpReceiver.ts` and `site/docs/tracing.md`.
- **Pitfalls on long Trials:** silent retries (`src/scheduler/retryPolicy.ts`), a 14-day cache, concurrency 4, no default per-Trial timeout, telemetry that sends one event even when disabled (`src/telemetry.ts:116-178` and its test), and a viewer on all interfaces (`src/server/server.ts:548`).

## Research results (2026-10-18): Traces

A structure-only scan, with event types and key names but no content: 880 live Session logs and 220 from #429's replays, Security sessions skipped.

- **Claude Code** (688 files) streams assistant, user, thinking, task, rate-limit, push and result events. Its usage carries the cache-time split, service tier and thinking tokens. Its results carry `total_cost_usd`, per-model usage, permission denials and sub-agent statistics. Tool results carry edit diffs and git operations. It has no compaction, hook or todo events.
- **Codex** (186 files) has threads, turns and items: commands, messages, file changes without diff text, web searches and sub-agent calls. Its usage includes cached and reasoning tokens. It has no timestamps, model or reasoning items.
- **Muse** records task lifecycles, policy decisions, todos and the workspace commit, with no token usage. **Grok** is a subset of Claude's format, with no per-event timestamps. **OpenCode** has steps with tokens and cost, and tools. **agy** has steps with usage, tools and sub-agent details.
- **The GenAI conventions** (at `547934a`, Development, no releases) cover chat, agent, tool and skill spans, models, conversation IDs, token usage including cache and reasoning, tool calls, and opt-in message content. **OpenInference** adds cost and thinking signatures, and free-form metadata for the rest.
- **OpenTelemetry allows prefixed custom attributes** such as `thirdshift.*`, with map, array or JSON-string values. Attribute length is unlimited by default. Events per span default to 128, while a session's events have a median of 248, a 90th percentile of 421 and a maximum of 2,672.
- **Viewers treat custom data differently.** Phoenix and MLflow show attributes and events. Workshop keeps only scalar attributes. Langfuse puts them under metadata, where they can't be queried.
- **thirdshift writes Session logs and never reads them back.** `follow` copies each line to the file and feeds the live interpretation (`src/session/recording.rs:227-253`). The Model evidence and review reports are named from the log's path (`recording.rs:137`, `src/session/review_reports.rs:36-49`). Resume passes the live session ID to the Harness (`src/session.rs:212-223`). Changing the format touches the path scheme, those two sidecars, the README's Logs section and tests that read logs back, such as `tests/codex_sessions.rs:304`.

## Research results (2026-10-19): the Console

- **#711 is closed** (2026-10-18), having shipped the Command record, `status`, `show`, `logs` and `stop`. **#776, the Console's Spec,** is open and `ready-for-agent`, with Tickets #814–#829. Confirmed by `gh issue view`.
- **What the Console reads.** It reads Command records, Activity logs, Command logs and Session logs through the same code as the CLI. It renders Claude's `stream-json` and Codex's `exec --json` logs, with per-tool renderers and a plain fallback, and only the current log layout. Its Out of Scope excludes any HTTP API, other machines and an SQLite index. Confirmed by #776's Implementation Decisions.
- **Silent, Slow and the Usual range.** A Run is Silent when its Session log and Command log have both stopped growing for `inbox.silent_minutes`. The Usual range is the 25th to 75th percentile of a stage's last 30 times in a repository on a machine. Confirmed by #776 and `CONTEXT.md` on `main`.
- **#813, charts over time,** is open for grilling. One of its candidates is tokens per day by Harness and Model, which thirdshift doesn't record today. Confirmed by `gh issue view 813`.
- **`show` reads Session log content** on `main`: the last 64 KiB of a live log, through the Harness's interpretation. Confirmed by `src/logs/reading/selection.rs:332-354` at `origin/main`.

## Research results (2026-10-20)

- **v0.13.0 shipped the Console today.** #776 and its Tickets #814–#829 are closed, and `main` is 411 commits ahead of this branch. Confirmed by `gh release list`, `gh issue view` and `git rev-list`.
- **What on `main` reads Session logs.** The Console's conversation renderer (`src/console/conversation.rs`, 174 lines) parses Claude's and Codex's events and shows any other line plain. The Waterfall's matcher (`src/logs/status/waterfall/matching.rs`, 188 lines) times tool calls from the Command log's timed lines. `show`'s latest event is in `src/logs/reading/selection.rs`. Confirmed by reading them at `origin/main`.
- **The Command record** holds the command's Harness, Model and Effort, each stage's name, start and Session log, the outcome, the pull request, a link to its parent record, and its Tickets (`src/logs/command_record.rs:30-106` on `main`). Confirmed by source.
- **Model evidence and review reports** are still named from the Session log's path: `models.json` beside it (`src/session/recording.rs:144`), and the reports beside it (`src/session/review_reports.rs:49`). Confirmed by source at `origin/main`.
- **ADR 0016** keeps async code inside the Console and the rest of thirdshift blocking, and it limits the Console's actions to Stop and its own look. Confirmed by `docs/adr/0016-the-console-is-a-long-running-reader.md` on `main`.
- **#776 shows Security sessions and findings behind the sign-in.** Confirmed by #776's body.
- **`CONTEXT.md` is now `GLOSSARY.md` on `main`,** and the ADRs run to 0018. Glossary terms go in during grilling, before the code: Command record, Silent and the Usual range came in "Grill #711" commits. Confirmed by `git log -S`.
- **Old Session logs.** There are 924 from production, not counting 71 Security files, and 215 from #429's replays. No line of Codex's carries a time, some of Claude's do, and every one of OpenCode's does. Confirmed by a count-only scan of the logs' keys, with Security sessions skipped and nothing printed.
- **Bugs and reverts.** thirdshift has no revert in its history or its pull requests. 35 of its issues opened since 3 October are labelled `bug`, out of 50 in all. Confirmed by `gh issue list`, `gh pr list` and `git log --grep`.

## Waiting on research

- **Rust eval frameworks and OpenTelemetry in Rust,** for Q43 (the framework) and Q44 (how thirdshift writes OTLP). The same research confirms the paper Q33 cites.
- **Every outbound call promptfoo makes,** with the switch that turns each off, for Q43.
- **Live checks before building:** whether the Harness logins work inside bwrap, and how to turn each Harness's memory off (Q15).
- **Whether local viewers show OTLP log records,** needed only if Q41 is (b).
- The landscape note's other live checks (§7) still wait until a question needs one, such as how much memory a viewer uses here.
