# Themes and Look

Themes and Look controls let operators switch built-in Themes, export or import custom Theme JSON, hide or show Inbox sections, and toggle animation motion.

## Sub-features

- `theme-switch` opens `Themes (Shift+T)`, applies another built-in Theme, and updates `html[data-theme]`.
- `theme-export-import` exports current Theme JSON, clones/edits it with `Make a Theme from this`, and saves a custom Theme into `$HOME/.thirdshift/console/themes/`.
- `sections-visibility` toggles visible Inbox sections via `Sections` and updates folder navigation.
- `pause-motion` toggles `#pause-motion` (`aria-pressed="true"`) and persists `data-paused="true"` on `<html>`.

## How to get to it (user POV)

- Click `Themes (Shift+T)`, `Sections`, or `Pause motion` in the Console footer (`<footer>`).

## Driving it with Playwright

Preconditions:

- Console started with `--no-sign-in` (full `act` and `admin` scopes) against an isolated `mktemp -d` `HOME` and `Doctor` passed.

- **Switch built-in Theme.** Open `${origin}/`, click `Themes (Shift+T)`. `#themes` opens; select `focus-dark` (or `garden-dark`) in `#theme-name` and click `Apply Theme`. `document.documentElement.dataset.theme` updates and `#look-status` reports `Console look saved.`.
- **Export and save custom Theme.** Click `Make a Theme from this` (`#edit-theme`) and wait for `#theme-document` to contain `-copy` JSON. Click `Save / import JSON` (`#save-theme`). `#theme-editor-status` reports `Theme saved. Choose it above to apply.` and the saved `.json` file exists under `$state_dir/.thirdshift/console/themes/`.
- **Close Themes and toggle Sections.** Click `Close Themes`, click `Sections`, uncheck `Show Resolved`, and click `Save sections`. Wait for `#inbox .folders a[data-group="resolved"]` to become `hidden` (and `#look-status` to update), then click `Close Sections` (`[data-close-sections]`). Reopen `Sections`, re-check `Show Resolved`, click `Save sections` (waiting until `[data-group="resolved"]` is visible again), and click `Close Sections` (`[data-close-sections]`) before interacting outside the modal.
- **Toggle Pause motion.** With dialogs closed, click `#pause-motion`. `aria-pressed` switches to `"true"` and `document.documentElement.dataset.paused` becomes `"true"`.
- **Proof.** Capture desktop `.png` showing the switched Theme or open Themes dialog, and record an `.mp4` walkthrough exercising Theme switch + Sections toggle + Pause motion.

## Gotchas

- `Apply Theme` and `Save / import JSON` require `act` and `admin` scopes respectively; read-only viewers see `Export JSON` only.
- Saving a custom Theme writes to `$HOME/.thirdshift/console/themes/` without applying it automatically; select its name in `#theme-name` and click `Apply Theme` to activate it.
- Submitting `#sections-form` (`Save sections`) sends an async `fetch('/sections')` and leaves `<dialog open id="sections-chooser">` open; wait for `#look-status` or the folder state change, then click `Close Sections` (`[data-close-sections]`) before clicking footer controls such as `Sections` or `#pause-motion`.
