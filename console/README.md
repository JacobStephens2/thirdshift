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

The available pages are Inbox (`1`), History (`5`) and Security (`7`). Links
keep the page in `?page=`. History includes every ended Command, including old
logs, and filters by outcome and repository. Security shows retained findings
and Session logs. Older records without finding metadata still show their logs.

`j` / `k` select headings and visible rows, wrapping at either end. `Space`
folds a heading or toggles a row's peek; `Enter` opens that row's full Command
log. `z` folds the current section. `Escape` closes help, jumps or the peek,
one layer at a time, then clears selection. `?` opens the key list. `g` opens
the jump list: `i` Inbox, `h` History, `s` Security, or a section's displayed
letter. Text fields keep their normal keys. The peek starts closed and sits
beside the list on a wide screen or at the bottom of a phone.

For an exact older Command, `show` and `logs` also accept the absolute record
or Command-log path. The Console adds `::name` to a record path to distinguish
a dispatched Run from its enclosing Command. A retained Session-log path can
select a historical Command without a record; `logs --session` accepts an exact
path from that Command's Session inventory as well as a session kind.
