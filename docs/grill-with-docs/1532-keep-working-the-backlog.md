# Grilling: #1532, keep working Ready issues without a cap

Issue: https://github.com/JacobStephens2/thirdshift/issues/1532
Spec: https://github.com/JacobStephens2/thirdshift/issues/1564

## Round 1 (answered)

Facts behind this round are under "Research results". In short: right now no cap is stopping thirdshift. The four `ready-for-agent` issues are Spec #1477, which is blocked by #1252, and its three Tickets. Three gates can hold the backlog back, and the issue's "or anything" could mean any of them. So each one gets its own question below. In the glossary's words, "in-progress issues" are **Claims**, and "unmerged PRs" count only because a Claim stays on while its pull request is open or its Ticket run failed.

**Q1.1 - The Claim limit.** The glossary gives the **Claim limit** two jobs: stop a broken **Base branch** from failing every **Ready issue** in turn, and stop pull requests from piling up unreviewed. Since ADR 0008, a broken Base branch already turns each Ticket run into a quick **Failed run** naming `fix <base> first`, or a **Base fix** when that is allowed. That makes the first job cheaper, but it still burns a Claim, and some sessions, on each Ready issue. The options:
- (a) Remove the Claim limit. Pickup takes Ready issues until there are none left.
- (b) Keep `pickup.limit`, but make "no limit" the shipped default, so a cap is opt-in.
- (c) Count only Claims the factory failed on: issues whose last Ticket run or Spec run ended failed and is still waiting for the **Day shift**. A pull request that is ready for review, or one that was merged, never holds pickup back. The default stays small, say 3, since three failures in a row on one repository usually mean a broken Base branch or a broken harness, not three bad issues.

*Recommended:* (c). It drops the "unmerged PRs" cap the issue complains about, and keeps the one protection that still pays for itself: a run of failures stops the factory from spending sessions on the whole backlog. (a) loses that protection. (b) keeps the cap that exists today, only switched off by default.

*Answer:* agree, and I'm wondering if pickup.limit would benefit from a rename, as it doesn't intuitively say to me that it is about a limit related to the number of failed runs from the factory. 

**Q1.2 - One Pass at a time on each machine.** A **Pickup run** is skipped while another **Pass** on the same repository is still running on the machine, including the Ticket run or Spec run it started. So each host works one Ready issue at a time on thirdshift, and the two hosts give two at most, whatever the Claim limit is. Is that part of #1532?
- (a) Out of scope. #1532 is about caps on how many issues sit waiting. How many run at once on a machine is its own design (worktrees, harness quota, resource limits), and would get its own issue.
- (b) In scope. Let Pickup runs on the same repository overlap on one machine, up to a set number, the way `spec.parallel` does for Tickets.

*Recommended:* (a). The issue names in-progress issues and unmerged PRs, and both of those are Claims. Running issues side by side touches the lock, the reservation in ADR 0017, and resource limits, so it needs its own grilling.

*Answer:* Thinking about the language here a little bit, I think that why don't we say a pickup run is skipped while another run on the same repository is still running on the machine? What distinction do we make between run and pass? 

My answer here is b, I want to enable running multiple pickup runs and generally different kinds of runs run by cronjobs perhaps in parallel on the server. I want that limit to be user configurable.

**Q1.3 - Blockers and readiness.** Today the backlog is stopped by readiness, not by a cap: #1477 is blocked by #1252, whose Spec PR #1279 is still a draft, and #1478-#1480 are Tickets reached only through #1477. Should "or anything" loosen any **Ready issue** rule?
- (a) No. A blocker is something the Day shift wrote down on purpose ("#1279 changes the same Verdict code"), so starting past it would build on code that is about to change. The readiness rules stay as they are.
- (b) Yes. Pick up a blocked issue anyway once its blocker carries a Claim, and stack its branch on the blocker's branch.

