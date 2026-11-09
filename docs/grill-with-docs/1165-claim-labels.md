# Grilling: #1165, in-progress stays on issues whose Run has ended

Issue: https://github.com/JacobStephens2/thirdshift/issues/1165
Research: [proposed direction](https://github.com/JacobStephens2/thirdshift/issues/1165#issuecomment-6369972244), #585, #710, #833, #1583, #1695, [#1532 grill](1532-keep-working-the-backlog.md)

Paused 2026-10-23 with Round 1 unanswered. Revised in place on 2026-10-27, and twice on 2026-11-09: after the #1532 grill and Spec #1564 shipped the `agent-failed` label and the **Failure limit** in place of the **Claim limit**, and for Spec #1695, which gives Triage runs a Claim. Q1.1 is withdrawn, Q1.2–Q1.4 are rewritten for the new labels, and Q1.5 is new. `GLOSSARY.md` is unchanged so far: its **Claim**, **Failure limit**, **Sweep** and **Ready issue** entries wait on Q1.2, Q1.3 and Q1.5.

Agreed in chat on 2026-10-23, before Round 1, and where each stands now:

- A **Failed run** that leaves work on `origin` swaps `in-progress` for a new label, `run-failed`. *Now:* #1564 added `agent-failed` for this case, but next to `in-progress` rather than in its place. Q1.5 asks whether to finish the swap.
- `run-failed` does not count toward the limit. *Now:* the reverse shipped. `agent-failed` is exactly what the Failure limit counts (Q1.1, withdrawn).
- A Spec run that stops only for the Day shift gets `ready-for-human`, not a failure label. Q1.3 questions this.

## Round 1 (open)

**Q1.1 - What stops Pickup working through every Ready issue when every Run fails?** *Withdrawn 2026-11-09, settled by the #1532 grill.* Pickup now counts open `agent-failed` issues against `pickup.failure_limit`, default 3. A broken Base branch or Harness stops the line after three failures, and pull requests waiting for review never count. One gap was left open here, and it is still open. A failure with nothing on `origin` releases its Claim to `ready-for-agent` and gets no `agent-failed`, so Pickup can take the same issue again on its next pass. See "Settled by earlier answers".

**Q1.2 - What label does a Run whose pull request waits for review leave?** A Ticket run or Spec run keeps `in-progress` (`Goal::ReadyForReview` and a held **Self-merge** leave the labels alone, `src/claim.rs:349`). A **Custom process**'s Delivery removes `in-progress` and puts nothing back (`Claim::remove`, #1388). The same ending therefore leaves different labels depending on which Command ran. When this was first asked, `in-progress` on these issues did a job, because the Claim limit counted them to stop unreviewed pull requests piling up. #1532 dropped that job on purpose: "a pull request that is ready for review or merged never counts, so the factory keeps working while reviews wait." Nothing counts these issues now. Two more facts bear on this. The Console's Inbox lists them under Ready for review, across the **Fleet**. And on pull requests, `in-progress` already means a live reservation that a **Review run** or **Fix run** takes off when it ends. The options:
- (a) Keep `in-progress` on both paths, which undoes that part of #1388. The label still overstates live work.
- (b) A new `in-review` label on both paths. It counts toward nothing, so it would be for reading only.
- (c) Take `in-progress` off on both paths, as the Custom process already does. The open pull request stands for the work. Pickup still won't take the issue again, because a started issue is a **Ready issue** only after a handback.

*Recommended:* (c). With no limit counting these issues, a second label would only repeat what the open pull request and the Inbox already show. Under (c), `in-progress` means one thing on issues and on pull requests: a live Run. It also makes the Ticket and Spec paths match what #1388 shipped. An issue whose Self-merge merged but whose closing failed keeps its Claim as it does today, since that is a finish still owed, not a review.

*Answer:* 

**Q1.3 - What does a Spec run that is only waiting leave on its Spec?** This matters more now than when first asked. A Spec run whose undone Tickets are all gated or blocked ends with `Tickets not done: #22 (labelled ready-for-human)` (`src/spec_run.rs:457`). That makes it a **Failed spec run**, so it keeps its Claim and gets `agent-failed`. Three such waiting Specs would stop Pickup on the repository under the Failure limit, though nothing is broken. Pickup already passes over a Spec with no **Startable Ticket**, so a Spec labelled `ready-for-agent` whose Tickets are all gated stays put on its own, and Pickup continues it once a Ticket becomes startable. The options:
- (a) `ready-for-human` on the Spec, with no `agent-failed`, when every undone Ticket is an **Unready Ticket** or is blocked only behind ones. A Spec waiting on ordinary work elsewhere, such as a Ticket in another running Spec, gets `agent-failed`.
- (b) thirdshift puts `ready-for-agent` back on any Spec whose undone Tickets are all gated, waiting or blocked, with no `agent-failed`. The **Ready issue** rule accepts that as a handback. The Spec continues on its own once the Day shift clears a gate or a blocker closes, and only the Tickets carry `ready-for-human`.
- (c) (a) for human gates, (b) for waiting on other work.

In every option, a Spec run with a failed or interrupted Ticket gets `agent-failed`, as #585 proposes. Two more cases from #585 are already happening on tourbot. In #880, the only open Ticket was `needs-info`, and the run "failed" after 34s. In #1689, one Ticket was `ready-for-human` and one sat in another repository, so no Spec run can start it. Both count as waiting here. A Ticket outside the Spec's repository is never a **Startable Ticket**, so Pickup passes over the Spec under (b) as well. #585 keeps its other decisions: the outcome name, exit code, notification wording and mixed results. This question decides only the labels.

*Recommended:* (b), with one guard. The Day shift does one thing, clearing the Ticket's gate, not two. The `ready-for-human` list then shows exactly the work that is human: Tickets, not Specs. The guard comes from the **Trusted account** rule. A Ready issue needs its latest `ready-for-agent` labelling by a Trusted account, with no edit since by an account that is not one. If thirdshift relabelled the Spec itself, as the owner's account, it would clear an untrusted edit made in between. So thirdshift puts `ready-for-agent` back only if no such edit has happened since the Day shift's last labelling. Otherwise the Spec gets `needs-triage`. This reverses the chat answer, so say if the Spec itself should still show `ready-for-human`.

*Answer:* 

**Q1.4 - Does a Failed run post its Recovery on the issue?** **Recovery** lines are built only from factory-owned facts. They reach the command log, the Run notification if one is configured, and the Console's Needs you. Since #1769, one Console shows Needs you for every machine in its **Fleet**, which covers most of #833's "which host?" for the Day shift. On GitHub, `agent-failed` says only that a Run failed, not why or what to do next. thirdshift already posts issue comments elsewhere: Base branch CI reports, and Triage runs, which say "This comment was generated by AI." #1695 adds a lot more of them: each triaged issue gets an AI-attributed Agent Brief, Questionnaire or closure comment. A #585 comment also asks for "a comment with the cause and Recovery line" on a waiting Spec. The repository is public, and Recovery lines include the host's paths and the full continue command. A comment from the owner's account is from a Trusted account, so the next session on the issue would be given it. The options:
- (a) A comment on the issue at each Failed run, marked as generated. It gives the cause, where the Run stopped, what was kept (the branch and pull request), the host, and how to retry ("apply `ready-for-agent`"). No local paths and no continue command.
- (b) The full Recovery lines, paths and command included.
- (c) No comment. `agent-failed` sends the Day shift to the Fleet Console.

*Recommended:* (a), a little more firmly than before, now that #1695 makes AI comments on issues routine. The Console covers the Day shift, but only on machines in its Fleet. The comment serves anyone reading the issue on GitHub, and it hands the next Continuation session a factual account of where the last Run stopped. A Run that fails again posts again, which leaves a history of attempts. Pick (c) if you would rather keep factory chatter off issues.

*Answer:* 

**Q1.5 - Does a failed Run that keeps its Claim still keep `in-progress` next to `agent-failed`?** This is the heart of #1165, now that #1564 has shipped. Today a Failed run with work on `origin` leaves both labels, so `in-progress` still doesn't mean "running". The pre-Round-1 answer was to swap one for the other, but it now runs into a #1532 rule (Q3.3 there, `src/pickup.rs:423`). Pickup reads an open issue with `agent-failed` and neither `in-progress` nor `needs-triage` as the Day shift having taken it over, and takes `agent-failed` off. Swap the labels without changing that rule, and the next Pickup run would clear every failure, so the brake would never hold.

**Spec #1695 already makes this swap, for Triage runs only.** It is being built now (Ticket #1891 is `ready-for-agent`). A failed Triage run on an issue replaces `in-progress` with `ready-for-human` plus `agent-failed`. #1695 then works around the rule above by adding a second take-over signal: the Sweep clears `agent-failed` once the Day shift removes `ready-for-human`. Left alone, that gives two take-over rules depending on which Command failed. #1583 (`ready-for-human`) asks the same about failed Custom process runs, which today keep `in-progress` and get `agent-failed`, like Ticket runs. The options:
- (a) Swap, with one take-over rule for every Command that fails while holding a Claim: Ticket, Spec, Custom process and Triage runs. The failure takes `in-progress` off and leaves `agent-failed`. You take an issue over by removing `agent-failed`, putting `ready-for-agent` back, or closing it. The #1532 "took `in-progress` off" rule goes, and #1695 drops its `ready-for-human` signal. Its failed Triage run still adds `ready-for-human`, but only so the Triage Pass passes over the issue.
- (b) Swap, and follow #1695 everywhere: a failure leaves `ready-for-human` plus `agent-failed`, and removing `ready-for-human` is the take-over. To retry a Ticket, you'd then take `ready-for-human` off and put `ready-for-agent` on, two edits, because `ready-for-human` makes an issue Unready.
- (c) Keep both labels on Ticket, Spec and Custom process runs, so "running" is `in-progress` without `agent-failed`. #1695's Triage runs stay the exception.

*Recommended:* (a). That is what #1165 asks for, and with Q1.2 (c) it leaves `in-progress` on an issue only while a Run is live. A single take-over act, removing the label that names the failure, is easier to learn than one per kind of Command. It also answers #1583: a failed Custom process counts like any other failure. Under (b), `ready-for-human` would mean "human work" on some issues and "the factory failed" on others. **This is time-sensitive.** If you pick (a), #1891 should drop its `ready-for-human` take-over rule before it lands, so say so and I'll comment on #1891. The cost is a change to a rule shipped two weeks ago. Issues that already failed also need `in-progress` taken off once, which the Sweep could do.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- **#1165 still stands.** The Fleet Console's Working group shows live Runs, but only on machines in that Fleet. Anyone reading GitHub, and both hosts, see the labels.
- **Interrupted runs:** a Failed run ended by interruption, not by a Stop, with work on `origin`, is treated like any other Failed run. The glossary already counts interruption as a Failed run.
- **Failed Security fixes:** these keep their Claim even with nothing on `origin` and get `agent-failed` today. Whatever Q1.5 decides applies to them too.
- **Unchanged endings:** a Stop still leaves `needs-triage`. #1695's Triage runs are the exception: they leave `ready-for-human`, since `needs-triage` is their own queue. A bug Ticket run that can't reproduce still leaves `needs-info`.
- **Triage runs hold a Claim** under #1695, so Q1.2–Q1.5 apply to them too, and the glossary's **Claim** entry gains them.
- **A failure with nothing on `origin`** still releases to `ready-for-agent` with no `agent-failed`, so Pickup can retry it at once and nothing brakes it. That is out of scope here, and gets its own issue if you want one.
- **Pickup:** it never takes an `agent-failed` issue, because a Ready issue needs `ready-for-agent`. The Day shift retries by applying `ready-for-agent`, and Pickup then starts a **Continuation** on the kept Issue branch and pull request.
- **Label upkeep:** making a Claim takes `agent-failed` off, and the **Sweep** takes both labels off closed issues. Both already ship.
- **Tickets:** a Ticket run inside a Spec run makes no Claim, so its Spec carries the outcome.
- **Out of scope:** a Run killed outright never ends its Claim (#710).
- **Glossary:** **Claim** changes with Q1.2 and Q1.5 (it also still omits the Custom process exception from #1388). **Failure limit** changes with Q1.5's take-over rule, and **Ready issue** with Q1.3's handback.

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

- Nothing.
