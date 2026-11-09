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

## Round 2 (open)

**Q2.1 - What is an Arm, and how far does a Trial run? (Q1.1, asked again)** Your Q1.1 answer sets the goal as process Experiments, and the next one you want to run is an edit to the Ticket run itself. The candidate edits from the hand-run comparison are a deterministic check that the new tests fail on the base, one bounded review pass by a different model family, and new rules in the review prompts. All of them live *outside* the implement session: in Steps, in review stages, in review prompts. So an implement-session-only suite (Q1.1's old option (a)) can't test them. The options:
- (a) **The implement session alone**, as recommended in Round 1. It's the cheapest, but it can't test the edit you want next.
- (b) **A process Arm.** An Arm is a thirdshift build plus a process file, Built-in or Custom, with its skills and process file pinned by content hash. A Trial runs that process end to end on the Ticket, through its review stages and Repairs, and stops where it would push. The Grader grades the final commit.
- (c) (b), plus an **external-runner Arm**: a pinned command that takes the repository at its base commit and the Ticket, and returns a branch, its session traces and its usage. thirdshift treats the command as a black box, as Harbor's agent interface does (research note §2). This is the only way to hold a process whose orchestrator is itself an agent choosing what to run next (#1468's triage note).

*Recommended:* (b) now, with (c) recorded as the later step. Comparing the Ticket run with an edited Ticket run needs only process Arms, and #1533's Built-in process files plus #1531 (which splits the code review out of the implement Step) give a Custom process room to change the reviews. An external runner matters only once the next Experiment pits thirdshift against an outside process, so it can wait for that need. Each Trial then costs several sessions instead of one, so the cost estimate shown before Trials (#1434 Q1.6) has to come per Arm.

*Answer:* 

**Q2.2 - Build the review and grading tooling into thirdshift, or adopt Langfuse or a similar tool?** The research note (§1) checked Langfuse, Braintrust, LangSmith, Phoenix, promptfoo, Inspect/Scout and Harbor against a process comparison:
- Langfuse "expects exactly one trace per experiment item", while its Claude Code and Codex integrations write one trace per turn. A process Trial can run 4–72 sessions.
- None of the tools shows code diffs side by side, hides which Arm made an output from a *human* reviewer, or randomises which side each Arm appears on. LangSmith's pairwise queue still puts "Run A on the left".
- None computes a Verdict.

The options:
- (a) **Build it in.** thirdshift keeps the runner, the Bar and the Verdict, and adds a blind pairwise review page to the Console. That page records a Day-shift Grader's call as a paired Score. thirdshift exports Trial traces over its existing OTLP output to any outside viewer, opened only *after* the human calls are recorded, so it can't break the blinding.
- (b) **Adopt Langfuse**, or a similar tool, as the experiment store and review UI. The blind page and the Verdict would still have to be built, and there would be two places holding the results.
- (c) **Use Harbor as the runner**, with thirdshift grading. This means a Python dependency and a Docker image per repository, and subscription login works only for Claude and Codex.

*Recommended:* (a). Every outside tool would still need the blind page and the Verdict built. thirdshift already has a Day-shift Grader on `/scores`, a shuffle for its model Graders and a packet stripper to reuse. Harbor's interface is the shape to copy for Q2.1's external runner later, not a dependency to take.

*Answer:* 

**Q2.3 - Does the suite include the blind pairwise human review page?** This depends on Q2.2 being (a). If not, it moves to a later round. The hand-run comparison's page worked like this:
- one page per task pair, showing the Ticket, a numbered acceptance checklist, the two diffs side by side (code before tests) and the execution evidence;
- no Arm identity and no model Scores;
- the reviewer first makes a sealed, unaided merge call per side, then optionally fills the model Graders' rubric.

Arena practice adds a choice of A, B, tie or both bad, with the Arms revealed only after the vote. Copilot Arena also found that people picked the first option 86% of the time even with randomised order, so each pair needs its own random sides. The options:
- (a) **In this suite's Spec**, as a Day-shift Grader whose call is reported *beside* the Verdict and doesn't count toward it at first.
- (b) **In this suite's Spec**, with the human call counting in the Verdict.
- (c) **A separate issue**, after the code-graded suite works.

*Recommended:* (c). The code-graded suite (Q1.5) is the minimum that works on its own, and it is what #1526's Experiment needs. The page is wanted, but it's a Console feature of its own. File it now as an issue blocked by this suite, with the hand-run page and the arena rules as its starting design. Once some Experiments have shown how the human calls compare with the code Graders, decide then whether the human call counts.

