# Grilling: #1516, Grow the code-review suite to 20 discriminating Eval tasks

Issue: https://github.com/JacobStephens2/thirdshift/issues/1516
Related: #1434 (closed; its A/A `83721fbb…`), #1477, #1478, #1480, #1481, #1494, #1498
Research: the private pool survey at `~/.thirdshift/evals/notes/1516-pool-survey.md` on the `thirdshift` host (it names private repositories, so only counts appear here)

The triage comment on #1516 left nine open questions. Round 1 asks them, adjusted by what the three other hosts showed (see "Research results"). Q8 of the triage comment is answered by a fact and is not asked: voice-agent is personal work.

## Round 1 (open)

**Q1.1 - Call new tasks "proposed Eval tasks"?** The issue body says "candidates", which collides with the **Candidate arm** in the same Experiments that measure them. An Eval task's manifest already has a `state`, and `proposed` is the state a new task starts in before `approved`. The options:
- (a) Say **proposed Eval task** in the issue and in results, with no glossary change: it is an Eval task in one of its states, not a new concept.
- (b) Add a glossary term for it.
- (c) Keep "candidate".

*Recommended:* (a). The glossary already has both words it needs, and a state isn't a term.

*Answer:* 

**Q1.2 - Start before #1481 is done?** #1477, #1478, #1480 and #1481 are all still open, and the host runs 0.17.3. Only recording Difficulty (`--record-difficulty`, #1478) and parking out-of-band tasks need #1477's release. Proposing, dry-running with `evals prepare`, approving and running the A/A all work on 0.17.3 today, and the pooled A/A results can have their Difficulty recorded once #1478 ships. The options:
- (a) Start now. #1481 blocks only recording Difficulty and parking, and the A/A's results are kept so Difficulty can be recorded from them later.
- (b) Wait for #1481, as the issue body says.

*Recommended:* (a). It takes the ~6-hour A/A off the critical path. The risk is that #1478's recording can't read an Experiment that ran before it shipped; Q2 would ask what to do if research shows that.

*Answer:* 

**Q1.3 - A/A the new tasks in a staging suite?** Running the A/A on all of `code-review` re-measures its 21 existing tasks too, about 120 Trials per round for nothing. A staging suite, say `code-review-proposed`, holds only the new tasks. Release mode picks suites by their `[dependencies]` paths (`src/evals/suite.rs`, `touched`), so a staging suite with no dependencies is never picked by a release. The options:
- (a) A staging suite `code-review-proposed`, with no `[dependencies]`, run at 3 Trials per Arm. Discriminating tasks then move into `code-review`; the rest are parked or dropped.
- (b) Add new tasks straight to `code-review` and A/A the whole suite each round.

*Recommended:* (a).

*Answer:* 

**Q1.4 - Measure on codex · gpt-6.1-sol · xhigh, both Arms on current `main`?** That is the setup of #1434's A/A `83721fbb…` and #1477's grill Q2.4, so new Difficulties are comparable with the 7 tasks already measured. The options:
- (a) Yes, the same setup, both Arms on whatever `main` is when the round starts.
- (b) The User config's current Harness, Model and Effort, whatever they are then.

*Recommended:* (a). Comparability matters more than tracking the config.

*Answer:* 

