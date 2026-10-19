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
them. The first page pairs with `thirdshift status`. A refresh reads the current
records; server-push updates are a later Ticket. Replacing the installed binary
makes the Console re-execute itself with its original arguments.

Page 6, `/logs`, lists every Command log and Session log in the current
`commands/` layout, including Security logs, and marks the live ones. Every
Command row links to its Run and Session logs, including child Ticket Runs.
Older `sessions/` folders are not linked. The same inventory is available with
`thirdshift logs` or `thirdshift logs --json`; `thirdshift logs --file <id>`
reads an exact listed file and accepts `--follow`.

Command logs are plain text. Claude and Codex Session logs are conversations
with folded tool groups, collapsed thinking, Bash, Edit, Read and Grep views,
and plain output for unfamiliar events. A live reader uses its own SSE stream
to append escaped HTML from a byte offset. Reconnecting resumes from the last
event's byte offset; incomplete live lines wait until their newline arrives.
The Follow newest line checkbox lets you read back while the Session grows.
The Playwright log test checks appends, retained DOM identity, folding across
batches, incomplete lines, reconnection and following on a phone viewport.