*Answer:* 

**Q2.4 - Per-repository toolchains: bwrap on the host, or containers?** Q1.2 says the suite must serve other repositories, and in #1434, 4 of 6 cross-repository candidates were discriminating. Today a Trial runs in bwrap with the host's Rust, Cargo and nvm mounted read-only, and nothing else is provisioned. The new host has room for images (#1484's comment). The options:
- (a) **bwrap on the host**, with each repository's toolchain installed on the host and declared in the suite. Containers come only when a repository needs services or a pinned system image (#698 Q6.7's trigger).
- (b) **A container image per repository**, built at task preparation, with every Trial and code Grader run inside it. This also answers the "a commit stops building on the host" problem.
- (c) (a) for thirdshift's own tasks now, and (b) designed in from the start for the first task from another repository.

*Recommended:* (c). thirdshift's own tasks run in bwrap today, so the first Experiment needn't wait for containers. But the outward goal means the task format should name its environment, `host` or an image, from day one, so another repository's tasks don't need a format change later.

*Answer:* 

**Q2.5 - Check Trial traces for the real fix?** Every real task's fix is public (Q1.2), and #1468's triage note says implement tasks need a leak check, since the hand-run comparison searched every Trial's traces for the shipped fix. The time-safe clone stops the repository from leaking it, but a session could still fetch the pull request or recall it from training data. The options:
- (a) **Yes.** After each Trial, search its transcripts for the fix's identifiers: the pull-request and issue numbers, the merge commit, the branch and the title. Hits go to the Day shift, with an auditable list of dismissed false positives. A confirmed leak voids the Trial.
- (b) **No.** Rely on the time-safe clone and the Trial sandbox's network allowlist.

*Recommended:* (a). It's cheap and mechanical. Note that the Trial sandbox already reaches only the provider, so a leak would most likely come from the Model's memory, which a search for identifiers catches only partly. The free time-split probe (scaling note §3.3) covers the rest.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- thirdshift keeps its own runner, its blind Graders, the Bar and the Verdict. Disk was never why a runner swap was declined (build-vs-buy §8.1).
- The real-task spine is Q6.7's construction: prepared once at the base commit with network, fix tests checked to fail at the base and pass at the fix, a time-safe clone, and a read-only copy for each Trial. Code Graders run with no network.
- Admission is #1477's rule, already implemented: Difficulty measured over at least 6 Trials, discriminating between 0.15 and 0.95, and Advisory below `min_discriminating_tasks` (20 by default).
- Disk and Docker are no longer constraints (the 2026-10-27 comment on #1484).
- #1434's code-review repair, the prerequisite for this grill, is closed.
- **Q1.1 is still open.** The answer widens the goal beyond the implement session. The goal is to run process-comparison Experiments through thirdshift: the Built-in Ticket run against another process, and next an edit to the Ticket run, on thirdshift's own tooling rather than bespoke tooling outside it. It also asks whether a blind human pairwise review page belongs in thirdshift, or whether thirdshift should adopt a tool such as Langfuse instead. Round 2 asks it again with research in hand.
- thirdshift's purpose is outward (Q1.2). The first tasks come from thirdshift itself, but nothing in the suite may assume thirdshift: the task format, the toolchain setup and the Arm's configuration must work for any repository the factory serves, and different repositories may run different processes. Per-repository toolchains (bwrap on the host or containers) therefore come up in Round 2.
- Real tasks include features as well as bug fixes, with a screen that rejects a task whose tests depend on names the Ticket doesn't state (Q1.3).
- Real tasks only for now. Every task records its provenance, `real` for now, so injected tasks can come later without a format change (Q1.4).
- The code Grader grades the Trial's own commit. It lays the fix's separate test files over that commit, along with any shared test fakes the fix changed, and runs the fail-to-pass tests and the pass-to-pass tests in the same test targets, offline and under a time cap. A task whose tests are inline in a source file isn't admitted (Q1.5).
- The suite's dependencies are the implement, tdd, codebase-design and code-review Factory skills, plus the other suites' prompt, Harness, Model and Effort entries (Q1.6).
- Three Trials per task (Q1.7).
- A new, bigger host runs the Experiments. The Lima VM builds and screens the first tasks until that host exists (Q1.8).
- Supply is tight. #1434's candidates came out about a third discriminating (7 of 20, #1516), and about 60 usable real candidates at that rate gives roughly 20. That's just enough, so another repository's tasks would also help reach 20. In #1434, 4 of the 6 cross-repository candidates turned out discriminating.

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

## Waiting on research

- Nothing.
