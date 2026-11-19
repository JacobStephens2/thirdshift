# Console verification map

Maintained source for verifying user-facing features of the `thirdshift` Console. Read [../SKILL.md](../SKILL.md) and `verify-ui` (Factory: `thirdshift-verify-ui`) for isolation, walkthrough recording, and transcoding rules, then follow the matching feature recipe.

## Console preconditions

- Follow [../SKILL.md](../SKILL.md) for isolated `seed-home.cjs` setup, `Doctor`, CSP rules, and `Cleanup`, and `verify-ui` (Factory: `thirdshift-verify-ui`) for recording/transcoding mechanics.
- Record any unmet seed precondition in the affected feature's `Gotchas` rather than weakening assertions.

## Feature entry contract

Each feature file starts with an H1 title, a summary paragraph, and four H2 headings in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with Playwright`, and `Gotchas`.

## Features

- [Inbox and Peek](./inbox-and-peek.md) covers folder tabs, repository filtering, Command details Peek, and inline Dismiss/Undo.
- [Waterfall and Logs](./waterfall-and-logs.md) covers span tree expansion, Slowest calls zoom, span Peek, and Session trace conversations.
- [Themes and Look](./themes-and-look.md) covers Theme switching, custom Theme export/import, Sections visibility, and Pause motion.
- [Time pages](./time-pages.md) covers Lanes window selection and Stages `All runs` / `By repository` grids.
- [Sign-in and Account](./sign-in-and-account.md) covers password authentication, `/account`, `/console-log`, and sign-out.
