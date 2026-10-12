# Second opinion replays

Research dates: 2026-10-10 to 2026-10-11, with two follow-ups on 2026-10-12. Implements the replay plan in [#429](https://github.com/JacobStephens2/thirdshift/issues/429), settled in its grilling log ([`.scratch/grill-with-docs/429-multi-family-review.md`](../../.scratch/grill-with-docs/429-multi-family-review.md), Q4–Q41). The background research is [multi-family-review.md](multi-family-review.md). The script that ran the sessions is beside this note, in [second-opinion-replays/](second-opinion-replays/).

**Question.** Does a **Second opinion** catch what thirdshift's own review lets through? A Second opinion is a review by another model family, or by the same model in a fresh session. Three places were tested, each against a bar fixed before any session ran (Q6, Q29, Q30):

| Piece | Replay | Bar |
|---|---|---|
| Second opinion in a Run | The 5 PRs behind 6 timing bugs that reached main (forward). The 4 PRs behind 3 bugs in Codex's work (reverse). | The other family catches at least 2 bugs the fresh same-family arm misses, and the checklist arm doesn't also catch them. If the checklist catches as many, add it to the review skill instead. If no arm catches more than 1, drop the idea. |
| Plan check | 20 plans Claude's Architecture reviews wrote (forward). 20 of the 27 Codex wrote (reverse). | It stops at least 3, and most of its stops are right. |
| Second opinion after the Spec review | All 30 Spec branches, as they stood before their recorded Spec review. | Real behaviour bugs the recorded review missed, in at least 6 of the 30 Specs, with no more than half of the graded findings wrong. |
| Spec review on its own Harness | The same 30 | The other family alone finds real behaviour bugs in at least as many Specs as the recorded review did. |

## TL;DR

- **Second opinion in a Run: fails, in both directions.** Forward, Codex caught 1 of the 6 timing bugs, and fresh Claude caught that one too. Reverse, Claude caught none of Codex's 3. The four new Harnesses caught 1 between them.
- **Timing checklist: no gain.** It caught 4, the same as fresh Claude without it, but not the same 4.
- **The review that matters is a fresh one.** Fresh Claude at max, on today's review skill, caught 4 of the 6 timing bugs. The recorded in-session reviews raised 2, and their authors declined both.
- **Plan check: passes only for Claude checking Claude's plans.** That arm stopped 10 of 20 plans, and 8 of the stops were right. Codex checking Claude's plans stopped 15, and 5 were right. On Codex's plans, which are the ones Weeding writes today, neither first arm passes. A follow-up with Claude at max passes exactly at the bar: 3 stops, 2 right, on the same two plans fresh Codex stopped rightly.
- **Second opinion after the Spec review: passes, for either family.** Codex found real missed bugs in 16 Specs, with 2 of 25 findings wrong. A fresh Claude session found them in 19, with 9 of 40 wrong. The two mostly find different bugs. A follow-up pilot of fresh Codex on Codex's own four Specs found real missed bugs in 2 of them, none wrong.
- **Spec review on its own Harness: fails.** Codex raised behaviour bugs in 22 Specs, against the recorded review's 23.

## Method

**Sessions.** 183 sessions ran between 2026-10-10 04:23 and 2026-10-11 20:12 UTC, one at a time. Each was a read-only review in a throwaway repository holding only the history up to the commit under review: the branch under review, plus a `base` branch for the comparison. No origin and no later commits were present. `gh` was signed out, with an empty `GH_CONFIG_DIR`, and the prompt forbade builds, tests, the network and any change to the repository (Q11).

Every session exited before its 2.5-hour timeout. None changed its repository: `HEAD` and `git status` were compared before and after each. One check that greps the sessions for `gh` use flagged two sessions, and both were quoted search patterns, not calls.

**Arms and settings.** Claude ran at max where it was the baseline Codex had to beat, and at high where it was the Second opinion being tried (Q31). Codex ran at xhigh throughout (Q9). Each new Harness ran at its best measured setting (Q17).

| Replay | Arm | Harness, Model, Effort | Sessions | Median minutes |
|---|---|---|---:|---:|
| Six timing bugs | fresh | Claude Code 2.1.296, Opus 5.5, max | 5 | 20.1 |
| | checklist | the same, with the [timing checklist](second-opinion-replays/timing-checklist.md) | 5 | 25.4 |
| | other | Codex 0.162.0, GPT-6.1 Sol, xhigh | 5 | 7.2 |
| | agy | Antigravity 1.3.3, Gemini 3.8 Flash, high | 5 | 11.8 |
| | grok | Grok Build 1.0.50, Grok 4.7, xhigh | 5 | 21.7 |
| | muse | Muse Code 1.4.4, Muse Spark 1.3, max | 5 | 7.0 |
| | opencode | OpenCode 2.0.24, MiMo-V2.6-Pro | 5 | 5.3 |
| Codex's three bugs | fresh | Codex, xhigh | 4 | 5.6 |
| | other | Claude, high | 4 | 6.0 |
| Claude's plans | other | Codex, xhigh | 20 | 5.3 |
| | fresh | Claude, max | 20 | 14.6 |
| Codex's plans | other | Claude, high | 20 | 3.4 |
| | fresh | Codex, xhigh | 20 | 4.6 |
| Spec branches | other | Codex, xhigh | 30 | 4.9 |
| | fresh | Claude, max | 30 | 19.3 |
| Follow-up: Codex's plans | othermax | Claude, max | 20 | 15.4 |
| Follow-up: Codex's Spec branches | fresh | Codex, xhigh | 4 | 9.5 |

**Prompts.** The bug replays used today's review skill, `thirdshift-code-review` with #486's evidence-backed findings, in every arm (Q26). The Spec replay pinned the skill at 479a1fb, the version the recorded Spec reviews ran. The plan check re-applied the Architecture review's four Strong tests to the published plan, reading the plan, `CONTEXT.md`, the ADRs and the code as they stood when the plan was made (Q2, Q10). A fresh agent that hadn't seen the bugs wrote the timing checklist from `CONTEXT.md` and the ADRs alone (Q12). Codex arms were told to start their sub-agents fresh, as #616 makes the factory do (Q28).

**The Spec replay's fresh arm was added after the fact.** The plan had only the Codex arm. Codex passed the bar, but elsewhere a fresh Claude session had matched or beaten it. So 30 fresh Claude sessions at max ran over the same branches, to tell a second family apart from a second look.

**Grading.**

- **Bugs:** graded against each bug's known fix. Each bug has a written catch criterion that a finding must meet: it has to name the defect and tie it to its consequence. C is caught. P is partial and doesn't count. M is missed. Each PR's reports were graded in one packet, with the arms behind shuffled labels and the run paths stripped. One fresh grader read each packet, and borderline grades were rechecked by hand.
- **Plans:** fresh agents graded every stop without knowing which arm made it. Labels were shuffled, and lines that would identify Codex were removed. A stop counts as right if the grader would also stop that plan.
- **Spec findings:** sorting agents listed each arm's behaviour-bug findings and marked whether the recorded review had raised each one. Fresh agents then graded every missed finding against the code at the reviewed commit, as real, wrong or unsure. A last pair of agents checked each arm's real findings against the other arm's full reports.

Q6 had Jacob grading the plans and the Spec findings. At his request, fresh agents did instead. All graders are Claude agents.

## Second opinion in a Run

**Six timing bugs (forward).** Forward means Codex reviewing Claude's work.

| Bug | PR | fresh (Claude max) | checklist (Claude max) | other (Codex xhigh) | recorded review |
|---|---|---|---|---|---|
| [#24](https://github.com/JacobStephens2/thirdshift/issues/24) SIGHUP skips the Failed run path | #19 | M | C | M | no |
| [#57](https://github.com/JacobStephens2/thirdshift/issues/57) Base moves during the CI wait | #22 | M | M | M | no |
| [#102](https://github.com/JacobStephens2/thirdshift/issues/102) a CI-fix Repair with no commit repeats | #22 | C | M | M | raised, declined |
| [#175](https://github.com/JacobStephens2/thirdshift/issues/175) conflict check against a moved ref | #17 | C | C | M | no |
| [#182](https://github.com/JacobStephens2/thirdshift/issues/182) Foreign commits during a Repair | #99 | C | C | C | no |
| [#303](https://github.com/JacobStephens2/thirdshift/issues/303) a Spec with every Ticket closed stalls Pickup | #293 | C | C | M | raised, declined |
| **Caught** | | **4** | **4** | **1** | **2, both declined** |

Codex's one catch, #182, was also caught by fresh Claude, so Codex added nothing Claude missed. The bar needed 2. The checklist arm caught #24, which fresh Claude missed, but missed #102. It marked its item about repeated steps "N/A" for #22. Q6's rule, "add the checklist to the review skill instead if it catches as many", was written as the alternative to a passing Second opinion: the checklist catching what the other family catches. No Second opinion passed. The checklist caught no more than the same model without it, 4 each but not the same 4, so this replay shows no gain from it. The drop rule doesn't apply either, because two Claude arms caught more than 1.

**Codex's three bugs (reverse).** Reverse means Claude reviewing Codex's work.

| Bug | PRs | fresh (Codex xhigh) | other (Claude high) | the factory's review |
|---|---|---|---|---|
| [#559](https://github.com/JacobStephens2/thirdshift/issues/559) a 2-second bound times the whole Run | #449 | M | P (the same test, for the wrong reason) | no |
| [#603](https://github.com/JacobStephens2/thirdshift/issues/603) a reproduction failure reads "audit failed" | #562, #577 | M | P (the Audit line for failures before the audit) | no |
| [#607](https://github.com/JacobStephens2/thirdshift/issues/607) no reason for a deliberately incomplete review | #592 | M | M | no |
| **Caught** | | **0** | **0** | **0** |

Claude came closest on two of the three, but neither partial meets its criterion. #603 is only partly visible in these PRs: a third PR, #566, added the test that expects "audit failed".

**The new Harnesses (forward, Q17).**

| Bug | Grok 4.7 xhigh | Muse Spark 1.3 max | MiMo-V2.6-Pro | Gemini 3.8 Flash high |
|---|---|---|---|---|
| #24 | M | M | M | M |
| #57 | M | M | M | M |
| #102 | M | M | M | P |
| #175 | P | M | M | M |
| #182 | C | P | M | P |
| #303 | M | M | M | P |
| **Caught** | **1** | **0** | **0** | **0** |

None reaches the bar. Grok's one catch is #182, which fresh Claude also caught.

**What the bug replays do show.** Fresh Claude caught 4, while the recorded reviews raised 2 and their authors declined both. This replay changed two things at once, so the 4 can't be split between them:
- **A fresh session.** The recorded reviews ran as sub-agents of the implement session, and the author then judged their findings.
- **Today's review skill.** It asks for evidence and for lists of files read, which the recorded reviews' skill didn't.

Neither is a Second opinion from another family.

## Plan check

| Plans | Arm | Stopped | Right | Bar (at least 3, most right) |
|---|---|---:|---:|---|
| Claude's 20 | other (Codex xhigh) | 15 | 5 | fails: 10 of 15 wrong |
| Claude's 20 | fresh (Claude max) | 10 | 8 | **passes** |
| Codex's 20 | other (Claude high) | 2 | 1 | fails: under 3 |
| Codex's 20 | fresh (Codex xhigh) | 4 | 2 | fails: half, not most |
| Codex's 20 | othermax (Claude max), follow-up | 3 | 2 | **passes**, exactly at the bar |

Twenty-two plans were stopped by at least one arm: 17 of Claude's and 5 of Codex's. Both forward arms stopped 8 of Claude's plans. The grader agreed with both stops on 5, with only Claude's on 2, and with neither on 1. Codex stopped three-quarters of Claude's plans, and most of those stops didn't hold.

Of the first four arms, the one that passed checks Claude's plans with Claude. Weeding has written its plans on Codex since 6 October, and on Codex's plans neither of the first two arms passed. Claude had checked those only at high, so a follow-up ran Claude at max over them (Q34); see "Follow-ups". It passes with nothing to spare, and its two right stops are the plans fresh Codex also stopped rightly. Both passing arms have Claude at max as the checker.

## Second opinion after the Spec review

| | Codex xhigh | Fresh Claude max |
|---|---:|---:|
| Behaviour bugs the recorded review missed | 25 | 40 |
| Graded real | 22 | 31 |
| Graded wrong | 2 | 9 |
| Unsure | 1 | 0 |
| Specs with a real missed bug (bar: 6) | **16** | **19** |

Both pass: at least 6 Specs, and well under half wrong. Most of fresh Claude's 9 wrong findings were behaviour that the Spec, or the branch's own README, asks for. Codex's 2 wrong were the same kind. Many real findings are minor: rare races, storage failures, edge cases in parsing.

**Where the two arms overlap.** Each arm's real findings were checked against the other arm's full reports. A partial counts as not raised, as in bug grading.

| | Real | The other arm raised it | Only this arm |
|---|---:|---:|---|
| Codex | 22 | 9 (plus 6 partial) | 13 bugs, in 10 Specs |
| Fresh Claude | 31 | 8 | 23 bugs, in 18 Specs |

So a second look after the Spec review finds real bugs whichever family takes it. Fresh Claude reaches more Specs and finds more bugs. Codex is wrong less often. Each finds a lot the other doesn't. There was one fresh Claude review per Spec, so this doesn't show whether Codex's 13 exceed what a second fresh Claude review would add by chance.

**Spec review on its own Harness.** Q6 compared against "about 15" Specs. The sorting found the recorded reviews raised behaviour bugs in 23 of the 30, and Codex alone raised them in 22. Those counts are the sorting agents' and weren't graded, except the missed findings above. Two sorting passes also disagreed on the recorded count: Codex's sorter found 23 and fresh Claude's found 26. Either way, Codex alone doesn't reach the recorded review's count. Swapping would also give up the recorded review's own catches, so this option isn't offered.

**Details withheld.** The graded findings name bugs in six repositories. Some are private, and some findings are still unfixed. They stay in the private grading files under `~/.thirdshift/replays/2026-10-09/grading/` and aren't listed here.

## Follow-ups (2026-10-12)

Two runs added after the write-up merged, each with the same script, isolation and grading prompts as the arm it extends. Both ran 2026-10-12, the plans from 02:16 to 07:43 UTC and the Specs from 02:30 to 03:08 UTC, one session at a time per Harness. All 24 sessions exited 0, and none changed its repository.

**Claude at max checks Codex's 20 plans** (Q34). Claude at high had stopped only 2. At max it stopped 3, and a fresh agent graded them blind against each plan's base commit.

| Plan | Stop | Reason |
|---|---|---|
| #463 | right | Decision 10 lets a forwarded interruption override a child that reached its goal, so a merged Ticket would be announced as interrupted and a merged Base fix reported as not merged. |
| #465 | right | Decision 4 lets an interruption win over every Git and GitHub step's exit status, so a step that changes state and finished anyway (a worktree add, a Claim label) is left with nothing to clean it up. |
| #499 | wrong | A tag can shadow the Base branch's short name, but the plan already asks for full ref names, and a builder fixes it in one line. |

That passes, exactly at the bar. Fresh Codex stopped #463 and #465 for the same reasons, and Claude at high stopped #465. So on Codex's plans every arm's right stops fall on the same two plans, and Claude at max adds one fewer wrong stop (1 against 2), not a new catch. With one session per cell, that difference is within noise.

**Fresh Codex on Codex's own Specs** (Q35). Codex had run five Spec reviews over four Specs, all on thirdshift: #432, #465, #486 and #427. #486's second review saw the first one's fix, so only the first is replayed. Each session used the review skill its recorded review ran: the older one for #432, #465 and #486, the evidence-backed one for #427. The same sorting and blind grading as the Spec replay followed.

| Spec | Real behaviour bugs the recorded review missed |
|---|---|
| #432 | 1, minor: a process a stopping CLI starts in a new session escapes the stop. |
| #427 | 2. The Security fix notification loses how the audit, reproduction and fix stages ended, and a failed reproduction reads "audit failed". That second half is #603, which reached main and was fixed on 9 October. The other is a minor gap on one `security-fix` path. |
| #465, #486 | None. Fresh Codex also missed the bug #486's recorded review found and fixed. |

So 2 of the 4 Specs, with 3 findings, all real and none wrong. Neither reverse bug-replay arm caught #603 in the Ticket PRs it came from (#562, #577). Four Specs can't meet the bar of 6 in 30, so this is a pilot. It's the pairing a Second opinion on the Spec run's own Harness would run today.

## What passes

| Piece | Result | Next |
|---|---|---|
| Second opinion in a Run, Codex reviewing Claude | Fails: 1 caught, 0 unique | Not built |
| Second opinion in a Run, Claude reviewing Codex | Fails: 0 caught | Not built |
| Timing checklist in the review skill | Ties fresh Claude | Not added |
| New Harnesses as a Second opinion | Fail: 1, 0, 0, 0 caught | Not suggested |
| Plan check | Passes only with Claude at max as the checker: on Claude's plans (10 stops, 8 right), and on Codex's exactly at the bar (3 stops, 2 right) | Undecided, in the follow-up grilling (Q41) |
| Second opinion after the Spec review | Passes for Codex (16 Specs) and fresh Claude (19) | Build it as opt-in User config. Family is a setting, and the follow-up grilling picks the default |
| Spec review on its own Harness | Fails: 22 vs 23 Specs | Not offered |

Q33 left two questions for after the replays. Who acts on a Second opinion's findings? What happens when a plan check disagrees? A short grilling settles them for each piece that passed, before `/to-spec`.

## Caveats

- **Small samples.** Six bugs, three bugs, 20 plans and 30 Specs only show large effects (Q6). The reverse Spec replay still waits: Codex has run only four Specs (Q16). The pilot on those four can't meet the bar of 6.
- **Claude graded Claude.** Every grader was a Claude agent, and Claude is one of the families compared. Blind labels hide which arm wrote a finding, but Codex and Claude reports differ in style.
- **One session per cell.** Run-to-run variation wasn't measured. A tie, or a one-bug difference, is within noise.
- **The bug replays changed two things together.** Fresh sessions and today's skill (Q26), so the fresh arm's gain over the recorded reviews can't be put down to either alone.
- **The Spec replay's fresh arm was added late**, after Codex's results were known. Its sessions, prompt and grading match the Codex arm's.
- **Partial catches don't count.** Claude's two partials on Codex's bugs, and agy's three, would change no result.

## Found along the way

- **agy 1.3.3 changed `-p`.** Since 9 October, `-p` takes the prompt as its value. The first five agy sessions exited at once with `-p took "--dangerously-skip-permissions" as its prompt`. The replay script now puts the prompt right after `-p`. thirdshift's own adapter (`src/harness/agy.rs`) put it last, so factory agy sessions failed the same way. Filed as #658 and fixed by #661.
- **The `gh` check's false positives.** It treats `|gh ` inside a quoted search pattern as a call. Both flags were checked by hand.

## Reproducing

[`replay.py`](second-opinion-replays/replay.py) builds each unit's repository and prompt, runs one Harness's pending units in order, and stops at a limit warning. [`stages.sh`](second-opinion-replays/stages.sh) runs the stages one after another, and pauses the Weeding or Security crontab line while a stage that shares its limits runs. The inputs are the manifests under `~/.thirdshift/replays/`. They were built from git, the Session logs and the PRs, and aren't in this repository. `replay.py show <unit>` prints a unit's exact prompt and command.
