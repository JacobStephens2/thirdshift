# A full disk is the host's failure, not the issue's

This amends [ADR 0029](0029-in-progress-marks-only-a-live-run.md)'s rule that a Run **failed with work on `origin`** gets `agent-failed` "whatever kind of Command failed", and narrows the reason [ADR 0026](0026-pickup-counts-failures-and-passes-share-a-repository-up-to-a-command-limit.md) gives the **Failure limit**. The rest of both stands.

When thirdshift.app's disk filled (#2091), every Run then on it failed: worktree creation, Session trace writes, failure commits and pushes. Continuations with work on `origin` were labelled `agent-failed`, so the Failure limit counted them, and the Day shift had to hand each issue back once space was freed (#2092). None of those failures said anything about the issue.

So a Run whose failure's cause chain is the OS's error for no space left, or says `No space left on device` as the OS and git word it, ends its **Claim** by releasing it even with work on `origin`. `in-progress` comes off, `ready-for-agent` goes back if the Claim took it off, and no `agent-failed` or Recovery comment is added. Work already pushed stays on `origin` for the next Continuation. The Command log, stderr and Recovery lines say that the host ran out of disk space and that it needs freeing. A Security fix still gets `agent-failed`, since Pickup never takes it and it waits for the Day shift either way.

Two causes never count, because their text is not the factory's own: a failed Harness Session's diagnostic, which the agent can write, and a Spec run's Tickets not done, whose causes can mix a full disk with real failures.

## Considered Options

- **Keep `agent-failed` and only reword the Recovery.** Rejected: the Failure limit would still stop the repository for a host problem, and each issue would still need a hand-back.
- **Recognise any host-level cause, such as memory or rate limits.** Left out of scope. The disk is the cause #2091 showed, and the other causes have their own waits.

## Consequences

- The Failure limit no longer brakes Pickup on a full disk. Instead, the Ready issue settling window delays the retake, and the disk limit from #2091, `resources.min_available_disk_mb`, holds each new session until space is free.
- Only the text match is shared with the agent's world, and both agent-writable causes are excluded, so an agent cannot release its own failure by printing the message.
