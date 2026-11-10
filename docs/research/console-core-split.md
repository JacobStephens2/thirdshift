# Console/core crate split: edit-rebuild experiment

This is the isolated experiment requested by #1889. The production layout and
[ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md) are unchanged.
Adopting a workspace remains a Day shift decision.

## Reproduction and scope

The Base commit is `9283d0b8379aa9cf5b645bfe95699813846cb3a0`. From a checkout
containing that commit and this prototype, on Linux with Python 3.12+, Git and
the pinned Rust toolchain/native build dependencies:

```sh
python3 prototypes/console-split/benchmark.py --samples 3 --jobs 4 --output /tmp/console-split-results
```

The new output directory receives host metadata, every command's log,
`progress.json` and, after complete success, `results.json`.
[The prototype README](../../prototypes/console-split/README.md) documents
independent split generation and validation. The script exports the pinned
commit, rather than measuring whatever is currently checked out.

The root package remains the `thirdshift` binary. `core/` is a library package,
`thirdshift-core`, whose `src/core.rs` module tree holds everything except
Console and CLI parsing/entry-point code. To minimize unrelated relocation,
sources stay under their original `src/` paths. Core's manifest points at
`../src/core.rs`; there is no dependency from core back to the binary.

Shared command values and retry rendering move out of `args.rs` into
`core_args.rs`, imported by both sides. `Scope` and Fleet pairing address
validation move into `console_contract.rs`. The original parser/child-run
roundtrip tests move to the binary, where both sides are available. Their
assertions and every integration-test file are preserved. The `test-support`
feature exposes shared unit-test helpers to the binary's dev dependency.
Core preserves the original package version and repository metadata, which
the updater, traces and Eval replay importer consume at compile time.

The mechanical pass widens 645 existing `pub(crate)` occurrences in 148 core
source files, as well as the moved command/Console contracts. It also exposes
`Given::to_args` and `start_from` for the relocated tests. Narrower internal
visibility stays narrow. The new library permits `clippy::new_without_default`:
publishing existing constructors triggers that lint, and adding new defaults
would go beyond a mechanical split. A production interface would need a much
more selective visibility pass.

## Host and method

