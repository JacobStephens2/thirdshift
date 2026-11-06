# TypeScript versus Rust: bounded vertical-slice experiment (#1840)

This is evidence for the Day shift's review of Spec #1831, not a migration
decision. **ADR-0001 is unchanged.** The TypeScript prototype demonstrates a
self-contained Ticket/Pickup/Console executable. Rust remains the production
implementation and covers considerably more behavior.

## Reproduction

The companion [raw JSON](typescript-vs-rust.json) records the exact measured
commit, tool versions, host, commands, outputs, exit codes, wall times, binary
bytes and per-file physical line counts. Run from the repository root:

```sh
export PATH="$HOME/.bun/bin:$PATH"
bun install --frozen-lockfile --cwd prototypes/typescript
bun prototypes/typescript/node_modules/playwright/cli.js install chromium
bun prototypes/typescript/benchmark.ts --ref 8700be5d --output /tmp/typescript-benchmark.json
```

The runner creates a detached Worktree at the selected commit and removes it
and its build outputs on success or command failure. It measures committed
files only. Measurement logs survive cleanup in `<output>.logs/`, including on
measurement failure. Bun install and Cargo fetch use locked dependency versions and run
outside the timing. Chromium and its Linux system dependencies must already be
installed. This is a Linux x86_64 experiment: the Bun build explicitly targets
`bun-linux-x64`; Rust builds for the host.
The Rust size is the native `release` artifact, not the distributed musl `dist`
artifact with thin LTO described in [ADR-0003](../adr/0003-static-binaries-through-github-releases.md).

“Cold” means empty build outputs, not a cold OS page cache or empty download
cache. Typecheck, build and test each start with independent empty output
directories. “Incremental” means the immediate **no-edit repeat**, not a rebuild
after a source change. TypeScript runs `tsc --noEmit --incremental` with a fresh
build-info file, then retains it; this differs from the normal prototype
`typecheck` script, which does not enable incremental metadata. Bun recompiles
the standalone executable and reruns tests; Cargo can reuse compiled artifacts.
The first model timing is already compiled, as is the second.

Rust uses `cargo check --bin thirdshift`, `cargo build --release --bin thirdshift`
and a sequential test bundle: `fresh_run`, `pickup_run`, `pickup_model`, then
the exact Console status/CLI parity and OpenAPI tests plus the `dismiss` filter.
Each Cargo command uses `--locked`. Separate target directories prevent check artifacts from
warming the test measurement. Builds use four jobs, tests use three threads,
and exact Console/model-only tests use one thread. Compiler wrappers and Rust flags are cleared.
TypeScript runs `bun run build.ts` (which invokes `bun build --compile`), and
the Ticket, Ready, Console and Pickup model test files. Tests compile their own
binary, so cold test timings include that cost. The runner's own fixture and
report tests are deliberately excluded to avoid recursion and unrelated work.
Rust Console coverage is HTTP/CLI; TypeScript also launches Playwright. Raw JSON
contains every command and argument rather than relying on these abbreviations.

## Measured results

Recorded 2026-11-06 on Linux 5.14.0 x86_64, an 8-vCPU DO-Regular VM with
16496074752 bytes of RAM. Tools: Bun 1.4.4, TypeScript 5.9.3, Rust 1.99.1,
Cargo 1.99.1 and Git 2.52.0. Every measured command exited zero.

| Operation (seconds) | TypeScript cold | TypeScript repeat | Rust cold | Rust repeat |
| --- | ---: | ---: | ---: | ---: |
| Typecheck | 4.921 | 2.207 | 110.730 | 0.531 |
| Standalone build | 0.734 | 0.800 | 351.612 | 0.588 |
| Slice test bundle | 103.711 | 100.531 | 448.080 | 236.132 |
| Model only, already compiled (first / repeat) | 0.723 | 0.701 | 2.235 | 2.305 |

| Executable | Bytes | MiB |
| --- | ---: | ---: |
| Bun Linux x64 standalone | 109766832 | 104.68 |
| Rust native host release | 27849936 | 26.56 |

Cold TypeScript compilation/typechecking took less wall time for this slice;
the cached Rust check/build repeats took less time. Bun's embedded runtime
made its executable larger. The test differences combine compilation and
different workloads, so these figures do not isolate a language speed factor.

The TypeScript bundle passed 54 tests; the Rust bundle passed 183, with three
ignored tests (fixture entrypoints and the known literal dispatch assertion).
These counts describe the selected workloads, not equivalent coverage.

## Source size

