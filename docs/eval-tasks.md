# Eval tasks and suites (format 1)

Only this format and made-up examples belong in the repository. Real Eval tasks,
inputs, success criteria and source repositories stay private on the host. A suite
lives at `~/.thirdshift/evals/suites/<suite>/`:

```text
reviews/
├── suite.toml
└── tasks/
    └── invented-counter/
        ├── task.toml
        └── first-spec.md
```

Suite and Eval task names are single directory names containing ASCII letters,
digits, hyphens or underscores. Manifests are UTF-8 TOML. Unknown fields and format
versions are errors. Missing suite storage lists no suites; malformed existing
manifests fail rather than silently disappearing from selection.

## Suite manifest

```toml
format = 1
trials_per_arm = 3

[dependencies]
factory_skills = ["skills/thirdshift-code-review/"]
# Optional for process suites; omitted by existing review suites.
builtin_processes = ["processes/", "src/processes.rs", "src/processes/"]
session_prompts = ["src/prompt.rs"]
harness_defaults = ["src/harness/", "src/config.rs"]
model_defaults = ["src/config.rs", "src/setup.rs"]
effort_defaults = ["src/config.rs", "src/setup.rs"]
```

`trials_per_arm` is a positive integer **per Eval task**. For example, the
code-review suite uses three and the Spec-review suite uses one. All five
original dependency categories must be declared; an unused category is `[]`.
`builtin_processes` is optional and defaults to `[]`. Each entry
is a repository-relative path: an exact file, or a directory prefix ending in
`/`. Wildcards, absolute paths, `.` and `..` components are refused. The example
selects this suite for `src/prompt.rs` or `src/harness/codex.rs`, but not
`src/prompt.rs.old`. Declare every source that can change the suite's session,
including default selection and settings. Shared sources can conservatively
touch more than one suite. The declarations, rather than hard-coded session
names, decide selection. Include both old and new paths for a rename.

## Eval task manifest

This is invented data, not a real bug or a usable repository:

```toml
format = 1
state = "proposed"
session = "code-review"
repository = "/private/invented-counter-source"
base = "1111111111111111111111111111111111111111"
change = "2222222222222222222222222222222222222222"
criteria = "Catch the invented off-by-one error; report no wrong findings."
checks = [["cargo", "test", "counter_regression"]]

[inputs]
".thirdshift-issue.md" = "first-spec.md"

[trial]
issue_url = "https://github.com/acme/widgets/issues/7"
base = "main"
branch = "issue-7"
pr_url = "https://github.com/acme/widgets/pull/1"

# Optional; written by experiments <id> --record-difficulty.
[difficulty]
passes = 4
trials = 6
experiment = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
commit = "3333333333333333333333333333333333333333"
harness = "claude"
model = "opus"
effort = "high"
```

- `state` is `proposed` or `approved`. Proposals are listed with their state and
  are never eligible for preparation or Trials. Approve one with
  `thirdshift evals approve <suite> <task> [--json]` or on the Console's
  Experiment pages. Approval changes only its state in the existing suite,
  preserving its inputs and manifest comments. Approving again is harmless.
  It starts no work and affects future Experiments only; an existing
  Experiment keeps its fixed Eval tasks, including when resumed.
