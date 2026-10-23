# Domain-to-code navigation

Start with the issue's terms in `GLOSSARY.md`, then use this index to find their owners. This is a navigation aid, not a second glossary. Search symbols with `rg -n` and read bounded ranges around the matches; widen to callers and tests when needed. Keep these pointers current when ownership moves.

| Issue term | Owning implementation / search symbol | Supporting tests |
| --- | --- | --- |
| Verdict, Bar, paired Scores | `src/verdict.rs` — `judge`; `src/verdict/prediction.rs` — posterior-predictive assurance | `src/verdict/tests.rs` (parent #977's standalone Verdict seam) |
| Claim | `src/claim.rs` — `Claim`, `make`, `end` | Inline tests; `tests/pickup_run.rs` |
| Ready issue, Claim limit, Sweep | `src/ready.rs` — `ReadyIssue`, `first`, `refresh`; `src/pickup.rs` — `gates`, `sweep`; `src/pickup/reservation.rs` — cross-machine Pickup reservation | Inline tests; `tests/pickup_run.rs` |
| Run, Continuation, Issue branch | `src/run.rs`; `src/branch.rs` | `tests/fresh_run.rs`, `tests/continuation.rs`, `tests/numbered_branches.rs` |
| Worktree | `src/worktree.rs` and `src/worktree/` | `src/worktree/acquisition_tests.rs`, `src/worktree/cleanup_tests.rs` |
| Spec run, Spec branch, Spec PR | `src/spec_run.rs` | `tests/spec_run.rs` |
| Ticket board, Tickets checklist, Unready Ticket, Startable Ticket | `src/spec_run/ticket_board.rs` — `TicketBoard`, `next_to_start`; `src/spec_run.rs` — `unstartable`, shared with `src/ready.rs`; `src/labels.rs` | Inline tests; `tests/spec_run.rs` |
| Delivery, Repair loop, Foreign commit | `src/delivery.rs` — `Delivery`; `src/delivery/repair_loop.rs`; Delivery stage: `src/delivery/stage.rs` — `Stage`, `Tracker`, its table to the `Stopped at:` and Command record stages | Inline tests; `tests/merge_run.rs` |
| Failed run | `src/failed_run.rs` | `tests/failed_run.rs` |
| Recovery | `src/recovery.rs` — `Recovery`, its add-a-fact methods and `lines` | Inline tests in `src/recovery.rs`; `tests/failed_run.rs`, `tests/spec_run.rs` |
| CI watch, Check re-run, Abandoned job, Declined CI fix | `src/ci.rs` — `Watch`, `assess`, `after_repair`; Delivery constructs one watch in `src/delivery.rs` — `RunOutside`; `src/delivery/repair_loop.rs` requests assessments and reports completed CI-fix Repairs | Inline CI assessment and Repair loop tests; `src/github/execution_tests.rs`; `tests/ci.rs` |
| Inherited failure, check-state reading, Base fix | `src/ci.rs` — `reading` of each `Conclusion` the GitHub adapter (`src/github.rs`, `checks_on`) reports; `src/base_fix.rs` | Inline tests; `src/github/execution_tests.rs`; `tests/ci.rs`, `tests/base_fix.rs` |
| Run notification | `src/notification.rs` — `RunNotification`; `src/email.rs` | `tests/run_notification.rs`, `tests/email_test.rs` |
| Harness, Model, Effort | `src/harness.rs` and `src/harness/` | `tests/harness_choice.rs`, `tests/harness.rs` |
| Session prompt, Factory skills | `src/prompt.rs`; `src/skills.rs`; `skills/` | `src/skills/tests.rs`, `tests/packaging.rs` |
| Trusted account, the issue a session gets | `src/trust.rs` — `issue_trust`, `ticket_trust`, `untrusted_edit` over `Reads`; the GitHub adapter's trust reads, `src/github/trust.rs`; `src/github/session_issue.rs` — `issue_for_session`; `src/delivery.rs` — `hand_over_issue` | Inline tests, including `src/trust.rs` and `src/prompt.rs`; `tests/fresh_run.rs`, `tests/pickup_run.rs` |
| Session log, Resume | `src/session.rs` and `src/session/`; `src/session/kind.rs` — `Kind`, `Logged`: every session kind, its Session-log token, the Command record stage it enters and its `--session` rule (`selects`) | `src/session/execution_tests.rs`, inline tests in `src/session/kind.rs`, `tests/background_work.rs` |
| Trial | `src/trial.rs` — versioned request/result protocol, shared `Sessions` execution with output outside factory logs | `tests/trial.rs`; protocol: `docs/trial.md` |
| Experiment, Bar, running Trials | `src/evals/experiment.rs` and `src/evals/experiment/` — fixed inputs, private storage, two workers and resume; ordinary Command recording in `src/logs/` | `tests/experiments.rs`; CLI: `docs/guide.md#experiments` |
| Plan-limit warnings | `src/harness/interpretation/limits.rs` — Claude/Codex quota detection; `src/harness/codex/limits.rs` — live native reports; `src/session/limits.rs` and `src/session/recording.rs` — Eval stop policy and shared pause signal; `src/evals/experiment/limits.rs` — supervision of older Arm builds | `src/harness/interpretation_tests.rs`, `tests/trial.rs`, `tests/experiments.rs`, `tests/fresh_run.rs` |
| Code Grader, Experiment Scores and Verdict, Arm minutes | `src/evals/experiment/grading.rs` — mechanical checks and paired suite Verdicts; `src/evals/experiment/integrity.rs` — repository snapshots; `src/evals/experiment/store.rs` — saved Verdict and minutes | `tests/experiments.rs`; check environment: `docs/eval-tasks.md` |
| OTLP JSON Lines (Session-trace file format) | `src/otlp_jsonl.rs` — application-independent requests, `Writer`, `Reader`; `src/otlp_jsonl/value.rs` — attribute values and byte encoding | `src/otlp_jsonl/tests.rs`; official examples and local Collector config in `src/otlp_jsonl/fixtures/` |
| Session trace recording and Trace ancestry | `src/session/trace.rs` — streaming records, ended spans and retained amendments; `src/logs/trace.rs` — deterministic Command, Run and stage identities | `tests/session_traces.rs`, `src/session/recording/tests.rs`, `src/logs/recording/tests.rs` |
| Whole Trace export | `src/logs/export.rs` — Command record ancestry and local OTLP JSON Lines export | `tests/export_trace.rs` |
| Experiment Trace, Arm tokens and cost | `src/evals/experiment/trace.rs` — Trial ancestry and private trace discovery; `src/evals/experiment/usage.rs` — Session-trace usage totals | `tests/experiments.rs` |
| Session trace reading, old Session logs | `src/logs/session.rs` — readable file selection and raw-event unwrapping for the Console, `show`, Silent and heartbeat | `tests/console.rs`, `tests/show_logs.rs`, `tests/run_health.rs`, `console/tests/logs.test.cjs`, `console/tests/inbox.test.cjs` |
| Command log, Activity log | `src/logs.rs`; `src/logs/recording.rs` — `Record` | `src/logs/recording/tests.rs`, `tests/command_logs.rs`, `tests/activity_log.rs` |
| Command-log allocation, Session naming, child inheritance and filename decoding | `src/logs/owner.rs` — `allocate`, `Owner`, `decode`, `session_matches`; `src/session/recording.rs` — captured Model recording | `src/logs/owner/tests.rs`, `src/logs/recording/tests.rs`, `src/session/recording/tests.rs`, `tests/command_logs.rs`, `tests/status.rs`, `tests/show_logs.rs` |
| Command record, status | `src/logs/recording.rs`; `src/logs/command_record.rs`; `src/logs/reading.rs` — `Snapshot`, exact-parent trees and observations; `src/logs/reading/history.rs` — historical recovery; `src/logs/status.rs` — formatting; `src/logs/process_identity.rs` | `src/logs/reading/tests.rs`, `tests/status.rs`, `src/logs/recording/tests.rs` |
| Silent, Slow, Usual range | `src/logs/reading/health.rs` — health observations and repository stage times; `src/config.rs` — silence threshold and safe waiting interval | `tests/run_health.rs`, `tests/ci.rs` |
| Inbox, Needs you | `src/logs/reading/inbox.rs` — local grouping, ordering, clearing and failure collapse; `src/logs/status.rs` — text and JSON | `tests/status.rs`, `tests/security_run.rs` |
| Console Lanes and Stages, stage times | `src/logs/status/time.rs` — time views from complete status observations; `src/console/time.rs`, `src/console/time.html` — page rendering | `tests/console.rs`, `console/tests/time.test.cjs` |
| Console Waterfall, tool-call timing | `src/logs/status/waterfall.rs` — retained Run/stage tree and scale; `src/logs/status/waterfall/trace.rs` — span times and running start records; `src/logs/status/waterfall/matching.rs` — legacy scoped Claude/Codex matching; `src/console/waterfall.rs` | `tests/console.rs`, `tests/console/waterfall.rs`, `console/tests/waterfall.test.cjs` |
| Command naming, Session selection, show, logs | `src/logs/reading/selection.rs` — resolution, captured outer owner and child-refusal target, Session inventory/selection and bounded latest events, each `--session` word matched by `src/session/kind.rs` — `selects`; `src/logs/status/inspect.rs` — formatting and log streaming | `src/logs/reading/tests.rs`, `tests/show_logs.rs` |
| Console Logs page, conversation, log stream, exact log inventory | `src/logs/reading/catalog.rs` — current-layout inventory shared with `logs`; `src/console/logs.rs` — pages and byte-offset SSE; `src/console/conversation.rs` — normalized rendering for all six Harnesses and legacy Claude/Codex logs | `tests/console.rs`, `tests/fixtures/console-*.jsonl`, `console/tests/logs.test.cjs` |
| Console account, Console log, sign-in | `src/console/accounts.rs` — durable accounts, sessions, slowdown and account events; `src/console/auth.rs` — HTTP authorization and CSRF; `src/console/account_commands.rs` — CLI; `src/terminal.rs` — shared hidden terminal input | `src/console/accounts/tests.rs`, `tests/console/accounts.rs` |
| Theme, hidden sections, Pause motion | `src/config/console.rs` — built-in names and restricted User config writes; `src/console/look.rs` — CLI/HTTP look actions; `src/console/assets/themes.css` — tokens; `src/console/assets/look.js` — picker and browser motion preference | `tests/console/look.rs`, `console/tests/inbox.test.cjs` |
| Custom Theme, Theme import/export, custom fonts | `src/config/console/themes.rs` — portable format, defaults, validation, folder and confined font reads; `src/console/look.rs` — shared saves and Console log; `src/console/assets/look.js` — JSON editor | `tests/console/look.rs`, `console/tests/custom-themes.test.cjs`; format in `console/README.md` |
| Console installation, proxy site, system service | `src/console/install.rs` — printed Linux installation script; `src/config/console.rs` — address and port | `tests/console_install.rs` |
| Console Cron jobs, crontab inventory | `src/cronjobs.rs` — read-only account crontab and CLI; `src/console/cronjobs.rs`, `src/console/cronjobs.html` — page rendering | `tests/console/cronjobs.rs`, `console/tests/cronjobs.test.cjs` |
| Stop | `src/logs/reading.rs` — `retained_work` reload by captured record; `src/logs/reading/selection.rs` — `Selection::stop_target`; `src/logs/stop.rs` — shared request, signaling, waiting and inspection; `src/console/stop.rs` — confirmation and authenticated Console action; `src/claim.rs` — Stop endings | `src/logs/reading/tests.rs`, `tests/stop.rs`, `tests/console/stop.rs`, `console/tests/stop.test.cjs`; inline Claim and Stop tests |
| User config, Setup | `src/config.rs`; `src/setup.rs` | `tests/user_config.rs`, `tests/setup.rs` |
| Architecture review, Architect run | `src/architect.rs` | `tests/architect_run.rs` |
| Security audit, Security review, Security run | `src/security.rs` and `src/security/`; which Runs run a Security review, and what every child Run is handed: `src/child_run/asks.rs` — `ChildAsks` | Inline tests; `tests/security_review.rs`, `tests/security_run.rs` |

Eval task manifests, suite dependency selection and repository preparation live in
`src/evals.rs` and `src/evals/`; `tests/evals.rs` tests the CLI with real Git
repositories. The private data format is documented in `docs/eval-tasks.md`.
The one-time #429 replay import is owned by `src/evals/import.rs` and exercised
through the same CLI tests with made-up manifests and grading files.

For example, `rg -n 'Claim|TicketBoard|RunNotification' src/claim.rs src/spec_run/ticket_board.rs src/notification.rs` locates the owners of Claim, Ticket board and Run notification without reading the whole glossary or README. Ticket board is an implementation term; the glossary defines its Spec run and Ticket vocabulary.

For integration-test fakes, `tests/fakes.rs` is the executable entry, `tests/fakes/<tool>.rs` holds each tool's behavior (for example, `tests/fakes/gh.rs`), and `tests/support/fakes.rs` builds and configures them. Search the relevant tool before reading a large fake or test file.

For generated prompt documentation, follow [AGENTS.md](../AGENTS.md#generated-prompts) to the sources and regeneration checks. For site browser tests, use the existing [setup and runner instructions](../site/tests/README.md).
