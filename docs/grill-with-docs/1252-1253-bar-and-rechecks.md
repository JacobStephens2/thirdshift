# Grilling: #1252 and #1253, the Bar and rechecks at our suites' sizes

Issues: https://github.com/JacobStephens2/thirdshift/issues/1252, https://github.com/JacobStephens2/thirdshift/issues/1253
Parent Spec: https://github.com/JacobStephens2/thirdshift/issues/977
Research: `src/verdict.rs`, `src/evals/experiment/grading.rs`, `src/evals/experiment/rechecks.rs` and `scripts/release.sh` on `issue-977`; the simulations under "Research results"

## Round 1 (answered)

**Q1.1 - Are these one problem?** #1252 says code review can't pass with 10 Eval tasks. #1253 says a Spec-review pass makes you recheck nearly every Candidate Trial. Both come from one cause: at our suites' sizes, the standing Bar (−10-point margin, 95% credible level, prior 1) only passes when the Arms agree on almost every task. Spec review, with 30 tasks, can't pass if even one task goes one Arm's way and another task goes the other's (lower bound −11.0; see Research results). And since a pass sits that close to the line, any one flip can tip it, so every Candidate Score counts as decisive. The options:
- (a) Grill them together, settle the Bar first, then the recheck rule on top of it. Write one decision record here, and close or rewrite both issues from it.
- (b) Settle them separately, as they were filed.

*Recommended:* (a). Any recheck rule settled before the Bar is set would have to be settled again once it is.

*Answer:* agree

**Q1.2 - Take #1252 and #1253 off #977 while this is settled?** A Spec run never starts a Ticket labelled `needs-triage`, and any Ticket not done when the others finish makes it a Failed spec run (`src/spec_run.rs:106`). So the Spec PR stays a draft and gets no Spec review or Delivery. The options:
- (a) Remove both from #977's sub-issues now and say so in a comment on each. They stay open. The Tickets this grill produces are published later as their own follow-up, Ticket or Spec, that links #977.
- (b) Leave them on #977 and finish this grill before the run's other Tickets land.
- (c) Leave them, and accept a Failed spec run that a rerun picks up after this grill.

*Recommended:* (a). #977 doesn't need these to be correct, just to be usable with small suites. Today's Bar is still a Bar, and the Verdict's other parts don't change. Racing the run puts pressure on a policy decision.

*Answer:* agree

**Q1.3 - What should a release do on an inconclusive Verdict?** `scripts/release.sh:259` asks "release anyway? [y/N]" on both fail and inconclusive. With a Bar no real release can pass, that prompt gets answered yes every time, and then it stops protecting anything. The options:
- (a) Keep the prompt on inconclusive, and set the Bar (later rounds) so that a release which changes nothing usually passes. Then an inconclusive is rare, and a reason to stop.
- (b) Prompt only on fail. An inconclusive releases with its intervals in the release summary. The gate then only catches a regression the data shows clearly: with 30 tasks, a large one.
- (c) Keep everything as it is, and accept the override as the normal path for now.

*Recommended:* (a). #977's Problem Statement asks whether the Candidate is worse *before* it ships. Under (b), "we couldn't tell" ships without a question, which is where we are today with no Experiments at all.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - Measure the noise before choosing numbers?** Nobody has measured how often two runs of one setup disagree on a task (Research results), and that rate decides which Bar a release that changes nothing can pass (Q1.3). Story 7 already lets one build run in both Arms. The options:
- (a) Run an A/A Experiment on the `thirdshift` host: one commit in both Arms, both suites (`--suite code-review --suite spec-review`), under today's Bar. Its per-task disagreements give the rate, and the Bar's numbers are chosen from it. That's about 120 Trials: 10 × 3 × 2 for code review, 30 × 1 × 2 for Spec review. It needs #1251 merged first, since Spec-review Trials stop without Trial inputs today. If `experiment` refuses identical Arms, allowing an A/A run is part of the work.
- (b) Choose the numbers now from the proxies, and revisit them once real Experiments have run.