*Recommended:* (a). Stacking branches is a large new idea, and #1477's body itself explains why it has to wait. If idle hosts are the real pain, the fix is unblocking #1252, which is a Day-shift action: #1279 is a draft that is waiting on you.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - Run, Pass and Command, and what the new limit counts.** You asked what separates a run from a Pass. In the glossary, "Run" on its own is no longer a term: it is the former name of **Ticket run**, on that entry's _Avoid_ list. The umbrella word is **Command**: one invocation of thirdshift that does factory work, of any kind. A **Pass** is a Command started with no **Issue URL**: an Architect run, a Pickup run, a Security run, or a Custom process with no issue. The difference that matters is that a Pass finds its own work and may be skipped. A Command a person starts on an Issue URL is never skipped. So "skipped while another run is still running" leaves open what counts. Following from Q1.2 (b), what should the new limit count?
- (a) Passes only. Today's lock lets N through instead of 1. A Ticket run you start by hand is invisible to it.
- (b) Every live Command on the repository on the machine, and any Command, hand-started included, waits or is refused at N.
- (c) Every live Command on the repository on the machine counts, but only a Pass is ever skipped for it. A Command you type is never refused, but it does fill a slot, so cron backs off while you work.

*Recommended:* (c). In the glossary's words: "a Pass is skipped while N Commands on the same repository are running on the machine". It keeps the rule that a typed command always runs, and stops cron from stacking N more on top of your own work. Whether hand-started Ticket runs take any lock today is under "Waiting on research", but that doesn't change which option you want.

*Answer:* agreed

**Q2.2 - A name for the Q1.1 (c) count.** `pickup.limit` and the **Claim limit** name the old idea: every Claim counts. Under Q1.1 (c), only Claims the factory failed on count. I propose a new term and a new setting:
- **Failure limit**: the number of open issues held by a failed Ticket run or Spec run, still waiting for the **Day shift**, at which a **Pickup run** takes no more.
- Setting `pickup.failures = 3`, commented "how many open issues the factory failed on stop a Pickup run taking another".
- An old `pickup.limit` line is refused with a message naming `pickup.failures`. Mapping it silently would be wrong, because your `limit = 10` meant 10 Claims of any kind, not 10 failures.

Alternatives: `pickup.failure_limit`, or `pickup.max_failed`. **Claim limit** goes on the new entry's _Avoid_ list as its former name.

*Recommended:* the three bullets above. `[pickup] failures = 3` reads as a count under its section, the way `spec.parallel = 3` does, and refusing the old key tells you its meaning changed.

*Answer:* agreed on the terms, and I want pickup.failure_limit for the setting - I think that is more self-evident than pickup.failures

**Q2.3 - Whose failures count.** A failed Claim can come from a Ticket run or Spec run that a Pickup run dispatched, one you started by hand on an Issue URL, an Architect run's plan, or a Security run's fix. Today a failed Security fix keeps its Claim on purpose, and no later run retries it. What does the Failure limit count?
- (a) Every open issue held by a failed Ticket run or Spec run, whatever started it.
- (b) Only those a Pickup run dispatched.

*Recommended:* (a). A broken Base branch or harness fails a hand-started run the same way, and that is the signal the limit exists to catch. Each one is also an issue waiting on you, which is the real cost.

*Answer:* agreed

**Q2.4 - Which kinds of Pass may overlap.** Added after the research came back (see "Research results"). Pickup runs are nearly ready to overlap: the reservation on origin, worktree creation and Base fixes already have their own locks. Architect runs and Security runs are not. They share one fixed review worktree, `<repo>-architect`, and the next run's stale disposal deletes a live one mid-session. They check "no open plan" or "base unchanged since the last audit", then publish, with no atomic step in between, so two of the same kind publish duplicate plans or findings. They also mark an issue `ready-for-agent` and then dispatch it themselves with no reservation, so an overlapping Pickup run could take the same issue. Your Q1.2 answer asked for "different kinds of runs run by cronjobs" in parallel. The options:
- (a) Only Pickup runs overlap, up to N. Architect runs, Security runs and issue-less Custom processes stay alone on the repository, and wait for every live Command there to end, as they do today.
- (b) Up to N of any kind, each kind still alone with itself. One Architect run, one Security run and some Pickup runs can run side by side, but never two Architect runs or two Security runs. That needs a review worktree per kind, and the Architect and Security dispatches taking the same origin reservation a Pickup run takes.
- (c) Up to N of any kind, two of a kind included. On top of (b), that needs each review worktree named per Command, a liveness check in stale disposal, and atomic publication of plans and findings.

*Recommended:* (b). It gives you the mixed cron load you asked for. Two Architect runs on one repository at once would only race each other for the same top recommendation, so (c) costs the most for the least. (a) is the smallest first Ticket if you would rather ship Pickup overlap before the rest.

*Answer:* agreed

