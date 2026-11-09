# `in-progress` marks only a live Run

This amends [ADR 0026](0026-pickup-counts-failures-and-passes-share-a-repository-up-to-a-command-limit.md)'s take-over rule and its last consequence. The rest of ADR 0026 stands.

A **Claim** used to outlive its Run. A Ticket run or Spec run whose pull request was ready for review kept `in-progress`, and so did a Failed run with work on `origin`, which ADR 0026 also labelled `agent-failed`. Meanwhile a **Custom process**'s Delivery took `in-progress` off at ready for review (#1388). The issue list therefore couldn't answer "what is running?" (#1165). The Day shift relabelled stale issues by hand, and stopped trusting the label.

So `in-progress` now means a live Run, on issues as it already did on pull requests. It goes on when a Run starts and comes off when it ends, and the issue is left marked with how the Run ended:

- **Ready for review**, or a **Self-merge** held with its pull request ready: no label. The open pull request stands for the work, and Pickup still won't take the issue, because a started issue is a **Ready issue** only after a handback.
- **Failed with work on `origin`**, or a failed Security fix: `agent-failed` in place of `in-progress`, whatever kind of Command failed. The Day shift takes over a failure in one of three ways: by putting `ready-for-agent` back, by taking `agent-failed` off, or by closing the issue.
- **A Spec run that is only waiting** on gated, blocked or out-of-repository Tickets: `ready-for-agent` back on its Spec and no `agent-failed`, unless an account that is not a **Trusted account** has edited the Spec since its last Trusted labelling, in which case `needs-triage`.

Decided in `docs/grill-with-docs/1165-claim-labels.md`: Q1.2, Q1.3, Q1.5, Q2.1, Q2.2 and Q2.4.

## Considered Options

- **A new `in-review` label** for a pull request waiting for review. It was recommended while the Claim limit counted those issues to stop reviews piling up. Rejected once ADR 0026 stopped counting them: with nothing counting it, the label would repeat what the open pull request and the Inbox's Ready for review already show.
- **Keep `in-progress` beside `agent-failed`**, so "running" is `in-progress` without `agent-failed`. Rejected: a reader of GitHub, or another host, would need a two-label search to see what is live, and that was #1165's complaint.
- **Follow Spec #1695 everywhere**: a failure leaves `ready-for-human` plus `agent-failed`, and removing `ready-for-human` is the take-over. Rejected: retrying a Ticket would take two label edits, because `ready-for-human` makes an issue Unready, and `ready-for-human` would mean "human work" on some issues and "the factory failed" on others.

## Consequences

- ADR 0026's take-over signal "taking `in-progress` off, leaving no `needs-triage`" goes. Without the change, the swap would let the next Pickup run clear every failure.
- The **Sweep** also takes `in-progress` off any open issue that carries `agent-failed`. Under the new rules the two never sit together, so this both migrates old issues and stays safe forever. Issues whose pull request was ready for review under the old rules are cleared by hand once, because GitHub alone can't tell them from a live Continuation.
- A failed Triage run (#1695) also gets `ready-for-human`, so the Triage Pass passes over it, but only `agent-failed` is the failure mark.
- A failure with nothing on `origin` still releases its issue to `ready-for-agent`, and nothing brakes Pickup retaking it. That is left to its own issue.
