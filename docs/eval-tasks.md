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
session_prompts = ["src/prompt.rs"]
harness_defaults = ["src/harness/", "src/config.rs"]
model_defaults = ["src/config.rs", "src/setup.rs"]
effort_defaults = ["src/config.rs", "src/setup.rs"]
```

`trials_per_arm` is a positive integer **per Eval task**. For example, the
code-review suite uses three and the Spec-review suite uses one. All five
dependency categories must be declared; an unused category is `[]`. Each entry
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
```

- `state` is `proposed` or `approved`. Proposals are listed with their state and
  are never eligible for preparation or Trials. Approval is a separate workflow
  (#1021); these commands do not approve anything.
- `session` is `code-review` or `spec-review`.
- `repository` names a local Git repository holding both commits, absolute or
  relative to the Eval task's folder. Preparation never fetches from the network.
- `base` and `change` are full commit object IDs (SHA-1 or SHA-256). The base must
  be an ancestor of the change. `base` is the last history visible to a Trial;
  `change` supplies the working-tree diff under review. They may be equal for a
  Spec-review Eval task needing no initial code diff.
- `criteria` is nonempty written success criteria for a model Grader. `checks`
  is optional (defaults to `[]`), with one nonempty argument vector per code
  Grader check. Arguments are not shell text. Checks are recorded here, not
  executed by preparation.
- `inputs` maps destination paths in the prepared repository to files relative
  to the Eval task's folder. Use, for example, a Spec's body **as first
  published**, not its revised answer. The table is required but may be empty.
  Paths must stay within their respective folders, and cannot address `.git`.
  Only these input files are copied; the manifest, criteria and checks remain
  outside the prepared repository.
- `trial` supplies the version-1 Trial's session inputs. It is optional for
  standalone preparation, and required for approved tasks selected by an
  Experiment. `base` and `branch` name distinct local branches; `issue_url`
  and `pr_url` describe the fixture's issue and PR. Spec-review Experiments
  create those local branches, commit the prepared change and push them to
  the disposable bare origin. Supply fixture services or disposable PRs for
  sessions that can reach GitHub, as described in the Trial protocol.

## Commands and repository preparation

```sh
thirdshift evals list
thirdshift evals touched skills/thirdshift-code-review/SKILL.md src/prompt.rs
thirdshift evals prepare reviews invented-counter /tmp/invented-trial
thirdshift evals verify /tmp/invented-trial 1111111111111111111111111111111111111111
```

All four commands write JSON to stdout, exit `0` on success, and exit `1` with
diagnostics on failure (argument errors exit `2`). `list` includes each suite's
dependencies, Trials per Arm and each Eval task's name, state and session.
`touched` returns a sorted array of suite names, once each, for the supplied
changed paths. With no changed paths it returns `[]`.

`prepare` requires an approved Eval task and a destination that does not exist.
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
The bare origin receives the same history verification. Preparation returns
`repository` and `origin` paths (`origin` is `null` for code review); `verify`
returns `{"safe":true}` on success. Keep or delete the entire destination for
the lifetime of its Trial; the repository and any bare origin are throwaway.

These commands prepare data for the Experiment runner; they start no Harness,
Trial or Grader. Isolation of the session's home, processes and network belongs
to the runner's sandbox, not to this Git preparation step.

## Arm preparation

Before its first Trial, an Experiment prepares both Arms together:

```sh
thirdshift evals prepare-arms <thirdshift-repository> <baseline-commit> <baseline-harness> <candidate-commit> <candidate-harness> <record.json>
```

Commits are full lowercase object IDs in the local thirdshift repository;
Harnesses use their CLI names (`claude`, `codex`, `agy`, `grok`, `muse`,
`opencode`). The command writes a format-1 JSON record to stdout and, atomically,
to a new file at `<record.json>`. Its parent directory must exist. It records
`baseline` and `candidate`, each with `commit`, the cached thirdshift `binary`,
and a `harness` holding its `name`, resolved `path` and `version`.

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
