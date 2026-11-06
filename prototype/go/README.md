# Go Ticket-run prototype

An isolated, Linux-only experiment for #1830 / #1832. The production Rust
binary and ADR 0001 stay as they are. This slice stops after the opening
Harness session, before Delivery, CI watch, Repair or Pickup.

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
Interruption exits 130. Claim ending uses its own bounded context so it still
finishes after interruption: a closed issue loses its Claim; an open issue
with no Issue branch or open PR on origin is released and made ready again;
otherwise its Claim stays, with `agent-failed` on failed or interrupted runs.

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
