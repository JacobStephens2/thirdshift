# Grilling: #1251, Spec-review Trials have no Trial inputs or PR service

Issue: https://github.com/JacobStephens2/thirdshift/issues/1251
Research: #698's grilling (`done/698-evals/`), Q3.11 and Q6.3; `docs/eval-tasks.md` and `docs/trial.md` on `issue-977`

## Round 1 (answered)

**Q1.1 - The PR question is already settled by #698.** #1251 asks how a Spec-review Trial gets a PR it can check and edit, and offers a fake `gh` service, throwaway real PRs, or a review-only prompt. #698 settled this. Q3.11 gave each Spec-review Trial a local bare origin, let its `gh` update fail harmlessly, and graded only its review reports; it turned down a review-only prompt because that isn't the production prompt. Q6.3 lets a Trial reach only its provider, which rules out real PRs. The Trial flag already works this way: a Trial's reports are read from its own report files, not from the PR (see Research results). The options:
- (a) **Keep #698's answer.** #1251 drops its three options and needs no PR service. A Spec-review Trial's `gh` calls fail, and only its reports are graded.
- (b) **Reopen Q3.11** for a fake `gh` service, so the session's PR checks succeed as they would in a Run.

*Recommended:* (a). Nothing found since #698 undercuts Q3.11, and (b) would add a service to keep in step with GitHub for no change in what is graded.

*Answer:* agree

**Q1.2 - The issue and PR URLs a Spec-review task records.** The replay manifests hold each Spec's issue URL, Base branch, Spec branch and PR URL, so the import can fill `[trial]` without your help. The URLs appear in the session's prompt. The options:
- (a) **The original URLs from the manifest.** This matches what the session saw in production, and what the Code-review import already writes. Two separate barriers keep the PR unchanged: the Trial can reach only its provider (Q6.3), and its fresh home has no `gh` login.
- (b) **Made-up URLs** on a repository that doesn't exist. Even a future sandbox that lets GitHub through couldn't change a real PR, but the session sees a URL that doesn't match its repository.

*Recommended:* (a). It keeps the two imports alike, and either barrier alone already protects the PR.

*Answer:* agree

**Q1.3 - An Experiment that selects a task with no `[trial]` table.** Today each such Trial stops with `missing Trial inputs`, after both Arms are built. Hand-made or proposed-then-approved tasks may still lack the table, even once the import fills it. The options:
- (a) **Refuse before building the Arms,** naming every task without a table. A release that touches the suite waits until they're filled.
- (b) **Leave those tasks out** with a warning, and run the rest.
- (c) **Count each as a void Trial.**

*Recommended:* (a). (b) and (c) quietly shrink the sample the Verdict rests on, and a release could pass on fewer tasks than you think.

*Answer:* agree

**Q1.4 - A live check of Q3.11's assumption.** Q3.11 assumed the failed `gh` call is harmless. The Spec review prompt's PR rule says to check the PR before each push or PR edit, and to stop if it is merged or closed. It says nothing about a check that can't be made. A session that stops early, or works around the failure, would still write its reports, but could spend its effort differently from a Run. Only a live session shows which. The options:
- (a) **A separate `ready-for-human` Ticket,** like #1009's live sandbox checks: run one imported Spec-review task through the Trial flag on the host, read its session log, and report how it handled the failed checks. #1251 itself is tested with the fake Harness.
- (b) **Part of #1251's acceptance,** run by the agent on the host. It spends one subscription session inside a Spec run.
- (c) **No live check;** the first release Experiment shows it.

*Recommended:* (a). It keeps #1251 agent-ready and puts a session on your subscription only when you choose to start one.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - Where the live-check Ticket goes.** Q1.4 made the live check a `ready-for-human` Ticket of its own. It needs #1251's import to have filled the tables, so it is blocked by #1251. A Spec run stops at any Ticket it can't take, as it did at #1018 and #1019. The options:
- (a) **Standalone, blocked by #1251.** Spec #977 can finish and merge without it. You run the check whenever you like, with a release build once the Spec has shipped.
- (b) **A Ticket of Spec #977, blocked by #1251.** The Spec's PR waits for the check, and you run it from a build of `issue-977`, as with #1018.

*Recommended:* (a). The check can only lead to a later prompt change; it doesn't change what #977 builds, so it shouldn't hold the Spec's PR.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- Spec-review Trials keep the production Spec review prompt (Q3.11 (a)).
- The import fills a Spec-review task's `[trial]` as it does a Code-review task's: `base` from the manifest's Base branch, `branch` from its Spec branch, written only when the table is missing, keeping later edits. A rerun completes the 30 tasks already imported on the host.
- The docs' instruction to "supply fixture services or disposable PRs" comes out of `docs/eval-tasks.md` and `docs/trial.md`.
- #698's Q3.11 stands: no PR service. A Spec-review Trial's `gh` calls fail, and only its reports are graded (Q1.1).
- #1251's "Decision needed" section and its three options come out of the issue (Q1.1).
- The import writes each Spec-review task's original issue and PR URLs from the manifest into `[trial]` (Q1.2).
- An Experiment refuses before building either Arm when any selected approved task has no `[trial]` table, naming each one. That includes release Experiments (Q1.3).
- #1251 is tested with the fake Harness; a live session is left to the separate Ticket (Q1.4).
- #1251 stays one Ticket in Spec #977, labelled `ready-for-agent` once this grilling ends. It has no open blockers, so the Spec's Recovery run builds it along with #1020 and #1021.
- The live check is #1255, standalone and `ready-for-human`, blocked by #1251, outside Spec #977 (Q2.1).

## Research results (2026-10-24)

- A Trial's graded reports come from its `standards` and `spec` report files, not from the PR, confirmed by source (`src/trial.rs`, `Reports`, on `issue-977`).
- A Trial's sandbox lets through only its Harness's provider hosts; `github.com:443` is refused, confirmed by source and its test (`src/sandbox.rs`, `a_disallowed_host_is_refused_through_the_proxy`).
- The Spec review prompt includes the shared PR rule (`PR_LIFECYCLE`, `src/prompt.rs:46`), with no instruction for a check that can't be made, confirmed by source.
- An Experiment reads each Trial's `[trial]` table only when the Trial starts, after the Arms are built (`src/evals/experiment/workers.rs:245`), confirmed by source.
- `spec-review` depends on `src/prompt.rs`, `src/trial.rs`, `src/session.rs`, `src/session/`, the Harness and config files and three Factory skills, confirmed by the imported `suite.toml` on the host.

## Waiting on research

- Nothing.
