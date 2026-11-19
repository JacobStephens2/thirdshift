# Homepage and press line

The homepage (`/`) introduces thirdshift, lets readers copy the install command, toggle compact reading and workflow disclosures, and step through a real recorded Ticket run on the interactive 4-step press line.

## Sub-features

- `home-install-copy` copies the curl command (or selects it when clipboard access is denied) and toggles `Show the full command`.
- `home-reading-folds` toggles `How bug issues are handled` and `Skills and Processes in detail`.
- `home-press-line` switches `.press-controls` between `Step 1`..`Step 4`, updates lit columns and `.press-term ol`, and toggles `Pause`/`Play`.
- `home-workflows` expands and collapses the seven `#grows details.workflow` cards without horizontal overflow.

## How to get to it (user POV)

- Open `$origin/` in desktop (`1440×900`) or mobile (`390×844`) viewports.
- Click `Copy`, disclosure `<summary>` headings, `Step 1`..`Step 4`, or `Pause`/`Play`.

## Driving it with Playwright

Preconditions:

- `serve-site.cjs` is running on `$origin` and `Doctor` passed.
- Playwright aborts external `https://**` requests (`fonts.googleapis.com` / `fonts.gstatic.com` fail only as aborted external requests, not `5xx`).

- **Install Copy and full command disclosure.** Open `${origin}/`. Click button `Copy` inside `.install` and wait for the button text to change to `Copied` or `Selected` (the click handler awaits `navigator.clipboard.writeText`). Click summary `Show the full command`; full GitHub release installer command becomes visible.
- **Step the interactive press line.** Scroll to `#how-a-run-works`. Click `Step 2 · Implement` (`[data-step="2"]`), then `Step 3 · Review` (`[data-step="3"]`), then `Step 4 · Finish` (`[data-step="4"]`). Each button sets `aria-current="step"`, `.unit-col.lit` advances, and `.press-term ol li` replays the step's stderr lines. Click `Pause` and confirm button text toggles to `Play`.
- **Expand a reading disclosure and workflow.** Click summary `Skills and Processes in detail`, then click summary `Ticket run` inside `#grows`. Both contents become visible and `document.documentElement.scrollWidth <= innerWidth` holds.
- **Proof.** Capture `1440×900` `.png` of the homepage press line / disclosures and record a Phase 2 `.mp4` walkthrough driving `Show the full command` -> `Step 2` -> `Step 3` -> `Step 4` -> `Pause` -> `Ticket run` workflow fold.

## Gotchas

- `reducedMotion: 'reduce'` hides `.press-controls`, `.press-steps`, and `.press-term`; leave reduced motion unset (`no-preference`) when driving the stepped press line.
- Abort external `https://**` font stylesheets in Playwright and filter `net::ERR_FAILED` for external URLs out of failed-request assertions so offline runs stay clean.
- Scroll `#how-a-run-works` into view before checking step buttons; `IntersectionObserver` also updates the active step on scroll.
