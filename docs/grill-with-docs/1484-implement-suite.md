# Grilling: #1484, an implement Eval suite

Issue: https://github.com/JacobStephens2/thirdshift/issues/1484
Research: [`docs/research/eval-suite-scaling.md`](../research/eval-suite-scaling.md) §2 and Recommendations 13–18; [`docs/research/eval-build-vs-buy.md`](../research/eval-build-vs-buy.md) §8.1; [`done/698-evals/settled.md`](done/698-evals/settled.md) Q6.7; [`1434-eval-follow-ups.md`](1434-eval-follow-ups.md) Q1.9

## Round 1 (answered)

**Q1.1 - What does a Trial run?** The suite tests "the implement session", but the fresh Ticket-run prompt (`src/prompt.rs:305`) asks one session to do several things: `/thirdshift-implement`, then a self-review with `thirdshift-code-review`, then fixing the findings, then pushing and opening a pull request. The later Standards, Spec and Security reviews and their Repairs are separate sessions. The options:
- (a) **The implement session alone.** Same skill and prompt, including its self-review. In place of the push and pull request, it commits locally on the Issue branch, as code-review Trials already do. The input is the Ticket's body at the base commit.
- (b) **The whole Ticket run**: implement, the review stages and Repairs, up to the point where a pull request would be ready. That's closer to what the factory merges, but each Trial costs several sessions, and a failure doesn't say which stage caused it.
- (c) Both, as two suites.

*Recommended:* (a). It's the stage that writes the code, it's what #1526 and #1468 need to measure, and its Trials stay one session each like every other suite's. A whole-run suite can come later if a change to the review stages needs one.

*Answer:* Part of my goal with #1484 and #1468 is to get to where I can run an experiment like the one I ran by hand comparing thirdshift's Built-in Ticket run with another team's heavier-review process. That needed work outside thirdshift. I'd like more of that kind of process experiment to run in thirdshift. For example, I want to test an edit to the Ticket run process based on that experiment's findings, using thirdshift's Experiment and Eval tooling rather than the bespoke process I used then.

That experiment also had a website for blind human review of the Arms' outputs, and I wonder whether that kind of UI would help, or be nice to have, as part of thirdshift's Eval tooling. Or I could switch to something like Langfuse instead of continuing to build the eval tooling into thirdshift.

The experiment had its own private repository of tooling, and some of it may be worth bringing into thirdshift, to run these experiments in thirdshift itself or by way of it.

**Q1.2 - Which repositories do the Eval tasks come from?** The other suites' tasks point at local git repositories, so the format doesn't care which one. The options:
- (a) **thirdshift only, to start.** Rust is the one toolchain the Trial setup already provisions (`toolchains.rs`), and its tests and targeted selectors are known. But it's public, so every fix is in its history and may be in a Model's training data (scaling note §3).
- (b) **thirdshift plus a private repository the factory serves**, such as an employer's. Less contamination, but a second toolchain and test setup.
- (c) **Public outside repositories** in SWE-bench style. That means a toolchain per repository, and it no longer tests thirdshift's own kind of work.

*Recommended:* (a), with the format kept repository-neutral. Contamination gets the free time-split probe (scaling note §3.3, probe 1) instead of a second repository. Q1.2's answer decides the sandbox question (bwrap or containers), which comes next round.

*Answer:* I can imagine using third shift to to well I actually already use it for a variety of repositories at educational travel adventures, for example, such as the Torbot repository and the orchestration repo. So I just want to make sure that and I can imagine slightly different processes being used for different repos, but all using third shift to orchestrate those processes.

But I guess here as long as I can optimize different configurations of Thurshift for different repos that it it work in not just itself. I don't want it to just be totally navel gazingly developing in that way. because I actually want it to be useful for other projects.

That's its its primary purpose is outward, not inward. And so I I think that A, I think I agree with the recommendation assuming that it aligns with what I'm or it does not conflict with what I'm saying here 