The runner counts raw physical lines, including blank lines and comments, in
production `.ts`/`.html` files and Rust `.rs`/`.html`/`.js`/`.css` files. Rust owner
files also embed unit tests. Separate owner-file inventories cover Ticket,
Pickup and Console; the JSON lists every counted file. These are auditable
ownership slices, **not dependency closures or feature-equivalent programs**.
In particular, Rust Console includes many pages absent from the prototype;
Rust Ticket support includes more Harnesses and recovery behavior. Shared
`skills/` text is embedded by both builds and excluded from these code counts.
Build scripts and external integration tests are excluded; model-test sources
are counted separately.

| Physical line inventory | TypeScript | Rust |
| --- | ---: | ---: |
| All production owner files/assets | 1146 | 155462 |
| Ticket owner-file slice | 659 | 45054 |
| Pickup owner-file slice | 201 | 2754 |
| Console owner-file slice | 286 | 18017 |
| Pickup model test | 158 | 426 |

The much smaller prototype covers much less behavior. These counts help locate
maintenance surfaces; they are not estimates of rewrite cost or code quality.

## Subprocess and signal ergonomics

The prototype's [Commands](../../prototypes/typescript/src/process.ts) uses
`node:child_process.spawn`, asynchronous pipe events and `StringDecoder` for
streaming JSON lines. `detached: true` creates a POSIX session/process group;
negative-PID signals reach the group. It records SIGINT/SIGTERM/SIGHUP and
escalates SIGINT → SIGTERM → SIGKILL at one-second intervals, even after the
leader exits. Claim ending and salvage can run after interruption. The compiled
tests exercise a signal-resistant forked Harness with no Bun executable on PATH.
These are concrete working ergonomics, rather than an assumption that killing
a parent kills its descendants. [Node detached processes](https://nodejs.org/api/child_process.html#optionsdetached),
[Node kill semantics](https://nodejs.org/api/child_process.html#subprocesskillsignal).

Rust's [process owner](../../src/process.rs) uses
`CommandExt::process_group(0)`, explicit pipe worker threads and RAII cleanup;
[interrupt.rs](../../src/interrupt.rs) records the same interruption signals
through `signal-hook`. It additionally tracks escaped groups and session-marked
processes in [tree.rs](../../src/process/tree.rs) and
[session.rs](../../src/process/session.rs). The shorter TypeScript module has a
narrower cleanup contract. There is no evidence here that TypeScript cannot
implement Rust's stronger contract. Bun also exposes `spawn`, `AbortSignal` and
timeouts, but adopting them would still require descendant ownership policy.
[Rust process groups](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.process_group),
[Bun subprocesses](https://bun.sh/docs/runtime/child-process#using-timeout-and-killsignal).

## Exhaustiveness

TypeScript's [Delivery](../../prototypes/typescript/src/delivery.ts) defines
tagged unions for publish/sync/watch/repair/ready, Repair causes and CI states.
Its switch defaults use `state satisfies never`, `state.cause satisfies never`
and `ci satisfies never`. An unhandled variant reaching those switches makes
the explicit checks fail `tsc --noEmit`; the model uses an `assertNever` function for phases.
This is explicit discipline: TypeScript does not require every switch to be
exhaustive. External JSON also needs runtime validation; a static union alone
cannot validate a GitHub response. [TypeScript narrowing](https://www.typescriptlang.org/docs/handbook/2/narrowing.html#exhaustiveness-checking),
[`satisfies`](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-9.html#the-satisfies-operator).

Rust [Delivery stages](../../src/delivery/stage.rs) use an `enum` and `match`;
[CI states](../../src/ci.rs) also carry data, such as `Failed(FailedChecks)`.
Rust requires exhaustive matching, but `_` can silently absorb new
variants. [Cargo.toml](../../Cargo.toml) enables `clippy::wildcard_enum_match_arm`
at `warn` to discourage that escape hatch. A maintained discriminated union
can meet this experiment's exhaustiveness requirement; [CI](../../.github/workflows/ci.yml)
runs Clippy with `-D warnings`, so the Rust warning blocks CI. Neither mechanism proves
that the chosen transitions are correct. [Rust matches](https://doc.rust-lang.org/book/ch06-02-match.html#matches-are-exhaustive),
[Clippy lint](https://clippy.rust-lang.org/master/index.html#wildcard_enum_match_arm).

## Console velocity and type sharing

The [shared Zod schemas](../../prototypes/typescript/src/console/schema.ts)
derive `InboxStatus`, `DismissRequest` and `DismissResponse` through `z.infer`.
The CLI, server, browser and Playwright tests import these types and runtime
parsers. [routes.ts](../../prototypes/typescript/src/console/routes.ts) generates
OpenAPI 3.1 components through `z.toJSONSchema` and records page/CLI/scope parity
in one table. A field change therefore has one contract definition and compiler
feedback across those callers, plus runtime parsing at JSON boundaries.
The existing Console test verifies route parity, authorization and persisted
Dismissal/Undo at desktop and mobile sizes. [Zod inferred types](https://zod.dev/basics#inferring-types),
[Zod JSON Schema](https://zod.dev/json-schema).

The production stack has Rust CLI/HTTP/Askama and JavaScript browser assets,
with Node/Playwright as test tooling, not a server dependency. Rust already
generates OpenAPI operations from its [route table](../../src/console/openapi.rs).
The prototype removes duplicate static contract definitions for its small
slice, but this experiment did not time a matched UI change or quantify engineer
velocity. Rust could generate TypeScript declarations with
[ts-rs](https://docs.rs/ts-rs/latest/ts_rs/) and JSON Schema with
[Schemars](https://docs.rs/schemars/latest/schemars/); that would require a
generation/checking workflow rather than direct shared imports. The prototype
has one token, one writer and a separate Inbox file, without production account,
GitHub settlement, retention or collapsed-row semantics. Faster compilation
and shared contracts are promising feedback mechanisms, not a measured delivery
speed multiplier.

## Model checking: ADR-0017

The TypeScript suite runs its own visited-state/predecessor BFS inside `bun test`.
It exhausts Current, retains shortest mutation counterexamples, and checks
reachability of crashes, stale reservation recovery, Claim release and Day shift
hand-back for three one-attempt hosts. Rust uses pinned `stateright` 0.31.0's
`spawn_bfs().join()` inside `cargo test`. Its four non-ignored tests include
Current claimed-dispatch safety, the known pre-Claim dispatch counterexample,
and two mutation counterexamples. See [the model boundary](../../prototypes/typescript/tests/pickup-model.md)
and [Rust model](../../tests/pickup_model.rs), plus
[Stateright BFS](https://docs.rs/stateright/0.31.0/stateright/struct.CheckerBuilder.html#method.spawn_bfs).

The timing rows include runner startup and all model assertions and mutations,
with Rust compilation excluded. They compare the actual suites, not equal state
graphs: TypeScript applies Claim before dispatch and checks both dispatch
properties; Rust deliberately records a counterexample to literal dispatch
safety before Claim while checking claimed-dispatch safety. Symmetry reduction,
state encodings and repeated exploration differ. Raw output retains visited
state counts and counterexamples. A ratio of these times would not identify
language speed or establish equal coverage. Both models are bounded and separate
from product code, so both still require drift review.

For scale, the TypeScript Current search visited 5832 unique states. Rust's
Current searches visited 7093 for literal Dispatch and 86745 for ClaimedDispatch;
its two mutations visited 20675 and 22520. These are different graphs and stopping
conditions, not equal work performed at different speeds.

## Limitations and Day shift decision

This is one paired observation per operation on a shared Linux host. Order is
fixed, no variance/confidence interval was measured, and cache effects and host
load can dominate short commands. No-edit repeats say nothing about the cost
of a representative source edit. Rust's full binary and broader suites prevent
an apples-to-apples language performance claim. Release binary bytes are disk
size, not resident memory or startup latency; the Bun binary contains its runtime
and does not need Bun installed separately. Rust's existing static release
targets and Bun's Linux x64 artifact are not a portability equivalence test.
[Bun standalone executables](https://bun.sh/docs/compiler/executables).

An exploratory run of all 237 Rust Console tests failed at the existing
[read-byte caching assertion](../../tests/console/idle.rs): 235 passed, one
failed and one was ignored. The isolated command
`cargo test --locked --test console idle::unchanged_requests_reuse_retained_records_but_replacements_are_visible -- --exact --nocapture`
also failed (`unchanged request reread 156041 bytes`, limit 48 KiB). Production
code, Rust tests, Cargo files and embedded skills have no diff from the comparison
base. The final benchmark selects the status/OpenAPI/Dismissal counterpart,
not idle-cache behavior; its green results do not imply that the whole Rust
Console suite passes on this host.

The prototype establishes a feasible packaging and shared-schema path and gives
repeatable feedback-cost observations. It omits Continuation, Spec runs,
Self-merge, inherited Base fixes, abandoned-job recovery, foreign-commit
reconciliation and substantial Console behavior. None of those omissions is a
language limitation. ADR-0001 remains the current decision until the Day shift
weighs the empirical report, production coverage, migration cost and maintenance
of a custom model checker.
