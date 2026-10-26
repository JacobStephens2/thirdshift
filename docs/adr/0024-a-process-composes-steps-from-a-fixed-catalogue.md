# A Process composes Steps from a fixed catalogue, and the safety rules stay inside the Steps

A **Process** file says which **Steps** run, in what order or in parallel, in which worktrees, and on which Harness, Model and Effort. It does not say how thirdshift merges, holds a **Claim**, budgets **Repairs** or records **Security findings**. Those rules live inside catalogue Steps such as **Delivery** and **Self-merge**. A file can include those Steps, and give them settings that only tighten the User config and the command line, but it can't reorder, drop or loosen what they do inside. The catalogue limits what *thirdshift* does on a file's behalf, not what agents can do. A session Step runs any skill, unsandboxed (ADR-0013), and a command Step runs any command. The catalogue grows when a Process needs a Step it lacks, as Archon's node types did. This follows Archon's rule, "YAML coordinates. Code computes. Agents judge." (#877, `docs/grill-with-docs/877-composable-by-file.md` Q1.1 and Q4.4; `docs/research/file-defined-factories.md`).

## Considered Options

- **Everything in the file, Warp-style**: the sessions, the gates, the Repair rounds and the merge rules all declared. Rejected:
  - A file could turn a Self-merge into a merge with no checks. The rules that keep an unattended merge safe took ADR-0004, 0008, 0010, 0014 and 0020 to get right, and every file author would have to get them right again.
  - None of the tools researched lets a file trigger a merge that skips its engine's checks. Warp never merges, Archon leaves merging to the forge, and gh-aw declares only the permission to merge while its engine keeps the checks.
  - Archon's own constitution warns that such formats grow "until it becomes a bad programming language".
- **Warp-style for everything except merging**: Delivery's rounds, the Claim and the Repair budgets as plain settings. Rejected, because a file could then loosen limits the User config sets. A Custom process copied from someone else could quietly turn off a safeguard the machine set.
- **Overrides only**: built-in Commands keep their sequences, and a file replaces a session's prompt or skill. Rejected as too little: it is #118, and nothing new can be composed.

## Consequences

- Safety-critical behaviour has one implementation, in Rust, whatever Process runs it. A Built-in process and a Custom process share the same Delivery and Self-merge.
- A Process that would lose work is refused before any session starts: an **Issue branch** worktree with no Delivery at the end, or parallel Steps sharing a worktree that one of them edits.
- Moving a Built-in process such as the Ticket run into a file waits until the catalogue has Steps for its parts, such as bug reproduction, Continuation, Resume and the Repair loop.
