# Grilling: #1494, Passes give way to a live Experiment

Issue: https://github.com/JacobStephens2/thirdshift/issues/1494
PR: https://github.com/JacobStephens2/thirdshift/pull/1982
Follow-up issue: #2118 (Q1.1 and Q1.3 still open)
Earlier decisions: [run-capacity.md](run-capacity.md) Q1.3, Q2.1-Q2.3, Q3.1-Q3.2

## Round 1 (partly answered by events)

**Q1.1 - Decisions the implementation made on its own.** PR #1982 has shipped, so this asks whether to change any of them. It settled three things the brief left open. The options:
- (a) **Accept all three:**
  - **Waiting Experiments take turns.** With two Experiments waiting, the first one to find no other live Commands starts, and the second waits for it, since a live Experiment is a live Command.
  - **Scope is the OS account, across every repository on the machine.** An Experiment holds back Passes on all repositories, not just the one it measures, and waits for that account's Commands on all of them. Other accounts are not counted.
  - **The results format goes to 11** with `no_wait`, `commands_at_start` and `waiting_for`. The Bar and the Experiment's identity are unchanged.
- (b) Change one of them (say which).

*Recommended:* (a). Taking turns follows from "a waiting Experiment is live" (run-capacity Q3.2). Two Experiments running at once would compete exactly as a Pass would. Scoping by account matches the brief, and the new host runs everything under one account (run-capacity Q2.6).

*Answer:* 

**Q1.2 - Who reviews and merges PR #1982.** Three ended attempts and an interrupted salvage went into it. The recovery Fix run on Lima left it ready for review with CI green on head `6c2ce8ef`, and Self-merge was disabled on purpose. The options:
- (a) **You read the PR and merge it by hand.** Read the summary, the guide section and `src/host_work.rs`.
- (b) **One Review run first, on a different Harness family from the one that wrote most of it** (Codex wrote the Ticket run; Claude did the recovery), then you merge.
- (c) Let a Review Pass pick it up whenever it does.

*Recommended:* (b). The work went through four hands on three machines and one interrupted salvage, so a fresh, targeted read is worth one session. It is a two-way door, so a review doesn't need to be heavier than that.

*Answer:* moot. PR #1982 was merged by hand on 2026-11-11 at 19:25, with no further Review run.

**Q1.3 - The collision across three machines.** The Fleet worked #1494 in parallel. Orchestrate's Pickup run held the issue from about 19:00 on 2026-11-10 until 01:08 on 2026-11-11. Meanwhile, test's Fix Pass took PR #1982 at 19:15, and its push was later refused because the branch had moved. A Pickup reserves the issue on origin (ADR-0017), but a Fix reserves the pull request, so neither reservation saw the other. #960 is close but different: a failing Run drafting a PR another Run is delivering. The options:
- (a) **File a new `needs-triage` bug:** "A Fix Pass takes a pull request whose Ticket run is still live on another machine". A Fix Pass should treat the closing issue's Pickup reservation as holding its PR.
- (b) Add it to #960 as a second case.
- (c) Leave it; the push refusal already stopped any damage.

*Recommended:* (a). It cost a session and left a branch that needed salvage, and it will recur on the new shared host. It is a separate mechanism from #960, which is about drafting rather than taking.

*Answer:* 

**Q1.4 - After the merge.** #1979 needs a release with this in it before it can install anything. The options:
- (a) **Cut a release right after #1982 merges,** with `scripts/release.sh` and no Experiment, because this change makes Passes skip and doesn't change what agents do. Then install the release on the existing hosts as usual.
- (b) Wait for the next scheduled release.

*Recommended:* (a). It unblocks #1979. The new skip only fires while an Experiment is live, so on the existing hosts it changes nothing until someone starts an Experiment there.

*Answer:* moot. v0.29.2 (2026-11-12 00:54) and v0.30.0 (2026-11-12 13:47) were released after the merge.

## Settled by events (shout if any is wrong)

- #1494 closed when PR #1982 merged on 2026-11-11, and it shipped in v0.29.2 and v0.30.0. It is no longer in the way of #1979's install step.
- Q1.1 and Q1.3 continue in #2118, labelled `ready-for-human`.

## Research results (2026-11-11)

- PR #1982 is ready for review, mergeable, and CI `test` and `site-test` pass on head `6c2ce8ef`. Confirmed by `gh pr view` and `gh pr checks`.
- The Fleet's history for PR #1982: orchestrate's Pickup and Ticket run failed; test's Fix run (19:15 on 11-10) failed; dev's Fix run was interrupted; Lima's Fix run (12:54 on 11-11) ended ready for review. Confirmed by `thirdshift status` on the Lima VM.
- The diff covers 36 files. The user-facing docs, the glossary and the site pages are updated. The glossary's Pass entry adds "for the same OS account" and the warning sentence. Confirmed by `git diff origin/main...origin/issue-1494`.
- #1494 is labelled `ready-for-human` again, from the recovery brief (2026-11-11 17:54), and stays out of Pickup while the PR is open.
