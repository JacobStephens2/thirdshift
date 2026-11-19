# Security Harness, Model, and Effort

Research date: 2026-11-19.

**Question.** Which Harness, Model, and Effort should thirdshift use for Security work across the fleet? The setting is `security.harness` in each host's `~/.thirdshift/config.toml`, with `security.routes` as the later Plan-usage walk. That one choice drives the Security review a Run makes of its own change and the scheduled Security runs (`thirdshift secure`), which audit a repository, reproduce findings, record private advisories, and dispatch fix Tickets. The owner expects Muse Code's Wonder Full model to be stronger than Grok for that work. This note tests that expectation against fleet outcomes, the way thirdshift invokes each Harness, Plan usage, and vendor primary sources.

**Method.**

- **Vocabulary.** Searched the issues-worktree `GLOSSARY.md`, `docs/agents/domain.md`, `docs/code-navigation.md`, and `docs/adr/` for Security review, Security run, Harness, Model, Effort, Plan usage, and Route. `docs/agents/domain.md` has no matches. `docs/code-navigation.md` points Security audit, Security review, and Security run at `src/security/` and the process files. The issues worktree is `68d9fbdb` (`v0.27.0-30-g68d9fbdb`, `Cargo.toml` 0.27.0) and its glossary has no Plan usage or `security.routes` entry. Deployed terms below come from the main checkout `GLOSSARY.md` at `e137ab54` (`v0.37.0-7-ge137ab54`, 2026-11-18): Harness, Model, Effort, Plan usage, Plan window, Security review, and Security run. The fleet binary on every host at tally time was thirdshift 0.39.0 (`gh release view` tag `v0.39.0`, published 2026-11-19T14:06:06Z). `src/plan_usage/router.rs`, `src/asks.rs`, and `src/harness/grok.rs` on `e137ab54` have the same git blob ids as the `v0.39.0` tag. `docs/guide.md` and `src/prompt.rs` on that checkout do not (guide `7ff1cb2a…` locally, `04d61f9e…` on the tag). Guide sentences below are from the checkout file that was read.
- **Fleet, read-only, 2026-11-19T11:04-05:00.** A Python tally over SSH (`jacob@thirdshift.app` and `jacob@stephens.page` via `bash -lc`; `jstephens@{dev,orchestrate,test,factory,preview}` directly; Lima via `limactl shell thirdshift`) read each host's `~/.thirdshift/config.toml` Harness and Security sections, `crontab -l` lines that mention thirdshift, `thirdshift plan-usage --local --json`, and `~/.thirdshift/logs/<owner>/<repo>/`. Counts below are my tally from that run. The script prints counts, Harness, Model, Effort, exit class, token percentiles, and file sizes. It does not print prompt bodies, advisory text, or secret values. Plan-usage identity fields were dropped, and nested utilization objects were replaced with an ellipsis, so this note has no utilization percents from the 11:04 read. `mdfind -name fleet-harness-choice.md` on this Mac returned no path. A later same-day read of `[harness.codex]` on all eight hosts, plus the full `[security]` and Harness sections on orchestrate and preview, is marked where it corrects the 11:04 Codex model. An orchestrate recount of `security-review: on` lines matched this tally: 47 Grok, 16 agy `gemini-4-hydrogen-xhigh`, 1 Codex.
- **Source.** `src/plan_usage/router.rs`, `src/plan_usage/route.rs`, `src/asks.rs`, `src/config.rs`, `src/harness/grok.rs`, `src/harness/grok/catalog.rs`, `src/prompt.rs`, `docs/guide.md`, ADR 0021, and ADR 0031, as cited.
- **Vendor pages fetched 2026-11-19.** Meta's choose-a-model, Muse Wonder, and Muse Code pages; xAI's Grok 4.7 model page, models index, and 2026-09-21 Grok 4.7 release note; OpenAI's GPT-6.1 Sol and GPT-6.2 Sol model pages. Local CLIs the same day: Muse Code 1.4.24 (1.4.24-R9131.1), `grok` 1.0.89 (28be5bb1bf65). A `muse serve` `model/list` after `initialize`, with `"jsonrpc":"2.0"`, from the main checkout. `muse schema generate-json-schema --experimental` from that same binary. No harness `exec`, no `usage/refresh`, and no `thirdshift secure`.
- **Later Muse session files.** `*.models.json` rows whose harness or model string contained `muse`, on factory, dev, orchestrate, and stephens.page. Counts and filenames only.
- **Prior project notes**, cited for what they measured.

**Out of scope.** Changing any host config, crontab, label, or Security session. Quoting advisory text, finding titles, or reproduction steps. Third-party leaderboards. Discovering a Muse model id by reading `~/.muse`.

## TL;DR

Keep Security on Grok Build, model `grok-4.7`, for the first session. On stephens.page that session is already `grok-4.7` at Effort `xhigh`. On the other seven hosts `[harness.grok]` leaves Model and Effort blank, the session omits `--reasoning-effort`, and the Security sessions in the logs observed `grok-4.7`.

Add explicit `security.routes` so a plan-limited or unstartable Security session walks agy at `claude-opus-5-5-high`, then Codex `gpt-6.2-sol` at `xhigh` with a 95% ceiling, then Muse `muse-wonder-full-1.0` at `max`. preview has no Muse route and its 11:04 Plan-usage view listed Codex, Grok, and Claude Code, so its second and third routes are Codex, then Claude Code `claude-opus-5-5` at `high`. The 11:04 logs and Plan-usage read still show Codex as `gpt-6.1-sol`. The route below uses `gpt-6.2-sol` because that is the model in `[harness.codex]` after 11:07.

