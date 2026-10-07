# Grilling: #429, multi-family review

Issue: https://github.com/JacobStephens2/thirdshift/issues/429
Research: `docs/research/multi-family-review.md`, `docs/research/model-strength-for-review.md`

## Round 1 (answered)

**Q1 - What job would the second opinion do?** You don't review factory PRs, so "catch what you catch now" was out. The options:
- (a) **Catch what slips through.** A second reviewer finds what the first misses, aimed at the timing bugs that reached main.
- (b) **Rule on the declined findings** before thirdshift merges.
- (c) **Let thirdshift mix models,** as a capability.
- (d) **None for now.**

*Recommended:* (a), if a re-run of the review on the six timing bugs supports it; (d) if nothing catches them.

*Answer:* agree

**Q2 - What does "in Architect runs" mean?** The options:
- (a) **A plan check:** another model checks the review's Strong call before thirdshift marks the plan ready.
- (b) **Only the code the plan produces.**
- (c) Both.
- (d) Neither.

*Recommended:* (a), after a replay of the plan check over a sample of past plans.

*Answer:* agree

**Q3 - Where in a Spec run?** The options:
- (a) **A second opinion after the Spec review.**
- (b) **A second opinion in every Ticket's Run.**
- (c) Both.
- (d) **The Spec review itself on another Harness, Model and Effort.**

*Recommended:* (d), after a replay over the 30 past Spec reviews.

*Answer:* I want the ability to do a, b, and/or d depending on User configuration I think.

## Round 2 (answered)

**Q4 - Do the replays decide what gets built?** Each of the three replays decides one piece:
- the six timing bugs decide the second opinion in a Run;
- a sample of past plans decides the plan check;
- the 30 past Spec reviews decide the two Spec-review options.

The options:
- (a) **Each piece waits for its replay.** If a replay comes out empty, that piece isn't built.
- (b) **Build the Spec review's own Harness, Model and Effort now; the rest waits.** It's the smallest piece, and the others reuse its mechanism.I
- (c) **Build everything now as opt-in config,** and let the replays decide what you turn on.

*Recommended:* (a). It's what you agreed in Q1: build where your record says it helps, and nothing where it doesn't. It also keeps your Day-shift time off pieces that may never be built.
- **Cost.** The replays need no change to thirdshift: a script outside it runs review sessions over past commits. Claude can write it when the grilling is done, so the replays run next. They're mostly machine time, plus an hour or two of your reading.
- **What passes** gets built as opt-in User config, as you asked in Q3.
- **Against (b):** it builds the option with the highest bar before seeing its evidence.

*Answer:* agree

**Q5 - Which direction do the replays test?** Every past Run but one was written by Claude, so the replays can only test Codex reviewing Claude's work. Your config now has Codex write, so the second opinion you'd actually run is Claude reviewing Codex.

In the July study, the gain followed whichever model was stronger, so one direction doesn't predict the other. Also, the one Codex Session log so far doesn't record the review's findings. A reverse replay would have to re-run the same-model review instead of reading its findings from the logs.

The options:
- (a) **Replay Codex reviewing Claude now,** and repeat in reverse once there's enough Codex work (say 20 Runs and a few Specs).
- (b) **Wait for Codex work,** and replay only the direction you'd run.
- (c) **Switch the writing back to Claude,** so the replays test the direction you'd run.

*Recommended:* (a). One direction can be tested today, and the reverse only costs a wait. Treat the first result as half the answer: a pass means the pairing works at least one way round, but an empty result doesn't rule out the reverse.

*Answer:* agree

**Q6 - What result gets each piece built?** Setting the bar before the replays run stops the results from being read to fit. With 6 bugs, 20 plans and 30 Specs, only a large effect will show. Every arm runs at the Effort you'd give a second opinion, so model family and Effort don't get mixed up. The proposal:
- **The six timing bugs, three arms:** the same model in a fresh session, the other family, and the same model with a timing checklist.
  - **Build the second opinion in a Run** if the other family catches at least 2 bugs that the fresh same-model arm misses, and the checklist arm doesn't also catch them.
  - **Add the checklist to the review skill instead** if it catches as many.
  - **Drop it** if no arm catches more than 1: review isn't the lever for what slips through.
