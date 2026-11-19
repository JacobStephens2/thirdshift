# How it works and Architecture

The engineer's walkthrough (`/how-it-works/`) and system diagrams (`/architecture/`) provide section index navigation, collapsible explanations, supporting reading disclosures, and horizontally scrollable SVG architecture diagrams.

## Sub-features

- `guide-sections` jumps through `01`..`05` via `Guide sections` navigation and unfolds sections `01`..`07`.
- `architecture-figures` jumps across `Figure 1`..`Figure 6` via `Figures` navigation and fits `.arch-scroll` containers on narrow viewports.
- `architecture-reading-more` toggles supporting `.reading-more` folds (`Console and host resources`, etc.).

## How to get to it (user POV)

- Open `$origin/how-it-works/` or `$origin/architecture/` from the homepage or site header navigation.

## Driving it with Playwright

Preconditions:

- `serve-site.cjs` is running on `$origin` and `Doctor` passed.

- **Navigate and unfold `/how-it-works/`.** Open `${origin}/how-it-works/`. Click `03 / Agent session` in `nav[aria-label="Guide sections"]`; `main.js` automatically opens its containing `<details class="fold">` so `#session` becomes visible. Clicking `#session-title` then folds it closed, and clicking `#session-title` again reopens it. Verify `document.documentElement.scrollWidth <= innerWidth`.
- **Navigate figures and disclosures on `/architecture/`.** Open `${origin}/architecture/`. Click `Figure 4` in `nav[aria-label="Figures"]` (`#spec-run`), then click `summary:has-text("Console and host resources")` on `#factory details.reading-more:has(> summary:has-text("Console and host resources"))`. Verify the disclosure gains `[open]` and its first paragraph (`p`) becomes visible, while narrow `390×844` layout keeps page scroll width within viewport.
- **Proof.** Capture `1440×900` `.png` of `/architecture/` (or `/how-it-works/`) with a section open and record a `.mp4` walkthrough navigating guide sections and architecture figures/folds.

## Gotchas

- With JavaScript enabled, clicking a `Guide sections` link (`#handoff`, `#session`, `#delivery`, etc.) automatically unfolds its `<details class="fold">` via `main.js`; clicking the `<summary>` heading immediately afterward toggles it closed. Without JavaScript, the `<details>` stays folded until its `<summary>` is clicked.
- `/how-it-works/` and `/architecture/` sections and diagrams are hand-written HTML; check that all internal `/docs/#...` and `/prompts/#...` links resolve against current targets.
- Below desktop breakpoints, `.arch-scroll` intentionally scrolls horizontally inside its frame rather than shrinking SVG text; assert overflow on `document.documentElement`, not `.arch-scroll` inner SVG width.
