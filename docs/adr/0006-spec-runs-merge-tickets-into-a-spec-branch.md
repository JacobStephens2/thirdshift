# A Spec run merges each Ticket into a Spec branch, running Tickets in parallel as separate thirdshift processes

A **Spec run** works through a **Spec**'s **Tickets** in the order their GitHub "blocked by" links allow, running up to `spec.parallel` (default 3) at once. Each Ticket's **Run** is a **Merge run** into the **Spec branch**, with no human review, so the next Tickets can start on top of it. The day shift reviews one **Spec PR** from the Spec branch into the **Base branch**, after a **Spec review** and the usual Repair loop; the command's `merge` ask applies only to that PR. Each Ticket's Run is a child `thirdshift` process, and the Spec run is a deterministic loop over the graph, the children's exit codes and their PR URLs. Agents run only in the Ticket sessions, their Repairs, the Spec review and the Spec PR's Repairs.

## Considered Options

- **Each Ticket a Merge run straight into the Base branch.** Rejected: the Spec would ship in pieces with no human ever looking at it, and a Spec left half done would leave `main` half done.
- **Each Ticket a Run left ready for review.** Rejected: the graph would stop at every Ticket until a human merged it, so a Spec could not be worked unattended.
- **Tickets one at a time.** Rejected for speed. Tickets running at once can collide, but a later Run already merges a Base branch that moved and hands a conflict to a conflict **Repair**, so a collision costs Repairs, not correctness.
- **Ticket Runs as threads in the Spec run's process.** Rejected: interrupt handling and progress output are process-wide, and a child process keeps each Run exactly what a standalone Run is, including its Failed run path, with a crash in one unable to take down its siblings.
- **An agent driving the whole Spec, as the `implement-spec` skill does.** Rejected: which Ticket is ready, what landed and what failed are facts thirdshift can read from GitHub and exit codes, so an agent's judgement is not needed for them.

## Consequences

- A Ticket closes when it lands on the Spec branch, before its work reaches the Base branch. Rejecting the Spec PR means reopening its Tickets by hand.
- A failed Ticket stops only the Tickets it blocks; the Spec run takes every other Ticket it can reach, then ends as a **Failed spec run** with a draft Spec PR. Rerunning the Spec picks up the Spec branch and each failed Ticket's Issue branch as **Continuations**.
- A repository whose CI does not run on pull requests into branches other than the default leaves Ticket PRs with no CI, which a Merge run treats as green.
