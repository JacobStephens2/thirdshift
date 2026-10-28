# Console browser tests

The Console is the local page served by the binary, separate from the public
site. Build it, then run Playwright against fixture Command records:

```sh
cargo build --locked
cd console
npm ci
npx playwright install --with-deps chromium
npm test
```

`THIRDSHIFT_BINARY` can name another built binary. Each run uses an isolated
home folder and a free localhost port, and closes its own server and browser.
Node and npm are test tools only; the release build embeds its assets.

For Console CSS and JavaScript development, serve the assets from this checkout:

```sh
THIRDSHIFT_CONSOLE_ASSETS=src/console/assets cargo run -- console --no-sign-in
```

In a debug build, `/assets/*` reads that directory with `no-store`; missing files
return 404. Open Console pages use their existing `/events` connection to swap
changed stylesheets in place after loading them, and reload for changed scripts,
usually within a second. No rebuild or manual reload is needed for asset edits.
Paths outside the directory and symlinks are refused. Custom Theme stylesheets
and fonts retain their usual handling.

Without the variable, and in release builds even when it is set, assets come
from the binary and disk edits have no effect. Askama templates and Rust code
stay compiled in and still require a build.

Tests load Playwright through `tests/playwright.cjs`, which gives every page a
5 s default timeout (#1391): a broken locator fails fast instead of waiting
30 s. A step that needs longer passes its own `timeout`.

The **Resources** page (`9`, or `g e`) shows a host snapshot and the configured
session admission decision, also available with `thirdshift resources --json`.
Reload to refresh it. Set `[resources] min_available_memory_mb` and
`max_load_per_cpu_percent` in User config; zero disables each limit.

The Themes picker offers **Make a Theme from this**, starting from any built-in
or custom Theme. Edit its JSON name and tokens, then **Save / import JSON**.
Choose the saved name in the picker to apply it. **Export JSON** gives a document
to copy into a file or paste into another Console; there is no file upload control.
Both CLI commands use exactly the same format and validation:

```sh
thirdshift console theme export garden-dark > garden.json
thirdshift console theme import my-garden.json
thirdshift console theme list --json
thirdshift console theme my-garden
```

Custom Themes are `~/.thirdshift/console/themes/<name>.json`. A minimal document:

```json
{"name":"my-garden","base":"garden-dark","tokens":{"bg":"#142018","font-body":"mine.woff2, sans-serif"}}
```

`base` is a built-in name with auto/light/dark mode (default `focus-auto`). Missing
tokens get that base's defaults, including tokens added in later releases. Export
includes every token. Names use ASCII letters, digits, hyphens and underscores,
up to 100 characters; `list`, `import`, `export` and altered built-in names are
reserved. Unchanged built-in exports import successfully without replacing them.
Invalid JSON, unknown fields/tokens and invalid values are refused with a reason.
Malformed files placed in the folder by hand are omitted from the picker/list.
Saving records a `theme-saved` Console log event with its actor; selecting records
`theme-changed`. Saving does not select the Theme.

Token values are strings. Each token accepts its values from any built-in Theme.
Colour tokens additionally accept hex colours (3/4/6/8 digits), `transparent`,
`currentColor`, or `light-dark(<hex>, <hex>)`. Body size accepts 10–32px, radius
0–32px, heading weight 100–900, heading case none/uppercase/lowercase/capitalize,
and activity speed 0.1–60s. Effects and activity motion use the built-in values,
keeping motion tied to activity. Arbitrary CSS and URLs are refused. User Themes
are their owner's responsibility to check for contrast.

Font tokens (`font-head`, `font-body`, `font-mono`) accept a comma-separated list
of built-in families, system stacks (`system-ui`, `sans-serif`, `serif`,
`monospace`, `ui-sans-serif`, `ui-serif`, `ui-monospace`) and font filenames.
Copy `.woff2`, `.woff`, `.otf` or `.ttf` files by hand into the themes folder's
`fonts/` directory, then use e.g. `mine.woff2, serif`. Filenames use ASCII letters,
digits, dots, hyphens and underscores, with no leading dot, up to 100 characters.
Fonts come from the Console's own address; other extensions, directories and
symlinks (including a symlinked `fonts/` folder) are refused. Unknown or unavailable
families fall back to the supplied stack, then sans-serif (monospace for code).

To open your machine's Inbox locally without accounts, run
`thirdshift console --no-sign-in` and visit the localhost address printed on
stdout. To require sign-in, set the allowlist in `~/.thirdshift/config.toml`:

```toml
[console]
address = "https://console.example.com" # used by console install
port = 8711
allowed_emails = ["you@example.com"]
other_consoles = ["https://console.example.test"]
other_accounts = "none" # or "readable", or ["conductor", "operator"]
theme = "focus-auto" # also focus-light or focus-dark
```

For a persistent service behind Caddy, Apache or nginx, see the manual's
[server installation instructions](../docs/guide.md#console-on-a-server).

Each allowed address must contain exactly one `@`, a nonempty local part,
and a domain containing a dot, with no leading or trailing dot. Whitespace
is not allowed. Setup trims each comma-separated entry, but rejects empty
items, including trailing commas, and asks again with the invalid entry named.
Loading a User config with an invalid entry also fails and names the entry.

Run `thirdshift console password --email you@example.com` in a terminal to set
the password, then `thirdshift console`. Password entry is hidden and asks
twice. With exactly one allowed address, `--email` can be omitted. An empty
allowlist refuses to serve unless `--no-sign-in` is explicit. Both modes bind
only to 127.0.0.1. Use HTTPS at the machine's reverse proxy for sign-in: cookies
are always Secure, HttpOnly and SameSite=Strict (localhost is suitable for tests).

Accounts live in `~/.thirdshift/console-accounts.json`, beside the Credentials,
with owner-only permissions. Passwords use argon2id; only hashes of session
tokens are stored. Changes use a file lock and atomic replace, including across
CLI and server processes. Sessions survive restarts and expire 30 days after
last use. Every action, including sign-in and sign-out, requires a CSRF token.
After five wrong passwords, an address gets one attempt per minute until a
successful sign-in or a new password resets the slowdown.

The Account page signs out the current browser. `thirdshift console sign-out
[--email <address>] [--json]` ends every session for that address; setting a new password
also ends all its sessions. Removing an address from `allowed_emails` blocks
it and revokes its sessions, while retaining the password for later re-adding.
The server checks current settings on requests and while streams remain open.

`thirdshift console log [--json]` and the Console log page show the same
account events from `~/.thirdshift/console.log`. They name the account but
never contain passwords, codes, session tokens or page views.

With a Resend key in `RESEND_API_KEY` or the Credentials, **Set your password**
on the sign-in page creates an account or resets its password by email code.
Only allowed addresses receive mail, from `email.from`, but every address gets
the same answer, without waiting for address-dependent delivery. Codes are
stored as argon2id hashes, last 15 minutes and allow
5 tries. An address may receive one code a minute and 10 in a rolling 24 hours;
these limits survive restarts and password changes. A correct code verifies
the address, sets its password, ends its old sessions and signs it in.
The Console log records codes sent and passwords set. Without a key the flow
is off; ask the owner to use `thirdshift console password` instead.

The header names this machine and links to those addresses, without fetching
them. The Inbox and its `/events` stream pair with `thirdshift status`. One
server observer checks records and log lengths every second; the page morphs
after a change notice, preserving the open section, selected row and scroll. If the
stream drops or is unavailable, the page polls every five seconds until it
reconnects. The heartbeat only beats for observed log growth, stays flat while
Silent and respects reduced motion. The tab counts Needs you and its favicon
marks urgent items (Slow alone is not urgent). Each row shows the same state
word as `thirdshift status`: Landing and Ready rows take their Theme's Landing
and Ready tag colours, Landing keeps Working's live-activity marker, and a live
row given a Stop shows Stopping with a drifting stripe until it ends. Pause
motion and reduced motion hold the stripe still.

The Inbox reads like a mailbox. Its sections, Needs you through Resolved, are
folders on the left with their counts, and one section's rows show at a time,
on one line each: state, subject, title, gist and age. On a desktop, issue
numbers stay visible as long repository or Process subjects shrink; hover
the subject for its full text. The gist is where the
Command stands, such as `review failed`; a Pass without an issue adds the
issue it dispatched, its first security finding or its cause. Hover an ended
row's age for its finish time. The list never scrolls inside itself; the page does. The
folder is in the URL, such as `/?section=done`, so it survives a reload and
works without JavaScript; with none named, the first section with rows opens.
The page opens with nothing selected and Command details closed. Click a row,
or select it and press `Space`, to open its details beside the list; they stay
open for the selected row as you move with `j` / `k` or switch folders. On a
phone the folders are a row of tabs and Command details open as a bottom sheet. Without
JavaScript, rows show in full, with their own Dismiss and logs links.

Command details open with the Inbox subject and the full issue title, before
Stop and Dismiss. The subject links to the issue, and long titles wrap. A
Command without an issue shows its Inbox subject alone; an unknown issue title
says “(title unavailable)”. The same recorded title is in
`thirdshift show <command> --json`. Below the heading is one line per recorded fact,
label then value. A long value, or a fact with its own facts, such as stages,
the dispatched Run or run metrics, starts folded behind a one-line preview (a
title, a name or a count); open it to drill down, as deep as the record goes.
Session traces fold into one count. Folds stay as you left them while live
updates refresh the same Command. A file path shows on one line with its end
kept and a copy icon that copies the whole path.

Each Needs you row has a **Copy** button at the end of its line for sharing its
problem with an agent. It copies the machine, Command selector, state, cause,
issue title, Harness, Tickets and any findings, with absolute issue, pull
request and Console log links, plus the `thirdshift show '<command>' --json`
command, without opening Command details. If clipboard access is unavailable,
a dialog selects the same text for copying by keyboard. The dialog keeps that
snapshot through live updates.

Dismiss is always visible on the row in Needs you and Ready for review,
beside Copy where available. `d` dismisses the selected row and selects the next
one, without opening Command details. An inline dismissal shows a temporary
`Dismissed <subject> · Undo` notice; Undo restores that Command's row. Notices
stay while hovered, focused, saving or showing an error. Command details also
keeps Dismiss and Undo. Both immediately
show “Dismissing…” or
“Taking back dismissal…” while the request is pending. The button stays disabled
through live updates to prevent duplicate submissions. A failed request shows
the error and enables the button for another try.

Ended rows show their recorded finish in this machine's time zone, to the
minute, followed by an explicit relative age, such as “Ended 2026-10-23 19:03
EDT · 52 min ago”. Every date and time the Console shows reads the same way,
including ones inside causes and Command details; records, CLI output and
Session traces keep their full RFC 3339 instants.
That age is time since finish, not run duration. Each nested Ticket run uses its
own finish time, including in History. Refreshing recomputes the relative age
and keeps the recorded instant. The information is rendered by the server and
remains visible without JavaScript. Missing or invalid finish times say “End
time unknown”; file modification times are not presented as recorded finishes.

Above its sections, the Inbox shows how many runs are active, such as
`2 active runs`, the same `active_runs` total as `thirdshift status --json`.
Each Command with live work anywhere in it counts once, whatever its group or
whether that section is open or hidden: a Spec run's Tickets, and the Spec run
or Ticket run a Pass dispatched, are part of that one. The live update refreshes
it as Commands start and end.

When no Commands are live on this machine, the Inbox explains that above its
sections. It points to pull requests in Ready for review, and offers the last recorded skipped Pass reasons from Activity logs
under **Last recorded skipped Passes**, and when each Pass last fired under
**Last fired Passes**, both folded closed on each page load. Open either to
read it; live updates keep it open while you read.
These reasons are retained observations, not a check that the scheduler still
runs. With none recorded, it says so. A live Command in
Needs you, or in a hidden section, still counts as running.

Choose **Turn notifications on** in the Console header and allow the browser's
permission prompt. This browser remembers the choice for this Console address;
the same button turns it off across its open tabs. Keep a Console tab open
(a pinned tab works): a rise in Needs you shows one browser notification with
the count, machine and newest arriving item. The item's state comes first, then
its issue and title, so a toast cut short still says what happened. Clicking it
focuses the tab and opens the Inbox. Opening or reloading a tab never notifies
for existing items, and several tabs share one notification per rise.
Notifications work from the other Console pages too.

Notifications require HTTPS (or localhost), browser notification support,
permission, browser storage and Web Locks for coordination between tabs. The
control explains when these are unavailable; if permission is blocked, change
it in the browser's site settings. There are no notifications with all Console
tabs closed, and no service worker, Web Push or subscription to set up.

History and Security folds and the last-look time stay in this browser.
Commands started since the previous visit are marked new, including those
arriving while the page is open.
Missing or malformed storage hides nothing. Replacing the installed binary
closes open streams and re-executes the Console with its original arguments.

The available pages are Inbox (`1`), Lanes (`2`), Stages (`3`), Waterfall (`4`),
History (`5`), Logs (`6`), Security (`7`) and Cron jobs (`8`). Inbox through Security links
keep the page in `?page=`. History includes every ended Command, including old
logs, and filters by repository and kind of outcome (merged, ready for review,
failed, plan and so on), not by each PR or error. Security shows retained findings
and Session traces. Older records without finding metadata still show their logs.

Pages give the high-level view first and open on demand. History and Security
list one line per Command, like the Inbox; a click opens its Command details
beside the list, and without JavaScript rows show in full. The Experiments
list shows each Experiment's headline and status line, and a click opens its
page. An Experiment page folds each suite's Verdict to its intervals, each
suite's Trial grid to its task count, and What was compared and the Bar to
their headings. Stages by repository folds each repository to its run count,
and Logs folds each repository to its log count when it lists more than one.
Folds stay as left through live updates.

Cron jobs (`/cronjobs`, or `g` then `j`) shows schedules and commands from the
Unix account running the Console's `crontab -l`. The same inventory is available
with `thirdshift cronjobs [--json]`. It selects active entries containing a
literal `thirdshift` word or an executable path ending in `/thirdshift`, including
commands after `cd` and wrappers. Shell scripts and variable-built executable
names are not inspected or evaluated. Comments and environment assignments
are omitted, and input after cron's first unescaped `%` is not searched.
Other users, system crontabs and other schedulers are outside this view.
Schedules retain cron's expressions and time zone; reload to see changes.
No matching entries or no crontab shows an empty view; an unavailable `crontab`
command or a read failure reports an error. This view never changes the crontab
or starts a Command.

Scores (`/scores`, or `g q`) lists the sample by week and each criterion's
calibration, from the same retained records as `thirdshift scores --json`.
Enter five pass/fail Scores and reasons in a sample's form. The model's Scores
are withheld from the response until all five of yours are saved, using the
same lock and atomic write as `thirdshift scores <pull request> --mine --json`.
Score again opens another blind entry. Reload to see changes from other clients.

Bug origins (`/bug-origins`, or `g u`) lists proposed, confirmed and rejected
origins with their evidence, pull requests, Ticket-run attribution and settled
Eval task. Confirm, Correct (a pull request URL or number) and Reject use exactly
the operations behind `thirdshift bug-origins <issue> --confirm`,
`--correct <pull request>` and `--none`, including their refusal of concurrent
Scoring writers. Every form requires CSRF; both pages require sign-in unless
the Console runs with `--no-sign-in`. Confirmation proposes an Eval task and
starts no work. `node --test console/tests/scoring.test.cjs` checks navigation,
all four actions, escaping and 390 px after a build.

Lanes shows each Command and its child Runs on a shared time axis, over the last
3.5, 12 or 24 hours. The window is kept in `hours=`; bars clip to that window,
while their text retains the whole stage's time. Older Commands are on History.
Stages defaults to **All runs**, one grid across repositories. **By repository**
shows one grid per repository, each folded to its name and run count; the
browser remembers an explicit view choice.
Each Spec run stays with its exact Ticket runs, including repeated attempts on
the same issue. Groups are newest-started first by their owning run, and Ticket
runs are newest-started first within each Spec group. Live state and finish
times do not change that order. Dispatch ancestry stays visible.
Rows follow the Command record's stage order: implement through cleanup, then
the stages of other Command kinds. Unrecognised stage names follow all known
stages in alphabetical order.
Stages fills the window's width, beside the peek when one is open. More runs
extend the grid to the right on desktop and phones, with stage names fixed at
the left. Each cell's Usual range belongs to that run's repository.
Live updates keep the visible run, selection and focus; **Newest runs** returns
the relevant grid to its left edge. Historical Runs without stage metadata show
an unknown time, distinct from a stage that was not reached. Choose
a column's Run link to open its exact peek. Repeated stages retain every visit;
missing end timestamps say “time unknown” rather than inventing a duration.
Both pages use the observations from `status --all --json`. The Rust parity
test checks their stage times and Usual ranges against the CLI; the Playwright
time tests check scale in all windows, grouping, both views, preferences, live
insertions, keyboard access, Themes, 390 px and Stages at 1920 px.

Waterfall chooses one retained Command with live, died and ended chips, keeping
its exact selection in `command=`. The chips fold under a Runs line that names
the Command shown; open it to choose another. Each chip and the selected Runs
line include the retained issue title when available, shortened with an ellipsis
to keep the choice brief; hover the title to read it in full. Older records
without a title and Commands without an issue keep their existing labels.
A span tree replaces the old Run sections:
Command → Runs → stages → Sessions → model calls, turns, tools and sub-agents.
Child Runs sit under the stage that started them; reused Sessions appear only
under their first stage. Commands, Runs and stages start expanded, Sessions
folded. Calls enter the DOM only after their Session is expanded.

The same tree and selected span details are available from the CLI:

```sh
thirdshift show widgets#7 --spans --json
thirdshift show widgets#7 --span <span-id> --json
```

The tree uses the recorder's stable Command, Run and stage span IDs. Each row
has its kind, status, duration and bar on a shared axis. Errors carry their
message, running calls extend to now with a hatched bar, and estimated timings
are dotted and marked `*`. Converted Session tools keep the old scoped
Command-log estimates, including unknown times. Commands without traces still
show their Runs and stages.

The open Slowest calls list ranks ten tool and model calls in the timeline's
window; turns and sub-agents are excluded. Selecting a call reveals its row
and summary Peek. Ancestors in the Peek select their own span. The card shows
applicable timing, usage, cost, call and error facts, relative-position bars,
input/output and attributes, plus a conversation link and Copy CLI. Conversation
links open the Session because that reader does not yet anchor individual
records. Command details opens the existing Peek with Stop, logs and the PR.
Its Run metrics come from the same details as `thirdshift show <command> --json`,
including nested Ticket runs. Recorded zeroes stay zero; unavailable signals say
Missing, including older Ticket runs without retained metrics.
That Peek's Copy CLI copies its `thirdshift show '<command>' --json` line; if
the browser refuses the clipboard, the line is selected to copy by keyboard,
and stays selected through live refresh. The Dismiss line works the same way:
`[Dismiss] thirdshift dismiss '<command>' [Copy CLI]`. Each CLI line stays on one
line, cut off at the pane's edge, with the whole command on hover and in the
copy. Session trace links also stay on one line, trimmed from the start so the
end of each path, which tells them apart, still shows.

After a build on Linux, `python3 console/reproductions/peek-cache-contention.py`
checks that parsing another Command's cold Session trace does not delay cached
Command details.

Historical Session signals are cached in memory and in disposable
`*.signals-cache.json` files beside their Session traces, with per-trace
`*.signals-cache.lock` files coordinating CLI processes. Appends, replacements
and binary changes invalidate saved signals. A read-only logs directory still
works by parsing without saving a cache. Command records are never rewritten.

Zoom fits a parent span or a padded window around a call. The sticky axis keeps
the zoom notice and Zoom out control visible; the slowest calls re-rank within
the window. Folds, selection, zoom and Peek disclosures survive live refresh.
On a phone, labels fill one row, bars and durations the next; revealed rows
stay between the sticky axis and the Peek bottom sheet.

Within the tree, arrows and `j` / `k` move focus, Right expands or enters a
child, Left folds or returns to the parent, Home/End reach either end, and
Enter/Space opens the Peek. Outside the tree, the Console's usual keys apply.

Rust route/CLI parity and timing tests live in `tests/console/spans.rs` and
`tests/console/waterfall.rs`. The Playwright span and Waterfall tests cover
folding at scale, keyboard navigation, Peek, zoom, live refresh, bar geometry,
escaping and 390 px. Retained timing checks also run with
`python3 console/reproductions/waterfall-fractional.py`,
`python3 console/reproductions/waterfall-stage.py`, and
`python3 console/reproductions/waterfall-trace-axis.py` after a build.

Experiments (`/experiments`, or `g` then `x`) uses Running, Needs you and Done
groups. Its Experiment page opens with What it tests: the Candidate's changes
from the Baseline (build with its commit title, Harness, Model, Effort or
Harness version), the suites it is judged on and the Bar's margin, or, when the
arms match, that it tests no change and shows run-to-run variation. It then
shows the live Trial grid, per-suite Verdict
intervals, pending rechecks, Arm comparison and folded Bar. A Trial or Eval task
opens the Peek; each retained attempt links to its Session log or Session trace.
Both pages use the same view as `thirdshift experiments [<id>] --json`, including
the saved refs/title, end time, resume command, active Trials and session IDs.
`thirdshift experiments <id> --session <session-id> [--json]` reads a listed
session; JSON returns its `id`, `trace` flag and `contents`.
These pages poll every two seconds independently of Command-record events, and
retain folds, filters and the open Peek. Rechecks stay in the CLI; Copy command
copies the current Trial's command. `node --test console/tests/experiments.test.cjs`
checks live updates and interaction at 390 px after a build.
The Experiments list also shows proposed Eval tasks; each Experiment page shows
proposals for its suites. Approve one there or with
`thirdshift evals approve <suite> <task> [--json]`. Both change only the manifest's
state and start no work. Approval affects future Experiments; an existing
Experiment keeps its fixed Eval tasks on resume.

`j` / `k` select headings and visible rows, wrapping at either end; on the
Inbox, open Command details follow the selection. `Space` toggles a row's
peek, or folds a heading; `Enter` opens that row's full Command log. `d` dismisses
a selected dismissible row and selects the next row. `z` folds
the current History or Security section. `Escape` closes help, jumps or the
peek, one layer at a time, then clears selection. `?` opens the key list. `g`
opens the jump list: `i` Inbox, `b` Lanes, `t` Stages, `f` Waterfall, `h` History, `o` Logs, `s` Security, `j` Cron jobs,
`c` Console log, `a` Account, or an Inbox section's displayed letter, which
opens its folder. Text fields keep their normal keys. The peek starts closed
and sits beside the list on a wide screen or at the bottom of a phone.

For an exact older Command, `show` and `logs` also accept the absolute record
or Command-log path. The Console adds `::name` to a record path to distinguish
a dispatched Run from its enclosing Command. A retained Session-trace path can
select a historical Command without a record; `logs --session` accepts an exact
path from that Command's Session traces as well as a session kind.
Page 6, `/logs`, lists every Command log and Session trace in the current
`commands/` layout, including Security logs, and marks the live ones. Every
Command row links to its Command logs and Session traces, including child Ticket Runs.
Older `sessions/` folders are not linked. The same inventory is available with
`thirdshift logs` or `thirdshift logs --json`; `thirdshift logs --file <id>`
reads an exact listed file and accepts `--follow`.
`thirdshift logs --command <id> --json` selects the same Command and child Run
logs as its Console link; the inventory's `owners` field lists those exact IDs.

Command logs are plain text. Session traces from all six Harnesses render their
normalized messages, tool calls and results as conversations with folded tool
groups and Bash, Edit, Read and Grep views. Unfamiliar events stay visible as
plain raw lines. Older Claude and Codex Session traces retain their conversation
rendering and collapsed thinking. A live reader uses its own SSE stream
to append escaped HTML from a byte offset. Reconnecting resumes from the last
event's byte offset; incomplete live lines wait until their newline arrives.
The Follow newest line checkbox lets you read back while the Session grows.
The Playwright log test checks appends, retained DOM identity, folding across
batches, incomplete lines, reconnection and following on a phone viewport.
When streaming is unavailable, the reader polls finite appended-line batches
from its last byte offset without replacing the rendered conversation.

Ready Runs dispatched by a Pass have their own Ready for review row in both
`status` and the Console. The complete Pass tree remains available in History
and `show`. Run `python3 console/reproductions/ready-dispatch.py` after a build
to check both Inbox views.

A live Command's Details offers Stop beside its equivalent CLI command.
The confirmation page explains `needs-triage`, offers Requeue instead
(`thirdshift stop --requeue`), and requires the current account's CSRF token.
Both actions use the CLI's Stop, including its process identity checks and
refusal to stop a single Ticket's Run. The Activity log names the Console
and the signed-in email; localhost mode records `localhost (no sign-in)`.
The page shows “still ending” and a stopping bar until cleanup records the
outcome, then lists the branches and pull requests left on `origin`.
It updates through `/events`, with polling if the stream disconnects.
The stopping bar respects reduced motion. HTTP tests use real Commands
with a waiting fake Harness and fake GitHub; Playwright checks confirmation,
cancellation, live controls and pushed completion on a phone.
`thirdshift stop <command> [--requeue] --json` waits quietly and returns the
outcome, `issue_branches` and `pull_requests` as one JSON object.
On Linux after a build, `python3 console/reproductions/session-stop.py` checks that
a live Session reader's Details link reaches its exact Command's Stop.

Claude Edit events include old/new text and show unified diff excerpts. For Codex,
thirdshift reads the thread's native session file after exit, under
`$CODEX_HOME/sessions` (default `~/.codex/sessions`). When the current turn's edits
match the stream in order and file path, the Session trace keeps each patch as
Codex's record of the edit. The reader replaces its placeholder with that diff,
including in an open live reader. Shell edits and missing, unreadable, changed-format
or mismatched session files keep the fallback. The format is unpublished, so this
evidence is optional and never changes a Run's outcome.
Run `cargo test --test console codex_patches` for the fake-Codex regression and
`node --test console/tests/logs.test.cjs` for the live-reader checks after building.