**Q2.5 - How a Pickup run sees a failed Claim.** GitHub can't tell one today. A failed Ticket run leaves `in-progress`, no label and no comment, and turns its pull request back into a draft, which is also what a live run's pull request looks like. Only a `thirdshift: failed ticket run (<reason>)` commit at the branch head tells them apart. A Self-merge held by a policy, security or merge hold leaves the pull request ready, so it looks the same as one ready for review. A failed Security fix says it failed only in the private advisory. The other host's history isn't readable from this one. The options:
- (a) A label. When a Ticket run or Spec run fails and keeps its Claim, thirdshift adds `agent-failed`. The **Sweep** takes it off closed issues, and it comes off when the Day shift hands the issue back with `ready-for-agent`. The Failure limit counts open issues that carry it.
- (b) Infer it: count Claimed issues whose Issue branch head is a failure commit, plus open `security-fix` issues with a Claim and no live run.

*Recommended:* (a). Both hosts and you see the same thing on GitHub. A label is one API call, not a branch read per Claim, and it keeps working if you push on top of a failed branch. A hold that left a ready pull request gets no label: it is waiting on review, not a sign of a broken Base branch or harness.

*Answer:* agreed

## Settled by earlier answers (shout if any is wrong)

- An open pull request, ready for review or merged, never holds a Pickup run back. Only Claims the factory failed on count (Q1.1).
- The glossary's **Claim limit** entry gets replaced. Its _Avoid_ note "concurrency (it counts issues waiting on the Day shift, not Ticket runs in flight)" stays true of the new count, and concurrency becomes a separate setting (Q1.1, Q1.2).
- How many Commands run at once on a repository on one machine becomes a user setting, and #1532 owns it (Q1.2).
- The Ready issue rules don't change. #1477 waits for #1252, and what frees the backlog today is #1279 (Q1.3).
- #1532 now holds two separate changes, the Failure limit and running Passes side by side. It will likely become a Spec with at least those two Tickets.
- A Pass is skipped while N Commands on the same repository are running on the machine. Hand-started Commands count toward N but are never refused (Q2.1). Today a Ticket run started on an Issue URL takes no lock at all, so it would need to register itself in whatever replaces the Pass lock.
- When two Pickup runs pick the same Ready issue, the one that loses the reservation moves on to the next Ready issue, instead of being skipped as it is today. Without that, N overlapping Pickup runs mostly skip each other (Q1.2).
- Overlapping Pickup runs each count before either one makes a Claim, so the Failure limit can be passed by up to N-1. That is accepted: it is a brake, not an exact count (Q1.1, Q1.2).

- The term is **Failure limit**, with **Claim limit** as its former name. `GLOSSARY.md` is updated (Q2.2). The setting is `pickup.failure_limit`, default 3 (Q2.2, Q1.1).
- A **Stop** never adds `agent-failed`: the person who stopped it has already acted, and the issue goes to `needs-triage` or back in line (Q2.5).
- Q2.4 (b) splits into Tickets: Pickup runs overlapping, then a review worktree per kind plus an origin reservation for Architect and Security dispatches.

- N is the **Command limit**: `[pass] command_limit`, per repository on one machine, default 1. Load across repositories stays with `[resources]` (Q3.1, Q3.2). `GLOSSARY.md` has the entry, and the Architect run, Pickup run, Security run and Pass entries are now skipped "at its Command limit".
- `agent-failed` comes off with `ready-for-agent` put back, with `in-progress` taken off, or with the issue closed (Q3.3).
- An old `pickup.limit` is accepted and ignored, with a warning naming `pickup.failure_limit` on stderr and once in the Activity log. The warning is removed a few releases later (Q3.4).
- The User config comment, `docs/guide.md`, and the Setup defaults change with the settings.

- #1532 becomes a Spec with three Tickets: the Failure limit, the Command limit for Pickup runs, and mixed Passes (blocked by the second). It is written with the to-spec and to-tickets skills (Q4.1).
- One ADR goes with the Failure limit Ticket and supersedes ADR 0009's "one unattended run per repository at a time on a machine" (Q4.2).

## Round 3 (answered)

