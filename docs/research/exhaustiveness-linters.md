# Exhaustiveness linters for Go and TypeScript versus Rust's `match`

This follows the exhaustiveness sections of the Go prototype report
([#1835](go-vs-rust-prototype.md#exhaustiveness-guarantees)) and the TypeScript
report ([#1840](typescript-vs-rust.md#exhaustiveness)). Those prototypes
replaced Rust's compiler check with a project-specific typed-AST test in Go
([`exhaustiveness_test.go`](../../prototype/go/exhaustiveness_test.go)) and
hand-written `satisfies never` defaults in TypeScript
([`delivery.ts`](../../prototypes/typescript/src/delivery.ts)). The question
here is whether existing linters close that gap. It is evidence for the Day
shift. [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md) is unchanged.

Sources are tool READMEs, package docs, source code at the cited tags, the Go
spec, the TypeScript and Rust docs, and Clippy source. Repository dates come
from the GitHub API on 2026-11-09.

## What was run and what was only read

Run locally in scratch copies of both prototypes (nothing in the repository was
changed), with Go 1.26.5, Node 26.7.0, TypeScript 5.9.3, ESLint 9.39.5 and
typescript-eslint 8.72.2:

- `exhaustive` v0.17.1, `go-check-sumtype` v0.5.0 and golangci-lint v2.15.0
  against `prototype/go`, against a mutant adding `ciCancelled` to `ciState`,
  and against a small sealed-interface fixture with payload structs.
- `@typescript-eslint/switch-exhaustiveness-check` at `error` and `tsc --noEmit`
  against `prototypes/typescript/src`, plus mutants adding a `closed` State
  variant, and a `tsc --strict` fixture for return-type checking.

Everything else below (option defaults, scope limits, maintenance) was read
from the cited sources.

## Go: `nishanths/exhaustive` (enum constants)

**Model.** An enumerated type is a defined type with a numeric or string
underlying type and no type parameters; its members are the constants of that
type declared in the same block
([enumerated doc](https://github.com/nishanths/exhaustive/blob/v0.17.1/passes/enumerated/doc.go)).
Variants are therefore constants only; they carry no payload.

**What it checks.** Expression switches on an enumerated type; with
`-check=typeswitch`, type switches; with `-check=mapliteral`, map literals. The
default is `switch` only. Switches in another package need to list only the
exported constants
([exhaustive doc](https://github.com/nishanths/exhaustive/blob/v0.17.1/passes/exhaustive/doc.go)).
If-chains are not analyzed: the analysis "scans the syntax tree ... for switch
statements" (same doc).

**`default`.** "The presence of a default case will not satisfy missing cases";
flag `-d` changes that, and `-defrequire` additionally demands a default (same
doc). The flag names in older releases (`-default-signifies-exhaustive`,
`-explicit-exhaustive-switch`, `-check=switch,map`) were renamed in the
v0.13–v0.15 rework; golangci-lint keeps the old option names as deprecated
aliases ([v2.15.0 changelog](https://github.com/golangci/golangci-lint/blob/v2.15.0/CHANGELOG.md),
[adapter](https://github.com/golangci/golangci-lint/blob/v2.15.0/pkg/golinters/exhaustive/exhaustive.go)).

**Gaps.** Go's zero value for a numeric type is `0`, and any representable
constant converts to the type ([Go spec: zero value](https://go.dev/ref/spec#The_zero_value),
[conversions](https://go.dev/ref/spec#Conversions)), so `ciState(9)` is a legal
value no linter can reject; Go switches have no exhaustiveness rule and default
cases are optional ([Go spec](https://go.dev/ref/spec#Switch_statements)). The
type-switch mode does not support generic types implementing the interface, and
skips `any` and `error` switches (exhaustive doc). `//exhaustive:ignore` turns
off a switch, with no required reason (same doc).

**Run.** On the unchanged Go prototype `exhaustive ./...` reports nothing. With
`ciCancelled` added, it reported `delivery.go:81:4: missing cases in expression
switch: ciCancelled` (exit 3), the same location the custom test reported
(`delivery.go:81:4: missing ciCancelled`). After adding a `default:` to that
switch it still failed; with `-d` it passed. On the payload fixture,
`-check=typeswitch` counts both `T` and `*T` as cases when methods have value
receivers, because `*T`'s method set includes `T`'s
([Go spec: method sets](https://go.dev/ref/spec#Method_sets)), so a switch
listing every value type was still reported. With pointer receivers it reported
only the genuinely missing `*Failed, *Passed`.

**Maintenance.** Latest tag v0.17.1 (2026-10-31); v0.13.0 through v0.17.1 all
landed between 2026-09 and 2026-10, after v0.12.0 in 2023-11.

## Go: `go-sumtype` and `go-check-sumtype` (sealed interfaces)

**Model.** A `//sumtype:decl` comment marks an interface that must be sealed by
an unexported method; variants are the types in the same package that implement
it, so each variant is a struct that can carry its own fields
([go-check-sumtype README](https://github.com/alecthomas/go-check-sumtype/blob/master/README.md)).
Facts are exported, so switches in importing packages are checked too
([analyzer.go](https://github.com/alecthomas/go-check-sumtype/blob/master/analyzer.go)).

**What it checks.** Type switches only (`checkFile` inspects
`*ast.TypeSwitchStmt`, [check.go](https://github.com/alecthomas/go-check-sumtype/blob/master/check.go)).
Single type assertions, if-chains, expression switches and maps are not checked.

**`default`.** A non-panicking `default` satisfies the check unless
`-default-signifies-exhaustive=false`; the flag defaults to `true`
([config.go](https://github.com/alecthomas/go-check-sumtype/blob/master/config.go)).
A default whose only statement is `panic(...)` never satisfies it (check.go).
There is no notion of a nil case, and an interface's zero value is `nil`
([Go spec](https://go.dev/ref/spec#The_zero_value)), so a nil variant always
falls through to `default` or past the switch.

**Run.** On the fixture, default settings reported the panicking-default switch
and passed the non-panicking-default switch; with
`-default-signifies-exhaustive=false` both were reported. The if-chain using
`c.(*Failed)` produced nothing.

**Upstream.** BurntSushi's original has the same model but always accepts a
default and has no flags ([README](https://github.com/BurntSushi/go-sumtype/blob/master/README.md));
it has no tags and its last commit is 2025-03-21. The fork says the original
"seems largely unmaintained"; it released v0.5.0 on 2026-08-28 and golangci-lint
pins a commit from later that day.

The Go prototype uses integer enums, not sealed interfaces
([`states.go`](../../prototype/go/states.go)), so this linter would find
nothing there today.

## Go: golangci-lint integration

Both are bundled: `exhaustive` since v1.28.0 and `gochecksumtype` since
v1.55.0. golangci-lint v2.15.0 (2026-10-16) pins `exhaustive` v0.15.0 and
`go-check-sumtype` at commit `ae6904d28606`. Neither is enabled by default: the
`standard` set is `errcheck`, `govet`, `ineffassign`, `staticcheck` and
`unused` ([linter registry](https://github.com/golangci/golangci-lint/blob/v2.15.0/pkg/lint/lintersdb/builder_linter.go)).
Defaults inside golangci-lint mirror the tools: `exhaustive`'s
`default-signifies-exhaustive` is `false` and `gochecksumtype`'s is `true`
([reference config](https://github.com/golangci/golangci-lint/blob/v2.15.0/.golangci.reference.yml)).
Any reported issue makes golangci-lint exit 1 (`issues-exit-code`, same file),
which fails a CI step.

Run: with `exhaustive` and `gochecksumtype` enabled, the unchanged prototype gave
`0 issues` (exit 0); the `ciCancelled` mutant gave one `exhaustive` issue (exit
1). With no configuration the same mutant produced only `errcheck` and
`staticcheck` findings and no exhaustiveness finding.

## TypeScript: `switch-exhaustiveness-check`

**Model.** It reports a `switch` whose discriminant type is a union of literal
types or an enum and which misses a constituent
([rule docs](https://typescript-eslint.io/rules/switch-exhaustiveness-check),
[source](https://github.com/typescript-eslint/typescript-eslint/blob/main/packages/eslint-plugin/src/rules/switch-exhaustiveness-check.ts)).
For discriminated unions, switching on `state.kind` makes the discriminant a
literal union, and TypeScript narrows the payload in each case
([TypeScript narrowing](https://www.typescriptlang.org/docs/handbook/2/narrowing.html#discriminated-unions)),
so payload-bearing variants are covered. The rule registers only a
`SwitchStatement` visitor (source), so if-chains and object lookups are
unchecked.

**`default`.** Defaults in the source are `considerDefaultExhaustiveForUnions:
false`, `allowDefaultCaseForExhaustiveSwitch: true`,
`requireDefaultForNonUnion: false`. With the first at `false`, a `default` does
not hide a missing union member; setting the second to `false` reports a
default on an already exhaustive switch; the third requires a default for
`number`/`string` switches (rule docs). The docs suggest the stricter setting
plus "a `satisfies never` check" when defaults are deliberately kept.

**Requirements.** The rule has `requiresTypeChecking: true` (source), so it needs
typed linting. It is in the `all` shared config only, not in `recommended` or
`strict`, and typescript-eslint does "not recommend" extending `all`
([shared configs](https://typescript-eslint.io/users/configs/)); it must be
enabled by name. At `error` severity ESLint exits 1
([ESLint exit codes](https://eslint.org/docs/latest/use/command-line-interface#exit-codes)).

**Run.** On the unchanged prototype the rule passes (exit 0); the existing
`satisfies never` defaults are allowed by the default options. Adding a
`closed` variant to `State` *and constructing it inside `run()`*: `tsc` failed
(`TS1360 ... does not satisfy the expected type 'never'`) and the rule failed
(`Switch is not exhaustive. Cases not matched: "closed"`). After deleting
`state satisfies never` but keeping an empty `default:`, `tsc` passed and only
the rule still failed. With `considerDefaultExhaustiveForUnions: true` it passed
too. Adding the variant to the type *without* assigning it anywhere in `run()`
passed both: `state` is declared with an initializer, and TypeScript narrows a
variable to its assigned types ([narrowing: assignments](https://www.typescriptlang.org/docs/handbook/2/narrowing.html#assignments)),
so the switch never sees `closed`. That is sound for that loop, but means the
check fires when a variant is first produced, not when it is declared.

**Maintenance.** typescript-eslint v8.72.2 was released 2026-11-09; the rule's
source last changed 2026-10-15.

## TypeScript: built-in partial checks

Without a linter, `never` checks are opt-in per switch, as the
[handbook](https://www.typescriptlang.org/docs/handbook/2/narrowing.html#exhaustiveness-checking)
shows. A function whose declared return type excludes `undefined` also catches
a missing case if every case returns: on the fixture, `tsc --strict` reported
`TS2366: Function lacks ending return statement and return type does not
include 'undefined'`. With a `string | undefined` return, only
[`noImplicitReturns`](https://www.typescriptlang.org/tsconfig/#noImplicitReturns)
(not part of `strict`) reported `TS7030: Not all code paths return a value`.
Neither helps the prototype's `run()`, whose switch is a statement inside a
`for (;;)` loop.

## Rust, as configured here

A `match` must cover every value: "Matches in Rust are *exhaustive*", and a
missing `None` is error E0004 ([Rust book](https://doc.rust-lang.org/book/ch06-02-match.html#matches-are-exhaustive)).
This is the compiler, cannot be turned off, applies across crates, and covers
payload-bearing variants such as `Ci::Failed(FailedChecks)`
([`src/ci.rs`](../../src/ci.rs)). Safe code cannot produce an enum value outside
its variants. Two holes remain. A `_` arm satisfies the compiler (same chapter),
and `if let` "lose[s] the exhaustive checking `match` enforces"
([Rust book](https://doc.rust-lang.org/book/ch06-03-if-let.html)); outside its
crate, a `#[non_exhaustive]` enum always needs a wildcard
([Reference](https://doc.rust-lang.org/reference/attributes/type_system.html)).

[`Cargo.toml`](../../Cargo.toml) sets
[`clippy::wildcard_enum_match_arm`](https://clippy.rust-lang.org/master/index.html#wildcard_enum_match_arm)
to `warn`, and [CI](../../.github/workflows/ci.yml) runs
`cargo clippy --all-targets -- -D warnings`. The lint is in Clippy's
`restriction` group, so it is opt-in; it reports both `_` and a bare binding
used as a catch-all
([declaration](https://github.com/rust-lang/rust-clippy/blob/master/clippy_lints/src/matches/mod.rs),
[implementation](https://github.com/rust-lang/rust-clippy/blob/master/clippy_lints/src/matches/match_wild_enum.rs)).
The crate currently has 43 `#[expect(clippy::wildcard_enum_match_arm, reason
= ...)]` exceptions. The "no default hides a new variant" half of Rust's gate
is therefore also a lint failing CI, the same mechanism as golangci-lint or
ESLint; the compiler supplies only the "no missing case" half.

## Comparison

| | Payload variants | `default` hides new variant? | Forms checked | Invalid values possible | Enforced by |
|---|---|---|---|---|---|
| Rust `match` + Clippy lint | yes | no (lint, `#[expect]` with reason) | `match` only, not `if let` | no (safe code) | compiler + opt-in lint |
| Go prototype test | no | no | expression switches, one package | yes | project test |
| Go `exhaustive` | no (constants) | no, unless `-d` | expression/type switches, map literals | yes | opt-in linter |
| Go `go-check-sumtype` | yes (structs) | yes, unless flag `false` | type switches | `nil` interface | opt-in linter |
| TS `satisfies never` | yes | no, where written | switches that include it | via casts/unvalidated JSON | `tsc` |
| TS `switch-exhaustiveness-check` | yes | no (default options) | switch statements | as above | opt-in typed lint |

## Conclusions

**Go.** `exhaustive` with default settings reproduces the prototype's custom
checker: it found the same missing case at the same line, does not accept a
`default`, and also handles other packages, type switches and map literals.
The only behavior it lacks is the custom test's "every enum has at least one
switch" rule. The prototype should have used it through golangci-lint instead
of maintaining 129 lines of typed-AST test, provided the config enables the
linter explicitly. It does not close the gap with Rust: variants remain bare
constants, zero and converted integers stay representable, and payload-carrying
variants need a second modeling style (sealed interfaces) with a second linter
whose default accepts `default` and ignores `nil`. With both linters configured
strictly, Go removes most of the *checking* gap for switches but none of the
*modeling* gap: Go types still cannot forbid an out-of-range value.

**TypeScript.** `switch-exhaustiveness-check` with default options gives
discriminated-union switches the same guarantee as Rust's `match` plus the
wildcard lint, payloads included, without per-switch discipline; the
`satisfies never` lines become a second, compiler-level guard rather than the
only one. The remaining differences are that it is a typed ESLint pass outside
`tsc` that must be enabled by name, it ignores if-chains, and union types do
not constrain values arriving through casts or unvalidated JSON (already noted
in the [TypeScript report](typescript-vs-rust.md#exhaustiveness)). The
prototype was right to rely on `satisfies never` given no lint setup, but a
production TypeScript port should enable the rule with
`allowDefaultCaseForExhaustiveSwitch: false` or keep the `satisfies never`
defaults it allows.

**Overall.** For the property the prototypes were asked to show (adding a
variant fails CI at every switch that does not handle it), configured linters
make Go integer enums and TypeScript discriminated unions roughly equivalent to
Rust as configured here. Rust's remaining advantage is that the missing-case
half is unconditional, and that its enums carry payloads and cannot hold
invalid values. Go keeps the largest residual gap.
