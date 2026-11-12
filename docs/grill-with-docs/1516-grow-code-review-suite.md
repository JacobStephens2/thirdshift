# Grilling: #1516, Grow the code-review suite to 20 discriminating Eval tasks

Issue: https://github.com/JacobStephens2/thirdshift/issues/1516
Related: #1434 (closed; its A/A `83721fbb…`), #1477, #1478, #1480, #1481, #1494, #1498, #1484 and its host Ticket #1887 (a process-task suite mined from the same thirdshift pull requests), #1876, #1531, #1979 (the `factory` Experiment host), #2106

**Resuming this grill elsewhere:** Round 1 is open and unanswered. Before writing Round 2, re-run the host survey (every host in `scripts/update-hosts.conf`, now including `factory`) and re-check the issues named above, since several closed while Round 1 waited. Detailed counts with repository names are in the private survey file on `thirdshift.app`.
Research: the private pool survey at `~/.thirdshift/evals/notes/1516-pool-survey.md` on the `thirdshift` host (it names private repositories, so only counts appear here)

The triage comment on #1516 left nine open questions. Round 1 asks them (Q1.10 and Q1.11 were added on 2026-11-09 from newer issues), adjusted by what the three other hosts showed (see "Research results"). Q8 of the triage comment is answered by a fact and is not asked: voice-agent is personal work.

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

*Recommended:* (a). Only the Spec-review A/A in #1481 takes hours, and #1516 doesn't depend on it. The two A/As must not run on the same host at the same time.

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

**Q1.5 - Write the host procedure into the issue's steps?** *Revised 2026-11-12:* the two bugs this question was about are fixed. #1494 (PR #1982) makes scheduled Passes skip while an Experiment is live, and makes a starting Experiment wait for live Runs unless given `--no-wait`. #1498 (PR #1985) keeps Trial launches working across `thirdshift update`. Both are in 0.30.0, which `thirdshift.app` and the factory host now run. What's left is #1434's finding that `experiment` refuses a PR URL in `[trial] issue_url` that `prepare` accepts. The options:
- (a) The steps say only: `[trial] issue_url` is an `/issues/N` URL, and the host runs 0.30.0 or later.
- (b) Also keep a manual pause of the crontab, as a belt-and-braces step.