**Q3.1 - Where N applies, and its name.** Q2.1 settled what N counts: live Commands on a repository on the machine, with only Passes ever skipped. Today no cap counts Commands across repositories either. Your crontab starts Pickup runs on 22 repositories every 10 minutes, so this machine can already run 22 at once, held back only by `[resources]` memory and load waits, which are off by default. The options:
- (a) Per repository only: `[pass] command_limit = N`, "how many Commands running on one repository on this machine stop a Pass starting another". Load across repositories stays with `[resources]`, as it is today.
- (b) Per machine only: `[pass] machine_limit = N`, counting live Commands across every repository.
- (c) Both settings.

*Recommended:* (a). It is the lock that exists today, made N wide, so #1532 changes one thing. A cap across all repositories is a separate problem that applies even at N = 1, and it can be its own issue with its own grilling (harness quota, fairness between repositories). The name follows your `failure_limit`, a noun plus `_limit`, and `[pass]` says that it skips only Passes.

*Answer:* agree

**Q3.2 - The shipped default for N.** The title of #1532 asks for a default that "just keeps working" Ready issues. One Command at a time already does that, just one issue after another. More than one costs harness quota and machine size that vary from user to user.
- (a) 1, today's behaviour. You set your own N.
- (b) 2.
- (c) 3, matching `spec.parallel`.

*Recommended:* (a). Under Q1.1 the default already stops being held back by pull requests waiting for review, which is what #1532 complained about. Running in parallel is a choice about the machine and the harness plan, and only the user can make it. The guide and the User config comment say how to raise it.

*Answer:* agree

**Q3.3 - What takes `agent-failed` off.** Q2.5 settled that thirdshift adds it, and that the Sweep takes it off closed issues. That leaves open issues:
- (a) Only these take it off: the issue's next Claim (you handed it back, and the factory took it again), or you removing it by hand. Handing back with `ready-for-agent` keeps it counting until the issue is taken again.
- (b) thirdshift takes it off as soon as you put `ready-for-agent` back.
- (c) It also comes off when you take `in-progress` off to take the issue over yourself.

*Recommended:* (b) plus (c). Either relabel is you acting on the failure, which is what the Failure limit waits for. Under (a), handing back three failures could still hold pickup until it took one of them. A Pickup run makes the check when it counts, after the Sweep: an open issue with `agent-failed` and `ready-for-agent`, or with neither `in-progress` nor `needs-triage` left, has the label taken off and is not counted.

*Answer:* agreed, and what is the Sweep?

*Checked:* the **Sweep** is the cleanup each Pickup run does before it counts: it takes `in-progress` off every closed issue that still has it, such as one whose pull request you merged by hand. Under Q3.3 it takes `agent-failed` off those too. Its glossary entry now says so.

**Q3.4 - What happens to an old `pickup.limit` line.** You agreed in Q2.2 that it is refused. Checked since then: an unknown key fails loading the User config (`src/config.rs:368`), so every Command fails, on both hosts, until each config is edited. After `thirdshift update`, every cron Pass across your 22 repositories would fail with a line in `cron.log` and no Run notification. The options:
- (a) Refuse it, as agreed. The failure says to replace it with `pickup.failure_limit`.
- (b) Accept it and ignore it, with a warning that names `pickup.failure_limit`. The warning goes to stderr and, once per change, to the Activity log.
- (c) Refuse it, but have `thirdshift update` rewrite the line to `failure_limit = 3` before the new binary first runs.

*Recommended:* (b). It keeps a fleet of cron jobs running across an update, and the old value means nothing under the new rule, so ignoring it changes nothing. Refusing a key is right for a typo, but this one was valid until now. Drop the warning a few releases later, and from then on the key is just unknown.

*Answer:* agree

## Round 4 (answered)

**Q4.1 - How #1532 ships.** The settled design is three changes that can each merge on its own. I propose making #1532 a Spec with three Tickets, in this order:
1. **Failure limit.** The `agent-failed` label and its lifecycle (Q2.5, Q3.3), the count in place of the Claim limit (Q1.1, Q2.3), `pickup.failure_limit`, and the ignored `pickup.limit` (Q3.4).
2. **Command limit for Pickup runs.** `[pass] command_limit` replaces the one-wide Pass lock, a hand-started Command registers itself so it counts (Q2.1), and the loser of a reservation moves on to the next Ready issue. Architect runs, Security runs and issue-less Custom processes still wait for an empty repository.
3. **Mixed Passes.** A review worktree per kind, an origin reservation for Architect and Security dispatches, and the Architect and Security gates allowing other kinds alongside, never two of a kind (Q2.4).

Ticket 2 is blocked by nothing, and Ticket 3 is blocked by Ticket 2.

