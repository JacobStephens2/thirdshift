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

## Round 3 (answered)

**Q9 - The Effort for each replay arm.** Q6 settled that every arm runs at the Effort you'd give a second opinion, so the families meet at full strength. Effort moves a model about as much as switching models does: Opus 5.5 scores 51.2 on AA's index at medium and 57.6 at max. In their own CLIs, GPT-6.1 Sol does best at xhigh (62.9, ahead of its own max at 60.1), and Opus 5.5's only measured setting is max (66.0). OpenAI names code review as a use for xhigh. The options:
- (a) **Each at its best measured setting:** Codex at xhigh, Claude at max.
- (b) **Your config's settings:** Codex at xhigh, Claude at medium.
- (c) **The same label for both:** xhigh.

*Recommended:* (a). Each family gets its strongest reviewer, so a poor result can't be put down to a weak setting. Under (b), the Claude arms would be the weaker reviewer by design. Max is Claude's most expensive setting, but it runs in only 10 sessions, or 30 if Q10 adds the same-model plan arm.

*Answer:* agreed for defaults and suggestions, though I want to be able to set this in User config

**Q10 - A same-model arm for the plan check?** As agreed in Q6, the plan replay has one arm: the other family. But Q7 made the family a setting, and the research found a fresh session of the same model about as good a checker as another family at the top tier. A second arm, Claude checking Claude's plans in a fresh session, shows whether the plan check needs another family at all, which decides its default. You'd grade the stopped plans without knowing which arm stopped them. The options:
- (a) **Add the same-model arm:** 20 more Claude sessions of about 7 minutes, graded blind.
- (b) **The other family only,** as agreed.

*Recommended:* (a). It's cheap next to what it decides: a plan check that works on the same family needs no second Harness. The Spec replay needs no such arm, since the recorded Spec review is its same-model baseline.

*Answer:* agree

**Q11 - May replay reviewers build and run tests?** The recorded reviews they're compared with only read: the in-session sub-agents spend about a minute each, too short to have run the test suite (an inference). Letting the replay reviewers run tests would mix "another family" with "evidence from running code", which the research says matters more than family. It would also load this 4-core, 7 GB machine while Pickup runs build. The options:
- (a) **Read only:** they read the code and run read-only commands (git, grep), with no builds or tests, one session at a time.
- (b) **Free to build and run tests.**

*Recommended:* (a). It measures the family, which is the question. Running code enters through the evidence-backed Ticket (Q8), and light replays can run beside the factory without starving it.

*Answer:* agree

**Q12 - Who writes the timing checklist?** The third arm of the six-bug replay is the same model with a timing checklist. Whoever writes the checklist after seeing the six bugs will, knowingly or not, write it to catch them, and the arm will look better than it is. Claude has seen them. The options:
- (a) **A fresh agent that hasn't seen the bugs** writes it from `CONTEXT.md` and the ADRs, without the issues or the git history: the timing risks a reviewer of thirdshift should check.
- (b) **Claude writes it** from a generic list of timing failures: shared state across processes, state that changes while you wait, repeated passes, cleanup on every exit path.
- (c) **Drop the checklist arm.**

*Recommended:* (a). It keeps the arm honest, and what it writes is the checklist you'd add to the skill if the arm wins.

*Answer:* agree

**Q13 - Who runs the evidence for a correctness finding?** Q8 settled that a correctness finding comes with a failing test or a reproduction. The research adds that the evidence has to be run, not just described: unchecked evidence raised false rejections as fast as real catches. Today's review sub-agents only read, in parallel, in the author's worktree. Standards findings keep citing the standard they break or naming the smell, since a smell can't fail a test. The options:
- (a) **The reviewer writes it, the author runs it.** Each correctness finding carries a test or a command, written out by the reviewer. The author runs it as written before deciding. If it fails as claimed, the author fixes the bug and keeps the test. If it passes, the author may decline, citing the run.
- (b) **The reviewer runs it.** Each sub-agent writes and runs its own failing test, and reports the output.
- (c) **A description is enough;** nothing is run.

*Recommended:* (a). The run decides, not the author's view of its own code, and no two agents write and build in the worktree at once. A failing test that's kept also guards the fix. (b) is more independent, but has the two sub-agents writing and building side by side.

*Answer:* agree

**Q14 - When a reviewer skipped changed files.** Q8 settled that each reviewer lists the files it read. In the research, reviewers left changed files unread in 67.9% of review runs, and mostly didn't say so. The options:
- (a) **Send each unread changed file back to that reviewer once,** then name any file still unread in the review's summary and in the PR body.
- (b) **Only name them,** in the summary and the PR body.
- (c) **The author reviews them itself.**

*Recommended:* (a). One more pass costs a minute or two and should close most gaps; a second retry would be churn. Naming what's still unread keeps a gap visible after the Self-merge.

*Answer:* agree

**Q15 - Findings declined as needing someone else's decision.** Two of the six timing bugs, #102 and #303, were caught by a same-model review. The author then declined each one as needing a decision from the Spec or the Day shift. Because the PRs merged themselves, nobody made that decision, and both bugs shipped. This builds on Q8, but a citation can't settle these declines: the finding is that the Spec is wrong, not that the finding is. So far, declines that leave the call to a human are rare: about 12 across the 199 merged PRs. The options:
- (a) **File each as an issue labelled `needs-triage`,** linked from the PR's Unaddressed findings, unless an open issue already covers it. It pauses nothing: only an Architect idea pauses Weeding.
- (b) **Hold the Self-merge:** a Merge run with such a finding leaves its PR ready for review instead of merging it.
- (c) **The author decides:** it may depart from the Spec to fix a real bug, and says so in the PR body.
- (d) **A Second opinion rules on them,** once one exists.
- (e) **Leave them in the PR body,** as now.

*Recommended:* (a), as part of the evidence-backed findings Ticket. It sends each call to the person the agent said should make it, at about one issue per 17 merged PRs so far. (b) would stop Merge runs over what is often a small call. (c) lets the factory overrule a Spec with nobody seeing it. A Second opinion couldn't make a Day-shift decision either.

*Answer:* agree

