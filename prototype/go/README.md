# Go Ticket-run prototype

An isolated, Linux-only experiment for #1830 / #1832 / #1833 / #1834. The production
Rust binary and ADR 0001 stay as they are. This slice runs a Ticket run through
Delivery, CI watch and bounded Repair sessions. Pickup can select and reserve
a Ready issue and start that same Ticket run.

Requires Go (the version in `go.mod`), Python 3 for tests/generation, Git,
authenticated `gh`, and Claude Code (`claude`) for real sessions. Tests use
fake `gh` and `claude` executables and never contact GitHub or an LLM.

From this directory:

```sh
go test ./...
CGO_ENABLED=0 go build -o /tmp/thirdshift-go .
```

The build produces a single static Linux binary; the CLI tests inspect its
ELF program headers and imported libraries. No Go dependencies are needed.
Run it from the Launch directory of a disposable repository:

```sh
/tmp/thirdshift-go https://github.com/owner/repo/issues/7
# Lower the default maximum of five Repairs (zero disables Repairs):
/tmp/thirdshift-go --repair-limit 2 https://github.com/owner/repo/issues/7
/tmp/thirdshift-go pickup
```

It checks Origin match, an open issue, and the issue author's Trusted account
association (`OWNER`, `MEMBER`, `COLLABORATOR`) before making a Claim. The
Launch directory must have a checked-out Base branch. This first slice creates
`issue-7` and a sibling Worktree `<launch-name>-issue-7`; an existing branch
or directory fails safely rather than resuming or overwriting it.

The Claim re-reads labels immediately before one whole-label REST PUT.
If that read shows the issue is no longer open, Ready was removed, or an
unready label was applied after the readiness snapshot, Pickup skips the
issue and a direct Ticket run stops, without writing labels. Otherwise the
PUT replaces `ready-for-agent`, clears `agent-failed`, and preserves every
other label from the re-read. Ending uses the same single PUT and re-reads
before it, so a label applied while branch and PR state is checked is kept.
The session gets `.thirdshift-issue.md` with the title, body,
and paginated comments, replacing each untrusted comment with a notice.
Factory skills are linked under `.claude/skills/thirdshift-*`. Both those
links and the issue file are excluded through the shared Git `info/exclude`.
The Worktree and materialized skills in the shared Git directory are retained
for inspection, including on failure. Remove the Worktree with `git worktree
remove` and its unused `thirdshift-go-skills-*` directory when finished.

Claude runs with null stdin, `-p --permission-mode auto --output-format
stream-json --verbose`. Parsed JSON events are emitted immediately to stdout;
malformed events, missing completion, error results, and process failures fail
the Ticket run. `SIGINT` or `SIGTERM` interrupts its current child process
group: SIGINT, a two-second grace, SIGTERM, another two seconds, then SIGKILL.
If the leader exits early, remaining group members are killed immediately.
Interruption exits 130. Failed ticket run salvage and Claim ending each use
their own bounded context so they still finish after interruption. Salvage
aborts an unfinished merge and pushes committed work ahead of the Base branch
to a draft pull request. A closed issue loses its Claim; an open issue
with no Issue branch or open PR on origin has only this run's label changes
restored (`ready-for-agent` only if it was present before acquisition, and
`in-progress` removed only if this run added it). Work left on origin keeps
its Claim, with `agent-failed` on failed or interrupted runs.

## Delivery and Repair

The opening Harness commits locally; thirdshift pushes the Issue branch and
opens a draft pull request targeting the selected Base branch. It captures
the PR number and verifies its open state, repository, branches and published
head before watching CI or changing readiness. A closed or merged PR stops
Delivery and salvage without reopening it. Existing same-repository PRs are
verified and reused. A fork pull request for the same branch name is ignored.
An opening session with no commits ahead of the Base branch ends without Delivery.

