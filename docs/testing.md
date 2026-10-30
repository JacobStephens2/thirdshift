# Local test selection

Run `scripts/affected-tests main` from anywhere in this checkout. It requires
Python 3 and Git, compares the working tree with the merge-base of the supplied
ref and `HEAD`, and includes staged, unstaged and untracked non-ignored files.
Renames include both paths. An invalid ref fails before running checks; an empty
diff prints that no tests are affected. Before each selected command starts,
the script prints its full-output log path. After it finishes, one summary line
shows the command, `PASS` or `FAIL`, wall time in seconds and the log path. Raw
check output stays in the log. Failures also print failing Rust test names and
panic messages, or failing `node --test` cases for the browser suites, so the
summary is enough to start investigating. Any failed command fails the entry
point and stops the remaining checks.

Logs live in `.affected-tests/` at the checkout root, which Git ignores. Each
command gets a separate log named after the command with a unique suffix;
subsequent runs keep earlier logs. Both stdout and stderr are saved in full.
Run the script in the foreground and read its summary; open the printed log
when more context is needed. Logs may be deleted when no longer needed.

```text
+ cargo nextest run (log: /checkout/.affected-tests/cargo-nextest-run-abc123.log)
PASS cargo nextest run (42.15s; log: /checkout/.affected-tests/cargo-nextest-run-abc123.log)
```

| Changed paths | Checks |
| --- | --- |
| `site/**`, `docs/guide.md` | `npm test --prefix site`; `cargo test --test share_card --test site_deploy`; `cargo test --bin thirdshift prompts_page` |
| `console/**`: the Console browser tests, their fixtures and packages | Console browser tests: `cargo build --locked`, then `npm test --prefix console` |
| `src/console.rs`, `src/console/**` (including templates and served assets), `src/config/console.rs`, `src/config/console/**`, and the shared fixtures the Console browser tests read: `tests/console/scoring_fixture.py`, `tests/fixtures/waterfall-{logs,spans}.jsonl` | Console browser tests and the full Rust suite |
| `scripts/release*`, `scripts/generate-release.py`, `scripts/test_generate_release.py` | `cargo test --test release`; `python3 -m unittest discover -s scripts -p test_generate_release.py` |
| Everything else, including `src/**`, shared test support/fakes, Cargo files and CI configuration | Full Rust suite |
| Several areas together | Full Rust suite plus any selected site, Console and release-generator checks; the full suite subsumes the targeted Rust checks |

Unknown paths print the reason for selecting the full Rust suite, and Console
paths print the reason for selecting the Console browser tests. The mappings
are deliberately narrow: adding a new area cannot silently opt it out. When a
Console browser test starts reading another file outside `console/`, add that
file to the Console source paths in `scripts/affected-tests`. For changes to
the selector or other Python tooling, also run
`python3 -m unittest discover -s scripts -p 'test_*.py'`, as CI does.

The full Rust suite uses `cargo nextest run` when `cargo nextest --version`
succeeds, otherwise it prints a fallback note and uses `cargo test`. Nextest is
optional; install it with `cargo install cargo-nextest --locked`. Both runners
cover the binary's unit tests and integration tests; this crate has no library
target or doctests. For a focused iteration, use
`cargo test --bin thirdshift <module>::tests` or `cargo test --test <file>`
(the file name without `.rs`).

