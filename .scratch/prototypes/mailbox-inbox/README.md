# PROTOTYPE: mailbox layouts for the Console Inbox

Question: the Needs you list was capped at 96-300px with its own scroll, so
one of ten rows showed. What should a mailbox-style Inbox look like?

Variants on the real Inbox route, `/?variant=`, with ← → or the yellow bar:

- `a` Three-pane mail: sections as folders, one-line rows, Details always open.
- `b` Tabbed inbox: sections as tabs, one-line rows that open in place.
- `c` Table: stacked sections with sticky headings, Details docked below.

Run: `cargo build && HOME=<sample home> ./target/debug/thirdshift console --no-sign-in`.
Files: `src/console/assets/prototype-mailbox.{js,css}` and one line in
`src/console/inbox.html`.

Verdict (2026-10-26): Jacob chose **A, three-pane mail**.