Delivery fetches and merges the Base branch, pushes, then watches check runs
and commit statuses for the full published head SHA. Pending results are polled.
An empty or passing listing during the first five seconds stays provisional, so
a check that registers after a faster one still counts. After that grace,
passing or absent CI plus a mergeable PR permits readiness. A workflow run that
is still open on the head stays pending even when the checks already listed
succeeded, and a completed failing run is red when its jobs never appeared.
A previously seen set of checks disappearing cannot establish green CI. Inherited failures fail
the run without spending a Repair on the Base branch's code.

A Base branch conflict or failed head checks start a Repair in the same Worktree
with refreshed trusted issue context. Repairs commit their changes, including
any conflicted merge, before Delivery pushes and checks the new head. There are
at most five Repairs (lowered by `--repair-limit`) and ten reconciliation rounds.
An unchanged Repair head fails; this prototype does not yet implement the Rust
runner's Check re-run or Abandoned job re-run policies (ADRs 0010 and 0020).
Failed Delivery drafts any existing PR before attempting a salvage push, so
a rejected push still leaves it as a draft. It preserves the Claim and
adds `agent-failed`; work that could not be pushed remains in the Worktree.
A draft-conversion error is reported while salvage still attempts to push
committed work, then attempts draft conversion again.

Run outcomes, Claim transitions, Delivery states and CI states use typed enums.
`go test ./...` type-checks the production syntax tree and rejects any switch
that does not explicitly cover every enum constant, including switches with
a default. Checker fixtures demonstrate missing and newly added variants fail.

## Pickup

`pickup` lists every page of open `ready-for-agent` issues and tries the lowest
number first. It checks the author, the latest Ready label's actor and subsequent
title/body edits against the Trusted account rule. Complete label history rejects
an active Claim and requires a Ready hand-back after a previous Claim ended,
including either ordering of a same-second hand-back. It passes over triage
labels, Base/Security fixes, sub-issues, open blockers, and issues shaped within
30 minutes. Existing origin Issue branches or PRs in any state require a hand-back.

Only Ticket runs are supported: issues with sub-issues (Specs) are passed over.
This experiment has no production Pass limits, Sweep or notifications,
and inherits the first slice's refusal to resume an existing local branch or
Worktree. It uses the checked-out Base branch and takes no Pickup-specific
options. The dispatched Ticket run uses the same `--repair-limit` as a direct run.

For each candidate, ADR-0017 creates `refs/heads/thirdshift/pickup/<number>` on
origin with an empty create lease and a unique, parentless, empty-tree commit.
Its message records the token, host, Launch directory, PID and start time.
Pickup then rechecks the issue, trust, complete history and Claim-application
count. A busy reservation or stale selection moves on to the next candidate;
no available candidate exits successfully without starting a session.

A successful Claim precedes Harness dispatch. The reservation lasts through
Claim ending, even on interruption, and release requires the owner's exact
commit as its lease. Remote errors fail closed; uncertain create or failed
release names the ref and token for manual recovery. Crashes retain reservations
until the Day shift confirms the owner stopped; reservations never expire.

The independent in-test BFS model checks both dispatch properties with three
hosts and requires shortest counterexamples for `WithoutClaimCount` and
`UnconditionalCreate`. See [the request mapping and limits](PICKUP-MODEL.md).

```sh
go test -run '^TestPickupModel' -count=1 -v
```

## Embedded Factory skills

Go cannot embed files outside its module. `factory-skills.json` is a generated
snapshot of the root `skills/` tree, embedded using `embed.FS`. Refresh it
explicitly when evaluating newer Factory skills:

```sh
go generate
python3 bundle-skills.py --check
```

The snapshot includes supporting files and licenses. Root Factory-skill edits
do not require the Go toolchain or run its suite. The prototype CI workflow
runs only when the prototype or its workflow changes, and checks snapshot
freshness. Existing Rust CI keeps its coverage.

From the repository root, `scripts/affected-tests issue-1830` selects Go tests
only when a path under `prototype/go/` changed. A Go-only change runs no Rust
tests. Mixed changes keep each area's checks; adding Go to a site-only change
does not expand its targeted Rust checks into the full Rust suite.
