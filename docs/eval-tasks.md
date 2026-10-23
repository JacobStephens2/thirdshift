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
supply `repo`, `spec_issue`, `merge_base` and `pre_review_head`. Other replay
fields (plans, branch names, recorded sessions) are ignored. Bug IDs may be
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
and adds only missing ones. It prints JSON counts, for example
`{"imported":4,"skipped":0}`, with the same exit rules as the other Eval commands.
Keep the source material and imported suites private; only made-up fixtures
belong in tests. Run the import itself only on the host that holds the material.

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
