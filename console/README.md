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
theme = "focus-auto" # also focus-light or focus-dark
```

For a persistent service behind Caddy, Apache or nginx, see the root README's
[server installation instructions](../README.md#console-on-a-server).

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
after a change notice, preserving the selected row, folds and scroll. If the
stream drops or is unavailable, the page polls every five seconds until it
reconnects. The heartbeat only beats for observed log growth, stays flat while
Silent and respects reduced motion. The tab counts Needs you and its favicon
marks urgent items (Slow alone is not urgent).

Folds and the last-look time stay in this browser. Commands started since the
previous visit are marked new, including those arriving while the page is open.
Missing or malformed storage hides nothing. Replacing the installed binary
closes open streams and re-executes the Console with its original arguments.

The available pages are Inbox (`1`), Lanes (`2`), Stages (`3`), Waterfall (`4`),
History (`5`), Logs (`6`), Security (`7`) and Cron jobs (`8`). Inbox through Security links
keep the page in `?page=`. History includes every ended Command, including old
logs, and filters by outcome and repository. Security shows retained findings
and Session logs. Older records without finding metadata still show their logs.

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

Lanes shows each Command and its child Runs on a shared time axis, over the last
3.5, 12 or 24 hours. The window is kept in `hours=`; bars clip to that window,
while their text retains the whole stage's time. Older Commands are on History.
Stages keeps live Runs first across repositories, then finished Runs newest first.
Tables continue when the repository changes or three columns are filled, so
each stage's Usual range belongs to that table's repository. On phones each
stage stacks its labelled cells. Historical Runs without stage metadata show
an unknown time, distinct from a stage that was not reached. Choose
a column's Run link to open its exact peek. Repeated stages retain every visit;
missing end timestamps say “time unknown” rather than inventing a duration.
Both pages use the observations from `status --all --json`. The Rust parity
test checks their stage times and Usual ranges against the CLI; the Playwright
time tests check scale in all windows, child selection, live ordering and 390 px.

Waterfall chooses one retained Command with live, died and ended chips, keeping
its exact selection in `command=`. Runs, stages and tool calls share one axis,
including a Spec run's Ticket Runs. It pairs with `show --json` and `logs`:
stage intervals come from the Command record. For a Session trace, each tool
interval uses its span's start and end, including overlapping calls and fractions
of a second. A running call uses its start record and extends to the current time
while its Command is live. A call left open in an ended or died Command retains
its start with “Time unknown” for its duration.
Only sessions with an old Session log and no Session trace use Claude/Codex tool
summaries matched to scoped `HH:MM:SS` Command-log lines in order, across midnight.
Those estimated intervals extend to the next matched call or stage boundary and
include waiting. Unmatched legacy calls stay visible with “Time unknown” and no
bar. Session links open their retained evidence. The Waterfall browser test checks
selection, span times, running calls, scale, unknown times, escaping and phone
layout; Rust checks trace timing, legacy matching and CLI parity.
The retained Spec-review checks are `python3 console/reproductions/waterfall-fractional.py`
and `python3 console/reproductions/waterfall-stage.py` from the repo root after a build.
The Rust router test walks the registered routes and checks their CLI pairs.
The fractional Run/stage boundaries also run in the Rust integration tests.
The retained Session-trace regression is
`python3 console/reproductions/waterfall-trace-axis.py`; it checks that completed
tool spans still draw bars after a Command dies without recording its end.

`j` / `k` select headings and visible rows, wrapping at either end. `Space`
folds a heading or toggles a row's peek; `Enter` opens that row's full Command
log. `z` folds the current section. `Escape` closes help, jumps or the peek,
one layer at a time, then clears selection. `?` opens the key list. `g` opens
the jump list: `i` Inbox, `b` Lanes, `t` Stages, `f` Waterfall, `h` History, `o` Logs, `s` Security, `j` Cron jobs,
`c` Console log, `a` Account, or a section's displayed
letter. Text fields keep their normal keys. The peek starts closed and sits
beside the list on a wide screen or at the bottom of a phone.

For an exact older Command, `show` and `logs` also accept the absolute record
or Command-log path. The Console adds `::name` to a record path to distinguish
a dispatched Run from its enclosing Command. A retained Session-log path can
select a historical Command without a record; `logs --session` accepts an exact
path from that Command's Session inventory as well as a session kind.
Page 6, `/logs`, lists every Command log and Session log in the current
`commands/` layout, including Security logs, and marks the live ones. Every
Command row links to its Run and Session logs, including child Ticket Runs.
Older `sessions/` folders are not linked. The same inventory is available with
`thirdshift logs` or `thirdshift logs --json`; `thirdshift logs --file <id>`
reads an exact listed file and accepts `--follow`.
`thirdshift logs --command <id> --json` selects the same Command and child Run
logs as its Console link; the inventory's `owners` field lists those exact IDs.

Command logs are plain text. Session traces from all six Harnesses render their
normalized messages, tool calls and results as conversations with folded tool
groups and Bash, Edit, Read and Grep views. Unfamiliar events stay visible as
plain raw lines. Older Claude and Codex Session logs retain their conversation
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
