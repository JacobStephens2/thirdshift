# A pass needs only the margin

This supersedes [ADR 0024](0024-pass-and-fail-use-separate-levels.md)'s pass rule; its two levels, fail rule and defaults stand.

ADR 0024 passed a metric only when the lower bound at the pass level was above the margin **and** its upper bound reached zero. For identical Arms, a central interval at level L has its upper bound below zero with probability (1 − L)/2, however many Eval tasks there are. With two noisy metrics that must both pass, an **A/A Experiment** could pass at most about ((1 + L)/2)² of the time: about 72% at the 70% default. So `spec-review` could never reach #1252's target, an A/A Experiment passing at least 80% of the time, however large it grew.

[The research note](../research/non-inferiority-pass-rule.md) found no support for the clause in primary sources. An interval lying wholly between the margin and zero is declared non-inferior, and noted as statistically worse (FDA 2016, Figure 2, scenario 6; CONSORT 2012, Figure 1, scenario D). Non-inferiority tests only the margin-side bound (ICH E9 §3.3.2), and Bayesian non-inferiority uses `Pr(d > margin) > c` (FDA's 2026 draft on Bayesian methods, §III.B).

So, per metric: fail when the upper bound at the fail level is below zero. Otherwise pass when the lower bound at the pass level is above the margin. Everything else is inconclusive. The prediction of additional Eval tasks uses the same decision. A pass whose pass-level interval lies wholly below zero is reported as **worse, within the margin** in the Verdict's JSON (`worse_within_margin`), the text view and the Console; it never changes the decision.

The method is `paired-dirichlet-v3`, and the Bar records `"pass_rule": "margin"`. A saved Bar without `pass_rule` keeps ADR 0024's rule: its Experiment keeps `paired-dirichlet-v2` (or v1), its stored Bar and Verdict when read, and the same rule when resumed with the same levels.

## Considered Options

- **Keep the clause.** Rejected: it caps a true no-op's pass rate below #1252's target on `spec-review` at any size, and has no precedent in non-inferiority practice.
- **Keep the clause at a wider level of its own.** Rejected: any fixed level still caps the A/A pass rate at ((1 + L)/2)², and it would add a third level to the Bar to protect a rule the sources do not use.
- **Drop the clause and report nothing.** Rejected: a consistent small loss inside the margin is still worth seeing, so the Verdict annotates it without blocking on it.

## Consequences

- The default pass level stays at #1268's 70% floor: at today's suite sizes no pass level from 70% to 95% lets an A/A Experiment pass 80% of the time on either suite, and without the clause a higher pass level only lowers that rate. Growing a suite now reaches the target, at about 20 Eval tasks for `code-review` and about 80 for `spec-review` (see docs/guide.md).
- A Candidate that is consistently a little worse, within the margin, now passes. The annotation is how the Day shift sees it.
