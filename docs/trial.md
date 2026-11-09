# Trial protocol, version 1

`thirdshift --trial /absolute/path/request.json` runs one session in a prepared
repository through the same session code as a Run. This internal flag is hidden
from `help`. `session_kind` accepts `code-review` and `spec-review`.
Both Arms' builds must support this protocol: the first Experiment's Baseline
must be the first release that ships the flag, or a later release.
The current Experiment supervises their Session traces (or older Arms' raw Session logs) for plan-limit
warnings even when an Arm build has no warning detector of its own.
It passes `THIRDSHIFT_TRIAL_TRACE_CONTEXT` to trace-aware Arms without changing
the version-1 request. Session traces inherit the Experiment's Command trace ID
and their Trial's parent span. The Experiment also joins older Arms' retained
Session traces after completion. The context is cleared before the Harness is
launched, like Command ownership; standalone Trials keep their own trace IDs.

The caller prepares an isolated repository at the required commit, its branch
and local Base branch, and a `.thirdshift-issue.md` containing the Spec's title,
body and only comments from Trusted accounts. The caller supplies the original
Spec PR URL for a Spec review. That Trial keeps the production Spec review
Session prompt, including its commit and PR-edit instructions. There is no PR
service: its `gh` calls fail inside the sandbox, and only its reports are graded.
A Code review uses the new review-only Session prompt and the production
code-review Factory skill on `base...HEAD`, with the implement session's
sub-agent rules. The caller must commit the reviewed patch on `branch` first.
The Code review writes both axis reports and leaves repository files, refs and
the index unchanged; it fixes and publishes nothing. Its `pr_url` input remains
required by version 1 but is unused.
A Trial does not acquire a Worktree,
make a Claim, fetch the issue, push, watch CI, or perform Delivery itself.

## Request

The request is a UTF-8 JSON object. Paths are absolute. `output_dir` must be
new or empty and outside both the default factory logs directory and the User
config's `logs.dir`, including through symlinks. Model and Effort are the
Harness's strings; `null` leaves either to the Harness's own default. User config
Harness defaults are not applied. No extra Model probe session is started.
The `.trial` marker reserves the output folder against concurrent reuse.

```json
{
  "version": 1,
  "repository": "/tmp/fixture/widgets",
  "session_kind": "spec-review",
  "inputs": {
    "issue_url": "https://github.com/acme/widgets/issues/7",
    "base": "main",
    "branch": "issue-7",
    "pr_url": "https://github.com/acme/widgets/pull/1"
  },
  "harness": "claude",
  "model": "opus",
  "effort": "high",
  "output_dir": "/tmp/trials/widgets-7"
}
```

`harness` accepts `claude`, `codex`, `agy`, `grok`, `muse` or `opencode`.
Factory skills and instruction-file fallbacks are installed as for a Run.
The build rejects unknown request versions before starting any session or
creating its output directory. Unknown fields and unsupported session kinds
are rejected too. An invalid request prints a clear message on stderr, emits
no result and exits `1`.

## Sandbox

The Trial's session runs in bwrap with process and network namespaces of its
own: it sees neither an unbound path nor the host's processes, and everything
it starts ends with it. Its network holds only its own loopback. Only these
are bound: the prepared repository, the Harness's binary (Codex's release
folder, since Codex looks for `codex-code-mode-host` beside it), the Factory
skills folder, a fresh home, the toolchains the session needs and socat,
read-only, and the proxy's socket. Harness flags stay as ADR 0013 has them,
except each Harness's own web-search switch, which a Trial's session adds.

An allowlisting CONNECT proxy runs outside the sandbox, listening on a Unix
socket in a short directory of its own under `/tmp` that is bound into the
sandbox. A socat relay inside turns `127.0.0.1:3128` into that socket, and the
Harness reaches its provider through `HTTPS_PROXY` (`HTTP_PROXY` too, with
`NO_PROXY=127.0.0.1,localhost`, since OpenCode calls its own local server
over HTTP). The proxy allows only that Harness's provider hosts, from #1009's
live-checked lists, and refuses names that resolve to loopback or to the
host's own addresses; anything it does not allow never resolves. A Harness
that ignores the proxy finds no route: without it, even DNS has none, and the
Trial fails into `session_unavailable`, void under the connection-failure
rules. Web search stays off by each Harness's own flag: Claude denies
`WebSearch` and `WebFetch`, Codex sets `web_search="disabled"`, Grok passes
`--disable-web-search`, Muse passes `--disable-web-tools`, and the Trial's
OpenCode config denies `webfetch` and `websearch`. agy has no such switch, so
a Trial keeps its session unchanged; the proxy still reaches only its provider
hosts.

