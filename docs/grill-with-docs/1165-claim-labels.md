# Grilling: #1165, in-progress stays on issues whose Run has ended

Issue: https://github.com/JacobStephens2/thirdshift/issues/1165
Research: [proposed direction](https://github.com/JacobStephens2/thirdshift/issues/1165#issuecomment-6369972244), #585, #710, #833, #1583, #1695, [#1532 grill](1532-keep-working-the-backlog.md)

Paused 2026-10-23 with Round 1 unanswered. Revised in place on 2026-10-27, and twice on 2026-11-09: after the #1532 grill and Spec #1564 shipped the `agent-failed` label and the **Failure limit** in place of the **Claim limit**, and for Spec #1695, which gives Triage runs a Claim. Q1.1 is withdrawn, Q1.2–Q1.4 are rewritten for the new labels, and Q1.5 is new. Rounds 1–3 answered 2026-11-09; the frontier is empty and the grill is done. `GLOSSARY.md` now has the new **Claim**, **Failure limit**, **Ready issue**, **Recovery** and **Sweep** entries.

Agreed in chat on 2026-10-23, before Round 1, and where each stands now:

- A **Failed run** that leaves work on `origin` swaps `in-progress` for a new label, `run-failed`. *Now:* #1564 added `agent-failed` for this case, but next to `in-progress` rather than in its place. Q1.5 asks whether to finish the swap.
- `run-failed` does not count toward the limit. *Now:* the reverse shipped. `agent-failed` is exactly what the Failure limit counts (Q1.1, withdrawn).
- A Spec run that stops only for the Day shift gets `ready-for-human`, not a failure label. Q1.3 questions this.

## Round 1 (answered)

**Q1.1 - What stops Pickup working through every Ready issue when every Run fails?** *Withdrawn 2026-11-09, settled by the #1532 grill.* Pickup now counts open `agent-failed` issues against `pickup.failure_limit`, default 3. A broken Base branch or Harness stops the line after three failures, and pull requests waiting for review never count. One gap was left open here, and it is still open. A failure with nothing on `origin` releases its Claim to `ready-for-agent` and gets no `agent-failed`, so Pickup can take the same issue again on its next pass. See "Settled by earlier answers".

**Q1.2 - What label does a Run whose pull request waits for review leave?** A Ticket run or Spec run keeps `in-progress` (`Goal::ReadyForReview` and a held **Self-merge** leave the labels alone, `src/claim.rs:349`). A **Custom process**'s Delivery removes `in-progress` and puts nothing back (`Claim::remove`, #1388). The same ending therefore leaves different labels depending on which Command ran. When this was first asked, `in-progress` on these issues did a job, because the Claim limit counted them to stop unreviewed pull requests piling up. #1532 dropped that job on purpose: "a pull request that is ready for review or merged never counts, so the factory keeps working while reviews wait." Nothing counts these issues now. Two more facts bear on this. The Console's Inbox lists them under Ready for review, across the **Fleet**. And on pull requests, `in-progress` already means a live reservation that a **Review run** or **Fix run** takes off when it ends. The options:
- (a) Keep `in-progress` on both paths, which undoes that part of #1388. The label still overstates live work.
- (b) A new `in-review` label on both paths. It counts toward nothing, so it would be for reading only.
- (c) Take `in-progress` off on both paths, as the Custom process already does. The open pull request stands for the work. Pickup still won't take the issue again, because a started issue is a **Ready issue** only after a handback.

*Recommended:* (c). With no limit counting these issues, a second label would only repeat what the open pull request and the Inbox already show. Under (c), `in-progress` means one thing on issues and on pull requests: a live Run. It also makes the Ticket and Spec paths match what #1388 shipped. An issue whose Self-merge merged but whose closing failed keeps its Claim as it does today, since that is a finish still owed, not a review.

*Answer:* agreed

**Q1.3 - What does a Spec run that is only waiting leave on its Spec?** This matters more now than when first asked. A Spec run whose undone Tickets are all gated or blocked ends with `Tickets not done: #22 (labelled ready-for-human)` (`src/spec_run.rs:457`). That makes it a **Failed spec run**, so it keeps its Claim and gets `agent-failed`. Three such waiting Specs would stop Pickup on the repository under the Failure limit, though nothing is broken. Pickup already passes over a Spec with no **Startable Ticket**, so a Spec labelled `ready-for-agent` whose Tickets are all gated stays put on its own, and Pickup continues it once a Ticket becomes startable. The options:
- (a) `ready-for-human` on the Spec, with no `agent-failed`, when every undone Ticket is an **Unready Ticket** or is blocked only behind ones. A Spec waiting on ordinary work elsewhere, such as a Ticket in another running Spec, gets `agent-failed`.
- (b) thirdshift puts `ready-for-agent` back on any Spec whose undone Tickets are all gated, waiting or blocked, with no `agent-failed`. The **Ready issue** rule accepts that as a handback. The Spec continues on its own once the Day shift clears a gate or a blocker closes, and only the Tickets carry `ready-for-human`.
- (c) (a) for human gates, (b) for waiting on other work.

In every option, a Spec run with a failed or interrupted Ticket gets `agent-failed`, as #585 proposes. Two more cases from #585 are already happening on tourbot. In #880, the only open Ticket was `needs-info`, and the run "failed" after 34s. In #1689, one Ticket was `ready-for-human` and one sat in another repository, so no Spec run can start it. Both count as waiting here. A Ticket outside the Spec's repository is never a **Startable Ticket**, so Pickup passes over the Spec under (b) as well. #585 keeps its other decisions: the outcome name, exit code, notification wording and mixed results. This question decides only the labels.

*Recommended:* (b), with one guard. The Day shift does one thing, clearing the Ticket's gate, not two. The `ready-for-human` list then shows exactly the work that is human: Tickets, not Specs. The guard comes from the **Trusted account** rule. A Ready issue needs its latest `ready-for-agent` labelling by a Trusted account, with no edit since by an account that is not one. If thirdshift relabelled the Spec itself, as the owner's account, it would clear an untrusted edit made in between. So thirdshift puts `ready-for-agent` back only if no such edit has happened since the Day shift's last labelling. Otherwise the Spec gets `needs-triage`. This reverses the chat answer, so say if the Spec itself should still show `ready-for-human`.

*Answer:* agreed

**Q1.4 - Does a Failed run post its Recovery on the issue?** **Recovery** lines are built only from factory-owned facts. They reach the command log, the Run notification if one is configured, and the Console's Needs you. Since #1769, one Console shows Needs you for every machine in its **Fleet**, which covers most of #833's "which host?" for the Day shift. On GitHub, `agent-failed` says only that a Run failed, not why or what to do next. thirdshift already posts issue comments elsewhere: Base branch CI reports, and Triage runs, which say "This comment was generated by AI." #1695 adds a lot more of them: each triaged issue gets an AI-attributed Agent Brief, Questionnaire or closure comment. A #585 comment also asks for "a comment with the cause and Recovery line" on a waiting Spec. The repository is public, and Recovery lines include the host's paths and the full continue command. A comment from the owner's account is from a Trusted account, so the next session on the issue would be given it. The options:
- (a) A comment on the issue at each Failed run, marked as generated. It gives the cause, where the Run stopped, what was kept (the branch and pull request), the host, and how to retry ("apply `ready-for-agent`"). No local paths and no continue command.
- (b) The full Recovery lines, paths and command included.
- (c) No comment. `agent-failed` sends the Day shift to the Fleet Console.

*Recommended:* (a), a little more firmly than before, now that #1695 makes AI comments on issues routine. The Console covers the Day shift, but only on machines in its Fleet. The comment serves anyone reading the issue on GitHub, and it hands the next Continuation session a factual account of where the last Run stopped. A Run that fails again posts again, which leaves a history of attempts. Pick (c) if you would rather keep factory chatter off issues.

*Answer:* agreed

**Q1.5 - Does a failed Run that keeps its Claim still keep `in-progress` next to `agent-failed`?** This is the heart of #1165, now that #1564 has shipped. Today a Failed run with work on `origin` leaves both labels, so `in-progress` still doesn't mean "running". The pre-Round-1 answer was to swap one for the other, but it now runs into a #1532 rule (Q3.3 there, `src/pickup.rs:423`). Pickup reads an open issue with `agent-failed` and neither `in-progress` nor `needs-triage` as the Day shift having taken it over, and takes `agent-failed` off. Swap the labels without changing that rule, and the next Pickup run would clear every failure, so the brake would never hold.

**Spec #1695 already makes this swap, for Triage runs only.** It is being built now (Ticket #1891 is `ready-for-agent`). A failed Triage run on an issue replaces `in-progress` with `ready-for-human` plus `agent-failed`. #1695 then works around the rule above by adding a second take-over signal: the Sweep clears `agent-failed` once the Day shift removes `ready-for-human`. Left alone, that gives two take-over rules depending on which Command failed. #1583 (`ready-for-human`) asks the same about failed Custom process runs, which today keep `in-progress` and get `agent-failed`, like Ticket runs. The options:
- (a) Swap, with one take-over rule for every Command that fails while holding a Claim: Ticket, Spec, Custom process and Triage runs. The failure takes `in-progress` off and leaves `agent-failed`. You take an issue over by removing `agent-failed`, putting `ready-for-agent` back, or closing it. The #1532 "took `in-progress` off" rule goes, and #1695 drops its `ready-for-human` signal. Its failed Triage run still adds `ready-for-human`, but only so the Triage Pass passes over the issue.
- (b) Swap, and follow #1695 everywhere: a failure leaves `ready-for-human` plus `agent-failed`, and removing `ready-for-human` is the take-over. To retry a Ticket, you'd then take `ready-for-human` off and put `ready-for-agent` on, two edits, because `ready-for-human` makes an issue Unready.
- (c) Keep both labels on Ticket, Spec and Custom process runs, so "running" is `in-progress` without `agent-failed`. #1695's Triage runs stay the exception.

*Recommended:* (a). That is what #1165 asks for, and with Q1.2 (c) it leaves `in-progress` on an issue only while a Run is live. A single take-over act, removing the label that names the failure, is easier to learn than one per kind of Command. It also answers #1583: a failed Custom process counts like any other failure. Under (b), `ready-for-human` would mean "human work" on some issues and "the factory failed" on others. **This is time-sensitive.** If you pick (a), #1891 should drop its `ready-for-human` take-over rule before it lands, so say so and I'll comment on #1891. The cost is a change to a rule shipped two weeks ago. Issues that already failed also need `in-progress` taken off once, which the Sweep could do.

*Answer:* agreed

*Checked:* answered on the text from before the #1695 revision, where (a) was the plain swap with one take-over rule, removing `agent-failed`. That part is settled. Applying it to #1695's Triage runs and to #1583's Custom processes was added later, so Q2.1 and Q2.2 put those to you.

## Round 2 (answered)

**Q2.1 - Does #1695's Triage run use the same take-over rule?** Q1.5 settled one take-over rule: a failure swaps `in-progress` for `agent-failed`, and the Day shift acts by putting `ready-for-agent` back, removing `agent-failed`, or closing the issue. Spec #1695, being built now as Ticket #1891, gives a failed Triage run `ready-for-human` plus `agent-failed`, and adds a second signal: the Sweep clears `agent-failed` once `ready-for-human` comes off. The options:
- (a) One rule. A failed Triage run still gets `ready-for-human` next to `agent-failed`, so the Triage Pass passes over it, but `agent-failed` comes off only by the Q1.5 rule. #1891 drops its `ready-for-human` signal. I comment on #1891 and #1695 now, before it lands.
- (b) Keep #1695's extra signal for Triage runs only.

*Recommended:* (a). The Day shift learns one act for every failure. Under (b), taking `ready-for-human` off would quietly clear a failure for one kind of Command only. Nothing is lost: if you answer a failed triage by relabelling the issue, you'd normally take `agent-failed` off in the same edit.

*Answer:* agreed

*Checked:* drafts of the comments for #1891 and #1695 are ready, but posting them was blocked by a permission check, so they wait for you (see "Waiting on research"). #1891 can't simply drop the `ready-for-human` signal before #1165 lands: today's `src/pickup.rs` check would clear `agent-failed` from a failed Triage run at once, since it has neither `in-progress` nor `needs-triage`. The draft says #1891 may treat `ready-for-human` like `needs-triage` in that check for now, but shouldn't document removing it as the take-over act.

**Q2.2 - Does a failed Custom process count toward the Failure limit (#1583)?** Today it does: `HeldClaim` ends its Claim through the same path as a Ticket run, so with work on `origin` it gets `agent-failed`. Under Q1.5 it would also lose `in-progress`. #1583 asks whether that is right, since #1564 named only Ticket runs and Spec runs. The options:
- (a) Yes. A failed Custom process on an issue waits for the Day shift exactly as a Failed ticket run does. Answer and close #1583 with that.
- (b) No. A Custom process releases or keeps its Claim without the label, and doesn't count.

*Recommended:* (a). A Custom process is Day-shift-authored and experimental, but an issue it failed on still waits for the Day shift. Leaving it out would also leave its failures on `in-progress`, which is what #1165 is removing.

*Answer:* agreed

**Q2.3 - Which failures post the Recovery comment (Q1.4)?** Q1.4 settled a generated comment with no local paths. Three cases need a boundary:
- **A failed Security fix.** Its issue is a fix issue on the repository, but its cause can describe the vulnerability, which today stays in the private advisory.
- **A failure with nothing on `origin`.** It releases to `ready-for-agent`, and Pickup can take it again at once, so a comment per failure could repeat on every Pickup pass.
- **A Spec run that is only waiting (Q1.3).** It gets no `agent-failed`. #585 asks for the cause on the Spec: which Tickets wait, and on what.

The options:
- (a) Comment only when the issue ends `agent-failed`, except a failed Security fix, whose Recovery stays in the advisory. Also comment on a waiting Spec, naming each undone Ticket and what it waits on. No comment on a release.
- (b) Comment on every Failed run, Security fixes and releases included.
- (c) Only `agent-failed` issues, Security fixes included, and no waiting-Spec comment.

*Recommended:* (a). It keeps vulnerability details private and can't repeat on every pass. It gives the waiting Spec what #585 asked for: "claimed and waiting for you" looks different from a failure on GitHub. A waiting Spec isn't taken again until a Ticket becomes startable, so its comment doesn't repeat either.

*Answer:* agreed

**Q2.4 - What happens to issues labelled the old way?** When this ships, open issues will still carry `in-progress` beside `agent-failed`, or `in-progress` while their pull request waits for review. Under the new rules, `in-progress` and `agent-failed` never sit on one issue, because a new Claim takes `agent-failed` off. The options:
- (a) The Sweep also takes `in-progress` off any open issue that carries `agent-failed`. That one rule is safe forever. The ready-for-review issues are cleared by hand once, from a list in the pull request, because a live Run can't be told apart from GitHub alone.
- (b) Clear both by hand once, with no Sweep change.
- (c) Leave them. They fade as issues close.

*Recommended:* (a). The `agent-failed` case can't be ambiguous, so the Sweep can own it. The ready-for-review case can be, because a Continuation could be live on an issue whose pull request was ready, and only a person checking the Inbox can tell.

*Answer:* agreed

**Q2.5 - File the unbraked release retry as its own issue?** A Ticket run that fails with nothing on `origin` releases to `ready-for-agent` with no `agent-failed` (unchanged by Round 1). Nothing stops Pickup taking the same issue again on every pass, as a Harness that fails before its first commit would. The options:
- (a) File it as a `needs-triage` issue now, linking #1165 and #1532, and keep it out of this Spec.
- (b) Fold it into this Spec: such a failure gets `agent-failed` too, with no Claim to keep.
- (c) Leave it.

*Recommended:* (a). It is a Failure-limit question, not a label-meaning one, and (b) would change what the Failure limit counts, which #1532 settled deliberately.

*Answer:* agreed

## Round 3 (answered)

Round 2 settles the last design branch. What's left is how to publish it.

**Q3.1 - Publish as a new Spec with three Tickets?** The work spans more than one session, so `docs/agents/triage-labels.md` makes it a Spec, published `needs-triage` until its Tickets are linked. #1532 set the precedent of a new Spec issue (#1564) rather than rewriting the problem issue. The proposed split:
1. **Claim endings.** Ready for review takes `in-progress` off on every path (Q1.2). A failure swaps it for `agent-failed`, Custom processes included (Q1.5, Q2.2). The take-over check becomes "`agent-failed` taken off" (Q1.5), and the Sweep takes `in-progress` off open `agent-failed` issues (Q2.4). These ship together, because the swap without the new check would clear every failure.
2. **Waiting Spec runs.** Put `ready-for-agent` back with no `agent-failed`, guarded by the Trusted-account check, or label the Spec `needs-triage` (Q1.3). Blocked by 1.
3. **Recovery comments.** A generated comment on every `agent-failed` issue except Security fixes, and the waiting-Spec comment (Q1.4, Q2.3). Blocked by 1 and 2.

Each Ticket updates the guide and the hand-written site pages that its change makes wrong. The one-time clearing of ready-for-review issues (Q2.4) can't happen until the Spec is released, so per "Split a Spec at a release" it becomes a standalone `ready-for-human` issue blocked by the Spec. The options:
- (a) A new Spec as above, which closes #1165 when it lands.
- (b) Rewrite #1165 as the Spec.
- (c) One Ticket for everything.

*Recommended:* (a). It matches #1564 and keeps #1165's history readable. The split follows the dependencies: 1 is the label meaning, 2 and 3 build on it.

*Answer:* agreed, using the to-spec and to-tickets skills

**Q3.2 - What happens to the related open issues?** This grill answers part of three of them:
- **#585**, waiting Spec runs. The labels and the comment are settled here. The outcome name, exit code, notification wording and mixed results are not.
- **#833**, which machine holds a Claim. The Recovery comment names the machine on a failure, but nothing names it while a Run is live.
- **#1165**, closed by the Spec.

The options:
- (a) Comment on #585 and #833 with what this Spec settles, and leave both open for the rest.
- (b) Fold #585's remaining decisions into this Spec.

*Recommended:* (a). #585's remaining questions are about outcomes and notifications, not labels. Folding them in would make the Spec wait on a grill of its own.

*Answer:* agreed

**Q3.3 - Record an ADR?** A decision earns one when it is hard to reverse, surprising without context, and a real trade-off. "`in-progress` marks only a live Run: a pull request waiting for review gets no label, and a failure gets `agent-failed` instead" passes all three. It changes what both hosts and every reader of GitHub rely on. A future reader will wonder why a Claimed issue with an open pull request has no label. And `in-review` was a real alternative, rejected once #1532 stopped counting reviews.
- (a) Yes, one short ADR covering Q1.2 and Q1.5.
- (b) No. The glossary and this file are enough.

*Recommended:* (a). The glossary says what the labels mean but not why the alternatives lost, and this file isn't where a future reader would look.

*Answer:* agreed

*Checked:* written as `docs/adr/0029-in-progress-marks-only-a-live-run.md`, amending ADR 0026's take-over rule.

## Settled by earlier answers (shout if any is wrong)

- **`in-progress` means a live Run**, on issues and on pull requests alike (Q1.2, Q1.5). It goes on when a Run starts and always comes off when it ends.
- **Ready for review:** a Ticket run, Spec run or Custom process whose pull request is ready, or whose Self-merge was held with its pull request ready, takes `in-progress` off and puts nothing back. `Claim::remove` from #1388 becomes the ending for all of them (Q1.2).
- **Pickup still won't retake those issues:** a started issue is Ready only after a handback (Q1.2).
- **A merged but still open issue** keeps `in-progress` until it is closed, as today (Q1.2).
- **Failure:** a Failed run with work on `origin`, and a failed Security fix even with nothing, swaps `in-progress` for `agent-failed`. The #1532 take-over rule "neither `in-progress` nor `needs-triage`" is replaced by "`agent-failed` taken off" (Q1.5, at `src/pickup.rs:423`).
- **A waiting Spec run** puts `ready-for-agent` back on its Spec, takes `in-progress` off and adds no `agent-failed`. It does so only if no untrusted edit has happened since the last Trusted `ready-for-agent` labelling, and labels the Spec `needs-triage` otherwise. Waiting means every undone Ticket is an Unready Ticket, blocked, waiting on other running work, or outside the Spec's repository (Q1.3).
- **A Spec run with any failed or interrupted Ticket** gets `agent-failed` (Q1.3).
- **A blocking cycle** among undone Tickets is a failure, not waiting: nothing unblocks it until someone reshapes the Tickets. Decided in chat on 2026-11-09, while checking the Spec's test seams.
- **#585** keeps the outcome name, exit code, notification wording and mixed results. This grill settles only the labels (Q1.3).
- **The Recovery comment** is marked as generated, names the machine and how to retry, and leaves out local paths and the continue command. A Run that fails again posts again (Q1.4). Its boundary is Q2.3.
- **Interrupted runs** are treated like any other Failed run. A Stop still leaves `needs-triage`, and a bug that can't be reproduced still leaves `needs-info`.
- **Label upkeep:** a new Claim takes `agent-failed` off, and the Sweep takes both labels off closed issues. Both already ship.
- **Tickets:** a Ticket run inside a Spec run makes no Claim, so its Spec carries the outcome.
- **Publication:** a new Spec, published `needs-triage` with the to-spec and to-tickets skills, and three Tickets: Claim endings; waiting Spec runs (blocked by 1); Recovery comments (blocked by 1 and 2). A standalone `ready-for-human` issue, blocked by the Spec, clears the old ready-for-review labels after release. The Spec closes #1165 (Q3.1).
- **Related issues:** #585 and #833 get a comment on what the Spec settles and stay open (Q3.2).
- **ADR 0029** records Q1.2 and Q1.5 (Q3.3).
- **Out of scope:** a Run killed outright never ends its Claim (#710).
- **One take-over rule for every failure,** Triage runs included: `agent-failed` comes off when the Day shift puts `ready-for-agent` back, takes `agent-failed` off, or closes the issue (Q2.1). A failed Triage run still also gets `ready-for-human`, which is #1695's business.
- **Custom processes** count toward the Failure limit and swap `in-progress` for `agent-failed` like any other failure (Q2.2).
- **Recovery comments** go on every `agent-failed` issue except a failed Security fix, and on a waiting Spec. A released issue gets none (Q2.3).
- **The Sweep** also takes `in-progress` off open issues that carry `agent-failed`, and keeps doing so. Issues whose pull request waits for review are cleared by hand once, after release (Q2.4).
- **The release-retry gap** becomes its own `needs-triage` issue, outside this Spec (Q2.5).
- **Glossary:** **Claim**, **Failure limit**, **Ready issue**, **Recovery** and **Sweep** updated on 2026-11-09. Triage runs join **Claim** when #1891 lands, since they don't hold one yet.

## Research results (2026-10-23)

- `Claim::end` leaves the labels as they are on `Goal::ReadyForReview` and on a failed Security fix, releases only when nothing is on `origin`, and removes the Claim once the issue is closed after a merge. Confirmed by source, `src/claim.rs`.
- A **Spec** with no **Startable Ticket** is not a **Ready issue**, so Pickup already passes over a waiting Spec. Confirmed by `GLOSSARY.md`.
- An **Unready Ticket** carries `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. Confirmed by `GLOSSARY.md`.
- `JacobStephens2/thirdshift` is public. Confirmed by `gh repo view`.
- #585 (open, `needs-triage`) proposes a distinct waiting outcome for Spec runs. Q1.3 answers the label part. Confirmed by the issue.

## Research results (2026-10-27)

- A Custom process's Delivery that leaves a ready-for-review pull request removes `in-progress` and puts nothing back. Confirmed by source, `Claim::remove` in `src/claim.rs` (commit 944a9766, #1388).
- #977, #978, #981 and their Tickets #1009, #1011, #1018 and #1019 are all closed. Confirmed by `gh issue view`.

## Research results (2026-11-09)

- The **Failure limit** replaced the **Claim limit**. A Failed ticket run or Failed spec run that keeps its Claim gets `agent-failed`, next to `in-progress`, and Pickup counts open `agent-failed` issues against `pickup.failure_limit`, default 3. A ready or merged pull request never counts. Confirmed by `GLOSSARY.md` (Failure limit), `src/claim.rs:30-40` and `docs/grill-with-docs/1532-keep-working-the-backlog.md` (Q1.1, Q2.5, Q3.3).
- Pickup takes `agent-failed` off an open issue that has `ready-for-agent`, or that has neither `in-progress` nor `needs-triage`. Confirmed by source, `src/pickup.rs:391-423`.
- The **Command limit** now caps live Commands per repository per machine, separately from failures. Confirmed by `GLOSSARY.md`.
- A Spec run whose undone Tickets are gated or blocked fails with "Tickets not done: …", so it gets `agent-failed`. Confirmed by source, `src/spec_run.rs:457` and its tests.
- **Review runs** and **Fix runs** put `in-progress` on a pull request only while they are live, and Pass selection skips `in-progress` pull requests. The Sweep takes `in-progress` off closed pull requests. Confirmed by `GLOSSARY.md` (Review run, Fix run, Sweep).
- One Console now shows, and can stop and dismiss, the Commands of every machine in its **Fleet**, Needs you included (#1769). Confirmed by `GLOSSARY.md` (Console, Fleet, Needs you).
- thirdshift already posts issue comments: Base branch CI reports (`src/github/base_ci.rs:571`) and Triage run links (`src/triage.rs:429`, marked "This comment was generated by AI."). Confirmed by source.
- Sessions are given an issue's comments by Trusted accounts only. Confirmed by `GLOSSARY.md` (Trusted account).
- Open `in-progress` issues on 2026-10-27: #1446, #1443, #1252. Not re-checked.
- Spec #1695 (`in-progress`) and its Ticket #1891 (`ready-for-agent`) give Triage runs a Claim. A failure replaces `in-progress` with `ready-for-human` plus `agent-failed`. A Stop leaves `ready-for-human`, or `needs-triage` with `--requeue`. The Sweep clears `agent-failed` once the Day shift removes `ready-for-human`. Confirmed by the issues' bodies. No pull request yet.
- #1583 (`ready-for-human`) asks whether a failed Custom process should get `agent-failed`. Today it does, through the same `Claim::end` path, and it keeps `in-progress`. Confirmed by the issue.
- #585 has two new cases: tourbot#880, whose only open Ticket was `needs-info`, and tourbot#1689, with one `ready-for-human` Ticket and one in another repository. Both runs failed with no Ticket failing. Confirmed by #585's comments.
- #1857 (closed): a Claim ending counts only same-repository pull requests as published, so a cross-repository one no longer stops a release. It doesn't affect these questions. Confirmed by the issue.

## Waiting on research

- Posting to GitHub was blocked by a permission check, so these wait for you: the comments on #1891 and #1695 (Q2.1), the answer to #1583 and closing it (Q2.2), and a new `needs-triage` issue for the release-retry gap (Q2.5). The drafts are in `/private/tmp/claude-501/-Users-jacob--herdr-worktrees-thirdshift-1165-triage/eb97d8f7-9a68-4adc-a09b-d3d2898f4fce/scratchpad/github-drafts.md`.