Some tests give a Trial, grader or Harness a deadline of a second or less, and
that deadline covers setup as well as the fake agent. On a busy many-core host,
running every test at once can use up the deadline before the fake agent starts.
This was observed on an 18-core host (#1472). CI runs on Linux; a local run on
a busy host can still hit the same deadlines. If timeout tests fail locally but
pass alone, rerun with fewer threads, for example `cargo test -- --test-threads=3` or
`cargo nextest run --test-threads 3`.

Set up browser dependencies following [the site instructions](../site/tests/README.md)
before selecting site checks, and [the Console instructions](../console/README.md)
before selecting Console checks. A missing Node, npm or Playwright dependency
fails visibly rather than dropping that check. Site generation checks remain
part of `npm test`; regenerate prompt changes as described in
[AGENTS.md](../AGENTS.md#generated-prompts).

## Local work and CI

During implementation, run typechecking and focused tests. Before additional
validation, check the branch's PR state: stop after it is merged or closed.
Reuse successful CI for the exact head instead of repeating equivalent full-suite
validation. Changes after that head need validation again. Push and open the PR
while work is still in progress when practical so CI can overlap local work;
this does not change Delivery or CI policy.

Local selection shortens feedback; CI retains its broader checks on every PR,
including Python tooling, formatting, clippy, site and Console browser tests,
and the Linux Rust suite. CI uses only Linux runners: x86_64 on every PR and
aarch64 on `main` pushes, nightly and manual runs, plus PRs into `main` labelled
`test:linux-arm`. See [CI performance](ci-performance.md). This script selects
Rust, site, Console browser and release checks; it does not replace CI's
coverage of both Linux architectures. Run local checks in Linux, including
an ARM Linux VM when contributing from an Apple Silicon Mac.

Measure local full-suite time with `/usr/bin/time -p cargo nextest run` (or
`/usr/bin/time -p cargo test` for the fallback), recording the command, result,
wall time and whether compilation was included in the PR's evidence. Timings
depend on host load and build cache; compare them with CI execution time rather
than CI queue delay.

## Console session expiry

For the Linux Console session-expiry regression, first build the binary, then run
`python3 tests/console/session-expiry.py --binary target/debug/thirdshift --settle-seconds 65`.
It needs Python 3 and a C compiler. Its child-only clock shim advances the Console's
wall clock by 30 days while leaving the host clock and monotonic timers unchanged.
After one recovery interval, open Inbox and log streams must close and receive no
new Command notices or log lines. The script cleans up its private Console and compiled shim.

## Setup PTY child lifetime

`Scenario::run_terminal` kills and reaps its immediate child when it returns or
unwinds, and bounds prompt and exit waits. Both PTY descriptors are close-on-exec;
the child gets a new session and controlling terminal so interruption and
terminal-restoration tests exercise normal terminal behavior.

On Linux the fixture also arms `PR_SET_PDEATHSIG` with SIGKILL before exec, and
checks the parent PID after arming it to close the parent-exit race. The child
ends when its spawning test thread dies, including SIGKILL of the test process,
even if another process retains a PTY master. This covers the immediate terminal
child, not descendants it may start. Production commands are unchanged. See
the [Linux parent-death signal contract](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html).

`cargo test --test setup_lifetime` runs real setup children in separate fixture
processes for successful completion, assertion unwinding, an exit timeout,
SIGKILL, and SIGKILL with an inherited PTY master held open. The supervisor uses
Linux's [subreaper facility](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html)
to adopt and reap exact recorded orphan PIDs. It owns the fixtures' temporary
subtrees and cleans up its recorded children even on a failed assertion.

## Release fixture child lifetime

Linux release-script and script-function tests use `FixtureChild` from the same
child-lifetime helper. Each command has a 120-second wait bound, with stdout and
stderr included in a timeout panic. File-backed input and output keep inherited
descriptors and large input from blocking that wait.

A dedicated guardian process acts as a Linux subreaper for each command. Normal
completion, timeout and assertion unwinding kill and reap its owned descendants,
including release-summary Harnesses that start new process groups. If SIGKILL
ends the fixture harness, EOF on the guardian's private, close-on-exec stdin pipe
requests the same cleanup before the guardian exits. The immediate command also
uses the setup helper's Linux parent-death signal. Cleanup names only unreaped
children of that guardian; git maintenance remains disabled in these fixtures.

`cargo test --test release lifetime::` runs the full release script with a focused
fake `gh`. Missing and pending runs at both CI and release-workflow waits trigger
a 500ms fixture deadline; an outer supervisor allows five seconds for the fixture
to fail with captured diagnostics. Separate cases cover successful completion,
assertion unwinding and SIGKILL of the fixture harness. The supervisor adopts and
reaps recorded script, guardian and descendant PIDs. A hung summary Harness is
also covered on timeout and SIGKILL. Fixture setup and its
supervisor handshake use the shared 120-second readiness bound.

Release-script tests run on Linux. These deadlines belong to tests;
production release polling still has no overall timeout and resumes on rerun.