*Recommended:* (a). The proxies hint at disagreement on 2 of 6 tasks, but none of them is a same-setup measurement. One A/A run turns a guess into a fact. Until then, releases keep today's Bar and its override, which Q1.2 made affordable.

*Answer:* agree

**Q2.2 - Grow code review with #1252's seven strong candidates?** More Eval tasks narrow every interval, whatever the Bar (10 → 17 tasks takes the identical case from −12.8 to −7.8). The triage comment on #1252 found 7 bugs since #607 that can be seen in their PR's diff, and 5 more that need context from outside the diff. The options:
- (a) Add the 7 strong ones, bringing code review to 17. An agent drafts each one's `criteria`, and you approve each before it can run, as the first 10 were approved.
- (b) Add all 12, bringing it to 22.
- (c) Add none for now.

*Recommended:* (a). A "partly visible" bug tests whether a review guesses context it wasn't given, which the code-review suite doesn't claim to measure.

*Answer:* agree

**Q2.3 - Which part of the Bar gives way?** Under Q1.3 (a), a release that changes nothing should usually pass. Today one credible level does two jobs: a pass needs the 95% lower bound above the margin, and a fail needs the 95% upper bound below zero. The options:
- (a) Lower the single credible level, say to 80%. That makes a pass reachable (Spec review, 30 tasks, ±2: −8.4), but makes noise fail an A/A Experiment about 1 time in 10 for each suite, up from 1 in 40.
- (b) Widen the margin, say to −20 points, at 95%. False fails stay rare, but a Candidate up to 20 points worse can pass.
- (c) Two levels. A pass needs the lower bound of an 80% interval above the −10 margin. A fail still needs the 95% upper bound below zero. Anything between is inconclusive.
- (d) A separate Bar for each suite, set from its own size.