**Q1.3 - Which past work counts as a real Eval task?** #698 Q6.7 said "a past bug". But most Ticket runs build features, not fixes, and the changes #1526 wants to test reach every Ticket. Fail-to-pass works for features too: the merged pull request's new tests fail at the base and pass at the merge. The catch is that a feature's tests call names the Ticket has to give, or a valid implementation fails to compile against them. The options:
- (a) **Bug fixes only**, as Q6.7 said.
- (b) **Any merged Ticket run's pull request** whose tests fail at the base and pass at the merge, features included. Screen out a task whose tests depend on names the Ticket doesn't state (scaling note Recommendation 18's "would the tests reject a valid fix?").

*Recommended:* (b). The suite should look like the work the implement session actually does, and the screen deals with the naming problem.

*Answer:* Agreed. I think the https://github.com/JacobStephens2/thirdshift/issues/1516 issue is somewhat related here possibly.

**Q1.4 - Real tasks only, or injected ones too?** The scaling note's Recommendation 14 grows the suite with injected bugs: `cargo-mutants` makes candidates, and a mutant that an existing test catches is a usable task. But an implement Trial needs a Ticket to work from, and a mutant has none, so someone has to write one from the failing behavior (SWE-smith has a Model write it). The options:
- (a) **Real only**, at least until the suite has run a few Experiments.
- (b) **Real plus procedural injection** (`cargo-mutants`), each task recording its provenance (`real` / `injected-procedural`), with the Verdict split by provenance (Recommendation 16) and injected tasks kept a minority of a gating suite (Recommendation 6).
- (c) (b) plus Model-written bugs (`injected-model`).

*Recommended:* (a), revised from (b) after the count under Research results. The last 19 days alone gave about 60 usable real candidates, so real tasks can probably reach 20 discriminating ones without injection. Real tasks avoid the mutant-Ticket problem and the question of whether mutants rank Arms the way real bugs do. Add provenance to the format now anyway (every task `real`), so that injection can come later without a format change.

*Answer:* agreed

**Q1.5 - What does the code Grader run, and on what?** Today's checks grade a snapshot taken *before* the session (`execution.rs:116-135`), because review Trials don't change code. Here the Grader has to grade the Trial's own commit. To do that, it puts the fix's test files on top of the Trial's commit and runs the fail-to-pass tests, plus enough pass-to-pass tests to catch breakage. In this repository, 71 source files have inline `mod tests { … }` blocks, so a fix's tests often live in the same file as the code it changes. Laying those files over the Trial's commit would replace its code. The options:
- (a) **Admit only tasks whose fix tests are in separate test files** (`tests/*.rs`, `*/tests.rs`, `*_tests.rs`; 105 integration-test files and 53 split module files today). Overlay those, then run the fail-to-pass tests and the pass-to-pass tests in the same test targets, offline, under a time cap.
- (b) (a), and also take the inline test blocks out of the fix and splice them into the Trial's file. More tasks, more fragile tooling.
- (c) Use an agent Grader for tasks that can't be overlaid.

*Recommended:* (a). It's mechanical and blind, and it fails safe: a task that can't be overlaid is simply not admitted. Graders that only a code check can run stay code checks.

*Answer:* agree

**Q1.6 - Which changes start an Experiment on this suite?** Each `suite.toml` lists dependencies, and a pull request touching one starts the suite. Both existing suites list `thirdshift-code-review/`, `thirdshift-codebase-design/` and `thirdshift-tdd/` as Factory skills, the same prompt, session and Trial source, and the Harness, Model and Effort defaults. The implement skill names `thirdshift-tdd` and `thirdshift-code-review`, and `thirdshift-tdd` names `thirdshift-codebase-design`. Proposed for this suite:
- `factory_skills`: `skills/thirdshift-implement/`, `skills/thirdshift-tdd/`, `skills/thirdshift-codebase-design/`, `skills/thirdshift-code-review/`
- the same `session_prompts`, `harness_defaults`, `model_defaults` and `effort_defaults` as the other suites

`thirdshift-to-tickets` stays out. It writes Tickets, and a Trial starts from a Ticket that's already written. `thirdshift-pr` stays out too, because a Trial doesn't open a pull request. One limit to note: a task's own `AGENTS.md` and `GLOSSARY.md` are as they were at its base commit. So a change to *those* files, such as #1526's second candidate, isn't something this suite can test as written.

*Recommended:* the list above.

*Answer:* agreed

**Q1.7 - Trials per Eval task.** Recommendation 17 says three, for the same reasons as code review: `pass_cubed` needs three, a flaky Trial costs a third of a task instead of all of it, and Difficulty can't be measured from one. The other choice the code allows is one.

*Recommended:* three.

*Answer:* agreed

**Q1.8 - Which machine builds tasks and runs this suite's Experiments?** The 2026-10-27 comment says the Day shift can provide a bigger host with room for container images, but doesn't name it. Tasks live privately on the machine that runs Experiments (`~/.thirdshift/evals/`), and Trials need bwrap, so it can't be the Mac. The options:
- (a) A new bigger host, provisioned for this suite.
- (b) The shared host, thirdshift.app.
- (c) The Mac's Lima VM (Ubuntu 24.04 with bwrap, currently 4 CPUs and 8 GiB, resizable).

*Recommended:* (a) for Experiments, using (c) to build and screen the first tasks while it's being set up. An implement Trial builds and tests Rust three times per task, and that shouldn't compete with the factory on the shared host.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - What is an Arm, and how far does a Trial run? (Q1.1, asked again)** Your Q1.1 answer sets the goal as process Experiments, and the next one you want to run is an edit to the Ticket run itself. The candidate edits from the hand-run comparison are a deterministic check that the new tests fail on the base, one bounded review pass by a different model family, and new rules in the review prompts. All of them live *outside* the implement session: in Steps, in review stages, in review prompts. So an implement-session-only suite (Q1.1's old option (a)) can't test them. The options:
- (a) **The implement session alone**, as recommended in Round 1. It's the cheapest, but it can't test the edit you want next.
- (b) **A process Arm.** An Arm is a thirdshift build plus a process file, Built-in or Custom, with its skills and process file pinned by content hash. A Trial runs that process end to end on the Ticket, through its review stages and Repairs, and stops where it would push. The Grader grades the final commit.
- (c) (b), plus an **external-runner Arm**: a pinned command that takes the repository at its base commit and the Ticket, and returns a branch, its session traces and its usage. thirdshift treats the command as a black box, as Harbor's agent interface does (research note §2). This is the only way to hold a process whose orchestrator is itself an agent choosing what to run next (#1468's triage note).

*Recommended:* (b) now, with (c) recorded as the later step. Comparing the Ticket run with an edited Ticket run needs only process Arms, and #1533's Built-in process files plus #1531 (which splits the code review out of the implement Step) give a Custom process room to change the reviews. An external runner matters only once the next Experiment pits thirdshift against an outside process, so it can wait for that need. Each Trial then costs several sessions instead of one, so the cost estimate shown before Trials (#1434 Q1.6) has to come per Arm.

*Answer:* agreed

**Q2.2 - Build the review and grading tooling into thirdshift, or adopt Langfuse or a similar tool?** The research note (§1) checked Langfuse, Braintrust, LangSmith, Phoenix, promptfoo, Inspect/Scout and Harbor against a process comparison:
- Langfuse "expects exactly one trace per experiment item", while its Claude Code and Codex integrations write one trace per turn. A process Trial can run 4–72 sessions.
- None of the tools shows code diffs side by side, hides which Arm made an output from a *human* reviewer, or randomises which side each Arm appears on. LangSmith's pairwise queue still puts "Run A on the left".
- None computes a Verdict.

The options:
- (a) **Build it in.** thirdshift keeps the runner, the Bar and the Verdict, and adds a blind pairwise review page to the Console. That page records a Day-shift Grader's call as a paired Score. thirdshift exports Trial traces over its existing OTLP output to any outside viewer, opened only *after* the human calls are recorded, so it can't break the blinding.
- (b) **Adopt Langfuse**, or a similar tool, as the experiment store and review UI. The blind page and the Verdict would still have to be built, and there would be two places holding the results.
- (c) **Use Harbor as the runner**, with thirdshift grading. This means a Python dependency and a Docker image per repository, and subscription login works only for Claude and Codex.

*Recommended:* (a). Every outside tool would still need the blind page and the Verdict built. thirdshift already has a Day-shift Grader on `/scores`, a shuffle for its model Graders and a packet stripper to reuse. Harbor's interface is the shape to copy for Q2.1's external runner later, not a dependency to take.

*Answer:* agreed

**Q2.3 - Does the suite include the blind pairwise human review page?** This depends on Q2.2 being (a). If not, it moves to a later round. The hand-run comparison's page worked like this:
- one page per task pair, showing the Ticket, a numbered acceptance checklist, the two diffs side by side (code before tests) and the execution evidence;
- no Arm identity and no model Scores;
- the reviewer first makes a sealed, unaided merge call per side, then optionally fills the model Graders' rubric.

Arena practice adds a choice of A, B, tie or both bad, with the Arms revealed only after the vote. Copilot Arena also found that people picked the first option 86% of the time even with randomised order, so each pair needs its own random sides. The options:
- (a) **In this suite's Spec**, as a Day-shift Grader whose call is reported *beside* the Verdict and doesn't count toward it at first.
- (b) **In this suite's Spec**, with the human call counting in the Verdict.
- (c) **A separate issue**, after the code-graded suite works.

*Recommended:* (c). The code-graded suite (Q1.5) is the minimum that works on its own, and it is what #1526's Experiment needs. The page is wanted, but it's a Console feature of its own. File it now as an issue blocked by this suite, with the hand-run page and the arena rules as its starting design. Once some Experiments have shown how the human calls compare with the code Graders, decide then whether the human call counts.

*Answer:* b, Except I want the option to be able to decide for various experiments whether they are going to include human grading or not, So for example, I want to be able to have experiments that include human gradient in the verdict, that report human gradient besides the verdict, or those which do not include human gradient at all. 

**Q2.4 - Per-repository toolchains: bwrap on the host, or containers?** Q1.2 says the suite must serve other repositories, and in #1434, 4 of 6 cross-repository candidates were discriminating. Today a Trial runs in bwrap with the host's Rust, Cargo and nvm mounted read-only, and nothing else is provisioned. The new host has room for images (#1484's comment). The options:
- (a) **bwrap on the host**, with each repository's toolchain installed on the host and declared in the suite. Containers come only when a repository needs services or a pinned system image (#698 Q6.7's trigger).
- (b) **A container image per repository**, built at task preparation, with every Trial and code Grader run inside it. This also answers the "a commit stops building on the host" problem.
- (c) (a) for thirdshift's own tasks now, and (b) designed in from the start for the first task from another repository.

