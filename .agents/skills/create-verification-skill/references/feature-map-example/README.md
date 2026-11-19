# Notes verification map

This directory is the maintained source for verifying the user-facing behavior of Notes. Read the index before driving the app, then use the matching feature file as the recipe.

## Baseline preconditions

- Start inside one `( ... )` subshell with `trap ... EXIT` registered before launching the server or browser daemon.
- Create an isolated data directory with `mktemp -d /tmp/notes-state.XXXXXX` (`NOTES_DATA_DIR`) and bind loopback on port `0` (`127.0.0.1:0`), reading the actual `ORIGIN` from readiness output.
- Seed notes titled `Quarterly plan` and `Grocery list`; load optional faux credentials only from `~/.config/thirdshift/verify-env/<owner>/<repo>.env` if the flow requires auth.
- Put `control-notes` and the `notes` CLI on `PATH`, and run `control-notes doctor` before driving.
- Drive only the instance started by this verification run.

## Driving conventions

- Start every recipe from the baseline state unless its preconditions say otherwise.
- Prefer ARIA roles and accessible names over CSS selectors or DOM position.
- Treat every command as literal. Keep quoted names and flags unchanged.
- Run browser exploration through `control-notes browser` and terminal actions through `control-notes cli -- <command>`.
- Record clean walkthroughs via a temporary `mktemp` Playwright script at `1440×900` (`390×844` when responsive layout changes) with zero unexpected `pageerror` or `5xx` responses.
- Restore seeded data after a mutation and preserve `mktemp` proof captures across cleanup.

## Proof and skip reporting

- Capture the user action and resulting state: `mktemp /tmp/after-desktop.XXXXXX.png` and H.264 `yuv420p` `+faststart` (`<= 8 MB`) `.mp4` walkthroughs (`mktemp /tmp/after-desktop.XXXXXX.mp4`).
- UI proof pairs an ARIA snapshot with a screenshot or walkthrough showing app identity and faux data.
- CLI proof includes the command, stdout, stderr, and exit code.
- Mutation proof includes a read-only second view of the stored value.
- Record the feature ID and entry point used with every artifact.
- Report an unreachable path with the attempted command and the unmet precondition.
- Verify skipped entry points independently rather than claiming coverage from another path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order.

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with <harness>` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

Keep implementation details out of the map. Name only user paths, stable handles, required state, commands, and observable proof.

## Features

- [Create a note](./create-note.md) covers browser and CLI creation, cancellation, persistence, and cleanup.
- [Search notes](./search.md) covers toolbar, keyboard, and CLI search with matching, empty, and clear states.
