# Successful review acquisition authorizes stale disposal

An Architecture review may leave disposable scratch when its factory process dies, but failed acquisition can also leave work that thirdshift explicitly retained. A later Architect run must preserve that distinction. Only a successfully acquired detached Worktree with durable evidence and unchanged checkout, administrative directory and common repository identity is eligible for stale disposal. Unmarked legacy or manually created paths remain ineligible; acquisition refuses and names them without granting authority retroactively.

Eligibility uses a private versioned administrative record paired with an excluded checkout token. Both share a fresh acquisition nonce and the record identifies the captured directory instances. It records ownership rather than the acquisition commit, so detached commits and edits remain disposable. Parsing rejects ambiguous or unsupported evidence. The repository worktree lock covers stale inspection, verified removal and the next acquisition, using the same pinned ownership inspection as live cleanup.

Stale inspection precedes acquisition's fetch. Thirdshift's origin fetches and the optional Launch directory fast-forward disable automatic maintenance, including Base branch preparation before review acquisition: Git 2.55's maintenance can otherwise prune a damaged Worktree registration without the ownership inspector's authority.

Atomic publication of the complete record without replacement is the final fallible acquisition step, after ownership capture, content synchronization and the last interruption check. Until then, failed-acquisition recovery remains armed and removes only positively identified attempt-owned markers before checking whether the checkout is clean and unchanged. Process death before publication leaves an ineligible checkout. This guarantees the publication point for process death, not durability across whole-machine power loss.

## Consequences

- Failed-acquisition retention survives retries. Missing, changed or unreadable evidence stops acquisition with an actionable diagnostic before an agent session starts.
- Successful scratch can be recovered after a crash without committing, pushing or deleting an Issue branch. Removal failure retains the remaining resources.
- A live owner retains its existing pinned identity even if its scratch experiment removes the markers; restart evidence adds authority across invocations, without replacing lifetime ownership.
- Automatically deleting every review pathname would erase retained work. Refusing every stale pathname would break recovery of successfully acquired scratch. Paired instance evidence preserves both policies; no migration or sweep grants disposal authority to older artifacts.
