# Experiment results (format 10)

`thirdshift experiments <id> --json` emits one UTF-8 JSON object matching
`Experiment` below. `thirdshift experiments --json` emits an array of those
objects. The starting command, `thirdshift experiment ... --json`, and
`thirdshift recheck ... --json` return the same view. Progress goes to stderr.
These results complement the [Eval task input format](eval-tasks.md) and
[Trial protocol](trial.md); there is no separate export command.

## Stability and versioning

`format: 10` identifies this result contract. All fields and types below, their
presence/null rules, enum values and meanings are stable within format 10.
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
Format 2 replaced format 1's single credible level and interval with the
two-level Bar: `pass_level`, `fail_level` and `pass_rule` in each Bar,
`pass_interval` and `fail_interval` in each assessment, and
`worse_within_margin`. It also added `decisive_verdicts` to pending rechecks
and lets a saved single-level Verdict omit `provisional_verdict`. Experiments
saved before the two-level Bar still show `credible_level` and `interval`.
Format 2 shipped in v0.17.4. Format 3 adds `bar.discriminating_tasks` and each
suite's `counts` and `measurement` fields for `paired-dirichlet-v4`, without
rewriting the stored Bar or Verdict of older Experiments.
Format 4 adds `bar.min_discriminating_tasks` and each suite's `advisory` flag.
Format 5 adds the prepared Arms' optional `process` with frozen file hashes and
per-Step Harness, Model and Effort. Older Arm records omit this field.
Only suites that are not advisory decide the overall Verdict and require
rechecks; every suite advisory gives inconclusive. Saved Experiments without
the minimum keep their original Bar, Verdict and aggregation policy.
The Arm record's `prepared.format` and each attempt's `version` identify their
own formats, independently of the result format.

Format 6 adds process Trial `leaks`, the `leak_hits` decision list,
`waiting_on_leaks` Verdict and `fix_leaked` Void cause. Each hit retains its
opaque ID, Trial ID, attempt index, Session ID, identifier kind and value, and
one-based Session evidence line. `decision` is null while pending; otherwise it keeps
`decision` (`confirm` or `dismiss`), `reason`, `reviewer` (the shared action
Actor) and `recorded_at`. Dismissed hits form the auditable false-positive list.
Confirmed hits overlay the Trial and matching attempt as Void in the view;
original Trial files remain intact. Pending hits hold the whole Verdict,
including hits from Advisory or measurement-only tasks.

Format 7 adds optional `bar.cost_ceiling` and `bar.time_ceiling`, and the
Verdict's optional `efficiency` and `line`. Saved Verdicts without these fields
retain their original shape and decision when read.

Format 8 adds optional `Setup.process_digest`, the content digest of a Custom
process and its referenced home skills. It identifies the Arm for Experiment
reuse and cost-estimate history. Built-in processes are identified by the build
and omit this field. Older saved setups also omit it.

Format 9 adds `pairs`: base-tests and cross-tests for process tasks whose
`process_grading.new_tests` names how to find and run an Arm's own new tests
([Eval task format](eval-tasks.md)). Baseline Trial *k* is paired with
Candidate Trial *k*. Each `PairSide` lists the Arm's `test_files`, the files its
final commit adds or changes since the base that match `new_tests.paths`. `base`
holds their checks on the unchanged base and `cross` their checks on the other
Arm's final commit, each with the code Grader's check shape. Only checks
whose command ran to an exit status decide the results. `fails_on_base` is
true when a base check failed that way and false when all passed;
`passes_on_other` is true when every cross check passed and false when one
failed. Each is null when the Arm has no new tests, when its run did not
finish (a timeout or a snapshot that could not be prepared, named in its
checks), or when `error` says why that run could not start. A base result
already decided stays when only the other Arm's commit is missing.
`disagreement` is true exactly when `passes_on_other` is false: something to
look into, not a defect. Pairs are evidence,
never Scores: they change no Score, Verdict or Void cause. Experiments
without `new_tests`, and pairs not yet finished, have none.

