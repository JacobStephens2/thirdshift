# Pickup reserves an issue on origin before dispatch

Two machines can read the same Ready issue before either writes its Claim.
Label edits accept an existing `in-progress`, and an ordinary Issue branch
push may accept the same head or fast-forward another Run's work. Neither
elects a winner. Issue #712's two-clone regression reproduces two implement
sessions from one Ready issue, including when both hosts use the same identity.

A Pickup run creates `refs/heads/thirdshift/pickup/<issue-number>` on origin
with an empty `--force-with-lease` expectation and a unique reservation commit
containing an empty tree and no parent. It never publishes the Launch directory's
unrelated local files or history.
Only one creator succeeds. This ref is separate from every Issue branch and
Spec branch, so losing does not write either branch or a PR. After acquiring
it, Pickup refreshes the selected issue's Ready state and Spec status before
dispatch. It also compares the number of Claim applications in the complete
label history with the observation made during selection. An intervening Claim
invalidates that selection even if a failed winner restored Ready. Timeline
order handles same-second changes without comparing the hosts' clocks, and
still works with settling disabled. A new later pass can select Ready again.
Selection also rejects an active Claim in that history, even when an older
issue listing still showed Ready; it cannot adopt the winner's active generation.
A busy or stale selection skips with exit 0 and no
notification. A remote error without evidence of another owner fails closed.

The reservation spans the Run or Spec run, including failed-work preservation
and Claim ending. Release uses a lease expecting exactly the owner's commit,
and finishes after Command interruption. A changed ref is retained. An
uncertain push result or failed release names the ref and token for recovery.

## Considered Options

- Re-read a label or claim comment after writing it: still permits both hosts
  to observe their own write before the other's, and the identity can be shared.
- Use the Issue branch: ordinary pushes can both succeed, and fresh and
  continued work need the same exclusion without changing branch history.
- Expire reservations: a slow or disconnected owner may still be running.
  No timer proves that it has stopped; stale reservations need human recovery.

## Consequences

- All Pickup hosts must run a version that honors reservations. Direct Issue URL
  commands and other Pass kinds keep their existing behavior; this decision
  arbitrates competing Pickup runs, not every possible writer.
- A crashed host can block an otherwise Ready issue until the Day shift confirms
  it stopped and deletes its reservation with the observed commit as the lease.
  Read the commit message for launch path, process, start time and token.
- These auxiliary branches can appear in origin branch listings. Repository
  rules must allow their creation and deletion; permission failures stop pickup.
- Releasing the reservation does not release a Claim. Existing hand-back and
  Continuation rules still decide whether a later Pickup can take started work.