*Recommended:* (c). It keeps what "worse" means (10 points) and what a fail means (95% sure it's worse at all), and only asks less certainty for a pass, which a human prompt backs up when it's missed. This changes the Verdict's method (`paired-dirichlet-v1`), so it gets an ADR. The levels themselves are set after Q2.1's measurement.

*Answer:* agree

## Round 3 (answered)

**Q3.1 - What does "usually passes" mean?** Q2.3 lets the pass level be set from Q2.1's A/A Experiment. That needs a target: how often an Experiment of a change that changes nothing should pass, given the measured disagreement and the suites' sizes. The options:
- (a) 80% of the time, with a floor: the pass level never goes below 70%. If 70% isn't enough to meet the target, the suite grows instead.
- (b) 90% of the time, with the same floor.
- (c) No target. Pick a pass level by eye from the A/A result.

*Recommended:* (a). One release in five stopping to ask is a prompt you still read. 90% needs a lower pass level, and that lets more real drops through as well. The A/A result will say how many. The floor stops "pass" turning into a coin toss.

*Answer:* agree

**Q3.2 - Does the A/A Experiment wait for the 7 new code-review tasks?** Q2.1 runs it after #1251. Q2.2 adds 7 tasks that you approve one by one. The options:
- (a) Wait until you've approved them, and measure on the 17-task suite the Bar will judge.
- (b) Run it on today's 10 now, and the 30 Spec-review tasks.

*Recommended:* (a). The rate per task should carry over, but the intervals the target is checked against depend on the suite's size. Measuring twice costs another ~120 Trials.

*Answer:* agree

**Q3.3 - Which Scores need a recheck?** Facts first (Research results, "Rechecks under a pass level of 80%"):
- With room under a pass, one flip no longer tips it. On 30 identical Spec-review tasks, flipping one Candidate Trial gives −8.5, still a pass, so no Score is decisive.
- Close to the line, nearly every Candidate Score is still decisive. With one task going each way (−6.2), one more flip gives −10.9.

So today's rule now asks for rechecks where a Score really decides the Verdict, and their number shows how close the pass is. The options, from #1253:
- (a) Keep the decisive rule. Add a batch command for a reviewed pending list (#1253 option 3), and have `experiments <id> --json` say which flip makes each Score decisive.
- (b) Keep the rule, and add only the explanation of which flip.
- (c) Recheck a seeded sample of k decisive Scores (#1253 option 2), with an ADR.

*Recommended:* (a). A pass that rests on one mis-scored Trial is still caught, which is what #1006 was for. The batch only saves typing: it lists every pending Score with its Trial link and the reason it's pending, and you confirm them all with one reason.

*Answer:* agree

**Q3.4 - What are the two levels called?** The glossary's **Bar** lists "the credible level" (`GLOSSARY.md:355` on `issue-977`). Under Q2.3 there are two. The options:
- (a) **Pass level** and **fail level**: the credible level a pass is judged at, and the one a fail is judged at. The Bar entry lists both, and "credible level" stays the general statistics word, not a Bar part.
- (b) "Pass confidence" and "fail confidence".

*Recommended:* (a). They say which Verdict each one decides, and "confidence" invites confusion with frequentist confidence intervals, which this isn't.

*Answer:* agree

## Round 4 (answered)

**Q4.1 - What gets published, and where?** The work this grill settled, in order:
1. **Two-level Verdict** (agent). Add a pass level and a fail level to the Bar. A pass is judged at the pass level against the margin, and a fail at the fail level (95%) against zero. The method gets a new name, and there's an ADR, docs/guide.md and the Bar's `--` flags. The default pass level stays at 95% until 5 sets it. That covers #1252's test pinning the smallest suite that can pass, under the new rule.
2. **Allow an A/A Experiment** (agent). One commit in both Arms with the same flags. I found no check that refuses identical Arms (`src/evals/arms.rs` on `issue-977`), so this may be a test and a sentence in docs/guide.md only.
3. **Seven code-review Eval tasks** (human, on the `thirdshift` host). Eval data stays private on the host (#977), so a Ticket run can't write it. You run an agent there to draft each one's `criteria`, as with #1018, then approve each.
4. **Run the A/A Experiment** (human, on the host). Blocked by 2, 3 and #1251. Post its per-suite disagreement on the issue.
5. **Set the default pass level** (agent), from 4's numbers against Q3.1's target and floor. Blocked by 1 and 4.
6. **Batch rechecks and which flip** (agent). The batch command and `experiments <id> --json` naming the flip that makes each Score decisive. Independent of the others.

The options:
- (a) Rewrite #1252 as a Spec with these 6 Tickets. Rewrite #1253 as Ticket 6 and attach it to #1252. Publish by `docs/agents/triage-labels.md`: #1252 stays `needs-triage` until every Ticket and blocking link is in place.
- (b) Open a new Spec, and close #1252 and #1253 as superseded by it.
- (c) Six standalone Tickets with no Spec.

*Recommended:* (a). It keeps the issues' history and the numbers they carry, and a Spec run follows the blocking order. Tickets 3 and 4 are `ready-for-human`, so a Spec run on #1252 fails on them (`src/spec_run.rs:106`) until you've done them and closed them. You can run its agent Tickets one at a time instead.

*Answer:* agree, we can use the to-spec and to-tickets skills for this

**Q4.2 - What does it build on?** The Bar, the Verdict and `thirdshift experiment` exist only on `issue-977` until #977's Spec PR merges, and the host can only run an Experiment once a release carries it. The options:
- (a) Mark the new Spec blocked by #977. Its Tickets start once #977 is merged to `main`.
- (b) Start Tickets 1, 2 and 6 now on `issue-977`, as #977's own follow-ups.

*Recommended:* (a). Q1.2 took these off #977 so its Spec run could finish without them. Building on its branch would put them back in its way.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- From #977 (user stories 9–10): the Bar is recorded before the first Trial and refused if it changes afterwards. Nothing here changes that. Any new Bar applies to Experiments started after it ships.
- From #1006: borderline Scores are always rechecked. Only the rule for *decisive* Scores is open here.
- From Q1.1: this file is the one decision record for #1252 and #1253, and both are closed or rewritten from it. The recheck rule waits for the Bar.
- From Q1.2: both leave #977's sub-issues, with a comment on each. The Tickets this grill produces are a follow-up linking #977, and #977's Spec run completes without them.
- From Q1.3: `release.sh` keeps prompting on fail and on inconclusive. The goal for the Bar is that an Experiment of a change that changes nothing usually passes.
- From Q1.3: #1252's option 4, staying inconclusive and saying why, isn't the end state. Naming the reason for an inconclusive can still be added on top.
- From Q2.1: one A/A Experiment on the `thirdshift` host measures how often two runs of one setup disagree, once #1251 is merged. If `experiment` refuses identical Arms, allowing it is part of the work. It runs on the User config's Harness, Model and Effort, the setup releases are judged on. Until the new Bar ships, releases use today's Bar and its override.
- From Q2.2: an agent drafts `criteria` for #1252's 7 strong code-review candidates (#847, #686, #652, #1153 and the other three in its table), and you approve each before it can run. That brings code review to 17 tasks. The 5 partly visible candidates stay out.
- From Q2.3: a pass needs the lower bound at a looser **pass level** above the −10 margin, and a fail still needs the 95% upper bound below zero. Anything between is inconclusive. It changes the method's name from `paired-dirichlet-v1`, and gets an ADR. The pass level is set from Q2.1's measurement, against Q3.1's target.
- From Q2.3: the margin stays −10 points and one Bar covers every suite. A Bar per suite (#1252 option 2) and a smaller prior (option 3) are out.
- From Q3.1: the pass level is the highest one at which an Experiment of a change that changes nothing, at the measured disagreement and the suites' sizes, passes at least 80% of the time. It never goes below 70%. If 70% isn't enough, the suite grows.
- From Q3.2: the A/A Experiment waits until you've approved the 7 new code-review tasks, and measures the 17-task suite.
- From Q3.3: a Score is still decisive if any one flip changes the provisional decision. Borderline Scores are still always rechecked. A batch command records your decision on a reviewed pending list with one reason, and `experiments <id> --json` names the flip that makes each Score decisive. #1253's option 2 (sampling) is out, so no ADR for rechecks.
- From Q3.4: the Bar's two levels are the **pass level** and the **fail level**. `GLOSSARY.md`'s **Bar** entry now lists both (updated on `main` in this grill).
- From Q4.1: #1252 is rewritten as a Spec with Q4.1's 6 Tickets, using the `to-spec` and `to-tickets` skills under `docs/agents/triage-labels.md`'s publication procedure. #1253 is rewritten as Ticket 6 and attached to #1252. Tickets 3 and 4 are `ready-for-human`. #1252 stays `needs-triage` until every Ticket and blocking link is read back.
- From Q4.2: #1252 is blocked by #977, and its Tickets start once #977's Spec PR has merged to `main`.
- From Q1.3: #1253's option 1, counting only flips that make or undo a fail, loses its case. An inconclusive now stops a release, so a Score that can move a pass to inconclusive matters.

## Research results (2026-10-24)

- **How an Experiment's Verdict combines its suites.** Any suite on fail makes it a fail. Otherwise any suite on inconclusive makes it inconclusive. Otherwise it passes (`src/evals/experiment/grading.rs:300-311`, `issue-977`). So while code review is always inconclusive, a flip in Spec review that only moves Spec review from pass to inconclusive leaves the Experiment's decision as it was, and isn't decisive. #1253's 30 rechecks only appear once #1252 is fixed, or on an Experiment that doesn't select code review. Confirmed by reading the source.
- **The decisive rule.** A Score is decisive if flipping its `passed` or its `wrong_finding` changes the Experiment's provisional decision in any way (`src/evals/experiment/rechecks.rs:70-121`). Confirmed by source.
- **The prior.** It is 1 pseudo Eval task spread evenly over all 2·T+1 possible paired differences, so 7 values with 3 Trials per task, and 3 with 1 (`src/verdict.rs:206,271`). Confirmed by source.
- **The Bar's lower bound at 95%,** from a re-implementation of `paired-dirichlet-v1` (Python with NumPy, 20,000 draws, in the session scratchpad). Lower bounds are in points; a pass needs one above −10. "±k" means k tasks favour each Arm by the smallest step, so the Arms are equal on average.

  | Case | 95% | 90% | 80% |
  | --- | --- | --- | --- |
  | Code review, 10 tasks, identical | −12.8 | −9.0 | −5.7 |
  | Code review, 10 tasks, ±2 (one Trial in three) | −16.4 | −13.2 | −9.9 |
  | Spec review, 30 tasks, identical | −5.8 | −4.0 | −2.5 |
  | Spec review, 30 tasks, ±1 | −11.0 | −8.7 | −6.2 |
  | Spec review, 30 tasks, ±2 | −14.0 | −11.1 | −8.4 |
  | Spec review, 30 tasks, ±3 | −16.2 | −13.6 | −10.3 |

  At 95%, both intervals (lower, upper):
  - Code review, identical: 14 tasks −9.2/+9.6, 17 tasks −7.8/+7.9, 22 tasks −6.1/+6.2.
  - Code review, ±2: 17 tasks −10.4, 22 tasks −8.0.
  - Spec review with 10% of tasks going each way: 30 tasks −16.3, 60 tasks −11.5, 100 tasks −8.9.
  - Spec review, 30 tasks, one Candidate Trial worse and the rest equal: −12.8/+3.7, the same at prior 0.5 (−12.6).
  - A prior of 0.5 helps code review (10 tasks, identical: −9.2) but barely moves a Spec-review flip.

- **Rechecks under a pass level of 80%,** from the same re-implementation. Lower bounds:
  - Spec review, 30 tasks: identical −2.4, with one Candidate Trial flipped worse −8.5.
  - Spec review, ±1: −6.2, with one more flip −10.9.
  - Spec review, ±2: −8.5, with one more flip −12.8.
  - Code review, 17 tasks: identical −3.6, with one Trial flipped −6.3. At ±2, −6.2, and with one more flip −8.4.
- **No one has measured how often two runs of one setup disagree on the same task.** #429's replays ran one session per cell and say a one-bug difference "is within noise" (`docs/research/second-opinion-replays.md:191`, also 143 and 161). #549 had "one run per cell" (`docs/research/security-review-mode-trial.md:45,106`). #698's Q3.2 expected the first real Experiments to measure it (`.scratch/grill-with-docs/done/698-evals/round-3.md:24`). Confirmed by an agent reading the sources.
- **The closest proxies suggest the Arms will disagree on many tasks, not a few.** None of these is a same-setup rate.
  - Fresh Claude max and the same model with a checklist each caught 4 of 6 timing bugs, but disagreed on 2 of the 6 (`second-opinion-replays.md:67-77`).
  - Two Graders sorting one review's Specs counted 23 and 26 of 30 (line 145).
  - In the Spec replay, about 0.30 wrong findings per fresh Claude review, and 0.07 per Codex review (lines 126-134). The share of reviews with at least one wrong finding, which is the Bar's per-Trial flag, isn't reported.
- **#698 already knew the sizes were small.** With Miller's example variances, 9 Eval tasks of 3 Trials can reliably detect only about a 44-point difference. 20 points needs about 44 tasks, and 10 points about 175 (`.scratch/grill-with-docs/done/698-evals/research.md:100-102`, `round-4.md:36`). The 10-point margin was settled anyway (`settled.md:15`).