Wonder Full's Muse Code catalog id is `muse-wonder-full-1.0`. `model/list` on this Mac returned that row, with catalog default Effort `max`. The catalog's default model is `muse-wonder-1.0`, not Wonder Full. Wonder Full stays off the first Security session. The 11:04 Security rows that used Muse observed `muse-spark-1.3`, and the later session-file read found no Security kind that observed `muse-wonder-full-1.0`. On the seven hosts that have Muse, the last `security.routes` entry names that model at Effort `max`. preview's list has no Muse entry. Issue #2306 is open: Plan usage still cannot see a Muse window filling up, which is why that entry is last.

The quality comparison the owner asked for is too thin to rank Wonder Full against `grok-4.7`. The call is reliability, observability, and headroom.

## What `security.harness` and `security.routes` do

A **Security review** is the guidance session a Ticket run starts after its opening session, or a Spec run starts on the Spec PR after the Spec review. A **Security run** is `thirdshift secure`: a Security audit of the Base branch, a reproduction session per Security finding, then optional fix Tickets and close checks (`GLOSSARY.md` in the main checkout; `docs/guide.md` Security review and Security runs).

`security.harness` chooses that work's Harness. Model and Effort for the first session come from the shared `[harness.<name>]` section. Command `harness`, `model`, and `effort` words choose every other session. They do not change the review. A blank `security.harness` makes the review follow the Command's Harness, and a Security run with no harness word follows `harness.routes` the way any Command does (`docs/guide.md` around the Security run Harness paragraph and the Security review paragraphs; `Flags::security_harness` and `Flags::security_review_harness` in `src/asks.rs`).

`security.routes` is the list tried after `security.harness`. An entry is a Harness name, or a table with `harness` and any of `model`, `effort`, `max_utilization`, and `allow_credits`. A blank model or effort on a route inherits `[harness.<name>]`. `allow_credits` defaults to false. When `security.routes` is empty, a plan-limited `security.harness`, or one that cannot start before its first turn, walks `harness.routes`, and with that empty too, `harness.default` (`docs/guide.md` "Routing by Plan usage"; `src/config.rs` key `("security", "routes")`; `Route::choice` in `src/plan_usage/route.rs`).

`Router::of` in `src/plan_usage/router.rs` builds the Security walk when the review's `chosen_by` is `SecurityConfig` or `Routes`. A `SecurityConfig` choice is pushed first. Non-empty `security.routes` are appended after it. An empty list appends `harness.routes`, or else `harness.default`. A later route that runs as an earlier one (same Harness, Model, and Effort) is dropped. The test `security_sessions_walk_security_routes_else_harness_routes_else_the_default` expects `[Codex, Muse, Agy]` when the Security choice is Codex and `security.routes` is Muse then agy. A command that names a Harness stays pinned: that test expects an empty route list for a `ChosenBy::Command` Grok choice. `security_harness` sets `ChosenBy::SecurityConfig` only when the command itself named no Harness (`src/asks.rs`).

ADR 0021 added `security.harness` after Spec run #776, where Codex refused the Security review. ADR 0031 amends that ADR: Security reviews and Security runs walk `[security] routes`, falling back through `[harness] routes`, per Step, from machine-local Plan usage. A cybersecurity safeguard refusal still does not route and still holds Self-merge. Recommending `security.routes` follows ADR 0031. `security.harness` remains the first Security choice, as ADR 0021 requires.

With today's empty `security.routes`, an unpinned Security session therefore starts on `security.harness` (Grok, using `[harness.grok]`) and, on a Plan-usage limit or a failure before the first turn, continues through the implementer routes: Codex at a 95% ceiling, then Muse, then Grok again (dropped when Model and Effort match the first try), then agy. preview's implementer routes are Codex, Grok, Claude Code, so its implicit Security walk is Grok, then Codex at 95%, then Grok, then Claude Code.

A non-default review Harness that fails preflight falls back to `harness.default`. One that passes preflight and returns a Harness error before any turns or tokens is retried once as `security-review-fallback` on the default Harness. A failure on the default, a model refusal, or a failure after work starts is not retried that way. With routes configured, a Plan-usage limit moves along `security.routes` (`docs/guide.md` Security review paragraphs).

## Fleet settings at 2026-11-19T11:04-05:00

Every host reported thirdshift 0.39.0. My tally printed `[harness]`, `[harness.*]`, and `[security]` only.

| Host | `harness.default` and `routes` | `[security] harness` | `[harness.grok]` | `[harness.muse]` | `[harness.agy]` | `[harness.codex]` |
|---|---|---|---|---|---|---|
| thirdshift.app, dev, orchestrate, test, factory, Lima | `codex`, then `{ harness = "codex", max_utilization = 95 }`, `muse`, `grok`, `agy` | `grok` | model and effort blank | `muse-wonder-full-1.0`, effort blank | `claude-opus-5-5-high`, effort blank | `gpt-6.1-sol` at `xhigh`, except dev `high` and test both blank |
| stephens.page | same seven-host routes | `grok` | `grok-4.7`, `xhigh` | `muse-wonder-full-1.0`, effort blank | `claude-opus-5-5-high`, effort blank | `gpt-6.1-sol`, `xhigh` |
| preview | `codex`, then Codex at 95%, `grok`, `claude` | `grok` | blank | blank | `claude-opus-5-5-high`, effort blank | `gpt-6.1-sol`, `xhigh` |

No host's printed `[security]` section contained a `routes` key. Claude Code's model is `claude-opus-5-5` except Lima `opus`. Effort is medium on thirdshift.app, dev, and test; max on stephens.page; high on orchestrate, factory, and preview; blank on Lima. Factory and preview also set `[harness.kimi] model = "kimi-k3.1-pro"`, which is not a route. thirdshift.app's printed `[security]` has `fix` and `review` and no `close` line. The other hosts print `close = true`.