**Q1.5 - Write the host procedure into the issue's steps?** Two open bugs can spoil an A/A on the same host: #1494 (scheduled Passes start Runs during an Experiment, so Trials time out) and #1498 (`thirdshift update` mid-Experiment stops it). And #1434 found that `experiment` refuses a PR URL in `[trial] issue_url` that `prepare` accepts. The steps would say:
- lower `pickup.limit` for the A/A (#1494);
- no `thirdshift update` while it runs (#1498);
- `[trial] issue_url` is an `/issues/N` URL.

*Recommended:* yes, all three, each removable once its bug is fixed.

*Answer:* 

**Q1.6 - No cap on cross-repository tasks, but report the mix each round?** In #1434, 4 of 6 cross-repository tasks were discriminating against 3 of 14 thirdshift ones, so they are the better yield. A suite heavy with them tests the review on code thirdshift doesn't know as well, which is arguably closer to how it's used. The options:
- (a) No cap; report the thirdshift / other split in each round's results.
- (b) Cap them, e.g. at half the suite.

*Recommended:* (a).

*Answer:* 

**Q1.7 - Leave employer repositories out of the first round?** What the hosts show (see "Research results"): `orchestrate` and `dev` have no Eval data and no Bug origins at all, because no Scoring run has ever run there, so there is no ready pool on the employer side. tourbot has about 108 fix-like merged PRs out of 985, so its pool is real but would have to be mined from scratch. Putting employer code on `thirdshift.app`, a personal host, is a data question for your employer, not just an Eval question. The options:
- (a) Not in round 1. Mine the personal pool first; decide on employer code only if round 1 leaves a gap.
- (b) Allow it now, cloning tourbot onto `thirdshift.app`.
- (c) Allow it, but build and run employer tasks on an employer host (`dev` or `orchestrate`) in a suite there, never copying the code to `thirdshift.app`.
- (d) Never.

*Recommended:* (a). The personal pool's estimate is 9–12 of 13 needed, so round 1 is worth running anyway, and its yield tells us how big the gap really is. If one is left, (c) avoids the data question, though it means a second suite on another host, which Q2 would have to square with "the `code-review` suite has 20".

*Answer:* 

**Q1.8 - Clone voice-agent onto the host?** voice-agent is `JacobStephens2/voice-agent`, public, described as a "Personal voice-first coding and operations agent", so it's personal work (this answers the triage's Q8). It's the second-largest source in the survey (22 unused Bug origins, 15 naming a PR), but it isn't cloned under `~/repos` on `thirdshift.app`, and an Eval task's `repository` is a local path. The issue body's "other repositories already cloned" would exclude it. The options:
- (a) Clone it into `~/repos/voice-agent` and include it as a source.
- (b) Only already-cloned repositories, as the body says.

*Recommended:* (a). It's about a quarter of the expected yield.

*Answer:* 

**Q1.9 - Confirm a Bug origin when the Eval task built from it is approved?** All 121 Bug origins on the host are `proposed`. Approving a task means reading its Spec, base and change, which checks the same claim a confirmation does (`thirdshift bug-origins <issue> --confirm`). The options:
- (a) Yes: approving the task confirms its Bug origin, and a task found to rest on a wrong origin uses `--correct <pr>` or `--none` instead.
- (b) Keep the two separate.

*Recommended:* (a).

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- Nothing yet: this is Round 1. Already settled outside this grill: the band 0.15–0.95, at least 6 Trials, the threshold of 20 (#1434 grill Q1.1–Q1.5), and the scope rule (a Spec line or documented standard covers the target bug).

## Research results (2026-10-28)

- `thirdshift.app`: 121 Bug origins, all `proposed`; 99 unused as Eval tasks, 59 of those naming an introducing PR. `code-review` holds 21 task directories. Suites were backed up on 2026-10-26 and 2026-10-27 (three `suites.bak-*` folders). Confirmed by reading `~/.thirdshift/evals/` and the survey file.
- voice-agent is `JacobStephens2/voice-agent`, public, "Personal voice-first coding and operations agent"; it is not cloned on `thirdshift.app` or `stephens.page` (which has only `voice-agent-scheduling`). Confirmed by `gh repo view` and `ls ~/repos` on both.
- `orchestrate` and `dev` (employer hosts, 0.17.3) have no `~/.thirdshift/evals`, so no suites, Scoring runs or Bug origins; both have Run logs for tourbot and orchestration. `stephens.page` has no evals either. Confirmed by `ls` on each.
- tourbot: 985 merged PRs, about 108 with "fix", "bug" or "regress" in the title; orchestration: 405 and 9. Confirmed by `gh pr list`.
- A suite is picked by release mode only when a changed path matches its `[dependencies]` (`src/evals/suite.rs`, `touched`), so a suite without dependencies isn't picked. Confirmed by source.
- #1477, #1478, #1480, #1481, #1494 and #1498 are all open. Confirmed by `gh issue view`.

## Waiting on research

- Nothing yet.
