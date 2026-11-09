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

**Q1.2 - Start before #1481 is done?** *Revised 2026-11-09:* #1477, #1478 and #1480 have shipped, and every host runs 0.24.0, which has `thirdshift experiments <id> --record-difficulty`. So #1481 is no longer waiting on a release, only on someone doing it, and no code-review task has a recorded Difficulty yet. #1481's own steps are short for code review: record Difficulty from `83721fbb…` and park 13 tasks. Proposing, dry-running with `evals prepare`, approving and the new A/A don't touch those 20 tasks. The options:
- (a) Start now, in parallel with #1481. Do #1481's code-review half (record and park) first, since it takes minutes, and its Spec-review A/A independently.
- (b) Wait until all of #1481 is done, Spec review's ~180-Trial A/A included, as the issue body says.

*Recommended:* (a). Only the Spec-review A/A in #1481 takes hours, and #1516 doesn't depend on it. The two A/As must not run on the host at the same time (Q1.5's load concern).

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

**Q1.7 - Leave employer repositories out of the first round?** What the hosts show (see "Research results"): the three employer hosts have no Eval data and no Bug origins at all, because no Scoring run has ever run there, so there is no ready pool on the employer side. But it is the busiest source of Ticket runs outside thirdshift: about 180 Runs that opened a PR on the employer's main repository since mid-October, and about 47 of its 300+ PRs merged since then have fix-like titles. Its pool is real but would have to be mined from scratch. Putting employer code on `thirdshift.app`, a personal host, is a data question for your employer, not just an Eval question. The options:
- (a) Not in round 1. Mine the personal pool first; decide on employer code only if round 1 leaves a gap.
- (b) Allow it now, cloning the employer repositories onto `thirdshift.app`.
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
- `orchestrate` and `dev` (employer hosts) have no `~/.thirdshift/evals`, so no suites, Scoring runs or Bug origins; both have Run logs for the employer's repositories. `stephens.page` has no evals either. Confirmed by `ls` on each.
- The employer's main repository: 985 merged PRs, about 108 with "fix", "bug" or "regress" in the title; its second: 405 and 9. Confirmed by `gh pr list`.
- A suite is picked by release mode only when a changed path matches its `[dependencies]` (`src/evals/suite.rs`, `touched`), so a suite without dependencies isn't picked. Confirmed by source.

## Research results (2026-11-09, every host in `scripts/update-hosts.conf`)

Details, with repository names, are appended to the private survey file on the `thirdshift` host. Counts only here.

- **Hosts checked:** `thirdshift.app`, `stephens.page`, the three employer hosts and the Mac, all on 0.24.0 except the Mac (0.18.7). The `lima:thirdshift` VM was stopped and not checked. Only `thirdshift.app` has any Eval data. Confirmed by `ls ~/.thirdshift` on each.
- **The suite hasn't changed:** `code-review` still has 21 tasks, 20 `approved` and 1 `proposed`, and none has a recorded Difficulty. #1477, #1478 and #1480 are closed; #1481, #1494 and #1498 are open. Confirmed by reading the task manifests and `gh issue view`.
- **Bug origins: 148, up from 121**, all `proposed`. The W45 Scoring run added 27. Those not yet used as an Eval task and naming an introducing PR: **82, up from 59** (thirdshift 60, voice-agent 14, five other personal repositories 9). About 5 of W45's 16 thirdshift ones are test flakes, so likely out of scope. A packets folder dated today suggests the W46 run is under way. Confirmed by reading `~/.thirdshift/evals/scoring/*.json` and the suite, `parked` and `dropped` folders.
- **Scoring already reaches repositories beyond `thirdshift.app`:** Bug origins exist for personal repositories that only `stephens.page` works on. But four of those repositories have about 85 Runs with a PR between them and almost no fix-like merges, so they add little. Confirmed by `activity.log` on each host and `gh search prs`.
- **thirdshift since the W45 window (2026-11-02):** 86 merged PRs, 11 with fix-like titles, not yet mined. Confirmed by `gh search prs`.
- **Ticket runs that opened a PR, by host, since mid-October:** `thirdshift.app` about 380 (thirdshift 207), `stephens.page` about 270 (thirdshift 103), the employer hosts about 300 (about 180 on the employer's main repository), the Mac a handful. Confirmed by `activity.log`.
- **Revised yield:** at the survey's scope rate (about 5 in 12) and #1434's discriminating rate (7 in 20), 82 PR-backed Bug origins give roughly 34 in scope and about **12 discriminating**, against 13 needed. Thirdshift fixes since 2026-11-02 and the W46 run could close the gap without employer code, but only just.

## Waiting on research

- Nothing yet.
