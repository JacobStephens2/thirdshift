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

On a **Spec**, `ready-for-agent` says its **Tickets** are published: every Ticket is a sub-issue of the Spec and every blocking link is in place. A Spec with no Tickets yet looks exactly like a standalone Ticket, so a **Pickup run** would start a plain Run on it.

- `/to-spec` labels a Spec that Tickets will follow `needs-triage`, and one that stands alone as one session of work `ready-for-agent`.
- `/to-tickets`, given a Spec, swaps the Spec's `needs-triage` for `ready-for-agent` as its last step.

Don't label a Spec `ready-for-agent` by hand before its Tickets exist.

## `in-progress` is not a triage role

`in-progress` is thirdshift's own label for a **Claim**: the mark that the factory has taken an issue. thirdshift adds it, in place of `ready-for-agent`, when a Run or a Spec run starts on the issue, and takes it off once the issue is closed. It is not a sixth triage role: no skill applies it, and it has no row in the table above.
