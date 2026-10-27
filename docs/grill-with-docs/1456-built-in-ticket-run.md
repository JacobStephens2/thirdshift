# Grilling: #1456, Built-in processes: move the Ticket run into a process file

Issue: https://github.com/JacobStephens2/thirdshift/issues/1456
Research: `docs/grill-with-docs/877-composable-by-file.md` (Q1.1, Q1.2, Q1.5, Q2.3, Q2.4, Q3.4, Q4.6), ADR-0024, ADR-0001

## Round 1 (open)

**Q1.1 - What "moved" means.** #877 Q1.5 agreed that each run type becomes a file that thirdshift itself runs. A file could also just describe the Ticket run while Rust keeps running it. The options:
- (a) **The file drives the Ticket run.** `processes/ticket-run.toml` is compiled into the binary, and every Ticket run runs through the same Process engine as a Custom process, whether it starts from an Issue URL, a Spec run, an Architect run, a Pickup run or a Security run. The Ticket run's own sequencing in `src/run.rs` goes away.
- (b) **The file is documentation.** It mirrors what `src/run.rs` does, and a test keeps the two in step. Nothing changes at run time.

*Recommended:* (a). With (b) there are two sources of truth, and nothing new becomes possible. #118 (overriding a built-in's prompts) also needs (a).

*Answer:* 

**Q1.2 - Behaviour stays identical.** Moving the Ticket run is a refactor of the riskiest path in thirdshift. The options:
- (a) **No behaviour change.** Every Ticket run does exactly what it does today: the same sessions, progress lines, Command log, Endings, Run notifications, Console view and Recovery. The existing tests (`tests/` fakes, `src/run.rs` and `src/delivery.rs` unit tests) pass with at most mechanical changes, such as a Command's stored kind. Anything that would change behaviour gets its own issue.
- (b) **Change what the move makes easy,** such as splitting sessions apart, in the same Spec.

*Recommended:* (a). A test suite that passes unchanged is the only practical proof that nothing safety-critical moved. Improvements are much easier once the file exists.

*Answer:* 

**Q1.3 - How coarse the new catalogue Steps are.** ADR-0024 listed bug reproduction, **Continuation**, **Resume** and the **Repair loop** as missing parts. In the code they aren't separate sessions. A Ticket run is one **Implementation** session (`implement`, with the `thirdshift-implement` skill), which reproduces a bug, fixes it, reviews and opens the PR. Then come the optional **Security review** and **Delivery**. Bug reproduction is the session prompt plus its `Bug reproduced` / `Bug not reproduced` completion lines (`src/prompt.rs:313`, `src/delivery.rs:432`). Continuation is how the branch is selected, Resume is retrying a session, and the Repair loop is already inside `delivery`. The options:
- (a) **Coarse: one new catalogue Step, `implement`.** It owns branch selection (fresh or Continuation), the bug prompt and its completion lines, the Base fix advice, and Resume. The file then reads roughly `implement` → `security-review` → `delivery` → `self-merge`. Resume becomes a property of every session Step, Custom ones included, since it is not specific to Ticket runs.
- (b) **Fine: separate Steps** for reproduction, implementation, review and PR opening, each its own session.

*Recommended:* (a). (b) splits one session into several, which changes behaviour and breaks Q1.2(a). Splitting can come later as a Custom process people try first.

*Answer:* 

**Q1.4 - Scope of this issue.** The other run types need different Steps: a Spec run coordinates Tickets on a **Spec branch**, an Architect run and a Security run have gates and their own sessions, and a Pickup run reserves issues. The options:
- (a) **The Ticket run only.** #1456 becomes the Spec for it. After it lands, the Spec, Architect, Security and Pickup runs are each filed as their own issue, in that order, each grilled against what the Ticket run taught.
- (b) **All five in one Spec,** with a Ticket per run type.

*Recommended:* (a). This follows #877 Q1.5 ("one at a time, starting with the Ticket run"). A Spec covering five run types would be the largest Spec yet, and its later Tickets would be designed before the format had been tested.

*Answer:* 

**Q1.5 - Can people see the built-in file?** #877 Q1.2 put built-in processes in a visible top-level folder of thirdshift's repository, and #877 Q2.3 forbids a Custom process from taking a built-in's name. The options:
- (a) **List it and print it.** `thirdshift process list` shows `ticket-run` marked as built-in, and a new `thirdshift process show <name>` prints any Process's file. Someone can save that as a Custom process under a new name and change it.
- (b) **List it only.** The file can be read on GitHub.
- (c) **Neither.** Built-ins stay out of `process` commands.

*Recommended:* (a). Copying the Ticket run is the most useful starting point for a Custom process, and printing it shows exactly what the installed version runs.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- Built-in process files live in a top-level folder of thirdshift's own repository, beside `skills/`, and are compiled into the binary (#877 Q1.2, ADR-0001).
- A Custom process can't take `ticket-run` or any other built-in name, so no file on a machine changes what a Pickup run or Spec run starts (#877 Q2.3).
- Safety rules stay inside catalogue Steps written in Rust. The Ticket run's file arranges Steps and doesn't restate the merge, Claim or Repair rules (ADR-0024).
- A Ticket run that sets no Harness, Model or Effort on a Step keeps today's per-Command choice (#877 Q2.4).
- Overriding the Ticket run's prompts or skills stays #118's job, after this lands (#877 Q2.3).

## Research results (2026-10-27)

- No built-in run type has a process file. `RESERVED` in `src/processes.rs:64` only blocks their names, confirmed by source.
- The catalogue today: skill session, command, `delivery`, `self-merge`, `run-published-issue` (`StepKind`, `src/processes.rs:293`), confirmed by source.
- The Ticket run's sessions are one `implement` session, an optional `security-review` and `repair-<n>` sessions inside Delivery (`src/session/kind.rs`). Bug reproduction happens inside the `implement` session, confirmed by source.

## Waiting on research

- None.
