# Round 4 (answered)

Part of the [#698 grilling](README.md).

**Q4.1 - How much of an Experiment the Console shows.** This follows from Q3.13, where you leaned to (c) and said you prefer one UI. With Session traces (Q3.19), a Trial writes the same format as a Run, so the Console can show a Trial's session with the code it already uses for a Run's. The options:
- (a) **The Console shows Experiments in full.** A Experiments page lists each Experiment with its verdict against its bar. Each Experiment's own page shows every Eval task by Arm, each Trial's Scores with the Grader's reasons, and each Trial's Session trace. Every page has its CLI command with `--json` (ADR 0016), which reads the Experiments folder, outside the factory's logs (Q3.8). promptfoo's viewer, if promptfoo stays, is left for debugging promptfoo itself.
- (b) **The Console shows a summary,** each Experiment's verdict and its numbers by suite, and promptfoo's viewer shows the Trial-by-Trial detail. This is Q3.13 (c) as written. The viewer sends analytics from your browser to `a.promptfoo.app`, including the text of what you click, so the browser would have to block that host.
- (c) **A summary now, and the detail later.**

*Recommended:* (a). It's the one UI you asked for, it keeps Trials out of promptfoo's analytics, and it doesn't tie an Experiment's detail to promptfoo, which the framework choice next round may drop. The `thirdshift` host's Console would show the private Eval tasks behind its sign-in, as it already shows Security sessions.

*Answer:* after using the console web ui for the first time this afternoon, I have a better sense of the challenge of making good UI, so I'm open to using a UI from a vendor / library / open source project. However, I'm also open a here, or possibly even both, unless that's too redundant perhaps or then too much long term maintenance.

*Checked (2026-10-21):* not settled yet. Research is comparing the Console with MLflow, Phoenix, Workshop, Grafana with Tempo and Loki, Langfuse, promptfoo's viewer and Inspect's, for traces and for Experiments. It checks what each shows, what it sends off the host, its sign-in and memory, and whether the Console could link straight into it. Q4.1 comes back with Q4.12 when that reports.

**Q4.2 - How an Experiment is started.** Several settled answers need one place that enforces them: the bar is fixed before an Experiment starts (Q3.4), each Eval task's throwaway repository is built and checked (Q3.6), promptfoo's settings apply every time (Q3.12), and a CLI command must list Experiments for the Console (Q4.1). The options:
- (a) **A `thirdshift experiment` command.** It takes the suites (Q3.3), the Candidate (Q4.3) and the bar. It records the bar with the Experiment before the first Trial, builds both Arms, prepares the Eval tasks, runs the Trials and Graders, applies the bar and writes the result. `thirdshift experiments` lists them, with `--json` for the Console. promptfoo, if it stays, runs underneath with Q3.12's settings.
- (b) **Run promptfoo by hand,** with a config kept in the repository. thirdshift only reads the results.
- (c) **A script beside the config,** as #429's `replay.py` was.

*Recommended:* (a). A bar the command has recorded can't change after the first Trial, and no setting can be forgotten.

*Answer:* I'm torn between a and b, as I like the idea of having evals built into thirdshift, but I'm wary to rebuild too much without a good enough reason. Perhaps it would be sufficient to have promptfoo side by side with it, but then perhaps there's benefit from more integration or something.

*Checked (2026-10-21):* not settled yet. Either way, someone writes the steps no framework knows about: building both Arms from their commits, preparing and checking each Eval task's repository, and bwrap. (a) writes them in Rust inside thirdshift, and (b) in promptfoo's hooks and scripts beside the config. Research is checking what promptfoo, Inspect and MLflow would carry themselves, Q4.4's verdict included. Q4.2 comes back with the framework choice.

**Q4.3 - What the Baseline arm is built from.** Q2.6 runs the Baseline fresh beside the Candidate, and Q3.8 makes an Arm a thirdshift build plus a Harness, Model and Effort. Skills and Session prompts are built into the binary. The options:
- (a) **The point where the Candidate's branch left `main`.** Both Arms are built the same way, the Candidate from its branch and the Baseline from that point, so the change is the only difference between them. A change to a Harness, Model or Effort default that needs no code uses one build for both Arms, with different flags.
- (b) **The installed release,** which is what production runs. Anything merged to `main` since that release then counts as part of the change.
- (c) **`main` as it is when the Experiment starts.** Anything merged since the branch left `main` then counts against the change.

*Recommended:* (a).

*Answer:* agree

**Q4.4 - How "worse" is measured.** Q3.4 fails the Candidate when it is worse than the Baseline by more than "the spread between the Baseline's own Trials". That needs a method. Anthropic's "Adding Error Bars to Evals" (Miller, 2024) compares two models question by question, and works out in advance how many questions a difference of a given size needs. By its formulas and example variances, 9 Eval tasks of 3 Trials each can reliably detect only a difference of about 44 points. 20 points would need about 44 Eval tasks, and 10 points about 175. Bowyer et al. (ICML 2025) add that the usual normal-approximation intervals understate the uncertainty below a few hundred items, and recommend Bayesian intervals for paired comparisons that small. The options:
- (a) **Paired by Eval task, with a Bayesian interval and three verdicts.** For each Eval task, take the Candidate's share of passing Trials minus the Baseline's, and give the mean a 95% Bayesian interval, as Bowyer et al. recommend.
  - The Candidate **fails** if the whole interval is below zero.
  - It **passes** if the whole interval is above a margin you fix with the bar, such as 10 points worse.
  - Anything else is **inconclusive**, and the report says how many more Eval tasks would settle it.

  Wrong findings are judged the same way. For the code-review suite, the report also gives pass^3, the share of Eval tasks where all three Trials pass, as a measure of consistency.
- (b) **Fail only when clearly worse.** As (a), but anything not clearly worse passes, so a real regression can hide in a small suite.
- (c) **Q3.4's words as they stand.** The Candidate fails if its total is below the lowest of the Baseline's three totals, one per repeat. It's simple, but it ignores the Candidate's own luck, and with 9 Eval tasks a single Trial can decide it.

*Recommended:* (a). It never reports a pass the data can't support. Expect most Experiments on 9 Eval tasks to come out inconclusive unless a change is large. That is the honest result, and the reason for Q4.9. The first Experiments' own spread will replace the example variances.

*Answer:* agree.

**Q4.5 - What Run metrics record.** Run metrics open now that grading is settled (Q3.5). A Run's metrics attach to its Command record, and its trends go to #813's charts (Settled). The options:
- (a) **Everything the Run and its Session traces show, as separate signals and never one number:**
  - from the Run: its outcome, Repairs, CI rounds, Unaddressed findings, the review's findings by kind, and each stage's time against its Usual range;
  - from its Session traces: tokens by Model, cost where the Harness reports one, tool errors, permission denials, killed work, repeated tool calls, and destructive commands a code Grader can spot, such as deleting files outside the worktree.

  thirdshift keeps no price table, so a Harness that reports no cost shows tokens only. A model Score is Q4.6's question.
- (b) **Only what the Run knows,** leaving out the Session-trace signals.
- (c) **(a), plus a price table,** so that every Harness gets a cost-equivalent.

*Recommended:* (a). A price table goes stale, and cost on a subscription is notional. Tokens and minutes compare well within one Model.

*Answer:* agree

**Q4.6 - Grading production Runs.** Q1.1 asked for production to be analyzed for accuracy and quality, and a Run's own facts measure friction more than quality. From 3 to 16 October the factory ended about 140 Commands a week. The options:
- (a) **A model Grader grades a random sample of finished Runs,** about 10 a week, spread across the Harnesses and Models in use. It sees the issue, the merged diff and the pull request, but not the Harness, Model or Effort, and it grades against Q4.7's rubric. Its Model differs from the Run's (Q3.5). You grade some of the same Runs, and its Scores show as uncalibrated until they agree with yours. It runs on your subscriptions and stops at a plan-limit warning, as Experiments do.
- (b) **Score every Run,** each in a session of its own.
- (c) **No model Scores in production.** Rely on Q4.5's signals and Q4.8's bugs.

*Recommended:* (a). Production Scores watch for drift, and Experiments decide changes, because production Runs work different issues and can't be compared like for like.

*Answer:* agree - like a Michelin star chef with multiple restaurants may come in and sample dishes from his restaurants, I want the model Grader to grade the sample of finished Runs.

*Checked (2026-10-21):* the chef has two restaurants. Both hosts make Runs: the `thirdshift` host on seven repositories and `stephens` on six, sharing only clave. Q5.1 asks where the tasting happens.

**Q4.7 - What a production Score judges.** Q1.1 put accuracy and quality first, and asked what else evals should consider. The options:
- (a) **Five things, each graded and shown on its own:**
  - **Correct:** the diff does what the issue asked, with no bug the Grader can name.
  - **Scoped:** it changes nothing the issue didn't ask for.
  - **Tested:** tests cover the change, and none was weakened to make it pass.
  - **Honest:** the pull request and the final message claim only what the diff and CI show.
  - **Standards:** it follows the repository's coding standards and reads like the code around it.
- (b) **Correct and Tested only.**
- (c) **Correct only.**

*Recommended:* (a). Safety stays with Q4.5's code Graders, which check every Run rather than a sample.

*Answer:* agree

*Checked (2026-10-21):* Honest's "final message" isn't on GitHub. Delivery runs the implement session through `Sessions::run`, which drops the final message (`src/delivery.rs:531`, `src/session.rs:207-216`), so it survives only in the Session log, on the host that ran the Run. Q5.1 asks whether Honest then judges the pull request only.

**Q4.8 - Tracing bugs back to the Runs that caused them.** Q1.1 (b) counted what became of a Run's pull request: reverted, or a bug traced to it. thirdshift has never had a revert, but 35 of its issues opened since 3 October are labelled `bug`. Nothing links a bug to the pull request that caused it, yet those links are the nearest thing production has to a right answer. The options:
- (a) **A short session proposes each link, and you confirm it.** When a Run on a `bug` issue merges, a session reads the fix and the history and names the commit that introduced the bug, if the history shows one. thirdshift maps that commit to its pull request, and to that pull request's Command record if a Run made it. Once you confirm or correct the link from the CLI, it counts in that Run's metrics and feeds Q4.9. Confirming from the Console as well would amend ADR 0016, which limits the Console's actions to Stop and its own look.
- (b) **You record the links by hand,** for example with a line on the bug's issue that names the pull request.
- (c) **No links:** reverts only.

*Recommended:* (a). It leaves the implement session's prompt alone, so it needs no Experiment of its own, and nothing counts until you confirm it.

*Updated 2026-10-20:* #919, filed today, asks for a way to dismiss or acknowledge what **Needs you** lists. That would also be a new Console action, and so an amendment to ADR 0016. If it goes ahead, confirming links from the Console could join the same amendment.

*Answer:* agree

*Checked (2026-10-21):* every factory pull request ends with a line naming its Harness, Model and Effort, such as "Built with claude · claude-opus-5-5 · medium". So once a bug is linked to a pull request, GitHub alone says what built it, whichever host ran it.

**Q4.9 - New Eval tasks from bugs found in production.** Under Q4.4, 9 Eval tasks will rarely give a clear verdict. #429's Eval tasks are bugs that reviews missed, and each link confirmed under Q4.8 is another: a bug the code review let through. The options:
- (a) **Each confirmed link proposes an Eval task** for the code-review suite: the pull request's diff as it was reviewed, and a draft catch criterion. It joins the suite once you approve it, and it stays private under `~/.thirdshift/replays/` (Q2.5).
- (b) **Eval tasks come only from deliberate studies,** as #429's did.
- (c) **(a), plus Spec findings you judge real or wrong in production,** for the Spec-review suite.

*Recommended:* (a). It grows the suite from the failures that matter most. Spec findings can follow once there's a way to record your judgment of them.

*Answer:* agree

**Q4.10 - The command and its Runs in a Trace.** Q3.17 made a Trace one command's whole tree, and Q3.16 put each session's part in a Session trace of its own. The levels above the sessions need a home too: the command, its Runs and their stages. The Command record already holds each Run's stages and when each began, and the Waterfall draws those levels from it. The options:
- (a) **Derive them from the Command record.** Each Session trace carries IDs computed from its Command record, so a whole Trace can be put together without guessing. The Console does that, and an export command writes the whole Trace as OTLP for any other tool.
- (b) **Write them as spans in one more file beside the Command record,** so the files on disk make a whole standard Trace by themselves. Stage times are then kept twice, in the record and in the file.
- (c) **Copy the command's and Run's spans into each Session trace.**

*Recommended:* (a). The Command record stays the one place those times are kept, so nothing has to be kept in step.

*Answer:* agree

**Q4.11 - Session logs from before the switch.** There are 924 Session logs from production, not counting Security sessions, and 215 from #429's replays. The Console reads them today. The options:
- (a) **Convert them once into Session traces,** with the same code that writes Session traces live, and keep the originals until the conversion is checked. Codex's lines carry no time and only some of Claude's do, so a record without a time gets one estimated from the Command log, as the Waterfall does now, and marked as estimated. The Console then needs one renderer, and Run metrics and #813's charts start with history.
- (b) **Leave them as they are.** The Console keeps its Claude and Codex code for them, and the Session-trace signals start at the switch.
- (c) **Convert them on demand,** when a chart or a Run's metrics need the history.

*Recommended:* (a). The conversion also serves as Q3.14's round-trip check, over every log there is. Security sessions' logs are converted on the machine like the rest.

*Answer:* agree

**Q4.12 - The Console as the trace viewer.** Q2.2 kept a local OpenTelemetry viewer as the fallback, and Q3.19 left a check open on whether Phoenix, MLflow and Workshop show log records. With Session traces, the Console shows any session from any Harness. The options:
- (a) **The Console is the trace viewer,** with no other viewer for now. Session traces are standard, so one can be added later without changing them, and the check waits until then.
- (b) **Also run Phoenix or MLflow on the `thirdshift` host,** for searching and comparing spans across Runs, which the Console doesn't do. This needs the check first, and memory on an 8 GB host.

*Recommended:* (a). It's the one UI you prefer, and #813's charts will cover trends across Runs.

*Updated 2026-10-20:* #914, filed today, asks whether thirdshift should feed Grafana's stack: Tempo for traces, Loki for logs, Prometheus or Mimir for metrics, and Alertmanager. Session traces would already suit Tempo, which takes OTLP spans, and Loki, which takes OTLP log records natively. So (b) could mean Grafana with Tempo and Loki rather than Phoenix or MLflow. That settles whether log records show, but not the memory. Metrics and alerts stay #914's own question.

*Answer:* I'm torn on this one. I want to consider it more. If the UI of Phoenix, MLFlow, Workshop, Grafana, or anything else considered here has strong advantages over the hand rolled Console as the trace viewer, I'm interested, or perhaps inspiration can be taken from these tools' viewers into the Console. I'm open to multiple UIs with sufficient benefit from that setup. The console perhaps could link to the other UI, so that moving between them doesn't require typing a different URL. Is this raindrop Workshop? Or something else? Or a Braintrust thing? Or does Langfuse become preferable? MLflow looks like it has potential. How does LangChain fit in here? or LangSmith? /research more on this if useful, but we may already have enough info on this. I'm even wondering if a /handoff to a /prototype ui session with another agent is warranted here. Or to use promptfoo's viewer? Or maybe it wouldn't be too hard to replicate these into thirdshift Console. 

*Checked (2026-10-21):* not settled yet. Research is running (see Waiting on research). What's already known:
- **Workshop** is Raindrop Workshop, a local trace viewer. It's MIT, one binary on SQLite, and takes OTLP over HTTP. It is separate from Raindrop's hosted product, and has nothing to do with Braintrust.
- **Braintrust** and **LangSmith** are proprietary hosted platforms. LangSmith runs on your own machines only as an Enterprise deployment on Kubernetes, and Braintrust's self-hosted data plane is too large for these hosts, so Q2.2 keeps both out.
- **LangChain** is a library for building LLM applications. thirdshift drives the Harness CLIs instead, so it has no place here. LangSmith is the same company's platform.
- **Langfuse** still needs Docker and about 16 GiB. `stephens` has Docker, 4 cores and 15.6 GiB, but it makes Runs too and already has 2.7 GiB in swap, so Langfuse would be tight there. The research checks its current minimum.
- **promptfoo's viewer** still sends analytics from your browser. The research checks whether a Content-Security-Policy that Caddy adds could stop that in every browser.
- **Linking from the Console** depends on each tool's URLs. The research looks for ones that open a given trace or Experiment.
- **A /prototype hand-off** fits once the research narrows the field, so the UI question will offer it.

**Q4.13 - How this becomes Specs.** Q3.18 keeps Experiments and Session traces both in this work, in one Spec or several. They touch different code. Session traces change every session, the Console's log view and Waterfall, and `show`. Experiments add new commands and the Eval harness. Run metrics need both. The options:
- (a) **Three Specs, for Session traces, Experiments and Run metrics,** each with its own Tickets, and all linked from #698. Session traces and Experiments can be built in parallel, and Run metrics follow both.
- (b) **One Spec for all of it.**
- (c) **Two Specs:** Session traces, and then Experiments with Run metrics.

*Recommended:* (a). Each can be reviewed and merged on its own, and the factory can work on the first two at once.

*Updated 2026-10-20:* since #921, merged into `main` today, a Pickup run passes over a Spec with no **Startable Ticket**, where before its Spec run would have failed. So all three Specs can be published at once: while the Run-metrics Spec's Tickets are blocked by the other two Specs' Tickets, the factory leaves it alone.

*Answer:* Agred, and we'll use the to-spec and to-tickets skills for these. 

*Checked (2026-10-21):* `to-spec` and `to-tickets` are among your skills. `docs/agents/triage-labels.md` overrides how they publish here: each Spec goes out `needs-triage`, and turns `ready-for-agent` only once all its Tickets, sub-issue links and blocking links are in place.

**Q4.14 - How thirdshift writes OpenTelemetry.** You asked in Q3.12 how well OpenTelemetry fits thirdshift. Q2.1 and Q3.14–Q3.19 already make Session traces OTLP, so this is about the Rust code that writes them. The OpenTelemetry Rust crates are at 0.33.1: logs are Stable, and traces are Beta. No crate writes the OTLP file format, which is itself still Development, and `opentelemetry-stdout` prints text for people, not OTLP. ADR 0016 keeps async code inside the Console, and session recording is blocking. The options:
- (a) **Write OTLP JSON directly,** with the serde thirdshift already uses. That adds no crates. It must follow four encoding rules: IDs in hex, enums as numbers, camelCase keys, and 64-bit numbers as strings. A test checks the output against the examples in OTLP's JSON specification. Records go to an OTLP endpoint, when one is set, through the reqwest client thirdshift already has.
- (b) **The OpenTelemetry SDK, writing the file through the official `opentelemetry-proto` types,** as Heroku's libcnb.rs does. That adds 17 crates, prost among them, and leaves the encoding to the official types. Those still needed JSON fixes as recently as 3 October, so the same test is worth having.
- (c) **(b), plus `opentelemetry-otlp` for the endpoint.** That makes 20 crates. It only sends to an endpoint, so the file is still written as in (b).

*Recommended:* (a). No crate writes the file, so thirdshift writes it in every option, and only (a) does so in one code path, with no new dependencies. The four rules are fixed by the specification, and the test holds the code to them.

*Answer:* If there's a gap here I'm open to either opensourcing our own Rust project for this potentially or contributing ot the OpenTelemetry Rust crate, or perhaps just using something else already existing. I wonder if it is worth separating this from thirdshift so thirdshift can use the separate crate we create and then others can use it too - who want Rust OTLP file format writing without wanting thirdshift. Or perhaps option a would be sufficient and not too handrolling in a bad way - sounds somewhat necessary, like there's not a great Rust option already.

*Checked (2026-10-21):* not settled yet. Research is checking three things:
- what opentelemetry-rust would accept, and when, since its file-exporter issue (#2602) is waiting on the specification;
- whether a crate outside the OpenTelemetry project already writes the format;
- what a crate of your own would take to publish and keep up.

Q4.14 comes back with it.
