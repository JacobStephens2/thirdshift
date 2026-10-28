# Grilling: #1533, the Spec, Architect and Security runs as Built-in process files

Issue: https://github.com/JacobStephens2/thirdshift/issues/1533
Research: #1456 (Spec: the Ticket run as a Built-in process file) and its grill `docs/grill-with-docs/1456-built-in-ticket-run.md`, #877, ADR-0024, ADR-0001, `processes/README.md`, related #1468 (Experiments with Custom processes as Arms), #118 (prompts and skills in configuration), #1484 (an implement Eval suite).

## Round 1 (answered)

**Q1.1 - Which issue this lives under.** No open issue covers it. #1456 Q1.4 agreed that the Spec run, Architect run, Security run and Pickup run would each be filed as their own issue after the Ticket run moved, in that order. #1456 closed on 2026-10-28 with its five Tickets done, and none of those four issues was filed. #1468 is the nearest open issue, but it is about Experiment Arms, not about the run types' files. The options:
- (a) File one new Spec issue, "the other run types as Built-in process files", that carries out #1456 Q1.4, links #1456, #1468 and #118, and owns this grill.
- (b) File the four issues #1456 Q1.4 named, one per run type, and grill the first (the Spec run) here.
- (c) Reopen #1456 and widen it.

*Recommended:* (a). The four run types share most of the open decisions (what stays in front of the file, how a Step starts child runs, what a Pass's gates become), so one grill and one Spec settle them once. Tickets can still be sliced per run type. #1456 is done and released; reopening it muddies what shipped.

*Answer:* agree

**Q1.2 - Which run types get a file.** Five Commands besides the Ticket run are sequenced in Rust: the Spec run, Architect run, Security run, Pickup run, plus the Scoring run and the Experiment. The options:
- (a) The four #1456 Q1.4 named: Spec, Architect, Security, Pickup. Scoring runs and Experiments stay as they are, because they measure Processes rather than do factory work.
- (b) All six.
- (c) Only the three that run sessions of their own (Spec, Architect, Security); Pickup's own work is selection and reservation, with nothing in it a user would compose, so it keeps running `ticket-run` or a Spec run as it does today.

*Recommended:* (a), with Pickup last and allowed to turn out as (c) once research shows how little of it is a sequence. An Experiment that runs a Process is #1468's subject, not a Process itself.

*Answer:* agree

**Q1.3 - Behaviour unchanged.** #1456 Q1.2 held the Ticket run's behaviour exactly as it was: names, sessions, progress lines, Command log, Endings, Run notifications, Console view and Recovery. Should the same rule hold for every run type here, so that each move is a pure refactor visible only through `process list` and `process show`, and behaviour changes the files make easy get their own issues?

*Recommended:* Yes. Unattended Passes run every few minutes on several machines; a move that also changes behaviour is hard to review and hard to roll back.

*Answer:* agree

**Q1.4 - Gates and dispatch stay in front of the file.** #1456 Q2.2 kept in Rust the choices that decide which Process runs and how it starts: Spec or Ticket, the Claim, Child asks and Limits. The Passes have more of those: the skip gates (another Pass running, an open Architect plan, a waiting Architect idea, a Ready issue, an unchanged Base branch, the Claim limit), and Pickup's issue selection and cross-machine reservation. Should all of those stay in Rust, decided before the first Step, so that a file says only what work runs once the Pass has decided to run?

*Recommended:* Yes. It extends #1456 Q2.2 and ADR-0024's rule that the safety rules stay out of the file. A copied file that drops a gate could make two Passes fight over one repository, or start work ahead of issues a human shaped. A Custom process already gets the one gate every Pass shares (no other Pass running).

*Answer:* agree

**Q1.5 - How the evals motive fits.** You want the files partly so that Processes can be modified and evaluated. Experiments today compare Arms that differ by build or by Harness, Model and Effort, judged on code-review and spec-review sessions; #1468 asks for Arms that differ by skill set or Custom process, and #1484 for an implement Eval suite. The options:
- (a) Keep evals out of this Spec. This Spec makes each run type a file a user can copy and change; #1468 then lets an Arm name a Process, and benefits from every file this Spec adds.
- (b) Fold #1468 in, so that this Spec ends with an Experiment that compares, say, `ticket-run` against a copy of it.
- (c) Add only the small piece evals need from the files, such as a Process's name and file hash recorded in its Session traces and Command record, so that Scores can later be grouped by Process.

*Recommended:* (a), plus (c) if research shows the Process a Command ran isn't recorded yet. #1468 still has open questions of its own (what an Arm is, whether a whole Ticket run is affordable as a Trial, how many tasks a Verdict needs per #1434), and they shouldn't hold up the files.

*Answer:* agree, I call the processes of the third shift factory, the default built-in ones, the Pocock workflow of Matt Pocock. And I want to set up third shifts. I want to get it to a point where it can be able to test this workflow, this process against that of a contractor I'm working with who is proposing a different process as more having better verification and just and better code quality.

And so but I want I want to be able to have a way to evaluate or test the difference between these two processes. So that's some of the direction I move in here. And I think that having process files for the current run types of third shift would make it easy to compare at a high level the differences the difference in the process of the current run types and then with this contractor's processes eventually. those the difference in that contractor's process involves different skills and a different number of code reviews and different code review skills and different it's very heavy on multi-family or cross-family review so I want the process system to be able to and the custom skill system and model harness effort level settings to be able to get to a point that can handle testing to processes for running a software factory like this and then even do evals on the difference well yeah not all of that has to be done in this spec or maybe we can make others another spec or another issue to grill or triage later to expand.

But that's some that's the direction I'm moving with this and wanting to move. In addition addition to just having the easily intelligible high-level files describing the processes of the different runs 

*Checked:* In a Ticket run the code review isn't a Step of its own. The `implement` session's prompt tells the agent to "Review with the `thirdshift-code-review` skill using {base} as the fixed point" and address the findings, inside the same session (`src/prompt.rs:250-265`, `fresh` and `continuation`). So copying `ticket-run` today can't change the number of reviews, their skills or their model family without dropping `implement` for plain skill Steps, and with it bug diagnosis, Continuation and the Base fix advice. The separate `code-review` session kind exists only for Eval Trials (`prompt::code_review`, `src/trial.rs:208`). #429 (Multi-family review) is open and `needs-triage`.

## Round 2 (answered)

*Pre-answer:* I think I also want to add to the repo sample processes to demonstrate what making a custom workflow would look like - maybe these sample processes can be based on the contractor workflow - which can be found in the parent-engagement-planning repo I have copied locally / is accessible on my gh account in the ETA org - thus giving an example to others of an alternate process but also setting up that experiment.

**Q2.1 - Pickup gets no file.** Q1.2 let Pickup turn out as "no file" if it has little sequence. Research found it has none: a Pass lock, a CI-red Ticket, a Sweep, the Claim-limit and Ready-issue gates, a Reservation, then one child run. Under Q1.4 all of that stays in Rust, which would leave a file with a single Step. Should the Pickup run get no Built-in process file, with the README saying it dispatches to `ticket-run` or the Spec run's file?

*Recommended:* Yes. A one-Step file explains nothing a sentence in the README can't, and gives a copier nothing to change.

*Answer:* agree

**Q2.2 - The Spec run's file.** The Spec run's sequence is the Claim and Spec-branch worktree (dispatch, so in front of the file per Q1.4), then the Ticket loop, the Spec review, an optional Security review, Delivery of the Spec PR and an optional Self-merge. The Ticket loop is dynamic: how many child runs, which ones and how many at once all come from the sub-issue graph at run time, and the format has no loops (Research). The options:
- (a) A catalogue Step `land-tickets` owns the whole loop, including the draft Spec PR and its checklist, on a new worktree kind `spec-branch`. The file reads `land-tickets` → `spec-review` → `security-review` → `delivery` → `self-merge`, and `spec-review` is a catalogue Step because it has a report contract Delivery reads. `security-review`, `delivery` and `self-merge` learn to work on a `spec-branch` worktree.
- (b) Add loops or fan-out to the format, so the file states "for each Startable Ticket, run `ticket-run`".
- (c) One catalogue Step `spec-run` that does everything, so the file is a single line.

*Recommended:* (a). It keeps the graph rules, trust checks and Failed spec run accounting inside a Step (ADR-0024), keeps the format free of control flow, and still shows the reader the shape: land the Tickets, review the whole, deliver. (c) explains nothing.

*Answer:* agree - 

**Q2.3 - The file names the Process its children run.** This is what makes your comparison possible across a Spec. Today a Spec run's Tickets, an Architect run's plan and a Security run's fix always run `ticket-run`. If `land-tickets` and `run-published-issue` took a `process` setting, defaulting to `ticket-run`, a copy of the Spec run's file could run every Ticket through the contractor's Ticket process. The named Process would have to declare an Issue-branch worktree, and `process check` would refuse one that doesn't, or that names a Process that doesn't exist. Should the setting be added in this Spec?

*Recommended:* Yes. It is small, the Built-in files keep the default so nothing changes (Q1.3), and without it a contractor's Ticket process can only be compared on standalone Tickets.

*Answer:* agree - I was considering whether to use the custom skills thing here for this, but maybe not? Or to use the process skills syntax?

*Checked:* They are one mechanism used together, not alternatives. A Step's `skill = "<name>"` is looked up as `thirdshift skill` looks it up: a Factory skill whose directory is exactly `<name>`, then the repository's `.agents/skills` or `.claude/skills`, then `~/.thirdshift/skills/<name>`, then `thirdshift-<name>` (`docs/guide.md:1115-1125`). So the contractor's skills go in `~/.thirdshift/skills/` as home skills, and the contractor's Ticket process names them with `skill =` on its Steps. A home skill can't replace a Factory skill such as `thirdshift-code-review` for every Process, because an exact Factory name wins first. That's good for comparison: what changed is always in a file, and the file hash from Q2.6 records it. What neither can do is change the skills a catalogue Step such as `implement` uses (Q3.3).

**Q2.4 - The Architect run's file.** Its sequence after the gates is the Architecture review session in a throwaway worktree, a conclusion that labels a plan or an idea, and, unless the command asked for the plan only, a child run on the plan. The options:
- (a) A catalogue Step `architecture-review` owns the session, the conclusion and its labels, and ends the Process successfully when there is no Strong plan or the command asked for the plan only. The file reads `architecture-review` → `run-published-issue`. That makes `architecture-review` the second Step, after `implement`, that can end a Process early, again as its own rule, not a file setting.
- (b) The review is a plain skill Step, and the conclusion and labelling move into `run-published-issue`, which skips itself when no plan was published.

*Recommended:* (a). The plan-or-idea rule is the Architect run's safety rule: only a Strong recommendation is implemented unattended. It belongs with the session that makes it, not in a generic dispatch Step that other files also use.

*Answer:* agree, 

**Q2.5 - The Security run's file.** Its sequence after the gates is the Security audit, private records, one reproduction session per finding, then, when allowed, a fix-publishing session and child run per reproduced finding, then, when allowed, the recheck-and-close. One gate also changes which Steps run: a recorded finding awaiting a fix goes straight to fixing. The options:
- (a) Four catalogue Steps, `security-audit` → `reproduce-findings` → `fix-findings` → `close-findings`, each deciding for itself whether it runs from the flags, the User config and the records, as `security-review` already does. The jump-to-fix gate becomes `security-audit` and `reproduce-findings` skipping themselves. The privacy rules of ADR-0015 stay inside them.
- (b) One catalogue Step `security-run`.
- (c) Fewer, coarser Steps: `audit-and-reproduce` → `fix-and-close`.

*Recommended:* (a). Each Step matches a stage a reader can name, and a copier can drop `fix-findings` and `close-findings` to make an audit-only Process. Every Security finding stays private until it is fixed, whatever the file says.

*Answer:* agree

**Q2.6 - What a Command records about its Process (Q1.5 (c)).** Research found that only a Custom process records its name, and nothing records Built-in or Custom, or which version of the file ran. To group Scores and Run metrics by Process later, every Command and every child run would record its Process's name, whether it was Built-in or Custom, and a hash of the file's bytes, in its Command record and as attributes on its Session traces. Should it record those three, and should the Console and `thirdshift status` keep showing run types as they do today?

*Recommended:* Yes to both. The hash tells apart two edits of the same Custom process, which a name alone can't, and keeping the display unchanged holds Q1.3.

*Answer:* agree

**Q2.7 - Splitting the code review out of `implement` is a follow-up, filed now.** Your contractor's process has a different number of code reviews, different review skills and cross-family review. A copy of `ticket-run` can't express that, because the review happens inside the `implement` session (Q1.5 *Checked*). Splitting it out, as an `implement` that stops before review followed by one or more review Steps, changes the Ticket run's sessions, so under Q1.3 it isn't this Spec. Should I file it now as its own `needs-triage` issue, linked to #429 and #1468, and add a comment to #1468 describing your Pocock-workflow-against-contractor comparison as its first concrete case?

*Recommended:* Yes. It is the next blocker on the road you described, and writing it down now keeps it out of this Spec without losing it.

*Answer:* agree, though consider my pre-answer as well here.

*Checked:* Read "pre-answer" as your Q1.5 answer, and wrote the comparison you described there into both. Filed #1531, "Split the code review out of the implement Step, so a Process can change its reviews" (`needs-triage`, `enhancement`), linked to #429, #1468, #1456 and #118. Its open questions include the `skill` override for catalogue Steps raised under Q2.3. Commented on #1468 with the Built-in processes against the contractor's process as its first concrete case: https://github.com/JacobStephens2/thirdshift/issues/1468#issuecomment-6475117327

**Q2.8 - What to call the default set.** You call thirdshift's Built-in processes, with the Factory skills, "the Pocock workflow". The glossary has no term for that set, and it lists "workflow" under _Avoid_ for Process. A comparison needs a name for each side. The options:
- (a) No new term here. Say "the Built-in processes", and leave the name for one side of a comparison to #1468's grill, where Arms are defined.
- (b) Add a glossary term now, such as "Factory default": the Built-in processes run with the Factory skills, as shipped.
- (c) Use "Pocock workflow" as the glossary term.

*Recommended:* (a). This Spec never needs the set as a whole, only its single files. A name chosen now would have to fit Arms that aren't designed yet. (c) also reads as Matt Pocock's own product.

*Answer:* agree

**Q2.9 - When to file the Spec issue (Q1.1).** The options:
- (a) At the end of the grill, as the full Spec, labelled `needs-triage` per `docs/agents/triage-labels.md`, then rename this file to its number.
- (b) Now, as a short `needs-triage` stub pointing at this file, filled in at the end.

*Recommended:* (a). That is how #1456 went, and a stub invites a Pickup or a triager to act before the decisions exist. A `needs-triage` stub is safe from Pickup, but it says nothing yet.

*Answer:* agree - we'll use the to-spec the to-tickets skills for this

## Round 3 (answered)

**Q3.1 - File names.** The Ticket run's file is `ticket-run.toml`, named after its run type. Should the new files be `spec-run.toml`, `architect-run.toml` and `security-run.toml`, so that `thirdshift process show spec-run` and the rest read as the run types the glossary already names?

*Recommended:* Yes. Each name is then reserved against Custom processes as `ticket-run` is.

*Answer:* agree

**Q3.2 - Running a copy of the Spec run.** Once `spec-run` is a file, someone copies it to `~/.thirdshift/processes/my-spec-run.toml` and runs `thirdshift process my-spec-run issue <spec url>`. Under Q1.4 and #1456 Q2.2, Spec-or-Ticket is decided before the file. For a Custom process, the file's worktree kind decides instead. So should a file that declares a `spec-branch` worktree:
- be refused when the issue has no sub-issues, and a file with an `issue-branch` worktree be refused when it has some, both before any session;
- be refused if it declares both kinds;
- make a Claim before its worktree, as a Spec run does?

*Recommended:* Yes to all three. They mirror what dispatch does for the Built-in, so a copy can't run a Spec as a Ticket or the reverse, and the Claim keeps a Pickup run off the Spec while the copy runs.

*Answer:* agree

**Q3.3 - Catalogue Steps keep their own skills in this Spec.** `implement`, the new `spec-review`, `architecture-review` and the Security Steps each name their Factory skills in their Session prompts. A copy can reorder, drop or add Steps around them, and give each its own Harness, Model and Effort, but can't swap the skill inside one. Should a `skill` override on catalogue Steps stay out of this Spec, left to #1531 and #118?

*Recommended:* Yes. It changes what those Steps promise (a report contract, bug-diagnosis lines, the plan-or-idea conclusion), and each Step needs its own rule for which skills may stand in. #1531 already asks.

*Answer:* agree

**Q3.4 - Order of the moves.** #1456 Q1.4 put them Spec, Architect, Security. The Architect run is now the smallest move: one new Step, and it reuses `run-published-issue`. It would also prove the second early-ending Step (Q2.4) and the `process` setting (Q2.3) before the bigger moves rely on them. The options:
- (a) Architect, then Spec, then Security.
- (b) Spec, Architect, Security, as #1456 said.
- (c) No order: each move depends only on its own Steps, so they run in parallel as the graph allows.

*Recommended:* (a) as the order of blocking links only where one move needs another's work, otherwise (c). In practice the Spec run's move waits only on the `process` setting, and the Security run's on nothing shared.

*Answer:* agree

**Q3.5 - Ticket slicing.** A proposal, one Spec, with each move Ticket also updating `processes/README.md`, the guide and the hand-written site pages for its run type:
1. Every Command and child run records its Process's name, Built-in or Custom, and the file's hash (Q2.6).
2. A `process` setting on `run-published-issue` (Q2.3).
3. An `architecture-review` catalogue Step, with its early successful end (Q2.4).
4. The Architect run runs from `architect-run.toml`. Blocked by 3.
5. A `spec-branch` worktree kind and a `land-tickets` catalogue Step with its `process` setting, plus the refusals and Claim of Q3.2. Blocked by 2.
6. A `spec-review` catalogue Step, and `security-review`, `delivery` and `self-merge` on a `spec-branch` worktree.
7. The Spec run runs from `spec-run.toml`. Blocked by 5 and 6.
8. `security-audit` and `reproduce-findings` catalogue Steps.
9. `fix-findings` and `close-findings` catalogue Steps.
10. The Security run runs from `security-run.toml`; `processes/README.md` says Pickup has no file (Q2.1). Blocked by 8 and 9.

Should the Spec be sliced like that?

*Recommended:* Yes. Each Step Ticket is testable alone through `thirdshift process`, as #1456's were, and each move Ticket is a refactor proved by the existing suites. 1 goes first only because it is independent and cheap.

*Answer:* agree

**Q3.6 - Testing and the Spec PR, as for #1456.** Should this Spec use #1456's Testing Decisions and Q3.3? That means:
- tests through the CLI with real Git, the fake `gh` and fake Harnesses, and no new seam;
- each new Step tested for success, refusals and failure;
- each move proved by its run type's existing integration suites passing with at most mechanical changes, plus a test that its compiled-in file passes `process check`;
- unit tests split, not weakened;
- the Spec PR left ready for review and never Self-merged, while the Day shift runs live checks from a build of the Spec branch on a test repository: an Architect run with a plan and one with only an idea, a Spec run with two Tickets, and a Security run that reproduces a finding.

*Recommended:* Yes. Passes run unattended on every machine, so the live checks are what stand between a refactor and a bad night.

*Answer:* agree

**Q3.7 - No new ADR.** ADR-0024 already covers composing Steps from a catalogue and keeping safety rules inside them. What this Spec adds (catalogue Steps that end a Process early, gates in front of the file, a Step naming the Process its children run) follows from it. Should there be no new ADR, with ADR-0024's last Consequence ("waits until the catalogue has Steps for its parts") left as the record that this was the plan?

*Recommended:* Yes. Nothing here is a fresh trade-off a future reader would wonder about. The `process` setting comes closest, and Q2.3's reason fits in the Spec.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- One new Spec issue owns this work and links #1456, #1468 and #118 (Q1.1).
- The Spec, Architect and Security runs get Built-in process files; Scoring runs and Experiments don't (Q1.2). Pickup is decided in Q2.1.
- Every move is a pure refactor: names, sessions, progress lines, Command log, Endings, Run notifications, Console view and Recovery unchanged. Behaviour changes the files make easy get their own issues (Q1.3).
- Pass locks, skip gates, Claims, the Claim limit, Pickup's selection and Reservation, and Spec-or-Ticket dispatch stay in Rust, decided before the first Step (Q1.4).
- Experiments that compare Processes stay in #1468; this Spec adds only what lets Scores be grouped by Process later (Q1.5), shaped in Q2.6.
- The direction behind the work, recorded for the Spec's Problem Statement: comparing thirdshift's Built-in processes and Factory skills against a contractor's process, which uses different skills, more code reviews and cross-family review, first by reading the files side by side, later by Experiments (Q1.5).
- Pickup gets no Built-in process file; `processes/README.md` says it dispatches to `ticket-run` or the Spec run's file (Q2.1).
- The Spec run's file reads `land-tickets` → `spec-review` → `security-review` → `delivery` → `self-merge`, on a new `spec-branch` worktree kind. `land-tickets` owns the Ticket graph, trust checks, the draft Spec PR, its checklist and Failed spec run accounting; the format gains no loops (Q2.2).
- `land-tickets` and `run-published-issue` take a `process` setting, defaulting to `ticket-run`. `process check` refuses a named Process that doesn't exist or has no Issue-branch worktree (Q2.3).
- Contractor or other replacement skills reach a Process as home skills named by `skill =` on its Steps, not by shadowing Factory skills (Q2.3 *Checked*).
- The Architect run's file reads `architecture-review` → `run-published-issue`. `architecture-review` owns the session, the plan-or-idea conclusion and its labels, and ends the Process successfully with no Strong plan or on plan-only (Q2.4).
- The Security run's file reads `security-audit` → `reproduce-findings` → `fix-findings` → `close-findings`. Each Step decides for itself whether it runs, the jump-to-fix gate becomes the first two skipping themselves, and ADR-0015's privacy rules stay inside them (Q2.5).
- Early ends stay a property of particular catalogue Steps (`implement`, `architecture-review`, the Security Steps skipping), never a file setting.
- Every Command and child run records its Process's name, Built-in or Custom, and a hash of the file's bytes, in its Command record and on its Session traces. `status` and the Console keep showing run types (Q2.6).
- Splitting the code review out of `implement` is #1531, not this Spec (Q2.7).
- No glossary term for the default set; "the Built-in processes" here, naming left to #1468 (Q2.8).
- The Spec is filed at the end of the grill with `to-spec`, labelled `needs-triage`, and its Tickets with `to-tickets`, per `docs/agents/triage-labels.md`; this file is then renamed to its number (Q2.9).
- As with `ticket-run`, the new Built-in files set no Harness, Model or Effort on any Step, so each keeps today's per-Command choice, and the Security review keeps `security.harness`.
- Glossary entries "Built-in process", "Step", "Pickup run" and the run types' entries are updated when their Tickets land, only as far as ownership moves, with no implementation detail.
- The files are `spec-run.toml`, `architect-run.toml` and `security-run.toml`, each name reserved against Custom processes (Q3.1).
- A Custom process with a `spec-branch` worktree is refused on an issue with no sub-issues, one with an `issue-branch` worktree is refused on an issue with some, a file can't declare both, and a `spec-branch` file makes a Claim before its worktree, all before any session (Q3.2).
- No `skill` override on catalogue Steps in this Spec; left to #1531 and #118 (Q3.3).
- Blocking links only where one Ticket needs another's work; in priority, the Architect run moves first, then the Spec run, then the Security run (Q3.4).
- The ten Tickets of Q3.5, each move Ticket updating `processes/README.md`, the guide and the hand-written site pages for its run type (Q3.5).
- #1456's Testing Decisions apply. The Spec PR is left ready for review and never Self-merged until the Day shift's live checks pass on a test repository: an Architect run with a plan and one with only an idea, a Spec run with two Tickets, and a Security run that reproduces a finding (Q3.6).
- No new ADR; ADR-0024 covers it (Q3.7).
- The frontier is empty after Round 3. The Spec is #1533; its Tickets follow with `to-tickets`.
- #1456's decisions stand: Built-in process files live in `processes/`, compiled in (ADR-0001), same format and validation as Custom processes, readable with `process show`, copied by hand, and a Custom process still can't take a Built-in name.
- A Command that carries out a Built-in process keeps its run type's name (GLOSSARY "Built-in process").
- ADR-0024 holds: the catalogue grows when a file needs a Step it lacks; safety rules stay inside catalogue Steps.

## Research results (2026-10-28)

- No open or closed issue files the Spec, Architect, Security or Pickup run as a Built-in process; searched `gh issue list --state all` for "Built-in process", "processes", "process file" and "spec run file".
- `processes/` holds only `README.md` and `ticket-run.toml` (`implement` → `security-review` → `delivery` → `self-merge`), confirmed by source.
- Pickup runs and Spec runs already start their Ticket runs through `ticket-run` (#1463, #1464, closed).
- The catalogue has five Steps: `implement`, `security-review`, `delivery`, `self-merge`, `run-published-issue` (`src/processes.rs:3026-3049`). `run-published-issue` starts a Ticket or Spec run on the issue the session Step just before it published, after trust and created-since checks (`docs/guide.md:1130-1146`). The Built-in adapter accepts only the first four, strictly in order, and refuses per-Step Harness, Model, Effort, `optional`, `parallel` and limits (`src/processes/ticket.rs:19-56`). Confirmed by source.
- The Process format has `version`, `description`, `[worktrees.<name>] kind = "throwaway" | "issue-branch"` (at most one Issue-branch), `[[steps]]` with `name`, one of `skill`/`command`/`catalogue`, `worktree`, `parallel`, `optional`, `harness`, `model`, `effort`, and `[limits]` with `repair_cap`, `forbid_self_merge`, `protected_paths`. Parallel groups exist; there are no gates, loops, dynamic fan-out or file-level early ends (`src/processes.rs:2578-2600, 2867-2890`). Confirmed by source.
- **Spec run** (`src/run.rs:270-360`, `src/spec_run.rs:86-215`): Claim and one Spec-branch worktree, push the Spec branch, then a loop starting child Ticket runs over the sub-issue graph, up to `parallel` at once, re-reading the graph after each; the Spec PR opens as a draft on the first landing; then a `spec-review` session and Delivery of the Spec PR with its Self-merge. Would need new Steps for the Ticket loop, the Spec review and a Spec-PR Delivery. Confirmed by source.
- **Architect run** (`src/architect.rs:278-525`, `src/pass.rs:252-378`): Pass lock; gates (open Architect plan, waiting Architect idea, Ready or settling issue); Architecture review session in a throwaway worktree at the Base branch; a conclusion that labels a plan or an idea; then, unless plan-only, a child run on the plan, close to `run-published-issue`. Would need the conclusion/labelling and an end when there is no plan. Confirmed by source.
- **Security run** (`src/security.rs:139-395`, `src/security/*.rs`): Pass lock; gates (Ready issue, failed earlier fix, a recorded finding awaiting a fix jumps straight to fixing, unchanged Base); Security audit session; private records; one reproduction session per finding (a count known only at run time); when allowed, a fix-publishing session and a child run; when allowed, a recheck-and-close. Only the child run resembles an existing Step. Confirmed by source.
- **Pickup run** (`src/pickup.rs:128-285`): Pass lock, a best-effort CI-red Ticket, a Sweep of `in-progress`, Claim-limit and Ready-issue gates, Reservation and refresh, then a child Spec or Ticket run. No sessions of its own. Confirmed by source.
- Experiments target only `code-review` and `spec-review` sessions (`src/evals/format.rs:18-21`, `src/trial.rs:37-40, 206-211`). Confirmed by source.
- The Command record has `process_name`, set only for Custom processes (`src/logs/command_record.rs:107-109, 411-414`). A Built-in `ticket-run` Command records as a plain Ticket run; nothing records Built-in vs Custom or the file's content or hash, in the Command record, Session traces or Experiment records. Confirmed by source; bears on Q1.5 (c).

## Waiting on research

- Nothing. Round 2 needs no more facts.