The Muse model string is a config comment ("Muse Wonder Full for every muse session") plus the value. It is the string in the file. Catalog verification is in the Wonder Full section.

`crontab -l` on each host: every `thirdshift secure` line has no `harness` word, so those Security runs take `security.harness` and the walk above. Implementer pins, which stay pinned and do not walk `security.routes`: orchestrate's Pickup and Triage of `/srv/orchestration` (`harness grok`), and Lima's wire-report Triage (`harness grok`). Some tourbot Pickup lines pass `no-security-review`. Those pins are context for implementer work.

Same-day backups `config.toml.bak-20261119T*` are present on each host. The newest on the seven Muse hosts is in the `T1052*` set. preview's newest in the tally is `config.toml.bak-20261119T104317`. The author's fleet note `~/.thirdshift/fleet/notes/2026-11-19.md` (lines 18, 26, and 28) records the 08:12 Security move to Grok, the 10:43 implementer order Codex then Muse then Grok then agy, and the 10:52 write of `[harness.muse] model = muse-wonder-full-1.0` on seven hosts. That file is the author's log. The 11:04 config read is the measurement.

Line 29 of the same note records an 11:07 write, after this tally: `[harness.codex] model` changed from `gpt-6.1-sol` to `gpt-6.2-sol` on all eight hosts, effort unchanged. A later read the same day of `[harness.codex]` on thirdshift.app, stephens.page, dev, orchestrate, test, factory, preview, and Lima showed `model = "gpt-6.2-sol"`. Effort was `xhigh` on six of those, `high` on dev, and blank on test. The full `[security]` section on orchestrate and on preview still had `harness = "grok"` and no `routes` key. preview's `[harness.claude]` was still `claude-opus-5-5` at `high`, and its `[harness.agy]` was still `claude-opus-5-5-high`. The outcome tables above describe sessions from the 11:04 logs, whose Codex rows request `gpt-6.1-sol`.

## Fleet Security outcomes