- **Plan check over 20 past plans** across the repos. Build it if it stops at least 3, and you'd have stopped most of the ones it stops.
- **All 30 Spec reviews.** The other family reviews each Spec branch as it stood before its Spec review.
  - **Offer a second opinion after the Spec review** if the bugs the recorded review missed include a real behaviour bug in at least 6 of the 30 Specs, and no more than half of a graded sample turns out wrong.
  - **Offer the Spec review on its own Harness** only if the other family, alone, finds real behaviour bugs in at least as many Specs as the recorded review did (about 15). Swapping gives up a review that works.

*Recommended:* use those bars. Your time goes on reading the plans the check stops and grading about 20 Spec findings. Claude can grade the six bugs against their known fixes.

*Answer:* agree

**Q7 - What's it called, and when does it go into the glossary?** The proposal:

> **Second opinion**:
> An agent session that reviews another session's work, on its own **Harness**, **Model** and **Effort** set in the **User config**: a **Run**'s change after its implement session, a **Spec branch** after its **Spec review**, or an **Architect plan** before thirdshift marks it ready. Usually another model family, never necessarily.
> _Avoid_: multi-family review, cross-family review, plan check

Three existing entries change too. **Harness**, **Model** and **Effort** each say "one per Command". They would become: one per Command, except a Second opinion's, and the Spec review's when the User config gives it its own.

The options:
- (a) **One term,** "Second opinion", for all three points.
- (b) **Separate terms,** such as "Plan check" for the plan.
- (c) **"Cross-family review"** or **"Multi-family review"**.

*Recommended:* (a). The family is a setting, not the point: your config could give it the same family at a higher Effort. One term keeps the config and the vocabulary small. The terms should go into `CONTEXT.md` with the Ticket that builds the feature, not now. Every factory session reads `CONTEXT.md`, so an Architecture review would take the term for something that already exists.

*Answer:* agree, though why avoid cross-family review? to make the technique more open to same-family second opinion review? I like the term second opinion for that flexibility.

*Checked:* yes, that's the reason. _Avoid_ lists names not to use for the concept. "Cross-family review" names one way of setting it up, so as the name it would read as a rule that the reviewer must come from another family. Calling a particular Second opinion cross-family, when it is, is fine.

**Q8 - Evidence-backed findings: when?** The research rates this as the best-supported change. It needs no second Harness, only a change to the skill and the prompts:
- a correctness finding comes with a failing test or a reproduction;
- a Spec finding quotes the requirement;
- a decline cites the line, plan decision or test that refutes the finding;
- each reviewer lists the files it read.

Your record adds two reasons:
- Nobody reads the ~7 declined findings per PR. Citations would let each decline be checked without you.
- The two review sub-agents spend about a minute each, and nothing asks them to show their work.

The options:
- (a) **Now, as its own Ticket.** The replays use a fixed copy of today's skill, so the comparison stays clean.
- (b) **After the replays.**
- (c) **Only inside the second opinion.**
- (d) **Not at all.**

*Recommended:* (a). It's worth doing whether or not a second opinion ever ships, and the factory can build it as one Ticket. One limit from the research: a test that passes is weak evidence that a finding is wrong. A decline should cite a test that exercises that particular finding, not just a green test suite.

*Answer:* agree

## Round 3 (open)

**Q9 - The Effort for each replay arm.** Q6 settled that every arm runs at the Effort you'd give a second opinion, so the families meet at full strength. Effort moves a model about as much as switching models does: Opus 5.5 scores 51.2 on AA's index at medium and 57.6 at max. In their own CLIs, GPT-6.1 Sol does best at xhigh (62.9, ahead of its own max at 60.1), and Opus 5.5's only measured setting is max (66.0). OpenAI names code review as a use for xhigh. The options:
- (a) **Each at its best measured setting:** Codex at xhigh, Claude at max.
- (b) **Your config's settings:** Codex at xhigh, Claude at medium.
- (c) **The same label for both:** xhigh.

