# Pickup BFS boundary

Run from the repository root:

```sh
bun test --cwd prototypes/typescript tests/pickup-model.test.ts
```

The in-test BFS explores three identical hosts, each making at most one selection
and reservation attempt for one issue. Every host can crash at every live phase;
at most three Claims can be applied. Selection fixes its Ready generation to the
Claim count it observed. Each property counts dispatches by **that selection**,
not by the count at dispatch. The checker stores visited states and predecessor
edges, so its first counterexample has the fewest modeled actions. Host
permutations are collapsed by sorting full host states with owner/observed-token
markers; histories and dispatch generations are retained in the identity.
It exhausts Current, and stops each mutation once both counterexamples are found.

## Requests and local control

| Action | Prototype operation |
| --- | --- |
| ListReady | `ReadyIssues.numbers` / issue-list GET |
| SelectHistory | `ReadyIssues.read` / complete label history and captured Claim count |
| CreateReservation | `Reservation.take` / empty-lease push of a unique parentless commit |
| RefreshIssue | second `ReadyIssues.read` / current issue GET |
| RefreshHistory | second history read; reject active Claim or changed count |
| PutClaimLabels | `Claim.make` / atomic PUT of Ready off, Claim on |
| Dispatch | `runTicket` / start the Ticket's Harness session after the Claim |
| FailBeforeClaim | failure before the Claim mutation; no session dispatched |
| FailAfterClaim | failed comment read, Worktree acquisition or Harness start |
| FailedRunReadsOrigin | `Claim.end` / fixed absent Issue branch and PR |
| ReadEndingLabels | `Claim.end` / current issue GET (before origin/PR reads) |
| PutEndingLabels | `Claim.end` / atomic PUT restoring entry labels |
| KeepClaim | successful or failed work with an origin branch or PR |
| ReleaseReservation | owner-token-leased ref deletion in `finally` |
| Crash | death without cleanup; committed labels/ref survive |
| DayReadsReservation | human observes origin's current token |
| DayDeletesStaleReservation | token-leased deletion only after confirmed owner death |
| DayHandsBack | Ready on, Claim off after claimed work is confirmed stopped |

The model projects out constant reads: trust, settling, parent/blocker checks,
label creation, comments, Worktree and skill extraction. Current issue reads
and history are linearizable and complete; pagination races and GitHub's actual
consistency are not proven. Origin/PR reads for Claim ending are fixed absent
or represented by KeepClaim. The model moves their side-effect-free reads
ahead of ReadEndingLabels; no modeled state depends on those fixed reads.
Unrelated labels are projected out. A lost response after a committed mutation
is represented by crashing after the mutation; acquisition never dispatches
on uncertain ownership. Recovery cannot delete a live owner's token. Unique
tokens are host IDs because each host attempts acquisition only once.

## Properties and Rust comparison

Both `one dispatch per Ready generation` and `one claimed dispatch per Ready
generation` must hold for Current. Dispatch follows Claim in this prototype,
so every dispatched session is already a claimed dispatch. Unlike Rust's
`tests/pickup_model.rs`, it has no pre-Claim dispatch window. The Rust green
suite explicitly retains a counterexample to the literal dispatch property;
this prototype's green test does not prove that Rust property.

Reachability assertions require three competing selections, crashes, stale-ref
recovery, failed-winner requeue, pre/post-Claim startup failures, retained Claims
and Day shift hand-back. WithoutClaimCount removes only the generation equality;
UnconditionalCreate removes only the empty-create lease. Both must produce
shortest counterexamples to both properties. The first permits a stale selection
after Claim release/hand-back; the second overwrites a live owner's reservation
and lets both owners refresh before either applies the Claim. Owner-leased
release remains enabled for both mutations.

Compared with Rust, this model does not cover diagnosis's needs-info ending
(the prototype has no diagnosis), direct Issue URL commands, Spec runs, other
Pass kinds, arbitrary human intervention, or liveness. It is bounded exploration,
not a proof over unbounded hosts or restarts. Product/model drift still requires
review: neither model imports the implementation under test.
