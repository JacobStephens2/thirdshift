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
