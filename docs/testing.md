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
