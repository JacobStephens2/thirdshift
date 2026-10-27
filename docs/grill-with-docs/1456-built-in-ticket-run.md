# Grilling: #1456, Built-in processes: move the Ticket run into a process file

Issue: https://github.com/JacobStephens2/thirdshift/issues/1456
Research: `docs/grill-with-docs/877-composable-by-file.md` (Q1.1, Q1.2, Q1.5, Q2.3, Q2.4, Q3.4, Q4.6), ADR-0024, ADR-0001

## Round 1 (answered)

**Q1.1 - What "moved" means.** #877 Q1.5 agreed that each run type becomes a file that thirdshift itself runs. A file could also just describe the Ticket run while Rust keeps running it. The options:
- (a) **The file drives the Ticket run.** `processes/ticket-run.toml` is compiled into the binary, and every Ticket run runs through the same Process engine as a Custom process, whether it starts from an Issue URL, a Spec run, an Architect run, a Pickup run or a Security run. The Ticket run's own sequencing in `src/run.rs` goes away.
- (b) **The file is documentation.** It mirrors what `src/run.rs` does, and a test keeps the two in step. Nothing changes at run time.

*Recommended:* (a). With (b) there are two sources of truth, and nothing new becomes possible. #118 (overriding a built-in's prompts) also needs (a).

*Answer:* agree

**Q1.2 - Behaviour stays identical.** Moving the Ticket run is a refactor of the riskiest path in thirdshift. The options:
- (a) **No behaviour change.** Every Ticket run does exactly what it does today: the same sessions, progress lines, Command log, Endings, Run notifications, Console view and Recovery. The existing tests (`tests/` fakes, `src/run.rs` and `src/delivery.rs` unit tests) pass with at most mechanical changes, such as a Command's stored kind. Anything that would change behaviour gets its own issue.
- (b) **Change what the move makes easy,** such as splitting sessions apart, in the same Spec.

*Recommended:* (a). A test suite that passes unchanged is the only practical proof that nothing safety-critical moved. Improvements are much easier once the file exists.

*Answer:* agree

**Q1.3 - How coarse the new catalogue Steps are.** ADR-0024 listed bug reproduction, **Continuation**, **Resume** and the **Repair loop** as missing parts. In the code they aren't separate sessions. A Ticket run is one **Implementation** session (`implement`, with the `thirdshift-implement` skill), which reproduces a bug, fixes it, reviews and opens the PR. Then come the optional **Security review** and **Delivery**. Bug reproduction is the session prompt plus its `Bug reproduced` / `Bug not reproduced` completion lines (`src/prompt.rs:313`, `src/delivery.rs:432`). Continuation is how the branch is selected, Resume is retrying a session, and the Repair loop is already inside `delivery`. The options:
- (a) **Coarse: one new catalogue Step, `implement`.** It owns branch selection (fresh or Continuation), the bug prompt and its completion lines, the Base fix advice, and Resume. The file then reads roughly `implement` → `security-review` → `delivery` → `self-merge`. Resume becomes a property of every session Step, Custom ones included, since it is not specific to Ticket runs.
- (b) **Fine: separate Steps** for reproduction, implementation, review and PR opening, each its own session.

*Recommended:* (a). (b) splits one session into several, which changes behaviour and breaks Q1.2(a). Splitting can come later as a Custom process people try first.

*Answer:* agree

**Q1.4 - Scope of this issue.** The other run types need different Steps: a Spec run coordinates Tickets on a **Spec branch**, an Architect run and a Security run have gates and their own sessions, and a Pickup run reserves issues. The options:
- (a) **The Ticket run only.** #1456 becomes the Spec for it. After it lands, the Spec, Architect, Security and Pickup runs are each filed as their own issue, in that order, each grilled against what the Ticket run taught.
- (b) **All five in one Spec,** with a Ticket per run type.

*Recommended:* (a). This follows #877 Q1.5 ("one at a time, starting with the Ticket run"). A Spec covering five run types would be the largest Spec yet, and its later Tickets would be designed before the format had been tested.

*Answer:* a

**Q1.5 - Can people see the built-in file?** #877 Q1.2 put built-in processes in a visible top-level folder of thirdshift's repository, and #877 Q2.3 forbids a Custom process from taking a built-in's name. The options:
- (a) **List it and print it.** `thirdshift process list` shows `ticket-run` marked as built-in, and a new `thirdshift process show <name>` prints any Process's file. Someone can save that as a Custom process under a new name and change it.
- (b) **List it only.** The file can be read on GitHub.
- (c) **Neither.** Built-ins stay out of `process` commands.

*Recommended:* (a). Copying the Ticket run is the most useful starting point for a Custom process, and printing it shows exactly what the installed version runs.

*Answer:* agree

## Round 2 (answered)

*Pre-answer:* I've wondered even about mermaid as a format vs toml for the Process documents, so it would be easier to visualize, but I suppose something could be rendered from the toml at some point in order to secure toml benefits. Or json even?

