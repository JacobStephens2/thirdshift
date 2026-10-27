# Grilling: #1434 and the follow-ups from the eval research

Issue: https://github.com/JacobStephens2/thirdshift/issues/1434
Related: #1252 (its Spec PR #1279 is still open, so `main` judges releases under the old single 95% Bar), #1371 (where out-of-scope code-review tasks are parked)
Research: [eval-suite-scaling.md](../research/eval-suite-scaling.md), [eval-build-vs-buy.md](../research/eval-build-vs-buy.md)

The research proposed six follow-ups: an admission rule for Eval tasks (#1434), Trials per task, a cost ceiling, a versioned output format, a one-off cross-check of the Verdict, and an implement suite. This grill decides which of them become work, and in what shape.

## Round 1 (answered)

**Q1.1 - Admit an Eval task to a gating suite only inside a measured pass-rate band?** The scaling note's §1.5 simulation: 10 solvable tasks plus 30 that always fail pass a really regressed Candidate 63% of the time, against 3% for the 10 alone. A task both Arms always fail adds a zero difference, which pulls the estimate toward zero, inside the −10-point margin. So a never-passing task is not just wasted tokens: it can hide a regression. #1434's step 1 classifies such tasks by judgment. The options:
- (a) A measured gate on top of #1434's judgment. A task is `approved` for a suite that gates releases only when (i) its target bug breaks a documented standard or contradicts a Spec line (#1370's scope rule, used as a checklist), and (ii) its Baseline pass rate over at least 6 Trials is between about 0.15 and 0.95. The measured rate and Trial count are recorded beside `state` in the task's manifest.
- (b) Judgment only, as #1434 is written today.
- (c) A measured lower bound only (at least 0.15), with no upper bound. Tasks that nearly always pass stay, because a real breakage shows up on them.

*Recommended:* (a). It turns a judgment that can drift into a check anyone can repeat. The upper bound is the weaker half: if Q2.x later splits the suite into regression and capability tasks, the band would be revisited there.

*Answer:* agree

**Q1.2 - Measure each task's pass rate by pooling an A/A Experiment's two Arms?** The scaling note proposes a Baseline-only run at 6 Trials per task: about 108 Trials for code review and 180 for Spec review. An A/A Experiment already runs one setup twice, 3 Trials per Arm. Pooled, that is 6 Trials of one setup per task, and #1434's step 3 already calls for an A/A rerun. The options:
- (a) Pool the two Arms of #1434's code-review A/A rerun for the measurement, and run one Spec-review A/A for its suite. No new Experiment mode is needed.
- (b) Add a Baseline-only mode to Experiments, and measure with it.
- (c) Measure by hand, outside Experiments.

*Recommended:* (a). It gets the measurement and the A/A rerun from the same Trials, with no code. Spec review has 1 Trial per Arm, so its pooled reading is 2 Trials per task. That is too few for the band, so Q1.4 decides how to measure it.

*Answer:* agree

**Q1.3 - Call an Eval task's measured pass rate its Difficulty?** The admission rule needs a name for "this task's measured Baseline pass rate, over N Trials". "Calibration" is taken: a Grader's Scores are "uncalibrated" until they agree with yours. The options:
- (a) **Difficulty**: an Eval task's pass rate measured on the Baseline setup, with its Trial count. A task is **discriminating** when its Difficulty falls inside the admission band.
- (b) **Pass rate**, with no glossary entry.
- (c) Another name you prefer.

*Recommended:* (a). It reads naturally in a manifest and a Verdict ("13 of 21 tasks discriminating"), and it doesn't collide with an existing term. It goes in `GLOSSARY.md` once agreed.

*Answer:* agree

**Q1.4 - Keep Spec review at 1 Trial per task in the Bar?** I suggested 2–3 Trials earlier. The scaling note disagrees, and I think it's right. Spec review's within-task noise is 43% of its per-task variance, but Miller's sizing formula says more tasks always beat more Trials for the same tokens. What 1 Trial can't do is tell a broken task from a hard one, and 17 of its 30 tasks never passed. That also reopens #1434's "out of scope: the `spec-review` suite", since floor tasks hide regressions there too. The options:
- (a) Keep 1 Trial in the Bar. Run a one-off 6-Trial Baseline measurement of Spec review (about 180 Trials) and apply Q1.1's band. Bring Spec review into #1434's scope.
- (b) Raise Spec review to 3 Trials in the Bar.
- (c) Leave Spec review alone, as #1434 says.

*Recommended:* (a). It spends tokens once on a measurement instead of on every release.

*Answer:* agree

**Q1.5 - Make a suite with too few discriminating tasks advisory?** This is #1434's step 4. A suite that can't tell Arms apart produces false passes more than false blocks (§1.5). At 10 discriminating tasks, a no-op passes 48% of the time and a real regression fails 34%; at 40, 73% and 94%. The options:
- (a) Yes. A suite with fewer than 20 discriminating tasks is reported in the release summary, but can neither pass nor block a release on its own. If every selected suite is advisory, `release.sh` says so and asks, as it does on inconclusive today.
- (b) Yes, with a different threshold.
- (c) No. Every suite with approved tasks gates.

*Recommended:* (a). 20 is halfway to the note's 30–40 target, and an advisory suite still asks rather than waving the release through.

*Answer:* agree

**Q1.6 - Show the worst-case cost before an Experiment starts, instead of a hard cost ceiling?** The build-vs-buy note proposes a ceiling like `claude plugin eval --max-cost-usd`. Dollars mean nothing on subscription logins, and tokens show as `unknown` for some Harnesses. The cost is already bounded by Trials × timeout: the #1370 A/A was 120 Trials × 30 minutes at most. The options:
- (a) Before the first Trial, print the Trial count, the worst-case session time (Trials × timeout) and an estimate from past Experiments' mean Trial time, and ask to confirm. No ceiling in the Bar.
- (b) A hard ceiling in session-minutes, saved with the Bar and checked before each Trial; reaching it pauses the Experiment.
- (c) Neither.

*Recommended:* (a). Experiments are started on purpose, so seeing the cost up front covers the real risk. A ceiling adds a new way to pause that resume then has to handle.

*Answer:* agree

**Q1.7 - File the versioned output format as a standalone Ticket?** There's a documented input format (`docs/eval-tasks.md`) and Trial protocol (`docs/trial.md`), but no documented output format: the per-Trial result, the Score with its checks and recheck history, the Verdict with its intervals and Bar. OTLP export already covers trajectories, but carries no Scores. The options:
- (a) A `ready-for-agent` Ticket: document today's `experiments <id> --json` and result files as format 1, in a new `docs/experiment-results.md`, with a test that fails if the JSON drifts from the document. No new export.
- (b) Also export to Inspect Scout's transcript schema now.
- (c) Don't file it.

*Recommended:* (a). The scaling note already needed this data and had to read the JSON shape from code. An export can follow once something reads it.

*Answer:* agree

**Q1.8 - Cross-check the Verdict now, as research, not as an issue?** The build-vs-buy note suggests checking the Dirichlet Verdict against independent methods: McNemar's test and a bootstrap interval. The scaling note already re-implemented the Verdict in Python and matched every published interval, but that re-implements the same method, so it doesn't count as an independent check. The options:
- (a) A background agent runs McNemar and a paired bootstrap (scipy, without MLflow) over Experiments `560af5c5…` and `199d80f2…` read-only, and adds a section to `eval-suite-scaling.md`.
- (b) Do it with MLflow's `compare_evaluations`, as the note suggests.
- (c) Skip it.

*Recommended:* (a). It's the same statistics without a heavy dependency, and an hour of an agent rather than a Ticket.

*Answer:* agree

**Q1.9 - Capture the implement suite as an idea issue now, and grill it later?** Settled in #698 (Q6.7) but never filed. The scaling note adds injected bugs graded by `cargo test` as its main task source, plus the host's limits: 4 cores, 8 GB, 32 GB of free disk. The options:
- (a) File a `needs-triage` idea issue now, linking Q6.7 and §2 and recommendations 13–18 of the scaling note. Grill it once #1434's suite repair is done.
- (b) Grill it now, in this file.
- (c) Don't file it yet.

*Recommended:* (a). Code review's repair will show whether the admission rule works before a second suite is built on it.

*Answer:* agree

*Checked:* **Difficulty** and **discriminating** are now in `GLOSSARY.md`, under Evals.

## Round 2 (answered)

**Q2.1 - Judge only discriminating tasks, and run the rest as measurement?** Q1.1 approves a task only once its Difficulty is in the band. But Q1.2 measures Difficulty with an Experiment, and an Experiment runs only `approved` tasks (`src/evals/experiment.rs:397`), so a new task can't be measured before it's approved. The options:
- (a) Move the gate from approval to judging. When an Experiment starts, it saves in the Bar the list of its discriminating tasks, from their recorded Difficulty. The Verdict judges only those. Approved tasks with no Difficulty yet still run, and their Trials are reported as measurement but never judged. Out-of-band tasks are parked (Q2.3), so they don't run at all.
- (b) Keep the gate at approval, and add an Experiment flag that also runs `proposed` tasks, outside the Verdict.
- (c) Approve first, measure, then park whatever falls outside the band. Until it's parked, an unmeasured task counts in the Verdict.

*Recommended:* (a). A task can't hide a regression in a Verdict it isn't judged in, and the list is fixed before the first Trial, like the rest of the Bar. The judged list is never chosen from the Experiment's own results. Choosing it that way would bias the Verdict. This refines Q1.1's answer: "approved only when in band" becomes "judged only when in band".

*Answer:* agree

**Q2.2 - Record Difficulty in the task's manifest, with a command that computes it from an A/A Experiment?** Q2.1 needs Difficulty in a form thirdshift can read. `task.toml` has no field for it today (`src/evals/format.rs:25`). The options:
- (a) A `difficulty` table in `task.toml`: passes, Trials, and the Experiment, commit, Harness, Model and Effort it was measured on. Written by a new command, `thirdshift experiments <id> --record-difficulty`, which pools both Arms and refuses unless the Arms are identical. Re-measuring replaces the table.
- (b) Written by hand, in the same table.
- (c) Kept outside the task, in a file for the whole suite.

*Recommended:* (a). Pooling by hand across 20 tasks is where mistakes creep in. Keeping it in the task means a parked task takes its Difficulty with it.

*Answer:* agree

**Q2.3 - Park out-of-band tasks under the existing `parked/` folder, split by reason?** The host already has `~/.thirdshift/evals/parked/` with a README, holding #1370's correctness-review tasks. The options:
- (a) `parked/floor/` for tasks below the band and `parked/ceiling/` for tasks above it, each keeping its Difficulty. The README says how a task comes back: re-measure it on the current setup.
- (b) One `parked/out-of-band/` folder.
- (c) Back to `proposed`, inside `suites/`.

*Recommended:* (a). A floor task is probably broken or out of scope, and a ceiling task is probably fine but uninformative. They come back for different reasons. The regression/capability split, deferred until Difficulty is measured (see Settled), may want the ceiling tasks later.

*Answer:* agree

**Q2.4 - Keep counting a task's Difficulty when it was measured on a different setup?** Difficulty is measured on one Harness, Model and Effort. A release that changes the Model judges the Candidate on tasks measured under the old one. The options:
- (a) It still counts. The Verdict lists how many judged tasks were measured on a setup other than the Baseline's, and the release summary shows it.
- (b) It doesn't: those tasks run as measurement only.

*Recommended:* (a). Option (b) would make every Model-change release advisory, and that's the release the gate matters for most. Difficulty mostly reflects the task, not the Model.

*Answer:* agree

**Q2.5 - Save the advisory threshold in the Bar, and call such a suite an Advisory suite?** Q1.5 decided that a suite with fewer than 20 discriminating tasks can neither pass nor block a release. The options:
- (a) `min_discriminating_tasks = 20` in the Bar, fixed before the first Trial, with a `--min-discriminating-tasks` flag. The suite still gets a Verdict, marked advisory. Add **Advisory suite** to the glossary: "An **Eval suite** with too few discriminating **Eval tasks** for its **Verdict** to pass or block a release."
- (b) A constant in `release.sh` only. Experiments don't know about it.

*Recommended:* (a). The Bar is where the rules a Verdict follows are fixed. If the rule lived only in `release.sh`, `experiments <id>` and the Console would show a pass that the release ignores.

*Answer:* agree

**Q2.6 - Estimate the cost from this machine's past Trials, and ask only on a terminal?** Q1.6 decided to show the worst case and an estimate before the first Trial, and ask. The options:
- (a) The estimate is the mean minutes of completed Trials on this machine, per suite and Harness. With no history, show only the worst case. Ask only when stdin is a terminal. `--yes` skips the question. `release.sh` asks once, through its existing prompt.
- (b) Always ask, with no `--yes`.

*Recommended:* (a). Scripts and the release flow need a way through. One question per release is enough.

*Answer:* agree

**Q2.7 - Package it as one Spec plus standalone Tickets?** The options:
- (a) A new **Spec** covering Q2.1–Q2.5: Difficulty in the manifest and its command, judging only discriminating tasks, and Advisory suites in the Verdict and `release.sh`. It's blocked by #1279, since it changes the same Verdict code. #1434 is rewritten as the Spec's `ready-for-human` host Ticket: measure, record, park, and measure Spec review too (Q1.4). The cost prompt (Q1.6, Q2.6) and the output format (Q1.7) become standalone `ready-for-agent` Tickets. The implement suite gets its idea issue (Q1.9).
- (b) Everything as standalone issues.
- (c) Fold everything into #1252's open Spec.

*Recommended:* (a). #1252 has merged nearly all of its work and is waiting on its Spec PR, so adding work there would hold that up. The cost prompt and the output format don't depend on the rest.

*Answer:* we'll run to-spec skill then to-tickets for this

*Checked:* **Advisory suite** is now in `GLOSSARY.md`, under Evals.

## Round 3 (answered)

**Q3.1 - Keep the cost prompt, the output format and the implement-suite idea out of the Spec?** Q2.7's answer sends this grill through `to-spec` and `to-tickets`. Three items don't depend on the Difficulty work. The options:
- (a) The Spec holds only Q2.1–Q2.5 and the host work. The cost prompt and the output format are filed as standalone `ready-for-agent` Tickets, and the implement suite as a `needs-triage` idea issue, next to the Spec, not under it.
- (b) All of them become Tickets of the one Spec.

*Recommended:* (a). A Spec's work reaches `main` only when its Spec PR merges, and this Spec is blocked by #1279. As Tickets inside it, the cost prompt and the output format would wait on both for no reason.

*Answer:* agree

**Q3.2 - Leave #1434's body alone until the A/A running on the host has reported, then attach #1434 to the Spec as its host Ticket?** A session on the host is working #1434 right now, with Experiment `83721fbb…` at 23 of 120 Trials. Its acceptance criteria are its own. The options:
- (a) Don't edit #1434 while that run is going. `to-tickets` publishes the Spec's other Tickets, and links #1434 as a sub-issue that blocks the "judge only discriminating tasks" Ticket. Once the A/A's results are posted, rewrite #1434's body to add recording Difficulty, parking, and measuring Spec review (Q1.4). Keep its existing criteria.
- (b) Rewrite #1434 now.
- (c) Leave #1434 as it is, and give the new host work a Ticket of its own.

*Recommended:* (a). Rewriting the issue under a running session risks it acting on criteria that changed underneath it. The A/A's pooled results are also the first Difficulty data, so a rewrite written after them can quote real numbers.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- The Spec is written with `to-spec`, then split into Tickets with `to-tickets`. Per `docs/agents/triage-labels.md`, it stays `needs-triage` until every Ticket and blocking link is published, and only then gets `ready-for-agent` (Q2.7).
- The Spec is blocked by #1279, since it changes the same Verdict code (Q2.7).
- An Experiment saves its list of discriminating tasks in the Bar before the first Trial. Only those are judged. Approved tasks with no Difficulty run as measurement, and out-of-band tasks are parked (Q2.1).
- Difficulty lives in a `difficulty` table in `task.toml`, written by `thirdshift experiments <id> --record-difficulty`. The command refuses unless the Arms are identical (Q2.2).
- Out-of-band tasks go to `parked/floor/` or `parked/ceiling/` on the host, keeping their Difficulty (Q2.3).
- Difficulty measured on another setup still counts, and the Verdict says how many judged tasks it applies to (Q2.4).
- The Bar holds `min_discriminating_tasks = 20`, with a flag. A suite below that is an Advisory suite: it gets a Verdict, marked advisory (Q2.5).
- The cost estimate is this machine's mean completed-Trial minutes, per suite and Harness. The question is asked only on a terminal, `--yes` skips it, and `release.sh` asks once (Q2.6).

- No off-the-shelf tool replaces Experiments; the build-vs-buy note recommends keeping it as built, and this grill doesn't reopen that.
- Code review stays at 3 Trials per task: within-task noise is already only 10% of its variance, and `pass_cubed` needs 3.
- Measuring Difficulty doesn't depend on the Bar, so it needn't wait for #1279. An A/A that is meant to test the two-level Bar does: it should run on a build that has it.
- The admission band is 0.15–0.95, over at least 6 Trials. Tasks must also pass #1370's scope checklist (Q1.1).
- Code review's Difficulty comes from the A/A rerun already running on the host, Experiment `83721fbb…`: both Arms at `1df25aea`, code review only, 3 Trials per Arm. Pooled, that's 6 Trials per task, and it needs no extra run (Q1.2).
- Spec review's Difficulty comes from one Spec-review A/A with `--trials-per-task 3`: 6 pooled Trials per task, about 180 Trials. The flag already exists (`src/evals/experiment/options.rs:129`), and the Bar accepts 1 or 3. Its Experiments stay at 1 Trial (Q1.2, Q1.4).
- #1434's "the `spec-review` suite is out of scope" no longer holds (Q1.4).
- Below 20 discriminating tasks a suite is advisory. If every selected suite is advisory, `release.sh` asks, as it does on inconclusive (Q1.5).
- No cost ceiling goes in the Bar (Q1.6).
- The output-format Ticket documents today's JSON as format 1, in `docs/experiment-results.md`, with a drift test. No Scout or Inspect export for now (Q1.7).
- The Verdict cross-check is research, appended to `eval-suite-scaling.md` (Q1.8).
- The implement suite gets a `needs-triage` idea issue now, and its grill comes after #1434 (Q1.9).
- Splitting into regression and capability suites waits until Difficulty has been measured, since how many tasks sit near the ceiling decides whether it's worth it.
- The cost prompt (Q1.6, Q2.6) and the output format (Q1.7) are standalone `ready-for-agent` Tickets, and the implement suite (Q1.9) is a `needs-triage` idea issue. None of them is under the Spec (Q3.1).
- #1434 isn't edited while Experiment `83721fbb…` runs. `to-tickets` links it to the Spec as a sub-issue that blocks the "judge only discriminating tasks" Ticket. After the A/A's results are posted, its body gains recording Difficulty, parking, and measuring Spec review, and keeps its existing criteria (Q3.2).
- The frontier is empty. What's left was deferred on purpose: the regression/capability split (until Difficulty is measured), the implement suite's grill (after #1434), and the Bar's prior (only if growing the suite isn't enough).

## Research results (2026-10-27)

- The host's code-review suite has 21 tasks: 20 `approved`, 1 `proposed`. 13 have never been run. Confirmed by `ls` and `grep state` on the host.
- An Experiment runs only `approved` tasks (`src/evals/experiment.rs:397`). A task's state is `proposed` or `approved` (`src/evals/format.rs:11`).
- `src/verdict.rs` on `main` still has one `credible_level` and still requires the interval to reach zero (`src/verdict.rs:255`). The two-level Bar and the 70% pass level are merged only into #1252's Spec branch, behind the open Spec PR #1279. Confirmed with `gh pr list`.
- Pooling an A/A Experiment's Arms is valid for measuring Difficulty, since both Arms are the same commit, Harness, Model and Effort. Inference, not tested.

- #1434's host work is already under way today. A backup, `suites.bak-20261027-pre1434`, was taken. 8 tasks from `chart35`, `keeplore` and `vaulted-agent` were added between 13:50 and 15:35. A Claude session on the host is watching Experiment `83721fbb…`, which had 23 of 120 Trial results at about 16:30. Confirmed by `ls` and `ps` on the host.
- `~/.thirdshift/evals/parked/` already exists, with a README and `correctness-review/` (8 tasks, including the former bug-303 and bug-559). `dropped/` holds tasks removed altogether. Confirmed by `ls` on the host.

- **The Verdict cross-check (Q1.8) found no sign the Verdict is miscalibrated.** Confirmed by McNemar's test and a cluster bootstrap over Experiments `560af5c5…` and `199d80f2…`, plus simulations. Details are in the appendix of [eval-suite-scaling.md](../research/eval-suite-scaling.md).
  - The rebuilt difference counts match the published ones in all 8 cells.
  - 14 of 16 method-by-rule comparisons agree. No method fails either A/A.
  - In simulations at 15, 30 and 40 tasks, the Dirichlet and bootstrap decision rates stay within 0.05 of each other. The Dirichlet's false-fail rate is 2%, about right for a one-sided 2.5% test.
  - The disagreements are all pass against inconclusive, never a fail. The main cause is the prior: at 17 tasks it lowers the lower bound by about 0.021, enough to flip #1267's code-review accuracy across the −0.10 margin by 0.002.
  - Possible follow-up: the prior puts weight on ±1 differences that no task has ever produced. A prior concentrated on small differences would narrow small-suite intervals without weakening the fail rule. Growing the suite is still the better fix, so this isn't a question yet.

## Waiting on research

- None.
