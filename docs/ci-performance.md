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
gate before enabling that requirement. CI's stable `test-macos` aggregate stays
unfiltered, with `always()` and an explicit success-only test rejecting failed,
cancelled or skipped shards.

## Generated Release ownership

Run `python3 scripts/generate-release.py` from the repository root; CI runs
it with `--check`. Install the pinned cargo-dist from `dist-workspace.toml`.
The generator checks that version, runs the real cargo-dist generator and adds
only PR paths, concurrency copied from CI, and its regeneration header. It restores the exact
original config and workflow on generation failure; `--check` also restores the
workflow before comparing. Build, smoke, host and publication jobs still come
entirely from cargo-dist and its configured reusable-workflow hooks.

Cargo-dist 0.33 has no concurrency/path-filter configuration or template override.
Its documented [`allow-dirty = ["ci"]`](https://github.com/axodotdev/cargo-dist/blob/v0.33.0/book/src/reference/config.md#allow-dirty)
permits customization. This disables its own CI freshness check, so our CI
explicitly checks the complete customized output. During generation the script
temporarily removes that opt-out, then restores it; `dist generate --allow-dirty`
would skip generation too. A changed upstream workflow shape fails generation
rather than silently dropping the policy. Do not edit release.yml by hand.

## Slow-test reporting and coverage

Nextest reports at 30 seconds, half the retrospective's 60-second long-tail
boundary, so a growing test appears before joining that tail. The
[nextest slow-test contract](https://nexte.st/docs/features/slow-tests/) treats
this as reporting only: no termination is configured, failures remain failures,
and no retries are added. Slow tests remain visible in the final summary.
Linux retains `cargo test`; macOS retains the same four hash partitions.

The final-message matrix now crosses `Review::from_final_message` and `Outcome`
in process, preserving all 11 original malformed cases and missing `None`.
Four end-to-end Runs retain missing/malformed messages in both Self-merge and
report-only modes, asserting exit status, public reason and ready/open PR.
The separate end-to-end private-report validation and deliberate-incomplete
cases remain. The unauthenticated publisher test remains end to end: its
per-Harness credential isolation is a process behavior, not a parser table.

## Measurements

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
are unchanged. That test then ran 4,221 programs (22% fewer) in about 17.2 s.
`an_ordinary_operation_inspects_ownership_in_few_git_processes` holds one
ordinary operation to its 23 git processes.
