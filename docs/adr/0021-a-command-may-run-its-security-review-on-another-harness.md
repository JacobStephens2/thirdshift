# A Command may run its Security review on another Harness

Until now every session of a **Command** ran on one **Harness**: its `harness` word, else `harness.default`, else Claude Code. `[security] harness` chose only a **Security run**'s. So a Security review ran on whatever Harness its Command chose, and when that Harness's Model refused security work, nothing could get the review done unattended. Spec run #776 lost its Security review this way: its Codex session was flagged as a possible cybersecurity risk while it read the `thirdshift-security-audit` skill, the refusal held the result as an unaddressed Security finding, and a Spec adding accounts, login codes and a `sudo` install script reached the Day shift unreviewed (#910).

Now a Security review runs on `security.harness` when that is set, with that Harness's own `[harness.<name>]` Model and Effort, in a Run, on a Spec run's Spec PR and in a Base fix. The command's `harness`, `model` and `effort` words don't change it: they choose the Command's Harness, and the User config's security choice wins for security sessions. With `security.harness` blank or missing, the review runs on the Command's Harness, as before. Every other session, Repairs and Resumes after the review included, keeps the Command's Harness.

This is the first step towards choosing a Harness per session. The direction is that a Command may use more than one Harness, each session on the one suited to it.

## Considered Options

- **Retry once on a fallback Harness after a refusal.** Initially rejected: it adds retry logic, and still spends a refused session first. [#975](https://github.com/JacobStephens2/thirdshift/issues/975) revises this for a Harness error before any turns or tokens: check sign-in before work, fall back to the configured default Harness if the review's check fails, and retry once on that default when an unchecked startup error surfaces at review time. Refusals and failures after work starts still receive no fallback.
- **A command word for the review's Harness.** Left out: the choice is about this machine's Harnesses and what each will do, which is the User config's to say. A Continuation reads the same User config, so the Recovery command needs no new word to reproduce the choice.
- **A general per-session Harness map.** Left for later: only the Security review has needed one so far.

## Consequences

- A Command checks its own Harness and its Security review's before work, including non-billed sign-in status where the CLI supports it. If a non-default review Harness can't run, the review uses `harness.default` (Claude Code when unset) with its own configured Model and Effort, warning with both Harnesses and the reason. If that default can't run, preflight fails. A Pass checks the review only when it may dispatch a Run. A Spec run hands the settled choice and the default fallback to every Ticket, and reviews each Ticket and the Spec PR, as #975's addendum requires.
- Both Harnesses' Factory skills are linked into the worktree.
- The review's Session trace, its `.models.json`, the Command log's line for it and the Run notification's session models name the Harness it ran on. The `Built with` line in the pull request and Run notification still names the Command's Harness.