*Recommended:* (c). thirdshift's own tasks run in bwrap today, so the first Experiment needn't wait for containers. But the outward goal means the task format should name its environment, `host` or an image, from day one, so another repository's tasks don't need a format change later.

*Answer:* agreed

**Q2.5 - Check Trial traces for the real fix?** Every real task's fix is public (Q1.2), and #1468's triage note says implement tasks need a leak check, since the hand-run comparison searched every Trial's traces for the shipped fix. The time-safe clone stops the repository from leaking it, but a session could still fetch the pull request or recall it from training data. The options:
- (a) **Yes.** After each Trial, search its transcripts for the fix's identifiers: the pull-request and issue numbers, the merge commit, the branch and the title. Hits go to the Day shift, with an auditable list of dismissed false positives. A confirmed leak voids the Trial.
- (b) **No.** Rely on the time-safe clone and the Trial sandbox's network allowlist.

*Recommended:* (a). It's cheap and mechanical. Note that the Trial sandbox already reaches only the provider, so a leak would most likely come from the Model's memory, which a search for identifiers catches only partly. The free time-split probe (scaling note §3.3) covers the rest.

*Answer:* agree

## Round 3 (answered)

**Q3.1 - Where does a process Trial stop?** The Built-in Ticket run is four Steps: `implement` (which includes its self-review), `security-review`, `delivery` and `self-merge` (`processes/ticket-run.toml`). Delivery pushes to `origin`, opens the pull request, watches CI, and starts Repairs for conflicts and red checks. A Trial has no GitHub and no CI. The options:
- (a) **Stop before the first publishing Step** (`delivery` or anything after it), and grade the Issue branch's head at that point. An Arm whose Process has no such boundary, or that needs GitHub earlier, is refused.
- (b) **Run a local stand-in for Delivery**: run the repository's checks locally in place of CI, and start Repairs for failures, as the real Repair loop would.

*Recommended:* (a). Every edit you want to test next sits before Delivery: a tests-on-base check would be a command Step, and a cross-family review and new review rules are review Steps. A local Delivery would mean writing a second Repair loop. If a later Experiment needs to test Repairs, it can add one then.

*Answer:* agree

**Q3.2 - Is a process Trial a Ticket run?** The glossary says a Trial "is not a Ticket run, and writes no Command record of its own". A process Trial now carries out the Ticket run's Process, or a Custom process, Step by Step. The private lessons suggest keeping its Command record, which holds Step times, Repairs and usage. The options:
- (a) **A Trial is still not a Command**, and no Ticket run. It *carries out* a Process inside the Experiment, and keeps a Trial record shaped like a Command record (Steps, sessions, outcome, usage) under the Experiment. It never appears in the Console's run lists, Needs you, or Scoring runs.
- (b) **Each process Trial is a real Ticket run Command** with a flag marking it as part of an Experiment.

*Recommended:* (a). A Trial has no Claim, no issue to label, no pull request and no factory side effects. Making it a Command would put fake runs into every place that counts real ones: the Console, Scoring runs and Run metrics. The Experiment can still show its Steps and usage.

*Answer:* agreed

