# Domain-to-code navigation

Start with the issue's terms in `CONTEXT.md`, then use this index to find their owners. This is a navigation aid, not a second glossary. Search symbols with `rg -n` and read bounded ranges around the matches; widen to callers and tests when needed. Keep these pointers current when ownership moves.

| Issue term | Owning implementation / search symbol | Supporting tests |
| --- | --- | --- |
| Claim | `src/claim.rs` — `Claim`, `make`, `end` | Inline tests; `tests/pickup_run.rs` |
| Ready issue, Claim limit, Sweep | `src/ready.rs` — `ReadyIssue`, `first`, `refresh`; `src/pickup.rs` — `gates`, `sweep`; `src/pickup/reservation.rs` — cross-machine Pickup reservation | Inline tests; `tests/pickup_run.rs` |
| Run, Continuation, Issue branch | `src/run.rs`; `src/branch.rs` | `tests/fresh_run.rs`, `tests/continuation.rs`, `tests/numbered_branches.rs` |
| Worktree | `src/worktree.rs` and `src/worktree/` | `src/worktree/acquisition_tests.rs`, `src/worktree/cleanup_tests.rs` |
| Spec run, Spec branch, Spec PR | `src/spec_run.rs` | `tests/spec_run.rs` |
| Ticket board, Tickets checklist, Unready Ticket | `src/spec_run/ticket_board.rs` — `TicketBoard`, `next_to_start`; `src/labels.rs` | Inline tests; `tests/spec_run.rs` |
| Delivery, Repair loop, Foreign commit | `src/delivery.rs` — `Delivery`; `src/delivery/repair_loop.rs` | Inline tests; `tests/merge_run.rs` |
| Failed run | `src/failed_run.rs` | `tests/failed_run.rs` |
| Inherited failure, Base fix | `src/ci.rs`; `src/base_fix.rs` | `tests/ci.rs`, `tests/base_fix.rs` |
| Run notification | `src/notification.rs` — `RunNotification`; `src/email.rs` | `tests/run_notification.rs`, `tests/email_test.rs` |
| Harness, Model, Effort | `src/harness.rs` and `src/harness/` | `tests/harness_choice.rs`, `tests/harness.rs` |
| Session prompt, Factory skills | `src/prompt.rs`; `src/skills.rs`; `skills/` | `src/skills/tests.rs`, `tests/packaging.rs` |
| Session log, Resume | `src/session.rs` and `src/session/` | `src/session/execution_tests.rs`, `tests/background_work.rs` |
| Command log, Activity log | `src/logs.rs`; `src/logs/recording.rs` — `Record` | `src/logs/recording/tests.rs`, `tests/command_logs.rs`, `tests/activity_log.rs` |
| Command-log allocation, Session naming, child inheritance and filename decoding | `src/logs/owner.rs` — `allocate`, `Owner`, `decode`, `session_matches`; `src/session/recording.rs` — captured Model recording | `src/logs/owner/tests.rs`, `src/logs/recording/tests.rs`, `src/session/recording/tests.rs`, `tests/command_logs.rs`, `tests/status.rs`, `tests/show_logs.rs` |
| Command record, status | `src/logs/recording.rs`; `src/logs/command_record.rs`; `src/logs/reading.rs` — `Snapshot`, exact-parent trees and observations; `src/logs/reading/history.rs` — historical recovery; `src/logs/status.rs` — formatting; `src/logs/process_identity.rs` | `src/logs/reading/tests.rs`, `tests/status.rs`, `src/logs/recording/tests.rs` |
| Silent, Slow, Usual range | `src/logs/reading/health.rs` — health observations and repository stage times; `src/config.rs` — silence threshold and safe waiting interval | `tests/run_health.rs`, `tests/ci.rs` |
| Inbox, Needs you | `src/logs/reading/inbox.rs` — local grouping, ordering, clearing and failure collapse; `src/logs/status.rs` — text and JSON | `tests/status.rs`, `tests/security_run.rs` |
| Console Lanes and Stages, stage times | `src/logs/status/time.rs` — time views from complete status observations; `src/console/time.rs`, `src/console/time.html` — page rendering | `tests/console.rs`, `console/tests/time.test.cjs` |
| Console Waterfall, tool-call timing | `src/logs/status/waterfall.rs` — retained Run/stage tree and scale; `src/logs/status/waterfall/matching.rs` — scoped Claude/Codex matching; `src/console/waterfall.rs` | `tests/console.rs`, `console/tests/waterfall.test.cjs` |
| Command naming, Session selection, show, logs | `src/logs/reading/selection.rs` — resolution, captured outer owner and child-refusal target, Session inventory/selection and bounded latest events; `src/logs/status/inspect.rs` — formatting and log streaming | `src/logs/reading/tests.rs`, `tests/show_logs.rs` |
| Console Logs page, conversation, log stream, exact log inventory | `src/logs/reading/catalog.rs` — current-layout inventory shared with `logs`; `src/console/logs.rs` — pages and byte-offset SSE; `src/console/conversation.rs` — Claude/Codex rendering | `tests/console.rs`, `tests/fixtures/console-*.jsonl`, `console/tests/logs.test.cjs` |
| Console account, Console log, sign-in | `src/console/accounts.rs` — durable accounts, sessions, slowdown and account events; `src/console/auth.rs` — HTTP authorization and CSRF; `src/console/account_commands.rs` — CLI; `src/terminal.rs` — shared hidden terminal input | `src/console/accounts/tests.rs`, `tests/console/accounts.rs` |
| Theme, hidden sections, Pause motion | `src/config/console.rs` — built-in names and restricted User config writes; `src/console/look.rs` — CLI/HTTP look actions; `src/console/assets/themes.css` — tokens; `src/console/assets/look.js` — picker and browser motion preference | `tests/console/look.rs`, `console/tests/inbox.test.cjs` |
| Console installation, proxy site, system service | `src/console/install.rs` — printed Linux installation script; `src/config/console.rs` — address and port | `tests/console_install.rs` |
| Stop | `src/logs/reading.rs` — `retained_work` reload by captured record; `src/logs/reading/selection.rs` — `Selection::stop_target`; `src/logs/stop.rs` — shared request, signaling, waiting and inspection; `src/console/stop.rs` — confirmation and authenticated Console action; `src/claim.rs` — Stop endings | `src/logs/reading/tests.rs`, `tests/stop.rs`, `tests/console/stop.rs`, `console/tests/stop.test.cjs`; inline Claim and Stop tests |
| User config, Setup | `src/config.rs`; `src/setup.rs` | `tests/user_config.rs`, `tests/setup.rs` |
| Architecture review, Architect run | `src/architect.rs` | `tests/architect_run.rs` |
| Security audit, Security review, Security run | `src/security.rs` and `src/security/` | `tests/security_review.rs`, `tests/security_run.rs` |

For example, `rg -n 'Claim|TicketBoard|RunNotification' src/claim.rs src/spec_run/ticket_board.rs src/notification.rs` locates the owners of Claim, Ticket board and Run notification without reading the whole glossary or README. Ticket board is an implementation term; the glossary defines its Spec run and Ticket vocabulary.

For integration-test fakes, `tests/fakes.rs` is the executable entry, `tests/fakes/<tool>.rs` holds each tool's behavior (for example, `tests/fakes/gh.rs`), and `tests/support/fakes.rs` builds and configures them. Search the relevant tool before reading a large fake or test file.

For generated prompt documentation, follow [AGENTS.md](../AGENTS.md#generated-prompts) to the sources and regeneration checks. For site browser tests, use the existing [setup and runner instructions](../site/tests/README.md).