*Recommended:* (a). Each family gets its strongest reviewer, so a poor result can't be put down to a weak setting. Under (b), the Claude arms would be the weaker reviewer by design. Max is Claude's most expensive setting, but it runs in only 12 sessions, or 32 if Q10 adds the same-model plan arm.

*Answer:* 

**Q10 - A same-model arm for the plan check?** As agreed in Q6, the plan replay has one arm: the other family. But Q7 made the family a setting, and the research found a fresh session of the same model about as good a checker as another family at the top tier. A second arm, Claude checking Claude's plans in a fresh session, shows whether the plan check needs another family at all, which decides its default. You'd grade the stopped plans without knowing which arm stopped them. The options:
- (a) **Add the same-model arm:** 20 more Claude sessions of about 7 minutes, graded blind.
- (b) **The other family only,** as agreed.

*Recommended:* (a). It's cheap next to what it decides: a plan check that works on the same family needs no second Harness. The Spec replay needs no such arm, since the recorded Spec review is its same-model baseline.

*Answer:* 

**Q11 - May replay reviewers build and run tests?** The recorded reviews they're compared with only read: the in-session sub-agents spend about a minute each, too short to have run the test suite (an inference). Letting the replay reviewers run tests would mix "another family" with "evidence from running code", which the research says matters more than family. It would also load this 4-core, 7 GB machine while Pickup runs build. The options:
- (a) **Read only:** they read the code and run read-only commands (git, grep), with no builds or tests, one session at a time.
- (b) **Free to build and run tests.**

*Recommended:* (a). It measures the family, which is the question. Running code enters through the evidence-backed Ticket (Q8), and light replays can run beside the factory without starving it.

*Answer:* 

**Q12 - Who writes the timing checklist?** The third arm of the six-bug replay is the same model with a timing checklist. Whoever writes the checklist after seeing the six bugs will, knowingly or not, write it to catch them, and the arm will look better than it is. Claude has seen them. The options:
- (a) **A fresh agent that hasn't seen the bugs** writes it from `CONTEXT.md` and the ADRs, without the issues or the git history: the timing risks a reviewer of thirdshift should check.
- (b) **Claude writes it** from a generic list of timing failures: shared state across processes, state that changes while you wait, repeated passes, cleanup on every exit path.
- (c) **Drop the checklist arm.**

*Recommended:* (a). It keeps the arm honest, and what it writes is the checklist you'd add to the skill if the arm wins.

*Answer:* 

**Q13 - Who runs the evidence for a correctness finding?** Q8 settled that a correctness finding comes with a failing test or a reproduction. The research adds that the evidence has to be run, not just described: unchecked evidence raised false rejections as fast as real catches. Today's review sub-agents only read, in parallel, in the author's worktree. Standards findings keep citing the standard they break or naming the smell, since a smell can't fail a test. The options:
- (a) **The reviewer writes it, the author runs it.** Each correctness finding carries a test or a command, written out by the reviewer. The author runs it as written before deciding. If it fails as claimed, the author fixes the bug and keeps the test. If it passes, the author may decline, citing the run.
- (b) **The reviewer runs it.** Each sub-agent writes and runs its own failing test, and reports the output.
- (c) **A description is enough;** nothing is run.

*Recommended:* (a). The run decides, not the author's view of its own code, and no two agents write and build in the worktree at once. A failing test that's kept also guards the fix. (b) is more independent, but has the two sub-agents writing and building side by side.

*Answer:* 

**Q14 - When a reviewer skipped changed files.** Q8 settled that each reviewer lists the files it read. In the research, reviewers left changed files unread in 67.9% of review runs, and mostly didn't say so. The options:
- (a) **Send each unread changed file back to that reviewer once,** then name any file still unread in the review's summary and in the PR body.
- (b) **Only name them,** in the summary and the PR body.
- (c) **The author reviews them itself.**

