# Go prototype versus Rust: empirical comparison

Issue #1835, under #1830. ADR-0001 remains unchanged; choosing a production
language belongs to the Day shift after reviewing the evidence.

## Reproduce

The measured revision and host, tool versions, input hashes, exact commands,
per-sample wall times and counted files are in
[results.json](go-vs-rust-benchmark-2026-11-06/results.json).

```sh
python3 prototype/go/benchmark.py --ref 82cc4dc2 --samples 2 --jobs 2 --output /tmp/thirdshift-benchmark-results
```

See [setup and runner semantics](../../prototype/go/README.md#reproducible-comparison).
The timed workloads are pinned to that revision; later changes add the report
and its contract checks.
Use a new/empty output directory. A rerun on another host produces new numbers.
Measured commands run sequentially, with caps of two build jobs, Go processors
and test threads (Go tests without `t.Parallel` remain serial); ADR-0017 Rust
test cases run one at a time. No external network or LLM calls
occur during test execution. Cargo downloads are prepared outside the timings.

## Scope and method

The Go executable implements direct Ticket runs, streamed Claude sessions,
Claim ending, Delivery, bounded Repairs and Pickup reservation/dispatch. The
corresponding Rust code is compiled as the production executable: it also
contains the Console, experiments, scoring, multiple Harnesses, Specs, security
and other features. Rust has no independently buildable library for just this
slice. Its build time and binary size therefore include that larger product.
This measures the actual development cost of changing those Rust owners; it
does not measure equal-feature implementations or establish a language-only
speedup. Go's own README lists its unsupported behavior.

| Behavior | Go owners | Rust owners / selected tests |
| --- | --- | --- |
| Ticket run and Claim ending | `run.go`, `claim.go`, `salvage.go` | `src/run.rs`, `src/claim.rs`; `fresh_run` |
| Delivery, Repair and CI | `delivery.go`, `ci.go`, `pull_request.go` | `src/delivery/`, `src/ci.rs`, `src/pull_request.rs`; `fresh_run`, `ci` |
| Harness and child lifetime | `session.go`, `process.go` | `src/session.rs`, `src/process/`, `src/harness/`; `harness` |
| Pickup and reservation | `pickup.go`, `ready.go`, `reservation.go` | `src/pickup.rs`, `src/ready.rs`, `src/reservation.rs`; `pickup_run`, `pickup_model` |

Each sample starts with a fresh detached Worktree and empty `GOCACHE`,
`GOMODCACHE` and `CARGO_TARGET_DIR`. Rust registry downloads and toolchains are
reused; OS filesystem caches are not flushed. Go's cold build compiles its
standard library cache; Rust reuses the standard library supplied by rustup.
Cold development builds use `go build -trimpath` with `CGO_ENABLED=0` and
`cargo build --locked --bin thirdshift`. An incremental build appends a comment
to each entry point, rebuilds with the same cache, then restores the source.
This is an edited package rebuild, rather than a no-change cache lookup.

`test_compile` follows those builds: its test artifacts have not been built,
but it is not the first command against an empty cache. Go uses `go test -c`
with default path handling, which differs from the development build's
`-trimpath` and can require separate cache entries. Rust compiles only
`fresh_run`, `pickup_run`, `harness`, `ci`
and `pickup_model`. `test_execution` invokes those compiled binaries directly;
Go's `TestMain` still performs its normal warm `go build`. The Go benchmark
contract is excluded to prevent recursion. Rust tests exercise the equivalent
Ticket/Delivery/Pickup/Harness/CI seams but are more numerous and cover a
larger product. These are real existing suites, not matched microbenchmarks.

`static_build` separately builds stripped executables: Go `-ldflags='-s -w'`,
Rust `--release --target x86_64-unknown-linux-musl` with
`CARGO_PROFILE_RELEASE_STRIP=symbols`. It uses the sample's caches, but Rust's
release/musl artifacts have not been built yet. `readelf -lW` and `-dW` must
show neither `INTERP` nor `DT_NEEDED`; a dynamic result fails the runner.
The Rust release profile differs from the distribution profile's thin LTO.
This is a static executable size comparison, not release packaging size.

## Measurements

Host: DO-Premium-AMD; 4 visible CPUs; go version go1.24.5 linux/amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13). Native GNU/Linux development builds; x86_64 musl Rust static builds.

