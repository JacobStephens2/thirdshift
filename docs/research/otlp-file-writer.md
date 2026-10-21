# Writing OTLP JSON Lines from Rust: upstream, a crate, or thirdshift's own

Research date: 2026-10-21 (America/New_York).

**Question.** In [#698](https://github.com/JacobStephens2/thirdshift/issues/698) Q4.14, the recommendation was (a): thirdshift writes each Session trace as OTLP JSON itself, with the serde it already has. Jacob answered that if there's a gap, he is open to four things: a Rust crate of thirdshift's own that others could use; contributing to OpenTelemetry Rust; using something that already exists; or (a), if that is enough and isn't hand-rolling in a bad way. This note covers six questions:

- what upstream would accept, and when (§1);
- what opentelemetry-rust-contrib requires (§2);
- which crates already exist (§3);
- what a standalone crate would take (§4);
- what to call it (§5);
- how cheaply (a) could be pulled out later (§6).

§7 is the direct input to Q4.14.

**Method.** Primary sources only, read on 2026-10-21:

- GitHub issues, pull requests and files, through read-only `gh api` calls;
- crates.io API records;
- crate sources, downloaded from static.crates.io into the session scratchpad and read without building;
- the OTLP specification, the OpenTelemetry specification and the protobuf JSON mapping;
- the Linux Foundation's trademark pages.

Links pin commits. Nothing was posted to GitHub, no downloaded code was built or run, and no Collector was started. This builds on the 2026-10-20 research summarised under "OpenTelemetry in Rust is at 0.33.1" in `.scratch/grill-with-docs/done/698-evals/`. Its dependency counts are reused here, not re-measured: 17 new crates for the SDK with the official types, and 20 with `opentelemetry-otlp`.

**Tags.** **[interpretation]**: judgment rather than documented fact. **[needs live check]**: settled only by a real run. **[unverified]**: looked for and not settled.

## TL;DR

- **Upstream would take a file exporter now, in the main repository, and one is already in review.**
  - [PR #3763](https://github.com/open-telemetry/opentelemetry-rust/pull/3763) adds `opentelemetry-otlp-file`, a separate crate with span, log and metric exporters.
  - Two maintainers settled its home: the main repository, not contrib. The spec's Development status only has to be stated in the crate's docs (§1.3).
  - Its CI passes, but nobody has approved it. It is waiting on its author for four review threads, and on a related refactor (#3512) that removes functions it calls (§1.2).
  - The specification page hasn't changed since February 2025, and nothing proposes making it Stable (§1.5).
- **Correction to the earlier research.** One crate does already write the format: the Tor Project's Arti publishes `otlp-file-exporter`, for traces only. And a PR did follow #2602. Q4.14's "No crate writes the OTLP file format" needs updating (§7).
- **Nothing that exists fits Session traces.**
  - #3763 and Arti's crate are SDK exporters. They need the OpenTelemetry SDK, about 18 new crates for thirdshift [interpretation]. They write a batch per line and one signal per file, and they read nothing back.
  - The other hits are a private API (Apollo), a custom envelope of gzipped protobuf (dev7a), and framework-bound string formatting under the LGPL (hopf-otel) (§3).
- **A crate of thirdshift's own would be small and cheap to keep up.**
  - Logs and traces come to 18 messages, 5 enums and 77 fields.
  - In two years, 16 commits touched those protos. Four changed the schema, and only `event_name` matters to a writer (§4.6).
  - With only serde and serde_json as dependencies, it can support Rust 1.71 and later (§4.5).
  - `otlp-jsonl`, `otlp-file` and `otlp-json-writer` are all free on crates.io (§5).
  - Avoid `opentelemetry-` and `otel-` in the name. OpenTelemetry® is a registered Linux Foundation mark, and the Foundation's policy says a mark shouldn't be part of a product name or be abbreviated (§5).
- **The best path [interpretation]: (a) now, to the standard in §4.3–§4.4 and behind the boundary in §6, so it can become a crate later.**
  - Publish the crate once a second user wants it.
  - To contribute upstream, test or review #3763 rather than write another exporter.
  - (a) isn't hand-rolling in a bad way: JetBrains does the same for its own OTLP JSON Lines files (§3.2).
- **A conflict to settle in the grilling.**
  - ADR 0019 puts a session's log records and spans in one file. The File Exporter spec says "Files must contain exactly one type of data".
  - The Collector's receiver would probably still read such a file [interpretation from its source; needs live check].
  - Writing two files per session would conform, at little cost (§7).

## 1. Upstream: opentelemetry-rust#2602 and PR #3763

### 1.1 The issue

[#2602](https://github.com/open-telemetry/opentelemetry-rust/issues/2602), "[Feature]: OTLP File Exporter", was opened on 2025-02-04 by joshwlewis. Two weeks later he merged a traces-only file exporter of his own into Heroku's libcnb.rs ([libcnb.rs#909](https://github.com/heroku/libcnb.rs/pull/909), merged 2025-02-19). The issue asks for "a `FileExporter` that complies with" the File Exporter specification. It was opened because [#2040](https://github.com/open-telemetry/opentelemetry-rust/pull/2040) stopped `opentelemetry-stdout` from taking a writer and printing JSON.

- The same day, maintainer Cijo Thomas [replied](https://github.com/open-telemetry/opentelemetry-rust/issues/2602#issuecomment-2634466371): "Happy to host it in the contrib repo. (Or even this repo.) Feel free to send a PR." The 2026-10-20 research quoted only the first sentence.
- On 2026-03-18, maintainer Scott Gerring added the label `S-blocked-spec` ("Status: Blocked on open or unresolved spec"), without a comment.
- The OTLP stabilisation tracker [#3061](https://github.com/open-telemetry/opentelemetry-rust/issues/3061) lists the issue as an open feature request. Since 2026-09-28, PR #3763 has claimed to close it.

### 1.2 The PR

[#3763](https://github.com/open-telemetry/opentelemetry-rust/pull/3763), "feat: add `opentelemetry-otlp-file` crate implementing the OTLP File Exporter", was opened on 2026-09-28 by garysassano, an OpenTelemetry organisation member. At head [`e8bf3b7`](https://github.com/garysassano/opentelemetry-rust/tree/e8bf3b7498fb9db5c85343d732cf0cfd386ecdc8/opentelemetry-otlp-file):

- **Exporters.** It provides `SpanExporter`, `LogExporter` and `MetricExporter` for the SDK.
  - Each export writes one line of compact OTLP JSON: a `TracesData`, `LogsData` or `MetricsData` object.
  - Lines go to stdout by default, or are appended to a file, or go to any `Write + Send`.
  - "Empty batches write nothing."
- **Encoding.** It uses `opentelemetry-proto`'s `with-serde` types and that crate's SDK-to-proto `transform` functions.
- **Manifest.** Its [manifest](https://github.com/garysassano/opentelemetry-rust/blob/e8bf3b7498fb9db5c85343d732cf0cfd386ecdc8/opentelemetry-otlp-file/Cargo.toml) depends on `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-proto` (features `gen-tonic-messages` and `with-serde`), `serde`, `serde_json` and `thiserror`. It targets Rust 1.75 and Apache-2.0, at version 0.33.1 to match the workspace.
- **Testing.** The PR body says the output was read by the Collector's `otlp_json_file` receiver (contrib 0.161.0 and 0.162.0) and was tried on AWS Lambda. The crate builds with Rust 1.75.
- **Size.** It adds 2,068 lines across 13 files. The repository's [CONTRIBUTING](https://github.com/open-telemetry/opentelemetry-rust/blob/f61467c7854196220cd546eb0367f62f889f3ad7/CONTRIBUTING.md) recommends "keeping PRs under 500 lines", and the author offered to split it.

State on 2026-10-21:

- On commit `e8bf3b7`, 29 checks pass and 3 are skipped.
- No review has approved the PR, so GitHub reports it as `blocked`.
- The project's PR dashboard says "Waiting on the author", for four inline threads that contributor herin049 opened on 2026-10-14. They ask for an `fsync` on files, a better name than `Output`, a simpler `output.rs`, and end-to-end tests that write a real file.
- It has no milestone.

[Interpretation:] the issue's `S-blocked-spec` label now lags behind the discussion on the PR.

The related refactor [#3512](https://github.com/open-telemetry/opentelemetry-rust/pull/3512), "chore: decouple opentelemetry-proto from opentelemetry-sdk", has been open since 2026-05-11. It removes the public `opentelemetry_proto::transform` module that #3763 calls.

- On 2026-10-19, maintainer Lalit Kumar Bhasin [wrote](https://github.com/open-telemetry/opentelemetry-rust/pull/3512#issuecomment-6270622892): "The file exporter in #3763 also uses these conversions for all three signals, so we should coordinate the changes." He suggested "a small, separate conversion crate".
- Scott Gerring [agreed](https://github.com/open-telemetry/opentelemetry-rust/pull/3512#issuecomment-6279550971) the same day.

### 1.3 Main repository or contrib, now or later

The maintainers ([README](https://github.com/open-telemetry/opentelemetry-rust/blob/f61467c7854196220cd546eb0367f62f889f3ad7/README.md#approvers-and-maintainers)) discussed this on the PR:

- **Scott Gerring, 2026-10-06** ([comment](https://github.com/open-telemetry/opentelemetry-rust/pull/3763#discussion_r4194258776)): "I feel like we should host this in `contrib` _or_ as an unstable feature in the OTLP crate (which will help with re-use), in particular because the spec is still in development."
- **Lalit Kumar Bhasin, 2026-10-08**, in two comments:
  - ([comment](https://github.com/open-telemetry/opentelemetry-rust/pull/3763#discussion_r4224422011)): "I'd keep this as a separate crate in this repo, unless other maintainers have concerns. The file exporter has a distinct configuration and dependency surface from HTTP/gRPC, and it already reuses the proto transforms."
  - ([comment](https://github.com/open-telemetry/opentelemetry-rust/pull/3763#discussion_r4224229839)): "The spec's Development status should be clear in the docs, but I don't think it requires moving this to contrib."
- **Scott Gerring, 2026-10-19** ([comment](https://github.com/open-telemetry/opentelemetry-rust/pull/3763#discussion_r4310830351)): "Fine for me !"

So upstream would accept a file exporter **now**, while the specification is still Development, as **a separate crate in the main repository** rather than in contrib.

That fits CONTRIBUTING's FAQ: "only the fully opened standards should live in SDK packages/repos", and the File Exporter is OpenTelemetry's own specification. CONTRIBUTING also asks for an `otel_unstable` feature flag around anything "with experimental status". The PR doesn't use one. Instead, it marks the whole crate Development in its README and crate docs, which is what Lalit Kumar Bhasin asked for.

When [interpretation]: the merge waits on the author's follow-up and on #3512. After that, a release rides the workspace's train. Recent releases were minors 0.31.0 (2025-09-25), 0.32.0 (2026-05-08) and 0.33.0 (2026-09-18), and a patch, 0.33.1 (2026-10-09). Expect months rather than weeks.

### 1.4 What #3763 would give thirdshift

Measured against what ADR 0019 and Q3.15–Q3.16 require of Session traces [interpretation, from the PR's source and README]:

| Session-trace need | #3763 |
|---|---|
| A log record on disk as each Harness event arrives | Yes, with the SDK's simple processor: one record per line, each repeating the resource. The batch processor holds records in memory |
| Spans written as they end, with one trace ID shared by a command's sessions | Through SDK spans. The trace ID comes from a remote parent context or a custom ID generator |
| Log records and spans in one file | Against its README: "give each signal its own file" |
| Converting old Session logs, with their own times and IDs | Awkward. The SDK records live events; it doesn't import old ones |
| Reading Session traces back (the Console, `show`, the Waterfall) | Not offered. Reading needs `opentelemetry-proto` |
| Blocking code outside the Console (ADR 0016) | Works. The default processors export on their own threads, and the README warns only about the async-runtime processors |
| No new dependencies | About 18 new crates: the 17 measured for the SDK with the official types, plus this one. prost is among them |

So taking #3763 means Q4.14's option (b), with the file writer already written, once it is released.

### 1.5 The specification

[`file-exporter.md`](https://github.com/open-telemetry/opentelemetry-specification/blob/e30cae5ce80e7bf9a8f051f20018b764d4bac029/specification/protocol/file-exporter.md), at `main` `e30cae5` (2026-10-20), says "**Status**: Development". The latest specification release is v1.61.0 (2026-09-14).

- Its last change was a link fix ([`0c853fe`](https://github.com/open-telemetry/opentelemetry-specification/commit/0c853fe71aad74fa8d26c76c3899e87e594a9f5f), 2025-02-26).
- Its last substantive change added the stdout exporter and the configuration table ([#4183](https://github.com/open-telemetry/opentelemetry-specification/pull/4183), merged 2024-11-26).

It requires:

- a JSON Lines file: UTF-8, one valid JSON value per line, `\n` between lines, and preferably the `jsonl` extension;
- the OTLP JSON encoding, with "Only top-level objects, `TracesData`, `MetricsData`, and `LogsData`";
- "Files must contain exactly one type of data: traces, metrics, or logs";
- an "output stream" option, which MUST exist and defaults to stdout, plus support for the metrics environment variables.

What was moving on 2026-10-21:

| Item | State | What it would change |
|---|---|---|
| [#3817](https://github.com/open-telemetry/opentelemetry-specification/issues/3817), "Is OTLP file exporter required for SDKs?" | Open since 2024-01-11, labelled `triage:accepted:ready-with-sponsor`. Closed as stale on 2026-08-21 and reopened the same day | Whether every SDK has to ship one. Its opening post notes that file names and rotation are unspecified |
| [#4940](https://github.com/open-telemetry/opentelemetry-specification/issues/4940), a pretty-print option | Accepted on 2026-07-15, with jack-berg as sponsor | An option, off by default. With it on, the output is no longer JSON Lines |
| [#5243](https://github.com/open-telemetry/opentelemetry-specification/issues/5243), appending to or truncating an existing file | Waiting for community feedback. trask wrote "+1 for append as the least surprising option" | What an SDK does when it opens the file itself |
| [#5355](https://github.com/open-telemetry/opentelemetry-specification/pull/5355), no export timeout for synchronous output that doesn't go over a network | A PR. On 2026-10-20 its dashboard said "Waiting on maintainers" and "Merge when ready" | Relaxes the export-timeout rule for stdout and file exporters |

- **No path to Stable.** Searches of the specification and configuration repositories on 2026-10-21 found no issue or PR proposing to make the page Stable.
- **Declarative configuration.** The schema names the exporter `otlp_file/development` ([`logger_provider.yaml`](https://github.com/open-telemetry/opentelemetry-configuration/blob/52fdbb78cfd5b32f2cf312d6549a5f4e9640183c/schema/logger_provider.yaml#L96), latest release v1.2.0 on 2026-09-11).
- **Other languages.** The [compliance matrix](https://github.com/open-telemetry/opentelemetry-specification/blob/e30cae5ce80e7bf9a8f051f20018b764d4bac029/spec-compliance-matrix.md) lists "OTLP File exporter" only in its logs table. It marks the exporter as implemented in Java, Python and C++, and not in Go, JS, Rust, .NET or Kotlin.
  - Python ships it in its main repository, as `opentelemetry-exporter-otlp-json-file`. Its first release on PyPI was 0.64b0, on 2026-06-24.
  - Java ships it in its main repository's `exporters/logging-otlp`, under an `internal` package.

What *is* Stable is the encoding. The OTLP README's maturity table lists the JSON for common, resource, trace and logs as Stable ([README](https://github.com/open-telemetry/opentelemetry-proto/blob/09769935deb8b2ebcf73ad1e2142b691d17bf70b/README.md#maturity-level) at `0976993`). [Interpretation:] the open items are about how an SDK exporter is configured, not about what goes in a line. A writer of lines carries little of the Development risk.

## 2. opentelemetry-rust-contrib

Its [CONTRIBUTING](https://github.com/open-telemetry/opentelemetry-rust-contrib/blob/d98a3b60f6a60cfc00232046cf74f36825ed095e/CONTRIBUTING.md) gained the sections "Component Ownership" and "Adding a New Component" on 2026-06-16 ([#653](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/653)). For a new component:

- "**Before writing a PR, open an issue** describing what you want to add and why it belongs here."
- The crate is named `opentelemetry-<kind>-<name>`, in a folder of the same name. It starts at version 0.1.0, under Apache-2.0. Its README carries `Stability` (alpha, beta or stable) and `Owners` rows, and its CHANGELOG starts with a `vNext` heading.
- "Every component must have at least one owner", listed in [`.github/component_owners.yml`](https://github.com/open-telemetry/opentelemetry-rust-contrib/blob/d98a3b60f6a60cfc00232046cf74f36825ed095e/.github/component_owners.yml), and "Two or more owners per crate is preferred".
- Owners "must be OpenTelemetry Members". The [membership guide](https://github.com/open-telemetry/community/blob/9c30b48354004ef818bcf3661ece2b32ca4dd787/guides/contributor/membership.md#member) requires multiple contributions, and sponsorship "by 2 approvers" who come "from multiple member companies".
- Owners "Respond to issues and PRs ... in a reasonable timeframe" and "Keep the crate working as the upstream `opentelemetry` SDK and broader ecosystem evolve". At releases, maintainers handle the trivial dependency bumps.
- "After the PR is merged, the first release of the crate is cut by a maintainer."

The ownership rule is enforced.

- [#609](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/609) (2026-05-09) said "each component must have an owner" and gave the unowned crates two weeks.
- `opentelemetry-datadog` and `opentelemetry-stackdriver` were deprecated in June ([#636](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/636), [#637](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/637)).
- `opentelemetry-aws` found a new owner.

On scope, the repository "hosts community-contributed exporters, instrumentation libraries, resource detectors, propagators, and other extensions that don't belong in the core opentelemetry-rust SDK". A file exporter would qualify. But the maintainers have placed #3763 in the main repository (§1.3), so contrib isn't the route for one [interpretation].

How long new components took, from the GitHub API:

| PR | Crate | Author | Lines added | Opened → merged | Days |
|---|---|---|---|---|---|
| [#562](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/562) | etw-traces | contributor | 1,851 | 2026-03-13 → 2026-06-12 | 91 |
| [#487](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/487) | config | contributor | 1,804 | 2025-11-10 → 2025-12-02 | 22 |
| [#248](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/248) | instrumentation-tower | contributor | 1,337 | 2025-04-16 → 2025-07-01 | 76 |
| [#21](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/21) | etw-logs | contributor | 1,138 | 2024-01-03 → 2024-03-15 | 72 |
| [#203](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/203) | user-events-trace | maintainer | 649 | 2025-03-18 → 2025-03-20 | 2 |
| [#43](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/43) | etw-metrics | contributor | 762 | 2024-02-27 → 2024-02-29 | 2 |
| [#49](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/49) | resource-detectors | contributor | 369 | 2024-03-13 → 2024-03-14 | 1 |

Crates from community contributors of 1,100 to 1,900 lines took three weeks to three months.

The open proposal [#643](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/643) (2026-06-11, a bridge from prometheus-client) shows the issue-first step at work. A maintainer agreed the next day that it belongs in contrib, and it has no PR yet.

## 3. Existing crates outside the OpenTelemetry organisation

### 3.1 crates.io

**Searches.** The crates.io API was queried for the six requested terms ("otlp file", "otlp json", "otlp jsonl", "opentelemetry file exporter", "otel file" and "otlp writer") and 11 variants. It was also paged through the complete results for "otlp" (1,024 crates), "otel" (1,217) and "opentelemetry" (2,089).

**Filter.** That gave 2,712 distinct crates. Of these, 43 mention OpenTelemetry and also a file, JSON Lines, JSON, stdout or a writer. The source of every plausible hit was read.

| Crate | Licence | Downloads | Last release | Owners | What it writes | Follows the spec and OTLP JSON? |
|---|---|---|---|---|---|---|
| [`otlp-file-exporter`](https://crates.io/crates/otlp-file-exporter) 0.2.0 | MIT OR Apache-2.0 | 169 | 2026-10-01 (first: 2026-09-02) | gabi-250, ijackson, nmathewson, dgoulet-tor (the Tor Project's Arti) | An SDK `SpanExporter` that writes `{"resourceSpans": ...}` per batch, through `opentelemetry-proto`'s serde | Yes for traces, with no logs. It depends on `opentelemetry-proto` 0.32 with default features, which pull in tonic, and needs Rust 1.92. Its README: "This at some point will be implemented upstream, at which point this crate will be deprecated" |
| [`apollo-opentelemetry-stdout`](https://crates.io/crates/apollo-opentelemetry-stdout) 0.4.0 | MIT | 32,263 | 2026-10-06 | BrynCooke and apollographql's runtime-readiness team | SDK exporters that write `ExportLogsServiceRequest` and the like as JSON lines, to stdout, stderr or a writer | The encoding, yes, through `opentelemetry-proto`. But it is "internally used by the `apollo-opentelemetry` crate and should not be used directly". Its exporters are only under `__private`, and it panics when a write fails |
| [`otlp-stdout-span-exporter`](https://crates.io/crates/otlp-stdout-span-exporter) 0.17.1 | MIT | 27,099 | 2026-04-02 | alessandrobologna | A JSON envelope whose `payload` is "base64-encoded-gzipped-protobuf" | No |
| [`hopf-otel`](https://crates.io/crates/hopf-otel) 0.5.0 | LGPL-3.0-or-later | 336 | 2026-10-10 | cpkb-bluezoo | OTLP JSON Lines built with `format!` strings, for the Hopf server framework | The shape, yes, but with string attributes only, and tied to its framework |
| [`otlp-metrics-exporter`](https://crates.io/crates/otlp-metrics-exporter) 0.1.0 | non-standard | 1,106 | 2025-03-08 | songokas | JSON for the `metrics` crate | Metrics only |

These hits don't write OTLP JSON files:

- `joy-telemetry` and `opai-agents-observability` write JSON Lines of their own shape.
- `zerodds-observability-otlp`, `oxtel-otlp`, `teksilo-analytics-otlp` and `rlg-otlp` only send OTLP over HTTP.
- `otlp2parquet` writes Parquet.

Downloads and dates come from the crates.io API on 2026-10-21. The crates' sources were read at the versions shown, and Arti's crate at VCS commit `ce8bc6e0`.

### 3.2 Code on GitHub that isn't published

Repository searches found applications, not libraries. For example, `even-wei/agentlog` is "Local-first no-daemon CLI for archiving agent hook events as OTLP Logs JSON", with no licence.

Code searches found many projects that write OTLP JSON in their own code, in three ways:

- **Through `opentelemetry-proto`'s serde:**
  - NVIDIA NeMo-Relay ([`otel_file.rs`](https://github.com/NVIDIA/NeMo-Relay/blob/7bbcbf125bbcaea45d4b652b740a23feb5ae8fad/crates/core/src/observability/otel_file.rs));
  - MystenLabs Walrus ([`tracing_export.rs`](https://github.com/MystenLabs/walrus/blob/c3c51b2a1d9e7ce8a0ce2a5efae3d4bb0999c7cf/crates/walrus-service/src/common/telemetry/tracing_export.rs), which cites the "OpenTelemetry Protocol File Exporter");
  - Heroku's libcnb.rs ([`tracing.rs`](https://github.com/heroku/libcnb.rs/blob/d967bd2c0b90681a764f99c5af1a2cd167725694/libcnb/src/tracing.rs)).
- **With hand-written serde types:** JetBrains' [`avl-trace/src/otlp.rs`](https://github.com/JetBrains/intellij-community/blob/103f8d7996624c0056a81f656d1a5971b7a704a2/tools/vm/crates/avl-trace/src/otlp.rs), 737 lines. It writes `spans.jsonl` and `logs.jsonl`. Its header says: "The structs are written by hand instead of generated from the OTLP protos, because the standard proto3 JSON mapping is the wrong JSON for this encoding." Its newtypes write nanosecond times and `intValue` as decimal strings.
- **With string formatting:** `hopf-otel` and `zerodds-observability-otlp`.

[Interpretation:] writing OTLP JSON in a project's own code is common. Typed serde structs are the careful way to do it, and string formatting is the way to avoid.

## 4. A crate of thirdshift's own

### 4.1 Scope

[Interpretation:]

- **Logs and traces, in both directions.** Session traces have to be written (records and spans) and read (the Console's log view, `show` and the Waterfall).
- **No metrics.** `metrics.proto` is the largest of the protos, and it is where `opentelemetry-proto`'s open serde bugs are (§4.2).
- **Size.** At `0976993`, `logs.proto`, `trace.proto`, `common.proto` and `resource.proto` together hold 18 messages, 5 enums and 77 fields.
- **No semantic conventions.** These stay outside the crate: the GenAI conventions are still Development, and attribute names are the caller's choice.

### 4.2 API shape

| | Its own serde types | `opentelemetry-proto` with `with-serde` | An SDK exporter |
|---|---|---|---|
| Dependencies | serde and serde_json | prost. Until #3512 lands, also `opentelemetry` and `opentelemetry_sdk` ([manifest](https://github.com/open-telemetry/opentelemetry-rust/blob/f61467c7854196220cd546eb0367f62f889f3ad7/opentelemetry-proto/Cargo.toml)) | The SDK and the proto crate, as in #3763 |
| Public API must follow | serde 1.x only | Every 0.x minor of `opentelemetry-proto`: 7 since 2024-10-21 (0.27.0 to 0.33.0) | The same |
| Correct encoding rests on | Its own tests (§4.4) | Upstream, which still needed fixes in 2026: [#3598](https://github.com/open-telemetry/opentelemetry-rust/issues/3598), closed 2026-07-21, and [#3764](https://github.com/open-telemetry/opentelemetry-rust/issues/3764), fixed by #3765 on 2026-10-03. NaN and infinite doubles still become `null` ([#3442](https://github.com/open-telemetry/opentelemetry-rust/issues/3442) and [#3496](https://github.com/open-telemetry/opentelemetry-rust/pull/3496) are open) | The same |
| Different from what upstream offers | Yes: no SDK, no prost, and it reads as well as writes | No. It would be a thin wrapper over the official types | No. It would duplicate #3763 |

[Interpretation:] only the first column is worth a crate. The second would be a few dozen lines over the official types, which #3512 will free from the SDK. The third is #3763.

The crate's public surface would be:

- data types mirroring `LogsData` and `TracesData`. These double as `ExportLogsServiceRequest` and `ExportTraceServiceRequest`, because each pair has the same single field 1 (`resource_logs` or `resource_spans`) in [`logs_service.proto`](https://github.com/open-telemetry/opentelemetry-proto/blob/09769935deb8b2ebcf73ad1e2142b691d17bf70b/opentelemetry/proto/collector/logs/v1/logs_service.proto) and `trace_service.proto`;
- newtypes for `TraceId` and `SpanId`;
- a writer that appends one complete line per call, holding a lock, then flushes, with an optional `fsync`. `write(2)` "does not make any guarantee that data has been committed to disk", and `fsync(2)` makes data retrievable "even if the system crashes or is rebooted". [Interpretation:] a process killed after `write` returns loses nothing, because the kernel already holds the data, so `fsync` matters only for a system crash or power loss;
- a line reader that accepts every form the specification allows.

### 4.3 The encoding rules

Q4.14 named four rules. The OTLP specification ([JSON Protobuf Encoding](https://github.com/open-telemetry/opentelemetry-proto/blob/09769935deb8b2ebcf73ad1e2142b691d17bf70b/docs/specification.md#json-protobuf-encoding)) and the [protobuf JSON mapping](https://protobuf.dev/programming-guides/json/) fix six more. Here are all ten, for writing (W) and reading (R):

1. `traceId`, `spanId` and `parentSpanId` are hex, and "not base64-encoded".
   - W: lowercase, 32 and 16 characters [interpretation].
   - R: "case-insensitive". The specification's own examples are uppercase.
2. "Values of enum fields MUST be encoded as integer values". These are `severityNumber`, `kind` and `status.code`.
3. Keys are "converted to lowerCamelCase. Original field names are not valid".
4. 64-bit integers "are encoded as decimal strings, and either numbers or strings are accepted when decoding". In these protos, that covers the five `fixed64` times and `intValue`. `droppedAttributesCount` and the `flags` fields are 32-bit and stay numbers.
5. A double is "a number or one of the special string values "NaN", "Infinity", and "-Infinity"". serde_json writes NaN as `null`, so `doubleValue` needs a helper.
6. `bytesValue` is standard base64, with padding.
7. A field at its default value "should" be omitted, and "Serializers should not emit `null`".
8. "Encoders SHOULD ensure `string` fields contain valid UTF-8" (added 2026-08-27 in [#845](https://github.com/open-telemetry/opentelemetry-proto/commit/3aae362b7c45f0d22cd20b78bb27e58c88584505)). A Harness line that isn't valid UTF-8 needs lossy decoding or a `bytesValue` body.
9. "senders SHOULD NOT create empty envelopes" (added 2025-02-26 in [#622](https://github.com/open-telemetry/opentelemetry-proto/commit/f246061de571bab74fe96a64ae4553624a197107)).
10. R: "receivers MUST ignore message fields with unknown names". So a reader must not use `deny_unknown_fields`. JetBrains' module does use it, which suits a closed format but not a library.

On top of these come the file rules from §1.5: one object per line, either `TracesData` or `LogsData`, and one signal per file.

### 4.4 Proving conformance

[Interpretation, most valuable first:]

1. **A round trip against the official types, in the tests only.**
   - Serialise with the crate's types, deserialise with `opentelemetry-proto`'s `with-serde` types, serialise again, and compare the two as JSON values. Then do the reverse.
   - As a dev-dependency, this adds nothing to the published crate or to thirdshift's binary. It only adds to test builds.
   - The official types have had bugs (§4.2), so a disagreement is a lead, not a verdict.
   - `opentelemetry-proto` 0.33.1 ships `tests/json_serde.rs` (2,399 lines), with canonical and alternative forms to borrow test cases from.
2. **The specification's examples.**
   - `opentelemetry-proto/examples/` holds `logs.json`, `trace.json`, `events.json` and `metrics.json` ([`0976993`](https://github.com/open-telemetry/opentelemetry-proto/tree/09769935deb8b2ebcf73ad1e2142b691d17bf70b/examples)). They are pretty-printed request payloads, maintained by hand and corrected several times. The latest correction was [#748](https://github.com/open-telemetry/opentelemetry-proto/commit/b363281e5c30f343bfd2405519cc3db93d11f252) on 2026-03-01, which wrote `uint64` and `fixed64` fields as strings.
   - The File Exporter page's own JSON Lines examples cover few fields, and every ID in its trace examples is empty.
   - Decode all of these, and compare the re-encoded output as JSON values.
3. **The Collector, as the reference reader.**
   - The `otlp_json_file` receiver is alpha for traces, metrics and logs ([`34c2da6`](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/34c2da6a9b67641bbf2e36250833fa773886439b/receiver/otlpjsonfilereceiver)). It reads with pdata, whose unmarshaler ignores unknown fields by default ([`pdata/plog/json.go`](https://github.com/open-telemetry/opentelemetry-collector/blob/3123b61db5448b8612d2da371169e5e40d24f1c5/pdata/plog/json.go)).
   - Read a fixture file with `start_at: beginning`, and with `max_log_size` above the longest line. The default is 1 MiB, and longer lines are split. Write the data back out with the `file` exporter and diff. #3763's author did exactly this.
   - In CI this means downloading `otelcol-contrib`. On GitHub's runners, nothing is sent from this host.

### 4.5 MSRV and CI

- **MSRV.** serde 1.0.229 declares Rust 1.56, and serde_json 1.0.151 declares 1.71, on crates.io. So a crate that depends only on serde and serde_json can declare `rust-version = "1.71"` or later.
  - OpenTelemetry Rust promises 1.75, and supports "The current stable Rust compiler and the three most recent minor versions before it".
  - Edition 2024, which thirdshift uses, became stable in [Rust 1.85](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/).
  - [Interpretation:] choose 1.85 with edition 2024, or 1.75 with edition 2021 to match OpenTelemetry Rust.
- **thirdshift has no MSRV.** It pins Rust 1.99.0 in `rust-toolchain.toml` and declares no `rust-version` (at [`ec959d3`](https://github.com/JacobStephens2/thirdshift/blob/ec959d3341502e3419371fa9871d8fd4e1bc243c/rust-toolchain.toml)). So the crate would need an MSRV job of its own.
- **CI** [interpretation]:
  - fmt, and clippy with `-D warnings`;
  - tests on stable, on Linux and macOS as thirdshift's CI already does;
  - a test job on the MSRV;
  - the round-trip tests;
  - a Collector job whenever the codec changes;
  - a docs build;
  - `cargo semver-checks` before each release.
- **Releases** can reuse thirdshift's trusted publishing (`rust-lang/crates-io-auth-action@v1` in `publish-crates.yml`).

### 4.6 Maintenance: how often the protos changed

From 2024-10-21 to 2026-10-21, 16 commits touched `logs.proto`, `trace.proto`, `common.proto`, `resource.proto` or the logs and trace service protos. Four changed the schema, and the other 12 changed only comments.

| Commit | Date | Change | Matters to a writer of logs and traces? |
|---|---|---|---|
| [`fcebfd2`](https://github.com/open-telemetry/opentelemetry-proto/commit/fcebfd2def995bd9f1139e49c5aa2dba57afb2ce) (#600) | 2024-12-11 | Added `LogRecord.event_name = 12`, which [#643](https://github.com/open-telemetry/opentelemetry-proto/commit/42319f8b5bf330f7c3dd4a097384f9f6d5467450) made Stable on 2025-04-10 | Yes |
| [`e4fd4f6`](https://github.com/open-telemetry/opentelemetry-proto/commit/e4fd4f681f583b1bdea4d375ad19ddc0dc1f2e9c) (#635) | 2025-03-28 | Added `EntityRef` and `Resource.entity_refs = 3` | Optional. A reader ignores it |
| [`2c8b633`](https://github.com/open-telemetry/opentelemetry-proto/commit/2c8b63368c7cbe5676a458ada4851bc3ef7b7e8a) (#733) | 2026-02-25 | Added `AnyValue.string_value_ref` and `KeyValue.key_ref` | No |
| [`9774645`](https://github.com/open-telemetry/opentelemetry-proto/commit/97746452a041810c3041073a781f5998a2cd31b5) (#768) | 2026-03-03 | Renamed those two to `*_strindex`, a week later | No |

In the same two years, the proto repository cut 9 releases (v1.4.0 to v1.11.1). The JSON text in its specification gained three clarifications, about empty envelopes, UTF-8 and 64-bit times. None of them changes what a correct writer outputs [interpretation].

[Interpretation:] following the format means about one small change a year. The real upkeep is owning the crate:

- answering issues and PRs from its users;
- bumping the MSRV and serde;
- handling security reports;
- keeping at least two crates.io owners, so the crate doesn't decay the way contrib's unowned crates did.

## 5. Naming

- **OpenTelemetry's guidance.** Its [marketing guidelines](https://github.com/open-telemetry/opentelemetry.io/blob/6a865f53d8e40c17f42772fb8fb100d62a61fb7e/content/en/community/marketing-guidelines.md) ask that its name be used "in line with the Linux Foundation's branding and trademark usage guidelines". The Registry's page for adding entries carries a note asking that "project names" follow them too. That note is inside an HTML comment, under a freeze notice ([`adding.md`](https://github.com/open-telemetry/opentelemetry.io/blob/c6df13913b1b0133aa46fdf9183c3a3f9ba64092/content/en/ecosystem/registry/adding.md)).
- **The Linux Foundation's policy.** Its [trademark usage guidelines](https://www.linuxfoundation.org/legal/trademark-usage) say:
  - "A trademark should not be used as part of your product name."
  - "A mark should not be combined with any other mark, hyphenated, abbreviated or displayed in parts."
  - For claims of compatibility, "Correct: <your product name> for <The Linux Foundation mark>" and "Correct: <your product name> compatible with <The Linux Foundation mark>", but "Incorrect: <The Linux Foundation mark> <your product name>".
- **The marks.** The Foundation's [trademark list](https://www.linuxfoundation.org/legal/trademarks) includes "OpenTelemetry®" among its registered marks. "OTel" and "OTLP" are not listed.
- **Practice is looser.** In crates.io's results for "opentelemetry", 34 of the 68 crates named `opentelemetry-*` or `opentelemetry_*` have their repositories outside the open-telemetry organisation. `opentelemetry-application-insights`, with 3.7 million downloads, is one. There are also 43 `otel-*` crates and 9 `otlp-*` crates. Contrib requires `opentelemetry-<kind>-<name>` for the crates it hosts.

On 2026-10-21, crates.io answered 404, meaning free, for all of these:

- `otlp-file`, `otlp-jsonl`, `otlp-json-writer`, `otlp-json`, `otlp-json-file`, `otlp-file-writer`, `otlp-jsonl-writer` and `otlp-writer`;
- `otlp-json-lines`, `otlpfile`, `otlp-lines`, `otlp-serde`, `otlp-json-encoding` and `otlp-records`;
- `otel-file` and `otel-jsonl`;
- `opentelemetry-otlp-file`, the unpublished name #3763 uses.

`otlp-file-exporter` is taken, by Arti (§3.1).

[Interpretation:] `otlp-jsonl` says what it does and uses no listed mark. It also can't be mistaken for #3763's `opentelemetry-otlp-file`. Describe it as "OTLP JSON Lines for OpenTelemetry", and carry the trademark notice. Avoid `otel-`, which abbreviates the registered mark.

## 6. Taking (a) now, and extracting it later

**Today.** A session's Harness lines are copied into a `File` created for that session, by `follow()`. See `src/session/recording.rs`, lines 95 and 233, at [`ec959d3`](https://github.com/JacobStephens2/thirdshift/blob/ec959d3341502e3419371fa9871d8fd4e1bc243c/src/session/recording.rs#L233). thirdshift is a single binary crate, with `src/main.rs`, no `lib.rs` and no `[workspace]`. Its `publish-crates.yml` publishes only `thirdshift`.

**The boundary to keep** [interpretation], so that extracting the code later is a move, not a rewrite:

- One module, say `src/otlp/`, that imports only `std`, `serde` and `serde_json`. It uses no `crate::` paths, and contains no Harness, Run, Session or Command types and no `thirdshift.*` names.
- Inside it:
  - the types listed in §4.1;
  - `TraceId` and `SpanId`, built from bytes. Generating random IDs stays in thirdshift, which already depends on `rand_core`;
  - a writer over any `Write`, which writes one complete line per call while holding a lock;
  - a reader over any `BufRead`.
- Outside it, in `session`:
  - turning each Harness event into a `LogRecord`;
  - the GenAI and `thirdshift.*` attributes;
  - the trace-ID policy and the file names;
  - sending the same JSON to an endpoint by HTTP POST.
- Its own unit tests and fixtures, kept beside it, including the round-trip and specification-example tests of §4.4. thirdshift has no library target, so these can't go under `tests/`.
- Every public type documented as though it were already published.

**Extracting.** With that boundary, extraction means moving the module into a workspace member (`crates/otlp-jsonl/`), adding its manifest, README, licence and CI jobs, and depending on it by both `path` and `version`.

Three costs are easy to miss:

- **Release order.** Cargo publishes a crate only when its dependencies are already on crates.io, so the release workflow would have to publish the library first.
- **Licence.** thirdshift is MIT, and OpenTelemetry's crates are Apache-2.0. Licensing the module as MIT OR Apache-2.0 from the start would keep upstream contribution open.
- **The CLA.** Contributing upstream means signing the CNCF CLA: "Make sure CLA is signed" (the main repository's CONTRIBUTING).

## 7. Direct input to Q4.14

| Jacob's option | What the research found | Fit [interpretation] |
|---|---|---|
| Contribute to OpenTelemetry Rust | Upstream would take a file exporter now, in the main repository, and #3763 is one (§1) | Writing another would duplicate it. Useful contributions would be: trying #3763 on a per-session file, reviewing it, or helping with #3512's conversion crate |
| Use something that exists | #3763 once it is released (the SDK route, about 18 crates). Nothing else fits (§3) | Only if thirdshift wants the SDK anyway. Even then, thirdshift still turns Harness events into records itself, and #3763 reads nothing back |
| A crate of thirdshift's own | No published crate writes and reads OTLP JSON Lines logs and traces without the SDK. The scope is small, the format changes about once a year, and the names are free (§4, §5) | Worth it once someone else needs it. The niche narrows when #3512 frees the official types from the SDK |
| (a) | Enough, and not hand-rolling in a bad way if it meets §4.3–§4.4 | Recommended now, behind the boundary in §6 |

Corrections for the grilling file:

- **No crate?** Q4.14 and the 2026-10-20 research say "No crate writes the OTLP file format", with #2602 "blocked on the specification". In fact, Arti's `otlp-file-exporter` writes it for traces. And #3763 is in review, with the maintainers agreed on where it goes.
- **Four rules?** Q4.14 counts "four encoding rules". Those four hold, and the same specifications add six details to them (§4.3).

**A conflict to decide.** ADR 0019 puts log records and spans in one file per session, but the File Exporter specification says "Files must contain exactly one type of data".

- **The Collector.** Its receiver parses each line once for each pipeline it's configured with. In the logs pipeline, a spans line yields no records and is skipped, because pdata ignores unknown fields. So a mixed file probably works there [interpretation, from `file.go` and `json.go`; needs live check].
- **Other readers** may not cope with a mixed file.

The options:

- (i) Two files per session, `….logs.jsonl` and `….spans.jsonl`. This conforms to the specification. Silent can watch the logs file, which grows with each event.
- (ii) One file, with the departure from the specification recorded in ADR 0019.

[Interpretation:] (i) costs little, and keeps the files readable by anything that follows the specification.

One more detail for either option: the Collector receiver's 1 MiB default `max_log_size` would split any record line that carries a very large raw Harness event.

## 8. Uncertainties and live checks

- **#3763's final shape and date** [unverified]. The open review threads and #3512 may change its API, for example how it converts SDK data. No release is scheduled.
- **Mixed files in the Collector** [needs live check]. This was read from source, not run.
- **Other readers of Session traces** [needs live check]. A local viewer, if Q4.12 picks one, wasn't checked for how it handles mixed files, default values or unknown fields.
- **The 18-crate figure for #3763.** It adds one crate to the earlier measurement of 17, and wasn't measured again.
- **Demand for a published crate.** The code in §3.2 suggests a niche, but none of its authors was asked.
- **The size of (a)** [interpretation]. JetBrains' module is 737 lines. By its author's count, #3763 is about 1,000 lines of code and docs plus about 780 lines of tests. Those two bound it, though thirdshift's module would cover reading as well as writing, for both logs and traces.

## Sources

Primary sources are linked beside each claim. To find the rest:

- **thirdshift.** At `ec959d3`: [`Cargo.toml`](https://github.com/JacobStephens2/thirdshift/blob/ec959d3341502e3419371fa9871d8fd4e1bc243c/Cargo.toml), [`src/session/recording.rs`](https://github.com/JacobStephens2/thirdshift/blob/ec959d3341502e3419371fa9871d8fd4e1bc243c/src/session/recording.rs), [`publish-crates.yml`](https://github.com/JacobStephens2/thirdshift/blob/ec959d3341502e3419371fa9871d8fd4e1bc243c/.github/workflows/publish-crates.yml) and [`ci.yml`](https://github.com/JacobStephens2/thirdshift/blob/ec959d3341502e3419371fa9871d8fd4e1bc243c/.github/workflows/ci.yml). Also [ADR 0019](../adr/0019-session-logs-become-session-traces.md), and Q4.14 in `.scratch/grill-with-docs/done/698-evals/`.
- **OpenTelemetry Rust.** `main` at [`f61467c`](https://github.com/open-telemetry/opentelemetry-rust/tree/f61467c7854196220cd546eb0367f62f889f3ad7). Issues [#2602](https://github.com/open-telemetry/opentelemetry-rust/issues/2602) and [#3061](https://github.com/open-telemetry/opentelemetry-rust/issues/3061). PRs [#3763](https://github.com/open-telemetry/opentelemetry-rust/pull/3763) (base `8d81bb7`, head `e8bf3b7`) and [#3512](https://github.com/open-telemetry/opentelemetry-rust/pull/3512). The label `S-blocked-spec`. Releases from crates.io's version list for `opentelemetry-proto`.
- **opentelemetry-rust-contrib.** `main` at [`d98a3b6`](https://github.com/open-telemetry/opentelemetry-rust-contrib/tree/d98a3b60f6a60cfc00232046cf74f36825ed095e): README, CONTRIBUTING, `.github/CODEOWNERS` and `.github/component_owners.yml`. Issues #609 and #643, and PRs #21, #43, #49, #203, #248, #487, #562, #636, #637 and #653.
- **Specifications.**
  - OpenTelemetry specification at [`e30cae5`](https://github.com/open-telemetry/opentelemetry-specification/tree/e30cae5ce80e7bf9a8f051f20018b764d4bac029): `specification/protocol/file-exporter.md` and its history, `spec-compliance-matrix.md`, and #3817, #4183, #4940, #5243, #5354 and #5355.
  - OTLP at [`0976993`](https://github.com/open-telemetry/opentelemetry-proto/tree/09769935deb8b2ebcf73ad1e2142b691d17bf70b): `docs/specification.md`, `README.md`, `examples/`, and the logs, trace, common, resource and service protos with their commit history since 2024-10-21.
  - Configuration schema at [`52fdbb7`](https://github.com/open-telemetry/opentelemetry-configuration/tree/52fdbb78cfd5b32f2cf312d6549a5f4e9640183c).
  - [Protobuf JSON mapping](https://protobuf.dev/programming-guides/json/).
- **Collector.** [`receiver/otlpjsonfilereceiver`](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/34c2da6a9b67641bbf2e36250833fa773886439b/receiver/otlpjsonfilereceiver) (`README.md`, `metadata.yaml`, `file.go`), and pdata at [`3123b61`](https://github.com/open-telemetry/opentelemetry-collector/tree/3123b61db5448b8612d2da371169e5e40d24f1c5).
- **Other languages.** PyPI's JSON record for `opentelemetry-exporter-otlp-json-file`, and the `exporter/` folder of `open-telemetry/opentelemetry-python`. The `exporters/logging-otlp` folder of `open-telemetry/opentelemetry-java`.
- **Crates.** crates.io API records and owners for every crate in §3.1. Sources of `otlp-file-exporter` 0.2.0, `apollo-opentelemetry-stdout` 0.4.0, `otlp-stdout-span-exporter` 0.17.1, `hopf-otel` 0.5.0, `zerodds-observability-otlp` 1.0.0-rc.6, `joy-telemetry` 0.24.0 and `opai-agents-observability` 0.1.0. Also `opentelemetry-proto` 0.33.1, read for the 2026-10-20 research. Versions and MSRVs for serde, serde_json, itoa, ryu, memchr and zmij.
- **Naming.** OpenTelemetry's marketing guidelines and the Registry page above, and the [community repository](https://github.com/open-telemetry/community/tree/9c30b48354004ef818bcf3661ece2b32ca4dd787)'s membership guide. The Linux Foundation's [trademark usage guidelines](https://www.linuxfoundation.org/legal/trademark-usage) and [trademark list](https://www.linuxfoundation.org/legal/trademarks). crates.io API lookups of each candidate name.
- **Durability.** The Linux man pages `write(2)` and `fsync(2)`, on this host.
