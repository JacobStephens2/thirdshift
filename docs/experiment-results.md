# Experiment results (format 1)

`thirdshift experiments <id> --json` emits one UTF-8 JSON object matching
`Experiment` below. `thirdshift experiments --json` emits an array of those
objects. The starting command, `thirdshift experiment ... --json`, and
`thirdshift recheck ... --json` return the same view. Progress goes to stderr.
These results complement the [Eval task input format](eval-tasks.md) and
[Trial protocol](trial.md); there is no separate export command.

## Stability and versioning

`format: 1` identifies this result contract. All fields and types below, their
presence/null rules, enum values and meanings are stable within format 1.
Adding, removing, renaming or retyping a field, changing its meaning or presence
rules, or changing an enum requires a new format number and an updated document
and drift test in the same change. Readers should reject unsupported formats.
Object key order, whitespace, diagnostic/reason text, absolute paths, opaque IDs
and the contents of review reports are not stable. Dynamic map keys (suite names,
Model names and Void causes) vary with the Experiment. Unknown measurements are
`null`, never zero. Timestamps are RFC 3339 strings; paths are JSON strings local
to the machine retaining the Experiment.

The private `experiment.json` already has `format: 1`. The CLI view projects its
metadata and retained evidence; it is not a copy of that internal storage file.
Older CLI builds without a `format` field predate this documented contract.
The Arm record's `prepared.format` and each attempt's `version` identify their
own formats, independently of the result format.

## Experiment view

`id` identifies the fixed pair of Arms and selected suites. `baseline` and
`candidate` hold resolved commits and requested Harness/Model/Effort; a `null`
Model or Effort leaves the choice to the Harness. `prepared` holds the pinned
binaries and Harness executables/versions (the [Arm record](eval-tasks.md)).
`suites` lists suite names; `tasks` lists the fixed approved Eval tasks by suite
and name, without disclosing their private inputs or success criteria.

`state` is `prepared`, `running`, `completed`, `interrupted` or `paused`.
`completed` means grading finished; its Verdict can still wait on rechecks.
`created_at` and nullable `ended_at` bound the Experiment. Nullable `baseline_ref`,
`candidate_ref` and `title` are display descriptions; `pause_reason`,
`resume_command` and `command_log` supply recovery and factory-log context.
They do not participate in Experiment identity or the Bar.

`running_trials` gives active Trial IDs and start times. `sessions` inventories
retained Trial Sessions: opaque Session ID, Trial ID, zero-based attempt index
and whether the readable file is an OTLP trace. Read one with
`thirdshift experiments <id> --session <session-id> --json`; this returns
`SessionContents` below (`contents` is UTF-8 text, not parsed JSON).
IDs are opaque: use the returned ID rather than constructing a filename.