Wall-clock seconds (each column is one fresh Worktree):

| Metric | Go 1 | Go 2 | Rust 1 | Rust 2 |
| --- | ---: | ---: | ---: | ---: |
| cold_build | 10.197 | 9.659 | 166.977 | 160.736 |
| incremental_build | 0.631 | 0.601 | 9.292 | 8.887 |
| test_compile | 17.080 | 15.552 | 81.400 | 80.010 |
| test_execution | 154.084 | 152.826 | 262.259 | 298.134 |
| model_check | 5.299 | 5.214 | 1.123 | 1.119 |
| static_build | 0.249 | 0.269 | 357.999 | 387.149 |

| Inventory | Go | Rust |
| --- | ---: | ---: |
| Physical source lines in manifest | 1615 | 29948 |
| Stripped static bytes, sample 1 | 3289272 | 21210280 |
| Stripped static bytes, sample 2 | 3289272 | 21210280 |

For these artifacts on this host, the Go prototype builds substantially
faster and yields a smaller executable. Its existing tests still take about
2.5 minutes, including real Delivery grace periods. Rust's broader selected
suite takes 4.4–5.0 minutes. The build difference is useful evidence of current
development cost, while the unequal feature and test scope prevents estimating
the benefit of a complete Go rewrite from these numbers.

Two samples on a shared host describe observed cost, without confidence
intervals or a claim that their ratio is a language benchmark. Full logs from
the run are retained at `/tmp/thirdshift-1835-benchmark.PgYbxF/`; the JSON
retains command/log names even after disposable Worktrees have been removed.

Source counts are physical lines, including blank lines and comments. The
JSON manifest lists every selected file: Go's production `*.go` files; Rust's
entry point and the `run`, `delivery`, `pickup`, `ready`, `reservation`, `claim`,
`process`, `session`, `harness`, `ci`, `pull_request` and `interrupt` owners.
Separate test/support files are excluded; Rust's inline tests remain counted.
Dependencies, embedded Factory-skill data, Go's benchmark tooling and Rust
owners outside this manifest are excluded. The Rust manifest includes more
Harnesses and lifecycle features, and omits transitive shared helpers: its
line count is an explicitly scoped inventory, not a normalized complexity
measure or a count of all code linked into the executable.

## Subprocess and signal ergonomics