*Recommended:* (a). The Pass skip was verified live on the factory host (#1979's closing comment).

*Answer:* 

**Q1.6 - No cap on cross-repository tasks, but report the mix each round?** In #1434, 4 of 6 cross-repository tasks were discriminating against 3 of 14 thirdshift ones, so they are the better yield. A suite heavy with them tests the review on code thirdshift doesn't know as well, which is arguably closer to how it's used. The options:
- (a) No cap; report the thirdshift / other split in each round's results.
- (b) Cap them, e.g. at half the suite.

*Recommended:* (a).

*Answer:* 

**Q1.7 - Leave employer repositories out of the first round?** What the hosts show (see "Research results"): the three employer hosts have no Eval data and no Bug origins at all, because no Scoring run has ever run there, so there is no ready pool on the employer side. But it is the busiest source of Ticket runs outside thirdshift: about 180 Runs that opened a PR on the employer's main repository since mid-October, and about 47 of its 300+ PRs merged since then have fix-like titles. Its pool is real but would have to be mined from scratch. Putting employer code on `thirdshift.app`, a personal host, is a data question for your employer, not just an Eval question.

*Revised 2026-11-12:* the new Experiment host from #1979, `factory`, is employer infrastructure: the employer's DigitalOcean account and domain, and its config in the employer's `orchestration` repository. It already clones and runs factory work on two employer repositories. So if Q1.10 moves `code-review` there, employer tasks can sit in the same suite without copying employer code anywhere new. The reverse question then arises: personal repositories' code on an employer host. thirdshift and most of the personal sources are public; two of the personal repositories with Bug origins are private. The options:
- (a) Not in round 1. Mine the personal pool first; decide on employer code only if round 1 leaves a gap.
- (b) Allow it now, cloning the employer repositories onto `thirdshift.app`.
- (c) Allow it now, on `factory` only, in the same `code-review` suite (needs Q1.10 (a)).
- (d) Never.

*Recommended:* (c) if Q1.10 is (a), otherwise (a). On `factory` the data question is already settled by where the code lives, and employer tasks are the biggest untapped source. Keep private personal repositories off `factory` unless you say otherwise; their few Bug origins aren't worth it.

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

**Q1.10 - Which host runs this suite's A/A?** *Added 2026-11-09, revised 2026-11-12.* #1484's grill (Q1.8) and `docs/grill-with-docs/run-capacity.md` put Experiments on a new host, and #1979 has now provisioned it: `factory`, 8 dedicated vCPU / 32 GB, Ubuntu 24.04, 0.30.0, with Passes that skip during a live Experiment. Its `~/.thirdshift/evals/suites` is empty so far; #1887 will put the process suite there. It runs factory work too, for thirdshift and two employer repositories, and it's in `scripts/update-hosts.conf`. `code-review` still lives on `thirdshift.app` (4 vCPU / 8 GB), where #1494's contention spoiled Trials in #1434's A/A. #2106 (open) may move `factory` to Ubuntu 26.04, which would matter mid-batch. The options:
- (a) Move `code-review` (and its staging suite) to `factory`, after a backup, and run all its Experiments there, including releases'. Clone the personal source repositories there as needed (see Q1.7).
- (b) Keep it on `thirdshift.app`, now that 0.30.0 skips Passes there too.
- (c) Build and screen on `thirdshift.app`, where the Bug origins are, and run only the A/As on `factory`.

*Recommended:* (a). One Experiment host keeps every suite's Difficulty measured under the same conditions, and it has twice the cores. Bug origins are written by Scoring runs on `thirdshift.app`, so proposing still reads them there; copy each proposed task across. Don't start an A/A batch while #2106's OS move is pending on `factory`.

*Answer:* 

**Q1.11 - Mine thirdshift's fixes once, for both this suite and #1887's?** #1887 proposes process tasks from recent merged thirdshift pull requests, and #1876 adds a command that drafts them. This issue proposes code-review tasks from the Bug origins of the same fixes. One fix can feed both: its own pull request becomes a process task (implement the fix), and the pull request that introduced the bug becomes a code-review task (catch it in review). The two screens differ: a process task needs tests in separate files that fail at the base, and a code-review task needs a Spec line or standard that covers the bug. The options:
- (a) Keep the two mining passes separate, but record in each suite's notes which fixes the other already used, so screening work isn't repeated.
- (b) One pass that screens each fix for both suites at once.
- (c) Fully independent.

*Recommended:* (a). The two suites are built at different times, and their screens share little, so a joint pass would only slow the first suite to reach 20. A shared list costs nothing and stops two people reading the same fix twice.

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

## Research results (2026-11-09, newer issues)

- `pickup.limit` is ignored in 0.24.0, with a User config warning; `pickup.failure_limit` (default 3) counts open issues the factory failed on. Confirmed by source (`src/config.rs`) and the warning in a host's `activity.log`.
- #1484's grill (Q1.8, Q4.3) put process-suite Experiments on a new, bigger host, with the Lima VM building and screening first; #1887 (opened today) is its host Ticket and mines recent merged thirdshift pull requests. #1876 adds a command that drafts process tasks from merged pull requests. Confirmed by `docs/grill-with-docs/done/1484-implement-suite.md` on `main` and `gh issue view`.
- (Superseded on 2026-11-12: the host exists; see below.)
- #1531 (open) would split the code review out of the implement Step. It changes the Ticket run, not the code-review session this suite runs, so it doesn't change this issue. Confirmed by reading #1531.

## Research results (2026-11-12)

- **`factory` exists** (#1979, closed): `factory.etadventures.com`, 8 vCPU / 32 GB, Ubuntu 24.04.5, thirdshift 0.30.0, weekly backups, Console in the Lima VM's Fleet, and in `scripts/update-hosts.conf`. It has `~/.thirdshift/evals/` with no suites and no Scoring files yet, clones thirdshift and two employer repositories, and runs Passes for them from cron. Its config lives in the employer's `orchestration` repository. Confirmed by ssh, `crontab -l` and #1979's closing comment.
- **#1494 and #1498 are fixed** (PRs #1982 and #1985), released in 0.29.2 / 0.30.0. Passes skip while an Experiment is live, verified on `factory` with an Activity log line. Confirmed by `gh` and `git tag --contains`.
- **`thirdshift.app`** runs 0.30.0. Its `code-review` suite is unchanged (21 tasks), and it still has the only Scoring files (3). Confirmed by ssh.
- **#2106** (open) proposes moving `factory` to Ubuntu 26.04, which its own text flags as a risk to A/A comparability mid-batch. #1481 and #1887 are still open. Confirmed by `gh issue view`.