**Q2.1 - The Security review as its own Step.** Today the optional **Security review** runs inside Delivery, between the opening session and the push (`src/delivery.rs`). A Custom process's `delivery` Step never runs it (`security_review: false`, `src/processes.rs:1264`). The Ticket run's file needs it somewhere. The options:
- (a) **A `security-review` catalogue Step.** It goes between `implement` and `delivery`, and decides for itself whether to run, from the command's flag and the User config, as today. It keeps today's fallback to the default Harness (#877 Q2.5 refuses fallback only for a Step that names its own Harness). Custom processes can use it too.
- (b) **A setting on `delivery`.** The Ticket run's file sets `security-review = true` on its `delivery` Step.

*Recommended:* (a). The file then shows every session a Ticket run can start, and a Custom process gets the Security review by adding one line. (b) hides a session inside a Step whose job is publishing.

*Answer:* agree

**Q2.2 - What stays in front of the file.** Some of what a Ticket run does today happens before its first session, and none of it is a Step: an issue with sub-issues goes to a **Spec run** instead, a run its command started makes a **Claim** while a child run doesn't, and child runs inherit their **Child asks** and Limits. The options:
- (a) **The Command dispatch keeps all of it.** `thirdshift <issue-url>` and every run that starts Ticket runs still decide Spec or Ticket and Claim or not in Rust, then run the `ticket-run` Process with what they decided. The file starts at `implement`.
- (b) **Move it into the file** as catalogue Steps, such as `claim` and `spec-or-ticket`.

*Recommended:* (a). These choices decide which Process runs at all, so a Step inside one can't make them. A Custom process with an Issue branch already takes its Claim the same way, from outside its Steps (#877 slice 2).

*Answer:* agree

**Q2.3 - Resume for every session Step, in this Spec.** Q1.3(a) makes Resume a property of every session Step. That's a behaviour change for Custom processes: a skill Step that ends its turn with background work killed would get one Resume, as an `implement` session does today. The options:
- (a) **Yes, in this Spec,** as its own Ticket before the Ticket run moves, with a glossary update so **Resume** no longer reads as Ticket-run-only.
- (b) **Only `implement` gets Resume** for now. Custom session Steps get it in a later issue.

*Recommended:* (a). Making it generic is about the same work as keeping it special-cased, and a Custom process's sessions lose background work in the same way.

*Answer:* agree

**Q2.4 - Custom processes can use `implement`.** The catalogue is shared (ADR-0024), so a new `implement` Step would be available to Custom processes as well. For example: `implement` on one Harness, a command Step running a three-family bug check, then `delivery`. The options:
- (a) **Yes,** on an Issue-branch worktree only, with the same refusals as other editing Steps.
- (b) **No, built-in only** for now.

*Recommended:* (a). It's the most useful Step to compose with, and refusing it means writing a second catalogue for built-ins only, which ADR-0024 avoids.

*Answer:* agree

## Round 3 (answered)

*Pre-answer:* Part of my intention with this putting these processes into this simple, easy-to-read process directory is trying to make it easier for people to understand how third shift works and the process that it operates. And to make it even easy as well to test modifications to it and evaluate those yeah and their impact on the output of the software factory that is third shift. 

**Q3.1 - When `implement` ends a Process early.** On an issue labelled `bug`, the `implement` session can end with `Bug not reproduced`. The Ticket run then ends successfully with triage notes and `needs-info`, opening no pull request (GLOSSARY "Ticket run"). In a file, that means a Step whose success skips every Step after it. No Step can do that today: a Step either succeeds and the next one runs, or it fails. With Q2.4, a Custom process can use `implement` too. The options:
- (a) **`implement` may end the Process successfully.** The Steps after it are skipped, the progress line and Ending say the bug was not reproduced, the Claim ends as a Ticket run's does today, and the Issue-branch worktree is cleaned up. This holds for a Custom process as well as the Ticket run.
- (b) **Only the built-in Ticket run gets the bug path.** In a Custom process, `implement` never diagnoses a bug.

*Recommended:* (a). The Ticket run's file needs it anyway, and with (b) a Custom process copied from `ticket-run` would quietly fix bugs it never reproduced. That loses the safeguard the bug path exists for.

*Answer:* agree

**Q3.2 - How to slice the Spec.** No slice needs a release before a later one (see "Split a Spec at a release" in `docs/agents/triage-labels.md`), so one Spec works. The proposed Tickets:
1. **The `processes/` folder, compiled into the binary,** with a README, plus `thirdshift process show <name>` for any Process. `process list` marks built-ins once there are any.
2. **Resume for every session Step**, with tests on a Custom process (Q2.3).
3. **A `security-review` catalogue Step** for Custom processes (Q2.1).
4. **An `implement` catalogue Step** for Custom processes: branch selection (fresh or **Continuation**), bug diagnosis and the early successful end (Q3.1), and the Base fix advice. Blocked by 2.
5. **The Ticket run runs from `processes/ticket-run.toml`**, with every starter (Issue URL, Spec run, Architect run, Pickup run, Security run) routed through it. Its sequencing leaves `src/run.rs`, and the existing tests pass with at most mechanical changes (Q1.2). This Ticket also updates the hand-written site pages. Blocked by 1, 3 and 4.

*Recommended:* this order. Tickets 1 to 4 each add something a Custom process can use on its own and leave the Ticket run untouched. Only 5 touches the Ticket run, and by then every Step it needs has been through a Custom process.

*Answer:* agreed I think, but we'll run the to-spec and to-tickets skills to finalize this. 

**Q3.3 - How the move is proven before it merges.** The tests are the main proof (Q1.2), but the Ticket run is what every unattended Command ends in. The options:
- (a) **The Spec PR is left ready for review, never Self-merged.** Before merging, you run real Ticket runs from a build of the **Spec branch** on a test repository: a fresh issue, a Continuation, a `bug` issue that reproduces and one that doesn't. You also run a Pickup run and a Spec run that start Ticket runs.
- (b) **Self-merge as usual** once the Spec PR is green.

*Recommended:* (a). This is the first Spec that rewires the path all other Commands depend on. A live check costs an afternoon, and a regression here would show up in every scheduled Pickup run.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- The file drives every Ticket run, whichever Command starts it, and the Ticket run's own sequencing leaves `src/run.rs` (Q1.1).
- No Ticket run behaviour changes. It keeps the name "Ticket run" in `thirdshift status`, the Console, the Activity log, Endings and Run notifications, not a Process name (Q1.2).
- The file reads roughly `implement` → Security review → `delivery` → `self-merge`. `implement` owns the fresh-or-**Continuation** branch selection, the bug-diagnosis prompt and its completion lines, and the Base fix advice (Q1.3).
- #1456 becomes the Spec for the Ticket run only. The Spec, Architect, Security and Pickup runs are filed afterwards, in that order (Q1.4).
- `thirdshift process list` shows `ticket-run` marked as built-in, and `thirdshift process show <name>` prints any Process's file (Q1.5).
- The folder is `processes/` at the repository root, matching `~/.thirdshift/processes/`. #877 left "processes or runs" as a musing, and Custom processes shipped as `processes`.
- Built-in process files live in a top-level folder of thirdshift's own repository, beside `skills/`, and are compiled into the binary (#877 Q1.2, ADR-0001).
- A Custom process can't take `ticket-run` or any other built-in name, so no file on a machine changes what a Pickup run or Spec run starts (#877 Q2.3).
- Safety rules stay inside catalogue Steps written in Rust. The Ticket run's file arranges Steps and doesn't restate the merge, Claim or Repair rules (ADR-0024).
- A Ticket run that sets no Harness, Model or Effort on a Step keeps today's per-Command choice (#877 Q2.4).
- Overriding the Ticket run's prompts or skills stays #118's job, after this lands (#877 Q2.3).
- `security-review` is a catalogue Step between `implement` and `delivery`. It decides for itself whether to run and keeps its fallback to the default Harness. Custom processes can use it (Q2.1).
- Spec-or-Ticket, Claim-or-not and the Child asks are decided in Rust before the `ticket-run` Process starts. The file starts at `implement` (Q2.2).
- Every session Step gets Resume, within this Spec, as its own Ticket. GLOSSARY "Resume" now says so (Q2.3).
- Custom processes can use `implement` on an Issue-branch worktree, with the same refusals as other editing Steps (Q2.4).
- GLOSSARY "Built-in process" and "Step" now say a Built-in process's Command keeps its run type's name, its file can be copied, and every catalogue Step is open to Custom processes.
- `implement` may end a Process successfully when a bug is not reproduced, skipping the Steps after it, in a Custom process as in the Ticket run (Q3.1).
- One Spec with the five Tickets in Q3.2's order and blocking links, to be finalized with `to-spec` and `to-tickets` (Q3.2).
- The Spec PR is left ready for review, never Self-merged. Live Ticket, Continuation, bug (reproduced and not), Pickup and Spec runs from a Spec-branch build come before the merge (Q3.3).
- The frontier is empty: the grill is complete.

## Research results (2026-10-27)

- Delivery owns the Ticket run's opening session (`OpeningSession::Implementation` or `BugDiagnosis`, chosen in `src/run.rs:586`), the Security review, the push, the Repair loop and the Self-merge. A Custom process's `delivery` Step has no opening session and never runs the Security review, confirmed by source.
- No built-in run type has a process file. `RESERVED` in `src/processes.rs:64` only blocks their names, confirmed by source.
- The catalogue today: skill session, command, `delivery`, `self-merge`, `run-published-issue` (`StepKind`, `src/processes.rs:293`), confirmed by source.
- The Ticket run's sessions are one `implement` session, an optional `security-review` and `repair-<n>` sessions inside Delivery (`src/session/kind.rs`). Bug reproduction happens inside the `implement` session, confirmed by source.

## Waiting on research

- None.