The fresh home holds the login, and the user-level instructions, rules,
settings and hooks a Run's session reads, those read-only. Codex's `auth.json`
is copied into the fresh home, so a refresh stays in that copy.
Claude receives `CLAUDE_CODE_OAUTH_TOKEN` from the Credentials'
`claude.setup_token`. Muse's `~/.config/muse/auth.json` and its
`.auth.json.lock`, and agy's
`~/.gemini/antigravity-cli/antigravity-oauth-token`, are each copied into the
fresh home. Grok Trials use a Trials home of their own at
`~/.thirdshift/trials/grok`, signed in once with
`GROK_HOME=~/.thirdshift/trials/grok grok login`. Each Trial copies only its login
and lock into the fresh home; Grok Trials run one at a time, and the Grok binary
is bound outside that home. The sandbox clears ambient environment variables
and hides Cargo registry credentials. An OpenCode Trial's provider key comes only from
`opencode.api_key` in the Credentials, through its environment; it never sees
production's `opencode.db`. The Trial's config sets that provider's `apiKey`
to `{env:…}` naming the variable, so the Trial's Model names its provider as
`<provider>/<model>`. A Trial whose sandbox cannot start is void with
`sandbox_unavailable`; one whose Harness has no login (a missing Grok login or
OpenCode key) fails before its session starts and is void with
`session_unavailable`. `THIRDSHIFT_TRIAL_NO_SANDBOX`, when set and not empty,
runs Trials and Graders without the sandbox: the Experiment suite sets it so
its fake Harnesses stay observable, and it is the emergency escape when bwrap
breaks on a machine.

## Memory

