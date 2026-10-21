# Round 5 (answered)

Part of the [#698 grilling](README.md).

*Q5.1–Q5.4 don't depend on the research. It has since reported, so Q5.5–Q5.10 bring back Q4.1, Q4.2, Q4.12 and Q4.14, with the framework choice and three questions the research raised.*

**Q5.1 - Where production grading runs.** Q4.6 grades a sample of about 10 finished Runs a week, and Q4.8 links bugs to the pull requests that introduced them. Both hosts make Runs, and each host's Command records stay on that host. But Q4.6's Grader judges the result, not the session. It sees the issue, the merged diff and the pull request, with the repository and its CI results for context, and all of those are on GitHub. Every factory pull request also names its Harness, Model and Effort (Q4.8's check). The options:
- (a) **A weekly Grading run on the `thirdshift` host, covering both hosts.** It is a new kind of Command: cron starts it, and the Console shows it and can stop it, like any other.
  - It samples the week's merged factory pull requests in every repository either host works on, spread across Harnesses and Models, and grades them.
  - Honest (Q4.7) judges the pull request's claims only. The Run's final message stays in its Session trace, on the host that ran it.
  - It proposes a link for each `bug` issue closed that week, whoever fixed it. Its first run works back through the 59 closed so far.
  - It keeps Grades and links by pull request on the `thirdshift` host, beside Replays. The signals a Run score takes from Session traces stay on the host that ran it. So the `thirdshift` host's charts show Grades and links for both hosts, but tokens and the other Session-trace signals only for its own Runs.
- (b) **Each host grades and links its own Runs,** from its own Command records. A clave bug fixed on one host but caused by the other host's pull request can't be linked to its Run.
- (c) **A command you run by hand** whenever you want a sample.

*Recommended:* (a). One place sees all the factory's work with nothing copied between hosts, and the evals stay together on the host that runs Replays. The Grading run would join Pickup runs and Architect runs among the Commands. If you want the chef in the name, it could be a **Tasting** instead.

*Answer:* Agree, and I think I want to prefer industry standardized language here, so probably Replays if that is more industry standard, but if there's not really a standard for this yet, I'm open to considering Tasting, but ultimately probably a term that is most self-descriptive for lower cognitive load and clarity of what is being described over something more metaphorical, unless the metaphor actually decrease the cognitive load - and Tasting might actually do that here - needs more thought / research possibly.

*Checked (2026-10-21):* in chat you named it the **Scoring run**. What it does is what LangSmith calls online evaluation and Braintrust calls online scoring, and the name says so without a metaphor (#943).

**Q5.2 - Names for Round 4's ideas.** Three things need words before the Specs are written:
- `GLOSSARY.md` says a **Grade** and a **Grader** judge a Trial against its Eval task. Q4.6 has them judge a sampled Run against Q4.7's five things as well.
- **Run score** clashes with the glossary, which tells agents not to call a Grade a "score". And Q4.5 settled that a Run score is never one number, which "score" suggests.
- The link from a bug to the pull request that introduced it, and to the Run that made it, has no name. Defect research calls such a change "bug-introducing".

The options:
- (a) **Widen Grade and Grader** to cover a sampled Run, **rename Run score to Run measures**, and call the link a **Bug origin**: the pull request that introduced a bug, and the Run that made it if one did, as you confirmed it.
- (b) **As (a), but keep Run score,** and take "score" off Grade's _Avoid_ list.
- (c) **A separate term for a Run's Grade,** such as Run grade, leaving Grade to Trials.

*Recommended:* (a). Once you agree, they go into `GLOSSARY.md` through a pull request to `main`, as #920 did, with the Grading run if Q5.1 is (a).

*Answer:* Is the glossary right to not call a grade a score? Again wanting to adopt industry standard language here. But in case that's all right, then I agree with the recommendation.

*Checked (2026-10-21):* the glossary was wrong to avoid "score". Anthropic's agent-eval article has graders produce scores ("combined grader scores must hit a threshold"), and Inspect, Langfuse and LangSmith call the result a score. So in chat you chose:
- **Score** in place of Grade;
- **Run metrics** in place of Run score, since the tools call tokens, latency and cost metrics;
- **Bug origin**, as proposed.

Grader stays, as Anthropic's word (#943).

**Q5.3 - What an Experiment's verdict does.** Q1.1 wants to know whether a change makes the agents worse before it reaches production. But both hosts' User configs make every Run a Merge run (`merge.always`), so a Run that changes a Factory skill, a Session prompt, or a Harness, Model or Effort default Self-merges like any other, with no Experiment. Q4.4's margin also decides how often an Experiment can pass: the narrower it is, the more Eval tasks a pass needs. The options:
- (a) **Such a Run leaves its pull request for review instead of Self-merging,** with a note naming the Eval suites its change touches (Q3.3). You run the Experiment and decide. A pass or a fail is clear, and an inconclusive verdict is your call, with the interval in front of you. The standing margin is 10 points.
- (b) **As (a), with a 20-point margin,** so more Experiments pass on today's small suites, at the cost of passing a change that may be up to 20 points worse.
- (c) **Experiments stay advisory.** Runs Self-merge as now, and you run an Experiment when you choose.

*Recommended:* (a). Nothing unmeasured reaches production, and no Run waits hours on an Experiment it can't start. With 9 Eval tasks most verdicts will be inconclusive at either margin, so your judgment carries them until the suite grows (Q4.9).

*Answer:* I'm thinking a, except to have an additional user config specifically for self-merging replay verdict findings. so then a user could control through their user config the merging behavior for different kinds of runs. So I want them to be able to set for an architect run. Do they want self-merge by default for a spec run, for a security run? and for this kind of evaluation run as well, I think.

*Checked (2026-10-21):* I read this as (a), plus User config settings that say whether each kind of Run Self-merges: the Runs an Architect run or a Security run starts, Spec PRs, and Runs whose change an Experiment would judge. Two facts the question missed bring it back as Q6.1 and Q6.2:
- **Production doesn't run `main`.** Every Run runs the installed binary, its Factory skills and Session prompts included (ADR 0001), and only `thirdshift update` replaces it, with the latest release. So a merged change reaches production when you release it. Since 1 October, `main` has taken 230 merges, 16 of which changed a Factory skill or a Session prompt, and 26 releases have gone out, 10 of them carrying such a change.
- **Each kind of Pass can already choose.** `merge` or `no-merge` on a Pass's crontab line overrides `merge.always` for everything that Pass starts.

**Q5.4 - How your Scores calibrate the Grader.** Q4.6's model Scores show as uncalibrated until they agree with yours. That needs a scale, a number of your own Scores, and a rule. The options:
- (a) **Pass or fail on each of Q4.7's five things, with a reason.**
  - You grade 20 past Runs first, without seeing the model's Scores, and then 2 from each week's sample.
  - A criterion's model Scores count as calibrated while they match yours on at least 18 of the last 20 Runs you both graded, and show as uncalibrated otherwise.
  - You grade from the CLI for now. Where else you can grade follows from Q5.5.
- (b) **As (a), without the first 20,** so calibration builds at 2 a week and takes about ten weeks.
- (c) **No rule.** The agreement is shown, and you judge it.

*Recommended:* (a). Pass or fail is the easiest Score to compare with yours, and twenty Runs up front make the model's Scores usable in days rather than months. A criterion that never calibrates, perhaps Standards, stays marked without holding up the others.

*Answer:* agree

**Q5.5 - Where Experiments and traces are shown.** This brings back Q4.1 and Q4.12. You're open to an existing UI, to the Console, or to both, as long as that isn't too redundant or too much upkeep.

*The terms:*
- A **span** is one timed step with a start and an end, such as a session, a model call or a tool call. It is written when it ends.
- A **log record** is one moment inside a span, such as a message or a tool's output, written as it arrives.
- A **Trace** is all the spans and records of one Command, joined by their IDs into a tree.

The Console's Waterfall already draws spans, and the conversation in its log view is log records.

The viewer research (`docs/research/eval-viewers.md`) found:
- **Every viewer builds the trace from its spans, but most never see the log records.** MLflow, Phoenix, Langfuse and Raindrop Workshop accept spans only. They get the tree, with its times and tokens, but not the content the log records hold, such as what the agent said and each tool's output. They get that only if the export command copies it onto the spans (Q4.10). Grafana keeps the records in Loki, but renders nothing as GenAI content.
- **No viewer computes an Experiment's verdict,** so thirdshift computes Q4.4's interval whichever one shows it. Phoenix's compare view comes closest to an Experiment page, and MLflow pairs Trials only by order.
- **MLflow fits best for traces across Runs.** It offers search, a timeline, cost, two or three traces side by side, and Scores with reasons, in one Python process on SQLite, and two settings silence it. Its memory beside the factory is unmeasured; the one report is 1.8 GiB idle.
- **The rest fall away.**
  - Grafana is too big for the `thirdshift` host, and Langfuse for `stephens`.
  - Raindrop Workshop drops span events, calls raindrop.sh from its page, and can start agents.
  - Phoenix takes OTLP only as protobuf and is ELv2.
  - promptfoo's and Inspect's viewers show Experiments, not traces.
- **A second viewer needs a second sign-in.** The Console's cookie goes only to `console.thirdshift.app`. A viewer elsewhere needs its own password, and one mounted inside the Console would run its scripts with the Console's rights.
- **Matching the viewers in the Console is a lot of code.** A span tree, span details, search, two traces compared and the Experiment pages come to about 2,100–4,000 lines, the size of the Console's first Spec again. Without search and comparison, it's about 1,400–2,700.

The options:
- (a) **The Console shows Experiments in full and every trace.**
  - The span tree grows out of the Waterfall during the Session-trace rework (Q3.19), and span details go in the Peek panel.
  - A /prototype session comes first and designs those two pages, borrowing from Phoenix's compare view and MLflow's trace view.
  - No second viewer for now. If you later want search across Runs beyond #813's charts, MLflow is trialled behind its own sign-in, linked from the Console by trace ID, after a memory check.
- (b) **As (a), with MLflow now** for traces across Runs, on the `thirdshift` host behind its own sign-in, with telemetry off. It is fed by the export command (Q4.10), which copies each log record onto its span, since MLflow takes spans only.
- (c) **Experiments in the Console, traces in MLflow only.** The Console links each Trial and Run to its trace there.

*Recommended:* (a). Every viewer leaves the verdict to thirdshift, so the Console draws Experiments either way. Only the Console shows a session live, since a spans-only viewer sees each step only once it ends. MLflow's extras, such as search across Runs, a timeline and traces side by side, would cost memory on an 8 GB host, a second sign-in and frequent upgrades, so I'd add it once you find you need them. A prototype first answers your point that good UI is hard, and a viewer added later changes nothing in the Session traces.

*Answer:* I can resize hosts for a bigger tool if we need more computer or ram or something, such as for Grafana or Langfuse (unless by too big you meant too big in functionality / feature set or something).

I'm okay with a second viewer needing a second sign-in. I'm thinking about Fred Brooks' No Silver Bullet ideas here about buying not building, and thinking about when to build.

Ultimately though I'm okay to go with the recommendation here.

*Checked (2026-10-21):* "too big" meant memory and disk, not features. Tempo asks for 16 GB with Grafana beside it, and Langfuse for 4 cores, 16 GiB and about 100 GiB of disk. A resized host would fit either, and the choice would then turn on what each one shows and what it costs to keep up. That leaves (a) as it was, since neither computes a verdict or shows a session live. On Brooks: (a) builds only what no product offers, the verdict with its Experiment pages and a live view of each session, whose span tree grows out of the Waterfall. Search and comparison across Runs can be bought later, from MLflow or another viewer, with no change to the Session traces, since they're standard OTLP.

**Q5.6 - How Experiments run.** This brings back Q4.2 and the framework choice. The runner research (`docs/research/eval-runners.md`) found:
- **The code that decides accuracy is thirdshift's in every option.** That covers the Trial flag, the plan-limit stop, Eval task repositories, Harness memory, bwrap, the Arm builds, the blind packets, Q4.4's interval, the bar and the `--json` listing. It comes to about 1,400–2,300 lines, mostly Rust.
- **A framework replaces only the middle:** expanding Trials, running two at a time with a timeout, storing results, resuming, and a viewer. That's about 300–550 lines in Rust, and each framework's glue costs about as much. So every option comes to roughly 1,500–3,100 lines.
- **Only a command can enforce** the bar, Q3.12's settings and the Baseline's commit. By hand, each depends on someone remembering.

The options:
- (a) **`thirdshift experiment`, running Trials itself in Rust.** There are no framework defaults to fight, and it keeps one language and one test suite with the fake Harnesses.
- (b) **`thirdshift experiment` with Inspect underneath.**
  - It brings epochs with pass^3, a clean stop and retry, Scores edited with their history, and a local viewer, and it sends nothing by default.
  - It adds Python and about 77 packages, and ships two or three releases a week with breaking changes about monthly.
  - It still scores a Trial that timed out, and signals only that Trial's direct child.
- (c) **`thirdshift experiment` with promptfoo underneath.** It buys the least. It adds Node with about 640 packages and an egress re-audit at every upgrade, and Trials running when it stops come back as errors to retry.
- (d) **promptfoo by hand,** Q4.2 (b). It enforces none of the rules, and Q4.4's verdict would live in a JavaScript hook whose results promptfoo doesn't keep.

*Recommended:* (a). The parts that make an Experiment honest are thirdshift's in every option. What a framework adds is where silent inaccuracy comes from: retries that start fresh sessions, caches, and timeouts that leave sessions running. That fits Q2.4's rule that accuracy outranks packaged tools. You'd still get outside experience: Session traces put thirdshift on OpenTelemetry, upstream's file exporter (#3763) needs reviewers, and promptfoo's telemetry fix (Q3.12) is still there to make.

*Updated 2026-10-21:* the sandbox research has reported (see Research results). Containers would make Harbor and Inspect's Docker sandbox runnable, but neither runs subscription Trials through thirdshift, so (a) stands. The sandbox work it recommends, a PID namespace and a network of its own through an allowlisting proxy, adds a few hundred lines of Rust. Every option needs it, so it doesn't change the comparison.

*Answer:* I agree with the recommendation.

**Q5.7 - The interval behind Q4.4's verdict.** Q4.4 chose a 95% Bayesian interval "as Bowyer et al. recommend". The runner research found that their paired method takes one pass or fail per Eval task per Arm. So it fits the Spec-review suite, but not the code review's three Trials. The options:
- (a) **One method for both suites: a Dirichlet over the values each Eval task's difference can take.**
  - With 3 Trials a difference is one of seven values from −1 to +1, and with 1 Trial one of three. The posterior mean difference is exactly Q4.4's quantity, and the pairing is built in.
  - The prior adds up to one pseudo Eval task, spread evenly over the values: 1/7 each with 3 Trials, 1/3 with 1. The interval comes from 20,000 seeded draws.
  - The seed, draw count, prior, margin, credible level and thirdshift version are recorded with the bar.
  - Wrong findings become a flag per Trial, "reported any wrong finding", run through the same code.
  - "How many more Eval tasks would settle it" is the smallest number with at least a 0.8 chance of a clear verdict.
- (b) **Bowyer's model for the Spec review, and the Dirichlet for the code review.**
- (c) **A t interval,** which isn't Bayesian, but held up as well in the same checks.

*Recommended:* (a). It's one method, and it measures exactly what Q4.4 asks. In the research's own small simulations, its 95% intervals held the true difference 93–98% of the time. The paper's uniform prior fell to 79% for large gaps. The first real Experiments replace those simulations, as Q4.4 says.

*Updated 2026-10-21:* the sandbox research suggests that a Trial which failed for reasons outside the agent, such as a broken sandbox or a plan-limit stop, should be void. It would be re-run, shown, and kept out of this interval, by rules fixed with the bar. Q3.5 already voids a Trial that changed its repository. Which failures count is next round's question, so answer this one as if void Trials simply drop out.

*Answer:* agree

**Q5.8 - Where the OTLP writer lives.** This brings back Q4.14, whose premise was out of date. OpenTelemetry Rust has a file-exporter crate, `opentelemetry-otlp-file`, in review as #3763 since 28 September. Its maintainers want it in their main repository, but it has no approval yet, so it is likely months away, and it needs the SDK, about 18 crates. Arti's `otlp-file-exporter` writes traces only. A crate of your own would be small: logs and traces are 18 message types, and only four schema changes touched them in two years (`docs/research/otlp-file-writer.md`). The options:
- (a) **Write it in thirdshift, as a module with no thirdshift types.**
  - It follows the encoding rules, and is tested by round trips through the official types and by the Collector's file receiver. The official types are a test-only dependency, so nothing is added at run time.
  - It moves into its own crate, such as `otlp-jsonl`, if anyone else wants it.
  - You help upstream by reviewing and testing #3763.
- (b) **Publish it as its own crate now,** used by thirdshift. That brings its own CI and MSRV, two crates.io owners, and releases ahead of thirdshift's.
- (c) **(a) until #3763 merges, then move to the SDK.**

*Recommended:* (a). It is Q4.14 (a) with a clean boundary, so publishing later is a move rather than a rewrite. OpenTelemetry® is a registered trademark, so the crate's name leaves out "opentelemetry" and "otel".

*Answer:* agree

**Q5.9 - One file or two per session.** The OTLP file format's specification says "Files must contain exactly one type of data: traces, metrics, or logs." ADR 0019 puts a session's log records and spans in one file. The Collector would probably still read it, but nothing promises that. The options:
- (a) **Two files per session, side by side where the Session log is today,** both named after the session:
  - one of log records, written as each Harness event arrives, which the Console's live view reads and Silent watches;
  - one of spans, written as each ends.

  ADR 0019 is amended on `main`.
- (b) **Keep one file,** and record the departure from the specification in ADR 0019. The export command writes conforming files when a tool needs them.
- (c) **Log records only.** Spans are built from the records whenever the Console or the export command needs them. This reopens Q3.15 and Q3.19, which have spans written as they end.

*Recommended:* (a). It follows the specification at the cost of a second file name, and keeps everything Q3.15 and Q3.19 settled. The Collector, or anything else that reads the format, can then take each file as it is.

*Answer:* agree

**Q5.10 - Codex's diffs (#854).** Codex's `--json` stream says which files an edit changed, but not how. So the Console shows "Patch unavailable in this log." for Codex edits, and #854 asks whether to capture them. Codex's own session file, under `~/.codex/sessions/`, keeps the diff of every edit Codex makes through `apply_patch`, and thirdshift already holds the session ID that names it. Edits made through shell commands aren't in it, and the format is Codex's own, not a published one. The options:
- (a) **thirdshift reads each edit's diff from Codex's session file,** and adds it to the matching record in the Session trace, marked as Codex's record of the edit.
  - Edits are matched to the stream's by order and file path, since the two give an edit different IDs.
  - Shell edits stay without a diff, and if the file can't be read, the record stays as it is.
  - Old Codex logs get their diffs in the conversion (Q4.11), where Codex's files still exist.
- (b) **Leave Codex's edits without diffs.** The Console keeps its fallback, and evals use the merged diff.
- (c) **Run Codex through `codex app-server`,** which streams diffs live. That changes how thirdshift runs Codex.

*Recommended:* (a). Q3.14 kept diffs because they matter for accuracy. #854 asked for what the session really changed, not a guess from the checkout, and this source is what Codex itself applied. It would close #854 within the Session-trace Spec.

*Answer:* agree