- `session` is `code-review`, `spec-review` or `process`. Process tasks also
  require `provenance = "real"` and `environment = "host"` at the top level.
  They start at `base`, with the Ticket inputs and a local origin; `change`
  remains a private reference fix and is not applied to their repository.
  Their Arm's Built-in process runs through its local Steps, stopping before
  publication. A delivered Trial passes only when both groups of code checks
  in its required `process_grading` table pass. Stopped-to-ask and failed do
  not pass. Review task checks and Model grading continue as before.

  ```toml
  [process_grading]
  test_files = ["tests/counter.rs"]
  test_fakes = ["tests/support/counter.rs"] # optional, defaults to []
  fail_to_pass = [["cargo", "test", "--offline", "--test", "counter", "regression"]]
  pass_to_pass = [["cargo", "test", "--offline", "--test", "counter", "neighbor"]]
  ```

  `test_files` is nonempty; `test_fakes` names shared test fakes changed by
  the reference fix. Each path is confined and relative to the repository.
  Both selector groups require at least one nonempty argument vector.
  After the Trial, the Grader exports its final commit, discarding uncommitted,
  untracked and ignored files. It replaces the named tests and fakes with their
  contents and executable modes from `change`, then runs `fail_to_pass` followed
  by `pass_to_pass`. Selectors share build artifacts in this private copy.
  They run offline in bwrap, sharing the Bar's timeout with preparation of the
  final-commit snapshot. Nonzero exits and
  timeouts fail checks; the Score records the selector and its reason. Editing
  the overlaid files cannot help a Trial pass. Python selectors search the
  repository after the standard library, so a committed module cannot replace
  one. Process tasks use this table instead of `checks`; review tasks cannot
  use it.

  The optional `[process_grading.new_tests]` table adds base-tests and
  cross-tests for each pair of Trials, as evidence beside the Scores:

  ```toml
  [process_grading.new_tests]
  paths = ["tests/*.rs"]
  command = ["cargo", "test", "--offline", "--test", "{stem}"]
  ```

  `paths` is nonempty; each pattern is confined and relative to the
  repository, and `*` matches within one path component. An Arm's new test
  files are those its final commit adds or changes since `base` that match a
  pattern. `command` is a nonempty argument vector run once per new test file,
  with `{path}` replaced by the file's repository path and `{stem}` by its
  name without the last extension. After both Trials of a pair finish, each
  Arm's new test files are overlaid on `base`, then on the other Arm's final
  commit, the same way as the reference tests above, and run offline in bwrap.
  Each run shares the Bar's timeout with preparing its snapshot. Results go to
  the Experiment's `pairs` ([Experiment results](experiment-results.md)); they
  never change a Score.
- `difficulty` is an optional, replaceable A/A reading: passes and non-void
  Trials pooled across both Arms, with the Experiment ID and its identical
  commit, Harness, Model and Effort. `model` and `effort` are omitted when the
  Experiment left them to the Harness's defaults. Passes cannot exceed Trials.
  A missing table, or fewer than 6 Trials, means **unmeasured**. A measured Eval
  task is **discriminating** when passes / Trials is in the inclusive band
  0.15–0.95. These are constants of the method, not Bar settings.
  `thirdshift experiments <id> --record-difficulty [--json]` records every Eval
  task from a completed A/A Experiment, leaving Void Trials and retry attempts
  out of both counts and using the latest rechecked Scores. It refuses an
  unfinished Experiment or differing Arms before writing any manifest. Each
  manifest is replaced atomically, preserving the rest of its contents, under
  the same exclusive suite lock as approval. Re-recording replaces the previous
  table. This changes neither the saved Experiment nor its Verdict.
  `thirdshift evals` (also `evals list`, optionally `--json`) lists the table or
  `null`, plus `measured` and `discriminating` booleans for each Eval task.
- `repository` names a local Git repository holding both commits, absolute or
  relative to the Eval task's folder. History import is local. Process screening
  runs on the host with network access so test tools can install dependencies.
- `base` and `change` are full commit object IDs (SHA-1 or SHA-256). The base must
  be an ancestor of the change. `base` is the last history visible to a Trial;
  `change` supplies the working-tree diff under review. They may be equal for a
  Spec-review Eval task needing no initial code diff.
