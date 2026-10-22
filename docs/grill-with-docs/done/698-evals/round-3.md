# Round 3 (answered)

Part of the [#698 grilling](README.md).

**Q3.1 - Names, revised.** This builds on Q2.7's check. Anthropic's article is the nearest thing to a standard, and its "agent harness" already means what thirdshift calls a Harness. Proposed terms:
- **Eval:** a test of thirdshift's agents.
- **Eval suite:** a set of Tasks for one kind of session, such as the code review's 9 bugs.
- **Task** (replaces Case): one past situation, with its inputs (a repository state and the kind of session to run on it) and its success criteria (a catch criterion, a right stop or a real finding).
- **Arm:** one setup being compared. The **Baseline arm** is the current setup and the **Candidate arm** holds the change. No standard word fits: Anthropic has none, and promptfoo's "provider" is narrower. Arm is the usual word in experiments, and #429 used it.
- **Trial:** one attempt at a Task by one Arm.
- **Grader:** what scores a Trial: code, a model or you. Its result is a **Grade**, the word #429 and `CONTEXT.md`'s Security triage already use.
- **Replay** (kept): one run of an Eval suite on a Baseline arm and a Candidate arm against a bar fixed in advance. Anthropic has no word for this. "Eval" would be ambiguous, and "Run" is taken.

The article also separates *capability* evals, which start at a low pass rate, from *regression* evals, which "should have a nearly 100% pass rate". #429's Tasks are bugs that reviews missed, so they are capability-style. Q3.4's bar has to be relative to the Baseline arm for that reason. Transcript waits for the Q2.1 research.

*Recommended:* these terms, in `CONTEXT.md` once you agree.

*Answer:* agree

*Checked (2026-10-20):* `main` has renamed `CONTEXT.md` to `GLOSSARY.md`, which tells readers not to call the Usual range a "baseline". The Baseline arm is a different thing: the Arm that runs the current setup in a Replay, where the Usual range is about stage times in production. So the two don't clash. But the Ticket entry tells agents to avoid "task", because they tend to call Tickets tasks. So Task is renamed **Eval task**, which you chose in chat on 2026-10-20. The terms are now in `GLOSSARY.md`, through #920.

*Renamed (2026-10-21):* to match the words eval tools use, Replay is now **Experiment** and Grade is now **Score**, and the Scoring run, Run metrics and Bug origin join them (#943, and Q5.1 and Q5.2). The file uses the new names from Q3.2 on, except in Q5.1 and Q5.2, which asked about them. Rounds 1 and 2, Q2.7, Q3.1 and your answers keep the words they were written in.

**Q3.2 - Trials per Task.** Models aren't deterministic, so one Trial per Arm can't tell a real gain from luck. #429 ran one. The article measures repeated Trials as pass@k (at least one of k succeeds) and pass^k (all k succeed). At #429's medians, a Claude max Trial takes about 20 minutes for a code review, 15 for a plan check and 19 for a Spec review, and a Codex xhigh one about 5 to 7. The options:
- (a) **3 Trials per Arm on each code-review Task, and 1 on each plan-check and Spec-review Task.** With 40 and 30 Tasks, the larger suites already average out much of the noise. A code-review Experiment on Claude max is 9 × 3 × 2 = 54 Trials, about 9 hours two at a time.
- (b) **3 everywhere.**
- (c) **1 everywhere,** as #429 did.

*Recommended:* (a).

*Answer:* agree

**Q3.3 - Which Eval suites an Experiment runs.** The options:
- (a) **Only the suites the change touches.** A code review skill change runs the code-review suite. A change to a Harness, Model or Effort default runs every suite whose sessions use that default.
- (b) **Every suite, every time.**

*Recommended:* (a). Running all three on Claude max is roughly a day of sessions.

*Answer:* agree

**Q3.4 - The bar.** Q2.6 settled that the Baseline arm runs fresh beside the Candidate. Because the Tasks are capability-style (Q3.1), an absolute pass rate tells you little. The options:
- (a) **A standing default bar, fixed before each Experiment.** The Candidate passes if it is no worse than the Baseline on accuracy (bugs caught, right stops, real findings) and no worse on wrong findings. "Worse" means a gap bigger than the spread between the Baseline's own Trials. Efficiency is shown but isn't part of the bar, unless the change is meant to save time or cost. You may change the bar before an Experiment starts, never after.
- (b) **A new bar written for each Experiment,** as #429 did.
- (c) **Fixed thresholds,** such as a minimum number of bugs caught.

*Recommended:* (a). It keeps each Experiment comparable with the last, and Q1.1 put accuracy ahead of efficiency.

*Answer:* agree

**Q3.5 - How Trials are graded.** The article says a model Grader "requires calibration with human graders", and claude.dev adds that the judge "should not be the model you are testing". The options:
- (a) **As #429, plus a human check.** Code Graders run first where a check is mechanical. For example, a Trial that changed its repository is void. Then fresh-session model Graders grade blind: Arm labels shuffled, identifying lines removed, and each Task's Trials in one packet, graded against its written success criteria. The Grader's Model belongs to neither Arm when another is available. You recheck every borderline Score, and every Score that decides the bar.
- (b) **You grade every Trial.**
- (c) **Model Graders only.**

*Recommended:* (a). (b) is the most accurate, but 54 Trials an Experiment is too many to grade by hand, and (c) is never checked.

*Answer:* agree

**Q3.6 - Keeping Trials honest.** #429 ran each Trial in a throwaway repository holding only the history up to the commit, with no origin and no tags, `gh` signed out, and a prompt forbidding network use and builds. #549 used fresh clones with `origin` removed. Three gaps remain:
- `replay.py` doesn't assert that no later commit survives, as SWE-bench does.
- Harness memory carries over between sessions: Claude's auto-memory, Grok's memory and Muse's reminders.
- Each Trial runs with full access to the host. That host holds the answers: the grading files under `~/.thirdshift/replays/`, and the real checkouts that contain the later fixes. Only the prompt keeps a Trial out of them.

This host already has bwrap, with the AppArmor profile that Security reviews use. The options:
- (a) **#429's setup, plus the three fixes.** Assert that no later commit survives, turn Harness memory off, and run each Trial in bwrap that shows it only its throwaway repository and its Harness's login files.
- (b) **The first two fixes, without bwrap.**
- (c) **As #429.**

*Recommended:* (a), if the Harness logins work inside bwrap, which needs a live check. It differs from a production Run only in what the session can see on disk.

*Answer:* agree

**Q3.7 - When and where Experiments run.** This host has 4 cores and 8 GB and handles about 2 building Runs at once. Review Trials don't build. Experiments use the same subscriptions as production Runs, and `replay.py` stopped at a plan-limit warning. The options:
- (a) **On demand, at most 2 Trials at once beside the factory,** stopping at a plan-limit warning and resuming later.
- (b) **Pause the factory's cron during an Experiment.**
- (c) **One Trial at a time,** as #429.

*Recommended:* (a). Stopping at the warning keeps an Experiment from using up the plan the factory needs.

*Answer:* agree

*Checked (2026-10-19):* "this host" is the `thirdshift` host, formerly tracewake, which holds #429's Tasks. thirdshift also runs on `stephens`. Experiments run on the `thirdshift` host, where the private Tasks are; see the Settled list.

**Q3.8 - How promptfoo starts a Trial.** This follows from Q2.4. thirdshift has no command that runs one session: every session runs inside a Run, a Spec run, an Architect run or a Security run. promptfoo's `exec:` provider truncates output at 1 MiB, returns no metadata and leaves sessions running after a timeout. Its JavaScript provider can stop a Trial when it's cancelled and return metadata. Skills are built into the thirdshift binary, so an Arm that changes a skill or a Session prompt needs its own build. The options:
- (a) **A hidden internal flag, like `--release-summary`,** such as `thirdshift --session <kind> --worktree D --logs D`, with the Harness, Model and Effort. It runs one session through the same code a Run uses: the Harness check, the production prompt, skills, sub-agent rules, review reports, Resume and Model evidence. It then prints one JSON line with the final message, log paths, HEAD before and after, and elapsed time. A small JavaScript provider spawns it and stops it on cancel. An Arm is a thirdshift build plus a Harness, Model and Effort.
- (b) **The same command, public and documented,** such as `thirdshift session`.

*Recommended:* (a). It exists for the Eval harness, as `--release-summary` exists for releases. It can go public later if #706's `thirdshift review` wants it.

*Answer:* agre

*Checked (2026-10-19), against the Console:* a Trial must stay out of the Command records. #776 works out each stage's Usual range from the last 30 Command records in a repository, and its Inbox lists every Command a machine's records hold. A Trial that wrote a record would skew the Usual ranges and appear as a Run. So the flag writes no Command record, and its `--logs` directory sits outside the factory's logs folder. The cost is that the Console doesn't show Trials while they run; Q3.13 covers where they are seen.

**Q3.9 - What the code-review suite replays.** Q2.3 chose #429's 9 bugs. In production the review runs as sub-agents inside the implement session, so a standalone review differs from what a Run does. The options:
- (a) **A review-only session,** which runs the production code review skill, with the implement session's sub-agent rules, on the merged pull request's diff. It tests the skill, the Harness, the Model and the Effort, but not how the implement session decides to call the review, or what its author's context adds.
- (b) **The whole implement session,** from the pull request's starting point, graded on whether the bug survives into the final diff. This is truest to production, but the session may write different code where the bug never arises, so the catch criterion may not apply. Each Trial is also far longer.
- (c) **(a) now, and (b) as the implement-session suite Q2.3 left for later.**

*Recommended:* (c). (a) is what a review skill change or a Model default change most affects, and its catch criteria already exist.

*Answer:* agree

**Q3.10 - The plan-check suite.** No plan check runs in production. It is #429's proposal, still open, and the Architecture review writes plans without one. A plan-check suite would protect nothing until a plan check is built. The options:
- (a) **Leave it out of the first suites,** and add it when the plan check is built, from #429's 40 graded plans.
- (b) **Keep it now,** to measure Model and Effort changes for a future plan check.

*Recommended:* (a). That leaves code review and Spec review to start with.

*Answer:* Agree. And I want a way to test changes to prompts and models, so for example if I change some wording in the code-review skill, I want to be able to test how that changes the run. And for changing the model too I want to be able to test impact of model changes on the processes run by thirdshift.

*Checked:* noted. The code-review suite (Q3.9 a) tests a wording change to the code review skill on the review itself. Its effect on a whole Run needs the implement-session suite (Q3.9 c), and a Model change touches every kind of session. Q3.20 asks which processes the suites must cover, and in what order.

**Q3.11 - The Spec review's commits.** The production Spec review prompt has the session commit its fixes and update the pull request through `gh`. A Trial's repository has no origin, and `gh` is signed out. The options:
- (a) **Give each Trial a local bare repository as its origin.** Commits and pushes then work as in production, the `gh` update fails harmlessly, and only the review reports are graded.
- (b) **Leave origin out and let the push fail,** grading the reports only. The session may spend effort working around the failure, which a Run never does.
- (c) **Add a prompt variant that stops after the review.** That isn't the production prompt.

*Recommended:* (a). It changes the least about what the session sees.

*Answer:* agree

**Q3.12 - promptfoo's defaults.** Several defaults would undermine Trials or send data off this host:
- **Silent retries.** Up to 3 retries fire when an error mentions "rate limit", "timeout", "502" and similar, and each one is a fresh session.
- **Caching.** Reruns return cached Trials unless caching is off.
- **Concurrency.** It defaults to 4.
- **The default grader.** With no API keys it falls back to the Codex SDK on your Codex login, unless a grader is set.
- **Telemetry.** It is on by default. Even `PROMPTFOO_DISABLE_TELEMETRY=1` sends one "telemetry disabled" event to `r.promptfoo.app`, which the source and its own test confirm.
- **The viewer.** `promptfoo view` listens on every network interface.

The options:
- (a) **A fixed setup for every Experiment:**
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
- **Any Harness, on its subscription.** Yes, by Q3.8's design. promptfoo never talks to a Harness: its provider starts thirdshift, which runs whichever Harness the Arm names, on the same logins a Run uses. OpenCode runs on its API key, as it does in production. A Grader can run the same way, so it can be any Harness too. promptfoo's own Claude and Codex providers go unused.
- **Nothing off the host.** That now holds for whatever framework is chosen; see the Settled list. Only the Harness sessions, Trials and Graders alike, talk to their providers, as Runs do. promptfoo doesn't manage this on its own, even with its four switches off:
  - each `eval` or `view` sends one POST to `r.promptfoo.app` with a persistent user ID and any email it has stored, and no setting stops it;
  - its viewer's page sends analytics from your browser to `a.promptfoo.app` on every load, including the text of what you click, and only a rebuild turns that off;
  - the first model-graded assertion probes Google Cloud's metadata server unless told not to;
  - a config folder that was ever signed in to Promptfoo Cloud sends each eval's providers and prompts there.

  It can still be kept silent. Run it headless, under a guard that refuses connections off the machine, with a config folder of its own and Google's detection off, and never open its viewer. Q4.1 weighs this, and so will the framework choice next round.
- **Rust options.** None fits. TensorZero is archived. Laminar's server is Rust, but its evals are Python or TypeScript, and hosting it yourself needs Docker and ClickHouse. Rig and Goose deleted their eval code. The rest are young crates with one maintainer each. The nearest, Mira, runs CLI programs with trials and arms but has no paired statistics. So in Rust this would be thirdshift's own code. Which framework to use is next round's question, since it depends on Q4.1, Q4.2 and Q4.4.
- **promptfoo's web UI.** It has one. `promptfoo view` is a local page that shows the Arms side by side from promptfoo's own SQLite database, and it has no sign-in. Q3.13 and Q4.1 cover how it relates to the Console.
- **OpenTelemetry.** It fits. Q2.1 and Q3.14–Q3.19 already have thirdshift write OpenTelemetry, since Session traces are OTLP. Q4.14 asks how to write it in Rust: with the OpenTelemetry crates, or directly.

**Q3.13 - Where Experiment results are shown.** promptfoo keeps results in its own SQLite database, shows two Arms as side-by-side columns in its viewer, and exports JSON.

*Updated 2026-10-19 for the Console.* #711 shipped the CLI half, and #776 specifies the Console. Three of the Console's rules bear on this:
- Every page is also a CLI command with `--json` (ADR 0016).
- Nothing is loaded from another site.
- It is served on public addresses behind sign-in.

`promptfoo view` has no sign-in and listens on every interface, so it can't sit behind the Console's address. The options:
- (a) **promptfoo's viewer, on localhost or through an SSH tunnel.** The Console doesn't show Experiments for now.
- (b) **A Experiments page on the Console,** behind a `thirdshift` command with `--json` that reads promptfoo's exported results, as the CLI-first rule requires. This becomes a Ticket under #776 or a follow-up.
- (c) **Both.** The Console gives a summary per Experiment, and promptfoo's viewer gives the Trial-by-Trial detail.

*Recommended:* (a) now, and (c) once Experiments run often enough that you want them on your phone. #776's Tickets are already cut, and a Experiments page would need its CLI command first.

*Answer:* I'm leaning towards c for integration / preferring one UI to use.

*Checked (2026-10-20):* the Console shipped in v0.13.0 today, and #776 and all its Tickets are closed. Recorded as (c). Q4.1 asks whether one UI means the Console shows each Trial in full too.

**Q3.14 - What a Trace holds beyond the conventions.** This follows from Q2.1 (b) and its check. You said dropping some things might be fine, depending on what they are. The options:
- (a) **Conventions, plus `thirdshift.*` attributes, plus each raw Harness event.** The attributes cover the gaps that matter for evals: cost, the cache split, rate limits, permission denials, interrupted or killed work, sub-agent lifecycle, diffs, push and pull-request events, the CLI's setup and the Session prompt, which no Claude log holds today. Each raw Harness event is kept as a span event on the model or tool span it belongs to, so nothing is lost. After a round-trip check shows (b) misses nothing you use, trim to (b).
- (b) **Conventions plus those `thirdshift.*` attributes, dropping the rest.** What goes is mostly low-value: thinking signatures, streaming deltas, progress heartbeats and idempotency keys.
- (c) **Conventions only.**

*Recommended:* (a), trimmed to (b) after the check. Values are written as JSON strings, which every viewer keeps, even Workshop.

*Answer:* agree

**Q3.15 - A Trace when a Run is killed.** *Depends on Q3.19; answer it only if Q3.19 is (b) or (c).* Today each Session log line is on disk as soon as it arrives. A span is written only when it ends, so a killed Run would lose its open session and Run spans.

*Updated 2026-10-19 for the Console:* a span also isn't written while it is open. So during a long tool call, such as a 20-minute build, a spans-only file stops growing, and #815 would mark a working Run Silent. Today Claude's progress events keep the log growing. The options:
- (a) **Write each span as soon as it ends,** and close the open ones with an error status when thirdshift is interrupted. A model call or tool call ends within seconds, so a hard kill loses only the open parents. The children carry their parent IDs, so the parents can be rebuilt. Long tool calls still need a heartbeat to stay clear of Silent.
- (b) **Buffer the raw stream in a temporary file,** convert it to the Trace when the session ends, and convert any leftover on the next start. Nothing is lost, but the Trace isn't live, and the Console's live log and Silent would have to read the buffer.

*Recommended:* Q3.19 (b) settles this as a matter of course: its log records are written as they arrive. Choose here only if you pick Q3.19 (c), and then (a).

*Answer:* agree

**Q3.16 - Where Traces are written.** The OTLP file format is JSON Lines, ideally `.jsonl`, and still Development. Rust has no exporter for it, so thirdshift would write its own. During an Experiment, promptfoo can receive the spans over OTLP and grade on them. Phoenix takes protobuf only. The options:
- (a) **One file per session, where its Session log is today, with all of a command's sessions sharing the command's trace ID.** Spans also go to an OTLP endpoint when one is set: promptfoo during an Experiment, or a viewer later.
- (b) **One file per command.** This holds the whole tree, but a command's child Runs would all write to the same file.

*Recommended:* (a). It keeps today's layout and sort order. The shared trace ID ties the files together, and no two sessions write one file.

*Answer:* agree

**Q3.17 - The terms for Traces.** *If Q3.19 is (a), Session log keeps its name, and Trace is the new file beside it.* This settles #700's rename. ADR-0011 established the Command log and the Activity log on purpose, and they aren't Harness streams, so they stay logs. The options:
- (a) **Trace** for one command's whole tree, its sessions and their model and tool calls. **Session trace** replaces Session log, for one session's part, kept in its own file. Anthropic's "Transcript" isn't added: the article treats it as a synonym of trace.
- (b) **Keep the name Session log** for the file, though it holds OTLP now.

*Recommended:* (a), with an ADR. Replacing Session logs with OpenTelemetry traces is hard to reverse and surprising without context, and it came out of a real trade-off. If you agree, I'll draft the ADR.

*Answer:* agree

**Q3.18 - Traces before or after the first Experiment.** Traces touch every production session, while the Experiment work is mostly new code beside them. thirdshift records no tokens or cost today, so an Experiment's efficiency numbers would come from Traces, or from parsing each Harness's Session log. The options:
- (a) **The first Experiment comes first.** It grades accuracy and reports minutes only. Tokens and cost come once Traces land, with no throwaway parser.
- (b) **Traces come first,** so the first Experiment has tokens, cost and trajectory assertions from the start.
- (c) **Both at once.**

*Recommended:* (a). Q1.1 put accuracy first, and the first Experiment doesn't wait on a change to every session.

*Updated 2026-10-19 for the Console:* the order that matters more is Traces against #819 and #827, not against the first Experiment. If Q3.19 is (b), building #819's Claude and Codex renderers, and #827's Command-log matcher, on today's Session logs means building both twice. The recommendation stays (a) for the Experiment. Q3.19 asks about the Console's order.

*Answer:* Agree - I don't care either way which come first here as long as we get both as part of this spec (or specs if the scope needs multiple)

**Q3.19 - Session logs, Traces and the Console.** This reopens Q2.1 because of new facts. The Console needs a file that grows as the session streams: for Silent (#815), for the live conversation sent by byte offset (#819), and for `show`'s latest event. A spans-only Trace grows only when a span ends. OpenTelemetry has a second signal that fits: **log records**, each tied to its trace and span, and written the moment they happen. OpenTelemetry's events are log records with an event name. The options:
- (a) **Keep Session logs as they are, and write a Trace beside each one** (Q2.1's original (a)). The Console's Tickets go ahead unchanged. #827 can read real span times from the Trace instead of matching Command log lines.
- (b) **Replace Session logs with a Session trace of OTLP log records plus spans.** Each Harness event becomes a log record as it arrives, with `thirdshift.*` attributes and the raw event, so nothing is lost and the file grows live. Spans are written as they end. Two Console Tickets change before they're built:
  - #819 renders the log records, which works the same for all six Harnesses, where #776 covers only Claude and Codex;
  - #827 reads span times, so it needs no Command-log matcher.

  #815's file-growth rule still works. Traces then come before #819 and #827.
- (c) **Replace Session logs with spans only,** as Q3.15 assumed, with a heartbeat span for long tool calls. The Console reads span events, and the 128-events-per-span limit has to be raised.

*Recommended:* (b). It keeps your Q2.1 choice, of one standard format and nothing lost. It also makes the Console better: six Harnesses rendered rather than two, and real times for the Waterfall. The cost is ordering: #819 and #827 wait for the Session trace, while #814–#818, #820–#826, #828 and #829 go ahead. Two checks are needed:
- whether Phoenix, MLflow and Workshop show OTLP log records, or only spans;
- whether `show`'s latest-event parser can read them.

*Answer:* agree, no problem on the ordering waiting if it makes a better system. 

*Checked (2026-10-20):* #819 and #827 shipped in v0.13.0 today, so (b) now means reworking them rather than waiting for them. The rework is small:
- the conversation renderer, `src/console/conversation.rs` on `main`, 174 lines, which reads Claude's and Codex's events;
- the Waterfall's Command-log matcher, `src/logs/status/waterfall/matching.rs`, 188 lines, which the records' own times replace;
- `show`'s latest event, in `src/logs/reading/selection.rs`.

Each log record keeps its raw Harness event (Q3.14), so the renderer can first unwrap the record and keep its Claude and Codex code, then grow to all six Harnesses. #815's Silent rule reads file growth, so it doesn't change. Recorded as (b); shout if the rework changes your answer.

**Q3.20 - Which processes the suites cover, and in what order.** This builds on your Q3.10 note: a wording change to a skill, or a Model change, should be tested on what thirdshift actually runs. Q2.3 and Q3.9 set the first suites: a review-only code review, and the Spec review. The kinds of session thirdshift starts are:
- the implement session, with its review sub-agents;
- Repairs and Resumes;
- the Base fix;
- the Spec review;
- the Architecture review;
- the Security review and the Security fix.

The release summary also calls a Harness, but it isn't a session.

The options:
- (a) **Code review and Spec review first. Then the implement session (Q3.9 c), graded against the merged fix and its tests, SWE-bench style. Then the Architecture review, once its plans have graded Tasks.** A Model change runs every suite that exists, and Security stays separate under ADR-0015.
- (b) **The implement session first,** since it is what a Run mostly is, before widening the review suites.
- (c) **Everything at once.**

*Recommended:* (a). The first two suites have Tasks with right answers today, while the implement suite needs grading criteria written for each Task.

*Answer:*  agree