**Q16 - Run the reverse direction now, where the work exists?** Q5 set the reverse replays for once Codex had written about 20 Runs and a few Specs. Since 5 October, Weeding on thirdshift has run on Codex: about 21 merged PRs from 11 plans, plus two Spec runs (#432, #465). No bug has surfaced in that work yet, so there is nothing for a timing-bug replay. The options:
- (a) **Add a reverse plan replay now:** Claude checks the 11 Codex plans, with Q10's same-model arm if you add it, and Q6's bar scaled to at least 2 of the 11. The reverse Spec and bug replays wait.
- (b) **Wait,** as Q5 has it.
- (c) **Run every reverse replay the work allows,** the two Specs included.

*Recommended:* (a). The plans are there now, and the reverse matters most for plans: Codex now plans everything Weeding builds on thirdshift. Two Specs are too few to show anything, and there are no bugs to replay yet.

*Answer:* agree. Thouhg what is a reverse plan replay?

*Checked:* it's the same plan check with the roles swapped. The first plan replay has Codex check 20 plans that Claude's Architecture reviews wrote. The reverse has Claude check the 11 plans that Codex's Architecture reviews have written since 5 October. With Q10, each direction also gets a same-family arm: Claude rechecks Claude's plans, and Codex rechecks Codex's.

**Q17 - Do the new Harnesses join the replays?** Main now runs sessions on six Harnesses: Claude Code, Codex, Antigravity (`agy`), Grok Build, Muse Code and OpenCode. Their models all score below both current reviewers on AA's index: Muse Spark 1.3 48.1, Grok 4.7 46.4, MiMo-V2.6-Pro 46.3 and Gemini 3.8 Flash 40.9, against 51.0 for GPT-6.1 Sol at xhigh and 57.6 for Opus 5.5 at max. The research warns that a weaker reviewer can make strong code worse when its findings are acted on. But the six-bug replay only counts catches, and another family might still see a timing bug the two strongest miss. Q7 makes the family a setting, so any of them could be configured as a Second opinion. The options:
- (a) **One arm per new Harness on the six-bug replay,** each at its best setting: about 20 short sessions on their own subscriptions, graded by Claude. An arm that passes Q6's bar then gets the Spec replay before it's suggested.
- (b) **Only the strongest new one,** Muse Spark 1.3 at max.
- (c) **Not in this batch:** two families first.
- (d) **Every replay for every new Harness.**

*Recommended:* (a). It's cheap, it uses none of your Claude or Codex limits, and it answers the question for the new Harnesses with your own bugs. With five other-family arms against one bar, one may pass by luck, which is why a pass has to hold up on the Spec replay too.

*Answer:* agree

**Q18 - Keep each reviewer's report.** Claude sessions keep both reviewers' reports inside the Session log. Codex sessions write them to files in `/tmp`, and the Session log holds none of them; only the newest pair survives (#475). The other four Harnesses aren't checked yet. Without the reports, no later check has anything to compare against: not a reverse replay, not a trial, not Q14's lists of files read. This builds on Q8's evidence-backed findings Ticket. The options:
- (a) **thirdshift keeps each reviewer's report beside the Session log,** on every Harness, as part of the evidence-backed findings Ticket.
- (b) **The reports go in the PR body.**
- (c) **Leave it to each Harness,** as now.

*Recommended:* (a). It's what makes every later check possible, starting with the reverse replays. (b) would bury a PR body nobody reads under two long reports.

*Answer:* agree

## Round 4 (answered)

**Q19 - Where do the replay script and its results live?** The replays happen once, but later decisions will cite their results, as they cite `docs/research/` today. About 130 sessions will leave raw reports, and your grading needs a place too. The options:
- (a) **A research note in `docs/research/`,** with the script beside it, in a PR you merge, and a summary on #429. The raw reports stay outside the repo, in `~/.thirdshift/replays/`. You grade in a file in this folder, with an `*Answer:* ` line for each plan or finding.
- (b) **Only a summary on #429,** with the script and the reports left in scratch.
- (c) **Everything in the repo,** raw reports included.

*Recommended:* (a). It's how #430's research landed, and whoever reruns a replay later needs the script, starting with the reverse Spec and bug replays. The raw reports are too bulky for the repo, and the note carries what they show.

*Answer:* agree

**Q20 - When do the replays run?** About 130 sessions, one at a time, take about 15 hours. 66 of them are Codex sessions at xhigh, and Codex now does all of Weeding's work, so the two would share Codex's limits. If a Weeding session is refused for a limit, its plan fails, and Weeding then waits for you. Another 42 are Claude sessions at max, and 20 run on the new Harnesses. Weeding starts its next Architect run within a minute of the last one ending, so there are almost no quiet gaps to run in. The options:
- (a) **Pause Weeding while the replays run,** about a day, by commenting out its crontab line, then turn it back on. Pickup runs carry on.
- (b) **Run alongside Weeding,** one session at a time, stopping at the first limit warning or refusal from either CLI.
- (c) **Run the Claude and new-Harness sessions alongside Weeding,** and pause Weeding only for the 66 Codex sessions.

*Recommended:* (a). A day without Weeding costs a day's worth of plans that nobody yet knows are good, and it rules out a failed plan from a refused session. The six-bug replay goes first: it's the cheapest, and it needs no grading from you. You, or Claude with your go-ahead, edit the crontab.

*Answer:* agreed. I'm running low on claude usage this week, so there's a chance we'd want to wait on the claude test until after 10p EDT Oct 9 when my usage resets. I'm at 93% used this week on my claude subscription. But the codex session and any other harnesses sessions could run in the meanwhile.

**Q21 - How is the evidence-backed work filed?** It has grown since Q8:
- evidence for correctness findings (Q13);
- citations on declines;
- lists of files read, with one retry (Q14);
- issues for declines someone else must decide (Q15);
- thirdshift keeping each reviewer's report on every Harness (Q18).

The first four change the skill and the Session prompts; the last changes thirdshift's code. Whatever `to-spec` and `to-tickets` file is labelled `ready-for-agent`, so the next Pickup run builds it and it merges itself. It reaches the factory with the next release. The options:
- (a) **A Spec with two Tickets:** the review's evidence rules first, then thirdshift keeping the reports, blocked by the first.
- (b) **One Ticket,** as Q8 said.
- (c) **Either shape, labelled `needs-triage`,** so you read it before the factory takes it.

*Recommended:* (a). The two halves change different things, each fits one session, and a Spec run ends with a Spec review over both. Your answers here are its design, so `ready-for-agent` is safe.

*Answer:* agree, we'll run the to-spec and to-tickets skills and maybe even find that more tickets are useful to keep more control on the context window size of the agents running implement skill on the tickets - though i may even have thirdshift just do a pickup run on the spec.

**Q22 - What happens to #429?** Its body still lists the first session's open questions, which are all settled now. The options:
- (a) **Rewrite its body** with the decisions and the replay plan, keep it open until the replays report, and link the evidence-backed work to it. If the Second opinion gets built, it gets a Spec of its own.
- (b) **Close it** once the evidence-backed work is filed, and open a new issue for the replays.
- (c) **Leave it as it is** until the replays report.

*Recommended:* (a). It's where the question started, and keeping #429 open keeps the triggers for the reverse replays in one place: more Codex Specs, and bugs surfacing in Codex's work.

*Answer:* agree

## Round 5 (answered)

**Q23 - Is this the plan?** No questions are left, apart from the two that wait on the replays' numbers. Nothing below happens until you agree. This session also runs on your Claude subscription, so until it resets, Claude's own part stays small: the script, the filing and #429.
1. **File the evidence-backed findings Spec** with `to-spec` and `to-tickets`, labelled `ready-for-agent`. A Pickup run builds it on Codex, and Architect runs give way to it while it runs. (Q21)
2. **Rewrite #429's body** with the decisions and the replay plan, and link the Spec. (Q22)
3. **Write the replay script.** A fresh Codex agent that hasn't seen the bugs writes the timing checklist. (Q12, Q19)
4. **Once the Spec run ends, pause Weeding** by commenting out its crontab line. Then run the 86 Codex and new-Harness review sessions, about 11 hours, stopping at any limit warning or refusal and resuming later. Pickup runs carry on, and Weeding comes back on when the sessions finish. (Q20)
5. **After 10 pm EDT on 9 October,** when your Claude usage resets, run the 41 Claude sessions alongside Weeding, since they share no limits with it. (Q20)
6. **Grade.** Claude grades the six bugs against their fixes. You grade the plans the checks stop, without knowing which arm stopped them, and about 20 Spec findings, in a file in this folder. (Q6, Q10, Q19)
7. **Write it up:** a research note in `docs/research/` with the script beside it, in a PR you merge, and a summary on #429. A piece gets built only if it passes its bar. (Q6, Q19)

The options:
- (a) **Yes,** and Claude may edit the crontab in step 4.
- (b) **Yes, but you'll edit the crontab yourself.**
- (c) **Change something,** and say what.

*Recommended:* (a).

*Answer:* agree.

## Round 6 (answered)

**Q24 - Do the evidence-backed Spec's test seams match what you expect?** `to-spec` checks the seams before it writes the Spec. Both are seams thirdshift already has, one for each half:
- **The Session prompts, through the prompts page.** The test that regenerates `prompts/` (`UPDATE_PROMPTS=1 cargo test prompts_page`) shows the new wording of the implement, Spec review and review Repair prompts: run each correctness finding's evidence, cite each decline, file a `needs-triage` issue for a call that isn't the agent's, and name unread files.
- **The Sessions module, through the seam its execution tests already use,** with the session faked. A session that leaves its reviewers' reports where its prompt said gets them kept beside its Session log, the same on every Harness. A session that leaves none gets a progress line, not a failure.

The skill's own text has no test seam: the Spec review reads it, and the replays later measure it. The options:
- (a) **These two.**
- (b) **Add a seam,** and say where.

*Recommended:* (a). Both seams exist, and each is the highest one that sees its half of the change, so no new seam is needed.

*Answer:* agree

## Round 7 (answered)

**Q25 - The Tickets for #486.** `to-tickets` asks you to approve the breakdown before it publishes anything. Today four prompts repeat the review's wording: the implement session (fresh and Continuation), the Spec review, and the Repair that reviews Foreign commits. So a prefactor comes first, and each later Ticket changes that wording in one place. Every Ticket edits the same skill text and the same shared wording, so they run one after another rather than side by side, which avoids conflict Repairs between them.
1. **One review instruction for the prompts that run the review.**
   - Blocked by: nothing.
   - Delivers: the four prompts take their review wording from one place, and the prompts page is unchanged.
2. **Correctness findings carry a test the author runs, and every decline cites its evidence.**
   - Blocked by: 1.
   - Delivers:
     - The reviewers write out a test or command for each finding that says behaviour is wrong, and the author runs it as written. A failure means a fix and a kept test; a pass allows a decline that cites the run.
     - Every Unaddressed finding cites what refutes it.
     - A call that isn't the agent's becomes a `needs-triage` issue, linked from its entry, unless an open issue covers it.
     - The Unaddressed finding entry in `CONTEXT.md`.
3. **Reviewers list the files they read.**
   - Blocked by: 2.
   - Delivers: each reviewer's list of the files it read. A changed file it skipped goes back to it once, and any file still unread is named in the review's summary and the PR body.
4. **thirdshift keeps each reviewer's report beside the Session log.**
   - Blocked by: 3.
   - Delivers:
     - In each prompt that runs the review, thirdshift names a place that git ignores, and the skill writes each axis's report there.
     - When the session ends, thirdshift keeps the reports beside its Session log, the same on every Harness. No reports means a progress line, not a failure.
     - The Session log entry in `CONTEXT.md`.

Each Ticket is labelled `ready-for-agent`. #486 swaps from `needs-triage` to `ready-for-agent` once all four are published. The options:
- (a) **These four, in this order.**
- (b) **Coarser:** merge 2 and 3, for three Tickets.
- (c) **Finer:** split 2 into the evidence for correctness findings and the citations on declines, for five Tickets.
- (d) **Change the blocking edges or the order,** and say how.

*Recommended:* (a). Each Ticket is a small change a session can hold whole, and each can be checked on its own: 1 by an unchanged prompts page, 2 and 3 by the prompts page's diff, and 4 through the Sessions seam. Five Tickets would spend a full Run on a few lines of wording, and three would put two different changes to the reviewers into one session.

*Answer:* agree

## Round 8 (answered)

Picked up again at 10:30 pm EDT on 9 October, just after your Claude week reset. Since Round 7, the evidence-backed Spec (#486) has merged and reached the factory, and the security work (#427) has shipped in 0.11.0. None of the replay plan has run yet: there's no script, no checklist and no reports. Weeding had been paused since 8 October by Architect idea #564; you closed it, and Weeding started again at 10:37 pm. Q26 and Q27 were asked first. Q28–Q30 come from the Codex logs check, which has now finished; see "Research results (2026-10-09)".

**Q26 - Which review skill do the replays use, now that #486 is live?** Q21's Spec settled that the replays use the review skill as it was before #486. It landed on 7 October (0.10.1), so the factory's review now asks for evidence behind correctness findings, citations on declines and lists of files read. A Second opinion would be added to that review, not to the old one. Here's what each replay compares against:
- **Six timing bugs:** every arm is a fresh session, with no recorded review to match. Either skill works, as long as every arm uses the same one. Under today's skill, the reviewers write a test for each correctness finding. Q11 means nobody runs it, and grading against the known fixes doesn't change.
- **Spec replay:** the baseline is the recorded Spec review, which used the old skill. To match it, the Codex arm needs the old skill too. Otherwise a fresh Claude arm on today's skill becomes the baseline, which adds 30 Claude sessions at max.
- **Plan check:** it doesn't use the review skill, so this doesn't touch it.

The options:
- (a) **The six-bug replay uses today's skill; the Spec replay keeps the old one.**
- (b) **The old skill everywhere,** as settled.
- (c) **Today's skill everywhere,** with a fresh Claude arm on today's skill as the Spec baseline.

*Recommended:* (a). The six-bug replay decides the Second opinion in a Run, and it costs nothing to ask that question of the review the factory actually runs. That also removes most of the "a pass overstates the gain" caveat from that replay. The Spec replay has to match its recorded baseline, and (c) spends 30 Claude max sessions just to rebuild one.

*Answer:* agree

**Q27 - The order of the replays, now that Claude's week has reset.** Q20 ran the Codex sessions first, with Weeding paused, because your Claude week was almost used up. Now it has reset, and nothing has run yet. Two things have changed since Q20:
- Security runs on thirdshift (every minute, `secure` in the crontab) run on Muse Code (`security.harness = "muse"`). So the Muse arm's five sessions share Muse's limits with them. If a Security session gets refused for a limit, its fix fails and Securing waits for you, just as a refused session fails a Weeding plan. For now Security runs are being skipped, because a Security finding waits for you. They start again once you triage it.
- Every Run that isn't a Ticket's now also starts a Security review (`security.review = true`), on the Run's own Harness, which is Codex. So Pickup and Architect runs use more of Codex's limits than they did when Q20 was settled.

The sessions: 41 on Claude at max, 66 on Codex at xhigh, and 20 on the four new Harnesses. The options:
- (a) **Claude first, starting tonight.** Claude writes the script, and a fresh agent that hasn't seen the bugs writes the timing checklist. Then the 41 Claude sessions run alongside Weeding, since they share no limits with it. Weeding pauses for the 66 Codex sessions. The new-Harness sessions run alongside those, with Security runs paused only during the Muse arm.
- (b) **Codex first,** as Q20 has it, with Security runs paused during the Muse arm.
- (c) **(a), but Security runs keep running** through the Muse arm.

*Recommended:* (a). It uses the fresh Claude week while Weeding keeps going, and it shortens Weeding's pause to the Codex sessions only. Pausing Security for five sessions costs less than a failed fix that waits for you. One risk: 41 sessions at max are a real share of the new week, and Q29 and Q30 could add about 13 more. The script stops at the first limit warning and picks up later.

*Answer:* agree, though I'm thinking 41 claude sessions at high instead of max effort.

*Checked:* that reopens part of Q9, which put each family at its best setting, so it's Q31 below. Q29 and Q30 also raise the Claude sessions to 54.

**Q28 - Should Codex's review sub-agents be told to start fresh?** #427's trial worried that Codex review sub-agents carry the implementer's conversation, which would make the factory's same-model review on Codex less independent. In the 54 Codex implement and Spec-review sessions since 7 October, they didn't. Each sub-agent got the opening prompt and the implement skill at most, and never the implementer's tool calls, output or reasoning. So far, the worry doesn't hold. But that's luck, not design:
- **The model chooses the setting each time.** It spawned 23 with `fork_turns: "none"`, 6 with `"all"`, and left it out for 25. Codex's own tool text says the last two should carry the parent's history.
- **Why they didn't, as far as we can tell:** Codex seems to copy only completed turns, and a headless session's one turn is still running when it spawns them (an inference).
- **A Resume is the likely hole.** It continues a session with `codex exec resume`, so its first turn is complete. A review run in a Resume could then hand the implementer's whole first turn to its reviewers. This is also an inference; no such case has been seen.
- **Security sessions on Codex already ask for fresh sub-agents.** Codex's Harness adds a line asking for `fork_turns: "none"`, and a test checks that the implement prompts don't get that line. Adding it would leave the prompts page as it is.

The options:
- (a) **File a small Ticket, `ready-for-agent`:** every Codex session that runs the review gets the same fresh-sub-agent line Security sessions get.
- (b) **Just record it on #429:** the worry didn't hold up.
- (c) **First check whether any Resumed Codex review carried the implementer's turns,** then decide.

*Recommended:* (a). It's one line in the Codex Harness, and it makes the reviewers' independence a setting rather than the model's choice or a detail of how Codex works. (c) would spend a search on a case that's rare, when the fix costs less than the search.

*Answer:* agree

**Q29 - How large is the reverse plan replay?** Q16 sized it to the 11 plans Codex had written by 7 October, with a bar of 2. There are now 27 (#448–#530, Spec #465 included), from Codex Architect runs between 6 and 8 October. The forward replay checks 20 plans, with a bar of 3. The options:
- (a) **20 of the 27, drawn the way the forward sample was, with the same bar of 3.** Both directions then answer the same question at the same size. This adds 9 sessions to each arm: Claude, as the other family, and Codex in a fresh session.
- (b) **All 27,** with the bar scaled to 4.
- (c) **The first 11,** at a bar of 2, as Q16 has it.

*Recommended:* (a). Codex now writes every plan Weeding builds on, so this is the direction you'd actually run, and it deserves the same footing as the forward replay. Drawing 20 the same way keeps the two results comparable. Your grading grows only by the plans the checks stop.

*Answer:* agree

**Q30 - A reverse bug replay over the bugs in Codex's work?** Q16 put the reverse bug replay off until bugs surfaced in Codex's work. Three have now:
- **#559:** a test that gave a whole Run two seconds to finish failed on macOS CI. It came from PR #449, for Codex plan #448.
- **#603:** a Security run reported a finished audit as "audit failed". It came from PRs #562 and #577.
- **#607:** a Security review that ended incomplete on purpose was reported as a generic failure. It came from PR #592.

Only #559 is about timing, and only loosely: it's a time limit in a test, not other Runs acting at the same moment. The reviewers' reports were kept for #562, #577 and #592 (#490), so the replay can also show whether the factory's own review raised each bug. The options:
- (a) **Replay all three now,** four PRs as they merged. One arm is Claude at max, as the other family, and the other is Codex at xhigh in a fresh session. Both use whichever skill Q26 picks for the six-bug replay. The bar is Q6's: build counts it as a pass if Claude catches at least 2 bugs that fresh Codex misses. That's eight sessions, with Claude grading against the fixes. It runs without the checklist and without the new Harnesses.
- (b) **Wait for timing bugs** in Codex's work, as Q16 has it.
- (c) **Run (a), but only as evidence,** with no bar.

*Recommended:* (a). It's eight sessions, and it tests the direction you'd actually run, Claude reviewing Codex. The forward replay is only about timing bugs, but the Second opinion's job (Q1) is catching whatever slips through, so any bug that reached main counts. A bar of 2 out of 3 is hard to pass by luck.

*Answer:* agree

## Round 9 (answered)

**Q31 - Claude's Effort in the replays.** In Q27 you suggested Claude at high instead of max, which saves a lot of your Claude week. Q9 put each family at its best setting so that a result couldn't be put down to a weak setting. Codex stays at xhigh, its best. Effort moves Opus 5.5 a long way: 51.2 at medium and 57.6 at max on AA's index, with high somewhere between. With Q29 and Q30, Claude now runs 54 sessions, in two roles:
- **Claude as the baseline, 30 sessions:** the six-bug replay's fresh and checklist arms, and the forward plan replay's same-family arm. Here Codex has to beat Claude. If Claude runs below its best, a Codex win could come from Claude's lower setting rather than from the family, and that would push toward building something that isn't needed.
- **Claude as the Second opinion being tried, 24 sessions:** the reverse plan replay and the reverse bug replay. Here Claude has to beat Codex. At high, a pass still counts, but a fail wouldn't rule out Claude at max.

The options:
- (a) **High for all 54,** as you suggested. Of the three, it uses the least of your week, but a forward pass could come from the gap in Effort.
- (b) **Max where Claude is the baseline (30), high where Claude is being tried (24).** No candidate ever gets a stronger setting than the baseline it has to beat, so every pass holds. High also becomes the suggested Effort for Claude as a Second opinion, since that's what would have been tested.
- (c) **Max for all 54,** as Q9 has it.

*Recommended:* (b). The only results that get something built are passes, and (b) is the cheapest split where every pass still means what it says. It moves 24 of the 54 sessions down to high. The cost is that a reverse fail leaves Claude at max untested, which is the same kind of half-answer Q5 already accepts.

*Answer:* agree

**Q32 - Is this the plan now?** It replaces Q23's steps 3–7. Nothing below happens until you agree. The sessions: 54 on Claude, 79 on Codex at xhigh (Q29 and Q30 add 13), and 20 on the four new Harnesses.
1. **File Q28's Ticket,** `ready-for-agent`: Codex sessions that run the review ask for fresh sub-agents, as Security sessions do. It changes the test that keeps that line out of implement prompts, and leaves the prompts page as it is.
2. **Commit this grilling log in a PR,** as #495 did, and **rewrite #429's body** around the changed replay plan. Answer the 8 October comment there: Codex's review sub-agents haven't carried the implementer's work, and the Ticket makes that a setting.
3. **Claude writes the replay script and draws the 20 Codex plans.** A fresh agent that hasn't seen the bugs writes the timing checklist.
4. **From tonight, the 54 Claude sessions run alongside Weeding,** at the Effort Q31 settles. They stop at the first limit warning and pick up later.
5. **Then Weeding pauses** while the 79 Codex sessions and the 20 new-Harness sessions run, by commenting out its crontab line. Security runs on thirdshift pause the same way, but only during the Muse arm. Both come back on when the sessions finish.
6. **Grade.** Claude grades the six timing bugs and Codex's three bugs against their fixes. You grade the plans the checks stop, in both directions and without knowing which arm stopped them, plus about 20 Spec findings, in a file in this folder.
7. **Write it up:** a research note in `docs/research/` with the script beside it, in a PR you merge, and a summary on #429. A piece is built only if it passes its bar.

The options:
- (a) **Yes,** and Claude may edit the crontab in step 5.
- (b) **Yes, but you'll edit the crontab yourself.**
- (c) **Change something,** and say what.

*Recommended:* (a).

*Answer:* agree, with considering when to run to-spec and to-tickets skills

*Checked:* both skills are yours to start (`/to-spec`, `/to-tickets`); a session can't start them itself. When to run them is Q33 below.

## Round 10 (answered)

**Q33 - When do `/to-spec` and `/to-tickets` run?** Two pieces of filing come out of this grilling. Q32 had Claude file the first one directly.
- **Q28's Ticket, now:** Codex sessions that run the review ask for fresh sub-agents. It's one line in the Codex Harness and a change to one test. That's too small for a Spec, and `to-spec`'s seam check has nothing to find: the seam is the Codex Harness's existing session tests.
- **Each piece that passes its bar, after the replays:** a Second opinion in a Run, after the Spec review, the plan check, or the Spec review on its own Harness. These are new features, and two questions still wait on the replays' numbers: who acts on a Second opinion's findings, and what happens when the plan check disagrees.

The options:
- (a) **You run `/to-tickets` on Q28's Ticket as soon as this grilling ends,** with this file as its plan. After the write-up merges, a short grilling settles the two waiting questions for each piece that passed. Then you run `/to-spec` and `/to-tickets` for it, as you did for #486.
- (b) **Claude files Q28's Ticket directly,** as Q32 had it. The pieces that pass get `/to-spec` and `/to-tickets` after the write-up, as in (a).
- (c) **Hold Q28's Ticket until the replays report,** and file everything together in one Spec.

*Recommended:* (a). `to-tickets` writes the Ticket in the shape the factory expects and checks it against the code and the ADRs, at no cost. (c) would leave the reviewers' independence up to the model's choice for days, for no gain: the fix doesn't depend on any replay. A built Second opinion needs both waiting questions answered before `to-spec` can write it.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- **Standalone Runs:** "every Ticket's Run" means any Run, standalone or a Ticket's. Whether they get separate switches is a config question for after the replays. (Q3)
- **Nothing is built before its replay.** The second opinion in a Run, the plan check and both Spec-review options each wait for their replay, and a piece that misses its bar isn't built. (Q4, Q6)
- **What passes is opt-in User config,** off by default. (Q3, Q4)
- **The replays change nothing in thirdshift.** A script outside it, which Claude writes once this grilling ends, runs review sessions over past commits. (Q4)
- **The first replays test Codex (GPT-6.1 Sol) reviewing Claude-written work, plus the reverse plan and bug replays.** The reverse Spec replay waits for more Codex Specs, and a reverse timing-bug replay waits for timing bugs in Codex's work. A pass in one direction is half the answer. (Q5, Q16, Q30)
- **The plan replay has two arms in each direction:** the other family, and a fresh session of the same family. You grade the plans they stop without knowing which arm stopped them, and a plan both stop is graded once. The reverse bar is 3 of 20, the same as the forward bar. (Q10, Q16, Q29)
- **The plan check re-applies the Architecture review's four Strong tests** to the published plan: real friction visible in the code, the deletion test, a settled design, no ADR contradicted. It reads the plan, `CONTEXT.md`, the ADRs and the code as they stood when the plan was made. It never reads the Architecture review's session. (Q2, Q10)
- **Replay reviewers only read:** no builds and no tests, one session at a time, in a throwaway worktree. This holds for every arm, the new Harnesses' included. (Q11, Q17)
- **The timing checklist comes first.** A fresh agent writes it from `CONTEXT.md` and the ADRs, without the issues or the git history, before the six-bug replay runs. (Q12)
- **Each new Harness gets one arm on the six-bug replay,** at its best setting: Gemini 3.8 Flash at high, Grok 4.7 at xhigh, Muse Spark 1.3 at max, and MiMo-V2.6-Pro at its one setting. An arm that passes Q6's bar has to pass the Spec replay too before it's suggested. (Q17, Q9)
- **A pass overstates the gain a little.** The replays measure what a Second opinion adds to today's review, and the evidence-backed review will catch some of the same things. (Q4, Q8)
- **The Second opinion's Harness, Model and Effort are set in the User config.** The replays' settings become its suggested defaults: Codex at xhigh, and Claude at high, the setting Claude is tried at as a Second opinion. (Q9, Q31)
- **The bars in Q6 are fixed** before any replay runs. Claude grades the six bugs against their fixes; you read the plans the check stops and grade about 20 Spec findings. (Q6)
- **The six-bug replay reviews each PR as it stood when it merged.** A Second opinion runs after the implement session, so that's what it would see, and #175's faulty check only exists from that point. The six bugs come from five PRs, so each arm runs five reviews. (Q6, Q7)
- **The term is Second opinion,** and the family is a setting. The entry reaches `CONTEXT.md` with the Ticket that builds it, along with the changes to Harness, Model and Effort. (Q7)
- **The evidence-backed work is a Spec, filed now** with `to-spec` and `to-tickets`. It has at least two Tickets, more if that keeps each implement session's context small, and is labelled `ready-for-agent` so a Pickup run takes it. (Q8, Q21)
- **The six-bug replay and the reverse bug replay use today's review skill,** in every arm, the checklist arm included. The Spec replay keeps the old skill, pinned at 479a1fb, to match its recorded baseline. The plan check doesn't use the review skill. (Q26, Q30)
- **The evidence-backed change covers every session that runs the review skill:** the implement session, fresh or continuing, the Spec review and the review Repair. (Q8)
- **Unaddressed findings carry each decline's citation** in place of a one-line reason, and the Unaddressed finding entry in `CONTEXT.md` changes with that Spec. (Q8, Q21)
- **In the evidence-backed review, the reviewer writes the evidence and the author runs it.** A correctness finding's test or command is run as written. A failure obliges the fix, and the test is kept; a pass lets the author decline, citing the run. (Q13)
- **Unread changed files go back to their reviewer once.** Any still unread are named in the review's summary and the PR body. (Q14)
- **A finding declined because someone else must decide becomes a `needs-triage` issue,** linked from the PR's Unaddressed findings, unless an open issue already covers it. (Q15)
- **thirdshift keeps each reviewer's report beside the Session log, on every Harness.** (Q18)
- **The replay script and a research note go in `docs/research/`,** in a PR you merge, with a summary on #429. The raw reports stay in `~/.thirdshift/replays/`, and you grade in a file in this folder. (Q19)
- **Claude runs at max when it's the baseline and at high when it's being tried.** That's 30 sessions at max: the six-bug replay's fresh and checklist arms, and the forward plan replay's same-family arm. The other 24 run at high: the reverse plan and reverse bug replays. Codex runs at xhigh throughout. A reverse fail leaves Claude at max untested. (Q9, Q31)
- **The order of filing:** you run `/to-tickets` on Q28's Ticket once this grilling ends, with this file as its plan. After the write-up merges, a short grilling settles who acts on a Second opinion's findings and what happens when the plan check disagrees, for each piece that passed. Then you run `/to-spec` and `/to-tickets` for it. Q32's step 1 becomes yours. (Q32, Q33)
- **Claude may comment out and restore the Weeding and Security crontab lines** for the Codex and Muse sessions. (Q32)
- **The Claude sessions go first, from 9 October, alongside Weeding.** Weeding pauses only for the Codex sessions, which share its limits. The new Harnesses' sessions run alongside the Codex ones, with Security runs paused only during the Muse arm, which shares Muse's limits with them. (Q20, Q27)
- **The reverse plan replay checks 20 of Codex's 27 plans,** drawn the way the forward sample was, with the same bar of 3. Claude is the other family; Codex in a fresh session is the same family. (Q16, Q29)
- **A reverse bug replay runs now** over Codex's three bugs (#559, #603, #607), with the four PRs as they merged. One arm is Claude, as the other family; the other is Codex at xhigh in a fresh session. Q6's bar applies: Claude must catch at least 2 that fresh Codex misses. Claude grades it against the fixes. There's no checklist arm and no new-Harness arm. The kept reports for #562, #577 and #592 also show whether the factory's own review raised each bug, which is reported alongside but isn't part of the bar. (Q30)
- **Codex's review sub-agents get told to start fresh,** by a small `ready-for-agent` Ticket that gives every Codex session that runs the review the line Security sessions get. The replay script asks the same of every Codex arm, so the replays test what the factory will run. (Q28)
- **The evidence-backed Spec runs before the Codex replays, not alongside them,** for the same reason Weeding pauses: the two would share Codex's limits. (Q20, Q21)
- **#429 stays open until the replays report,** with its body rewritten around the decisions and the replay plan, and the evidence-backed Spec linked. A built Second opinion gets a Spec of its own. (Q22)
- **An ADR comes with the Second opinion's build, if it's built.** A session on a Harness other than its Command's breaks one Harness per Command. That's hard to reverse, surprising without context, and the result of a real trade-off. (Q7)
  - It's still the first break. `security.harness` picks the Harness for a whole Security run, its fix Runs included, and a Run's Security review runs on the Run's own Harness. So one Harness per Command still holds. (Checked 9 October.)
- **Any fresh agent that hasn't seen the bugs can write the timing checklist,** not only Codex. Codex was picked only to save your Claude week, which has now reset. (Q12, Q20)
- **The reverse Spec replay still waits.** Codex has run four Specs: #432, #465, #486 and #427. That's too few to measure against a bar of 6 in 30. When it does run, Codex's own recorded Spec reviews can be its baseline, since #490 keeps their reports. (Q16, Q18)
- **The replays that use the old skill pin it at 479a1fb** (#391, 5 October), the last commit before #486 changed it. (Q21, Q26)
- **The Security review is no Second opinion.** It runs on the Command's Harness and looks for vulnerabilities only, so none of the replays include it. It is the precedent for the shape a Second opinion in a Run would take: a session a Run starts after its opening session, when the User config asks for one. (Q7)

## Research results (2026-10-06)

- No human has reviewed a factory PR since about 27 September: every PR merged itself. Confirmed by the Command logs from 3 October, and by merge timing before that (an inference).
- The "Review clean-ups" commits are the factory's own implement sessions fixing what their same-model review found, confirmed by finding each commit in its session's Session log.
- Until 5 October, almost everything was written and reviewed by Claude Opus 5.5; the one Codex PR was docs (#426). Confirmed by the Session logs and the Activity log.
- Since 5 October, Weeding on thirdshift has run on Codex (GPT-6.1 Sol, xhigh): about 21 merged PRs from 11 plans, plus two Spec runs (#432, #465). Confirmed by the Activity log and the merged PRs.
- Codex Session logs hold none of the reviewers' reports. The sessions wrote them to files in `/tmp`, and only #475's pair survives. Confirmed by tracing #475's Session log and listing `/tmp`.
- Main now supports six Harnesses: Claude Code, Codex, Antigravity (`agy`), Grok Build (`grok`), Muse Code (`muse`) and OpenCode (`opencode`). `CONTEXT.md` still says one Harness per Command. Confirmed by `CONTEXT.md` and `src/harness.rs` on `origin/main`.
- The four new Harnesses' models all score below both current reviewers on AA's index: Muse Spark 1.3 48.1 (max), Grok 4.7 46.4 (xhigh), MiMo-V2.6-Pro 46.3, Gemini 3.8 Flash 40.9 (high), against GPT-6.1 Sol 51.0 (xhigh) and Opus 5.5 57.6 (max). All four can start sub-agents. Confirmed by `docs/research/model-strength-for-review.md` §0 and `docs/research/harness-candidates.md`.
- 16 bugs reached main on thirdshift. About 10 showed up only in live runs (environment, tool versions, harness behaviour). About 6 are timing bugs: what happens when other Runs act at the same moment, or a pass repeats (#24, #57, #102, #175, #182, #303). Confirmed by the issues and their fixes (an agent's sorting, with the six titles checked).
- **Correction:** two of the six timing bugs were caught by a same-model review, then declined as needing a decision nobody made, because the PRs merged themselves. An earlier version of this file said no review raised any of them.
  - #102: PR #22's Spec reviewer described it, and the author declined it because the fix "needs a spec decision".
  - #303: the Spec review of #212 described it, and Spec PR #292 declined it as "the Day shift's decision. Worth an issue of its own."; no issue was filed.
  - No review raised the other four.
  - Confirmed by the PR bodies.
- The six bugs come from five PRs: #57 and #102 both came from #22. #175's faulty check was added in answer to the review's own suggestion, after the reviewers had looked. Seeing #175 and #303 takes context beyond the PR's diff. Confirmed by the replay inputs, built from git and the Session logs.
- Declines that explicitly leave the call to a human or a later issue are rare: 12 of 1,152 declined findings, in 10 of 199 merged PRs. Confirmed by a pattern match over the PR bodies; it's a floor, since the wording varies.
- The replays' inputs are ready:
  - the PR behind each timing bug, with its fix;
  - the 30 Spec branches as they stood before their Spec reviews, with the recorded reviews;
  - 20 plans drawn across the repos (19 Tickets, 1 Spec), each with the commit it was planned on.
  - Confirmed against the local clones.
- Authors decline about 7 findings per PR, and nobody reads them, confirmed by the PRs' Unaddressed findings sections.
- Architect runs: 209 Architecture reviews produced 202 plans and 4 ideas. The review rated its own pick Strong 98% of the time, and nobody else checked a plan. Confirmed by the Activity logs, the Session logs and the `architect-plan` and `architect-idea` issues.
- In Spec PRs, about 28% of declined findings were declined because the plan or an ADR had settled the point, confirmed by a pattern match over the PR bodies (an estimate).
- About half of the 30 Spec reviews fixed a behaviour bug the Tickets' reviews had passed, by the sessions' own summaries (keeplore's c9317c4 checked).
- The two review sub-agents in an implement session take about a minute each, confirmed by Session log timestamps.
- The recorded reviews can be pulled from the Claude Session logs (366 of 390 implement sessions, all 30 Spec reviews). Confirmed by parsing the logs.
- Claude's seven-day usage window reached 70% on 5 October, confirmed by the rate-limit events in the Session logs.
- Skills are built into the binary, and the factory runs whatever `thirdshift update` last installed: 0.10.0, since 6 October, 23:16. A merged change to the review skill reaches the factory only with a release. Confirmed by ADR 0001, `src/update.rs` and the installed binary.
- `to-spec` and `to-tickets` label what they file `ready-for-agent`, so the factory takes it without triage. Confirmed by the skills' text.
- Weeding on thirdshift starts the next Architect run within a minute of the last one ending. Confirmed by the Activity log.
- Effort moves Opus 5.5 from 51.2 (medium) to 57.6 (max) on AA's Intelligence Index. In their own CLIs, GPT-6.1 Sol scores best at xhigh (62.9; 60.1 at max), and Opus 5.5 has only a max row (66.0). Confirmed by `docs/research/model-strength-for-review.md` §4.1, from AA's published results and vendor docs.

## Research results (2026-10-09)

- None of the replay plan has run yet: no script, no checklist, nothing in `~/.thirdshift/replays/`, and nothing on the `429-multi-family` or `grilling-429-second-opinion` branches past main. Weeding on thirdshift is still on. Confirmed by listing the folder, `git log` and `crontab -l`.
- #486 and its four Tickets merged on 7 October (#494, #497, #498, #512, #496). They're in every release since 0.10.1, and the factory runs 0.11.0. Confirmed by `git tag --contains` and `thirdshift --version`.
- The User config now runs the factory on Codex at **high** Effort, not xhigh, and Claude at medium. Security runs use Muse Code (`security.harness = "muse"`), and every Run reviews its change for Security findings (`security.review = true`). Confirmed by `~/.thirdshift/config.toml`.
- `security.harness` only picks the Harness for a Security run. Every other Command still picks one Harness for all its sessions. Confirmed by `CONTEXT.md`'s Harness entry and `src/asks.rs`.
- Weeding on thirdshift was paused from 8 October by Architect idea #564. It started again at 10:37 pm on 9 October, on Codex at high, once #564 was closed. Security runs are being skipped because a Security finding waits for the Day shift. Confirmed by the Activity log.
- **Codex's review sub-agents haven't carried the implementer's work.** Confirmed by reading the parent and child rollouts of 54 Codex implement and Spec-review sessions from 7–9 October, on Codex 0.160.1–0.162.0. What each child saw at first was 17.8k–23k tokens, against 34k–200k in its parent. Forked children held the opening prompt and the implement skill, about 2.5k tokens more, and nothing after that.
  - The model picks `fork_turns` itself: `"none"` 23 times, `"all"` 6 times, and left out 25 times.
  - Why `"all"` didn't carry the parent's turns is an inference: Codex seems to copy only completed turns.
  - thirdshift asks for `fork_turns: "none"` only in Security sessions (`src/harness/codex.rs`), and `tests/codex_sessions.rs` checks that implement prompts don't mention it.
- **Correction:** Weeding first ran on Codex on 6 October, not 5 October. The Architect sessions on 5 October were Claude's. Confirmed by the Session logs.
- **Codex has written 27 plans,** #448–#530, Spec #465 included, from 28 Architect runs between 6 and 8 October. The last of those runs filed idea #564 instead of a plan. Confirmed by `gh issue list -l architect-plan` and the Activity log.
- **Codex has run four Specs:** #432, #465, #486 and #427. #486's Spec runs failed, and it landed as #496. Confirmed by the Activity log and the PRs.
- **Three bugs have reached main from PRs Codex wrote:**
  - #559, from PR #449; found with `git log -S`.
  - #603, from PRs #562 and #577.
  - #607, from PR #592.
  - Two other timing bugs aren't Codex's. #508 comes from #390, which Claude wrote. #509 comes from a 30 September commit, and whether Codex's Worktree work made that race wider is unverified.
  - Confirmed by the issues and their fixes.
- **Reviewer reports are kept on Codex since the factory upgraded on 7 October** (#490). That covers 32 of 32 thirdshift implement sessions since #526 and #427's Spec review. Repair sessions keep none. chart35 has 5 of 6, the missing one cut off before its review, and keeplore has 1 of 1. Confirmed by listing the `.standards` and `.spec` files beside the Session logs.
- **Today's review skill asks for things the old one didn't:**
  - the list of changed files;
  - from both reviewers, a list of the files they read, and a test or command that fails if a behaviour finding is right;
  - an 800-word limit, up from 400;
  - one retry for changed files left unread;
  - reports written where the Session prompt says.
  - The reviewers still only read; the implementer runs their tests. The last commit before #486 changed it is 479a1fb. Confirmed by `git log -- skills/thirdshift-code-review/` and `src/prompt.rs`.

## Filed

- **#486, Evidence-backed review findings:** the Spec from Q8, Q13–Q15, Q18 and Q21. It's `ready-for-agent` now that its Tickets are published, so a Pickup run takes it once it settles.
  - #487: one review instruction for the prompts that run the review. Blocked by nothing.
  - #488: correctness findings carry a test the author runs, and every decline cites its evidence. Blocked by #487.
  - #489: reviewers list the files they read. Blocked by #488.
  - #490: thirdshift keeps each reviewer's report beside the Session log. Blocked by #489.
- **#429's body rewritten** with the decisions, the record and the replay plan, linking #486. It stays open until the replays report. (Q22)
- **#616, Codex sessions that run the code review ask for fresh sub-agents:** Q28's Ticket, filed with `/to-tickets` on 9 October as a sub-issue of #429, labelled `ready-for-agent`. (Q28, Q33)

## Waiting on research

- **The replays themselves,** once the script runs. Two questions wait on their numbers and aren't asked yet: who acts on a second opinion's findings, and what happens when the plan check disagrees.
