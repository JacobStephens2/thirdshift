# Grilling: #1165, in-progress stays on issues whose Run has ended

Issue: https://github.com/JacobStephens2/thirdshift/issues/1165
Research: [proposed direction](https://github.com/JacobStephens2/thirdshift/issues/1165#issuecomment-6369972244), #585, #710, #833

Already agreed in chat (2026-10-23), before Round 1:

- A **Failed run** that leaves work on `origin` swaps `in-progress` for a new label, `run-failed`, instead of keeping its Claim.
- `run-failed` does not count toward `pickup.limit`. Q1.1 questions this.
- A Spec run that stops only for the Day shift gets `ready-for-human` in place of `in-progress`, not `run-failed`. Q1.3 questions this.
- Still open: Recovery on the issue (Q1.4) and Ready for review (Q1.2).

## Round 1 (open)

**Q1.1 - What stops Pickup working through every Ready issue when every Run fails?** The glossary gives the **Claim limit** two jobs: "It keeps a broken **Base branch** from failing every **Ready issue** in turn, and pull requests from piling up unreviewed." Today a Failed run keeps its Claim, so failures fill the limit and Pickup stops. With `run-failed` left out of the limit, the brake is gone. Picture a broken Base branch with `base.fix` off, or a Harness outage like today's Grok `402 Payment Required`, which failed 48 Commands on stephens.page. A Pickup run every 10 minutes would then label one Ready issue after another `run-failed`, and the Day shift would have to hand each back. The options:
- (a) Accept it. Inherited failures and the Base fix cover the broken Base branch, and Harness outages are rare.
- (b) Count `run-failed` toward the Claim limit after all, which takes back answer 1.
- (c) A separate **Failure limit** in the User config, say `pickup.failure_limit`, default 3. Pickup is skipped while the repository has that many open `run-failed` issues. Failures don't use up live slots, but they do stop the line.
- (d) Pickup is skipped after N Failed runs in a row that Pickup dispatched on the repository, until the Day shift hands one back.

*Recommended:* (c). It keeps answer 1's point, that a failure isn't a live Run, while keeping the brake. It is also easy to read: the skip line can name the `run-failed` issues holding it, as the Claim limit's names its Claims (#787). (d) catches more, but needs a way to reset that the Day shift would have to learn. Note that a failure that leaves nothing on `origin`, such as a Harness that fails at the first session, releases its Claim to `ready-for-agent` today. Pickup can take that same issue again on its next pass, and no label brakes it under any option. Is that a separate issue?

*Answer:* 

**Q1.2 - What label does a Run whose pull request waits for review leave?** Today it keeps `in-progress` (`Goal::ReadyForReview` ends the Claim with no change, in `src/claim.rs`). That is half of what made the label untrustworthy: #1043, #965 and #957 all looked live. The Claim limit's second job, stopping unreviewed pull requests from piling up, relies on these issues counting. The options:
- (a) Keep `in-progress`. The limit still works, but the label still overstates live work.
- (b) Swap it for a new label, `in-review`, which is still a Claim and still counts toward the Claim limit. `in-progress` then means a live Run and nothing else.
- (c) Take the label off and let the open pull request stand for the work. The limit then no longer brakes unreviewed pull requests.

*Recommended:* (b). It is the only option where the issue list answers "what is running?" and the Claim limit keeps both of its jobs. `in-review` mirrors `in-progress` and stays clear of the `ready-for-*` triage roles. The three issues relabelled by hand today would get `in-review`.

*Answer:* 

**Q1.3 - Which label does a Spec run that is only waiting leave on its Spec?** The agreed answer is `ready-for-human`. Two cases test it.
- **Waiting on another Spec's ordinary work.** #978's undone Tickets wait on #1014, a Ticket in #977. If #977 were still running, with nothing gated on a human, #978 would end "Tickets not done" with no failure and no human action needed. `ready-for-human` would be wrong for it.
- **Waiting on a human gate.** Pickup already passes over a Spec with no **Startable Ticket**. A Spec labelled `ready-for-agent` whose Tickets are all gated or blocked therefore stays put on its own, and Pickup continues it once a Ticket becomes startable. The `ready-for-human` work the Day shift owes is on the Tickets themselves: #1009, #1011, #1018 and #1019.

The options:
- (a) `ready-for-human` on the Spec when every undone Ticket is an **Unready Ticket**, or is blocked, in any Spec, only behind ones. A Spec waiting only on ordinary work elsewhere gets `run-failed`.
- (b) thirdshift puts `ready-for-agent` back on any Spec whose undone Tickets are all waiting, gated or blocked, and the **Ready issue** rule accepts it as a handback. The Spec continues on its own once the Day shift clears a gate or a blocker closes, and only the Tickets carry `ready-for-human`.
- (c) (a) for human gates, (b) for waiting on other work.

In every option, a Spec run with any failed Ticket gets `run-failed`, as #585 proposes.

*Recommended:* (b). The Day shift does one thing, clearing the Ticket's gate, rather than two, clearing it and then handing the Spec back. The `ready-for-human` list then shows exactly the work that is human: Tickets, not Specs. This reverses the chat answer, so say if the Spec itself should still show it. The cost: the Ready issue rule gains an exception. Today a started issue is Ready again only after a Day shift handback, and with (b) thirdshift could make a waiting Spec Ready itself.

*Answer:* 

**Q1.4 - Does a Failed run post its Recovery on the issue?** **Recovery** lines are built only from factory-owned facts. Today they reach only the command log on the host that ran the Run and, if asked for, the Run notification. A `run-failed` label on GitHub would say that a Run failed, but not why or what to do. The repository is public, and Recovery lines include the host's paths (for #977, `/home/jacob/repos/thirdshift`) and the full continue command. The options:
- (a) A comment on the issue at each Failed run: the cause, where it stopped, what was preserved (the branch and pull request), and how to retry ("apply `ready-for-agent`"). It names the host, as #833 asks, but no local paths and no continue command.
- (b) The full Recovery lines, paths and command included.
- (c) No comment. The label points the Day shift to `thirdshift status` on the hosts.

*Recommended:* (a). Handing back with `ready-for-agent` makes the host-specific continue command unnecessary, since Pickup on any host continues on the kept branch. The comment then needs only what GitHub can act on. A Run that fails again posts again, which also leaves a history of its attempts.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- **Interrupted runs:** a Failed run ended by interruption, not by a Stop, with work on `origin`, gets `run-failed` too. The glossary already counts interruption as a Failed run.
- **Failed Security fixes:** a failed Security fix gets `run-failed` even with nothing on `origin`, instead of keeping its Claim, since it already stays with the Day shift today.
- **Unchanged endings:** a Stop still leaves `needs-triage`. A Failed run with nothing on `origin` still releases its Claim to `ready-for-agent`.
- **Pickup:** it never takes a `run-failed` issue, because a Ready issue needs `ready-for-agent`. The Day shift retries by applying `ready-for-agent`, which the Ready issue rule already accepts as a handback after the last Claim ended. Pickup then starts a **Continuation** on the kept Issue branch and pull request.
- **Label upkeep:** making a Claim takes `run-failed` off, and the **Sweep** takes it off closed issues, as both do for `in-progress`.
- **Tickets:** a Ticket's Run makes no Claim, so it gets no label of its own. Its Spec carries the outcome.
- **Out of scope:** a Run killed outright never ends its Claim (#710).
- **Glossary:** **Claim**, **Claim limit**, **Sweep** and **Ready issue** in `GLOSSARY.md` change once Round 1 settles the Claim limit (Q1.1), what counts as a Claim (Q1.2) and the handback rule (Q1.3).

## Research results (2026-10-23)

- The **Claim limit**'s two stated jobs are stopping a broken Base branch failing every Ready issue in turn, and stopping unreviewed pull requests piling up. Confirmed by `GLOSSARY.md`.
- A **Spec** with no **Startable Ticket** is not a **Ready issue**, so Pickup already passes over a waiting Spec. Confirmed by `GLOSSARY.md` (Startable Ticket, from #884).
- An **Unready Ticket** carries `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. Confirmed by `GLOSSARY.md`.
- `Claim::end` leaves the labels as they are on `Goal::ReadyForReview` and on a failed Security fix, releases only when nothing is on `origin`, and removes the Claim once the issue is closed after a merge. Confirmed by source, `src/claim.rs:231`.
- Pickup counts open `in-progress` issues for the Claim limit, and the Sweep removes `in-progress` from closed issues. Confirmed by source, `src/pickup.rs:241` and `src/pickup.rs:263`.
- `JacobStephens2/thirdshift` is public. Confirmed by `gh repo view`.
- #977, #978 and #981 all ended "Tickets not done", waiting on #977's four `ready-for-human` Tickets (#1009, #1011, #1018, #1019) through #1014 and #1029. Confirmed by `thirdshift status --json` and the #977 command log on stephens.page.
- #585 (open, `needs-triage`) proposes a distinct waiting outcome for Spec runs and asks what Claim behaviour it should have. Q1.3 answers that part. Confirmed by the issue.

## Waiting on research

- Nothing.
