# Where the Rust build's time goes

The Go ([#1835](go-vs-rust-prototype.md)) and TypeScript
([#1840](typescript-vs-rust.md)) comparisons found much faster feedback loops
than the Rust binary's. They timed the whole production binary, because Rust has
no separately buildable library for the prototypes' slice. This note profiles
that binary's build to see where the time goes, and whether a split library or
build settings would narrow the gap. It is evidence for the Day shift.
[ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md) is unchanged.

## Method

Measured on 2026-11-09 at `6c26e8d9` (Release 0.24.0) in the Lima VM on the
Mac (Ubuntu 24.04 aarch64, 4 vCPUs, 8 GiB), with rustc 1.99.1 from
`rust-toolchain.toml`. Dependencies were fetched first; the target directory
started empty. These are single observations on a different host from the two
comparisons (both on DigitalOcean x86_64), so compare proportions, not the
absolute seconds.

```sh
cargo fetch --locked
CARGO_TARGET_DIR=/tmp/tt cargo build --locked --bin thirdshift --timings
# then, twice each: append a comment to src/run.rs and rebuild
cargo build --locked --bin thirdshift
cargo check --locked --bin thirdshift
```

Per-unit start and duration came from `UNIT_DATA` in Cargo's timing report.
The source edit was reverted afterwards.

## Results

| Step | Seconds |
| --- | ---: |
| Cold `dev` build, all 251 units | 65.6 |
| `aws-lc-sys` build script run (C compile) | 3.0 → 32.6 |
| `thirdshift` crate itself | 36.4 → 65.6 (29.1) |
| Rebuild after editing `src/run.rs` | 7.6, 8.4 |
| `cargo check` after the same edit | 2.0 |

No other unit took more than 4.5 s (`hyper`).

**Cold builds wait on one C library.** `rustls` uses `aws-lc-rs` as its
crypto provider, whose `aws-lc-sys` build script compiles AWS-LC from C for
about 30 s. `rustls` → `reqwest` → `thirdshift` cannot start until it ends,
so the cold build is roughly that build script plus the `thirdshift` crate,
end to end. Rust dependencies compile in parallel inside that window.
Switching reqwest to `ring` alone does not remove aws-lc:
[#203](https://github.com/JacobStephens2/thirdshift/issues/203) found that
`axoupdater`, through axoasset's `remote` feature, enables reqwest's
`default-tls`. Cargo features unify, so aws-lc still builds alongside ring,
as the [comment above `axoupdater` in `Cargo.toml`](../../Cargo.toml#L18)
explains. Removing that stack requires replacing the updater
([#1592](https://github.com/JacobStephens2/thirdshift/issues/1592)), not just
changing our reqwest features.

**A split library would save little on cold builds.** The `thirdshift` crate's
29 s barely overlaps dependency compilation, so splitting it into a workspace
could at most parallelise part of that 29 s. The slice the prototypes
implemented is also not self-contained: `run.rs`, `pickup.rs`, `delivery/`,
`claim.rs` and `session.rs` together import about 40 other crate modules,
including `spec_run`, `security`, `sandbox` and `notification`. A slice-only
library would need production decoupling, not just a new `Cargo.toml`.

**The edit loop is mostly codegen and linking.** After the edit, type checking
takes 2 s and a full rebuild 8 s, so about 6 s goes to code generation and
linking. The Go prototype's equivalent edit rebuild took about 0.6 s. The
experiments below measure a faster linker and the Cranelift backend for `dev`
builds. A workspace split that recompiles less after an edit remains unmeasured.
The repository sets only `debug = "line-tables-only"` in `[profile.dev]` and
has no `.cargo/config.toml`.

## Experiments

These measurements come from
[#1868](https://github.com/JacobStephens2/thirdshift/issues/1868), taken on
2026-11-09 in a scratch Worktree at `82f88ec8`, in the Lima VM (Ubuntu 24.04
aarch64, 4 vCPUs, 8 GiB, rustc 1.99.1 except for the nightly variants).
Each time is wall-clock seconds around `cargo build -q --bin thirdshift`.
Each edit appended a trailing comment to the Ticket run module (`src/run.rs`).
Linker flags were set through `RUSTFLAGS`; the first build after changing
`RUSTFLAGS` rebuilds every crate and is excluded from the edit column. Blank
cold-build cells mean no measurement was recorded. Nothing from the experiment
trees was committed.

| Variant | Cold build (s) | Edit rebuild (s) |
| --- | ---: | ---: |
| Baseline (system `cc`, GNU ld) | 58.7 | 7.0, 7.0, 7.0 |
| `-C link-arg=-fuse-ld=lld` | | 4.0, 4.2 |
| `-C link-arg=-fuse-ld=mold` | | 3.8, 4.5 |
| No `axoupdater`, reqwest on `ring` | 42.4 | 6.8, 6.0, 6.5 |
| Nightly 1.102.0 (2026-11-08), LLVM backend | 62.2 | 7.1, 6.8, 6.7 |
| Nightly, LLVM + lld | | 4.0, 4.4 |
| Nightly, Cranelift backend | 57.0 | 6.2, 6.1, 6.2 |
| Nightly, Cranelift + lld | | 4.0, 3.9 |

The "No `axoupdater`" variant measures #1592's end state for the build only.
It removed the dependency, stubbed the update module's version check and update
functions to return errors, and switched reqwest to `rustls-no-provider` plus
rustls 0.23 with `ring` and `std`. `cargo tree -i aws-lc-sys` then found
nothing. This measures build weight, not a working updater.

**Removing the updater's aws-lc saves about 16 s (28%) of a cold build, and
nothing on an edit rebuild.** The saving is less than the roughly 30 s build
script because the remaining dependencies become the critical path. Every new
Run Worktree and CI job pays a cold build.

**On this aarch64 host, lld or mold cuts the edit rebuild from about 7 s to
about 4 s.** The two are equivalent within these samples. This saving does not
carry over to x86_64 Linux hosts: since
[Rust 1.90](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/),
`x86_64-unknown-linux-gnu` uses the bundled `rust-lld` by default, so the
DigitalOcean measurements in #1835 and #1840 already used LLD. Using a faster
linker on aarch64 Linux would require a Cargo config entry for that target and
lld or mold installed on each such host.

**Cranelift adds little once LLD is used.** Edit rebuilds still take about 4 s,
and Cranelift needs nightly, which the pinned
[`rust-toolchain.toml`](../../rust-toolchain.toml) rules out.

**What remains of the edit loop is about 2 s of type checking and 2 s of code
generation for the single `thirdshift` crate.** Only a smaller recompilation
unit, such as a workspace split, targets that. A split has not been measured.

## Limits

One to three samples per measurement, on a 4-vCPU aarch64 VM. Each edit was a
single trailing comment in one file, which still forces the crate to recompile
and relink, but says nothing about edits that change many dependants. No
workspace split has been measured.
