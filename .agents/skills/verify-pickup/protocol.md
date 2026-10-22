# Pickup model boundary

`tests/pickup_model.rs` implements Stateright's public `Model` interface. It is a
model of the protocol, independent of the Rust product implementation; source
drift must be checked even when it passes. This is bounded exploration, not an
unbounded proof for every host count.

## Bounds and assumptions

- Three independent hosts compete for one initially Ready, open issue. `Pickup`
  takes a host-count parameter; the checked bound is three. Each host makes one
  selection and one reservation attempt. A later host can select again after a
  failed winner restores Ready or the Day shift hands it back. There are at most
  three Claim applications. No clocks, settling delays or host identities elect
  the winner; tokens are unique per attempt.
- GitHub labels and complete, ordered Claim history are linearizable reads and
  writes. The bounded history fits one page; each `candidate` query is one
  request. We do not establish GitHub's consistency or multi-page snapshot
  behavior. A PUT atomically swaps Ready/Claim labels, as `set_labels` does.
  Count applications only when `in-progress` was absent, including idempotent
  writes by broken competing owners.
- Every other Ready gate is fixed favorable: trusted, settled, unblocked, no
  parent, no disqualifying labels; a Spec has a Startable Ticket. Work published
  on origin is abstracted into the Run ending: keep the Claim, or fail with
  nothing on origin and restore its entry labels. Unrelated labels and read-only
  checks are projected out. A bug Run may instead hand reproduction back for
  information, removing Ready and Claim without requeueing. `needs-info` itself
  is projected out: Ready stays absent until a later Day shift hand-back.
  Both Run kinds share dispatch and Claim handling.
- Crashes may occur at every live phase and never resume that attempt. A crashed
  owner's ref remains. The Day shift confirms death, observes its token, then
  deletes with that token as the lease. Hand-back follows confirmation that all
  dispatched work has stopped; it atomically removes Claim and puts Ready back.
  The model does not permit deleting a live owner's reservation.
- Reads succeed, except that failure before the Claim may end the Run without a
  write. Lost responses after a committed request have the same retained remote
  state as crashing immediately afterward. Local control, dispatch, completion
  and death are separate markers; they are not remote atomicity assumptions.

## Actions → code requests

Each external action is one request; intermediate local work cannot combine two
requests. Constant reads (trust, branch/PR lookup, Spec Tickets) are projected out
under the fixed gates above. The local markers retain dispatch-before-Claim.

| Model action | Product request / control |
| --- | --- |
| `ListReady` | `ready::first` → `GitHub::open_issues_labelled`: issue-list read |
| `SelectHistory` | `standing_of` → `GitHub::candidate`: history read; reject active Claim and capture count |
| `CreateReservation` | `Reservation::take`: `git push --no-verify --force-with-lease=<ref>: origin <unique-token>:<ref>`; absent ref only |
| `RefreshIssue` | `ready::refresh` → `GitHub::issue`: current issue read |
| `RefreshHistory` | `standing_of` → `candidate`: current history; active Claim and selected-count checks |
| `Dispatch` | local `pickup::run_through` → `pass::dispatch` → `run::run_to_end`; records a Run or Spec run before `claim::make` |
| `ReadClaimLabels` | `Claimed::label_in_progress` → `GitHub::issue_labels`; an existing Claim is a successful no-op |
| `PutClaimLabels` | `Edit::apply` → `set_labels`: whole-label PUT, Ready off / Claim on |
| `FailedRunReadsOrigin` | failed Run's `put_labels_back_unless_on_origin` → `branch::started`: absent origin work (branch and PR read results fixed absent) |
| `KeepClaim` | local `Claimed::end`: ready for review, or failed with origin work, keeps Claim |
| `ReadNeedsInfoLabels` | `Claim::needs_info` → `GitHub::issue_labels`: ending labels read after the session's needs-info handoff |
| `PutNeedsInfoLabels` | `Claim::needs_info` → `Edit::apply` → `set_labels`: Ready and Claim off, needs-info on |
| `ReadEndingLabels` | `put_labels_back_unless_on_origin` → `issue_labels`: current labels read |
| `PutEndingLabels` | `release_labels` → `set_labels`: restore labels changed on entry, keep others from ending read |
| `ReleaseReservation` | `Reservation::drop`: `git push --no-verify --force-with-lease=<ref>:<owner-token> origin :<ref>`; other tokens survive |
| `FailBeforeClaim` | local Run failure before label mutation; outer reservation still drops |
| `Crash` | environment process death at any phase; no Drop cleanup |
| `DayReadsReservation` | Day shift `git ls-remote --refs origin <ref>` |
| `DayDeletesStaleReservation` | Day shift token-leased deletion after confirmed owner death |
| `DayHandsBack` | Day shift label PUT after dispatched work ended |

Local commit construction is projected out: `mktree` and parentless
`commit-tree` produce unique tokens, never shared source history. Lost create
responses fail closed; they do not grant another dispatch. Reservation errors,
permission handling and warning text are not checked.

## Counterexamples and sensitivity

The literal property counts dispatches by the Claim count **at selection**, so
silently adopting a later generation cannot make a stale selection look safe.
The separate claimed-dispatch property counts only dispatches whose Claim
handling completed. It measures the two safeguards; it does not replace the
requested property.

The current literal counterexample is: host A selects and reserves a Ready
issue, refreshes, dispatches a Run, then fails before making the Claim. A releases
its reservation. Host B selects the same Ready generation, reserves, refreshes
and dispatches. No Claim application changed the generation. A crash at that
boundary followed by Day shift stale-ref deletion permits the same result.

Run the explicit safety assertion from SKILL.md to see this failure. The normal
CI check retains a positive counterexample regression so the full suite can
remain green without concealing the finding.

The two mutations affect only the model. They correspond to precise product
edits: delete `claim_generation == selected.claim_generation` in `ready::refresh`,
or replace the empty create lease in `Reservation::take` with an unconditional
force push. The model otherwise stays identical, including crashes and endings.

```bash
PICKUP_MODEL_VARIANT=no-claim-count cargo test --locked --test pickup_model assert_requested_property -- --ignored --exact --nocapture
PICKUP_MODEL_VARIANT=no-empty-lease cargo test --locked --test pickup_model assert_requested_property -- --ignored --exact --nocapture
```

Both commands must fail with a claimed-dispatch counterexample. Without the count
comparison, host B selects generation 0, waits while A dispatches, makes its Claim,
fails and restores Ready, then reserves and dispatches that stale selection.
Without the empty lease, B overwrites A's reservation; both refresh before the
Claim and dispatch. Claim edits accept an existing Claim, so both can complete
Claim handling. Neither variant changes the owner-token release lease.

## Left out

Direct Issue URL commands and other Pass kinds as writers, Worktree acquisition
and disposal, Self-merge's expected-base guard, Sweep of closed issues, Stop
label policies, Ticket concurrency inside Spec runs, arbitrary human edits,
unbounded restarts/host counts, pagination races, git/GitHub consistency, and
liveness. No product code is changed by this verification task.
