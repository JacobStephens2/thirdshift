# Console verification map

Maintained source for verifying user-facing features of the `thirdshift` Console. Read [../SKILL.md](../SKILL.md) and `verify-ui` (Factory: `thirdshift-verify-ui`) for isolation, walkthrough recording, and transcoding rules, then follow the matching feature recipe.

## Console preconditions

- Seed an isolated `mktemp -d` `HOME` via `node .agents/skills/verify-console/scripts/seed-home.cjs "$state_dir"` (`--sign-in` for sign-in flows) with `PATH="$state_dir/bin:$PATH"`.
- Keep `[console] port = 0`, read `$origin` from stdout, run `Doctor` before driving, and drive only the isolated instance (leave personal logs and real GitHub endpoints untouched).
- Keep default CSP during Phase 1 exploration (filtering only the documented `/waterfall` zoom `style-src 'self'` errors in [waterfall-and-logs.md](./waterfall-and-logs.md)); use `bypassCSP: true` only on Phase 2's `recordVideo` context.
- Record any unmet seed precondition in the affected feature's `Gotchas` rather than weakening assertions.

## Feature entry contract

Each feature file starts with an H1 title, a summary paragraph, and four H2 headings in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with Playwright`, and `Gotchas`.

## Features

- [Inbox and Peek](./inbox-and-peek.md) covers folder tabs, repository filtering, Command details Peek, and inline Dismiss/Undo.
- [Waterfall and Logs](./waterfall-and-logs.md) covers span tree expansion, Slowest calls zoom, span Peek, and Session trace conversations.
- [Themes and Look](./themes-and-look.md) covers Theme switching, custom Theme export/import, Sections visibility, and Pause motion.
- [Time pages](./time-pages.md) covers Lanes window selection and Stages `All runs` / `By repository` grids.
- [Sign-in and Account](./sign-in-and-account.md) covers password authentication, `/account`, `/console-log`, and sign-out.
