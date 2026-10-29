# Pickup counts failures, not Claims, and Passes share a repository up to a Command limit

This supersedes [ADR 0009](0009-the-operating-system-schedules-thirdshift.md)'s "one unattended run per repository at a time on a machine"; the rest of ADR 0009 stands.

A **Pickup run** used to stop at its Claim limit: `pickup.limit` open issues labelled `in-progress`, 3 by default. Every **Claim** counted, and a Claim stays on while its pull request waits for review. So a repository with three pull requests waiting for the **Day shift** took nothing more, however healthy the factory was (#1532). The limit had two jobs: to stop a broken **Base branch** or **Harness** from failing every **Ready issue** in turn, and to stop pull requests piling up unreviewed. ADR 0008 made the first one cheaper, but it still costs a Claim and some sessions per Ready issue. The second job is the one #1532 complained of.

So a Pickup run now counts only the issues the factory failed on: the **Failure limit**, `pickup.failure_limit`, 3 by default. When a **Failed ticket run** or a **Failed spec run** keeps its Claim, whatever started it, thirdshift labels the issue `agent-failed`. A pull request that is ready for review or merged never counts. The label comes off when the Day shift acts on the failure: by putting `ready-for-agent` back, by taking `in-progress` off, leaving no `needs-triage`, to take the issue over, or by closing it, which the **Sweep** clears. The issue's next Claim takes it off too.

How many Commands may run at once on one repository becomes a setting of its own, the **Command limit**, `[pass] command_limit`, 1 by default. A **Pass** is skipped while that many Commands on its repository are running on the machine. A Command a person starts on an **Issue URL** counts towards it but is never refused.

Decided in `docs/grill-with-docs/1532-keep-working-the-backlog.md`: Q1.1, Q1.2, Q2.1, Q2.2, Q2.3, Q2.5, Q3.1, Q3.3 and Q3.4.

## Considered Options

- **Remove the limit**, and take Ready issues until there are none. Rejected: three failures in a row on one repository usually mean a broken Base branch or Harness, not three bad issues, and without a brake the factory spends a session on every Ready issue in the backlog.
- **Keep `pickup.limit`, off by default.** Rejected: it keeps the count #1532 complained of, only switched off, and a user who switches it on gets the same stall.
- **Infer a failure from GitHub instead of labelling it**: count Claimed issues whose **Issue branch** head is a `thirdshift: failed ticket run` commit, plus open `security-fix` issues with a Claim and no live run. Rejected:
  - A failed Claim and a live one look alike on GitHub: `in-progress` and a draft pull request. Telling them apart takes a branch read per Claim on every pass.
  - A push on top of the failed branch hides the failure commit.
  - A failed Security fix says it failed only in its private advisory.
  - A label is one API call, and both hosts and the Day shift see the same thing.
- **A limit per machine** on Commands across every repository, in place of or beside the one per repository. Rejected for now: it is a separate problem that applies even at a Command limit of 1, involving harness quota and fairness between repositories. Load across repositories stays with `[resources]`.

## Consequences

- A held **Self-merge** that leaves its pull request ready gets no label: it waits on review, not on a fix. Neither does a **Stop**, whose issue the person who stopped it has already dealt with, nor a bug Ticket run that ended with `needs-info`.
- Labelling finishes after Command interruption, as Claim ending does. A failure to label is a warning naming the command to run by hand.
- A Claim whose release failed, so that thirdshift can't tell whether it stays, gets no label: its warning already asks the Day shift to release it by hand, and a label on a Claim that should have been released would hold Pickup back for nothing.
- An old `pickup.limit` is accepted and ignored, with a warning naming `pickup.failure_limit`, so cron jobs keep running across `thirdshift update`. Refusing it would fail every scheduled Pass until each User config was edited. The warning goes when the key becomes unknown, a few releases later.
- Overlapping Pickup runs each count before either makes a Claim, so the Failure limit can be passed by up to one less than the Command limit. It is a brake, not an exact count.
- Each newer Pass holds a shared lock on the older binary's exclusive Pass-lock file until its process ends. An older Pass keeps every newer Pass out too, even above a Command limit of 1: admitting one beside it would leave the newer Pass unable to hold that file after the older Pass ends. Raising the limit permits overlap between newer Passes while preserving the older binary's exclusive access (Spec #1564, story 35).
- A failed issue stays `in-progress` and keeps its draft pull request, as before; `agent-failed` is what tells it from a live one.
