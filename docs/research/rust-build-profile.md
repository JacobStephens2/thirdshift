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
Switching the TLS crypto provider (for example to `ring`) is the obvious
candidate, but it changes the TLS stack and needs its own review.

**A split library would save little on cold builds.** The `thirdshift` crate's
29 s barely overlaps dependency compilation, so splitting it into a workspace
could at most parallelise part of that 29 s. The slice the prototypes
implemented is also not self-contained: `run.rs`, `pickup.rs`, `delivery/`,
`claim.rs` and `session.rs` together import about 40 other crate modules,
including `spec_run`, `security`, `sandbox` and `notification`. A slice-only
library would need production decoupling, not just a new `Cargo.toml`.

**The edit loop is mostly codegen and linking.** After the edit, type checking
takes 2 s and a full rebuild 8 s, so about 6 s goes to code generation and
linking. The Go prototype's equivalent edit rebuild took about 0.6 s. Untried
options aimed at those 6 s: a faster linker (neither mold nor lld is installed
in the VM), the Cranelift backend for `dev` builds, and a workspace split that
recompiles less after an edit. The repository currently sets only
`debug = "line-tables-only"` in `[profile.dev]` and has no `.cargo/config.toml`.

## Limits

One sample per measurement, on a 4-vCPU VM. The edit was a trailing comment in
one file, which still forces the crate to recompile and relink, but says
nothing about edits that change many dependants. None of the candidate
improvements above has been measured.
