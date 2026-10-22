# Local test selection

Run `scripts/affected-tests main` from anywhere in this checkout. It requires
Python 3 and Git, compares the working tree with the merge-base of the supplied
ref and `HEAD`, and includes staged, unstaged and untracked non-ignored files.
Renames include both paths. An invalid ref fails before running checks; an empty
diff prints that no tests are affected. Each selected command is printed, and
any failed command fails the entry point.

| Changed paths | Checks |
| --- | --- |
| `site/**`, `docs/guide.md` | `npm test --prefix site`; `cargo test --test share_card --test site_deploy`; `cargo test --bin thirdshift prompts_page` |
| `scripts/release*`, `scripts/generate-release.py`, `scripts/test_generate_release.py` | `cargo test --test release`; `python3 -m unittest discover -s scripts -p test_generate_release.py` |
| Everything else, including `src/**`, shared test support/fakes, Cargo files and CI configuration | Full Rust suite |
| Several areas together | Full Rust suite plus any selected browser and release-generator checks; the full suite subsumes the targeted Rust checks |

Unknown paths print the reason for selecting the full Rust suite. The mappings
are deliberately narrow: adding a new area cannot silently opt it out. For
changes to the selector or other Python tooling, also run
`python3 -m unittest discover -s scripts -p 'test_*.py'`, as CI does.

The full Rust suite uses `cargo nextest run` when `cargo nextest --version`
succeeds, otherwise it prints a fallback note and uses `cargo test`. Nextest is
optional; install it with `cargo install cargo-nextest --locked`. Both runners
cover the binary's unit tests and integration tests; this crate has no library
target or doctests. For a focused iteration, use
`cargo test --bin thirdshift <module>::tests` or `cargo test --test <file>`
(the file name without `.rs`).

Set up browser dependencies following [the site instructions](../site/tests/README.md)
before selecting site checks. A missing dependency fails visibly rather than
dropping that check. Site generation checks remain part of `npm test`; regenerate
prompt changes as described in [AGENTS.md](../AGENTS.md#generated-prompts).

## Local work and CI

During implementation, run typechecking and focused tests. Before additional
validation, check the branch's PR state: stop after it is merged or closed.
Reuse successful CI for the exact head instead of repeating equivalent full-suite
validation. Changes after that head need validation again. Push and open the PR
while work is still in progress when practical so CI can overlap local work;
this does not change Delivery or CI policy.

Local selection shortens feedback; CI retains its broader checks on every PR,
including Python tooling, formatting, clippy, site and Console browser tests,
and the Linux Rust suite. The macOS gate/full-suite policy remains in
[CI performance](ci-performance.md). This script selects Rust, site and release
checks; it does not replace CI's cross-platform and Console browser coverage.

Measure local full-suite time with `/usr/bin/time -p cargo nextest run` (or
`/usr/bin/time -p cargo test` for the fallback), recording the command, result,
wall time and whether compilation was included in the PR's evidence. Timings
depend on host load and build cache; compare them with CI execution time rather
than CI queue delay.

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

macOS retains Drop cleanup and terminal hangup when the last master closes. It
has no Linux parent-death signal or subreaper facility: the abrupt-death guarantee
with a retained master is Linux-only, and the lifecycle supervisor test is
explicitly skipped on macOS with that platform limit. Existing setup interruption
and terminal-restoration tests still run there.

## Release fixture child lifetime

Linux release-script and script-function tests use `FixtureChild` from the same
child-lifetime helper. Each command has a 120-second wait bound, with stdout and
stderr included in a timeout panic. File-backed input and output keep inherited
descriptors and large input from blocking that wait.

A guardian holds a private process group for each command. Normal completion,
timeout and assertion unwinding kill that group and reap the immediate command
and guardian. If SIGKILL ends the fixture harness, EOF on the guardian's private,
close-on-exec stdin pipe makes it kill its group, including itself. The command
also uses the setup helper's Linux parent-death signal. Cleanup names only the
owned group; git maintenance remains disabled in these isolated fixtures.

`cargo test --test release lifetime::` runs the full release script with a focused
fake `gh`. Missing and pending runs at both CI and release-workflow waits trigger
a 500ms fixture deadline; an outer supervisor allows five seconds for the fixture
to fail with captured diagnostics. Separate cases cover successful completion,
assertion unwinding and SIGKILL of the fixture harness. The supervisor adopts and
reaps recorded script, guardian and descendant PIDs. Fixture setup and its
supervisor handshake use the shared 120-second readiness bound.

Release-script tests remain Linux-only. These deadlines belong to tests;
production release polling still has no overall timeout and resumes on rerun.