`results` holds terminal Trials. `unfinished_trials` holds checkpointed attempts
awaiting continuation; a paused Experiment can have both. `minutes` sums each
Arm's completed attempt durations, including voids and timeouts. `voids` counts
completed Void attempts and groups them by cause, including unfinished Trials.
`findings` contains human-readable resource diagnostics, currently repeated
memory-cap kills. `usage` groups observed tokens and USD costs by Arm and Model;
unreported Model identity uses the key `unknown`. Input tokens include cache
reads and writes; cache counters are also reported separately. Partial totals
follow the [guide's Session usage rules](guide.md#experiments).

## Trial files and Scores

Private artifacts live under `~/.thirdshift/evals/experiments/<id>/`:

```text
experiment.json                  # fixed inputs, state, Verdict, recheck history
results/<trial-id>.json           # one terminal TrialResult
trials/<trial-id>/pending.json    # checkpointed TrialResult before publication
trials/<trial-id>/attempt-<suffix>/ # retained Trial protocol output and evidence
```

Trial result files have the `TrialResult` shape below, governed by their parent
Experiment's format. They have no separate format field. Trial indices are
zero-based within an Eval task and Arm; `outcome` is `succeeded`, `failed` or
`void`. `attempts` retains each Trial protocol result in execution order,
including retries/resumes. Its `version: 1`, timing milliseconds, nullable
failure cause, Void cause, Session ID, log, Model evidence and report paths are
defined by the [Trial result protocol](trial.md#result). Report text and Session
trajectories stay in those files; [OTLP export](guide.md#experiments) remains
the trajectory export.

`score` is nullable until graded. `checks` is an ordered list of mechanical
checks: nullable argv `command` (null for built-in checks), boolean `passed`
and human-readable `reason`. `model` is a nullable Model Grader decision with
its blind packet `label`, decision, reason, wrong-finding flag and borderline
flag. The Score passes only if all mechanical checks and the Model Grader pass.
Mechanical checks alone cannot judge a wrong finding. `borderline` flags a
Score needing the Day shift's judgment.

The original Score and Model decision remain unchanged after rechecking.
The CLI overlays `score.rechecks` in recording order, omitting that key when
empty. The latest recheck's `passed` is effective; its `wrong_finding` overrides
the original only when non-null. A null value leaves the original wrong-finding
flag in force. Each entry retains `reason` and `recorded_at`. Terminal result
files retain original Scores; the history is stored in `experiment.json` and
overlaid in the view. Do not rewrite the result files to apply a recheck.

## Verdict and Bar

`verdict` is null until assessment. Its `verdict` is `pass`, `fail`,
`inconclusive` or `waiting_on_rechecks`. `provisional_verdict` is the paired
decision before required rechecks; `borderline_trials` counts borderline Scores.
`pending_rechecks` names Trials requiring review, with `borderline` and
`decisive` flags. `decisive_flips` says whether flipping `passed`,
`wrong_finding`, or both would change the whole Experiment's decision.

`verdict.suites` maps suite names to paired Verdicts. Each retains its statistical
`bar`, producer `thirdshift_version` and seeded sampling `method` description,
`accuracy` and `wrong_findings` assessments, final decision, retained/dropped
Eval task counts and nullable `pass_cubed`. The latter is present as an object
only for three-Trial suites: each Arm's share of retained Eval tasks for which
all three Trials passed.

Each assessment has `difference_counts`, ordered from -1 to +1 in steps of
1 / Trials per Eval task, its credible `interval` (`lower`, `upper`), decision
and nullable `more_tasks` forecast. Differences are Candidate minus Baseline;
for wrong findings, positive means fewer wrong findings in the Candidate.
`more_tasks.target_probability` is the forecast target; `additional_tasks`
is null if the search budget ran out. `clear_probabilities[k-1]` estimates the
chance of a clear Verdict after k additional completed, non-void Eval tasks.

The Experiment's top-level `bar` fixes the margin, credible level, prior, seed,
draw count, per-suite Trials per Eval task, per-Trial timeout in seconds and
Void cause/retry policy. `void_rules.retries` counts fresh retries after the
first attempt (at most two). Suite Bars hold the corresponding scalar
`trials_per_task` and the posterior prediction budget (`simulations`,
`interval_draws`, `max_additional_tasks`), rather than the execution policy.
Any failed suite fails the Experiment; otherwise an inconclusive suite or more
than a tenth of tasks dropped as void makes it inconclusive. Required rechecks
withhold the final decision as `waiting_on_rechecks`.

## Checked field contract

This JSON type notation is part of the document, read directly by
`cargo test --test experiments result_format`. A string names a primitive type
or another definition; `a|b` lists allowed string values. `integer` means an
unsigned integer; `number` permits integer or fractional JSON numbers. A suffix
`?` on a type permits null; a suffix `?` on a field name permits omission.
Other fields are required, including nullable fields. A one-item array means
an array of that item type (possibly empty). `{"*": T}` is a string-keyed map
of T; all other objects have exactly the listed fields. Numeric literals are
exact values. The test checks real CLI output from fake Harnesses and retained
Trial files, rejecting undocumented, missing and retyped fields recursively.

<!-- result-contract -->
```json
{
  "Experiment": {
    "format": 1,
    "id": "string",
    "baseline": "Setup",
    "candidate": "Setup",
    "suites": ["string"],
    "bar": "Bar",
    "prepared": "Prepared?",
    "state": "prepared|running|completed|interrupted|paused",
    "baseline_ref": "string?",
    "candidate_ref": "string?",
    "title": "string?",
    "ended_at": "string?",
    "pause_reason": "string?",
    "resume_command": "string?",
    "command_log": "string?",
    "created_at": "string",
    "tasks": [{"suite": "string", "name": "string"}],
    "running_trials": [{"id": "string", "started_at": "string"}],
    "sessions": [{"id": "string", "trial_id": "string", "attempt": "integer", "trace": "boolean"}],
    "results": ["TrialResult"],
    "unfinished_trials": ["TrialResult"],
    "verdict": "Verdict?",
    "pending_rechecks": ["Pending"],
    "minutes": {"baseline": "number", "candidate": "number"},
    "usage": {"baseline": "Usage", "candidate": "Usage"},
    "voids": {"baseline": "Voids", "candidate": "Voids"},
    "findings": ["string"]
  },
  "Setup": {"commit": "string", "harness": "string", "model": "string?", "effort": "string?"},
  "Prepared": {"format": 1, "baseline": "PreparedArm", "candidate": "PreparedArm"},
  "PreparedArm": {
    "commit": "string", "binary": "string",
    "harness": {"name": "string", "path": "string", "version": "string"}
  },
  "Bar": {
    "margin": "number", "credible_level": "number", "prior": "number",
    "seed": "integer", "draws": "integer", "trials_per_task": {"*": "integer"},
    "timeout_seconds": "integer", "void_rules": {"retries": "integer", "causes": ["VoidCause"]}
  },
  "VoidCause": "interrupted|session_unavailable|usage_limit|rate_limit|repository_changed|sandbox_unavailable|host_killed|memory_cap",
  "TrialResult": {
    "suite": "string", "task": "string", "arm": "baseline|candidate", "trial": "integer",
    "outcome": "succeeded|failed|void", "score": "Score?", "attempts": ["Attempt"]
  },
  "Attempt": {
    "version": 1, "outcome": "succeeded|failed|void",
    "timings": {"total_ms": "integer", "session_ms": "integer"},
    "void_cause": "VoidCause?", "cause": "string?", "session_id": "string?",
    "log": "string?", "model_evidence": "string?",
    "reports": {"standards": "string?", "spec": "string?"}
  },
  "Score": {
    "passed": "boolean", "wrong_finding": "boolean", "borderline": "boolean",
    "model": "ModelScore?", "checks": ["Check"], "rechecks?": ["Recheck"]
  },
  "ModelScore": {
    "label": "string", "passed": "boolean", "reason": "string",
    "wrong_finding": "boolean", "borderline": "boolean"
  },
  "Command": ["string"],
  "Check": {"command": "Command?", "passed": "boolean", "reason": "string"},
  "Recheck": {"passed": "boolean", "reason": "string", "wrong_finding": "boolean?", "recorded_at": "string"},
  "Pending": {
    "id": "string", "suite": "string", "task": "string", "arm": "baseline|candidate",
    "trial": "integer", "borderline": "boolean", "decisive": "boolean",
    "decisive_flips": ["passed|wrong_finding"]
  },
  "Verdict": {
    "verdict": "pass|fail|inconclusive|waiting_on_rechecks",
    "provisional_verdict": "Decision?", "suites": {"*": "SuiteVerdict"}, "borderline_trials": "integer"
  },
  "Decision": "pass|fail|inconclusive",
  "SuiteVerdict": {
    "bar": "SuiteBar", "thirdshift_version": "string", "method": "string",
    "accuracy": "Assessment", "wrong_findings": "Assessment", "verdict": "Decision",
    "retained_tasks": "integer", "dropped_tasks": "integer", "pass_cubed": "PassCubed?"
  },
  "SuiteBar": {
    "margin": "number", "credible_level": "number", "prior": "number",
    "seed": "integer", "draws": "integer", "trials_per_task": "integer",
    "prediction": {"simulations": "integer", "interval_draws": "integer", "max_additional_tasks": "integer"}
  },
  "Assessment": {
    "difference_counts": ["integer"], "interval": {"lower": "number", "upper": "number"},
    "verdict": "Decision", "more_tasks": "MoreTasks?"
  },
  "MoreTasks": {"target_probability": "number", "additional_tasks": "integer?", "clear_probabilities": ["number"]},
  "PassCubed": {"baseline": "number", "candidate": "number"},
  "Usage": {"models": {"*": "Measurement"}, "cost_usd": "number?"},
  "Measurement": {
    "input_tokens": "integer?", "output_tokens": "integer?",
    "cache_read_tokens": "integer?", "cache_write_tokens": "integer?", "cost_usd": "number?"
  },
  "Voids": {"count": "integer", "causes": {"*": "integer"}},
  "SessionContents": {"id": "string", "trace": "boolean", "contents": "string"}
}
```