*Recommended:* as listed. Ticket 1 alone fixes what #1532's title describes. Ticket 2 is where most of the risk sits, and Ticket 3 is only safe once Ticket 2's counting exists.

*Answer:* agreed - we'll finalize with to-spec and to-tickets skill calls

**Q4.2 - An ADR.** Two choices here meet all three tests for an ADR: hard to reverse, surprising without context, and a real trade-off. I propose one ADR, "Pickup counts failures, not Claims, and Passes share a repository up to a Command limit", recording:
- why open pull requests stopped counting
- why an `agent-failed` label rather than inference from branch heads
- why the limit is per repository and not per machine
- that it supersedes ADR 0009's "one unattended run per repository at a time on a machine"

*Recommended:* yes, one ADR, written with Ticket 1 of the Spec.

*Answer:* agree

## Research results (2026-10-28)

- Right now 2 issues carry a Claim on thirdshift: #1535 (its Ticket run is live) and #1252 (Spec, PR #1279 is a draft). Confirmed with `gh issue list -l in-progress`.
- Your `~/.thirdshift/config.toml` sets `pickup.limit = 10`, `merge.always = true`, `base.fix = true` and `pickup.wait_minutes = 1`. The shipped default is 3, and `limit = 0` is rejected ("must be a whole number from 1 up"). Confirmed in `src/config.rs:59-61,143,278-279,1619-1620`.
- The 13:15 Pickup run took #1535, and logged `#1477 blocked by #1252` and `#1478/#1479/#1480 is a Ticket of #1477, which is not ready`. Confirmed in `~/.thirdshift/logs/cron.log`.
- `cron.log` has 5,121 lines of `at the Claim limit on JacobStephens2/thirdshift`, all at `pickup.limit is 3`, from before you raised it to 10. The other host may still run with 3; its config isn't readable from here.
- On this machine, thirdshift's Pickup runs every odd minute (`1-59/2` in crontab). While a Pickup run's Ticket run is live, each one after it is skipped for the lock.

- Six things rely on the Pass lock being exclusive (source read by a sub-agent):
  - The stale disposal of review worktrees (`src/worktree/review_recovery.rs:211-262`). It has no liveness check, and `<repo>-architect` is shared by the Architect review and the Security audit, fix and close (`src/worktree.rs:481-497`).
  - The Security run's rule that a pending fix means a failed one (`src/github/advisories/description.rs:139-149`).
  - The Claim limit count (`src/pickup.rs:259-282`).
  - Pickup's choice of Ready issue. The loser of a reservation is skipped as `IssueUnavailable` and doesn't try the next issue (`src/pickup.rs:221-233`).
  - The window between an Architect or Security run marking its issue Ready and dispatching it, with no reservation (`src/architect.rs:543-550`, `src/security/fixing/publication.rs:115`).
  - The Architect and Security gates, which don't see work already Claimed (`src/architect.rs:438-458`, `src/security.rs:184-192`).
- Already safe without the Pass lock:
  - worktree creation and removal (`thirdshift-worktrees.lock`)
  - fetches (`thirdshift-worktree-refs.lock`)
  - `launch.pull` (`merge --ff-only`; a collision is only a warning)
  - Base fix sharing (`thirdshift-base-fix.lock`)
  - origin reservations
  - Activity log appends (at worst a skip line is duplicated)
  - Command records and logs
  - per-worktree `target/` dirs
- A Ticket run or Spec run started on an Issue URL takes no Pass lock, and doesn't exclude Passes or other such runs (`src/launch.rs:115`, `src/claim.rs:46-52`).
- `[resources]` (`min_available_memory_mb`, `max_load_per_cpu_percent`, both off by default) waits before each session for memory and load, machine-wide. It doesn't count Commands, and it releases every waiting session at once (`src/resources.rs`, `src/session.rs:204,249`).
- On GitHub, a failed Claim and a live one look alike: `in-progress` and a draft pull request. Only a `thirdshift: failed ticket run (<reason>)` commit at the branch head tells them apart (`src/worktree/ownership.rs:128-140`). A Self-merge held by a PolicyRefusal, SecurityHold or MergeHold keeps the pull request ready (`src/delivery.rs:650-652`). The origin reservation exists only while a Pickup-dispatched run is live (`src/pickup.rs:221-224`).

## Waiting on research

- Nothing.
