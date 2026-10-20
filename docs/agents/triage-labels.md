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

On a **Spec**, `ready-for-agent` says its **Tickets** are published: every Ticket is a sub-issue of the Spec and every blocking link is in place. Until then the Spec is labelled `needs-triage`, because a `ready-for-agent` Spec with no Tickets yet looks exactly like a standalone Ticket, and a **Pickup run** would start a plain Run on it.

### Day shift publication

This repository-owned procedure overrides the imported Day shift `to-spec` and `to-tickets` skills' readiness and parent-modification defaults. Keep it here so updating those skills preserves the procedure.

1. Before publishing with `to-spec`, establish whether Tickets will follow from the agreed scope; ask the user if that is unresolved. Publish a multi-session Spec with `needs-triage`. A single session of work is a standalone Ticket and can be labelled `ready-for-agent` when fully specified.
2. With `to-tickets`, ensure the Spec is labelled `needs-triage`, with `ready-for-agent` absent, before publishing its approved Tickets. Publish the Tickets, attach each as a sub-issue of the Spec, and create every blocking link. Keep the Spec `needs-triage` while publication is incomplete, including after a partial failure.
3. Read back the published Tickets, sub-issue relationships and blocking links. Once every approved Ticket and edge is present, replace the Spec's `needs-triage` with `ready-for-agent` as the final publication step. Preserve its other labels, body and open state. This label change is the exception to `to-tickets`' parent-modification restriction.

Factory skills named `thirdshift-to-spec` and `thirdshift-to-tickets` follow their Session prompt instead: thirdshift owns the top issue's readiness after validating the published plan.

## `in-progress` is not a triage role

`in-progress` is thirdshift's own label for a **Claim**, defined in `GLOSSARY.md`. thirdshift applies and removes it. It is not a sixth triage role: no skill applies it, and it has no row in the table above.
