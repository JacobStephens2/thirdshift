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

To open your machine's Inbox, run `thirdshift console --no-sign-in` and visit
the localhost address printed on stdout. Without `--no-sign-in` the command
refuses to serve until the sign-in Ticket ships. In `~/.thirdshift/config.toml`:

```toml
[console]
port = 8711
other_consoles = ["https://console.example.test"]
theme = "focus-auto" # also focus-light or focus-dark
```

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

The available pages are Inbox (`1`), Lanes (`2`), Stages (`3`), Waterfall (`4`), History (`5`), Logs (`6`) and Security (`7`). Links
keep the page in `?page=`. History includes every ended Command, including old
logs, and filters by outcome and repository. Security shows retained findings
and Session logs. Older records without finding metadata still show their logs.

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
stage intervals come from the Command record, and Claude/Codex tool summaries
match their scoped `HH:MM:SS` Command-log lines in order, across midnight.
Each tool interval extends to the next matched call in its Session or its stage
boundary. These include waiting; they are not measured tool execution times.
Unmatched calls stay visible with “Time unknown” and no bar. Session links open
their retained evidence. The Waterfall browser test checks selection, scale,
unknown times, escaping and phone layout; Rust checks matching and CLI parity.
The retained Spec-review checks are `python3 console/reproductions/waterfall-fractional.py`,
`python3 console/reproductions/waterfall-stage.py` and
`python3 console/reproductions/waterfall-parity.py` from the repo root after a build.
The fractional Run/stage boundaries also run in the Rust integration tests.

`j` / `k` select headings and visible rows, wrapping at either end. `Space`
folds a heading or toggles a row's peek; `Enter` opens that row's full Command
log. `z` folds the current section. `Escape` closes help, jumps or the peek,
one layer at a time, then clears selection. `?` opens the key list. `g` opens
the jump list: `i` Inbox, `a` Lanes, `t` Stages, `h` History, `o` Logs, `s` Security, or a section's displayed
letter; `f` opens Waterfall. Text fields keep their normal keys. The peek starts closed and sits
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

Command logs are plain text. Claude and Codex Session logs are conversations
with folded tool groups, collapsed thinking, Bash, Edit, Read and Grep views,
and plain output for unfamiliar events. A live reader uses its own SSE stream
to append escaped HTML from a byte offset. Reconnecting resumes from the last
event's byte offset; incomplete live lines wait until their newline arrives.
The Follow newest line checkbox lets you read back while the Session grows.
The Playwright log test checks appends, retained DOM identity, folding across
batches, incomplete lines, reconnection and following on a phone viewport.

Claude Edit events include old/new text and show unified diff excerpts. Codex
`file_change` events retain only file paths and change kinds, so the reader
shows their plain fallback and says the patch is unavailable. The Spec-review
reproduction, `python3 console/reproductions/codex-edit-diff.py` from the repo
root, preserves the unmet Codex diff expectation; it currently fails. Capturing
patch evidence for Codex requires a separate retention decision.