Format 10 adds the Grader panel: the Bar's optional `panel` and the
Experiment's `panel` reports. `bar.panel` lists the `seats` (Harness, Model and
optional Effort), the `vote` unit (`family`, the default, or `model`) and
`arm_family_rule` (on by default). Each seat reads every pair of process
Trials in a blind packet, once in each presentation order, with the sides
swapped in the second. Each `PanelPair` lists every seat as a `PanelGrader`:
its `family` (its provider: the Harness's own, or the provider an OpenCode
Model names), whether it `votes`, and its two `orders`. Each `PanelCall` names
the Arm `shown_as_a`, gives each Arm's per-criterion `rubric`, merge
reason and `ready_to_merge` call, then the comparison's `reason` and
`preference`. A Grader's `preference` and `merge` calls stand only when both
orders agree; a call that flips with the order is `tie`. With
`arm_family_rule`, a seat from either Arm's family does not vote while a seat
from another family sits on the panel. `votes` lists the units that counted:
one per family, each taking its own majority first, or one per Model (seats
naming the same Model at different Efforts or on different Harnesses take
their own majority first, as a family does). A call
needs more than half of the votes, otherwise it is `tie`; the panel's tied
merge call is `not_ready`. Pairs appear once every seat has called both
orders. The panel is reported beside the Verdict and changes no Score,
Verdict or Void cause. Experiments without a panel have none.

