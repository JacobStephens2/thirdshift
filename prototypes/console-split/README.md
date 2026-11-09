# Console/core crate split experiment

This prototype applies only to Base commit
`9283d0b8379aa9cf5b645bfe95699813846cb3a0` (the `main` commit used for #1889).
Production sources, Cargo configuration, CI and ADR-0001 are unchanged.

On Linux with Git, Python 3.12+, the repository's pinned Rust toolchain and
native build dependencies installed, run from this checkout:

```sh
python3 prototypes/console-split/benchmark.py --samples 3 --jobs 4 --output /tmp/console-split-results
```

The output directory must be new or empty. The runner first validates the split
with `cargo build --locked`, `cargo clippy --all-targets -- -D warnings` and
`cargo test --no-fail-fast` (first checking the Eval replay import and platform
diagnostic regressions), then measures
both layouts sequentially. It records the host,
CPU count, compiler, actual default linker command, environment, commands,
wall times and per-command logs. `results.json` is written only after every
command passes; `progress.json` and logs remain if a command fails. Temporary
source trees and target directories are removed even on failure. No GitHub
or Harness credentials are required. Allow several GB of free temporary disk
and enough time for twelve cold dependency/test compilations.

Each of three samples uses fresh targets for the binary's cold dev build and,
separately, cold `cargo test --no-run`. After the binary build, edits append a
comment to `src/console/commands.rs`, `src/run.rs` and `src/logs.rs`; both
`cargo build --locked --bin thirdshift` and `cargo check --locked --bin
thirdshift` are measured. The original bytes are restored after each edit,
and an unmeasured warm command establishes that baseline before the next
location. No flags change between warm and measured commands. Targets are
never shared between layouts or samples; dependency downloads happen before
timing. The OS page cache is retained. Layout order alternates across samples.
The runner clears inherited Cargo/Rust build flags, wrappers and profile
overrides, uses incremental compilation, and records its build job count.

To inspect or validate the split independently:

```sh
python3 prototypes/console-split/prepare.py /tmp/thirdshift-split
cd /tmp/thirdshift-split
cargo build --locked
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
```

`prepare.py` exports committed sources with `git archive`, then applies uniquely
anchored transformations to that pinned version. It refuses a nonempty
destination. The output is a two-package workspace: the root `thirdshift`
binary and `core/`'s `thirdshift-core` library. Sources retain their original
relative paths under `src/` to avoid unrelated include/template relocation.
`src/core.rs` declares core's modules; `src/main.rs` imports those modules and
retains Console and argument parsing. Shared command values/retry formatting
are extracted into `src/core_args.rs`, and `Scope`/Fleet address validation into
`src/console_contract.rs`. The core manifest has no dependency on the binary.
Only existing crate visibility is mechanically widened, plus two helpers
used by the relocated parser/child-run unit tests. Core permits
`clippy::new_without_default` because its existing constructors become public;
the prototype adds no new constructor behavior. The binary's dev dependency
enables core's `test-support` feature. Integration tests remain byte for byte
unchanged. Core preserves the package version and repository metadata used by
runtime behavior. Dependency versions, linker and build profiles are unchanged.

The export contract and the report's top-level suite count have independent checks:

```sh
python3 -m unittest discover -s prototypes/console-split -p 'test_*.py'
```

The measurements and adoption costs are in
[`docs/research/console-core-split.md`](../../docs/research/console-core-split.md).