My tally, logs under `~/.thirdshift/logs/<owner>/<repo>/`, hosts and timestamps above. Activity "started" lines are `Security run started: …, on <harness> · <model> · <effort>`. Blank Model and Effort render as `default model` and `default effort` (`Choice`'s `Display` in `src/harness.rs`). Activity "ended" classes are the tally's first matching bucket (`merged`, `ready`, `exited <harness> <code>`, `final_line`, `interrupted`, `refused`, `quota`, `sandbox`, `recorded`, or `other` by length only). A line can carry more than one idea; the bucket is the first regex hit. `final_line` matches "omitted a valid final line", "ended early", or "ended without a final line", and the tally does not date those lines against the 09:11 deploy of 0.39.0, so they stay undated. `*.models.json` rows are `kind`, requested Model, Effort, and observed models, grouped by a `20YYMM` stamp in the filename. Command-log "on" lines are `security-review: on <harness> · <model> · <effort>`. Two "on" rows on thirdshift.app matched prose inside a log (`docs/guide.md` text). They are 1 each and are omitted below. Token percentiles are my tally's nearest-rank index on summaries that matched `<n> input tokens` and `<n> output tokens`. `nonusage_summary` counts session-ended lines that did not match. Reproduction counts are full matches of `reproduced <severity> <size>` or the exact line `not reproduced` (`Outcome` display in `src/security/reproduction.rs`). `REPRODUCTION_OTHER` was 0 on every host.

### Security runs (activity.log)

| Host | Repos with a Security start | Starts | Ends (bucket) | Skip lines |
|---|---:|---|---|---:|
| thirdshift.app | 16 | grok default/default 26; muse default/default 13; agy `gemini-4-hydrogen-xhigh` 9; codex `gpt-6.1-sol` xhigh 4; claude `claude-opus-5-5` medium 3; codex high 1 | merged 31; ready 10; exited codex 1: 5; other 3; final_line 2; exited muse 1: 2; interrupted 1; refused 1 | 91 |
| stephens.page | 16 | grok `grok-4.7` xhigh 110; agy hydrogen 9; claude `claude-opus-5-5` max 8 | merged 102; exited grok 1: 7; final_line 6; interrupted 3; other 3; ready 2; sandbox 1; quota 1; refused 1; recorded 1 | 347 |
| dev | 1 | grok default/default 15; agy hydrogen 1 | ready 8; exited grok 1: 5; other 1; interrupted 1; quota 1 | 23 |
| orchestrate | 3 | grok default/default 8; codex `gpt-6.1-sol` xhigh 5 | exited codex 1: 5; ready 5; interrupted 2; exited grok 1: 1 | 21 |
| test | 0 | none | none | 0 |
| factory | 0 | none | none | 0 |
| preview | 0 | none | none | 0 |
| Lima | 1 | claude `opus` default effort 2 | recorded 1; ready 1 | 4 |

Grok is the Harness with the large completed Security-run body. stephens.page recorded 110 Grok starts at `grok-4.7` / xhigh, and 102 merged end-buckets across every Harness on that host (127 starts in total). thirdshift.app recorded 26 default-Grok starts, and 31 merged end-buckets across every Harness. Codex Security runs show up as exit 1 in the activity buckets (5 on thirdshift.app, 5 on orchestrate). Muse Security runs show up as 13 starts on thirdshift.app and 2 exit-1 buckets there. Those Muse starts are the activity display "default model", which the models files below resolve to `muse-spark-1.3`.

### Security reviews and other Security sessions

Command-log "on" lines, security-review only:

| Host | security-review "on" lines |
|---|---|
| thirdshift.app | grok default/default 128; muse default/default 68; agy hydrogen 14; claude medium 5; review-fallback codex xhigh 3 |
| stephens.page | grok `grok-4.7` xhigh 211; agy hydrogen 65; muse default/default 5; claude max 4; review-fallback codex xhigh 2 |
| dev | grok default/default 88; agy hydrogen 6; claude medium 2 |
| orchestrate | grok default/default 47; agy hydrogen 16; codex xhigh 1 |
| factory | agy hydrogen 92; grok default/default 30; claude high 4; review-fallback codex xhigh 2 |
| preview | grok default/default 11; claude high 1 |
| Lima | claude `opus` default effort 29; agy default/default 2; grok default/default 2 |
| test | no "on" lines; 7 session-ended security-review lines, all without a token summary |

`*.models.json` files whose kind contains "security": thirdshift.app 494, stephens.page 844, dev 274, orchestrate 272, Lima 171, factory 147, preview 60, test 7. Parse errors: 0. A search of the tally text for `muse-wonder`, `muse-ember`, and `claude-opus-5-5-high` found those strings in config and plan-usage lines. No `security-review` or other `security-*` models row contained them.

Muse rows that did appear, all observed `muse-spark-1.3` unless noted: thirdshift.app 202610 security-review 71, security-audit 9, fix-publishing 3, reproduction-1 1, plus 1 audit with no observed model. stephens.page 202610 security-review 21. No 202611 Muse Security models row appeared in the tally's printed top rows, and the wonder/spark/ember search found no 202611 spark row.

Grok security-review rows are mostly observed `grok-4.7`. Examples from the printed top rows: thirdshift.app 202611 requested None, effort None, observed `grok-4.7`: 91, plus 1 observed `grok-4.6` and 3 observed none; 202610 observed `grok-4.7`: 46. stephens.page 202610 `grok-4.7` / xhigh observed `grok-4.7`: 174; 202611 the same: 168, plus 2 observed none. dev 202611 requested None, observed `grok-4.7`: 117, plus 1 `grok-4.6`. The same pattern holds for Grok audit, reproduction, fix-publishing, and close-check rows in those files: requested effort None where `[harness.grok]` is blank, and `xhigh` on stephens.page.

agy Security rows in the printed models lists are `gemini-4-hydrogen-xhigh` (thirdshift.app 202611 reviews 26; stephens.page 202611 reviews 74; orchestrate 202611 reviews 49; factory reviews 95 in the 11:04 models list's head). Lima has 4 agy security-review files with requested None and no observed model. The configured agy model `claude-opus-5-5-high` has no Security models row in this tally. It was written at about 08:15 (`fleet/notes/2026-11-19.md` line 19, and the config comments). Sessions after that write may still be inside the hydrogen counts if they started earlier; the tally does not split the hydrogen rows by hour.

Codex security-review rows request `gpt-6.1-sol` at high (older) or xhigh, and the printed rows show no observed model (`observed=-`). Claude Code rows observe `claude-opus-5-5` when the request names that model or `opus`.

### Reproduction outcomes and private-file sizes

Reproduction lines are the score a reproduction session printed. They are not a quality ranking across Harnesses, because the tally does not join each line to the Harness that produced it. They do show that completed reproduction sessions exist in volume on the hosts where Grok has been the Security Harness.

| Host | Reproduction outcomes (my tally) |
|---|---|
| stephens.page | medium single 88; not reproduced 28; high single 19; low single 11; medium spec 4; informational single 1; low spec 1 |
| dev | high single 52; medium single 49; high spec 5; critical single 5; low single 5; medium spec 3; informational single 2; not reproduced 2; critical spec 1 |
| thirdshift.app | not reproduced 28; medium single 24; high single 6; low single 4; medium spec 3; low spec 1 |
| orchestrate | medium single 12; high single 10; high spec 3; not reproduced 3; low single 2; informational single 1 |
| Lima | high single 2; not reproduced 1; medium single 1 |
| factory, preview, test | none |

Private report files, counts and sizes only. `security-reviews/` is the review's private report directory (`docs/guide.md`). `audits/` is the Security audit output directory. The 131072-byte mark is the tally's bucket, equal to 32 times a 4096-byte page. `PAGE_SIZE` was not read on the hosts.

| Host | security-reviews files | over 131072 | max | audit files | over 131072 | max |
|---|---:|---:|---:|---:|---:|---:|
| thirdshift.app | 324 | 0 | 32678 | 611 | 44 | 3344651 |
| stephens.page | 3003 | 16 | 298724 | 1102 | 32 | 548417 |
| dev | 138 | 0 | 59788 | 1129 | 26 | 852865 |
| orchestrate | 225 | 0 | 73388 | 749 | 45 | 725840 |
| factory | 143 | 1 | 132175 | 0 | 0 | 0 |
| Lima | 165 | 0 | 8380 | 60 | 0 | 91426 |
| preview | 50 | 0 | 9557 | 0 | 0 | 0 |
| test | 7 | 0 | 4031 | 0 | 0 | 0 |

### Tokens, where the summary matched

These are session-ended lines with a token summary, so they under-count sessions. stephens.page reproduction ended lines were 155, all `nonusage_summary`. dev reproduction ended lines were 124, all `nonusage_summary`.

| Host | security-review input p50 / max (n) | security-review output p50 / max |
|---|---|---|
| thirdshift.app | 1296007 / 11521721 (163) | 10347 / 183288 |
| stephens.page | 650372 / 5905648 (132) | 59154 / 176691 |
| orchestrate | 649373 / 69567935 (86) | 80359 / 168976 |
| factory | 516468 / 1703675 (97) | 117306 / 318306 |
| dev | 549116 / 3155604 (11) | 111208 / 178693 |
| Lima | 594215 / 2488139 (10) | 12866 / 53879 |
| preview | 2203512 / 3107484 (4) | 15029 / 16231 |

Audit input on orchestrate, where five summaries matched: p50 23716015, max 27085361. thirdshift.app audit input n=13, p50 5057676, max 15794662. Several audit summaries on dev and stephens.page were 0.

## Wonder Full

Meta's choose-a-model page (fetched 2026-11-19, <https://dev.meta.ai/docs/choose-a-model>) lists Muse Wonder as `muse-wonder-1.0`, Muse Spark as `muse-spark-1.3` (recommended), and Muse Ember as `muse-ember-1.0`. It says Muse Wonder Full is the higher-capability variant of Muse Wonder, available only in Muse Code, and that no model id on the Model API selects it. The Muse Wonder page (<https://dev.meta.ai/docs/muse-wonder>) repeats that: Muse Wonder Full is in Muse Code on the Power Usage and Maximum Usage plans; `"max"` on `muse-wonder-1.0` does not select it. The page publishes one model id, `muse-wonder-1.0`. The Muse Code page (<https://dev.meta.ai/docs/muse-code>) says the first-run default model is `muse-spark-1.2`. None of these pages states a Muse Code model id for Wonder Full.

On this Mac, 2026-11-19, `muse --version` printed Muse Code 1.4.24 (1.4.24-R9131.1). `muse model-profile show muse-wonder-full-1.0 --effort max` and `muse model-profile show muse-not-a-real-model-xyz --effort max` both exited 0 and printed a profile, including `base_instructions_variant full` for the made-up id. `docs/research/harness-candidates.md` (2026-10-05) already recorded that `show` accepts any string. That command does not identify a catalog id.

`muse models` was not run (it is interactive). A later `muse serve` on this Mac, cwd the main checkout `/Users/jacob/GitHub/JacobStephens2/thirdshift`, sent `initialize` and then `model/list`, each with `"jsonrpc":"2.0"`. `model/list` returned `source` `providerCatalog`, provider `meta`, and five rows. The same binary's `muse schema generate-json-schema --experimental` describes `model/list` as the catalog of models the host will accept in `session/setModel`, newest `releaseDate` first. The rows were:

| modelId | Default row | Context | Output | Default effort | Efforts |
|---|---|---:|---:|---|---|
| `muse-wonder-1.0` | yes | 1007997 | 128000 | `max` | `minimal` through `max` |
| `muse-wonder-full-1.0` | no | 1007997 | 128000 | `max` | `minimal` through `max` |
| `muse-ember-1.0` | no | 1007997 | 128000 | `max` | `minimal` through `max` |
| `muse-spark-1.3` | no | 1007997 | 128000 | `max` | `minimal` through `max` |
| `muse-spark-1.2` | no | 1007997 | 128000 | `xhigh` | `minimal` through `xhigh` |

`muse-wonder-full-1.0` is the Muse Code catalog id. The Model API pages above still publish no API id for Wonder Full. The fleet note at 11:00 records the same id from the same method. The 11:04 configs contain that string on seven hosts.

`Route::choice` in `src/plan_usage/route.rs` uses a route's own model and effort when they are set, and otherwise the `[harness.<name>]` values. `parse_route` accepts `harness`, `model`, `effort`, `max_utilization`, and `allow_credits`. A Security route may be `{ harness = "muse", model = "muse-wonder-full-1.0", effort = "max" }`. `Muse::session` in `src/harness/muse.rs` passes `--model` and `--reasoning-effort` only when the choice has them. Wonder Full's catalog default effort is `max`. A blank effort omits the flag, so the recommended route sets `max` on the route itself.

Later `*.models.json` reads, requested model null and effort null in every row below:

- dev, three files, kinds repair and implement: `fix-20261119T070032-0500~pjbdlchdKQPvB7rl-repair-1.models.json`, the matching `repair-2`, and `fix-20261119T080032-0500~Cj5MsQnNELuPUOS0-implement.models.json`. Each observed `muse-wonder-full-1.0`. These stamps are before the 10:52 config write. None is a Security kind.
- factory, six implement and repair files at `20261119T063721`, `T070721`, and `T080721`. Each observed `muse-wonder-1.0`.
- stephens.page, 57 Muse rows: 56 observed `muse-spark-1.3` in `202610` filenames, and one implement row with an empty observed list. No row carried `wonder`, and no filename matched `20261119T1`.
- orchestrate, 2 Muse rows. Neither filename matched `20261119T1`, and neither requested or observed string contained `wonder`.

No Security kind in these reads observed `muse-wonder-full-1.0`. The 11:04 tally's Muse Security rows remain `muse-spark-1.3`. The owner's interactive lead `muse-ember-1.0` does not appear in any Security models row here. Hosts did not share one unattended Muse model on the morning of 2026-11-19: dev's blank-request sessions observed Wonder Full, and factory's observed `muse-wonder-1.0`, which is the default row in the Mac catalog above.

Issue #2306 is open: <https://github.com/JacobStephens2/thirdshift/issues/2306>. Title: Plan usage never observes muse. The issue states that thirdshift 0.38.0 sent `muse serve` requests without `"jsonrpc":"2.0"`, that Muse Code 1.4.24 rejected them, and that with the field added `usage/read` returned `{"result":{}}` with no usage object. The regenerated schema says `usage/read` reads the last-observed window without a model call and omits `usage` when nothing has been observed. `usage/refresh` is experimental: one fresh read, a key mint, at most one per host per 60 seconds. This pass did not call `usage/refresh`. `src/plan_usage/read.rs` still documents `rpc` as sending requests without the `jsonrpc` member, and the call it sends is `usage/read`. An empty result matches the schema until Muse has observed usage, and thirdshift does not call the refresh method. A Muse route can be chosen while the window is full, and the session is then refused. The 11:04 plan-usage JSON listed Muse as inactive, model `muse-wonder-full-1.0`, effort null, on the seven hosts that set it. Dev and factory printed a cooldown key on that Muse object. No utilization percent was in the scrubbed JSON.

## How Grok is invoked, and the preview exit

`Grok::session` in `src/harness/grok.rs` (blob matches `v0.39.0`) appends `-p` and the prompt string as one argument. It passes `-m` only when the choice has a Model, and `--reasoning-effort` only when the choice has an Effort. `Catalog::settle` in `src/harness/grok/catalog.rs` leaves a missing Model and a missing Effort as `None`. A blank `[harness.grok]` therefore launches Grok Build's own default Model and omits the effort flag. On this Mac, `grok models` (2026-11-19) printed default model `grok-4.7`, with `grok-4.7`, `grok-4.7-build-fast`, `grok-4.6`, and `grok-4.5` available. The fleet models files agree: blank-config Security sessions observed `grok-4.7`, with a few `grok-4.6`.

The Security review prompt in `src/prompt.rs` at `e137ab54` (`security_review_prompt`) tells the session to use the `thirdshift-security-audit` skill in guidance mode and to `git diff` the branch. My count of that format-string body, before interpolation, is 3052 bytes. `src/prompt.rs` on the `v0.39.0` tag is a different blob; PR #2301 (merged 2026-11-19T13:17:25Z, <https://github.com/JacobStephens2/thirdshift/pull/2301>) adds an example object to the prompt. The checkout skill file `skills/thirdshift-security-audit/SKILL.md` is 22039 bytes. The prompt names that skill, and those bytes stay in the skill file. A guidance review's `-p` value is a short template plus the issue URL, branch names, and the usual headless instructions.

`security_reproduction` in the same file embeds the recorded finding after the instruction template. Audit, fix-publishing, and close-check sessions can carry the private record as well. The file-size table is the evidence that some private audit artifacts, and a minority of review report files, are larger than 131072 bytes. Those sizes are files on disk. They are the reason a Grok reproduction or audit prompt can become one argument large enough to matter on Linux. This pass did not measure a live argv.

The preview spans file named in the task exists. My tally: `~/.thirdshift/logs/JacobStephens2/thirdshift/commands/pickup/2306-20261119T081901-0500~O2G3vEfg5vtFbWmd-security-review.spans.jsonl`, 15089 bytes, 1 line, phrase `grok exited 2`, and 0 JSON string literals longer than 200 characters. That exit is a one-second-scale failure in the task's account; the spans file does not show a long prompt argument. It was not re-run.

PR #2301 is the report-shape fix. Its body says the parser still rejects top-level `severity` and `fix_size` (`deny_unknown_fields`), and that before the fix `Outcome` hid `InvalidReport` behind "session failed, ended early or omitted a valid final line". The fleet notes say all eight hosts moved to 0.39.0 at 09:11. agy Security failures with that generic wording from before 0.39.0 are that bug. The undated `final_line` activity buckets (stephens.page 6, thirdshift.app 2) are not split here into before and after that release.

## Plan usage at 11:04

`thirdshift plan-usage --local --json` succeeded on every host (`PLAN_USAGE rc=0`). The scrubbed objects:

- Codex was the active route everywhere, plan name `pro` where printed. The active route's `max_utilization` was 95. Model `gpt-6.1-sol`. Effort `xhigh`, except dev `high`. test's config leaves Codex model and effort blank; the scrubbed test lines were not in the effort sample above. Primary window `resets_at` `2026-11-26T13:19:36Z` on thirdshift.app and `2026-11-26T13:19:35Z` on factory, preview, and Lima.
- Grok was inactive. stephens.page's Grok route object was model `grok-4.7`, effort `xhigh`. thirdshift.app, factory, preview, and Lima printed Grok model null and effort null, matching a blank `[harness.grok]`.
- agy was inactive, model `claude-opus-5-5-high`, on the hosts whose plan objects included it. A cooldown key was printed for agy on dev, orchestrate, and Lima. preview's plan objects listed Codex, Grok, and Claude Code.
- Claude Code plan name `max`. `seven_day` `resets_at` `2026-11-21T02:00:00Z` with status `allowed_warning` on thirdshift.app, stephens.page, dev, factory, preview, and Lima, and `allowed` on orchestrate. test printed plan `max` and no `seven_day` line in the scrubbed JSON.
- Muse, as above, inactive and unobserved as a percentage.

The 08:15 fleet note (line 18) records an earlier snapshot in the author's words: Codex 0% after a redeemed reset until 11-26 08:13, agy Gemini weekly about 10.7% left, agy Claude/GPT group 99% left, Grok about 1% weekly until 11-23, and Muse at 103% of a 5-hour window. Those percents were not remeasured at 11:04. The same note says Muse was upgraded to Maximum usage at 10:43 (line 26). ADR 0031 calls Claude Code, agy, and Grok passive for Plan usage: they learn exhaustion from a rejection around the first turn, because they have no non-billed read that routing can poll. Codex is the Harness whose 11:04 read carried a reset time on a primary window.

## Vendor primary sources

Fetched 2026-11-19. Security-audit or security-review claims are included only where the vendor page says them.

- **Grok 4.7.** <https://docs.x.ai/developers/models/grok-4.7>: model name `grok-4.7`, context 500,000, reasoning efforts `low`, `medium`, `high`, `xhigh`, default `high`. The models index says the knowledge cut-off of Grok 4.7 is May 2026, and "For everything else, including code, use Grok 4.7." The 2026-09-21 release note <https://x.ai/news/grok-4-7> says Grok 4.7 "balances strong cyber defense capabilities with low refusal rates for legitimate use" and "shows the highest safety on HackerBench v0.3", described there as xAI's own benchmark, "allowing only 3.3% of risky dual-use prompts through while rarely blocking legitimate security work." It also says Grok 4.7 is available in Grok Build. That is a vendor release-note claim about cyber defense and legitimate security work. The API default effort `high` is the docs.x.ai statement. The installed `grok --help` names `--reasoning-effort` and the help lines matched here do not print a default. Blank `[harness.grok]` effort omits the flag, so the CLI's own default applies.
- **Muse.** The pages in the Wonder Full section describe Muse Wonder as Meta's most capable model for complex software engineering and long-horizon agentic work, Muse Spark for agentic and coding work at scale, and Muse Ember for high-volume sub-agent work. Context window on the comparison table is 1,048,576. Those fetched pages do not say that Muse Wonder or Muse Wonder Full performs security review or security audit. Wonder Full's Model API id is absent from them. The Muse Code catalog id is in the Wonder Full section.
- **Codex `gpt-6.1-sol`.** <https://developers.openai.com/api/docs/models/gpt-6.1-sol>: "Use GPT-6.1 Sol for complex coding, computer use, and professional work." `reasoning.effort` is `low`, `medium` (default), `high`, `xhigh`, and `max`. Context 1,050,000, max output 128,000, knowledge cutoff Apr 30, 2026. The page says Codex and ChatGPT Work use a default 500K context for this model. The fetched page does not claim a security-review or audit capability. This is the model the 11:04 Security rows and Plan-usage read requested.
- **Codex `gpt-6.2-sol`.** <https://developers.openai.com/api/docs/models/gpt-6.2-sol>, fetched the same day after the 11:07 config write: model id `gpt-6.2-sol`, "Our go-to model for everyday work, from coding to powering agents at scale." Context 1,050,000, maximum input 922,000, max output 128,000, knowledge cutoff Jun 01, 2026. The page says the API rejects reasoning effort `none`. The fetched page does not list the effort enum and does not claim a security-review or audit capability. This is the model `[harness.codex]` names after 11:07.
- **Claude Code and agy.** No official Claude or agy model card was fetched in this pass, so this note records no vendor security claim for `claude-opus-5-5` or `claude-opus-5-5-high`.

## What earlier project research measured

`docs/research/security-review-mode-trial.md` (2026-10-08, graded 2026-10-09) compared guidance with a full audit on PRs #529, #527, and #525, using Claude Code 2.1.292 `claude-opus-5-5` at medium and Codex 0.161.0 `gpt-6.1-sol` at xhigh. All 12 sessions completed. There were no seeded defects, and the note says detection was inconclusive. Guidance was adopted for the opt-in Security review. That trial did not run Grok, Muse, or agy.

`docs/research/harness-candidates.md` (2026-10-05) measured headless CLI mechanics for Muse 1.4.2, Grok 1.0.46, and agy 1.2.17. It records that `muse model-profile show` accepts an arbitrary string.

`docs/research/model-strength-for-review.md` (2026-10-05) measured the Artificial Analysis Intelligence Index and related boards. Those scores stay out of this note.

`docs/experiment-results.md` and `docs/eval-tasks.md` specify result and task formats. Harness comparison is outside those two format specs.

## Recommendation

Fleet outcomes come first. Grok, observed as `grok-4.7`, is the Harness that has completed Security runs, Security reviews, and reproduction sessions across the fleet. stephens.page has that record at Effort `xhigh`. The other hosts have it with Effort left to Grok Build. Muse's Security record is `muse-spark-1.3` from 202610 logs, on two hosts, and includes activity exit-1 buckets. Wonder Full's catalog id is verified, and dev's blank-request implement and repair sessions on the morning of 2026-11-19 observed it. No Security session in this tally or in the later session-file read observed it. Codex has completed some Security reviews and has several Security-run exit-1 buckets. agy's Security record in the logs is `gemini-4-hydrogen-xhigh`; the report-shape bug fixed in 0.39.0 accounts for generic final-line failures before that release; the model configured since 08:15 has not yet appeared in a Security models file.

Invocation comes next. Grok's review prompt is a short `-p` argument. Reproduction and audit prompts can embed private records, and audit files over 131072 bytes exist on four hosts, so a Grok argv limit remains a risk for those session kinds. The preview `grok exited 2` spans file is small and has no long string. Muse cannot be observed by Plan usage while #2306 is open.

Headroom comes next. Codex is the observable implementer primary, with a primary window resetting 2026-11-26, and it is already capped at 95% for implementer routes. Claude Code's seven-day window was `allowed_warning` on six hosts, resetting 2026-11-21T02:00:00Z. The author's morning note gave the agy Claude/GPT group much more room than the Gemini group; the 11:04 read did not repeat those percents. The configured agy model is the Claude model id.

Vendor docs come last. Grok 4.7's own release note talks about cyber defense and legitimate security work. The fetched Muse and GPT-6.1 Sol pages talk about software engineering and coding and do not make a security-audit claim.

### 1. TOML to apply

Paste into `~/.thirdshift/config.toml` on thirdshift.app, stephens.page, dev, orchestrate, test, factory, and Lima. This sets `security.harness` and `security.routes` only. It leaves `[harness.grok]`, `[harness.codex]`, `[harness.agy]`, and `[harness.muse]` as they are.

```toml
[security]
harness = "grok"
routes = [
  { harness = "agy", model = "claude-opus-5-5-high" },
  { harness = "codex", model = "gpt-6.2-sol", effort = "xhigh", max_utilization = 95 },
  { harness = "muse", model = "muse-wonder-full-1.0", effort = "max" },
]
```

preview, whose Plan-usage view listed Codex, Grok, and Claude Code:

```toml
[security]
harness = "grok"
routes = [
  { harness = "codex", model = "gpt-6.2-sol", effort = "xhigh", max_utilization = 95 },
  { harness = "claude", model = "claude-opus-5-5", effort = "high" },
]
```

The keys are the ones `parse_route` accepts (`src/plan_usage/route.rs`) and the guide's route table uses. The first Security session's Model and Effort stay whatever `[harness.grok]` says, because `security.harness` does not carry its own model key (`src/asks.rs` `security_harness`). On stephens.page that is `grok-4.7` at `xhigh`. On the other seven hosts both are blank: Grok Build's default Model, observed in these logs as `grok-4.7`, and no `--reasoning-effort` flag. Putting the same model and effort on a Grok entry in `security.routes` would be a second try, after the blank-effort first try, on every host except stephens.page, where it would match the first try and be dropped.

Unifying the first session onto `xhigh` means editing the shared `[harness.grok]` section. That section also applies to implementer sessions that fall through to Grok. This note does not ask for that edit.

### 2. Fallback routes, in order

Seven hosts, after the first Grok session:

1. agy, model `claude-opus-5-5-high`, effort inherited from `[harness.agy]` (blank on all eight, so agy's own effort). This is the model already configured for every agy session. It is the fallback with a fleet config and, in the author's morning note, Claude/GPT headroom. It has no Security models row yet.
2. Codex, `gpt-6.2-sol` at `xhigh`, ceiling 95%. The route's model and effort apply to this Security fallback. `[harness.codex]` stays the implementer setting, including dev's implementer effort `high` and test's blank effort. The explicit model is the 11:07 value. A later edit to `[harness.codex]` does not move this fallback, because a route model is its own. The 95% ceiling is the same number the implementer Codex route uses, so this fallback gives way at that utilization.
3. Muse, `muse-wonder-full-1.0` at `max`. The route names the catalog id and the catalog's default effort for that row. It does not edit `[harness.muse]`, which on these seven hosts already sets the same model and a blank effort. This entry is last because #2306 leaves the route looking available until a session is refused, and because no Security session has observed this model yet. With `security.routes` set, the walk no longer continues into `harness.routes`, so leaving Muse off the list would drop the Muse fallback the empty list has today.

preview, after the first Grok session, uses the two Harnesses in its routes and in its 11:04 Plan-usage view. That view listed Codex, Grok, and Claude Code, so this list has no Muse entry:

1. Codex, `gpt-6.2-sol` at `xhigh`, ceiling 95%.
2. Claude Code, `claude-opus-5-5` at `high`, matching preview's `[harness.claude]`. preview's 11:04 plan view had no agy object and no Muse object.

A `thirdshift secure` line with no harness word uses this walk. A command that already says `harness grok` stays pinned and does not walk `security.routes` (`Router::of`). The scheduled secure lines do not say that. The orchestrate and Lima `harness grok` lines are Pickup and Triage.

### 3. Risks

- The first session's Effort is the shared `[harness.grok]` value. Seven hosts omit it. stephens.page sets `xhigh`. Implementer sessions that land on Grok share that section.
- The Codex Security route spends the same Codex Plan window the implementer routes use, and it stops at 95%. A busy Security day still consumes that window up to the ceiling. The route pins `gpt-6.2-sol`. The 11:04 Security rows requested `gpt-6.1-sol`, and the model id changed once already on 2026-11-19.
- agy `claude-opus-5-5-high` is unmeasured on Security work. The logged agy Security sessions used `gemini-4-hydrogen-xhigh`. A cooldown key was present for agy on dev, orchestrate, and Lima at 11:04.
- Grok and agy are passive for Plan usage (ADR 0031). Routing learns they are exhausted when a session fails the structured check. Until #2306, Muse's read returns an empty result, so a Muse route stays selectable until a session is refused.
- Grok passes the whole prompt as one `-p` argument. Guidance reviews are small. Reproduction and audit prompts embed finding text, and audit files larger than 131072 bytes exist on thirdshift.app (44, max 3344651), stephens.page (32, max 548417), dev (26, max 852865), and orchestrate (45, max 725840). A Linux argument-length failure would show up on those session kinds.
- The preview `grok exited 2` on the #2306 Security review is unexplained by argument length. One later replay is mentioned in the fleet notes and was not repeated here.
- Claude Code on preview is `allowed_warning` until 2026-11-21T02:00:00Z. It is last in that host's list because it is the remaining Harness in the plan view.
- `muse model-profile show` still prints a profile for any string. The catalog id comes from `model/list`, not from `show`. `[harness.muse]` on the seven hosts already names `muse-wonder-full-1.0` for every Muse session, implementer sessions included. This note leaves that section in place. The Security route adds Effort `max`, which a blank `[harness.muse]` effort does not pass.
- Activity end buckets and token percentiles are my tally's classifications of log lines. They are not a hand-labeled quality grade, and reproduction counts are not joined to a Harness.

### 4. What evidence would change the call

- Unattended Security reviews and reproduction sessions whose `*.models.json` observed `muse-wonder-full-1.0`, with outcomes that can be set beside the `grok-4.7` record above. The catalog id is no longer the missing piece. That sample would move Muse earlier than last.
- A fix for #2306 such that `thirdshift plan-usage` shows Muse windows and routing skips a full window before starting a session. That would move the Muse route ahead of the Codex route, because the Codex window is shared with implementer work and the Muse plan was raised to Maximum usage on 2026-11-19.
- A paired sample: the same changes reviewed by `grok-4.7` and by a verified Wonder Full session, with private grading of the kind `docs/research/security-review-mode-trial.md` recorded, and seeded or previously known findings so an empty report is not treated as a pass.
- Security-run activity on agy `claude-opus-5-5-high` after 0.39.0, with reproduction outcomes and without the pre-0.39.0 generic final-line failure.
- A measured Grok argument-length failure on reproduction or audit prompts, which would move those session kinds off Grok even if reviews stay.
- A Codex or Claude Code window at its ceiling while Grok still has room, which would keep the order and would pause Security when every listed route is unavailable (ADR 0031). A Codex window with substantial room and a run of Grok exit-1 Security runs would move Codex earlier in `security.routes`.
