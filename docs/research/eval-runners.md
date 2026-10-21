# Eval runners for thirdshift: what each option leaves thirdshift to write

Research date: 2026-10-21 (America/New_York).

**Question.** Q4.2 of the [#698 grilling](../../.scratch/grill-with-docs/done/698-evals/README.md) asks how a Replay is started: (a) a `thirdshift replay` command that records the bar, builds both Arms, prepares the Eval tasks, runs Trials and Graders, applies the bar and writes the result; or (b) promptfoo run by hand from a config kept in the repository, with thirdshift only reading the results. This note works out, for six candidates, what each provides natively for each settled requirement, what thirdshift would still write (in which language, roughly how much), and what keeping it running looks like. It builds on [eval-tools.md](eval-tools.md), [eval-tools-landscape.md](eval-tools-landscape.md), and the grilling's research results of 2026-10-18 and 2026-10-20 (promptfoo's egress audit, the Rust survey and the error-bars papers), and doesn't repeat them.

The candidates:

- **(A)** promptfoo 0.124.1 by hand, with a config in the repository;
- **(B)** promptfoo under a `thirdshift replay` command;
- **(C)** thirdshift's own runner in Rust, with no framework;
- **(D)** Inspect AI;
- **(E)** MLflow GenAI evaluation (`mlflow.genai.evaluate` with a custom predict function and scorers);
- **(F)** Mira 0.5.0.

**Method.**

- Source was read at pinned commits, docs at those commits or live on 2026-10-21, and release metadata from npm, PyPI, crates.io and the GitHub API on 2026-10-21:
  - promptfoo `0.124.1`, commit [`421e795`](https://github.com/promptfoo/promptfoo/tree/421e7959642c5d4cc1c983259a268de1c6f847b9), still npm's `latest`;
  - Inspect AI `0.3.282`, commit [`386e099`](https://github.com/UKGovernmentBEIS/inspect_ai/tree/386e09942510e93de2db6d108f24f62f65eecbdc), PyPI's latest;
  - MLflow `3.17.2`, tag `v3.17.2`, commit [`d0b397f`](https://github.com/mlflow/mlflow/tree/d0b397f85b602e8f146be0d0373b28bba2ca2b74), PyPI's latest;
  - Mira `0.5.0` (crates `mira-eval`, `mira-cli`, `mira-judge`), tag `v0.5.0`, commit [`334506e`](https://github.com/everruns/mira/tree/334506ef19a1d975dace0f2c51092548a10c7b3e);
  - Bowyer et al., [arXiv 2503.01747v3](https://arxiv.org/abs/2503.01747), and the authors' [`bayes_evals`](https://github.com/sambowyer/bayes_evals/tree/b9711844e00e9664258de321966ecd258a7273d6) at `b971184`;
  - thirdshift at `dfadafa` on this branch, which merged `main` on 2026-10-20.
- Nothing was installed, built or run. Downloads were read as data only.
- Line counts for code thirdshift would write are estimates **[interpretation]**, calibrated against #429's [`replay.py`](second-opinion-replays/replay.py) (631 lines of Python, which prepared workspaces, ran six CLIs serially, resumed and stopped at plan limits, with no graders, statistics, sandbox or concurrency) and against thirdshift's own modules.
- **[interpretation]** marks a fit judgment or estimate rather than a documented fact. **[needs live check]** marks something only a real run would settle.

## TL;DR

- **The parts that decide accuracy are thirdshift's code in every option.** No candidate prepares an Eval task's repository, sandboxes a Harness, turns its memory off, grades a blind packet spanning both Arms, computes Q4.4's paired Bayesian interval, or keeps a bar from changing. That core is about 1,400–2,300 lines, mostly Rust whatever runs the Trials. (§1)
- **A framework can replace only the middle:** expanding Eval tasks, Arms and Trials, running two at a time with a timeout, storing results, resuming, and a viewer. In Rust, on thirdshift's existing process code, the middle is about 300–550 lines, and each framework's glue costs about as much. All six options come to roughly 1,500–3,100 lines **[interpretation]**. (§1, §8.2)
- **promptfoo by hand (A) enforces nothing.** Each run, a person sets Q3.12's dozen settings, builds the Baseline from the right commit and leaves the bar alone. Packet grading and Q4.4's interval end up in a JavaScript `afterAll` hook whose results promptfoo never stores. Its hooks can run shell steps, change tests and read results; `--repeat` pairs the Arms row by row but stores no repeat index; derived metrics can't see the other Arm; the JSON export has no published schema; a stopped eval resumes; and nothing in it computes statistics. (§2)
- **promptfoo under `thirdshift replay` (B) enforces everything, then buys little.** It replaces the middle for 490–780 lines of glue in two languages, Node with about 640 packages, and an egress audit at every upgrade. Its resume counts Trials that were in flight as errors to retry. (§2.6, §2.8)
- **Inspect (D) covers the most:** epochs with pass^3, two at a time, a clean stop and `eval-retry`, Grades edited with their history, and a local viewer with no analytics. It sends nothing by default. It has no pairing, no packets across runs, only unpaired intervals and an editable log, and it ships two or three releases a week with breaking changes about monthly. (§4)
- **MLflow (E) and Mira (F) fit worse.** MLflow reports usage until a variable stops it, and four of its defaults would change Trials: an extra validation call, up to 40 at once, retries on "429", and a 300-second scorer timeout added in a patch release. It has no repeats and no resume, though its write-once run params can hold the bar. Mira is Rust and silent, but has one maintainer and async APIs, retries on text such as "429", and leaves a session running after a timeout. (§5, §6)
- **Q4.4's interval:** Bowyer et al.'s paired method fits only the one-Trial Spec review. A Dirichlet over each Eval task's difference fits both suites, for 150–200 lines of Rust, 50–80 of Python or 120–180 of JavaScript. Its prior weight and seed belong in the bar's record. (§7)
- **Read [interpretation]:** Q4.2 needs (a) whatever runs the Trials, because only a command can enforce the bar and the settings. Under (a), (C) fits "accuracy first, don't rebuild without reason" best: it has no framework defaults to fight, keeps one language and one test suite, and costs about the same code. If outside experience is worth adding Python, Inspect is the strongest framework. (§9)

## 1. What stays thirdshift's code in every option

Most of what makes a Trial honest and a verdict sound sits outside every candidate. None of them prepares a historical repository, sandboxes a Harness, turns its memory off, grades a packet that spans two Arms, computes Q4.4's interval, or knows what a bar is. So these are thirdshift's code whichever runner is chosen:

| Requirement | Why no candidate covers it | Language | Rough size **[interpretation]** |
|---|---|---|---|
| **Starting a Trial:** the hidden one-session flag that prints one JSON line (Q3.8) | Only thirdshift's own code runs a session as a Run would | Rust | 200–300 lines |
| **The plan-limit stop:** noticing a warning in the Harness's stream | Each Harness reports limits its own way. thirdshift already reads Claude's usage-limit errors (`src/harness/interpretation/stream.rs:393`), but not its warnings | Rust, reported in the flag's JSON line | 80–150 |
| **Eval task repositories:** history only up to the commit, the check that no later commit survives, a local bare origin for Spec review | Git plumbing tied to the Eval task format. `replay.py` did the first part in about 30 lines of Python (`build_workspace`, L383-L413), without the check or the origin | Rust | 150–250 |
| **Harness memory off** | Each Harness's own flag or setting, in thirdshift's Harness code | Rust | 30–80, after Q3.6's live check |
| **bwrap** | It wraps the Harness process thirdshift starts, and has to know each Harness's login files | Rust | 100–200, after Q3.6's live check |
| **Arms:** the Baseline from the commit where the branch left `main`, the Candidate from its branch, or one build with two flag sets | Builds of thirdshift itself | Rust | 100–150 |
| **Code Graders** | Domain checks, such as a Trial that changed its repository being void | Rust, or the framework's assertion language | about 100 |
| **Model Graders:** one blind packet per Eval task holding both Arms' Trials, labels shuffled, graded through a Harness on a subscription | No candidate groups the Trials of two Arms and all their repeats into one grading call (§2.2, §4, §5, §6) | Rust, plus a Grader prompt | 300–450 |
| **Q4.4's statistics** | No candidate computes a paired Bayesian interval (§7) | Rust, or the framework's language | 150–250 (§7) |
| **The bar,** recorded before the first Trial and refused if it changes | No candidate has the notion | Rust | about 50 |
| **`thirdshift replays --json`** for the Console | ADR 0016 makes every Console page a CLI command first, and the command reads the Replay folder | Rust | 150–300 |

That is roughly 1,400–2,300 lines and their tests, however the Trials are scheduled **[interpretation]**. About 550–800 of them, the Graders and the statistics, could be written in a framework's language instead of Rust. The rest belongs in Rust, because it lives in the code that starts Harnesses or behind the Console's CLI commands; in (A), a person does the Arm builds and the bar instead. thirdshift's fake Harnesses (`tests/fakes/`, one per Harness) and its process code serve all of it: sessions already run in their own process group (`src/process.rs:151`), and interruption is handled in `src/interrupt.rs`. Neither thirdshift nor any candidate reads Claude's limit warnings or Codex's `rate_limits` today.

**What a framework can take over is the middle:** expanding suites, Eval tasks, Arms and Trials; running two at a time with a per-Trial timeout; storing each Trial's result; resuming after a stop; and a viewer. Written in Rust on thirdshift's process code, that middle is about 300–550 lines **[interpretation]**. The question for each candidate is what it charges to replace those lines.

## 2. promptfoo 0.124.1: options (A) and (B)

Cited at commit `421e795`, npm's `latest` on 2026-10-21.

### 2.1 What the extension hooks can do

- **Four hooks:** `beforeAll`, `beforeEach`, `afterEach` and `afterAll`, each a function in a JavaScript or Python file listed under `extensions` ([reference.md L353-L364](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/configuration/reference.md#L353-L364)).
- **Shell steps: yes, as code.** JavaScript hooks run inside promptfoo's Node process and Python ones in a subprocess ([evaluatorHelpers.ts L667-L669](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluatorHelpers.ts#L667-L669)). A hook can do whatever Node can, `child_process` included; promptfoo has no shell-step hook of its own **[interpretation]**.
- **Changing tests: yes, before they run.**
  - `beforeAll` may replace the prompts, tests, scenarios, `defaultTest`, filters, derived metrics and red-team settings, but not the providers ([evaluatorHelpers.ts L831-L849](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluatorHelpers.ts#L831-L849)). The Arms are therefore fixed by the config.
  - `beforeEach` runs once per step, which is one test row on one provider: once per Trial, before the provider is called. It may replace that step's test ([evaluator.ts L3878-L3881](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L3878-L3881)).
- **Reading results: yes, but not changing the verdict.**
  - `afterEach` sees each row after grading. It may add named scores and metadata, but "`success`, `score`, and `response.output` are **not** overridable" ([reference.md L611](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/configuration/reference.md#L611)). If it throws, the row is stored without its changes ([evaluator.ts L3943-L3946](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L3943-L3946)).
  - `afterAll` receives every stored result, the prompts with their metrics, the suite, the eval ID and the config ([evaluator.ts L5060-L5074](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L5060-L5074)). Its return value is ignored, because it "runs after results are persisted" ([evaluatorHelpers.ts L889-L891](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluatorHelpers.ts#L889-L891), [reference.md L615](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/configuration/reference.md#L615)).

So hooks could build the Arms, prepare repositories, grade packets and compute Q4.4's interval. But whatever `afterAll` concludes lives in files it writes itself; promptfoo's stored results and its viewer never see it.

### 2.2 Whether `--repeat` keeps repeats paired by test

- **Each repeat is a row of its own.** The expansion loop gives every repeat a new `testIdx` ([evaluator.ts L2694-L2723](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L2694-L2723)), and every provider runs on every row ([L2762](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L2762)). So both Arms have a result on each row: rows pair the Arms one repeat at a time.
- **Nothing stored groups one test's repeats.** The provider is told the repeat index ([providers.ts L103-L115](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/types/providers.ts#L103-L115)), but it is stripped from stored results with the other runtime variables ([evaluator.ts L761-L798](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L761-L798)). The three rows of one test share only the test case each one stores. Grouping by Eval task means keying on its description or metadata, or having the provider write the task ID and repeat index into its response metadata **[interpretation]**.
- **Per-test repeat counts exist.** A test's `options.repeat` overrides the global count ([test-cases.md L67-L79](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/configuration/test-cases.md#L67-L79), [index.ts L958-L959](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/types/index.ts#L958-L959)). One config can give code-review Eval tasks 3 Trials and Spec-review ones 1.
- **Comparison assertions see one row.** `select-best` and `max-score` are keyed by `testIdx` ([evaluator.ts L2959-L2972](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L2959-L2972), [L4589](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L4589)). They compare one repeat of each Arm, never an Eval task's six Trials together.

### 2.3 Derived metrics and post-processing

- **A derived metric belongs to one column, and a column is one Arm.** It is a mathjs expression, or a function of that column's running named-score totals and a count, and it returns one number ([evaluator.ts L2034-L2062](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L2034-L2062), [index.ts L1066-L1079](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/types/index.ts#L1066-L1079)). It never sees the other Arm's per-test values, so it can't form a paired difference, let alone an interval.
- **The only place inside promptfoo for a paired interval is `afterAll`,** in JavaScript, with the result kept outside promptfoo (§2.1).

### 2.4 The JSON export

- **Shape.** `-o results.json` writes an `OutputFile` ([index.ts L1533-L1561](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/types/index.ts#L1533-L1561), [output.ts L412-L457](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/util/output.ts#L412-L457)):
  - `evalId`, `shareableUrl`, and the `config` with secrets stripped;
  - `results`: `{version: 3, timestamp, results: EvaluateResult[], prompts: CompletedPrompt[], stats}`, each prompt carrying its column's metrics;
  - `metadata`: the promptfoo and Node versions, platform, export and creation times, and author;
  - optionally `vars`, `runtimeOptions`, `traces` and `blobAssets`.
- **Each `EvaluateResult`** holds `testIdx`, `promptIdx`, the stored `testCase`, `provider` (`id`, `label`), `vars`, `response` (output, metadata, tokens, cost), `success`, `score`, `failureReason` (0 none, 1 an assertion, 2 an error), `error`, a `gradingResult` with each assertion's pass, score and reason, `namedScores`, `latencyMs`, `cost` and `metadata` ([index.ts L380-L427](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/types/index.ts#L380-L427)).
- **Large evals.** A JSON export that doesn't fit in memory fails and points to `.jsonl`, one result per line ([output.ts L505-L532](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/util/output.ts#L505-L532)).
- **No schema is published.** The site ships only `config-schema.json`, and the docs' example ([outputs.md L62-L80](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/configuration/outputs.md#L62-L80)) shows `results.outputs` and `results.providers`, which the type doesn't have. A reader in thirdshift would follow the TypeScript type, which any 0.x release may change **[interpretation]**.

### 2.5 Pinning the bar

- **promptfoo has no bar across Arms.** A test's `threshold` is a pass score for one row.
- **A bar kept in the config's `metadata` is frozen per eval, but not enforced.**
  - The config, `metadata` included, is stored with the eval when it starts ([load.ts L1050-L1072](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/util/config/load.ts#L1050-L1072)), with its tests inlined ([L1180-L1189](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/util/config/load.ts#L1180-L1189)).
  - `--resume` rebuilds from that stored config, not the file on disk ([doEval.ts L106-L142](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/node/doEval.ts#L106-L142)).
  - But providers and hooks are stored as `file://` paths, so their code is read from disk again on resume. And nothing stops a fresh eval with a different bar **[interpretation]**.

### 2.6 Stopping and resuming

- **Resume exists.** `promptfoo eval --resume [evalId]` "reuses the original run's effective runtime options (e.g., `--delay`, `--no-cache`, `--max-concurrency`, `--repeat`), skips completed test/prompt pairs, ignores CLI flags that change test ordering to keep indices aligned" ([command-line.md L202-L209](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/usage/command-line.md#L202-L209)).
- **Pausing is Ctrl-C.** The first SIGINT aborts the eval and keeps what is stored; a second exits at once ([doEval.ts L905-L939](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/node/doEval.ts#L905-L939)). This is CLI behaviour only. A thirdshift provider that sees a plan-limit warning could send its own process SIGINT, as the red-team command does when Ctrl-C arrives as keyboard input ([shared.ts L38-L40](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/redteam/shared.ts#L38-L40)) **[interpretation; needs live check]**.
- **Trials in flight come back as errors.** A provider call that fails or is aborted is stored as an `ERROR` row ([evaluator.ts L1829-L1862](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L1829-L1862)), and resume counts `ERROR` rows as done unless it is retrying ([L3186-L3214](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L3186-L3214)). So a stopped Replay needs `--resume` and then `--retry-errors` or `promptfoo retry <evalId>`, which re-run every `ERROR` row, genuine failures included ([command-line.md L211-L219](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/site/docs/usage/command-line.md#L211-L219), [retry.ts L15-L16](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/commands/retry.ts#L15-L16)) **[interpretation]**. `retry` also accepts a different config file than the one stored ([retry.ts L17-L20](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/commands/retry.ts#L17-L20)).

### 2.7 Statistics

None. A search of `src/` for Bayesian, Wilson and bootstrap intervals, credible and confidence intervals, standard errors, and paired or significance tests found only unrelated strings. The viewer shows each column's pass rate and can open another eval beside this one ([CompareEvalMenuItem.tsx](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/app/src/pages/eval/components/CompareEvalMenuItem.tsx)), with no statistic between them.

### 2.8 What (A) and (B) leave to thirdshift

Both need all of §1. promptfoo then supplies the middle: tests × Arms × repeats, two at a time (`-j 2`), a per-Trial timeout that cancels the provider, storage in its SQLite with the export, `--resume` and `promptfoo retry <evalId>`, and its viewer, which is usable only while the browser blocks `a.promptfoo.app` (Q4.1). Besides §1, thirdshift writes:

| | (A) by hand | (B) under `thirdshift replay` |
|---|---|---|
| **The provider** that starts a Trial through the hidden flag, returns its JSON line as output and metadata, and kills it on cancel (Q3.8) | JavaScript, 120–200 lines | Same |
| **Q3.12's settings:** the socket guard, a config folder of its own, Google's metadata detection off, and an environment scrubbed before the Harnesses | The guard in JavaScript, 50–80 lines, and a dozen settings typed for every run, or a launch script, which is Q4.2 (c) | The same guard; Rust starts promptfoo with the rest, 60–100 lines |
| **The config:** two providers for the Arms, tests generated from the private Eval task files, per-test repeat counts, no cache, no retries, two at a time, the timeout | YAML or JSON in the repository, about 80 lines, and a test generator in JavaScript, about 50 | Rust writes it into the Replay folder as JSON, 100–150 lines |
| **Building the Arms** | By hand, or a `beforeAll` hook in JavaScript, about 60 lines | Rust (§1) |
| **Recording the bar** | A file, or the config's `metadata`; neither is enforced (§2.5) | Rust (§1) |
| **Packet grading and Q4.4's interval** | An `afterAll` hook in JavaScript, roughly 400–600 lines, outside thirdshift's Rust tests and invisible to promptfoo; or Rust, at which point thirdshift does more than read results | Rust (§1), once promptfoo exits |
| **Stopping at a plan-limit warning, and resuming** | The provider sends SIGINT; a person then runs `--resume` and `retry` (§2.6) | Rust drives the same sequence, 60–100 lines |
| **Reading promptfoo's results** for `thirdshift replays --json` | Rust types for the part of `OutputFile` it needs, 100–150 lines | Same |

- **(A) costs least to start and enforces nothing.** Each run depends on a person setting a dozen variables, starting Node through the guard, building the Baseline from the right commit and leaving the bar alone. A forgotten guard sends the `r.promptfoo.app` event, which no switch stops. The verdict logic ends up in JavaScript, or the "only reads the results" boundary moves **[interpretation]**.
- **(B) enforces everything, and then promptfoo replaces only the middle.** Its glue comes to roughly 490–780 lines across JavaScript and Rust, against the 300–550 lines of Rust it saves, and it brings Node, about 640 packages and an egress audit at every upgrade (§2.9) **[interpretation]**.
- **A hybrid** runs the Trials in promptfoo by hand and then a `thirdshift replay grade <evalId>` that reads the export, grades packets and applies a bar recorded beforehand. Every step added to enforce Q3.4 and Q3.12 moves it toward (B) **[interpretation]**.

### 2.9 Maintenance

- **Cadence.** npm has 68 releases from 2025-10-21 to 2026-10-21, across minors 0.119 to 0.124. The pace has slowed: 3 in August, 2 in September and 2 so far in October, after 27 in the 0.120 line alone. It is still 0.x, so any minor may break.
- **Breaking changes in that year.** Five releases carry a breaking-changes section:
  - 0.120.0 moved promptfoo to ES modules, so `.js` custom providers load as ESM; the same release added a CommonJS fallback ([CHANGELOG.md L2239](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L2239), [L2246](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L2246), [L2386-L2393](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L2386-L2393));
  - 0.121.0 changed how the Claude Agent SDK provider resolves paths ([L1412-L1414](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L1412-L1414));
  - 0.122.0 dropped Node 20 ([L508-L510](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L508-L510)), and the floor is now Node 22.22.0;
  - 0.123.0 sent GPT-5.6 and later to the Responses API ([L315-L317](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L315-L317));
  - 0.124.0 made several SDKs opt-in installs ([L40-L48](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/CHANGELOG.md#L40-L48)).

  Only the move to ESM and the Node floor would touch a thirdshift provider and hooks **[interpretation]**.
- **Weight.** The published package declares 76 dependencies and 27 optional ones, and unpacks to 31.7 MB in 706 files. The repository's lockfile resolves 637 packages outside its dev tree, 150 of them optional; that is a lockfile count, not an install.
- **Every upgrade needs Q3.12's egress audit again,** because no set of switches silences it (grilling, Research results 2026-10-20) **[interpretation]**.

## 3. thirdshift's own runner in Rust: option (C)

- **What it has natively:** no framework, but most of the parts already exist in thirdshift:
  - the session code a Trial reuses, `Sessions::within(...).run` (`src/session.rs:136-235`), which links the Factory skills, prepares review reports and gives a killed session its Resume, as in a Run;
  - sessions in their own process group (`src/process.rs:151`) and interruption (`src/interrupt.rs`), for the per-Trial timeout and the stop;
  - the Harness interpretation that already reads Claude's usage-limit errors (`src/harness/interpretation/stream.rs:393`);
  - `serde_json`, and CLI commands with `--json` that the Console pairs with its pages (ADR 0016);
  - one fake per Harness in `tests/fakes/`, so a whole Replay can be tested without a subscription.
- **What thirdshift writes:** §1's core, plus the middle, for roughly 1,700–2,850 lines of Rust and their tests **[interpretation]**. The middle breaks down as:
  - expanding suites, Eval tasks, Arms and Trials: about 50 lines;
  - two workers that interleave the Arms: 100–150;
  - the per-Trial timeout on the existing process code: 30–50;
  - one JSON file per Trial in the Replay folder: 80–120;
  - resume, which skips Trials that have a result and re-runs any that were in flight: 60–100.
- **Results:** plain JSON under `~/.thirdshift/replays/<replay>/`, which `thirdshift replays --json` and the Console read without translation. There is no viewer beyond the Console (Q4.1 (a)).
- **Egress:** nothing new. thirdshift updates itself only through `thirdshift update` (`src/main.rs:146`), and the sessions talk to their providers as Runs do.
- **Maintenance:** no new runtime or language. A dependency is added only if Q4.4's interval needs a crate (§7). The upkeep is thirdshift's own, and the Harnesses changing under it costs the same in every option.
- **What it gives up:** a ready viewer, an assertion library, other people fixing the runner's bugs, and the experience of an outside tool **[interpretation]**.

## 4. Inspect AI 0.3.282: option (D)

Cited at commit [`386e099`](https://github.com/UKGovernmentBEIS/inspect_ai/tree/386e09942510e93de2db6d108f24f62f65eecbdc), PyPI's latest, uploaded 2026-10-19. MIT, from the UK AI Security Institute, for Python 3.11 or later.

### 4.1 Telemetry and egress

- **Nothing is on by default.** Its "telemetry" hook is a function you name in `INSPECT_TELEMETRY` ([hooks/\_legacy.py L1-L34](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/hooks/_legacy.py#L1-L34)). Newer hooks load from installed packages' entry points, and a banner prints when any are on ([hooks/\_startup.py L16-L56](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/hooks/_startup.py#L16-L56)). Inspect registers none itself, and a search for analytics, crash reporting and version checks found nothing else.
- **Two quirks.**
  - `.env` files are loaded automatically, searching up from the working directory ([\_util/dotenv.py L15-L26](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_util/dotenv.py#L15-L26)).
  - Besides the log folder, it writes under `~/.local/share/inspect_ai/` ([\_util/appdirs.py L8-L26](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_util/appdirs.py#L8-L26)): a sample buffer kept for three days, and traces that record every subprocess's command line ([util/\_subprocess.py L376-L378](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/util/_subprocess.py#L376-L378)). Those would sit outside `~/.thirdshift/replays/`.
- **The viewer's page** loads only its own assets and bundled fonts. One leftover is a jsDelivr URL in MathJax's speech engine; whether rendering math fetches it is **[needs live check]**.

### 4.2 A solver that runs a Trial, without Docker

- **Yes.** `inspect_ai.util.subprocess` is a "convenience method for solvers, scorers, and tools" ([util/\_subprocess.py L109-L150](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/util/_subprocess.py#L109-L150)). The `local` sandbox is "Local file system (no sandbox)" ([sandboxing.qmd L98-L110](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/sandboxing.qmd#L98-L110)), and a solver needs no sandbox at all, since it runs in the eval's own process. `--model none` runs with no model ([models.qmd L35-L49](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/models.qmd#L35-L49)).
- **A timeout or cancel reaches only the direct child.** It sends SIGTERM, waits two seconds, then SIGKILL, and starts no new session or process group ([util/\_subprocess.py L254-L260](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/util/_subprocess.py#L254-L260), [L400-L429](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/util/_subprocess.py#L400-L429)). thirdshift's flag would have to stop its own Harness session on SIGTERM.
- **Two at a time is easy.** `max_samples` follows the model's connection limit, 10 by default ([\_util/constants.py L8-L9](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_util/constants.py#L8-L9)), and a named `concurrency(name, n)` caps any block of code ([util/\_concurrency.py L357-L380](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/util/_concurrency.py#L357-L380)).
- **A time limit doesn't fail a Trial:** "samples that encounter limits are still scored" ([setting-limits.qmd L25](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/setting-limits.qmd#L25)). A code Grader would have to void them.

### 4.3 Epochs

- **Repeats are grouped by sample.** Each epoch is one record carrying its sample's `id` and its `epoch` ([log/\_log.py L446-L455](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/log/_log.py#L446-L455)). Metrics reduce each sample's epochs first, by their mean unless told otherwise, and the reducers include `pass_at_k`, `pass_k_k` ("all k epoch attempts succeed") and `at_least_k` ([metrics.qmd L327-L390](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/metrics.qmd#L327-L390)). So pass^3 comes free.
- **Across runs, nothing pairs.** Two Arms are two runs, or two tasks, and matching their samples is the reader's job.

### 4.4 Scorers and the log

- **A scorer grades one sample's one epoch.** It takes `(state, target)` and returns a `Score` with a value, answer, explanation and metadata ([scorer/\_scorer.py L35-L40](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/scorer/_scorer.py#L35-L40), [scorer/\_metric.py L112-L136](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/scorer/_metric.py#L112-L136)). Metrics include accuracy, `stderr(cluster=)`, `bootstrap_stderr`, `ci` (t or bootstrap) and `ci_wilson` ([metrics.qmd L30-L80](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/metrics.qmd#L30-L80)). None is Bayesian or paired.
- **Grading can wait until after the run:** `--no-score`, then `inspect score <log>`, one log at a time ([scoring-workflow.qmd L8-L83](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/scoring-workflow.qmd#L8-L83)). Since a scorer sees one sample and one epoch, nothing in Inspect sees both Arms at once **[interpretation, from the signatures]**. A packet grader would read both logs itself and write the Grades back with `edit_score()`, which keeps each Grade's history ([eval-logs.qmd L378-L456](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/eval-logs.qmd#L378-L456)).
- **A Grader through a Harness is possible.** A custom model provider (`@modelapi`) can do anything in `generate()` ([extensions-model-api.qmd L10-L40](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/extensions-model-api.qmd#L10-L40)), so `model_graded_qa` could reach a Harness session through thirdshift's flag **[interpretation]**.
- **The log.** The default `.eval` is a ZIP of zstd-compressed entries, and `.json` is the alternative ([eval-logs.qmd L73-L105](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/eval-logs.qmd#L73-L105)). Its top level holds version, status, eval, plan, results, stats, error, invalidated, log and config updates, tags, metadata, samples and reductions ([log/\_log.py L1319-L1365](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/log/_log.py#L1319-L1365)). Unlike promptfoo's export, it has a JSON Schema, printed by `inspect log schema`. Logs may hold NaN and Inf, which browsers' and Node's JSON parsers reject, so readers in other languages need a lenient parser ([L704-L716](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/eval-logs.qmd#L704-L716)). The format has changed under its readers before: 0.3.206's condensed format forced Inspect Scout to update ([eval-logs.qmd L107-L132](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/eval-logs.qmd#L107-L132)).

### 4.5 Comparing runs, the viewer, stopping and the bar

- **No paired comparison.** The docs send readers to the log API "to compare across runs" ([log-viewer.qmd L14](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/log-viewer.qmd#L14)), and describe `ci()` as useful for seeing whether two models' intervals overlap ([metrics.qmd L60](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/metrics.qmd#L60)), which is an unpaired check.
- **`inspect view`** is a FastAPI server on `127.0.0.1:7575` ([\_util/constants.py L12-L13](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_util/constants.py#L12-L13)). It serves the page's own files and an API that lists, reads and downloads the logs in its log folder. `inspect view bundle` writes the same page as a static site instead, and publishes it only if told to write to Hugging Face or S3 ([log-viewer.qmd L273-L338](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/log-viewer.qmd#L273-L338)). It refuses any address other than loopback without a token or an explicit unsafe flag ([\_view/network.py L98-L118](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_view/network.py#L98-L118)). Its API can also edit and delete logs ([fastapi_server.py L231-L333](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_view/fastapi_server.py#L231-L333)), and it renders log content as trusted unless given `--no-trust-content` ([viewer/\_config.py L278-L284](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/viewer/_config.py#L278-L284)). Unlike promptfoo's viewer, it has no analytics to block.
- **Stopping and resuming work.** By default the first sample error fails the eval ([handling-errors.qmd L46-L53](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/handling-errors.qmd#L46-L53)). `eval-retry` keeps completed samples, and crash recovery re-runs those that were in flight ([L136-L200](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/handling-errors.qmd#L136-L200)). `inspect ctl task drain` lets running samples finish and leaves the rest for later ([control-channel.qmd L9-L39](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/control-channel.qmd#L9-L39)). So a solver that raises at a plan-limit warning stops the Replay cleanly, and `eval-retry` resumes it **[interpretation]**.
- **One default must go:** `eval_set` retries a failed task up to 10 times, immediately, reusing completed samples ([\_eval/evalset.py L318-L329](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_eval/evalset.py#L318-L329); immediate since 0.3.218, [CHANGELOG.md L1252-L1258](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L1252-L1258)). Each retry is a fresh Trial.
- **The bar can be recorded, but not fixed.** The eval's header (task arguments, metadata, git revision, package versions) is written before any sample runs ([log/\_log.py L1108-L1220](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/log/_log.py#L1108-L1220), [log/\_recorders/eval.py L224-L228](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/log/_recorders/eval.py#L224-L228)), but `edit_eval_log` and the viewer can change it ([eval-logs.qmd L466-L500](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/eval-logs.qmd#L466-L500)).

### 4.6 Maintenance

- **Cadence.** 119 releases on PyPI in the year to 2026-10-21, two or three a week, still 0.x. `main` took 285 commits in the last 30 days, from a project with 336 contributors (GitHub API, 2026-10-21).
- **Breaking changes come about monthly:**
  - 0.3.282 scoped tool state per agent ([CHANGELOG.md L6-L8](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L6-L8)), and 0.3.281 raised the floor to Python 3.11 ([L33-L35](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L33-L35));
  - 0.3.262 made a panel of model graders need a strict majority ([L382](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L382));
  - 0.3.260 removed old `ctl` spellings and made media need materializing ([L532](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L532), [L546](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L546)), and 0.3.259 made ragged CSV rows raise ([L575](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md#L575));
  - besides those, 0.3.218 changed the retry default and 0.3.206 the log format.
- **Weight.** 40 direct requirements, about 77 packages in all on Linux with Python 3.12 (an approximation from its `uv.lock`), among them boto3, aiohttp, numpy, tiktoken, textual, fastapi and uvicorn. The wheel is 35 MB.

### 4.7 What thirdshift would still write **[interpretation]**

About 500–850 lines of Python: the task and an Eval task loader (80–120), the solver that calls the hidden flag (80–120), code Graders (50–100), packet grading across both Arms' logs (150–250), and Q4.4's statistics (50–80, §7) with the summary file the listing reads (100–170). If `thirdshift replay` starts it, add a Rust launcher of 150–250 lines. thirdshift's listing should read the summary file, not the `.eval` logs.

Inspect would buy the scheduling and bookkeeping: epochs, pass^k, two at a time, a clean stop and resume, Grades edited with their history, and a local viewer with no analytics. It wouldn't buy pairing, packets, Bayesian statistics, isolation or a fixed bar.

## 5. MLflow 3.17.2 GenAI evaluation: option (E)

Cited at tag `v3.17.2`, commit [`d0b397f`](https://github.com/mlflow/mlflow/tree/d0b397f85b602e8f146be0d0373b28bba2ca2b74), released to PyPI on 2026-10-21.

### 5.1 Telemetry and other egress

- **The Python client reports usage by default,** and has since 3.2.0. `MLFLOW_DISABLE_TELEMETRY=true` or `DO_NOT_TRACK=true` turns it off ([usage tracking](https://mlflow.org/docs/latest/community/usage-tracking/)). `DO_NOT_TRACK=1` is ignored, because only the string `true` counts ([utils.py L153-L158](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/utils.py#L153-L158)).
  - `import mlflow` sets up the client ([\_\_init\_\_.py L454-L456](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/__init__.py#L454-L456)). The first event fetches a config from `config.mlflow-telemetry.io`, and records go to whatever ingestion URL it names ([constant.py L8-L9](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/constant.py#L8-L9), [client.py L184-L218](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/client.py#L184-L218)).
  - Each event carries session and installation IDs, versions, the OS, the tracking URI's scheme, and a coding agent detected from environment variables, such as Claude Code or Codex ([schemas.py L103-L121](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/schemas.py#L103-L121), [utils.py L94-L135](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/utils.py#L94-L135)). An `evaluate` adds the scorer classes, the row count and the column names, and every scorer call sends an event of its own ([events.py L179-L220](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/events.py#L179-L220), [L926-L976](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/events.py#L926-L976)). No content is sent.
  - With the switch set, no client is created ([track.py L19-L24](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/track.py#L19-L24), [client.py L551-L556](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/client.py#L551-L556)). Unlike promptfoo, nothing was found that is sent anyway.
- **The server and its page.**
  - `mlflow server` sends a start event ([events.py L584-L607](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/events.py#L584-L607)).
  - The page's click telemetry goes to the MLflow server, which forwards it through its own client. The server also fetches the page's telemetry config from a CloudFront host ([handlers.py L8189-L8262](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/handlers.py#L8189-L8262), [TelemetryClient.ts L158-L187](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/telemetry/TelemetryClient.ts#L158-L187)). The server's variable turns this off for every browser ([handlers.py L8202-L8203](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/handlers.py#L8202-L8203)).
  - It binds `127.0.0.1` by default ([cli_args.py L162-L173](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/utils/cli_args.py#L162-L173)), and has no sign-in unless started with `--app-name basic-auth`.
- **Two more paths to close:** the default judge is `openai:/gpt-4.1-mini` ([judges/utils/\_\_init\_\_.py L35-L41](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/judges/utils/__init__.py#L35-L41)), and tracing exports to any `OTEL_EXPORTER_OTLP_*` endpoint in the environment ([otlp.py L89-L104](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/utils/otlp.py#L89-L104)).

### 5.2 A predict function that runs a Trial

- **It works.** `evaluate(data, scorers, predict_fn)` passes each row's `inputs` as keyword arguments, and wraps a function that makes no span in `mlflow.trace` ([base.py L62-L67](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/base.py#L62-L67), [trace_utils.py L579-L583](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/utils/trace_utils.py#L579-L583)). A plain function that starts a subprocess is fine.
- **Four defaults would change the Trials:**
  - Before the run, it calls the function once on the first row to check that it traces. That is an extra session nobody records, unless `MLFLOW_GENAI_EVAL_SKIP_TRACE_VALIDATION=true` ([trace_utils.py L569-L577](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/utils/trace_utils.py#L569-L577)).
  - With `MLFLOW_GENAI_EVAL_MAX_WORKERS` unset, the pool is sized from an automatic rate: 10 per second, doubled, times 2 seconds, so 40 threads ([harness.py L127](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/harness.py#L127), [L138-L150](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/harness.py#L138-L150), [L175-L191](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/harness.py#L175-L191)). Predicting and scoring run in separate pools at once, so even set to 2 it allows two Trials and two graded rows together ([L589-L619](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/harness.py#L589-L619)).
  - An exception whose text contains "429" or "rate limit" is retried up to 3 times, each a fresh session ([rate_limiter.py L49-L69](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/rate_limiter.py#L49-L69), [environment_variables.py L942](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/environment_variables.py#L942)). A plan-limit message would start new sessions.
  - Every scorer gets a 300-second timeout ([scorers/base.py L69-L70](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/scorers/base.py#L69-L70)). When it fires, the thread is abandoned and a Grader's subprocess keeps running ([L968-L996](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/scorers/base.py#L968-L996)).
- **No timeout or kill for a plain predict function.** Shutting the pool down cancels only rows that haven't started ([harness.py L299-L300](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/harness.py#L299-L300)).

### 5.3 Scorers, repeats and comparing runs

- **A scorer sees one row.** `@scorer` takes any of `inputs`, `outputs`, `expectations` and `trace`, and returns a bool, number, string or `Feedback` ([scorers/base.py L1550-L1621](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/scorers/base.py#L1550-L1621)). Session scorers see a group of traces, but are refused when a predict function is given ([session_utils.py L195-L215](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/session_utils.py#L195-L215)). A packet per Eval task is therefore a second pass in thirdshift's code.
- **Judges call APIs.** `make_judge` and the built-in judges take a provider URI and an optional base URL ([make_judge.py L114-L126](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/judges/make_judge.py#L114-L126)). None runs a CLI on a subscription, though a scorer is plain Python and can.
- **No repeats.** There is no parameter for them, so 3 Trials means three copies of a row.
- **Comparing runs.** The page can set a baseline run and show both runs' traces side by side, with a bar chart per assessment ([EvaluationRunCompareSelector.tsx L134](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/components/evaluations/EvaluationRunCompareSelector.tsx#L134), [AggregationUtils.ts L598-L640](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/genai-traces-table/utils/AggregationUtils.ts#L598-L640)). It shows no interval and no paired statistic.

### 5.4 Resume, storage and the bar

- **No resume.** Running again is a new run that repeats every row.
- **Storage.** The default tracking URI is `sqlite:///mlflow.db` in the working directory ([store/tracking/\_\_init\_\_.py L12-L13](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/store/tracking/__init__.py#L12-L13)), and no server is needed. `EvaluationResult` holds a `run_id`, `metrics` and a pandas `result_df` ([entities.py L256-L262](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/entities.py#L256-L262)), with no JSON export of its own.
- **The bar can be frozen.** `evaluate` logs into the active run, and a run's params can't change once logged: "Changing param values is not allowed" ([sqlalchemy_store.py L2084-L2091](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/store/tracking/sqlalchemy_store.py#L2084-L2091)). Of the six, it is the only one with a write-once record **[interpretation]**. Applying the bar is still thirdshift's code.

### 5.5 Maintenance

- **Cadence.** PyPI received 35 uploads in the year to 2026-10-21: 21 final 3.x releases, 12 of them minors (3.6 to 3.17), plus 9 release candidates and 5 backports to 2.x. That is about one minor a month.
- **Changes to `evaluate` in that year:**
  - 3.6.0 deprecated the v2 evaluation and removed custom-scorer registration ([CHANGELOG.md L1760](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/CHANGELOG.md#L1760), [L1778](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/CHANGELOG.md#L1778));
  - 3.11.1 overlapped predicting and scoring ([L802](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/CHANGELOG.md#L802));
  - 3.16.1, a patch release on 2026-09-17, added the scorer timeout under its features ([release notes](https://github.com/mlflow/mlflow/releases/tag/v3.16.1)). Its 300-second default would cut off a 15-minute Grader without a word.
- **Weight.** `mlflow` has 20 direct requirements, among them Flask, alembic, docker, graphene, gunicorn, matplotlib, numpy, pandas, pyarrow, scikit-learn, scipy and sqlalchemy. That comes to roughly 87 packages, an approximation from PyPI metadata rather than an install. It needs Python 3.10 or later and is Apache-2.0. `evaluate` imports pandas, which only the full package declares.

### 5.6 What thirdshift would still write **[interpretation]**

About 600–1,100 lines of Python on top of §1's Rust, plus about 87 packages on the host:
- the predict function and the row expansion for 3 or 1 Trials, with a semaphore so predicting and grading together stay at two sessions;
- the five settings above, set on every run;
- a ledger for stopping at a plan-limit warning and resuming, since MLflow has neither: 150–250 lines;
- code Graders as `@scorer`s, 50–150 lines, and the packet grading as a second pass, 150–250 lines;
- Q4.4's interval in Python, where scipy is already present (§7).

MLflow would add tracing and Grade storage, and a local page with a baseline compare. thirdshift's listing should read a summary file thirdshift writes, not MLflow's SQLite schema, which moves with its database migrations.

## 6. Mira 0.5.0: option (F)

Cited at commit [`334506e`](https://github.com/everruns/mira/tree/334506ef19a1d975dace0f2c51092548a10c7b3e), tag `v0.5.0`, from which the crates `mira-eval`, `mira-cli` and `mira-judge` 0.5.0 were published on 2026-07-25. `main` is six documentation and chore commits ahead, at `2d9e70b` (2026-10-12). Nothing newer is on crates.io.

### 6.1 Shape

- **A library and a CLI.** `mira-eval` is the library and `mira-cli` builds the `mira` host. The host and a *study* are separate processes that exchange newline-delimited JSON over the study's stdio ([how-it-works.md L43-L62](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/docs/how-it-works.md#L43-L62)). The library's in-process `Runner` is a sequential loop with no concurrency, timeout or saved runs ([runner.rs L342-L378](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/runner.rs#L342-L378)).
- **`CliSubject` runs a binary** in a fresh temporary directory, passes the prompt through a placeholder or stdin, gives the child the whole environment, and buffers its output with no cap ([subject.rs L94-L280](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/subject.rs#L94-L280)). It doesn't pass the trial index.
- **A timeout doesn't stop the session.** A timeout drops the case and sends a best-effort cancel ([exec.rs L333-L349](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/exec.rs#L333-L349)). Nothing in the crates sets `kill_on_drop` or a process group, so a Harness session keeps running.
- **It is async.** The subject and scorer traits run on tokio ([subject.rs L30-L31](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/subject.rs#L30-L31)), while ADR 0016 keeps async code inside the Console. Linking Mira would put tokio on the Replay path **[interpretation]**.

### 6.2 Arms, Trials and resume

- **Arms** can be targets, axes, or an experiment's `[[treatments]]`, which differ only in environment variables ([experiment.rs L34-L40](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/experiment.rs#L34-L40)). Plans run eval, sample, target, axes, trial in that order, so two targets interleave per Eval task ([main.rs L1900-L1971](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/main.rs#L1900-L1971)).
- **Trials** are set per eval with `Eval::trials(n)` ([eval.rs L234-L237](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/eval.rs#L234-L237)), and keyed `eval/sample@target[params]#i`.
- **Three defaults would change the Trials:**
  - concurrency defaults to 8, and retries to 4 ([main.rs L438-L456](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/main.rs#L438-L456));
  - a case is re-queued when any error's text contains "429", "quota", "rate limit", "overloaded" or "try again later", and only the last attempt is kept ([lib.rs L686-L696](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/lib.rs#L686-L696), [exec.rs L392-L405](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/exec.rs#L392-L405)). An error that quotes issue #429 would trip it **[interpretation]**;
  - experiment mode runs each treatment as a child `mira run` with the defaults above, and stops at the first one that exits non-zero, which any failed case causes ([main.rs L1091-L1126](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/main.rs#L1091-L1126), [L1333-L1338](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/main.rs#L1333-L1338)). A Baseline that fails one Eval task would stop the Candidate from running **[interpretation]**.
- **Resume.** Each finished case is written to `cases/<key>/result.json`, and `--resume <run_id>` skips every recorded case, the N/A ones from infrastructure errors included ([main.rs L1187-L1192](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/main.rs#L1187-L1192), [L1235-L1240](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/main.rs#L1235-L1240)). A subject can't stop the whole run; the protocol cancels one run at a time.

### 6.3 Graders, results and statistics

- **A scorer grades one Trial.** `Scorer::score` takes one sample and one transcript, and `model_graded(rubric, judge)` takes a judge function of your own ([scorer.rs L32-L35](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/scorer.rs#L32-L35), [L707-L733](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/scorer.rs#L707-L733)). There are no packets, blinding or shuffling. `mira-judge` needs an OpenAI or Anthropic API key.
- **Results** go to `<results>/<run_id>/` as `meta.json`, `report.json`, `report.html` and one `result.json` per case ([config.rs L9-L21](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/config.rs#L9-L21)), and the folder may be absolute, so it can sit under `~/.thirdshift/replays/`. The HTML report is self-contained, and a test checks it loads nothing remote ([report.rs L1185-L1196](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/report.rs#L1185-L1196)).
- **Statistics:** per eval, sample and target, a pass rate, unbiased pass@k, a mean and a standard deviation ([aggregate.rs L24-L101](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-eval/src/aggregate.rs#L24-L101)). There is no pass^k, interval or paired difference. Plans "record baseline identity but [do] not yet apply automated comparison gates" ([experiments.md L47](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/docs/experiments.md#L47)).
- **The bar can't be pinned.** `[compare]` takes only a grouping and a baseline ([experiment.rs L84-L94](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/crates/mira-cli/src/experiment.rs#L84-L94)), and the plan copied into each run folder isn't compared on resume.
- **Egress.** No telemetry was found. The library has no HTTP client, and the CLI's only network path is an opt-in `--publish` to `app.everruns.com`.

### 6.4 Maintenance

- **Releases:** 0.1.0 to 0.5.0 between 2026-06-23 and 2026-07-25, four of the five with breaking changes, such as renamed core types and a verb-first CLI whose old flags go at 1.0 ([CHANGELOG.md L23-L40](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/CHANGELOG.md#L23-L40), [L255-L270](https://github.com/everruns/mira/blob/334506ef19a1d975dace0f2c51092548a10c7b3e/CHANGELOG.md#L255-L270)). None in the 88 days since. An open pull request changes the protocol's errors and version negotiation.
- **People:** created 2026-06-20; all 82 commits and 83 pull requests are one author's; no issue has ever been filed; 8 stars and no forks (GitHub API, 2026-10-21).
- **Weight:** `mira-eval` resolves to 36 crates, 8 of them new to thirdshift's lockfile; the `mira` binary to 133, 27 of them new (`cargo tree` on an isolated copy, nothing built). Edition 2024 and Rust 1.85, like thirdshift.

### 6.5 What thirdshift would still write **[interpretation]**

About 1,700–2,500 lines of Rust in all. Mira replaces the middle: the scheduler with timeouts, the run store with resume, and reports, which §1 puts at 300–550 lines and the Mira research at 400–700 with the reports. But the study around the hidden flag costs 300–450 lines of its own, so the total is about (C)'s. On top of that, thirdshift works around its silent retries, sessions left running after a timeout, experiment mode's early stop, and the N/A cases that resume skips, and takes on a pre-1.0 dependency with one maintainer and async APIs.

## 7. Q4.4's statistics, in each language

No candidate computes them (§2.7, §4.4, §5.3, §6.3), so this is thirdshift's code in every option. What changes is the language and the library behind it.

### 7.1 What Bowyer et al. recommend, and whether it fits

- **Their paired method** ([arXiv 2503.01747v3](https://arxiv.org/abs/2503.01747), §3.4, App. C and E.2) takes one binary outcome per item per model. Both outcomes come from one latent bivariate normal with means Φ⁻¹(θ_A) and Φ⁻¹(θ_B) and a correlation ρ, under Beta(1,1) priors on the θs and a prior that favours positive ρ. The likelihood needs only the 2×2 counts. Inference is importance sampling with 10,000 draws and resampling, and the interval is the 2.5th to 97.5th percentiles of θ_A − θ_B. Several trials per item "would" need a hierarchical model, which the paper doesn't give (App. D).
- **Their code** is [`sambowyer/bayes_evals`](https://github.com/sambowyer/bayes_evals/tree/b9711844e00e9664258de321966ecd258a7273d6) at `b971184` (MIT, last changed 2025-05-28, not on PyPI). It is one 467-line module that depends on numpy, matplotlib, scipy and pandas, unpinned ([setup.py L8-L13](https://github.com/sambowyer/bayes_evals/blob/b9711844e00e9664258de321966ecd258a7273d6/setup.py#L8-L13)). Its `paired_comparisons` takes binary data only, uses NumPy's global generator with no seed, and returns P(A > B) rather than an interval ([bayes_evals.py L129-L231](https://github.com/sambowyer/bayes_evals/blob/b9711844e00e9664258de321966ecd258a7273d6/src/bayes_evals/bayes_evals.py#L129-L231)).
- **The fit to Q4.4:**
  - The Spec-review suite, one Trial per Arm and pass or fail, fits §3.4 as published.
  - The code-review suite, three Trials per Arm, doesn't. Treating its 27 Trial pairs as 27 items pools repeats, which Miller (§3.1) calls inconsistent.
  - Wrong-finding counts have no method in the paper.
- **A method that fits both suites [interpretation]: a Dirichlet over the values each Eval task's difference can take.** With 3 Trials an Eval task's difference is one of 7 values, −1 to +1 in thirds; with 1 Trial, one of 3. The posterior is Dirichlet(α + counts), the mean difference Δ = Σ πᵥ·v is exactly Q4.4's quantity, and the interval is the percentiles of sampled Δ. It is the paired, any-number-of-Trials form of the paper's own Dirichlet-categorical model (§3.5, Eq. 9), and pairing is built in, because each Eval task's own difficulty cancels in its difference.
- **The prior weight α is the one real choice, and belongs with the bar.** Synthetic coverage checks, run for this note in standard-library Python on made-up data (9 Eval tasks of 3 Trials, 400–500 datasets per scenario), gave:

  | Method | Coverage of a 95% interval |
  |---|---|
  | Dirichlet, α = 1 per value (the paper's uniform prior) | 1.00 when the true Δ is near 0, but 0.79 at Δ = −0.42, because it pulls toward 0 |
  | Dirichlet, α = 1/7 per value (one pseudo-Eval-task in all) | 0.93–0.98 |
  | Normal approximation | 0.905–0.936 |
  | t with 8 degrees of freedom | 0.95–0.97 |
  | Bayesian bootstrap ([Rubin 1981](https://doi.org/10.1214/aos/1176345338)) | 0.83–0.89, and [0, 0] whenever every difference is equal |

  These are this research's own simulations, not published figures **[interpretation]**.
- **"How many more Eval tasks would settle it."** Two ways **[interpretation]**:
  - project the interval's width by 1/√n, the scaling behind Miller's power analysis (§5): about 15 lines, deterministic, assuming the centre stays put;
  - simulate more Eval tasks from the posterior and report the smallest number with at least a 0.8 chance of a clear verdict, an "assurance" ([O'Hagan, Stevens and Campbell 2005](https://doi.org/10.1002/pst.175)): about 40 lines, and cheap because the Dirichlet updates in closed form.

  In one synthetic case, a small gap with a mean of −0.037 and an interval of [−0.25, +0.21], the projection said about 65 more Eval tasks. The assurance was 0.07 with 9 more, 0.55 with 45 and 0.73 with 90, which is Q4.4's warning that 9 Eval tasks will rarely decide.
- **Wrong findings [interpretation].** The simplest reading of "judged the same way" is a flag per Trial, "reported any wrong finding", run through the same code with the sign reversed. Raw counts would need more values per Eval task, or a Gamma–Poisson model nobody here has validated.

### 7.2 Cost by language **[interpretation]**

Each estimate covers the draws, the quantiles, the three verdicts, pass^3, the more-Eval-tasks figure and the JSON it writes, with tests on top.

| Language | Lines | Library |
|---|---|---|
| **Rust** | 150–200 | [`rand_distr` 0.6.0](https://crates.io/crates/rand_distr/0.6.0) (MIT or Apache-2.0, 2026-02-10, Rust 1.85) has `Dirichlet` and the Marsaglia–Tsang gamma sampler, and adds two crates, itself and `libm`. A hand-written gamma sampler instead costs about 30 more lines and no crate. `rand` 0.10.3 and `rand_pcg` 0.10.2 are already in `Cargo.lock` through `quinn-proto`. |
| **Python** (Inspect, MLflow) | 50–80 | numpy's `default_rng(seed).dirichlet`. numpy is already a requirement of both. |
| **JavaScript** (promptfoo's `afterAll`) | 120–180 | Written by hand: no maintained package gives a seeded Dirichlet sampler. [jstat](https://www.npmjs.com/package/jstat) was last released on 2022-11-21. |

**Determinism.** A sampled interval must give the same verdict when the analysis is run again, so the seed, the draw count, α, the margin, the credible level and the thirdshift version belong in the bar's record, before the first Trial **[interpretation]**. Two cautions:
- rand's `StdRng` "even with a fixed seed, output is not portable" ([rand 0.10.3, src/rngs/std.rs](https://docs.rs/crate/rand/0.10.3/source/src/rngs/std.rs)), so a named generator such as PCG is the safer choice. NumPy likewise reserves the right to change its `Generator` streams between releases ([policy](https://numpy.org/doc/stable/reference/random/compatibility.html)).
- At 20,000 draws, changing the seed moved the bounds by about 0.001. A bound within about 0.005 of zero, or of the margin, needs more draws before it decides a verdict **[interpretation]**.

## 8. The six options side by side

### 8.1 Against the settled requirements

"thirdshift" means thirdshift writes it; §1 gives the sizes.

| Requirement | (A) promptfoo by hand | (B) promptfoo under `replay` | (C) Rust runner | (D) Inspect | (E) MLflow | (F) Mira |
|---|---|---|---|---|---|---|
| **Arms** | Two providers in the config; builds by hand | Providers in a generated config; builds in Rust | Rust | A task argument per Arm; builds in Rust | One run per Arm; builds in Rust | Targets; builds in Rust |
| **3 or 1 Trials per suite** | Native: per-test `repeat` | Native | Rust | Native: epochs per task | Copied rows | Native: `trials(n)` |
| **Baseline beside Candidate** | Native: both Arms on each row | Native | Rust | thirdshift orders two tasks | thirdshift orders rows | Native: targets interleave |
| **At most 2 at once** | `-j 2` | `-j 2` | Rust | `concurrency("trial", 2)` | Workers 2, but Trials and Graders overlap | `-j 2` |
| **Per-Trial timeout that stops the session** | Native: cancels the provider | Native | Rust, on existing process groups | Signals only the direct child; the Trial is still scored | None | Nothing kills the session |
| **Stop at a plan-limit warning, resume** | SIGINT, `--resume`, then `retry` **[needs live check]** | The same, driven by Rust | Rust | Raise or drain, then `eval-retry` | None: a ledger thirdshift writes | No whole-run stop; resume skips N/A cases |
| **Defaults that would change Trials** | 3 retries, cache, 4 at once, telemetry (Q3.12) | The same, set by Rust | None | `eval_set`'s 10 retries | 40 workers, 3 retries, an extra validation call, 300 s scorer timeout | 8 at once, 4 retries on text matches |
| **Honest Trials:** repositories, the later-commit check, bare origin, memory off, bwrap | thirdshift | thirdshift | thirdshift | thirdshift | thirdshift | thirdshift |
| **Code Graders** | JavaScript assertions | JavaScript or Rust | Rust | Python scorers | Python scorers | Rust scorers |
| **Blind packet per Eval task** | thirdshift, in `afterAll` | thirdshift | thirdshift | thirdshift, Grades written back with history | thirdshift, a second pass | thirdshift |
| **Q4.4's interval and verdicts** | thirdshift, in JavaScript | thirdshift | thirdshift | thirdshift; Inspect adds pass^k and unpaired intervals | thirdshift | thirdshift; Mira adds pass@k |
| **Bar recorded first, unchangeable** | Frozen per eval in `metadata`, not enforced | Rust | Rust | Recorded first, but editable | Write-once run params | Not possible |
| **Results under `~/.thirdshift/replays/`** | Config folder and export there | The same | Native | Logs there; buffers and traces in `~/.local/share/inspect_ai/` | Tracking database there | Results folder there |
| **`--json` listing** | Rust reads promptfoo's export | The same | Rust | Rust reads a summary Python writes | The same | Rust reads `meta.json` |
| **Nothing leaves the host** | Guard, own config folder, no viewer (Q3.12), by hand | The same, enforced | Nothing new | Nothing by default; no `.env` above, no hook packages | Two variables, and the server's for its page | Nothing by default |
| **Viewer** | `promptfoo view`, with `a.promptfoo.app` blocked | The same | The Console | `inspect view`, local, no analytics | MLflow's page, local | A self-contained HTML report |

### 8.2 What each leaves thirdshift to write, and its upkeep **[interpretation]**

| | Code thirdshift writes, in all | Languages | Added to the host | Releases in the last year | Breaking changes | Who maintains it |
|---|---|---|---|---|---|---|
| **(A)** | 1,550–2,550: the Rust core without Arm builds or the bar, plus a provider, guard, config, and packets and Q4.4 in an `afterAll` hook. A person sets Q3.12's settings, builds the Arms and leaves the bar alone | Rust, JavaScript, YAML | Node 22.22+, about 640 packages | 68, still 0.x | 5 flagged; the move to ESM and the Node floor touch thirdshift | OpenAI |
| **(B)** | 1,900–3,100: the whole Rust core, plus 490–780 lines of glue in place of the middle's 300–550 | Rust, JavaScript | The same | The same | The same | OpenAI |
| **(C)** | 1,700–2,850 | Rust | Nothing, or `rand_distr` and `libm` | thirdshift's own | None from outside | Jacob |
| **(D)** | 1,500–2,600: the Rust core without the Graders and statistics, 500–850 lines of Python, and a 150–250-line launcher | Rust, Python | Python 3.11+, about 77 packages | 119, still 0.x | About monthly, the log format once | UK AI Security Institute, 336 contributors |
| **(E)** | 1,550–2,800: the same split, with 600–1,100 lines of Python and a launcher | Rust, Python | Python 3.10+, about 87 packages | 21 final releases, 12 of them minors | `evaluate` changed in 3.6, 3.11 and a 3.16 patch | Databricks |
| **(F)** | 1,700–2,500 | Rust, async | 8 crates for the library, 27 for the CLI | 5, all in June and July | 4 of 5 | One person |

**The totals barely move** **[interpretation]**. Every option lands between about 1,500 and 3,100 lines, because the bulk is §1's core, which no framework touches. A framework replaces only the middle, and its glue costs about what the middle would. What changes between options is what is enforced, how many languages and dependencies there are, and who else changes the code underneath.

## 9. Which fits "accuracy first, don't rebuild without reason" **[interpretation]**

- **On Q4.2, (a) is needed whatever runs the Trials.** Q3.4's bar fixed before the first Trial, Q3.12's settings on every run, and the Baseline built from the right commit are rules a command can enforce and a person running promptfoo by hand can only remember. (b) by hand leaves each of them to memory, and its verdict logic in JavaScript. "Side by side" works only as the hybrid in §2.8, and each rule it enforces moves it toward (a).
- **Under (a), (C) fits best.** The accuracy-critical code is thirdshift's in every option. What a framework adds is exactly where silent inaccuracy comes from: retries that start fresh sessions, caches, defaults that changed in patch releases, timeouts that leave sessions running, and resume rules that count in-flight Trials as done. (C) has none of those, and keeps the whole Replay in one language under one test suite with the fake Harnesses. Its total is within a few hundred lines of every framework option (§8.2). It doesn't rebuild a framework; it writes the 300–550-line middle that each framework's glue would cost anyway.
- **Of the frameworks, Inspect is the strongest second choice.** It covers the most natively (epochs, pass^3, two at a time, a clean stop and resume, Grades edited with their history), sends nothing by default, and has the cleanest local viewer. Against it are a second language, about 77 Python packages, and a project that releases two or three times a week with monthly breaking changes, one of them to its log format. promptfoo under (a) buys the least for its cost, because the Console is to be the viewer (Q4.1) and its own viewer needs a browser block. MLflow carries the most risky defaults and no resume. Mira is too young, and its retries and its sessions left running after a timeout work against honest Trials.
- **For outside experience without putting a tool in the Trial path:** the Session traces work (Q2.1, Q3.19, Q4.14) already puts thirdshift on OpenTelemetry, where viewers and collectors are interchangeable; the promptfoo telemetry fix (Q3.12) is still a good first contribution; and a Replay's results could be exported to an outside viewer later, without changing who runs the Trials.

## 10. Uncertainties and live checks

- **None of this was run.** The line counts are estimates from code reading and `replay.py`, not from a prototype.
- **promptfoo's stop at a plan limit** relies on a provider sending its own process SIGINT, and on how in-flight Trials come back after `--resume` **[needs live check]**.
- **Inspect's viewer** may fetch MathJax data from jsDelivr when a log renders math **[needs live check]**.
- **The coverage figures in §7** come from small synthetic simulations. The first real Replays' spread should replace them, as Q4.4 already says.
- **Q3.6's live checks** (Harness logins inside bwrap, turning each Harness's memory off) apply equally to every option and could change §1's sizes.
- **Dependency counts** for promptfoo, Inspect and MLflow are approximations from lockfiles and registry metadata, not installs.

## Sources

- **thirdshift** at `dfadafa`: `src/session.rs`, `src/process.rs`, `src/interrupt.rs`, `src/harness/interpretation/stream.rs`, `src/main.rs`, `Cargo.lock`, `tests/fakes/`, [ADR 0016](../adr/0016-the-console-is-a-long-running-reader.md), [`replay.py`](second-opinion-replays/replay.py), and the [#698 grilling](../../.scratch/grill-with-docs/done/698-evals/README.md).
- **promptfoo** 0.124.1: [source at `421e795`](https://github.com/promptfoo/promptfoo/tree/421e7959642c5d4cc1c983259a268de1c6f847b9) and the [npm registry](https://registry.npmjs.org/promptfoo), read 2026-10-21.
- **Inspect AI** 0.3.282: [source at `386e099`](https://github.com/UKGovernmentBEIS/inspect_ai/tree/386e09942510e93de2db6d108f24f62f65eecbdc) and [PyPI](https://pypi.org/project/inspect-ai/0.3.282/).
- **MLflow** 3.17.2: [source at `d0b397f`](https://github.com/mlflow/mlflow/tree/d0b397f85b602e8f146be0d0373b28bba2ca2b74), [PyPI](https://pypi.org/project/mlflow/3.17.2/), the [usage-tracking docs](https://mlflow.org/docs/latest/community/usage-tracking/) and the [v3.16.1 release notes](https://github.com/mlflow/mlflow/releases/tag/v3.16.1).
- **Mira** 0.5.0: [source at `334506e`](https://github.com/everruns/mira/tree/334506ef19a1d975dace0f2c51092548a10c7b3e) and the [`mira-eval` crate](https://crates.io/crates/mira-eval/0.5.0).
- **Statistics:** Bowyer, Aitchison and Ivanova, [arXiv 2503.01747v3](https://arxiv.org/abs/2503.01747); Miller, [arXiv 2411.00640](https://arxiv.org/abs/2411.00640); [`bayes_evals` at `b971184`](https://github.com/sambowyer/bayes_evals/tree/b9711844e00e9664258de321966ecd258a7273d6); Rubin, "The Bayesian Bootstrap", [Ann. Statist. 9(1), 1981](https://doi.org/10.1214/aos/1176345338); O'Hagan, Stevens and Campbell, "Assurance in clinical trial design", [Pharm. Stat. 4(3), 2005](https://doi.org/10.1002/pst.175); [`rand_distr` 0.6.0](https://crates.io/crates/rand_distr/0.6.0), [`rand` 0.10.3](https://docs.rs/crate/rand/0.10.3), [NumPy's random compatibility policy](https://numpy.org/doc/stable/reference/random/compatibility.html).
