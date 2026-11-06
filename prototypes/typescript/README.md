# TypeScript Ticket, Pickup and Console prototype

This isolated experiment implements the opening half of a Ticket run (#1836)
and a Pickup run with bounded model checking (#1838), plus the shared-schema
Console slice (#1839), under Spec #1831.
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
pull request exists, and otherwise retained. The prototype ends after one
Harness session; Delivery and CI Repair belong to later tickets.

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
closed. The reservation stays through Claim ending, and release uses the exact
owner token even after interruption. Uncertain acquisition or failed release
prints the ref and token for human recovery. No timer expires a reservation.

The prototype applies the Claim **before dispatch**. This deliberately avoids
the Rust model's known dispatch-before-Claim counterexample without modifying
the production protocol. [The model boundary](tests/pickup-model.md) records the
request mapping, assumptions, bounded coverage and comparison limits.

`bun test` compiles the binary and exercises it with real temporary Git
repositories and fake `gh` and `claude` executables, including a signal-resistant
forked Harness. Tests run the executable with no Bun on PATH. Test waits have
generous overall bounds; the one-second escalation intervals are production
behavior, not test deadlines.

## Shared-schema Console slice

Install Chromium once for the Playwright test (and its OS dependencies if needed):

```sh
bunx playwright install chromium
# On a fresh Linux host: bunx playwright install --with-deps chromium
```

`src/console/schema.ts` defines the runtime Zod schemas and derives the TypeScript
types for Inbox status and Dismissal requests/responses. The CLI, HTTP handlers,
browser client and browser test consume those schemas. OpenAPI 3.1 schemas are
generated from them. `src/console/routes.ts` pairs every route with its Console
page, CLI twin and required scope; the parity test compares the document against
the table and sends a request to every registered route. The build bundles the
typed browser client and embeds it and the HTML in the standalone executable.

The slice uses a separate prototype Inbox file, not Rust Command records. It
does not yet ingest Ticket runs or reproduce GitHub settlement, retention,
collapsed rows, kind changes, account visibility or Activity/Console logs. Seed
that file with a payload matching the shared schema, for example:

```sh
export THIRDSHIFT_PROTOTYPE_INBOX=/tmp/typescript-inbox.json
cat > "$THIRDSHIFT_PROTOTYPE_INBOX" <<'JSON'
{"activeRuns":0,"rows":[{"id":"ticket-7","title":"Ticket #7 failed","group":"needs-you","active":false,"dismissal":null}]}
JSON
./build/thirdshift status --json
./build/thirdshift dismiss ticket-7 --json
./build/thirdshift dismiss ticket-7 --undo --json
./build/thirdshift console openapi
export THIRDSHIFT_PROTOTYPE_TOKEN="$(openssl rand -hex 32)"
./build/thirdshift console --port 3939 --scope act
```

Open the printed localhost URL and enter that token. JSON routes require it as
`Authorization: Bearer <token>`. The HTML login shell and bundled script are
public and contain no Inbox data or token. The browser keeps the entered token
in session storage for reloads. `--scope read` permits status and OpenAPI but
refuses Dismissal/Undo with 403; `act` includes read access. This local-only
prototype uses one configured token and binds exclusively to `127.0.0.1`; it
does not implement production Console accounts or token administration.

Without the override, the Inbox file lives at
`~/.local/state/thirdshift-typescript/inbox.json`; a missing file gives an empty
Inbox. The active-run count is calculated from the rows. Dismissal moves a
waiting row to Working if active or Resolved if ended, keeping its prior group
for Undo. Other rows cannot be dismissed. Updates are serialized inside one
Console and atomically replace the file. Run only one writer at a time, including
CLI actions. Both CLI and HTTP actions return the fresh shared status payload.

`bun test tests/console.test.ts` runs compiled CLI/HTTP parity, authorization,
and Playwright checks at 1440×900 and 390×844, including Dismissal, reload, Undo
and persisted CLI status. Browser failures are failures, not skipped checks.
Optional `AFTER_DESKTOP` and `AFTER_MOBILE` paths capture the restored Inbox.

`scripts/affected-tests issue-1831` selects only `tsc --noEmit` and `bun test`
when all changes are under this directory. Changes elsewhere do not select
this suite. For mixed changes, prototype checks are added independently to
the existing selector rules. Install dependencies here before using that command.
