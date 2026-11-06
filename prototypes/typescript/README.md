# TypeScript Ticket and Pickup prototype

This isolated experiment implements a Ticket run through Delivery and bounded
CI/conflict Repair (#1836 and #1837), and a Pickup run with bounded model
checking (#1838), under Spec #1831.
It does not replace the Rust executable or change ADR-0001.

On Linux, with Bun 1.4.4, Git and GitHub CLI installed:

```sh
cd prototypes/typescript
bun install --frozen-lockfile
bun run typecheck
bun test
bun run build
./build/thirdshift https://github.com/owner/repository/issues/123
./build/thirdshift pickup
```

Run the executable from the target repository's Launch directory on its Base
branch. It accepts only a canonical `https://github.com/owner/repository/issues/N`
URL: userinfo, a port, a query, and a fragment are rejected before any GitHub
call or session. It then checks Origin match and the issue's open state and
Trusted account authorship, takes the Claim, and starts `claude` with null
stdin, streaming JSON output and `--permission-mode auto`. SSH and HTTPS GitHub
origins are supported.
The executable needs `git`, authenticated `gh` and `claude` on PATH; Bun and a
checkout of this prototype are needed only to build it.

The build script invokes `bun build --compile --target=bun-linux-x64`, embedding
every file under the repository's `skills/` directory into a transient typed
entrypoint. Build output stays under ignored `build/`. Factory skills are
extracted into the target repository's common Git directory and linked under
the Worktree's `.claude/skills/`, with entries in the shared `info/exclude`.
The issue file includes only comments by Trusted accounts; other comments get
a placeholder. Worktrees, branches and skill files are retained for inspection.
This first slice intentionally fails safely on an existing Issue branch instead
of implementing Continuation or numbered Issue branches.

SIGINT, SIGTERM and SIGHUP record interruption. The active subprocess group gets
SIGINT, then SIGTERM after one second, then SIGKILL after another second, even
if its leader has exited. Claim ending runs after cancellation: a new Claim
is restored to its prior labels when neither an Issue branch on origin nor a
pull request exists, and otherwise retained.

After the opening Harness session commits, Delivery pushes the Issue branch,
opens or verifies a draft pull request against the Base branch, merges Base
branch updates, and watches check runs and commit statuses on the exact head.
It waits briefly for initially absent checks to register. Passing or absent CI
and a mergeable, unchanged head allow Delivery to mark the pull request ready
for review. Conflicts and failed CI start follow-up Harness sessions, up to
five Repairs per Ticket run. Delivery bounds reconciliation to ten rounds and
CI/mergeability waiting to ten minutes (subprocess durations are separate).

Failed ticket run salvage aborts an unfinished merge, pushes commits ahead of
the Launch directory's Base branch, and opens or keeps a draft pull request.
Salvage and Claim ending finish even after interruption. Published work retains
the Claim; nothing on origin releases a new Claim and restores prior readiness.
Worktrees and local commits remain available if publication fails.

Run outcomes, Claim transitions, Delivery stages and CI check states use tagged
unions with exhaustive `satisfies never`/`assertNever` branches, checked by
`bun run typecheck`. The prototype deliberately covers the core Repair loop:
Continuation, Self-merge, inherited-failure/Base fix detection, Check re-runs,
Abandoned job recovery and Foreign commit reconciliation remain Rust behavior;
an unexpected PR head or identity fails safely here.

Pickup derives the repository from the Launch directory's GitHub origin and
lists every page of open `ready-for-agent` candidates, lowest number first.
It checks Trusted account authorship, the latest Ready labelling's actor,
subsequent title/body edits, Claim history, open blockers, parentage, disqualifying
labels and the five-minute shaping wait. History and edit connections are read
to completion. This Ticket-only slice passes over Specs (issues with sub-issues),
and fails safely at Worktree creation for an existing local Issue branch, as
the direct Ticket command does. Continuation and Spec runs are not implemented.
More than 100 blockers is conservatively passed over rather than truncated.

Each candidate is reserved using ADR-0017's empty create lease on
`refs/heads/thirdshift/pickup/N`. Its unique commit has an empty tree, no parent,
and host/process/recovery metadata. Pickup re-reads readiness and compares the
Claim-application count before applying its Claim and starting the Ticket
session. A busy or stale candidate is passed over for the next candidate;
exhaustion skips with exit 0. Remote errors without another owner's token fail
closed. The reservation stays through Delivery, salvage and Claim ending, and
release uses the exact owner token even after interruption. Uncertain acquisition
or failed release prints the ref and token for human recovery. No timer expires
a reservation.

The prototype applies the Claim **before dispatch**. This deliberately avoids
the Rust model's known dispatch-before-Claim counterexample without modifying
the production protocol. [The model boundary](tests/pickup-model.md) records the
request mapping, assumptions, bounded coverage and comparison limits.

`bun test` compiles the binary and exercises it with real temporary Git
repositories and fake `gh` and `claude` executables, including a signal-resistant
forked Harness, green Delivery, CI and merge-conflict Repair recovery, bounded
Repair exhaustion, pending/delayed checks, commit statuses and draft salvage.
Tests run the executable with no Bun on PATH. Test waits have
generous overall bounds; the one-second escalation intervals are production
behavior, not test deadlines.

`scripts/affected-tests issue-1831` selects only `tsc --noEmit` and `bun test`
when all changes are under this directory. Changes elsewhere do not select
this suite. For mixed changes, prototype checks are added independently to
the existing selector rules. Install dependencies here before using that command.