*Recommended:* (a). One more pass costs a minute or two and should close most gaps; a second retry would be churn. Naming what's still unread keeps a gap visible after the Self-merge.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- **Standalone Runs:** "every Ticket's Run" means any Run, standalone or a Ticket's. Whether they get separate switches is a config question for after the replays. (Q3)
- **Nothing is built before its replay.** The second opinion in a Run, the plan check and both Spec-review options each wait for their replay, and a piece that misses its bar isn't built. (Q4, Q6)
- **What passes is opt-in User config,** off by default. (Q3, Q4)
- **The replays change nothing in thirdshift.** A script outside it, which Claude writes once this grilling ends, runs review sessions over past commits. (Q4)
- **The first replays test Codex (GPT-6.1 Sol) reviewing Claude-written work.** The reverse waits for about 20 Codex Runs and a few Specs, and a pass in one direction is half the answer. (Q5)
- **The bars in Q6 are fixed** before any replay runs. Claude grades the six bugs against their fixes; you read the plans the check stops and grade about 20 Spec findings. (Q6)
- **The term is Second opinion,** and the family is a setting. The entry reaches `CONTEXT.md` with the Ticket that builds it, along with the changes to Harness, Model and Effort. (Q7)
- **Evidence-backed findings get their own Ticket now.** The replays use the `thirdshift-code-review` skill as it was before that Ticket lands. (Q8)
- **The evidence-backed change covers every session that runs the review skill:** the implement session, fresh or continuing, the Spec review and the review Repair. (Q8)
- **Unaddressed findings carry each decline's citation** in place of a one-line reason, and the Unaddressed finding entry in `CONTEXT.md` changes with that Ticket. (Q8)

## Research results (2026-10-06)

- No human has reviewed a factory PR since about 27 September: every PR merged itself. Confirmed by the Command logs from 3 October, and by merge timing before that (an inference).
- The "Review clean-ups" commits are the factory's own implement sessions fixing what their same-model review found, confirmed by finding each commit in its session's Session log.
- Almost everything was written and reviewed by Claude Opus 5.5. The one exception is a docs PR written by Codex (#426). Confirmed by the Session logs and the Activity log.
- 16 bugs reached main on thirdshift. About 10 showed up only in live runs (environment, tool versions, harness behaviour). About 6 are timing bugs that no review raised: what happens when other Runs act at the same moment, or a pass repeats (#24, #57, #102, #175, #182, #303). Confirmed by the issues, their fixes and the recorded reviews (an agent's sorting, with the six titles checked).
- Authors decline about 7 findings per PR, and nobody reads them, confirmed by the PRs' Unaddressed findings sections.
- Architect runs: 209 Architecture reviews produced 202 plans and 4 ideas. The review rated its own pick Strong 98% of the time, and nobody else checked a plan. Confirmed by the Activity logs, the Session logs and the `architect-plan` and `architect-idea` issues.
- In Spec PRs, about 28% of declined findings were declined because the plan or an ADR had settled the point, confirmed by a pattern match over the PR bodies (an estimate).
- About half of the 30 Spec reviews fixed a behaviour bug the Tickets' reviews had passed, by the sessions' own summaries (keeplore's c9317c4 checked).
- The two review sub-agents in an implement session take about a minute each, confirmed by Session log timestamps.
- The recorded reviews can be pulled from the Claude Session logs (366 of 390 implement sessions, all 30 Spec reviews). The one Codex Session log holds only the author's summary. Confirmed by parsing the logs.
- Claude's seven-day usage window reached 70% on 5 October, confirmed by the rate-limit events in the Session logs.
- Effort moves Opus 5.5 from 51.2 (medium) to 57.6 (max) on AA's Intelligence Index. In their own CLIs, GPT-6.1 Sol scores best at xhigh (62.9; 60.1 at max), and Opus 5.5 has only a max row (66.0). Confirmed by `docs/research/model-strength-for-review.md` §4.1, from AA's published results and vendor docs.

## Waiting on research

- **The replays' inputs.** An agent is listing the PR that introduced each of the six timing bugs, with the fix that shows what a catch must say. It's also listing the 30 Spec branches as they stood before their Spec reviews, with the recorded reviews, and 20 plans drawn across the repos, each with the commit it was planned on. For the replays (Q6).
- **The replays themselves,** once the script runs. Two questions wait on their numbers and aren't asked yet: who acts on a second opinion's findings, and what happens when the plan check disagrees.
