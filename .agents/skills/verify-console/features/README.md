# Console verification map

Maintained source for verifying user-facing features of the `thirdshift` Console. Read this index before driving, then follow the matching feature recipe.

## Baseline preconditions

- Run inside one `( ... )` subshell with `trap cleanup EXIT` registered before launching the server or browser.
- Seed an isolated `mktemp -d` `HOME` via `node .agents/skills/verify-console/scripts/seed-home.cjs "$state_dir"` (`--sign-in` for sign-in flows) with `PATH="$state_dir/bin:$PATH"`.
- Bind loopback on port `0` (`[console] port = 0`), read `$origin` (`http://127.0.0.1:<port>`) from stdout, and run `Doctor` before driving.
- Drive only the isolated Console instance started by this run; keep personal logs and network calls untouched.

## Driving conventions

- Prefer ARIA roles, accessible names, and stable `data-*` attributes over CSS layout order.
- Test both `1440×900` desktop and `390×844` mobile viewports when layout or responsive sheets change.
- Attach `pageerror`, `console` error, `requestfailed`, and `5xx` response listeners before navigation and assert zero unexpected errors.
- Wait on expected DOM/URL state after each click or key press rather than fixed sleeps outside brief `~300–500 ms` walkthrough pacing beats.

## Proof and skip reporting

- Write proof captures with `mktemp` outside `$state_dir` (`/tmp/after-desktop.XXXXXX.png` and H.264 `yuv420p` `+faststart` `<= 8 MB` `/tmp/after-desktop.XXXXXX.mp4`).
- Confirm captured PNGs and transcoded MP4s remain intact and playable after cleanup finishes.
- Record unmet seed preconditions in the affected feature's `Gotchas` rather than weakening assertions.

## Feature entry contract

Each feature file starts with an H1 title, a summary paragraph, and four H2 headings in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with Playwright`, and `Gotchas`.

## Features

- [Inbox and Peek](./inbox-and-peek.md) covers folder tabs, repository filtering, Command details Peek, and inline Dismiss/Undo.
- [Waterfall and Logs](./waterfall-and-logs.md) covers span tree expansion, Slowest calls zoom, span Peek, and Session trace conversations.
- [Themes and Look](./themes-and-look.md) covers Theme switching, custom Theme export/import, Sections visibility, and Pause motion.
- [Time pages](./time-pages.md) covers Lanes window selection and Stages `All runs` / `By repository` grids.
- [Sign-in and Account](./sign-in-and-account.md) covers password authentication, `/account`, `/console-log`, and sign-out.
