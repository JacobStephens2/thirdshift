# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

Edit the right-hand column to match whatever vocabulary you actually use.

## `ready-for-agent` on a Spec

On a **Spec**, `ready-for-agent` says its **Tickets** are published: every Ticket is a sub-issue of the Spec and every blocking link is in place. Until then the Spec is labelled `needs-triage`, because a `ready-for-agent` Spec with no Tickets yet looks exactly like a standalone Ticket, and a **Pickup run** would start a plain Ticket run on it.

### Day shift publication

This repository-owned procedure overrides the imported Day shift `to-spec` and `to-tickets` skills' readiness and parent-modification defaults. Keep it here so updating those skills preserves the procedure.

1. Before publishing with `to-spec`, establish whether Tickets will follow from the agreed scope; ask the user if that is unresolved. Publish a multi-session Spec with `needs-triage`. A single session of work is a standalone Ticket and can be labelled `ready-for-agent` when fully specified.
2. With `to-tickets`, ensure the Spec is labelled `needs-triage`, with `ready-for-agent` absent, before publishing its approved Tickets. Publish the Tickets, attach each as a sub-issue of the Spec, and create every blocking link. Keep the Spec `needs-triage` while publication is incomplete, including after a partial failure.
3. Read back the published Tickets, sub-issue relationships and blocking links. Once every approved Ticket and edge is present, replace the Spec's `needs-triage` with `ready-for-agent` as the final publication step. Preserve its other labels, body and open state. This label change is the exception to `to-tickets`' parent-modification restriction.

Factory skills named `thirdshift-to-spec` and `thirdshift-to-tickets` follow their Session prompt instead: thirdshift owns the top issue's readiness after validating the published plan.

### Split a Spec at a release

A Spec's work reaches the **Base branch** only through its **Spec PR**, once every Ticket is done, so no release, deployment or host install can contain an earlier Ticket's work while the Spec is still running. A Ticket that needs one, to run, verify or migrate anything, blocks its own Spec from the start. When shaping the scope with `to-spec` and again before publishing Tickets with `to-tickets`, check for such a Ticket. Where one would need a release of earlier Tickets in the same Spec, split the Spec at that point instead:

1. Spec A: the work that has to ship first, published with its Tickets as above.
2. Release step: a standalone `ready-for-human` issue for the release, deployment or host install and any Day shift step after it, blocked by Spec A.
3. Spec B: the work that comes after, blocked by the release step, published with its Tickets as above. A **Pickup run** does not take it while the release step is open.

#700 shows why. Its Ticket #994 (`ready-for-human`) said to run a conversion "once a release with #993 is installed on each host", and #997 was blocked by #994. #993 had landed on the **Spec branch**, so only merging the Spec PR could release it, and the Spec PR was waiting on #994. The **Spec run** a Pickup run started failed after 7h 37m with `Tickets not done: #994, #995, #996, #997`, and PR #1049 broke the deadlock by hand, landing the first 12 Tickets early without the **Spec review**. Split at #994, Spec A would have held Session traces and the conversion's code, the release step installing the release and running the conversion, and Spec B the switch and the span tree.

The Factory `thirdshift-to-tickets` skill creates and modifies no Spec, so it publishes only Spec A's Tickets and reports the rest in its final message as needing a separate Spec, for the Day shift to split.

## `in-progress` is not a triage role

`in-progress` is thirdshift's own label for a **Claim**, defined in `GLOSSARY.md`. thirdshift applies and removes it. It is not a sixth triage role: no skill applies it, and it has no row in the table above.