**Q3.3 - What does an Arm pin, and how?** An Arm today is a thirdshift build plus its Harness, Model and Effort, and a Trial runs with a fresh home, so `~/.thirdshift/skills` never reaches it (#1468). A Custom process Arm may load home skills, and different Steps may use different Harnesses and Models. The options:
- (a) **The Arm names a Process (a Built-in name, or a Custom process file) and a list of home skills.** `prepare-arms` copies them into the Arm's frozen folder and records each file's content hash, along with the Harness, Model and Effort per Step. Every Trial gets those copies in its fresh home, and the hashes are checked before each Trial starts.
- (b) **Built-in processes only for now.** An edited Ticket run is a Candidate build with a changed `processes/ticket-run.toml` or prompts. Custom processes and home skills come later.

*Recommended:* (a). Your next Experiment may well be a Custom copy of the Ticket run, and that needs home skills. Freezing by hash is what the hand-run comparison found it needed, since its skills moved dozens of commits in a few days.

*Answer:* agree

**Q3.4 - What happens when a Trial stops to ask a question?** In the hand-run comparison, one Arm stopped twice to ask a fair planning question and delivered nothing. A Trial runs unattended. The options:
- (a) **Each Trial ends as delivered, stopped-to-ask or failed.** Stopped-to-ask and failed both count as a failed Score in the Verdict, and the category is reported beside it. An outside failure, such as a sandbox or plan-limit stop, stays a Void Trial, under rules fixed with the Bar.
- (b) As (a), but stopped-to-ask is a Void Trial.
- (c) Answer questions automatically from a brief fixed with the Arm.

*Recommended:* (a). Under an unattended factory, a Process that stops to ask has delivered nothing, which is just what the Verdict should see. Reporting the category keeps a fair question visible rather than hidden in "failed".

*Answer:* agree

**Q3.5 - Do cost and time enter the Verdict?** A process Trial can run many sessions across providers. The hand-run comparison priced every session's tokens from a frozen price table, labelled each by role, took usage-limit stalls out of the time, and reported paired median cost and time ratios. One of its Arms cost several times more. The options:
- (a) **Reported beside the Verdict**: per-Arm cost and time, paired median ratios and a per-role breakdown. A Trial with incomplete accounting is a Void Trial. The Verdict itself stays about quality.
- (b) (a), plus an optional **cost or time ceiling in the Bar**, such as failing a Candidate that costs more than twice the Baseline whatever its quality.

*Recommended:* (b), with the ceiling optional and off by default. The hand-run comparison pre-registered exactly that kind of rule, and the Bar is where a rule fixed before an Experiment belongs. With the ceiling off, it's the same as (a).

*Answer:* agree

**Q3.6 - How does human grading work in each of its three modes?** Q2.3: the Bar chooses one of three modes: no human grading, human grading reported beside the Verdict, or human grading counted in it. On the blind page the human makes a merge call per side, and can also say which side is better (A, B, tie or both bad). With 20 tasks, 3 Trials and 2 Arms, that's 60 pairs to read. The options for the counted mode:
- (a) **A Trial passes only if the code Grader passes *and* the human calls it ready to merge.** Baseline Trial *k* is paired with Candidate Trial *k*, and every pair is read.
- (b) **The human's pairwise preferences make a second Verdict**, and the Experiment passes only if both Verdicts pass.
- (c) As (a), but the human reads a random sample of pairs fixed by the Bar's seed, and Trials they don't read count on the code Grader alone.

*Recommended:* (a), with the pairwise preference always recorded and reported, in any mode where the human grades. The per-side merge call fits the Verdict's per-Trial pass/fail as it stands. A second Verdict needs new statistics. Sampling makes some Trials easier to pass than others.

*Answer:* agreed. I also want to make it so that an experiment can be set up such that the human only reads a sample of the pairs. So in that example with the 60 pairs to read, I would probably have LLM graders go through all 60 pairs and whatever deterministic testing or checking or evaluation we can do, but I might only want to have ten pairs gone through by humans. 

**Q3.7 - What evidence do Graders and the human page get?** The hand-run comparison found that the evidence that worked was the cross-test: running each Arm's new tests against the other Arm's code. It also ran each Arm's new tests on the unchanged base. The options:
- (a) **Both, as evidence, not Scores**: base-tests and cross-tests, shown on the human page and given to any agent Grader. A cross-test failure is something to look into, not an automatic defect.
- (b) Base-tests only.
- (c) Neither for now.

*Recommended:* (a). It's mechanical, it runs in the same offline sandbox the code Grader already uses, and it's what showed defects that the diffs and the fail-to-pass tests missed.

*Answer:* agree

**Q3.8 - Pairwise model Graders in this Spec?** The hand-run comparison also used model Graders: a panel from several model families, each reading a pair in both presentation orders (a flip counts as a tie), with a per-criterion rubric. They're useful, but each was the same model as one Arm, and their preferences pointed the way self-preference predicts. The options:
- (a) **Not in this Spec.** The code Grader plus optional human grading is enough. Pairwise model grading becomes a follow-up issue.
- (b) In this Spec, reported beside the Verdict.

*Recommended:* (a). It keeps the Spec to what the next Experiment needs. The research also showed model Graders and the human valued different things, so the human page should come first.

*Answer:* b

## Round 4 (answered)

**Q4.1 - With a sample, what decides the pairs the human doesn't read?** Q3.6: in counted mode a Trial passes only if the code Grader passes and it's called ready to merge. The Bar can also limit the human to a sample, say 10 of 60 pairs, while model Graders read all 60 (Q3.8 (b)). For the 50 unread pairs, someone else has to make the merge call. The options:
- (a) **The model Graders' merge call stands in for the human's** on unread pairs. The sample is drawn by pair with the Bar's seed, so both sides of a pair always have the same reader, and Baseline and Candidate are judged the same way. The Experiment reports how often the human and the models agreed on the sampled pairs. If agreement falls below a level the Bar fixes, the Verdict is inconclusive.
- (b) **A sample is only allowed when human grading is reported beside the Verdict.** Counted mode means every pair is read.
- (c) **The Verdict counts the sampled pairs alone**, with fewer pairs and a wider interval.

*Recommended:* (a). It's what you described, and it's fair between Arms because each pair has one reader. The agreement check stops a model panel that disagrees with you from quietly deciding the Verdict. The sampled pairs double as the human-agreement measure the hand-run comparison reported.

*Answer:* agreed

**Q4.2 - What is the model Grader panel?** Q3.8 (b) puts pairwise model Graders in this Spec, and Q4.1 may let their calls count. Today there's one model Grader, a fresh session that reads a packet with shuffled neutral labels. In the hand-run comparison, each Grader was the same model as one Arm, and their preferences leaned the way self-preference predicts. The options:
- (a) **A panel fixed in the Bar, with Models from at least two families.** Each Grader reads every pair in both orders, and a call that flips with the order counts as a tie. The panel's call is the majority. A Grader from the same family as either Arm's Model is reported, but doesn't vote when another family is available.
- (b) One model Grader, read in both orders.
- (c) A panel, but every family votes.

*Recommended:* (a). Two families and both orders were what the hand-run comparison needed in order to trust model calls at all. Leaving same-family Graders out of the vote follows the rule the scaling note gives for injectors (§2.4 risk 3).

*Answer:* agreed, and I want the option to set the model Grader panel to include a number of models - perhaps at least from 1 - 7 models. For example I might want a panel of:
1. Fable 5.2 via claude from Anthropic
2. Gemini 4 Argon via agy from Google
3. GPT-6 Astra via codex from OpenAI
4. Kimi K3.1 Pro from Fireworks on whatever harness /research suggests would be best for thirdshift to use to run this model
5. Muse Wonder via muse from Meta
6. Grok 4.7 via grok from SpaceXAI
7. DeepSeek V4.1 Flash 1017 from Baseten on whatever harness /research suggests would be best for thirdshift to use to run this model

I might even want to be able to run a few models from a given family, as different models within a given family can have various strengths I think (may want to /research to check me on that, but that's my current impression), so these I may also want to run:
8. Opus 5.5 via claude from Anthropic
9. GPT-6.1 Sol via codex from OpenAI
10. GPT-6.1 Terra via codex from OpenAI
11. Sonnet 5.5 via claude from Anthropic

12. Muse Wonder Full via muse from Meta when it becomes publicly available
13. Gemini 4 Carbon via agy from Google when it becomes publicly available

**Q4.3 - How is Difficulty measured on process Trials?** Admission needs at least 6 Trials per candidate in an A/A (#1477). With about 60 candidates, that's about 360 Ticket runs up to Delivery, each several sessions. The options:
- (a) **In batches.** Screen candidates by reading them (Q1.3, and SWE-bench Verified's two questions), then measure about 20 at a time with the Built-in Ticket run on both Arms. Stop once 20 are discriminating.
- (b) Measure all candidates at once.
- (c) Measure with the implement session alone, as a cheaper stand-in.

*Recommended:* (a). At #1434's rate of about one in three, reaching 20 discriminating tasks takes about three batches, and you can stop early. (c) measures a different thing, so its Difficulty wouldn't carry over to process Trials.

*Answer:* agree

**Q4.4 - How are the issues laid out?** The design now covers several things:
- the task format (provenance, environment, Process) and how tasks are built and screened;
- process Trials, with the code Grader, leak check, cost, and outcome categories;
- the blind human page and its three modes;
- the pairwise model panel.

#1468 asked how an Arm is defined, and Q2.1 answered it except for the external runner. The options:
- (a) **#1484 becomes one Spec, with Tickets for each part.** #1468 is narrowed to the external-runner Arm and stays `needs-triage`. A host Ticket (`ready-for-human`) covers building, screening and measuring the first tasks on the Lima VM, then the new host.
- (b) Two Specs: one for process Trials and the code-graded suite, one for human and model pairwise grading.

*Recommended:* (b). The code-graded suite is enough on its own for #1526's Experiment and your first Ticket-run edit. Pairwise grading is a large Console feature that can follow without holding it up. The second Spec is blocked by the first. #1468 is narrowed as in (a).

*Answer:* agreed, created with the to-spec and to-tickets rules

## Round 5 (answered)

**Q5.1 - How small can a panel be?** Q4.2 settled a panel from at least two families. Your answer asks for anywhere from 1 to about 11 Models, with several from one family. The two fit together if the Bar allows any size and only the stricter uses demand more. The options:
- (a) **Any panel from one Model up**, with its family make-up reported beside the Verdict. When model calls *stand in* for the human on unread pairs (Q4.1), the voting panel must span at least two families, neither of them an Arm's, or the Experiment refuses to start.
- (b) Always at least two families.
- (c) Any size, with no family rule anywhere.

*Recommended:* (a). A single-Model panel is useful for cheap runs, where model calls are only reported. Once model calls decide the Verdict for a person, they need the cross-family check the hand-run comparison relied on.

*Answer:* agreed I think, though I'm torn between a's probably best practice guiding and the flexibility of c - which could even be used to test the claims which a is based on, so I probably lean towards c.

**Q5.2 - What happens on a tie in the panel?** With an even number of voters, or with order-flipped calls counted as ties, the panel can split evenly. The options:
- (a) **A tied merge call counts as not ready to merge, and a tied preference is a tie.** Both Arms face the same rule.
- (b) A tie goes to the human, adding that pair to the sample.
- (c) A tied merge call counts as ready.

*Recommended:* (a). It's the cautious reading for an unattended factory, and it's symmetric between the Arms. (b) would make the sample depend on the outcome and break the seeded draw.

*Answer:* I'm leaning towards c if there are other non merge call counts that show the candidate as preferable, mabye the merge call count is the same but the candidate is cheaper to run for example.

**Q5.3 - How is the panel's cost handled?** With 60 pairs, 2 orders and *N* Models, a panel of 11 means about 1,300 Grader sessions. Some Models (Kimi on Fireworks, DeepSeek on Baseten) are billed by API price, not subscription. The options:
- (a) **The pre-Experiment estimate includes the panel**, per Model, priced from the same frozen price table as the Arms (Q3.5). Panel spend is reported apart from the Arms' cost, and the Bar's cost ceiling applies only to the Arms.
- (b) As (a), plus a separate spending limit for the panel, which pauses grading when reached.

*Recommended:* (b). Grading pauses rather than failing (as an unavailable Grader already does), and API-billed Models can spend real money without the plan-limit stops subscriptions have.

*Answer:* agreed

## Round 6 (answered)

**Q6.1 - Family rules as Bar settings?** Q5.1 leans toward (c), no family rule, so that an Experiment can test the very claims the rules rest on. Q4.2 agreed one family rule, and Q5.1 (a) proposed a second:
- a Grader from either Arm's family doesn't vote while another family is available;
- model calls can stand in for the human only when the voting panel spans two families.

The options:
- (a) **Both rules are Bar settings, on by default.** An Experiment can switch either off, and the Bar records it. The panel's family make-up and each Grader's own calls are always reported, so an Experiment with the rules off measures whether same-family Graders actually lean toward their family's Arm.
- (b) Both are Bar settings, off by default.
- (c) No family rules at all, only reporting.

*Recommended:* (a). It gives the flexibility you want, and it lets you test the claims. The default stays where the evidence points until your own Experiments say otherwise. Research on how independent same-family Graders are is pending, and if it changes the picture, the default can move without a format change.

*Answer:* agreed

**Q6.2 - Same quality, but cheaper: how does that show?** Your Q5.2 answer is about a Candidate whose merge-ready calls match the Baseline's but which costs less. A tie rule on each side's merge call doesn't get you there. That tie happens inside one pair's call, and counting it as ready would raise *both* Arms equally, so it never favours the cheaper one. What you want is already partly there: under ADR 0025, a Candidate no worse than the Baseline beyond the margin already **passes**, and Q3.5 reports paired cost and time ratios beside the Verdict. The options:
- (a) **Keep Q5.2 (a) for a tied merge call (not ready), and leave cost beside the Verdict.** You read "pass" next to "0.6× cost" and decide.
- (b) (a), plus a **combined line in the Verdict**: when quality passes and the Candidate's paired cost (or time) ratio is credibly below 1, the Verdict reads "pass: no worse, and cheaper" (or "faster"). When quality passes but the Candidate costs more, it says that too. The quality decision itself stays unchanged.
- (c) Make cost part of the quality decision, so that a cheaper Candidate can pass with a wider quality margin.

*Recommended:* (b). It puts "same quality, cheaper" on the Verdict's first line without mixing cost into the quality statistics.
 That mixing is what (c) would do, and it would need an ADR of its own. A tied merge call stays "not ready", the same for both Arms.

*Answer:* agreed

**Q6.3 - One vote per Model, or one per family?** (Added after the research returned.) [`docs/research/eval-grader-panel.md`](../research/eval-grader-panel.md) §2–3 checks your impression that Models within one family have different strengths:
- On errors, two Models from one family are only slightly more correlated than two from different families (0.437 against 0.389), and among frontier Models no more so. So a second Model from a family does add signal.
- Family preference, a Grader favouring work from its own family, is real and points one way. A five-family panel shrank it on code but didn't remove it.
- Accuracy stops improving at about five judges, and a plain majority is enough.

The options:
- (a) **One vote per family.** Each family first takes the majority of its own Models, and the panel then takes the majority of families. Several Models per family still help, by steadying that family's vote, and each Model's calls are reported.
- (b) **One vote per Model.** A family with four Models outvotes one with one.
- (c) A Bar setting, defaulting to (a).

*Recommended:* (c). Per-family voting keeps a well-stocked family's shared preference from deciding the result. That's the one bias the evidence says is real, and it points one way. As a Bar setting, like Q6.1's family rules, it can still be switched to (b) for an Experiment that tests whether it matters. The research leaned further, toward requiring *three* voting families before the panel decides for the human. That's a stricter version of Q6.1's two-family rule, and the number can be a Bar setting too, defaulting to two.

*Answer:* agreed

**Q6.4 - Which Harness runs Kimi on Fireworks and DeepSeek on Baseten?** You're open to adding Harnesses (research note §1). What the research found:
- OpenCode, already a thirdshift Harness, lists both providers with tool calling for these exact Models (`fireworks-ai/accounts/fireworks/models/kimi-k3p1-pro`, `baseten/deepseek-ai/DeepSeek-V4.1-Flash-1017`).
- To use them, thirdshift needs three changes:
  - add `api.fireworks.ai` and `inference.baseten.co` to the proxy's allowed provider hosts (`src/proxy.rs`), since a Grader naming either provider fails today as "unknown provider";
  - let OpenCode hold one API key per provider instead of a single `opencode.api_key`;
  - build the panel itself, since `model_grader.rs` picks one Grader, and its exclusion rule compares exact Model names, not families.
- A new Harness for either Model isn't worth it yet:
  - Kimi CLI can run headless (`--print`, `stream-json`), but its docs say nothing about usage output.
  - DeepSeek's own agent is a preview desktop and web app with no headless mode.
  - An adapter is about 1,000 lines, and a short Grader session gains little from the vendor's own agent loop.

The options:
- (a) **OpenCode for both**, with the three changes above, and Kimi CLI looked at again if Kimi ever becomes an Arm.
- (b) A new Kimi CLI Harness now, with OpenCode for DeepSeek.

*Recommended:* (a).

*Answer:* b

## Round 7 (answered)

This round should empty the frontier. When it's answered, confirm whether the design is understood well enough to publish the two Specs.

**Q7.1 - What if Kimi CLI can't report usage?** Q6.4 (b) adds a Kimi CLI Harness. The research found Kimi CLI's docs say nothing about token usage in its output. The panel's cost estimate and spending limit (Q5.3) need usage per Grader session. The options:
- (a) **The Harness Ticket starts by checking Kimi CLI's real output.** If it reports usage, use that. If not, the Harness estimates from the transcript's characters, priced from the frozen table and marked *estimated* in every report, and the panel's spending limit counts the estimate.
- (b) If it can't report usage, fall back to OpenCode for Kimi.
- (c) Leave Kimi's spend unreported.

*Recommended:* (a). It keeps your choice of Harness, and an honest "estimated" label is better than a gap. An estimate that's wrong by a factor would show up against Fireworks' billing in the first pilot.

*Answer:* agreed

**Q7.2 - Where does the Kimi CLI Harness go?** A Harness isn't just for Graders: once it exists, an Arm can use it too, and so can Ticket runs. The options:
- (a) **Its own issue**, outside both Specs, so the Harness is built and tested like the other six (setup, sandbox hosts, sign-in, usage, session traces). Spec 2's panel Ticket is blocked by it only for Kimi seats.
- (b) A Ticket inside Spec 2.

*Recommended:* (a). A Harness is used across thirdshift, and earlier Harnesses each came as their own piece of work. Spec 2 can start with OpenCode, claude, codex, agy, grok and muse seats while it's built.

*Answer:* agreed

**Q7.3 - Record two decisions as ADRs?** Two decisions are hard to reverse, surprising without context, and the result of a real trade-off:
1. **A Trial carries out a Process but is not a Command** (Q3.2). A future reader will wonder why a process Trial, which runs a whole Ticket run's Steps, writes no Command record and never shows in the Console's run lists.
2. **A Grader panel can stand in for the Day shift on unread pairs, guarded by agreement on a seeded sample** (Q4.1, Q5.1, Q6.1, Q6.3). The Verdict may then rest mostly on model calls, which is surprising for a gate, and the agreement level and family rules are its safeguards.

The options:
- (a) Both ADRs, written now beside this file and linked from the Specs.
- (b) Only the second.
- (c) Neither.

*Recommended:* (a).

*Answer:* On 1, I'm assuming that we still get a log of the trial run. Even though it's not a command. I'm even thinking like is is is this a good distinction to make? Or is it kind of arbitrary?

Like a trial run as a command versus just a trial?

On 2, For the human grading, I I would like let's say in that example where we have the human grade a sample of ten. And but all the model graders are grading 60 pairs I don't feel like that other 50 would need to be that in the human pair set I don't think it would need to be filled in by an LLM grader.

We could just leave it not not graded and just take the humans sample set as their grade and contribution. 

## Round 8 (answered)

**Q8.1 - Is "a Trial is not a Command" a real distinction?** (Q7.3's answer.) First, yes: a Trial keeps a full log. Its record holds each Step, every session's traces, the outcome, the usage and the times, and the Experiment's pages show it. What it doesn't get is a Command record of its own. The distinction isn't arbitrary. The glossary already draws it the same way for a **Command**: "A child Ticket run, whether for a Spec's Ticket or a Base fix, and a Ticket run or Spec run a Custom process starts, are part of the command that started them, not commands of their own." An **Experiment** is itself a Command, so its Trials are its parts, as a Spec run's child Ticket runs are the Spec run's parts. What being a Command of its own would bring with it:
- a Claim, issue labels and a pull request, none of which a Trial has;
- a row in the Console's run lists, in Needs you and in the Activity log;
- a place in Scoring runs' samples and in Run metrics, where Trials would mix fake runs into real factory figures.

The options:
- (a) **Keep the distinction.** A Trial is part of its Experiment Command. Its record reuses the Command record's *shape* (Steps, sessions, usage), so the Console's run-detail and trace views can show it, but it's filed under the Experiment and counted only there. Write the ADR, citing the child-Ticket-run precedent.
- (b) **Make each Trial a Command**, flagged as part of an Experiment, with every factory view filtering the flag out.
- (c) Keep the distinction, and skip the ADR, since the glossary's Command entry already explains it.

*Recommended:* (a). (b) puts a filter in every place that counts Commands, and one missed filter skews a factory figure. Reusing the record's shape gets you the views without that risk. The ADR is still worth writing, because a process Trial is the first part of a Command that runs a whole Process on its own.

*Answer:* agreed

**Q8.2 - The second ADR?** Q7.3's answer covered only the first ADR. The second records that a Grader panel can stand in for the Day shift on unread pairs, guarded by agreement on a seeded sample and by the family rules.

*Recommended:* write it.

*Answer:* I want to think more about whether or not well, I'm imagining a setup where, for example, we have maybe a greater panel of five models and then a human greater or maybe two human greaters for example and each of the five models grades fifty pairs, but the humans only grade ten of the pairs.

Can that be enough without the a model grading those unred pairs? 

**Q8.3 - Shared understanding?** Every other branch is settled (see the list below). If Q8.1 and Q8.2 are agreed, is the design understood well enough to write the ADRs and publish the two Specs, the Kimi CLI Harness issue, the host Ticket and #1468's narrowing?

*Answer:* Let's clarify 8.2 first

## Round 9 (open)

**Q9.1 - With humans reading only a sample, who decides the Verdict?** (Q8.2's answer.) Your example: a panel of five Models reads all 50 pairs, and one or two people read 10. "Can that be enough without the models grading the unread pairs?" Yes, if what's counted is chosen to match. Treat the panel and the people as two separate switches in the Bar, each set to *off*, *beside* the Verdict, or *counted*. The human switch also takes a sample size. The combinations cover every setup you've described:

| Panel | People (sample) | What the Verdict rests on |
|---|---|---|
| counted | beside, 10 of 50 | Code Grader plus the panel's merge calls on all 50 pairs. People's calls on their 10 are reported beside, with people–panel agreement. The people calibrate and don't decide. |
| beside | counted, 10 of 50 | Code Grader plus people's merge calls on **only the 10** pairs they read. The panel is reported beside. |
| counted | counted, 10 of 50 | Q4.1's stand-in: people's calls on their 10, the panel's on the other 40, and an inconclusive Verdict if agreement on the 10 falls below the Bar's level. |
| off / beside | counted, all 50 | People on every pair (Q3.6). |

Is 10 pairs enough on its own (row 2)? Only for a large difference. The hand-run comparison's 8–0–3 preference over 11 pairs would come out decisive. A modest difference would give a wide interval and an **inconclusive** Verdict, but not a wrong one, since the Verdict already reports how many more pairs it would need. The options:
- (a) **The two switches, with all four rows allowed**, the Bar recording which.
- (b) Only rows 1 and 4: people either calibrate or read everything.
- (c) Only Q4.1's stand-in (row 3), as agreed.

*Recommended:* (a). It's the flexibility you've asked for throughout, each row is a clear statement of what decided the Verdict, and the Bar fixes the row before the Experiment starts.

*Answer:* 

**Q9.2 - How do two or more people grade?** Your example has two human Graders. The options:
- (a) **Each person gets a seeded sample, with a set overlap.** For example, both read 4 of the same pairs, and each reads 3 more alone. On a shared pair, they must both call it ready for it to count as ready, and a disagreement counts as not ready, as a tied panel does (Q6.2). People–people agreement on the overlap is reported beside people–panel agreement.
- (b) Both read the same pairs, with a majority across people.
- (c) Their samples are split, with no overlap.

*Recommended:* (a). The overlap is what tells you how much two careful people disagree, which is the ceiling for how well any panel can agree with them. The rest of each sample widens coverage. The overlap size is a Bar setting, and 0 gives (c).

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- thirdshift keeps its own runner, its blind Graders, the Bar and the Verdict. Disk was never why a runner swap was declined (build-vs-buy §8.1).
- The real-task spine is Q6.7's construction: prepared once at the base commit with network, fix tests checked to fail at the base and pass at the fix, a time-safe clone, and a read-only copy for each Trial. Code Graders run with no network.
- Admission is #1477's rule, already implemented: Difficulty measured over at least 6 Trials, discriminating between 0.15 and 0.95, and Advisory below `min_discriminating_tasks` (20 by default).
- Disk and Docker are no longer constraints (the 2026-10-27 comment on #1484).
- #1434's code-review repair, the prerequisite for this grill, is closed.
- **Q1.1 is answered by Q2.1: a process Arm.** It was open after Round 1, because the answer widened the goal beyond the implement session. The goal is to run process-comparison Experiments through thirdshift: the Built-in Ticket run against another process, and next an edit to the Ticket run, on thirdshift's own tooling rather than bespoke tooling outside it. It also asks whether a blind human pairwise review page belongs in thirdshift, or whether thirdshift should adopt a tool such as Langfuse instead. Round 2 asks it again with research in hand.
- thirdshift's purpose is outward (Q1.2). The first tasks come from thirdshift itself, but nothing in the suite may assume thirdshift: the task format, the toolchain setup and the Arm's configuration must work for any repository the factory serves, and different repositories may run different processes. Per-repository toolchains (bwrap on the host or containers) therefore come up in Round 2.
- Real tasks include features as well as bug fixes, with a screen that rejects a task whose tests depend on names the Ticket doesn't state (Q1.3).
- Real tasks only for now. Every task records its provenance, `real` for now, so injected tasks can come later without a format change (Q1.4).
- The code Grader grades the Trial's own commit. It lays the fix's separate test files over that commit, along with any shared test fakes the fix changed, and runs the fail-to-pass tests and the pass-to-pass tests in the same test targets, offline and under a time cap. A task whose tests are inline in a source file isn't admitted (Q1.5).
- The suite's dependencies are the implement, tdd, codebase-design and code-review Factory skills, plus the other suites' prompt, Harness, Model and Effort entries (Q1.6).
- Three Trials per task (Q1.7).
- A new, bigger host runs the Experiments. The Lima VM builds and screens the first tasks until that host exists (Q1.8).
- Supply is tight. #1434's candidates came out about a third discriminating (7 of 20, #1516), and about 60 usable real candidates at that rate gives roughly 20. That's just enough, so another repository's tasks would also help reach 20. In #1434, 4 of the 6 cross-repository candidates turned out discriminating.
- An Arm is a thirdshift build plus the Process its Trials carry out, Built-in or Custom, with its skills and Process pinned by content hash (Q2.1). The glossary's **Arm** entry now says so. A Trial runs that Process end to end on the Ticket. So this is no longer only an *implement* suite: it tests the whole Ticket run up to the point Q3.1 settles.
- Q1.6's dependencies grow to match. They add `processes/` and every Factory skill a Ticket run's Steps load before Delivery, such as `skills/thirdshift-security-audit/` for the Security review.
- Each Trial now costs several sessions, so the cost estimate shown before an Experiment comes per Arm, from that Arm's own history (#1468's triage note).
- An external-runner Arm, a black-box command shaped like Harbor's agent interface, is the later step, for when an Experiment pits thirdshift against an outside process. It's recorded on #1468 rather than built here (Q2.1).
- thirdshift builds its own review and grading tooling. Outside viewers get Trial traces only through the existing OTLP export, and only after the human calls are recorded (Q2.2).
- Human grading is in this suite's Spec. The Bar fixes one of three modes before the Experiment starts: none, reported beside the Verdict, or counted in it (Q2.3). The blind page randomises the sides for each pair and hides Arm identity and model Scores until the calls are recorded.
- The task format names each task's environment, `host` or a container image. thirdshift's own tasks use `host` (bwrap) now, and the first task from another repository brings the image path (Q2.4).
- After each Trial, its transcripts are searched for the real fix's identifiers. The Day shift dismisses false positives on an auditable list, and a confirmed leak voids the Trial (Q2.5).

- A process Trial stops before the first publishing Step (`delivery` or anything after it), and the Issue branch's head at that point is graded. An Arm whose Process needs GitHub before then is refused (Q3.1).
- A Trial is not a Command and not a Ticket run. It carries out a Process inside the Experiment, and keeps a Trial record of its Steps, sessions, outcome and usage under the Experiment. It never appears in the Console's run lists, Needs you or Scoring runs (Q3.2). The glossary's **Trial** entry now says a Trial may be many sessions.
- An Arm names a Process (a Built-in name or a Custom process file) and its home skills. `prepare-arms` freezes them with content hashes, along with the Harness, Model and Effort per Step, and the hashes are checked before each Trial (Q3.3).
- A Trial ends delivered, stopped-to-ask or failed. Stopped-to-ask counts as failed in the Verdict and is reported beside it. Outside failures stay Void (Q3.4).
- Cost and time are priced per session and role from a frozen price table, with usage-limit stalls left out of the time. They're reported beside the Verdict as paired median ratios, and a Trial with incomplete accounting is Void. The Bar may set a cost or time ceiling, off by default (Q3.5). The glossary's **Bar** entry now lists it.
- In counted mode, a Trial passes only if the code Grader passes and it's called ready to merge. Baseline Trial *k* is paired with Candidate Trial *k*. The pairwise preference (A, B, tie or both bad) is always recorded where anyone grades pairs. The Bar may limit the human to a sample of pairs, and model Graders read every pair (Q3.6).
- Base-tests and cross-tests are run offline and given to the human page and every model Grader as evidence, never as Scores (Q3.7).
- Pairwise model Graders are in this Spec (Q3.8). This reverses the recommendation, and follows from Q3.6's answer, since model Graders read every pair.

- On pairs the human doesn't read, the model panel's merge call stands in for the human's. The sample is drawn by pair with the Bar's seed. Human–panel agreement on the sample is reported, and agreement below the Bar's level makes the Verdict inconclusive (Q4.1).
- The panel is fixed in the Bar. Each Grader reads every pair in both orders, and an order flip is a tie. A Grader from either Arm's family is reported but doesn't vote while another family is available. The panel can hold 1 to about 11 Models, several from one family, over any Harness thirdshift supports (Q4.2). Q5.1 and Q5.2 settle the size and tie rules, and the vote unit and Harness choices are waiting on research.
- Difficulty is measured in batches of about 20 screened candidates, with the Built-in Ticket run on both Arms, until 20 are discriminating (Q4.3).
- Two Specs, written with the `thirdshift-to-spec` and `thirdshift-to-tickets` rules and `docs/agents/triage-labels.md`'s publication procedure. The first covers process Trials and the code-graded suite, with a `ready-for-human` host Ticket for building and measuring the tasks. The second, blocked by the first, covers the blind human page and the pairwise model panel. #1468 is narrowed to the external-runner Arm (Q4.4). Publishing waits until this grill's frontier is empty.

- A panel can be any size from one Model up, and its family make-up is always reported. Whether family rules apply is asked again as Q6.1 (Q5.1).
- The pre-Experiment estimate includes the panel, per Model, priced from the frozen price table. Panel spend is reported apart from the Arms, and the panel has its own spending limit that pauses grading when reached. The Bar's cost ceiling covers only the Arms (Q5.3).
- Q5.2's answer was really about Verdicts, not ties, so it's asked again as Q6.2.

- Both family rules are Bar settings, on by default: a Grader from either Arm's family doesn't vote while another family is available, and model calls stand in for the human only when the voting panel spans enough families (Q6.1). Each Grader's own calls are always reported.
- A tied merge call counts as not ready, for both Arms. When quality passes and the Candidate's paired cost or time ratio is credibly below 1, the Verdict says "pass: no worse, and cheaper" (or "faster"). When quality passes and the Candidate costs more, it says that too. The quality decision itself is unchanged (Q6.2).
- The vote unit is a Bar setting, defaulting to one vote per family, where each family first takes the majority of its own Models. How many families the panel must span before it stands in for the human is a Bar setting too, defaulting to two (Q6.3).
- DeepSeek on Baseten runs through OpenCode. Kimi on Fireworks gets a new Kimi CLI Harness now (Q6.4 (b), against the recommendation). thirdshift adds `api.fireworks.ai` and `inference.baseten.co` to the proxy's provider hosts, and an API key per OpenCode provider.
- The glossary gains **Grader panel**.

- The Kimi CLI Harness work starts by checking the CLI's real output. If it doesn't report usage, the Harness estimates usage from the transcript, priced from the frozen table and labelled *estimated*, and the panel's spending limit counts the estimate (Q7.1).
- The Kimi CLI Harness is its own issue, outside both Specs. Spec 2's panel blocks on it only for Kimi seats (Q7.2).

- A Trial is part of its Experiment Command, as a Spec run's child Ticket runs are part of it. It keeps a full log: a record shaped like a Command record, with Steps, session traces, outcome, usage and times, filed under the Experiment and shown by the Console's run-detail and trace views. It is counted nowhere else. An ADR records this, citing the child-Ticket-run precedent (Q8.1).
- The second ADR (the panel standing in for people) and the shared-understanding check wait on Q9.1 and Q9.2 (Q8.2, Q8.3).

## Research results (2026-11-09)

- Two suites exist, `code-review` (3 Trials) and `spec-review` (1). Their tasks live outside the repository at `~/.thirdshift/evals/suites/`, confirmed by source (`src/evals/import.rs:121`, `src/evals/bug_origin.rs:42`).
- A task's `Session` is only `CodeReview | SpecReview`. Nothing is implement-shaped, and no task records provenance. Confirmed by source (`src/evals/format.rs:17-22`).
- Trials run only in bwrap, with no container support. Only Rust is provisioned (`rustup` per pin); host `~/.cargo`, `~/.rustup` and `~/.nvm` are mounted read-only. Confirmed by source (`src/sandbox.rs:692-737`, `src/evals/experiment/execution/toolchains.rs`).
- Code-Grader checks grade a copy of the repository taken before the session, so that review Trials can't change what's graded. Confirmed by source (`src/evals/experiment/execution.rs:116-135`).
- Difficulty and admission are implemented, with A/A pooling, ≥6 Trials, the 0.15–0.95 band and a 20-task Advisory floor. Confirmed by source (`src/evals/format.rs:57-65`, `src/evals/experiment.rs:110-125`).
- Trials per task must be 1 or 3. The per-Trial timeout is 1800 s by default and also bounds checks. Confirmed by source (`src/evals/experiment.rs:68,90,262-265`).
- The fresh Ticket-run prompt has the implement session self-review with `thirdshift-code-review`, then push and open a pull request. Confirmed by source (`src/prompt.rs:305-323`).
- This repository has 71 source files with inline `mod tests { … }` blocks, 53 split test-module files and 105 integration-test files, confirmed by `rg` count.
- #1526 (trim implement-session reading) and #1468 (skill sets or Custom processes as Arms) both need this suite before their Experiments can run.

- **The supply of real tasks** (for Q1.3–Q1.5), from the last 300 merged pull requests (2026-10-21 to 2026-11-09), classified by their `git diff`:
  - 198 changed `src/` or `tests/`, after leaving out releases and docs-only pull requests. 186 of those close a Ticket: 46 bugs and 140 features.
  - **92 have a Ticket, change `src/` and keep their tests only in separate files: 24 bugs and 68 features.** 66 more touch inline `mod tests` blocks (about 35%).
  - About 25 of the 92 are large refactors or Spec pull requests (500–2,500 lines), which make poor tasks. That leaves about **60–65 usable**, with a median of 84 changed non-test lines.
  - A sample of three: #1374 and #1263 are usable, since the Ticket names, or the base already has, what the tests call. #1603 isn't, because its tests assert a URL, attributes and headings that its three-line issue never states. That's the Q1.3 screen at work.
  - A fix's tests sometimes depend on shared test fakes (`tests/fakes/gh.rs`, `with_recorded_signal`) that the same pull request changes, so the overlay must bring those along.
  - Good first candidates: #1374, #1263, #1176, #1621 and #1513.
  - Confirmed by a script over local `git diff`s and `gh` metadata, kept in the session's scratchpad.

- **What the hand-run process comparison suggests bringing into thirdshift**, generic lessons only (the details stay private), confirmed by reading its notes repository and review site. Most important first:
  1. **Whole-process Arms.** A Trial runs end to end, from the Ticket to a commit, and keeps its Command record. Each Arm's skills and process files are pinned by content hash and checked before every Trial.
  2. **Cost and time per session and role.** Count every session a process starts, including sub-agents and command-line children. Price tokens from a frozen price table, attribute the cost to roles, and leave usage-limit stalls out of the time. A Trial with incomplete accounting is a Void Trial. Report paired median cost and time ratios.
  3. **Execution evidence.** Run each Arm's new tests against the unchanged base, and against the other Arm's code (cross-tests). Give the results to every Grader as evidence. A cross-test failure is a disagreement to look into, not an automatic defect.
  4. **Blind pairwise human review.** One page per task pair shows the task, a numbered acceptance checklist, the two diffs side by side (code before tests) and the execution evidence. It shows no Arm identity and no model Scores. The reviewer first makes a sealed, unaided merge call per side, then optionally fills the same rubric the model Graders use. Records are append-only and per reviewer.
  5. **Pairwise model grading in both orders, by a panel from several families.** The rubric asks per criterion, with evidence, and puts the reason before the winner. A result that flips with the order counts as a tie. Report each Grader's agreement with the human and its position-bias rate.
  6. **Outcome categories.** Each Trial is delivered, stopped-to-ask or failed. A process's own failures count as losses and outside failures are Void. Seeded repeat Trials measure run-to-run variation.
  7. **A leak check on Trial traces.** Search the transcripts for the real fix's identifiers, and keep a list of dismissed false positives.
  8. **A paired trace view.** Show the two Arms' sessions side by side, with tool calls by kind and time to first edit.
- That comparison's tooling was about 6,200 lines of Python, bash and PHP outside thirdshift. thirdshift already covers blind packets, bwrap Graders, the Bar and Verdict, and token reconciliation. It has only part of, or none of: process Arms, per-role pricing, cross-tests, both-order panel grading, the human review page and the leak check.
- The comparison's write-up proposes three Ticket-run edits to test next: a deterministic check that the new tests fail on the base, one bounded cross-family review pass, and test-proportionality and stop-the-loop rules in the review prompts.

- **Process Arms, outside tools and blind human review**, written up in [`docs/research/eval-process-arms-and-human-review.md`](../research/eval-process-arms-and-human-review.md) from primary sources:
  - Langfuse takes outside experiment items but expects one trace per item. It has no diff view, no blind or pairwise human review, and no significance test.
  - No tool checked hides the Arm from a human reviewer or randomises the sides.
  - Every runner treats the agent scaffold as a black box: a repository at its base commit and an instruction go in, and a patch, a trajectory and usage per model and sub-agent come out.
  - thirdshift's `/scores` form already holds back the model Grader's Scores until the human's are saved, but it shows which Harness, Model and Effort made the work. Nothing grades two Trials as a pair.
- #1531 (split the code review out of the implement Step) and #429 (multi-family review) are open, and #1533's Built-in process files are merged (`processes/ticket-run.toml`), confirmed by `gh` and the repository.
- **The model Grader panel**, in [`docs/research/eval-grader-panel.md`](../research/eval-grader-panel.md), from primary sources:
  - OpenCode runs both API-served Models. Three thirdshift changes are needed: allowed proxy hosts, an API key per provider, and the panel.
  - Same-family Graders are only slightly more correlated in their errors than cross-family ones, but family preference is real and points one way.
  - Accuracy stops improving at about five judges.
  - At API rates, an 11-Model panel over 60 pairs in both orders costs about $900. That rests on an assumed session size, to be checked on one pilot pair.

## Waiting on research

- Nothing.
