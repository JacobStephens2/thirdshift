# CI queueing and runtime

Issue [#725](https://github.com/JacobStephens2/thirdshift/issues/725) separates
runner start delay from test execution. Follow-up suite performance belongs
with [#570](https://github.com/JacobStephens2/thirdshift/issues/570); the completed
[#174](https://github.com/JacobStephens2/thirdshift/issues/174) is a historical
baseline, not a target for the current host.

## Workflow policy

CI and Release cancel older runs only for the same workflow and PR number.
Using the PR number isolates fork branches with identical names. Every non-PR
run includes its run ID: even a pending main or tag run cannot be replaced by
GitHub's one-pending-run concurrency limit. Publication dependencies and tag
triggers are generated unchanged.

Release runs for paths that can affect packaged artifacts (`src/`, `skills/`,
Cargo manifests, toolchain, README/license), and for pipeline changes
(`.github/workflows/`, `scripts/`, `dist-workspace.toml`). Site, docs, generated
prompt copies and tests alone do not change the artifacts. Adding another
packaged input requires updating `RELEASE_PATHS` in the generator.

At implementation time, `gh api repos/JacobStephens2/thirdshift/branches/main/protection`
returned 404 “Branch not protected”, and `gh api repos/JacobStephens2/thirdshift/rules/branches/main`
returned `[]`. The factory's `src/github.rs` observes available checks and
accepts success/neutral/skipped; `scripts/release.sh` waits for existing workflow
runs and does not require Release on PRs. Thus there is no current required
Release check stranded by the path filter. If protection/rulesets later require
a Release check on every PR, replace the trigger filter with an always-created
gate before enabling that requirement.

CI and native Release builds use only Linux runners (#1666). CI keeps `ubuntu-24.04`
for x86_64 and `ubuntu-24.04-arm` for aarch64; Release builds only the two
static Linux musl targets. Native macOS has no CI jobs or release artifact.

### Job execution bounds

Every CI job and every PR-reachable Release job has a job-level
`timeout-minutes` (#1496). This bounds a hung job while CI watch waits for
its result; it does not add a deadline to CI watch or include runner queue
time. GitHub's [default job timeout is 360 minutes](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idtimeout-minutes).

The limits below were sized on 2026-10-28 UTC using `gh run list --workflow
<workflow> --status success` (separately filtering PR, push and schedule events),
then `gh run view <id> --json jobs`. Durations are `completedAt - startedAt`
for successful jobs, including setup and cleanup, excluding skipped jobs.

CI samples: PR runs [40613734239](https://github.com/JacobStephens2/thirdshift/actions/runs/40613734239),
[40609348854](https://github.com/JacobStephens2/thirdshift/actions/runs/40609348854),
[40608541526](https://github.com/JacobStephens2/thirdshift/actions/runs/40608541526);
main pushes [40609570774](https://github.com/JacobStephens2/thirdshift/actions/runs/40609570774),
[40606663194](https://github.com/JacobStephens2/thirdshift/actions/runs/40606663194),
[40600192827](https://github.com/JacobStephens2/thirdshift/actions/runs/40600192827);
and nightly [40503051764](https://github.com/JacobStephens2/thirdshift/actions/runs/40503051764).

| CI job | Observed execution (seconds) | Limit (minutes) |
| --- | --- | --- |
| `console-test` | 98–182 | 15 |
| `site-test` | 59–102 | 10 |
| `rust-x86 / run (n)` (per partition, or nightly cargo test) | 388–544 | 60 |
| `rust-arm / run (n)` (per partition, or nightly cargo test) | 420–476 | 60 |
| `rust-x86 / complete`, `rust-arm / complete` (coverage check) | 7–8 | 5 |
| `test`, `test-linux-arm` (stable gates) | 2–3 | 5 |

The `test` row's earlier 725–983 seconds came from the unsharded job above.
Partition, coverage and gate rows come from four-way [PR CI 43069335799](https://github.com/JacobStephens2/thirdshift/actions/runs/43069335799)
(#2031). Each partition job keeps the former whole-suite 60-minute bound, so
nightly's single `cargo test` worker keeps its existing budget; the gates only
read their architecture's result. Removed macOS jobs are excluded from these
active bounds.

Release samples: PR runs [40614474450](https://github.com/JacobStephens2/thirdshift/actions/runs/40614474450),
[40613734215](https://github.com/JacobStephens2/thirdshift/actions/runs/40613734215),
[40609348838](https://github.com/JacobStephens2/thirdshift/actions/runs/40609348838),
[40608541663](https://github.com/JacobStephens2/thirdshift/actions/runs/40608541663);
tag runs [40592949213](https://github.com/JacobStephens2/thirdshift/actions/runs/40592949213),
[40547557482](https://github.com/JacobStephens2/thirdshift/actions/runs/40547557482),
[40542265662](https://github.com/JacobStephens2/thirdshift/actions/runs/40542265662).
The sampled PRs skipped Rocky and WSL, so their execution figures come from
the tag runs of the same reusable workflow.

| PR-reachable Release job | Observed execution (seconds) | Limit (minutes) |
| --- | --- | --- |
| `plan` | 19–26 | 5 |
| `build-local-artifacts` (x86_64 Linux musl) | 180–319 (historical mixed-platform sample) | 20 |
| `build-local-artifacts` (aarch64 Linux musl) | Not yet measured on hosted CI | 20 |
| `build-global-artifacts` | 20–30 | 5 |
| `custom-release-smoke / gate` | 3–6 | 5 |
| `custom-release-smoke / rocky` | 11–16 | 5 |
| `custom-release-smoke / wsl` | 62–66 | 10 |
| `custom-release-smoke / arm-linux` | Not yet measured on hosted CI | 5 |

Full Rust suites get at least 3.6 times the slowest observed execution;
the browser jobs get at least 4.5 times. Sampled Release builds get
at least 3.7 times. Small orchestration jobs have a five-minute floor to
allow slow runner setup and artifact transfers; WSL has ten minutes for
distribution provisioning. These are operating budgets with headroom,
not guarantees against every possible healthy slowdown. Re-measure when
the work in a job grows.

GitHub does not accept `timeout-minutes` on a reusable-workflow call, so the
smoke limits live on its concrete jobs in `release-smoke.yml`. `host`,
`announce`, `custom-publish-crates` and `custom-release-notes` only execute
on tag publication; their jobs cannot hold a PR's CI watch and are outside
this change. Generated Release limits belong to `scripts/generate-release.py`.

### Linux test runner

Issue [#1317](https://github.com/JacobStephens2/thirdshift/issues/1317) moves
the full Linux Rust suite to `cargo nextest run` on PRs, pushes to `main` and
manual runs. The nightly Linux run keeps `cargo test`, so CI still exercises
the runner contributors use without installing nextest. Formatting, clippy,
Python tooling and Release regeneration still run for every event, behind the
same stable `test` gate.

Both runners cover every unit and integration test in this binary-only crate;
there is no library target or doctest coverage to lose. No test exclusion, retry
or test timeout is added. Nextest can schedule tests from different binaries at the
same time, while `cargo test` runs each binary in turn
([nextest's execution model](https://nexte.st/docs/design/how-it-works/)). The
existing default profile reports slow tests without terminating them. Both
Linux architectures use `rust-tests.yml`, with partition counts selected in
`ci.yml`. Nightly retains one full `cargo test` suite. The stable `test` and
`test-linux-arm` gates require all partitions and their coverage check to
succeed, using `always()` and accepting only `success`.

The successful [PR CI run 40191701060](https://github.com/JacobStephens2/thirdshift/actions/runs/40191701060)
on 2026-10-25 spent **692 seconds** in `cargo test`, out of **744 seconds**
for the Linux `test` job. Its macOS gate took 182 seconds, Console browser
checks 121 seconds and site checks 60 seconds. These are job execution times
from the Actions jobs API, excluding queue delay. Linux test execution was
the critical path; runner changes target that cost while preserving coverage.

The first successful [nextest PR run 40194983650](https://github.com/JacobStephens2/thirdshift/actions/runs/40194983650)
on the same date took **620 seconds** for `cargo nextest run` and **670 seconds**
for the Linux `test` job, including a one-second nextest installation. That is
about **10% less** for both the test step and the job than the baseline above.
These are separate CI runs, not a controlled benchmark; runner load and build
cache can vary, so this comparison does not establish the typical saving.

### Which runs test on ARM Linux

Issue [#1663](https://github.com/JacobStephens2/thirdshift/issues/1663) adds the
stable `test-linux-arm` check on `ubuntu-24.04-arm`. It runs the full suite on
pushes to `main`, nightly and manual runs. An ordinary PR skips it; a PR into
`main` labelled `test:linux-arm` opts in through the `labeled` activity type.
Any label starts a whole new CI run, x86_64 included, and cancels an older run
on the same PR. Label a PR after its CI run has finished: unfinished jobs from
the cancelled run stay on the same head as cancelled check runs, which the
factory's CI watch reads as failed (#925). The label stays on the PR, so later
pushes also run the full ARM suite.

The ARM work runs the same `rust-tests.yml` jobs as x86_64 Linux: the first
partition runs `cargo clippy --all-targets -- -D warnings`, and each of four
partitions runs `cargo nextest run --profile ci --partition hash:m/4`, followed
by the exact coverage check; nightly runs one `cargo test` worker instead. Each
worker has the same 60-minute bound and a separate architecture-specific Rust
cache. The runner class stays
`ubuntu-24.04-arm`; x86_64 uses `ubuntu-24.04`.

Releases also build `aarch64-unknown-linux-musl` on `ubuntu-24.04-arm` and
run its static-link/version smoke check before publication. That five-minute
smoke job shares the existing tag/PR gate with Rocky and WSL; it does not run
the full suite. Rocky and WSL coverage remains x86_64 only.

### When the nightly run is red

GitHub sends a scheduled workflow's failure
notification to the user who last changed its `schedule` cron
([GitHub: notifications for workflow runs](https://docs.github.com/en/actions/concepts/workflows-and-actions/notifications-for-workflow-runs)),
so the Day shift hears of it there, and sees it as the red run in the CI
workflow's history on `main`. Fix forward on `main`: open the fixing PR and label
it `test:linux-arm` when ARM coverage is needed, so its own run shows that suite
green before it merges.

## Generated Release ownership

Run `python3 scripts/generate-release.py` from the repository root; CI runs
it with `--check`. Install the pinned cargo-dist from `dist-workspace.toml`.
The generator checks that version, runs the real cargo-dist generator and adds
PR paths, concurrency copied from CI, PR-job permissions and execution bounds,
and its regeneration header. It restores the exact
original config and workflow on generation failure; `--check` also restores the
workflow before comparing. Build, smoke, host and publication jobs still come
from cargo-dist and its configured reusable-workflow hooks, with the PR-job
policy applied by our generator.

Cargo-dist 0.33 has no concurrency/path-filter configuration or template override.
Its documented [`allow-dirty = ["ci"]`](https://github.com/axodotdev/cargo-dist/blob/v0.33.0/book/src/reference/config.md#allow-dirty)
permits customization. This disables its own CI freshness check, so our CI
explicitly checks the complete customized output. During generation the script
temporarily removes that opt-out, then restores it; `dist generate --allow-dirty`
would skip generation too. A changed upstream workflow shape fails generation
rather than silently dropping the policy. Do not edit release.yml by hand.

## Slow-test reporting and coverage

### Retained timings and partition benchmarks (#2031)

Non-nightly Rust jobs use `cargo nextest run --profile ci`, with the same
selection, ignored-test policy, deadlines and failure semantics as the default
profile. The CI profile writes JUnit for every executed test, retaining output
only for failures. Each partition uploads its JUnit, current unsharded and
partition JSON inventories, source revision, cache-hit result and summary for
14 days, including after test failure. Setup failures may leave no report;
they still fail the architecture gate. Nightly keeps the normal `cargo test`
runner and does not run a second suite just to generate JUnit.

`scripts/ci-timings.py summary <report-directory>` emits total test-seconds,
per-binary totals and the twenty slowest tests, plus `summary.json`. Test-seconds
sum individual test durations, including overlapping tests; they are work,
not elapsed time. `verify --count N <partition-directories...>` requires the
current unsharded inventories to agree, each selected test to belong to exactly
one partition, and each partition's JUnit execution to match its selection.
It reports partition balance in test-seconds. Counts come from the current
inventory, including unit and integration binaries, rather than a fixed count
from a historical run. Ignored tests remain in the inventories and remain
unselected.

Run a controlled benchmark from a branch containing the workflow:

```sh
gh workflow run ci.yml --ref <branch> -f benchmark=true
```

This opt-in dispatch runs one, two and four `hash:m/n` partitions on each Linux
runner class at the dispatch's exact revision. Normal CI and its architecture
gates also run. Independent cached builds retain the existing Rust compilation
model: no build-once archive/distribution approach is proposed or installed.
Thus build-artifact size and transfer cost are zero; Rust cache transfer and
timing-report upload remain visible in the step timings. Do not infer that an
archive would be faster without separately measuring its size, upload,
download, extraction, dependency delay and relocatability on this suite.

The `benchmark-report` artifact contains the Actions run and jobs API records
and a machine-readable comparison. Its unattended table reports elapsed
critical path from workflow creation through each variant's coverage check,
and runner minutes summed over that variant's workers and coverage check.
These are actual allocated runner durations, not billing-rounded minutes;
the report job and normal CI jobs are shared overhead outside each variant.
Per-job records separate start delay (including queue and dependency delay),
setup/toolchain/cache steps, test compilation, inventory, execution and report
transfer. API timestamps have one-second resolution. Repeat the dispatch at
the same revision to observe variance and distinguish cache-hit/miss reports;
one sample, especially a small difference, cannot establish an improvement.

The manual `probe=failure` input fails only the first of the four partitions;
`probe=skipped` skips all partition work. Both deliberately produce red
architecture gates without running tests. They are diagnostics, never a way
to report green coverage. The default `probe=none` executes all required work.
The workflow's normal PR, main and nightly policies and Release dependencies
are unchanged.

### Measured selection: four partitions (2026-11-11)

The controlled samples are [cold run 43059270501](https://github.com/JacobStephens2/thirdshift/actions/runs/43059270501)
and [warm run 43060539830](https://github.com/JacobStephens2/thirdshift/actions/runs/43060539830).
Every variant checked out **`deb46450b2ce95acd3225327bd4262850ba308ae`**,
using nextest 0.9.151 and the pinned Rust toolchain. Both use the hosted Linux
runner classes named below. All fourteen cold workers reported cache misses
(the inspected cache log says “No cache found”); all fourteen warm workers
reported cache hits. Workspace tests still compile on each worker. All six
variants in each run passed their exact inventory/execution coverage check:
4,233 selected tests on x86_64, 4,232 on ARM, and 97 ignored on each. The
architecture difference is preserved, rather than imposing x86's count on ARM.

Elapsed seconds include start delay through the variant's coverage gate;
runner minutes include its workers and coverage gate. These benchmark paths
exclude normal CI's clippy, Python, formatting and Release-generator checks.
The normal configuration is measured separately below.

| Runner class | Partitions | Cold elapsed s | Cold runner min | Warm elapsed s | Warm runner min |
| --- | ---: | ---: | ---: | ---: | ---: |
| ubuntu-24.04 | 1 | 1418 | 23.48 | 1608 | 26.68 |
| ubuntu-24.04 | 2 | 915 | 28.93 | 851 | 27.52 |
| ubuntu-24.04 | 4 | 514 | 31.53 | 514 | 31.05 |
| ubuntu-24.04-arm | 1 | 1349 | 22.30 | 1342 | 22.22 |
| ubuntu-24.04-arm | 2 | 861 | 25.67 | 761 | 24.50 |
| ubuntu-24.04-arm | 4 | 514 | 29.88 | 490 | 28.65 |

Four partitions shorten the measured critical path by **62–68% versus one**
and **36–44% versus two**, at **16–34% more runner minutes than one** or
**9–17% more than two**. We select four for non-nightly runs on both
architectures. Nightly remains one `cargo test` suite. The elapsed improvement
is much larger than the observed sample variation (about 14% for the x86
baseline and 12% for two-way ARM). Four-way x86 elapsed was unchanged; ARM
varied by under 5%. Two samples do not establish a typical production saving,
and the small cold/warm difference is not evidence of a cache-induced speedup.

The following per-worker ranges separate phases in seconds. They are ranges
across a variant's workers, not additive pieces of one critical-path worker.
Start delay is workflow creation to worker start, including queue delay;
coverage-gate dependency delay is included in the elapsed table above.
Setup excludes cache transfer; report transfer means timing artifacts, not
compiled-build distribution. Cleanup and gaps between steps remain included
in runner minutes and elapsed time. Both whole-run jobs APIs retain the
individual step timestamps; `ci-timings.py benchmark` exposes them and their
phase totals in `benchmark.json`.

| Arch | Cache | Parts | Start delay | Setup | Cache transfer | Compile | Inventory | Tests | Report transfer |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| x86 | cold | 1 | 5 | 14 | 1 | 141 | 2 | 1238 | 1 |
| x86 | cold | 2 | 5–6 | 13–18 | 1–3 | 152 | 1–2 | 660–708 | 1–2 |
| x86 | cold | 4 | 5–9 | 11–14 | 0–1 | 113–152 | 1–2 | 268–344 | 1 |
| ARM | cold | 1 | 8 | 10 | 2 | 126 | 1 | 1183 | 2 |
| ARM | cold | 2 | 7–8 | 9–10 | 1–2 | 144–150 | 1 | 592–610 | 1 |
| ARM | cold | 4 | 7–8 | 8–10 | 0–4 | 121–134 | 1 | 278–320 | 1–2 |
| x86 | warm | 1 | 4 | 14 | 5 | 85 | 2 | 1481 | 1 |
| x86 | warm | 2 | 4–5 | 13–14 | 4–7 | 80–97 | 2 | 706–713 | 1 |
| x86 | warm | 4 | 4–5 | 12–13 | 4–7 | 69–96 | 1–2 | 309–382 | 1–2 |
| ARM | warm | 1 | 6 | 9 | 4 | 97 | 1 | 1208 | 2 |
| ARM | warm | 2 | 6–7 | 9 | 4–5 | 74–103 | 1 | 622–627 | 1 |
| ARM | warm | 4 | 7–39 | 10–12 | 4–6 | 76–110 | 1–2 | 285–336 | 1–2 |

Four-way work totals and balance are retained in each worker's `summary.json`
and its job summary, including all binary totals and the slowest twenty tests.
Partition values and totals below are rounded independently.
Cold x86 partitions used 1,062.9 / 1,276.3 / 1,285.7 / 1,361.9 test-seconds
(4,986.7 total); warm used 1,414.2 / 1,454.6 / 1,515.7 / 1,223.6 (5,608.1).
Cold ARM used 1,103.7 / 1,151.8 / 1,217.6 / 1,265.2 (4,738.4); warm used
1,132.8 / 1,174.9 / 1,302.1 / 1,330.8 (4,940.6). Hash membership is stable;
timings vary with runner load. This is measured balance, not ideal linear
speedup. Each report retains timings without successful-test output.

The delivered four-worker configuration also passed [normal PR CI 43060775809](https://github.com/JacobStephens2/thirdshift/actions/runs/43060775809)
on PR head `206261e4`, checkout merge `811e37d6`, with ARM opted in. Including
each architecture's coverage and stable gate, x86 took **827 s / 36.37 runner
minutes** and ARM **712 s / 30.75 runner minutes**. This includes normal
tooling and clippy, and overlap with the benchmark runs; it is validation of
the delivered workflow, not another controlled sample at the benchmark SHA.
Browser checks and PR Release builds also passed and are outside those
Rust-path totals.
More workers can queue behind an account's other work: the warm benchmark's
ARM worker start delays reached 39 seconds; normal CI workers started
20–226 seconds after workflow creation. Re-measure under different concurrency
limits rather than assuming that four runners will always start together. Build archives were not
introduced or benchmarked, and no archive-based saving is claimed.

On the delivered four-way workflow at `d8e6b3cb`, the [single-partition failure probe](https://github.com/JacobStephens2/thirdshift/actions/runs/43131195970)
failed only `run (1)` on each architecture while partitions 2–4 succeeded, and
the [skipped-work probe](https://github.com/JacobStephens2/thirdshift/actions/runs/43131198843)
skipped every partition; both made `complete`, `test` and `test-linux-arm`
fail. Earlier probes [43059533495](https://github.com/JacobStephens2/thirdshift/actions/runs/43059533495)
and [43059538541](https://github.com/JacobStephens2/thirdshift/actions/runs/43059538541)
ran the one-partition workflow at `deb46450`. The probe fails before tests
run, so hosted CI has not yet uploaded JUnit after a real test failure; that
upload step uses `always()`. Local tests execute those gate
commands for success, failure, cancellation, skipped and missing results, and
reject missing artifacts, duplicate membership, differing revisions and
incomplete execution. Every enabled diagnostic probe exits unsuccessfully,
even if a caller supplies an arbitrary value such as `success`.

Nextest reports at 30 seconds, half the retrospective's 60-second long-tail
boundary, so a growing test appears before joining that tail. The
[nextest slow-test contract](https://nexte.st/docs/features/slow-tests/) treats
this as reporting only: no termination is configured, failures remain failures,
and no retries are added. Slow tests remain visible in the final summary.
Both Linux architectures use the runner policy above.

The final-message matrix now crosses `Review::from_final_message` and `Outcome`
in process, preserving all 11 original malformed cases and missing `None`.
Four end-to-end Ticket runs retain missing/malformed messages in both Self-merge and
report-only modes, asserting exit status, public reason and ready/open PR.
The separate end-to-end private-report validation and deliberate-incomplete
cases remain. The unauthenticated publisher test remains end to end: its
per-Harness credential isolation is a process behavior, not a parser table.

## Historical measurements

The macOS measurements below describe the workflow before #1666 removed native
macOS CI. They are retained as performance history; none describes active jobs.

The successful [main CI run 38990121846](https://github.com/JacobStephens2/thirdshift/actions/runs/38990121846)
created 2026-10-17 01:16:05 UTC was queried through the jobs API. Start delay is
workflow creation to job start, including startup; it is not a scheduler-only
queue metric. Execution is job start to completion, including setup/build.

| macOS shard | Start delay (seconds) | Execution (seconds) |
| --- | ---: | ---: |
| 1 | 1528 | 984 |
| 2 | 434 | 856 |
| 3 | 2396 | 860 |
| 4 | 1695 | 842 |

Local targeted baseline on x86_64 Linux, Rust 1.99.0: the original 22-Run
final-line table passed in **72.42 s test execution**, with **190.84 s** total
wall time including an initial **118 s** compile. Load at observation was about
7. This is not a macOS result, and cannot establish hosted queue savings.

The PR records after-measurements and supersession observations. Live tag
publication is not triggered just to benchmark CI; tag isolation is checked
against the event concurrency expressions and unchanged publication graph.

After the move, the in-process table passed in **<0.01 s** and the four
end-to-end Runs passed in **12.28 s** test execution (**15.44 s** total,
including **3.02 s** compilation). These targeted executions on the same host
show the table's cost fell; concurrent host load and macOS runtime can differ.
The expected behavior and all original malformed inputs remain covered.

## Where macOS test time goes

Issue [#846](https://github.com/JacobStephens2/thirdshift/issues/846) asked for
shorter macOS CI. In [main CI run 39337614995](https://github.com/JacobStephens2/thirdshift/actions/runs/39337614995)
each macOS shard built in about 33 seconds, then spent 990 to 1220 seconds
running its quarter of the suite, so the tests themselves are the cost.

Locally on x86_64 Linux, the Run tests are bound by spawning processes:
`spec_run a_ticket_needs_only_a_trusted_author_not_a_ready_label_of_its_own`
used 12.3 s of system time and 6.2 s of user time in 19.6 s, and ran 5,415 programs.
About 82% were git commands from Worktree ownership inspection, which runs at
least twice around every ordinary Worktree operation. Spawning a process costs
more on macOS, where the same tests run about five times slower.

Each inspection now reads the checkout's top level, administrative directory,
common directory and HEAD with one `git rev-parse` instead of four. The checks
are unchanged: when a path holds a newline, so the output is not one line per
value, `Git::checkout_location` reads each value with its own process again. That test then ran 4,221 programs (22% fewer) in about
17.2 s on the same host; concurrent host load and macOS runtime can differ.
`an_ordinary_operation_inspects_ownership_in_few_git_processes` holds one
ordinary operation to its 23 git processes.

### After #857: the macOS breakdown

Issue [#894](https://github.com/JacobStephens2/thirdshift/issues/894) asked
where the macOS shards' 3,900-4,250 seconds still go. Five passes of a
temporary diagnostic workflow on `macos-26` (3 vCPU Apple M1, macOS 26.6,
Homebrew git 2.56.0 first on `PATH`) measured it. It lived only on the
`issue-894-profile` branch:

| Pass | Run | What it measured |
| --- | --- | --- |
| 1 | [39544139072](https://github.com/JacobStephens2/thirdshift/actions/runs/39544139072) | 23 of the 26 tests below, each alone; one `spec_run` partition at 3 and 6 threads |
| 2 | [39545174138](https://github.com/JacobStephens2/thirdshift/actions/runs/39545174138) | seven of them alone under dtrace; the same seven with git's fsync off and with Xcode's git; one process of each program |
| 3 | [39546189462](https://github.com/JacobStephens2/thirdshift/actions/runs/39546189462) | the four shards with Spotlight off and Xcode's git; the seven with Spotlight off |
| 4 | [39547327478](https://github.com/JacobStephens2/thirdshift/actions/runs/39547327478) | the four shards with each change alone |
| 5 | [39551482447](https://github.com/JacobStephens2/thirdshift/actions/runs/39551482447) | the other 19 tests alone under dtrace |

The 26 tests are the 16 that [main CI run 39430050727](https://github.com/JacobStephens2/thirdshift/actions/runs/39430050727)
flagged over 60 seconds and ten more of the slowest in
[39536115478](https://github.com/JacobStephens2/thirdshift/actions/runs/39536115478),
12 of the 26 in `spec_run`. Pass 1 measured 23 of them.

"Alone" means the test binary run directly, `<binary> --exact <test>
--test-threads 1`, from a Python driver that read wall time and the
`RUSAGE_CHILDREN` CPU time of the test and every process it reaped. Under
dtrace (SIP is off on the hosted image) the driver ran `sudo dtrace` with
`progenyof(<driver pid>)` predicates: `proc:::exec-success` and
`proc:::exit` counted each program the test started and summed its lifetime;
`fsync` and `fcntl(F_FULLFSYNC | F_BARRIERFSYNC)` and `__semwait_signal`
(where `thread::sleep` waits) summed by program; `profile-997` sampled every
CPU, system-wide, by program. The same tests were then straced on x86_64
Linux, whose git command lines, unlike dtrace's program names, say which
code ran each git. Over the 26 tests the two platforms' git counts agree
within 1% in total and within 12% for each Run test; the worktree unit test,
which drives git through its own scripts, ran 0.61 times as many on macOS.

**The slowest tests are git processes.** Alone, each of pass 1's 23 tests
was on a CPU for 66-93% of its wall time (78% on average), so it waits
little: no fixed sleep or poll interval dominates. Under dtrace, git
processes were alive for 77-99% of each test's wall time, 88% summed over
all 26:

| Test (alone) | Pass | Wall s | CPU s | macOS git processes | git alive s | Linux git processes | Launch recheck | Ownership inspection | Other git |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `spec_run security_findings_refusals_and_incomplete_reviews_hold_only_the_spec_self_merge` | 5 | 259.9 | 190.3 | 15,606 | 213.2 | 15,360 | 41% | 45% | 14% |
| `security_run unauthenticated_public_publishers_stage_single_fixes_and_specs_with_every_harness` | 5 | 234.1 | 168.2 | 15,112 | 189.6 | 14,800 | 42% | 43% | 15% |
| `spec_run security_review_on_a_spec_obeys_config_and_command_overrides` | 5 | 116.1 | 91.4 | 8,582 | 103.5 | 8,442 | 41% | 45% | 13% |
| `security_review repeated_fingerprints_reuse_private_records_and_preserve_day_shift_grades` | 5 | 116.0 | 82.3 | 6,020 | 93.5 | 5,923 | 42% | 44% | 14% |
| `spec_run without_a_limit_at_most_three_tickets_run_at_once` | 5 | 95.8 | 79.3 | 5,158 | 94.5 | 5,174 | 41% | 46% | 13% |
| `pickup_run pickup_wait_in_the_user_config_shortens_lengthens_or_disables_settling` | 2 | 89.7 | 67.3 | 4,946 | 79.4 | 4,781 | 38% | 39% | 23% |
| `spec_run a_review_required_spec_pr_stays_ready_without_an_attempt_or_another_round` | 5 | 89.0 | 67.4 | 4,208 | 80.5 | 4,143 | 41% | 45% | 13% |
| `architect_run base_carries_the_named_branch_to_the_dispatched_spec_run_whatever_is_checked_out` | 5 | 88.3 | 68.2 | 4,410 | 79.6 | 4,378 | 41% | 44% | 15% |
| `spec_run spec_delivery_rejects_late_retargeting_and_keeps_the_original_identity` | 2 | 83.9 | 69.0 | 7,630 | 77.0 | 7,495 | 41% | 45% | 14% |
| `spec_run successful_ticket_completion_with_an_invalid_spec_pr_starts_no_review` | 5 | 60.4 | 47.5 | 3,011 | 56.2 | 2,953 | 41% | 43% | 16% |
| `issue_number a_number_resolves_supported_origins_from_nested_directories` | 2 | 60.3 | 41.0 | 3,185 | 48.5 | 3,129 | 40% | 42% | 18% |
| `bin worktree::acquisition_tests::recovery_retains_changed_checkout_authority_during_removal` | 2 | 58.9 | 52.6 | 4,486 | 58.0 | 7,365 | 29% | 23% | 48% |
| `architect_run base_fix_reaches_each_tickets_run_of_the_dispatched_spec_run` | 5 | 55.0 | 43.7 | 3,036 | 50.2 | 2,987 | 42% | 45% | 14% |
| `spec_run repairs_exhausted_on_the_spec_pr_send_it_back_to_draft_and_exit_1` | 5 | 52.4 | 41.1 | 2,994 | 46.3 | 2,954 | 40% | 45% | 14% |
| `architect_run merge_merges_the_runs_pull_request_or_the_spec_pr` | 5 | 49.9 | 40.9 | 2,985 | 47.8 | 2,969 | 41% | 44% | 15% |
| `ci a_check_re_run_cannot_deliver_or_merge_without_readable_nonempty_checks` | 2 | 48.7 | 34.6 | 3,078 | 39.0 | 3,022 | 41% | 45% | 14% |
| `spec_run a_linear_spec_lands_each_ticket_in_order_then_opens_a_ready_spec_pr` | 2 | 47.3 | 32.8 | 1,900 | 36.5 | 1,872 | 41% | 45% | 14% |
| `spec_run independent_tickets_run_at_once_and_the_ticket_they_block_waits_for_both` | 5 | 46.2 | 38.0 | 2,948 | 43.6 | 2,913 | 41% | 46% | 13% |
| `ci a_repair_that_already_re_ran_ci_to_green_delivers_or_self_merges_the_unchanged_head` | 5 | 44.2 | 35.9 | 3,146 | 39.6 | 3,090 | 41% | 45% | 14% |
| `spec_run a_spec_pr_replaced_during_review_gets_its_checklist_on_the_delivered_identity` | 5 | 44.1 | 36.0 | 3,638 | 40.2 | 3,582 | 41% | 45% | 14% |
| `spec_run two_tickets_meeting_the_same_inherited_failure_share_one_base_fix` | 5 | 42.5 | 36.5 | 3,742 | 40.5 | 3,344 | 42% | 46% | 13% |
| `spec_run two_tickets_starting_together_in_one_launch_directory_both_get_their_worktrees` | 5 | 39.1 | 31.9 | 2,592 | 36.8 | 2,924 | 42% | 45% | 13% |
| `run_notification delivery_failures_name_the_stopped_stage_in_terminal_and_notification` | 5 | 36.0 | 27.3 | 2,896 | 30.3 | 2,848 | 41% | 45% | 14% |
| `architect_run base_fix_lets_the_dispatched_run_start_a_base_fix` | 5 | 36.0 | 30.0 | 2,976 | 33.4 | 2,928 | 41% | 44% | 15% |
| `security_run status_tracks_security_stages_and_nests_the_dispatched_fix` | 5 | 34.4 | 27.4 | 2,648 | 29.4 | 2,610 | 44% | 37% | 19% |
| `merge_run unblocked_or_unknown_review_decisions_allow_the_guarded_merge` | 2 | 30.6 | 24.8 | 2,680 | 28.3 | 2,636 | 41% | 44% | 15% |

The last three columns split the Linux trace's git processes; summed over
the 26 tests they are 41%, 43% and 17%.
"Launch recheck" is `git rev-parse --git-common-dir`, mostly
`Directory::verify_repository`, which
[#858](https://github.com/JacobStephens2/thirdshift/issues/858) holds.
"Ownership inspection" is `worktree list --porcelain -z`, the combined
`rev-parse --show-toplevel …`, `rev-parse --symbolic-full-name HEAD` and the
branch `for-each-ref`. "Other git" is everything else, including git's own
subprocesses and the scenario's setup. Everything else the tests start is
small: the fake `gh`, `claude` and `codex` lived 1-3% of each test's wall
time, and shells under 2%, except in the worktree unit test, whose own
fake-git scripts ran 141 shells for 6 s, and
`spec_run without_a_limit_at_most_three_tickets_run_at_once`, 6 s of
shells in 96 s.

What did not matter:

- **fsync.** Git's fsyncs took under 0.1 s in each test. With
  `core.fsync = none` the seven tests took 344 s against 346 s.
- **Sleeps and polls.** thirdshift's own `thread::sleep` totals (up to 60 s
  in a Spec run test) are a parent waiting at the 100 ms poll while its child
  Runs work. That overlaps their git time and adds no wall time.
- **More test threads.** One third of `spec_run` took 493 s at 3 threads and
  461 s at 6 (-6%): CPU use stayed at about 58% of three cores either way.

What did:

- **Each git process costs more on macOS.** One process of each program,
  started 200 times in a scratch clone (pass 2), median:

  | Program | macOS ms | x86_64 Linux ms |
  | --- | ---: | ---: |
  | `/usr/bin/true` | 1.7 | 1.1 |
  | `/bin/sh -c true` | 4.2 | 1.2 |
  | Homebrew `git rev-parse --git-common-dir` | 13.2 | 2.4 |
  | Xcode `git rev-parse --git-common-dir` | 9.1 | |
  | `/usr/bin/git --version` (the `xcrun` shim) | 16.7 | |
  | the fake `gh`, debug build | 3.7 | 3.7 |
  | `thirdshift --version`, debug build | 7.7 | |

  Homebrew's git loads `pcre2` and `gettext` from `/opt/homebrew`, outside
  the dyld shared cache. Under the shards' parallel load the gap grows.
- **Spotlight.** In the dtrace pass, `mdworker_shared`, `mds_stores` and
  `mds` took 18-35% of all CPU samples in the first three tests after the
  build, against git's 18-21%. They index the repositories the tests create.

**The changes at the time.** Both macOS jobs in CI turned Spotlight indexing off
(`sudo mdutil -a -i off`) and put Xcode's git (`xcrun -f git`, 2.50.1 on
this image) first on `PATH`, with the `git-*` programs beside it. Summed
nextest time of the four shards, build excluded:

| Runs | Shards 1 / 2 / 3 / 4 s | Sum s |
| --- | --- | ---: |
| `main`, five runs from [39511690293](https://github.com/JacobStephens2/thirdshift/actions/runs/39511690293) to [39536115478](https://github.com/JacobStephens2/thirdshift/actions/runs/39536115478) | 595-994 / 789-1,012 / 773-981 / 728-951 | 3,327-3,595 |
| Spotlight off (pass 4) | 800 / 835 / 852 / 722 | 3,209 |
| Xcode's git (pass 4) | 549 / 475 / 571 / 537 | 2,132 |
| Both (pass 3) | stopped early / stopped early / 341 / 555 | |

Xcode's git alone took 38% off the mean of `main`'s five runs. Spotlight off
alone was 7% below that mean, inside the ±15% run-to-run noise; it is kept for
the CPU the samples show it freeing. In passes 3 and 4 two `security_run`
tests failed under Xcode's git: they run thirdshift with only a link to git on
`PATH`, and Xcode's git finds `git-upload-pack` only beside itself, not in
its exec path as Homebrew's and Linux's do. They now link it too.

**Follow-up at the time.** 83% of the git processes in the 26 tests were Worktree
ownership inspection and the launch-side recheck. Both are
read-only here; the macOS numbers above are on #858, which decides whether
inspection may recheck less often. Linux CI keeps Ubuntu's git.
