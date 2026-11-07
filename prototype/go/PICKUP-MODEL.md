# Bounded Pickup model

`pickup_model_test.go` is an independent, deterministic BFS model of the Go
prototype's ADR-0017 protocol. It stores comparable states in a visited set and
FIFO queue, retaining predecessor/action links. The first counterexample is
minimal in action count; mutation tests pin its length and replay the witness.
This is bounded exploration, not a proof for arbitrary host counts or GitHub
consistency. No external model checker or Go dependency is required.

## Bounds and assumptions

- Three hosts, one initially open Ready issue, one attempt per host, at most
  three Claim applications. Each selection records the Claim count as its Ready
  generation. Later hosts may select after a failed winner or Day shift hand-back.
- Git ref creates and owner-leased deletes are atomic. Tokens are unique per
  attempt; an existing reservation is never adopted or expired. The Day shift
  observes the token and confirms the owner crashed before deleting it.
- Issue labels and complete, ordered history are linearizable snapshots. History
  pagination/snapshot consistency, network failures, permissions and trust are
  outside the model. CLI tests cover pagination, trust and remote errors. All
  other Ready gates are fixed favorable, for a Ticket (Spec runs are unsupported).
- A successful label edit atomically adds Claim/removes Ready. Applying Claim
  when already present adds no event. Distinct reads, writes, local dispatch,
  completion and crashes remain separate actions. Constant reads such as label
  creation, branch/PR absence, and setup details are projected out.
- Each host may crash at every live phase and never resumes that attempt. Its
  reservation remains until confirmed-dead recovery. A failure before dispatch
  still executes Claim ending; a lost response after a write has the state of
  a crash or failed request after that write. A dispatched session may leave
  origin work and keep its Claim, leave no work and restore entry labels, or
  close its issue. Issue closure and ending's label read/write are separate
  actions. Day shift hand-back can reopen a closed issue.
- Day shift can withdraw Ready. Hand-back removes Claim/adds Ready only after
  dispatched work ended. Day shift does not delete a live reservation.

## Requests and control flow

| Model action | Go request or control |
| --- | --- |
| `ListReady` | `pickupRun`: paginated open Ready listing; nonessential data projected out |
| `SelectHistory` | `readReady`: complete events history, reject active Claim, capture generation |
| `CreateReservation` | `reserve`: push unique parentless commit with empty create lease |
| `RefreshIssue` | `pickupReserved` → `readReady`: current issue/labels, retained as Claim entry |
| `RefreshHistory` | `readReady`: history check; `pickupReserved`: selected-generation comparison |
| `PutClaimLabels` | `runTicket` → `makeClaim` → `changeLabels`: re-read labels, then one REST whole-label PUT, Ready off / Claim on. A gate label applied after the refresh snapshot skips the PUT and the dispatch |
| `PrepareWorktree` | `runTicket`: local preparation after successful Claim |
| `Dispatch` | `runTicket` → `harnessSession`: start the opening Harness session |
| `FailBeforeDispatch` | Claim request/setup/session-start failure; no dispatched session |
| `RunEndsWithoutOriginWork` | session ends, origin branch and PR absent |
| `ReadEndingLabels` | `endClaim` → `readIssue`; branch/PR reads fixed absent |
| `RestoreEntryLabels` | `endClaim`: undo this attempt's Claim/Ready changes, or remove Claim if the ending read found the issue closed |
| `KeepClaimWithOriginWork` | `endClaim`: origin work exists, keep Claim |
| `IssueClosed` | session closes issue; subsequent ending reads state and removes Claim without restoring Ready |
| `ReleaseReservation` | `reservation.release`: token-leased delete on a completion context |
| `Crash` | process death; no deferred completion |
| `DayReadsReservation` | operator observes the owner token |
| `DayDeletesCrashedReservation` | owner-token lease after confirmed death |
| `DayWithdrawsReady`, `DayHandsBack` | Day shift label changes |

The Go slice applies Claim **before Harness dispatch**, as requested by #1834.
`TestPickupClaimRequestsMatchModelAtomicTransitions` keeps acquisition and ending
on single whole-label PUTs: `gh issue edit` splits add/remove into concurrent
mutations and does not implement the modeled swap.
The Rust model's documented pre-Claim dispatch counterexample concerns its
earlier dispatch marker. It is not silently removed from the Rust model and a
green Go suite does not establish the Rust literal property. Here, both
`one dispatch per Ready generation` and `one claimed dispatch per Ready generation`
hold at the documented Go dispatch marker. Both predicates remain explicit.
Reachability assertions cover three competing selections, failed-winner Ready
restoration, pre-dispatch failures, issue closure, crash recovery and hand-back.

## Counterexamples and sensitivity

`WithoutClaimCount` removes only the generation comparison after reservation.
Host B selects generation 0; host A claims, dispatches and ends. After Ready is
restored, B reserves and dispatches its stale generation-0 selection. The
shortest witness has 19 actions.

`UnconditionalCreate` replaces the empty create lease with an unconditional
overwrite. Both hosts reserve and refresh before either Claim. Their idempotent
Claim edits then permit both sessions to dispatch generation 0. The shortest
witness has 16 actions. Owner-token release remains guarded in both mutations.

Run safety and positive counterexample regressions:

```sh
go test -run '^TestPickupModel' -count=1 -v
```

These optional negative assertions intentionally exit nonzero with the trace:

```sh
PICKUP_MODEL_VARIANT=WithoutClaimCount go test -run '^TestPickupModelMutations/WithoutClaimCount$' -count=1 -v
PICKUP_MODEL_VARIANT=UnconditionalCreate go test -run '^TestPickupModelMutations/UnconditionalCreate$' -count=1 -v
```

When changing `pickup.go`, `ready.go`, `reservation.go`, `claim.go` or the
dispatch ordering in `run.go`, compare the requests and interleavings here
with the code before relying on the checker. Preserve counterexamples rather
than weakening either property to obtain a passing suite.
