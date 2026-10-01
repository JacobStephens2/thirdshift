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

## `in-progress` is not a triage role

`in-progress` is thirdshift's own label for a **Claim**, defined in `CONTEXT.md`. thirdshift applies and removes it. It is not a sixth triage role: no skill applies it, and it has no row in the table above.
