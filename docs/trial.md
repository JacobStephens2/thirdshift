# Trial protocol, version 1

`thirdshift --trial /absolute/path/request.json` runs one session in a prepared
repository through the same session code as a Run. This internal flag is hidden
from `help`. `session_kind` accepts `code-review` and `spec-review`.
Both Arms' builds must support this protocol: the first Experiment's Baseline
must be the first release that ships the flag, or a later release.
The current Experiment supervises their raw Session logs for plan-limit
warnings even when an Arm build has no warning detector of its own.

The caller prepares an isolated repository at the required commit, its branch
and local Base branch, and a `.thirdshift-issue.md` containing the Spec's title,
body and only comments from Trusted accounts. The caller supplies the draft
Spec PR URL for a Spec review. That Trial uses the ordinary Spec review Session
prompt, including its commit and PR-edit instructions: use fixture services or
disposable PRs when the Trial must not change a production PR.
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

## Result

Once a valid request starts, stdout is exactly one JSON line; progress is on
stderr. Exit `0` means a result was emitted, including a failed or Void Trial.
The result version is independently identified by `version: 1`.

```json
{"version":1,"outcome":"succeeded","timings":{"total_ms":1234,"session_ms":1200},"void_cause":null,"cause":null,"session_id":"session-123","log":"/tmp/trials/widgets-7/trial-spec-review.jsonl","model_evidence":"/tmp/trials/widgets-7/trial-spec-review.models.json","reports":{"standards":"/tmp/trials/widgets-7/trial-spec-review.standards","spec":"/tmp/trials/widgets-7/trial-spec-review.spec"}}
```

- `outcome`: `succeeded` means the Harness session finished successfully; it is
  not a Score. `failed` includes refusals, context-window failures and other
  failures attributable to the Arm. `void` identifies an interruption, a known
  login or connection failure, a provider server error before work, a Harness
  that exits before work without a diagnostic, or a provider usage/rate limit.
  The Experiment's Bar decides which Void causes to retry or exclude.
  A live Claude plan-limit warning or rejection, or a Codex quota report at
  85% in either window, stops the session and makes the Trial void with
  `usage_limit`. Ordinary Runs keep running. Codex's retained token-count
  reports are checked while the session runs when exec does not stream them.
  The Experiment's code Grader also marks a changed repository as void with
  `repository_changed`, before running any Eval task checks. This compares the
  session's starting and ending HEAD, refs, index, tracked and untracked file
  contents, including ignored files and its excluded issue input. Only the
  session-generated Factory skill symlinks are omitted; report scratch is
  removed by session completion. Harness instruction fallback links are created
  before the comparison, so later edits to them still invalidate the Trial.
  Repository changes retry fresh under the Bar.
- `timings`: elapsed wall time in integer milliseconds. `total_ms` includes
  request preparation; `session_ms` includes skills setup, the session and any
  Resume, and report preservation.
- `void_cause`: `interrupted`, `session_unavailable`, `usage_limit` or
  `rate_limit` for a Void Trial; otherwise `null`.
- `cause`: failure diagnostic, or `null` on success.
- `session_id`: the last session's Harness ID if available, otherwise `null`.
- `log`: absolute path to the last session's raw event log, otherwise `null`.
  Its OTLP files use the same stem with `.logs.jsonl` and `.spans.jsonl`.
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