Format 10 also adds each `PairSide`'s `diff`, the Grader panel's optional
`panel` calls per `Pair`, and `people`. `diff` lists every file the Arm's
final commit changes since the base, code files before tests (`test` is true for a file
matching `new_tests.paths` or in a test location), each with its `git diff`
text: the blind pairwise page reads this copy once the Trial repositories are
gone. `panel`, when present, is the panel's merge call per Arm (a tied call is
not ready) and its preference. `people` holds the Day shift's blind pairwise
calls ([guide](guide.md#experiments)): one `PairReview` per sealed pair of
merge calls, in Arm terms, with the preference and optional rubric the reader
then gave (`preference` is null until given), in record order per pair.
A reader's merge calls are sealed until their preference; a review begun after
that reader's own preference, once the Arms were revealed, has `blind` false.
`agreement` compares each reader's latest blind merge calls with the panel's,
per side, on pairs the panel has read; `compared` is zero when there are none.

`thirdshift experiments <id> --pair <pair-id> --json`, and its `--merge` and
`--prefer` forms, emit `PairPage`: what the calling reader may see of one pair.
`a` and `b` are the two `PairSide`s on the sides drawn for the pair from the
Bar's seed, with no Arm named. `ticket` is null when the Eval task's Ticket
input can't be read; `checklist` holds the items under its "Acceptance
criteria" heading. `next` is the reader's next call. `mine` lists the reader's
records and `revealed` stays null until the reader has given a preference;
it then names the Arm on each side with its `Setup`, the panel's calls, and
every reader's records. Each record is kept as
`reviews/<pair-id>/<n>.json` with the `PairRecord` shape; records are only
ever added, numbered from `000000`, beside a `lock` file.
People's calls are reported beside the Verdict and change no Score, unless
the Bar counts them.

Format 10 also adds who decides a Verdict ([ADR 0028](adr/0028-who-decides-a-verdict-is-fixed-in-the-bar.md)).
`bar.panel.role` is `beside` (the default) or `counted`; `families` (2 by
default) is how many families the voting panel must span to stand in for
people, and `agreement` (0.8 by default) the least people–panel agreement, as
a share of merge calls on the people's sample, at which it may. `bar.people`,
absent when anyone may read any pair beside the Verdict, sets the people's
`role` (`off`, `beside` or `counted`), the `readers`, the `sample` of pairs per
reader (null when every reader reads every pair) and the `overlap` every
reader's sample shares. The sample is drawn from the judged pairs of process
Trials by the Bar's seed: the first `overlap` pairs drawn go to every reader,
then each reader takes the next pairs alone. `people.queues` lists each
reader's pairs. Readers need every judged process Eval task to name
`process_grading.new_tests`, since people read pairs on their pair page. With
the people `off`, merge calls are refused.

When either is counted, a Trial of a judged process Eval task passes only if
its Score passes and whoever decides its pair calls it ready, and the Verdict
gains `deciders`. The people's call on a pair is each reader's latest blind
merge call; on a pair several readers share, an Arm is ready only if every
reader calls it ready. With only the panel counted, the panel decides every
pair. With only people counted on a sample, the Verdict rests on the sampled
pairs alone, each pair a unit of the statistics (the suite's `bar` then has one
Trial per Eval task), and `left_out_pairs` counts the rest. With people counted
on every pair, they decide every pair. With both counted, people decide the
pairs they read and the panel stands in on the others; the Bar is refused
before the first Trial when the voting panel spans fewer than `families`
families, and the Verdict is `inconclusive` when `agreement`, people–panel
agreement on the sample counted per reader and side, is below the Bar's level,
with `inconclusive` saying why. `people_pairs`, `panel_pairs` and
`waiting_pairs` count the judged pairs, outside Void-dropped tasks, each
decided. The Verdict is `waiting_on_people` while a reader has not called a
pair given to them; its `provisional_verdict` counts such pairs on their
Scores alone. A reader's call recalculates a completed Experiment's Verdict.
`people.people_agreement` compares readers' latest blind merge calls per side,
two readers at a time, on pairs more than one has read.

Process Trials add `delivered` and `stopped_to_ask` outcomes. Stopped-to-ask is
scored as failed. `outcomes` reports terminal delivered, stopped-to-ask and
failed counts per Arm, excluding Void attempts. `Setup.process`, when present,
names the selected Process or Custom process file. Each process attempt's `steps` retains names,
outcomes, start/end times and elapsed milliseconds; all sessions remain in the
shared `sessions` listing. Existing review records omit these optional fields.

## Experiment view

`id` identifies the fixed pair of Arms and selected suites. `baseline` and
`candidate` hold resolved commits and requested Harness/Model/Effort; a `null`
Model or Effort leaves the choice to the Harness. `prepared` holds the pinned
binaries and Harness executables/versions (the [Arm record](eval-tasks.md)).
`suites` lists suite names; `tasks` lists the fixed approved Eval tasks by suite
and name, without disclosing their private inputs or success criteria.

`state` is `prepared`, `running`, `completed`, `interrupted` or `paused`.
`completed` means grading finished; its Verdict can still wait on rechecks.
`name` is the saved name when `name_set` is true, otherwise the readable default
derived from the Arms (including `A/A at <ref>` and `Release <ref>`). Names can
change in any state without changing the Experiment's identity, Bar or results.
`created_at` and nullable `ended_at` bound the Experiment. Nullable `baseline_ref`,
`candidate_ref` and `title` are display descriptions; `pause_reason`,
`resume_command` and `command_log` supply recovery and factory-log context.
`release` records whether `--release` has run for this Experiment and remains
true on later resumes. These fields do not participate in Experiment identity
or the Bar.

`running_trials` gives active Trial IDs and start times. `sessions` inventories
retained Trial Sessions: opaque Session ID, Trial ID, zero-based attempt index
and whether the readable file is an OTLP trace. Read one with
`thirdshift experiments <id> --session <session-id> --json`; this returns
`SessionContents` below (`contents` is UTF-8 text, not parsed JSON).
IDs are opaque: use the returned ID rather than constructing a filename.

`results` holds terminal Trials. `unfinished_trials` holds checkpointed attempts
awaiting continuation; a paused Experiment can have both. `minutes` sums each
Arm's completed attempt durations, including voids and timeouts. Time spent
paused between invocations at a usage limit is outside those durations.
`voids` counts
completed Void attempts and groups them by cause, including unfinished Trials.
`findings` contains human-readable resource diagnostics, currently repeated
memory-cap kills. `usage` groups observed tokens and USD costs by Arm and Model;
unreported Model identity uses the key `unknown`. Input tokens include cache
reads and writes; cache counters are also reported separately. Partial totals
follow the [guide's Session usage rules](guide.md#experiments).

`pricing` is the frozen, dated token price table for this Experiment. New
Experiments copy the bundled standard-rate snapshot; `experiment ... --prices
<file.json>` supplies a replacement with `date`, `source` and `models` (exact
observed Model names). Each rate has `input`, `output`, `cache_read` and
`cache_write` in USD per million tokens. These are comparison prices, separate
from the Harness-reported costs in `usage`. The bundled snapshot uses the
shortest cache-write TTL; provide a table for your billing tier or other Models.
After any Trial starts, prices cannot change. Resuming without `--prices` keeps
the saved table; passing it again checks that the contents still match.

`accounting` reports each Trial's cost and active attempt time, its sessions with
their attempt indices, observed Models, and the role of the Step that started
them. Resumes and fallback sessions belong to that same Step. Arm totals and
`roles` include all spent attempts, including Voids. A missing session usage
measurement (input or output), unknown observed Model, or missing frozen price
leaves cost null and makes a Trial Void with an `accounting_gap` cause naming
the gap. Existing external Void causes retain their cause and pause behavior;
an accounting gap in an earlier attempt still voids a later completed Trial.
An attempt stopped by the operator retains its Session facts with
`accounting.incomplete: true`; it does not consume the completed-attempt retry
budget. Completed interrupted Voids retain the saved Bar's retry policy.

`paired_median_ratios` pairs terminal, non-Void Trials by suite, task and Trial
index, divides Candidate by Baseline within each pair, then takes the median.
It includes failed quality outcomes when their accounting is complete. Zero
Baseline denominators are omitted for that metric; no eligible ratios means
null. Old Experiments without a price table keep their original outcomes and
show unknown priced costs. Pre-run estimates use the suite and the exact Arm
setup (build, Harness, Model, Effort and Process) from earlier non-Void Trials,
including every attempt's time and any complete priced cost.

## Trial files and Scores

Private artifacts live under `~/.thirdshift/evals/experiments/<id>/`:

```text
experiment.json                  # fixed inputs, state, Verdict, recheck history
results/<trial-id>.json           # one terminal TrialResult
trials/<trial-id>/pending.json    # checkpointed TrialResult before publication
pairs/<pair-id>.json             # one pair's diffs, base-tests and cross-tests (evidence)
reviews/<pair-id>/<n>.json       # one append-only blind pairwise record
trials/<trial-id>/attempt-<suffix>/ # retained Trial protocol output and evidence
```

Trial result files have the `TrialResult` shape below, governed by their parent
Experiment's format. They have no separate format field. Trial indices are
zero-based within an Eval task and Arm; `outcome` is `succeeded`, `delivered`,
`stopped_to_ask`, `failed` or
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

`--cost-ceiling` and `--time-ceiling` set positive finite Candidate/Baseline
ratios before the first Trial; both are off by default. Each ceiling compares
the paired median ratio, including complete non-Void Trials from all suites.
A ratio strictly above a ceiling fails the Experiment. A missing ratio or
incomplete accounting under a set ceiling cannot pass (inconclusive unless
another metric fails). Zero Baseline denominators are unmeasured.

`efficiency.quality_verdict` retains the quality decision before ceilings;
suite quality statistics and ADR-0025's decision are unchanged. `cost` and `time`
each report `ratio` (paired median), `interval` (nullable), valid `pairs`,
`complete` and nullable `ceiling`. The interval uses a Bayesian bootstrap of
the paired median at the Bar's pass level, seed and draw count. Independent
exponential weights are shared by all Trials of each suite/task pair; at least
two tasks and complete paired measurements are needed. It describes variation
over observed tasks, without adding unobserved tasks. `method` records this rule.
An interval wholly below 1 adds "cheaper" or "faster"; a median above 1 says
"costs more" or "takes longer". `line` combines the decision and these facts,
for example "pass: no worse, and cheaper". A passing quality regression inside
the margin keeps its "worse, within the margin" annotation. Pending rechecks
and leak decisions still hold the Verdict.

`verdict` is null until assessment. Its `verdict` is `pass`, `fail`,
`inconclusive` or `waiting_on_rechecks`. `provisional_verdict` is the paired
decision before required rechecks; `borderline_trials` counts borderline Scores.
`pending_rechecks` names Trials requiring review, with `borderline` and
`decisive` flags. `decisive_flips` says whether flipping `passed`,
`wrong_finding`, or both would change the whole Experiment's decision, and
`decisive_verdicts` maps each such flip to the provisional decision it gives.
A saved single-level Verdict may omit `provisional_verdict` or record it as
null; both are preserved as saved.

`verdict.suites` maps suite names to paired Verdicts. Each retains its statistical
`bar`, producer `thirdshift_version` and seeded sampling `method` description,
`accuracy` and `wrong_findings` assessments, final decision, retained/dropped
Eval task counts and nullable `pass_cubed`. The latter is present as an object
only for three-Trial suites: each Arm's share of retained Eval tasks for which
all three Trials passed.

New Experiments use `paired-dirichlet-v4`: only the names in the Experiment
Bar's per-suite `discriminating_tasks` list enter the statistics, predictions
and required rechecks. Each suite adds `counts`: `judged` counts retained listed
tasks, `measurement_only` counts retained unlisted tasks, `void_dropped` counts
all tasks dropped after exhausted Void retries, and `other_setup` counts judged
tasks whose recorded Difficulty's commit, Harness, Model or Effort differs from
the Baseline's. The first three partition the approved tasks. `retained_tasks`
and `dropped_tasks` refer only to the judged list, including for dropout guards.
`measurement` lists each unlisted task and its Trials: Arm, zero-based Trial
index, outcome, effective passed/wrong-finding flags and original borderline
flag. Full original Scores, rechecks and attempt evidence remain in `results`.
An empty judged list gives an inconclusive suite. Older Experiments omit the
list, counts and measurement, keeping their stored method, Bar and Verdict.

The Experiment Bar's `min_discriminating_tasks` defaults to 20 and is set by
`--min-discriminating-tasks`. Each suite's `advisory` flag says whether its fixed
discriminating list is shorter than that minimum, before Void dropouts.
Advisory suites retain their own Verdict and intervals but are excluded from the
overall decision, dropout guard and required rechecks. When every suite is
advisory, the overall Verdict is inconclusive. Saved Experiments without the
minimum omit `advisory` and retain their original aggregation policy.

Each assessment has `difference_counts`, ordered from -1 to +1 in steps of
1 / Trials per Eval task, its intervals (`lower`, `upper`), decision and
nullable `more_tasks` forecast. A two-level Bar gives `pass_interval` and
`fail_interval` at its pass and fail levels; a saved single-level Bar gives one
credible `interval`. `worse_within_margin`, present only when true, marks a
pass whose pass-level interval lies wholly below zero: worse, but within the
margin. It never changes the decision. Differences are Candidate minus Baseline;
for wrong findings, positive means fewer wrong findings in the Candidate.
`more_tasks.target_probability` is the forecast target; `additional_tasks`
is null if the search budget ran out. `clear_probabilities[k-1]` estimates the
chance of a clear Verdict after k additional completed, non-void Eval tasks.

The Experiment's top-level `bar` fixes the margin, levels, prior, seed,
draw count, per-suite Trials per Eval task, per-Trial timeout in seconds and
Void cause/retry policy. A two-level Bar records `pass_level`, `fail_level` and
`pass_rule`: `margin` means a pass needs only the pass-level lower bound above
the margin; `reaches_zero`, shown only by two-level Bars saved before that rule,
also needed the pass-level upper bound to reach zero. A saved single-level Bar
records `credible_level` instead. `void_rules.retries` counts fresh retries after the
first attempt (at most two). Suite Bars hold the corresponding scalar
`trials_per_task` and the posterior prediction budget (`simulations`,
`interval_draws`, `max_additional_tasks`), rather than the execution policy.
Among suites that are not advisory, any failed suite fails the Experiment;
otherwise an inconclusive suite or more than a tenth of their judged tasks
dropped as void makes it inconclusive. All suites advisory gives inconclusive. Required rechecks
withhold the final decision as `waiting_on_rechecks`.

## Paused Command records

An Experiment or Scoring run stopped by a plan-limit warning or rejection has
Command outcome `paused`, separate from a failure or interruption. `show --json`
and `status --json` include its saved `resume_command` and `limit`: `harness`,
`window`, fractional `utilization`, and `resets_at` in RFC 3339. Unknown facts
are null. These fields are optional on older or other Command records; older
`failed` records keep that outcome. Without a reported reset time the cause
keeps the earlier wording. With one it names the stopping window, utilization
and local reset time. The Inbox shows Paused in Needs you, with that cause and
a copyable resume command; starting the same Experiment or a later Scoring
run takes it out of Needs you. Dismissal also works. Finished Trials and Scores
are kept. There is no automatic resume, failure email or failure metric.

The `CommandPause` projection in the contract below covers those fields on
the Command record and on `show --json`; it does not change the Experiment view.

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
  "CommandPause": {
    "outcome": "paused|failed|interrupted",
    "resume_command": "string",
    "limit": {
      "harness": "string?", "window": "string?", "utilization": "number?", "resets_at": "string?"
    }
  },
  "Experiment": {
    "format": 10,
    "name": "string",
    "name_set": "boolean",
    "id": "string",
    "baseline": "Setup",
    "candidate": "Setup",
    "suites": ["string"],
    "bar": "Bar",
    "prepared": "Prepared?",
    "state": "prepared|running|completed|interrupted|paused",
    "pricing": "Prices?",
    "accounting": "Accounting",
    "baseline_ref": "string?",
    "candidate_ref": "string?",
    "title": "string?",
    "ended_at": "string?",
    "pause_reason": "string?",
    "resume_command": "string?",
    "command_log": "string?",
    "release": "boolean",
    "created_at": "string",
    "tasks": [{"suite": "string", "name": "string"}],
    "running_trials": [{"id": "string", "started_at": "string"}],
    "sessions": [{"id": "string", "trial_id": "string", "attempt": "integer", "trace": "boolean"}],
    "results": ["TrialResult"],
    "unfinished_trials": ["TrialResult"],
    "verdict": "Verdict?",
    "pending_rechecks": ["Pending"],
    "leak_hits": ["LeakHit"],
    "pairs": ["Pair"],
    "panel": ["PanelPair"],
    "people": {
      "reviews": ["PairReview"], "agreement": "Agreement", "people_agreement": "Agreement",
      "queues": [{"reader": "string", "pairs": ["string"]}]
    },
    "minutes": {"baseline": "number", "candidate": "number"},
    "usage": {"baseline": "Usage", "candidate": "Usage"},
    "voids": {"baseline": "Voids", "candidate": "Voids"},
    "outcomes": {"baseline": "OutcomeCounts", "candidate": "OutcomeCounts"},
    "findings": ["string"]
  },
  "Prices": {"date":"string", "source":"string", "models":{"*":"Rate"}},
  "Rate": {"input":"number", "output":"number", "cache_read":"number", "cache_write":"number"},
  "Accounting": {
    "baseline":"ArmAccounting", "candidate":"ArmAccounting", "trials":["TrialAccounting"],
    "paired_median_ratios":{"cost":"number?", "time":"number?", "pairs":"integer"}
  },
  "TotalAccounting": {"sessions":"integer", "cost_usd":"number?", "time_ms":"integer"},
  "ArmAccounting": {"sessions":"integer", "cost_usd":"number?", "time_ms":"integer", "roles":{"*":"TotalAccounting"}},
  "TrialAccounting": {"id":"string", "arm":"baseline|candidate", "cost_usd":"number?", "time_ms":"integer", "roles":{"*":"TotalAccounting"}, "sessions":["SessionAccounting"]},
  "SessionAccounting": {"attempt?":"integer", "path":"string", "role":"string", "time_ms":"integer", "models":{"*":"Measurement"}, "cost_usd":"number?", "gap":"string?"},
  "OutcomeCounts": {"delivered": "integer", "stopped_to_ask": "integer", "failed": "integer"},
  "Setup": {"commit": "string", "harness": "string", "model": "string?", "effort": "string?", "process?": "string", "process_digest?": "string"},
  "Prepared": {"format": 1, "baseline": "PreparedArm", "candidate": "PreparedArm"},
  "PreparedArm": {
    "commit": "string", "binary": "string",
    "harness": "FixedHarness", "process?": "FrozenProcess"
  },
  "FixedHarness": {"name": "string", "path": "string", "version": "string"},
  "FrozenProcess": {
    "name": "string", "directory": "string", "files": {"*": "string"},
    "steps": [{"name": "string", "harness": "FixedHarness", "model": "string?", "effort": "string?"}]
  },
  "Bar": {
    "margin": "number", "pass_level?": "number", "fail_level?": "number",
    "pass_rule?": "margin|reaches_zero", "credible_level?": "number", "prior": "number",
    "seed": "integer", "draws": "integer", "trials_per_task": {"*": "integer"},
    "discriminating_tasks?": {"*": ["string"]},
    "min_discriminating_tasks?": "integer",
    "cost_ceiling?": "number", "time_ceiling?": "number",
    "timeout_seconds": "integer", "void_rules": {"retries": "integer", "causes": ["VoidCause"]},
    "panel?": "Panel", "people?": "People"
  },
  "Panel": {
    "seats": [{"harness": "string", "model": "string", "effort": "string?"}],
    "vote": "family|model", "arm_family_rule": "boolean",
    "role": "beside|counted", "families": "integer", "agreement": "number"
  },
  "People": {
    "role": "off|beside|counted", "readers": ["string"], "sample": "integer?", "overlap": "integer"
  },
  "Agreement": {"compared": "integer", "agreed": "integer"},
  "VoidCause": "interrupted|session_unavailable|usage_limit|rate_limit|repository_changed|sandbox_unavailable|host_killed|memory_cap|accounting_gap|fix_leaked",
  "TrialResult": {
    "suite": "string", "task": "string", "arm": "baseline|candidate", "trial": "integer",
    "outcome": "succeeded|delivered|stopped_to_ask|failed|void", "score": "Score?", "attempts": ["Attempt"], "leaks?": ["LeakHit"]
  },
  "LeakHit": {
    "id": "string", "trial_id": "string", "attempt": "integer", "session": "string",
    "kind": "merge_commit|pull_request|issue|branch|title", "identifier": "string", "line": "integer", "decision": "LeakDecision?"
  },
  "LeakDecision": {
    "decision": "confirm|dismiss", "reason": "string", "recorded_at": "string",
    "reviewer": {"who": "string", "from_console": "boolean", "console_holder?": "string"}
  },
  "Attempt": {
    "version": 1, "outcome": "succeeded|delivered|stopped_to_ask|failed|void",
    "timings": {"total_ms": "integer", "session_ms": "integer"},
    "void_cause": "VoidCause?", "cause": "string?", "session_id": "string?",
    "log": "string?", "model_evidence": "string?",
    "reports": {"standards": "string?", "spec": "string?"},
    "steps?": [{"name": "string", "outcome": "string", "started_at": "string", "ended_at": "string", "elapsed_ms": "integer"}],
    "accounting?": {"sessions":["SessionAccounting"], "incomplete?":"boolean"}
  },
  "Score": {
    "passed": "boolean", "wrong_finding": "boolean", "borderline": "boolean",
    "model": "ModelScore?", "checks": ["Check"], "rechecks?": ["Recheck"]
  },
  "ModelScore": {
    "label": "string", "passed": "boolean", "reason": "string",
    "wrong_finding": "boolean", "borderline": "boolean"
  },
  "Pair": {"suite": "string", "task": "string", "trial": "integer", "baseline": "PairSide", "candidate": "PairSide", "panel?": "PanelCalls"},
  "PanelCalls": {"baseline_ready": "boolean", "candidate_ready": "boolean", "preference": "ArmPreference"},
  "ArmPreference": "baseline|candidate|tie|both_bad",
  "PairReview": {
    "pair": "string", "suite": "string", "task": "string", "trial": "integer",
    "reader": {"who": "string", "from_console": "boolean", "console_holder?": "string"},
    "recorded_at": "string", "blind": "boolean", "baseline_ready": "boolean", "candidate_ready": "boolean",
    "preference": "ArmPreference?", "rubric": {"*": "ArmPreference"}
  },
  "PairPage": {
    "experiment": "string", "pair": "string", "suite": "string", "task": "string", "number": "integer",
    "ticket": "string?", "checklist": ["string"], "a": "PairSide", "b": "PairSide",
    "rubric": [{"name": "string", "question": "string"}], "reader": "string",
    "next": "merge|preference", "mine": ["PairRecord"], "revealed": "PairReveal?"
  },
  "PairReveal": {
    "a": "baseline|candidate", "b": "baseline|candidate", "a_setup": "Setup", "b_setup": "Setup",
    "panel": "PanelCalls?", "records": ["PairRecord"]
  },
  "PairRecord": {
    "reader": {"who": "string", "from_console": "boolean", "console_holder?": "string"},
    "recorded_at": "string", "call": "merge|preference",
    "a_ready?": "boolean", "b_ready?": "boolean",
    "preference?": "Preference", "rubric?": {"*": "Preference"}
  },
  "Preference": "a|b|tie|both_bad",
  "PairSide": {
    "diff": [{"path": "string", "test": "boolean", "patch": "string"}],
    "test_files": ["string"], "fails_on_base": "boolean?", "passes_on_other": "boolean?",
    "disagreement": "boolean", "base": ["Check"], "cross": ["Check"], "error": "string?"
  },
  "PanelPair": {
    "suite": "string", "task": "string", "trial": "integer", "graders": ["PanelGrader"],
    "votes": [{"name": "string", "preference": "ArmPreference", "merge": "MergeCalls"}],
    "preference": "ArmPreference", "merge": {"baseline": "ready|not_ready", "candidate": "ready|not_ready"}
  },
  "PanelGrader": {
    "harness": "string", "model": "string", "effort": "string?", "family": "string",
    "votes": "boolean", "orders": ["PanelCall"], "preference": "ArmPreference", "merge": "MergeCalls"
  },
  "PanelCall": {
    "shown_as_a": "baseline|candidate", "baseline": "PanelSide", "candidate": "PanelSide",
    "reason": "string", "preference": "ArmPreference"
  },
  "PanelSide": {
    "rubric": [{"criterion": "string", "met": "boolean", "evidence": "string"}],
    "merge_reason": "string", "ready_to_merge": "boolean"
  },
  "MergeCalls": {"baseline": "ready|not_ready|tie", "candidate": "ready|not_ready|tie"},
  "Command": ["string"],
  "Check": {"command": "Command?", "passed": "boolean", "reason": "string"},
  "Recheck": {"passed": "boolean", "reason": "string", "wrong_finding": "boolean?", "recorded_at": "string"},
  "Pending": {
    "id": "string", "suite": "string", "task": "string", "arm": "baseline|candidate",
    "trial": "integer", "borderline": "boolean", "decisive": "boolean",
    "decisive_flips": ["passed|wrong_finding"],
    "decisive_verdicts": {"passed?": "Decision", "wrong_finding?": "Decision"}
  },
  "Verdict": {
    "verdict": "pass|fail|inconclusive|waiting_on_rechecks|waiting_on_leaks|waiting_on_people",
    "provisional_verdict?": "Decision?", "suites": {"*": "SuiteVerdict"}, "borderline_trials": "integer",
    "efficiency?": "Efficiency", "line?": "string", "deciders?": "Deciders"
  },
  "Deciders": {
    "panel": "off|beside|counted", "people": "off|beside|counted",
    "people_pairs": "integer", "panel_pairs": "integer", "waiting_pairs": "integer", "left_out_pairs": "integer",
    "agreement?": "Agreement", "inconclusive?": "string"
  },
  "Efficiency": {
    "quality_verdict": "Decision", "method": "string", "cost": "Comparison", "time": "Comparison"
  },
  "Comparison": {
    "ratio": "number?", "interval": "Interval?", "pairs": "integer", "complete": "boolean", "ceiling": "number?"
  },
  "Decision": "pass|fail|inconclusive",
  "SuiteVerdict": {
    "advisory?": "boolean",
    "bar": "SuiteBar", "thirdshift_version": "string", "method": "string",
    "accuracy": "Assessment", "wrong_findings": "Assessment", "verdict": "Decision",
    "retained_tasks": "integer", "dropped_tasks": "integer", "pass_cubed": "PassCubed?",
    "counts?": {"judged": "integer", "measurement_only": "integer", "void_dropped": "integer", "other_setup": "integer"},
    "measurement?": [{"task": "string", "trials": [{"arm": "baseline|candidate", "trial": "integer", "outcome": "succeeded|delivered|stopped_to_ask|failed|void", "passed": "boolean", "wrong_finding": "boolean", "borderline": "boolean"}]}]
  },
  "SuiteBar": {
    "margin": "number", "pass_level?": "number", "fail_level?": "number",
    "pass_rule?": "margin|reaches_zero", "credible_level?": "number", "prior": "number",
    "seed": "integer", "draws": "integer", "trials_per_task": "integer",
    "prediction": {"simulations": "integer", "interval_draws": "integer", "max_additional_tasks": "integer"}
  },
  "Assessment": {
    "difference_counts": ["integer"], "pass_interval?": "Interval", "fail_interval?": "Interval",
    "interval?": "Interval", "verdict": "Decision", "worse_within_margin?": "boolean",
    "more_tasks": "MoreTasks?"
  },
  "Interval": {"lower": "number", "upper": "number"},
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
