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

To open your machine's Inbox locally without accounts, run
`thirdshift console --no-sign-in` and visit the localhost address printed on
stdout. To require sign-in, set the allowlist in `~/.thirdshift/config.toml`:

```toml
[console]
port = 8711
allowed_emails = ["you@example.com"]
other_consoles = ["https://console.example.test"]
theme = "focus-auto" # also focus-light or focus-dark
```

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
never contain passwords, session tokens or page views. Email account creation
and password reset are a later Ticket; ask the owner to use the password command.

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