Go's [`process.go`](../../prototype/go/process.go) uses `os/exec`, one goroutine
waiting on `Cmd.Wait`, and a `select` over completion and context cancellation.
It assigns a process group, sends SIGINT then SIGTERM with two-second grace
periods, then SIGKILL, and waits for the child. The deferred group kill handles
members left after the leader exits. [`session.go`](../../prototype/go/session.go)
streams JSON through a pipe while supervision runs concurrently. Claim ending
and salvage receive independent contexts after interruption. This is compact,
but descendant processes that escape the group are outside that mechanism.
`WaitDelay` addresses pipe/child waiting in `os/exec`; it does not implement
the escalation policy. See the [Go Cmd contract](https://pkg.go.dev/os/exec#Cmd).

Rust's [`process.rs`](../../src/process.rs) owns the child and I/O worker threads,
uses completion channels and RAII cleanup, and propagates errors from partial
setup. [`process/tree.rs`](../../src/process/tree.rs) snapshots descendants and
groups, with ten-second signal grace periods;
[`process/session.rs`](../../src/process/session.rs) also sweeps work by session
marker or scope after a leader exits. Those guarantees account for some of
the extra implementation. The production CLI's child supervision uses Rust
threads; `tokio` supplies the Console server runtime and blocking-task bridge
([`console.rs`](../../src/console.rs)), rather than replacing that supervisor.
Goroutines make the simple wait/cancel flow shorter; they do not remove the
need to implement process ownership, escaped-child cleanup and cancellation
semantics. The comparison is an inspection of these implementations, not an
ergonomics score inferred from timing.

## Exhaustiveness guarantees

Go can model closed alternatives with a package-private marker interface and
distinct variant structs, or with a named integer type and constants. Neither
pattern makes the compiler require every type/constant in a switch; interface
closure also depends on the package discipline. The prototype uses the latter:
four named state types in [`states.go`](../../prototype/go/states.go), plus
[`exhaustiveness_test.go`](../../prototype/go/exhaustiveness_test.go)'s typed-AST
check. It rejects missing constants even with a `default`, and includes negative
fixtures for a newly added variant. This gives a useful test-time gate for
this package's expression switches. It is not general sum-type checking:
runtime integer values can be invalid, payloads are not variant-specific, and
other dispatch forms or packages would require checker coverage. The
[Go specification](https://go.dev/ref/spec#Switch_statements) defines switch
behavior without an exhaustiveness requirement.

Rust's payload-bearing `enum` and `match` make missing cases a compile error
([Rust book](https://doc.rust-lang.org/book/ch06-02-match.html)). A wildcard
can absorb future variants, so [`Cargo.toml`](../../Cargo.toml) enables
[`clippy::wildcard_enum_match_arm`](https://clippy.rust-lang.org/master/index.html#wildcard_enum_match_arm)
and CI runs clippy with `-D warnings`;
documented exceptions use `#[expect]`. Rust therefore offers compiler and
lint guarantees without maintaining a project-specific syntax checker.
The prototype demonstrates mitigation, rather than parity of guarantees.

## Model checking ADR-0017

Go's [`pickup_model_test.go`](../../prototype/go/pickup_model_test.go) implements
a deterministic BFS with comparable states, a visited set and predecessor/action
links. Its tests check the guarded protocol and require shortest counterexamples
for `WithoutClaimCount` (19 actions) and `UnconditionalCreate` (16). The
[request mapping and bounds](../../prototype/go/PICKUP-MODEL.md) are part of
the evidence. Maintaining the exploration engine, witness replay and sensitivity
tests is project work; no third-party model checker is used.

Rust's [`tests/pickup_model.rs`](../../tests/pickup_model.rs) uses
`stateright = 0.31.0` with `spawn_bfs().join()`, property assertions and
discovery paths ([stateright Checker](https://docs.rs/stateright/0.31.0/stateright/trait.Checker.html)).
Both models bound competition to three hosts and test reservation leases and
the Claim-count generation check. Their state graphs and dispatch markers
differ: Go claims before Harness dispatch; Rust's model retains the known
pre-Claim literal dispatch counterexample and separately checks claimed-dispatch
safety. Rust's ignored negative assertion stays ignored in these timings;
the passing regression requires the known counterexample to remain visible.
Go's guarded protocol checks both dispatch predicates.

Retained model output: [Go sample 1](go-vs-rust-benchmark-2026-11-06/1-go-model_check.log),
[Go sample 2](go-vs-rust-benchmark-2026-11-06/2-go-model_check.log),
[Rust sample 1](go-vs-rust-benchmark-2026-11-06/1-rust-model_check.log),
[Rust sample 2](go-vs-rust-benchmark-2026-11-06/2-rust-model_check.log).

In the first sample, Go's guarded search visited 1,993,250 states and its
mutations visited 119,601 / 223,036 states. Rust's claimed-dispatch safety
search visited 86,745 states, its known literal-dispatch counterexample search
7,093, and its mutations 20,675 / 22,520. These are different exploration
workloads; state totals cannot be treated as matched checker throughput.

`model_check` records the compiled model-test command's complete wall time,
including Go `TestMain`'s warm build, test startup, safety exploration,
mutation/counterexample exploration and output. Rust runs four ordinary tests;
Go runs three (one has two mutation subtests). It is not pure checker-kernel
time, and different explored states preclude attributing a timing ratio to
BFS implementation quality. Both remain bounded evidence, not proof for
unbounded hosts or arbitrary GitHub consistency. Neither model is changed by
this work.

## Decision boundary: ADR-0001

This report supplies reproducible build/test/size observations and concrete
tradeoffs for the Day shift. A full Go migration still needs product parity,
an agreed replacement for Rust's exhaustiveness gate, and a maintainable
verification strategy. The two small-sample suites and inventories cannot
price that rewrite. [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md)
is deliberately left unchanged.
