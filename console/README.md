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

The available pages are Inbox (`1`), History (`5`), Logs (`6`) and Security (`7`). Links
keep the page in `?page=`. History includes every ended Command, including old
logs, and filters by outcome and repository. Security shows retained findings
and Session logs. Older records without finding metadata still show their logs.

`j` / `k` select headings and visible rows, wrapping at either end. `Space`
folds a heading or toggles a row's peek; `Enter` opens that row's full Command
log. `z` folds the current section. `Escape` closes help, jumps or the peek,
one layer at a time, then clears selection. `?` opens the key list. `g` opens
the jump list: `i` Inbox, `h` History, `o` Logs, `s` Security, or a section's displayed
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