Each Trial and each Grader runs in a scope of its own, from
`systemd-run --user --scope`, with `MemoryHigh`, `MemoryMax` and
`MemorySwapMax` from the User config's `[trial]` section (defaults `1G`,
`2G` and `1G`, chosen above #1009's observed Harness resident sizes). The scope's
own process raises its `oom_score_adj` to the maximum, so a system-wide kill
takes the Trial or Grader before the factory; that covers old Arm builds
through inheritance, and the Trial raises its own too, which covers direct
runs. The scope sets [`OOMPolicy=kill`](https://www.freedesktop.org/software/systemd/man/255/systemd.scope.html),
so a tool killed by OOM takes down its whole Trial or Grader, even when its
parent catches the tool's failure. A Trial its cap kills dies alone and is
void with `memory_cap`. The launcher watches the scope's
`memory.events` while the Trial runs, and reads the user journal's `oom-kill`
result for the scope after a suspicious exit, which survives the scope
itself. Either is the outer launcher's evidence; without it the Trial stays
failed. Each attempt keeps its scope's unit name beside it, for
`journalctl --user -u` after. Each code check runs in its own Grader scope,
under the Trial timeout and caps. A cap-killed Grader stops the Experiment
with a diagnostic; raise the cap and rerun to retry. `THIRDSHIFT_TRIAL_MEMORY_HIGH`,
`THIRDSHIFT_TRIAL_MEMORY_MAX` and `THIRDSHIFT_TRIAL_MEMORY_SWAP_MAX`, when set
and not empty, override the User config's caps: the Experiment suite uses them
for a small test cap. Scopes need the calling user's manager: run
`loginctl enable-linger <user>` once, so they work while logged out. Without
a reachable manager the Experiment stops before dispatch and names the linger
step. Logged-out launchers discover the user's runtime directory when their
environment omits it.

## Result

Once a valid request starts, stdout is exactly one JSON line; progress is on
stderr. Exit `0` means a result was emitted, including a failed or Void Trial.
The result version is independently identified by `version: 1`.

```json
{"version":1,"outcome":"succeeded","timings":{"total_ms":1234,"session_ms":1200},"void_cause":null,"cause":null,"session_id":"session-123","log":"/tmp/trials/widgets-7/trial-spec-review.logs.jsonl","model_evidence":"/tmp/trials/widgets-7/trial-spec-review.models.json","reports":{"standards":"/tmp/trials/widgets-7/trial-spec-review.standards","spec":"/tmp/trials/widgets-7/trial-spec-review.spec"}}
```

- `outcome`: `succeeded` means the Harness session finished successfully; it is
  not a Score. `failed` includes refusals, context-window failures and other
  failures attributable to the Arm. `void` identifies an interruption, a known
  login or connection failure, a provider server error before any Trial work
  (including work before Resume), a sandbox
  that could not start, a host kill, a Trial-only memory cap, or a provider
  usage/rate limit. An unexplained nonzero exit is a failure, even before work.
  `memory_cap` must come from the outer launcher's evidence of the Trial-only
  cap, rather than an agent's own out-of-memory error, which is a failure.
  A signal alone cannot establish an external cause: an unexplained SIGKILL
  stays failed, and systemd-run reports its scope's signal death as exit code
  137 rather than as a signal. `host_killed` also requires the outer
  launcher's evidence. Version-1 results can already report their distinct
  causes. The Experiment's Bar decides which Void causes to retry or exclude.
  A live Claude or Codex quota window at 85%, or a Claude rejection, stops
  the session and makes the Trial void with `usage_limit`. A Claude warning
  with no readable utilization also stops; warnings whose readable windows
  are all below 85% continue. Ordinary Runs keep running. Codex's retained token-count
  reports are checked while the session runs when exec does not stream them.
  The Experiment's code Grader also marks a changed repository as void with
  `repository_changed`, before running any Eval task checks, unless the Trial
  already failed (for example, on a timeout). Known failures stay failed. This
  compares the session's starting and ending HEAD, refs, index, tracked and
  untracked file contents, including ignored files and its excluded issue input. Only the
  session-generated Factory skill symlinks are omitted; report scratch is
  removed by session completion. Harness instruction fallback links are created
  before the comparison, so later edits to them still invalidate the Trial.
  Repository changes retry fresh under the Bar.
- `timings`: elapsed wall time in integer milliseconds. `total_ms` includes
  request preparation; `session_ms` includes skills setup, the session and any
  Resume, and report preservation.
- `void_cause`: `interrupted`, `session_unavailable`, `usage_limit`,
  `rate_limit`, `sandbox_unavailable`, `host_killed`, `memory_cap` or
  `repository_changed` for a Void Trial; otherwise `null`.
  An Experiment retries up to twice under the fixed Bar, keeps every cause per
  Arm and drops an exhausted task from both Arms. More than a tenth dropped
  makes the Verdict inconclusive. Repeated `memory_cap` stops produce an Arm
  finding in `thirdshift experiments <id>`.
  Fresh execution and recovery use the saved Bar's cause list and retry count:
  only a latest Void result with an allowed cause and remaining budget retries.
  A retained success, failure, missing or excluded Void cause, or exhausted
  history finishes without another attempt, including after failed terminal
  publication. Completed usage-limit, rate-limit and interrupted Void results
  checkpoint before pausing; if terminal, they publish before returning the
  pause. Recovery never re-emits a retained pause. Attempt results, pending
  history and terminal results publish in that order while execution evidence
  remains live; publication errors leave only the successfully saved prefix.
- `cause`: failure diagnostic, or `null` on success.
- `session_id`: the last session's Harness ID if available, otherwise `null`.
- `log`: absolute path to the last session's `.logs.jsonl` Session trace, otherwise `null`.
  Its span file uses the same stem with `.spans.jsonl`. Older Arms may return a raw event log.
- `model_evidence`: absolute path to its `.models.json` sidecar, otherwise `null`.
- `reports`: each axis's preserved report path, or `null` if that report is
  missing. Reports are opaque and missing reports do not change the outcome,
  just as in a Run.

Killed background work gets the Run's one Resume, retaining its Harness,
Model, Effort and review-report directory. Resume artifacts use
`trial-<session_kind>-resume` as their stem; the result points to the last session's
artifacts. Earlier artifacts remain in `output_dir`. A Resume that also leaves
killed work gets no further Resume, as in a Run.

No Command record, Command log or Activity log is created. All Trial session
artifacts stay in `output_dir`; the Inbox, Usual ranges and Silent therefore
ignore them. An inherited Command owner is cleared before launching the Harness.
