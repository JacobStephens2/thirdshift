# Inbox and Peek

The Inbox groups seeded Commands by status (`Needs you`, `Ready for review`, `Landing`, `Working`, `Done`, `Resolved`), filters by repository, opens Command details in the Peek pane, and supports inline Dismiss and Undo.

## Sub-features

- `inbox-overview-folds` displays open `Needs you` and `Ready for review` groups with folded `Done` and `Resolved` summaries.
- `inbox-folders-filter` filters by section folder and repository while preserving URL parameters.
- `inbox-peek` opens Command details (`#peek-content h2`), Copy CLI controls, and folded facts.
- `inbox-dismiss-undo` dismisses a `Needs you` row inline, shows the Undo notice, and restores the row.
- `inbox-mobile-sheet` switches folders as tabs and opens Peek as a bottom sheet at `390×844`.

## How to get to it (user POV)

- Open `$origin/` (`1 Inbox` or `g` then `i`).
- Select a folder link in `nav[aria-label="Inbox sections"]` (`Overview`, `Needs you`, `Ready for review`, `Done`, `Resolved`).
- Click a Command row (or select with `j`/`k` and press `Space`) to open Command details.

## Driving it with Playwright

Preconditions:

- Console started with `--no-sign-in` against a `seed-home.cjs` directory and `Doctor` passed.
- Desktop viewport `{ width: 1440, height: 900 }` (repeat responsive check at `{ width: 390, height: 844 }`).

- **Verify overview and active count.** Navigate to `${origin}/`. `#inbox` renders `.active-runs` (`0 active runs`), `Needs you` shows `acme/widgets#7`, and `Ready for review` shows `acme/widgets#18` (pull request `#18` linked by seeded issue `#8`).
- **Open Command details Peek.** Click the row's state badge (`#inbox > [data-group="needs you"] .command > .row-head > .state`) or its `Details` link. `#peek` becomes visible, `#peek-content h2` contains `acme/widgets#7`, and `.peek-failure` shows `Search test failed: empty query should show all widgets.`.
- **Filter by repository.** Click `Done` in `nav[aria-label="Inbox sections"]`, select `acme/widgets` in `.machine-filter [name="repository"]`, and click button `Filter`. URL updates to `/?machine=&repository=acme%2Fwidgets&section=done` (`machine`, `repository`, then `section` in form order) and shows `acme/widgets#9` without `acme/gadgets#10`.
- **Dismiss and Undo inline.** Return to `${origin}/?section=overview`. Click `Dismiss` on the `Needs you` row (`#inbox [data-group="needs you"] .command button:has-text("Dismiss")`). `#dismissal-notices` displays `Dismissed` and `Undo`; clicking `Undo` restores `acme/widgets#7` to `Needs you`.
- **Proof.** Capture desktop screenshot (`page.screenshot({ path: shot })`) showing Inbox with Peek open, and record a Phase 2 `1440×900` `.mp4` walkthrough traversing Overview -> Peek -> Done repository filter -> Dismiss & Undo.

## Gotchas

- `seed-home.cjs` omits live processes so `0 active runs` renders and `Idle Passes` disclosure starts closed; open `#idle-passes summary` only if activity logs are seeded.
- Wait for `#peek-content h2` or `page.waitForURL` after clicks; clicking the center of `.command` can hit an `<a>` inside `.subject` and navigate away instead of opening Peek, so click `> .row-head > .state` (or `.peek-link`).
- Clipboard permissions in headless Chromium can deny `navigator.clipboard.writeText`; clicking row `Copy` then opens `#copy-context` dialog as the expected fallback.
