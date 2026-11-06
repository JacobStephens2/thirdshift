# Go Ticket-run prototype

An isolated, Linux-only experiment for #1830 / #1832 / #1833. The production
Rust binary and ADR 0001 stay as they are. This slice runs a Ticket run through
Delivery, CI watch and bounded Repair sessions. Pickup remains separate work.

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
```

It checks Origin match, an open issue, and the issue author's Trusted account
association (`OWNER`, `MEMBER`, `COLLABORATOR`) before making a Claim. The
Launch directory must have a checked-out Base branch. This first slice creates
`issue-7` and a sibling Worktree `<launch-name>-issue-7`; an existing branch
or directory fails safely rather than resuming or overwriting it.

The Claim replaces `ready-for-agent` and clears `agent-failed`, preserving
other labels. The session gets `.thirdshift-issue.md` with the title, body,
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
Delivery and salvage without reopening it. Existing PRs are verified and reused.
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