- `criteria` is nonempty written success criteria for a model Grader. Review `checks`
  is optional (defaults to `[]`), with one nonempty argument vector per code
  Grader check. Arguments are not shell text. Checks are recorded here, not
  executed by review-task preparation. After a Trial, the Experiment runs each check with
  an isolated copy of the prepared repository, captured before the session,
  as its working directory. Session edits and commits cannot change the code
  these checks grade. Exit zero
  passes; nonzero or the Bar's timeout fails the check. A sandbox that cannot
  start makes the Trial void. Each check gets
  `THIRDSHIFT_TRIAL_STANDARDS_REPORT`, `THIRDSHIFT_TRIAL_SPEC_REPORT` and
  `THIRDSHIFT_TRIAL_LOG` (empty: raw Trial logs are hidden from Graders).
  Report variables name copies in the Grader's packet, empty when absent.
  These paths let a check inspect the reports without putting grading criteria
  in the Trial's repository. Checks run after repository-change detection and
  may create their own build artifacts. Their commands and reasons are saved
  privately in the Trial's Score.

  After code checks, a fresh model Grader reads one `packet.json` per Eval task.
  It contains `criteria` and `trials`: each Trial has a shuffled neutral `label`,
  `outcome`, and optional `standards` and `spec` report text. Identifying lines
  are removed; Arm mappings, artifact paths and raw session streams stay out.
  The Grader writes `scores.json` as `{"scores": [...]}`, with exactly one
  object per label: `label`, boolean `passed`, nonempty `reason`, boolean
  `wrong_finding` and boolean `borderline`. Missing, duplicate or unknown labels,
  unknown fields and invalid values pause grading rather than supplying defaults.
  `THIRDSHIFT_GRADER_PACKET` and `THIRDSHIFT_GRADER_SCORES` name those two files.
  Saved Trial Scores combine mechanical and model pass/fail and retain the
  model response separately. The Verdict counts borderline Trials and uses
  wrong-finding flags in its existing paired assessment. Each private Grader
  session retains OTLP log records and spans, model evidence and stderr beside
  its packet, including when interrupted or timed out. Model Graders run in
  the Trial sandbox with a fresh home and provider-only network, binding only
  their packet workspace. They cannot read Trial logs, Arm mappings or the
  Experiment's other files. An unavailable Grader pauses grading without
  publishing Scores; resuming reuses completed Trials. Code Graders run offline
  in bwrap, with only their report packet and the prepared repository.

  When the Bar names a Grader panel, each seat reads each pair of process
  Trials in its own `packet.json`, once per presentation order. It contains
  `order` (1 or 2), `ticket` (the input copied to `.thirdshift-issue.md`, or
  null), `criteria`, the `rubric` (each criterion's `name` and `question`) and
  two `sides`, labelled `A` and `B` at random and swapped in the second order.
  Each side has its `outcome`, its `diff` against `base` (null when it cannot
  be read within the Bar's timeout, as when the Trial's repository is no longer
  retained; added lines a report would lose as identifying are removed) and its Pair `evidence` without error text
  (null without `new_tests`). The Grader writes `calls.json` with exactly
  `sides`, `reason` and `preference`: each side has `label`, a `rubric` rating
  per criterion (`criterion`, boolean `met`, nonempty `evidence`), a nonempty
  `merge_reason` and boolean `ready_to_merge`, and `preference` is `A`, `B`,
  `tie` or `both_bad`. `THIRDSHIFT_GRADER_PACKET` and `THIRDSHIFT_GRADER_CALLS`
  name those two files. An invalid response pauses grading; each validated call
  is saved, so resuming asks only for the calls still missing.
- `inputs` maps destination paths in the prepared repository to files relative
  to the Eval task's folder. Use, for example, a Spec's body **as first
  published**, not its revised answer. The table is required but may be empty.
  Paths must stay within their respective folders, and cannot address `.git`.
  Only these input files are copied; the manifest, criteria and checks remain
  outside the prepared repository.
- `trial` supplies the version-1 Trial's session inputs. It is optional for
  standalone preparation, and required for approved tasks selected by an
  Experiment. Missing tables are reported by task name before either Arm is
  built, including for release Experiments. `base` and `branch` name distinct
  local branches; `issue_url` and `pr_url` are the original issue and PR URLs.
  Complete Trial repository preparation creates those local branches, commits
  the staged reviewed patch and, for Spec review, pushes them to the disposable
  bare origin. A Spec-review Trial keeps the production Spec
  review prompt. There is no PR service: its `gh` calls fail inside the sandbox,
  and only its reports are graded.
  Code-review Experiments also commit the prepared patch on the Issue branch,
  so the production review skill sees `base...HEAD` as reviewed. They create no
  origin and push nothing. Their review-only prompt retains both axis reports
  without fixing findings or changing repository files, refs or the index.

The imported code-review suite declares three Trials per Arm per Eval task;
the Spec-review suite declares one. Three-Trial Verdicts include `pass_cubed`
for each Arm: the fraction of retained Eval tasks where all three Trials passed.

## Commands and repository preparation

```sh
thirdshift evals list
thirdshift evals approve reviews invented-counter --json
thirdshift evals touched skills/thirdshift-code-review/SKILL.md src/prompt.rs
thirdshift evals prepare reviews invented-counter /tmp/invented-trial
thirdshift evals verify /tmp/invented-trial 1111111111111111111111111111111111111111
```

All four commands write JSON to stdout, exit `0` on success, and exit `1` with
diagnostics on failure (argument errors exit `2`). `list` includes each suite's
dependencies, Trials per Arm and each Eval task's name, state and session.
`touched` returns a sorted array of suite names, once each, for the supplied
changed paths. With no changed paths it returns `[]`.

`prepare` is the inspection operation: it requires an approved Eval task and a
destination that does not exist. Trial inputs are optional for inspection.
It creates an independent Git repository in a temporary sibling folder, fetches
only the base's ancestry from the local source, and checks out a detached base.
It stages the binary-capable `base..change` patch in the index without importing
the change's commit objects, then copies the declared inputs. The reviewed change,
including added files and submodule commit updates, is visible with
`git diff --cached HEAD`; copied inputs may also be untracked or unstaged.
Patch generation uses fixed prefixes, plain output and submodule commit IDs,
independently of the source's display settings. No source worktree, configuration,
hooks, alternate object store, tags or branches are copied. Reflogs are expired
and unreachable objects pruned. Before publishing the destination, preparation
uses `verify` to compare **all** visible history with the base's ancestry. A
planted later commit, including one on another ref, fails verification. Failure
removes the temporary repository and leaves the destination absent.

A code-review repository has no origin. A Spec-review repository gets a fresh
local bare origin at `<destination>/.git/eval-origin.git`, seeded only with the base. It
is pushable: the session can create a branch, commit and push as in production.
The bare origin receives the same history verification. Inspection preparation returns
`repository` and `origin` paths (`origin` is `null` for code review); `verify`
returns `{"safe":true}` on success. Keep or delete the entire destination for
the lifetime of its Trial; the repository and any bare origin are throwaway.

Trial execution requests a complete repository through a separate preparation
operation. It requires Trial inputs with valid, distinct Base branch and Issue
branch names, including custom names containing slashes. It performs the same
base-only import and verification as inspection, then creates the requested
branches and one synthetic reviewed commit directly above the recorded base.
The commit uses the existing index: copied inputs and tracked worktree overlays
are not added to the reviewed patch. The issue input is excluded locally.
Code review has no origin; Spec review seeds the staged private origin with the
requested Base branch, then publishes both requested refs without removing the
seed. Inspection keeps its conventional `base` seed; that name reserves no
prefix in complete Trial preparation. The configured origin URL and
returned paths name the final destination and remain usable after publication.
Branch validation, checkout, commit and push share preparation's isolated Git
policy, ignoring caller hooks and global/system settings. All preparation
finishes in the temporary sibling before publication; failures leave the
destination absent and launch no Arm. Existing destinations are refused.
Trial execution then establishes the selected Harness's instruction fallback
and captures integrity before launching the Arm. Repository-change detection
protects `.thirdshift-issue.md`, every destination in `[inputs]`, and the Base
branch and synthetic change commits under review. Editing or deleting an input,
moving the Base branch, or altering or removing a protected Git object makes
an otherwise successful Trial void with `repository_changed`. Ordinary Issue
branch commits, pushes to the local origin, report files, and other tracked,
untracked or ignored working-tree changes do not void a Trial. For tasks with
`checks`, a private copy captured before the session supplies their repository;
checks share their own build artifacts, never the session's edits. Process
Trials instead grade the final commit with the reference fix's test overlay,
as described in `process_grading` above.
`evals verify` retains its
inspection contract: it expects the detached base, before the synthetic commit.

These commands prepare data for the Experiment runner; they start no Harness,
Trial or Grader. Isolation of the session's home, processes and network belongs
to the runner's sandbox, not to this Git preparation step.

## Process task preparation and proposals

Process tasks record their separate tests and command-vector selectors:

```toml
[process_grading]
test_files = ["tests/counter.rs"]
test_fakes = ["tests/support/mod.rs"]
fail_to_pass = [["cargo", "test", "--test", "counter", "increments"]]
pass_to_pass = [["cargo", "test", "--test", "counter"]]
```

The optional `[process_grading.real_fix]` table retains private leak-check
identifiers: `pull_request` and `issue` URLs, `branch`, `title`, and `issue_title`. Proposals
populate these from the source Ticket and pull request. The merge commit is
always `change`. Older manually authored tasks also check their available
`trial` references and the reference commit's title. This data stays outside the
Trial request and prepared repository.

`evals prepare` requires this selection for process tasks. In disposable
checkouts it overlays these files from `change` onto both `base` and `change`,
then requires every fail-to-pass selector to fail at the base and pass at the
merge. A refusal names the selector and side. Each command has a ten-minute
deadline and runs with host network access. A changed inline Rust test module
in a source file is refused, naming the file; production edits outside an
unchanged inline test module remain eligible. Separate overlays must be regular
test files and may not cross symlinks; executable modes are preserved. Screening leaves the source checkout
untouched, and the prepared Trial still contains only base history and Ticket
inputs, with neither the fix nor its test overlay exposed.

`thirdshift evals propose-process <repository> [--limit <1..100>]` reads recent
merged GitHub pull requests for a local repository's origin and drafts private
`process` Eval tasks. It lists each qualifying pull request's bug or feature
kind, non-test added/deleted line count, test files, shared fakes and selectors.
Pull requests must close a Ticket (an issue with no sub-issues), change source,
and have separate tests without changed inline tests. Rust integration tests
and Python test scripts are recognized automatically; other layouts are reported
as skipped for manual task authoring. Security fixes are excluded. The command
keeps a private repository holding both commits and writes the Ticket title and
body as `.thirdshift-issue.md` input. Proposals are not screened or approved
automatically: review the selectors and issue input before approval and
preparation. Repeating the command preserves existing proposals and approvals.
Merge, squash and rebase ranges include the entire Ticket's change. For rewritten
linear history, discovery compares the original PR's patch with the merged
range; an ambiguous range is skipped for manual task authoring. The API twin,
`POST /api/v1/evals/propose-process`, requires `admin` and accepts a known Launch
directory's `owner/repo` plus an optional `limit`, rather than a filesystem path.

The new suite has three Trials per Eval task. Its dependencies include the
implement, tdd, codebase-design, code-review and security-audit Factory skills,
Built-in process files and their Rust engine, and the other suites' Session
prompt, Harness, Model and Effort sources. Selection remains manifest-driven.

## One-time import of #429's material

Run `thirdshift evals import-replays` on the host holding the private material.
It reads the paths used by `docs/research/second-opinion-replays/replay.py`:

```text
~/.thirdshift/replays/
├── inputs-2026-10-07/manifest.json   # timing_bugs and spec_reviews
└── 2026-10-09/
    ├── inputs/manifest.json         # codex_bugs
    ├── context/<repo>/<issue>.json  # cached original title/body/parent/tickets
    └── grading/eval-import.json     # catch criteria and scored findings
```

The replay manifests retain their original shape: timing bugs have `id` and
`introducing_pr`; reverse bugs have `id` and `introducing_prs`. Each PR supplies
`number`, `merge_base`, `head_as_merged` and `implements_issue`. Spec entries
supply `repo`, `spec_issue`, `merge_base`, `pre_review_head`, `spec_issue_url`,
`spec_pr_url`, `base_branch` and `spec_branch`. Other replay
fields (plans, recorded sessions) are ignored. Bug IDs may be
numbers or directory-safe strings, optionally prefixed with `#`.

The replay script does not define a machine-readable grading format. Put the
existing written catch criteria and real/wrong decisions in this grading index
on the private host before importing; do not infer missing grades. This invented
example documents its complete shape:

```json
{
  "bugs": {"7": "Name the invented lost update and its consequence."},
  "specs": {
    "invented-counter-19": [
      {"finding": "The invented retry loses a write.", "score": "real"},
      {"finding": "The invented counter is too small.", "score": "wrong"}
    ]
  }
}
```

Keys in `bugs` match manifest IDs exactly (including `#`, if present); Spec keys
are `<repo>-<spec_issue>`. A Spec with no known findings has an explicit `[]`.
Missing criteria, missing Spec grades, blank findings and scores other than
`real` or `wrong` fail the import. An `unsure` decision must be resolved first.
Both replay manifests and every referenced cached issue body are required;
the importer never fetches issues, repositories or grades from the network.
Local source repositories are `~/repos/<repo>/`, as in the replay script;
bug repositories are `~/repos/thirdshift/`.

Code-review imports include their review-only `[trial]` inputs: the original
thirdshift issue and PR URLs, a local `main` Base branch and `issue-<n>` Issue
branch. The PR URL is unused and no GitHub service is needed. Rerunning the
import fills this table in older tasks only when it is missing, preserving
local edits and any existing Trial inputs. Spec-review imports also include
`[trial]`: `issue_url` and `pr_url` copy `spec_issue_url` and `spec_pr_url`,
and `base` and `branch` copy `base_branch` and `spec_branch`. No PR service is
provided; the sandbox blocks `gh` calls and only the review reports are graded.

The importer creates approved Eval tasks in `code-review` (three Trials per Arm)
and `spec-review` (one). Each bug/introducing-PR pair becomes
`bug-<id>-pr-<number>`; each Spec becomes `<repo>-<spec_issue>`. Thus the nine
bugs may occupy more than nine Eval tasks when one bug spans several PRs.
Cached original issues, parents and Tickets become `.thirdshift-issue.md` inputs;
catch criteria and scored findings stay in `task.toml`, outside Trials. The
suites declare production review skills, prompts and Harness/default sources as
dependencies. No plan-check or reverse-Spec suite is imported.

All source data is checked before publishing either suite. Each new Eval task
is published as a complete folder. Repeating the command preserves existing
valid Eval tasks and suite declarations, even if edited after the first import,
apart from completing missing Trial inputs, and adds missing tasks.
Completed tables count as skipped tasks in the JSON counts, for example
`{"imported":4,"skipped":0}`, with the same exit rules as the other Eval commands.
Keep the source material and imported suites private; only made-up fixtures
belong in tests. Run the import itself only on the host that holds the material.

## Arm preparation

Custom runners use the symmetric selection form (the legacy positional build
form below remains available):

```sh
thirdshift evals prepare-arms <repository> <record.json> --baseline-runner old --candidate-runner new --suite process
thirdshift evals prepare-arms <repository> <record.json> --candidate-runner new --baseline HEAD --suite process
thirdshift evals prepare-arms <repository> <record.json> HEAD --baseline-runner old --suite process
```

Each runner is `~/.thirdshift/runners/<name>.toml` or an explicit `.toml` path.
The repository need not be a thirdshift checkout when both Arms are Custom
runners, and no Cargo build runs for those Arms. Only declared Harnesses are pinned. A native
Baseline defaults to HEAD when Candidate is a Custom runner. A runner conflicts
with a commit or Process/Harness/Model/Effort selection on the same Arm. Custom
runners require explicit process suites; review suites and `--release` are
refused before preparation or dispatch.

```toml
command = ["python3", "{runner_dir}/runner.py"]
git_exclude = [".scratch/"]
providers = ["openai"]
harnesses = { claude = [], opencode = ["anthropic/claude", "baseten/model"], kimi = ["kimi-code/kimi-for-coding"] }
[files]
"runner.py" = "runner.py"
"skills/example" = { home = ".agents/skills/example" }
[roles.implement]
model = "example-model"
effort = "high"
```

`command` is an argv array, with no shell splitting. `{runner_dir}` expands in
each argument, and the absolute request.json path is appended as the final
argument. The working directory is the prepared repository. `files` maps source
paths relative to the manifest (or absolute source paths) to bundle-relative
string targets, or `{ home = "relative/path" }` targets inside fresh HOME.
Absolute targets, `..`, overlaps and nonregular tree entries are refused.
`roles` is descriptive metadata; it does not set prices.

The Arm's `runner` records the parsed manifest, frozen `directory`, SHA-256
`files`, and source `provenance` (enclosing Git repository path, optional origin,
relative source path, HEAD and dirty boolean). Dirty local sources are allowed.
Hashes cover the original manifest and every mapped file. Directory entries
end in `/` and hash their ordered subtree, preserving empty folders. Each Trial
verifies both frozen Arms and copies verified bytes into its private runner directory
and HOME. A frozen-input mismatch stops the Experiment before launch.

Each Trial encloses the runner and all its children in one `bwrap` sandbox
inside the Trial's memory scope. `harnesses` maps CLI names to Model lists:
OpenCode requires `<provider>/<model>` entries and Kimi requires configured
aliases; other Harnesses may use an empty list. Preparation pins real binary
paths and `--version`, and resolves the declared provider hosts before Trials.
Only declared Harness logins are copied into fresh HOME; operator settings,
instructions and other logins are excluded unless explicitly mapped by `files`.
Grok holds the shared Trials login lock for the whole Trial. OpenCode gets one
config covering all its declared providers, with `webfetch` and `websearch` denied.
Executable wrappers at the front of PATH apply the same web-search restrictions
as native Trials and exec the pinned Harness, preserving its streams and status.

`providers` declares direct-SDK access: `anthropic`, `openai`, `xai`, `meta`,
`google`, `fireworks-ai`, `baseten` or `kimi`. Supply Trials-only keys as quoted
strings under `[providers]` in `~/.thirdshift/credentials.toml` (mode 600),
for example `openai = "..."`. Only declared keys enter the sandbox, as
`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `XAI_API_KEY`, `META_API_KEY`,
`GOOGLE_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY` or `KIMI_API_KEY`.
Missing declared logins or keys produce `session_unavailable` Void Trials.
One HTTPS proxy allows the union of declared Harness and SDK provider hosts;
GitHub, general web hosts and unbound host paths remain unreachable. Every
declared provider family participates in the panel's Arm-family voting and
stand-in breadth checks. `THIRDSHIFT_TRIAL_NO_SANDBOX` remains the test/emergency
escape from confinement.

Each Trial starts the runner with an empty environment. thirdshift sets `PATH`,
`HOME` to that Trial's fresh home, the home's XDG base directories,
`THIRDSHIFT_EVAL_LIMIT_SIGNAL`, `CARGO_HOME` when a toolchain was prepared,
`GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, and `LANG`, `LC_ALL` or
`LC_CTYPE` when the operator set one and it is a single locale token. The
runner does not receive the rest of the operator's environment. The sandbox also
sets the declared credentials and proxy variables.

Custom runners receive version-1 request.json with `repository`, `issue_file`,
`issue_url`, `base`, `branch`, `runner_dir`, `output_dir` and `record_command`.
`record_command` is an argv prefix. Append `--role <role> -- <harness> [args...]`
to wrap a supported Harness CLI call. It preserves stdin, stdout, stderr and
exit status while sending events to the host recorder. OTLP `.logs.jsonl` and
`.spans.jsonl` stream live into the Trial's read-only `session/` folder; an 85%
Claude or Codex quota warning pauses the Experiment with `usage_limit`.
The runner receives EOF on stdin and writes `output_dir/result.json`, for example:

```json
{"version":1,"outcome":"delivered"}
```

Other outcomes are `stopped_to_ask`, `failed` and `void`. Void requires
`void_cause` equal to `session_unavailable`, `usage_limit` or `rate_limit`;
optional `cause` explains the outcome. Outer supervision retains precedence.
Missing or malformed results, unexplained nonzero exits and self-reported outer
Void causes fail the Trial, with exit status and stderr retained. Frozen
`git_exclude` patterns are appended to `.git/info/exclude` beside the issue
file exclusion; matching committed HEAD paths also fail the Trial. Code Graders
and Pair evidence grade committed HEAD, ignoring uncommitted files (listed in
attempt diagnostics). No simulated user answers questions. `stopped_to_ask`
fails its Score and appears in the outcomes tally.

For post-run recording, write `output_dir/sessions.json` as an array. Each
entry names a `role`, RFC3339 `started_at` and `ended_at`, and either:

- `harness` plus `stream_file`, containing that Harness's raw stream; or
- `transcript_file` plus `models`, mapping observed Model names to
  `uncached_input_tokens`, `cache_read_tokens`, `cache_write_tokens` and
  `output_tokens`. Transcripts may contain normalized Harness-independent
  events, one JSON object per line, or plain text.

All files must be regular top-level files inside `output_dir`. The host converts
them into the same OTLP traces as live sessions, retaining transcripts for
fix-leak checks, Trial Peek, Waterfall and export. Pricing uses the Experiment's
frozen `prices.json`, including cache tokens in input totals, and ignores
runner-reported USD. Converted Claude and Codex streams also retain quota stops.
List each session once; do not list a live-recorded session again.

A web-search tool call in normalized Session evidence voids the Trial as
`web_search`. Host proxy evidence counts provider connections and non-probe
Harness invocations. Provider traffic with no sessions, or fewer Harness
sessions than Harness runs that contacted a provider, voids the Trial as
`accounting_gap`. A runner with declared `harnesses` or `providers` that ends
`delivered` or `stopped_to_ask` with no sessions also has an accounting gap.

Before its first Trial, an Experiment prepares both Arms together:

```sh
thirdshift evals prepare-arms <thirdshift-repository> <baseline-commit> <baseline-harness> <candidate-commit> <candidate-harness> <record.json>
```

Commits are full lowercase object IDs in the local thirdshift repository;
Harnesses use their CLI names (`claude`, `codex`, `agy`, `grok`, `muse`,
`opencode`, `kimi`). The command writes a format-1 JSON record to stdout and, atomically,
to a new file at `<record.json>`. Its parent directory must exist. It records
`baseline` and `candidate`, each with `commit`, the cached thirdshift `binary`,
and a `harness` holding its `name`, resolved `path` and `version`.

Append `--baseline-process <name-or-file.toml>` or
`--candidate-process <name-or-file.toml>` to freeze a Process per Arm; the
unspecified Arm then uses `ticket-run`. Names resolve Built-in processes or
Custom processes in `~/.thirdshift/processes/`. Each Arm's optional `process`
contains `name`, a private frozen `directory`, a `files` map from home-relative
paths to SHA-256 hashes, and `steps` with each Step's `name`, pinned `harness`,
`model` and `effort`. Home skills come from `~/.thirdshift/skills/`; their full
directories are frozen, including helpers. Factory skills stay in the build,
and repository skills stay in the Eval task. The flags `--baseline-model`,
`--baseline-effort`, `--candidate-model` and `--candidate-effort` set inherited
Step choices; fields explicitly set by a Step take precedence.

Frozen folders live beside the Arm record. Keep them with the record. Before
every Trial the Experiment checks both Arms' file sets and content hashes,
then copies verified bytes into that Trial's fresh home. A changed, added or
missing file stops the Experiment with its filename before that Trial starts.

Each missing build runs `cargo build --release --locked` in a detached worktree
at that Arm's commit, with a private target directory. The finished executable
is published at `~/.thirdshift/evals/builds/<commit>/thirdshift`. A per-commit
lock prevents concurrent Experiments from building the same commit twice.
The temporary worktree and target directory are removed after success or
failure. A failed build reports Cargo's output, publishes no cache entry or
Arm record, and prevents Trials from starting. A successful first Arm's cache
entry is reusable if preparing the other Arm fails.

Harnesses are resolved and queried with `--version` once per distinct Harness
before building either Arm. Version lookup failure stops preparation. Trial
launchers must use the recorded real path, including for version queries and
Resumes, rather than looking up the CLI name again. Moving an installation's
symlink then cannot switch the Harness between Arms. This fixes symlink-based
updates; replacing the executable in place is outside that guarantee.

This command prepares the runner's inputs; it starts no Trials. Tests seed
`builds/<commit>/thirdshift` with the test binary for both Arms, and use fake
Cargo commands when exercising cache misses, so they compile no Arm builds.