Measured on 2026-11-09 on `stephens`, a DigitalOcean KVM x86_64 VM running
Ubuntu 24.04 (Linux 6.8.0-147), with four DO-Premium-AMD vCPUs and 16 GiB RAM.
Rust is `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, LLVM 23.1.1; Cargo is 1.99.1.
The linker probe records `cc` (Ubuntu GCC 13.3.0) with Rust's default
`-fuse-ld=lld`. No linker, dependency version or profile changes are part of
the experiment. These numbers are from a different host than the aarch64
[build profile](rust-build-profile.md); absolute times are not comparable.

There are three samples for each layout and metric. Commands run sequentially
with four build jobs and incremental compilation enabled. Inherited Cargo/Rust
flags, wrappers and profile overrides are cleared. Each layout/sample starts
with a fresh source export and empty target. Cold test compilation uses a
separate empty target, including dependencies, after the binary target is
removed. Validation uses its own target, removed before measurements.
Dependency fetching precedes timing; the OS page cache is retained. Layout
order alternates Base/split, split/Base, Base/split. This is a shared host,
without external-load control; three samples describe this run rather than
establishing a precise population speedup.

After a cold dev binary build, each edit appends a comment to one file:

| Location | File |
| --- | --- |
| Console handler | `src/console/commands.rs` |
| Ticket run | `src/run.rs` |
| logs | `src/logs.rs` |

The runner restores the original bytes after every edit and runs an unmeasured
warm command before each subsequent location. `cargo check` gets its own
unmeasured warm command too. Thus each measurement invalidates only its named
location; restoration and the first compilation after any configuration
difference are excluded. The measured commands are:

```sh
cargo build --locked --bin thirdshift
cargo test --locked --no-run
# Each of the three edited files, with its baseline warmed first:
cargo build --locked --bin thirdshift
cargo check --locked --bin thirdshift
```

## Measurements

All values are wall-clock seconds. Each list contains samples 1, 2 and 3;
changes compare the medians, calculated before rounding. Negative change means
the split took less time. The complete host record, commands, results and
unrounded per-sample data are in
[`results.json`](../../prototypes/console-split/results.json). Full logs are
produced in the runner's chosen output directory.

| Measurement | Base samples | Split samples | Base median | Split median | Change |
| --- | --- | --- | ---: | ---: | ---: |
| Cold dev binary build | 129.342, 124.428, 124.090 | 123.152, 127.736, 124.123 | 124.428 | 124.123 | −0.25% |
| Cold test compile | 211.850, 195.380, 206.142 | 205.255, 218.628, 205.917 | 206.142 | 205.917 | −0.11% |
| Console edit, build | 10.444, 9.899, 11.142 | 3.800, 4.058, 3.930 | 10.444 | 3.930 | −62.37% |
| Ticket run edit, build | 10.264, 10.159, 10.508 | 10.269, 10.176, 10.771 | 10.264 | 10.269 | +0.06% |
| logs edit, build | 10.359, 9.927, 10.384 | 10.424, 9.960, 10.306 | 10.359 | 10.306 | −0.51% |
| Console edit, check | 5.158, 5.481, 5.307 | 1.641, 1.510, 1.496 | 5.307 | 1.510 | −71.55% |
| Ticket run edit, check | 5.398, 5.190, 5.634 | 6.093, 5.462, 5.440 | 5.398 | 5.462 | +1.19% |
| logs edit, check | 5.102, 5.222, 5.410 | 5.732, 5.195, 5.466 | 5.222 | 5.466 | +4.67% |

**Console edits got faster:** the median rebuild saved 6.514 s (62.37%), and
the median check saved 3.797 s (71.55%). **Core rebuilds did not meaningfully
slow down:** Ticket run edits cost another 0.006 s (0.06%), while logs edits
saved 0.053 s (0.51%). **Core checks were slower in the observed medians:**
another 0.064 s (1.19%) for Ticket run and 0.244 s (4.67%) for logs. Cold
build/test-compile medians were effectively unchanged. These small core/cold
differences overlap the observed sample variation; this run establishes the
large Console benefit more clearly than a stable small core penalty.

The command logs show where the split saves work: a Console edit compiles only
`thirdshift`, reusing `thirdshift-core`; a Ticket run or logs edit compiles core
and then the binary. This removes the core compiler work from the Console edit
loop while retaining the final executable link. Core edits still invalidate
the library and its consumer, which is consistent with the small check cost.
The nearly unchanged cold times are consistent with keeping the same external
dependencies and nearly all the same compilation work. These are inferences
from Cargo's package rebuilds and wall times, not a measured allocation among
macro expansion, type checking, monomorphization and linking; no nightly
`-Ztime-passes` profile was collected.

These measurements concern the named Rust handler edit. They do not establish
the same benefit for every Console asset: core's `config/console/themes.rs`
still includes Console CSS for Theme validation, so a CSS edit can invalidate
core too. The prototype keeps those existing resource paths.

## Faithfulness

The runner validated the split before measuring, with a private target removed
after validation. All commands exited 0:

| Command | Wall seconds | Result |
| --- | ---: | --- |
| `cargo build --locked` | 128.986 | Pass |
| `cargo clippy --all-targets -- -D warnings` | 77.708 | Pass |
| `cargo test --test evals replay_import_completes_trial_inputs_and_preserves_local_edits -- --exact` | 68.399 | 1 passed |
| `cargo test --test packaging non_linux_compilation_reports_only_the_platform_error -- --exact` | 0.871 | 1 passed |
| `cargo test --no-fail-fast` | 2352.408 | 3927 passed, 0 failed, 91 ignored |

The full-suite time includes remaining compilation. It covers both packages,
all integration tests, Pickup's model checks and the library's doctest target
(zero doctests). Ignored tests retain their original annotations.
Counts use the 97 top-level suite summaries; three nested fixture re-execution
summaries are excluded rather than counted again.
`--no-fail-fast` changes failure reporting, not test selection.
It allowed a complete view of all test targets in one run.

Two initial exports failed existing tests: omitting the core package's
`repository` produced `/issues/16` instead of its full GitHub URL, and moving
parser/child-run tests without their inherited Linux guard produced extra
non-Linux diagnostics. The generator preserves both contexts now. The exact
regressions above pass, and the prototype's independent export check verifies
package metadata and unsupported-platform diagnostics for both crate roots.
The integration tests themselves were not edited.

## Cost of adoption and next seams

- **Visibility:** replace the broad prototype export with a deliberate library
  surface. Shared command values and Console scopes/address validation need
  stable owners; parser and core must remain acyclic. Decide whether the
  existing constructors should gain defaults or keep the narrow lint allowance.
- **Tests:** decide ownership of parser/child-run tests and a supported helper
  feature. Keep unit tests in both packages and preserve integration tests'
  `CARGO_BIN_EXE_thirdshift` behavior. Source-inventory tests, embedded fixtures
  and `CARGO_MANIFEST_DIR` paths must follow the chosen physical layout.
  Preserve compile-time package metadata and inherited platform guards too;
  both affected behavior in the initial exports.
- **CI and local selection:** `cargo test --bin thirdshift` would no longer run
  core unit tests. `scripts/affected-tests` and CI must cover both packages,
  including direct core changes, helpers, both lockfile entries and features.
  Workspace defaults here include both packages so bare `cargo test` includes
  the complete suite. This PR changes neither CI nor the production selector.
- **Release builds and packaging:** keep the executable named `thirdshift` and
  explicitly select its package in install/dist jobs as necessary. Measure
  release/LTO behavior separately; dev timings say nothing about release time
  or binary size. Embedded skills/processes/assets must remain self-contained.
  The prototype's manifest-relative paths and path-only dependency are unsuitable
  for crates.io packaging: the `cargo publish --locked` workflow would need
  proper library packaging, versioning and publication order. The version-bump
  script currently updates one package/lock entry and would need workspace
  support. Release adoption has not been attempted here.

The next candidate to try is the Evals, Scoring and Experiments cluster,
including its verdict calculation. Its external callers concentrate in CLI,
Console, GitHub's scoring adapter and logs. It has fewer incoming module
connections than the Ticket run/dispatch cluster, but logs stores Experiment
descriptions and reads confirmed Bug origins, so those shared values would
need explicit owners. This experiment does not split that cluster or choose
its final interface.
