# Waterfall and Logs

Waterfall draws a Command's spans across one shared time axis with slowest-call ranking, zoom, keyboard navigation, and direct links into the Session trace conversation reader.

## Sub-features

- `waterfall-expand` expands the span tree through `Ticket runs`, `Stages`, `Sessions`, and `Every call`.
- `waterfall-slowest-and-zoom` selects a Slowest call, opens its span Peek, and zooms the axis around that call.
- `waterfall-logs-reader` opens the Session trace conversation (`Open in conversation` or `/?page=logs`).

## How to get to it (user POV)

- Choose `4 Waterfall` in `nav[aria-label="Pages"]` (or press `4` or `g` then `f`).
- Click `Waterfall` inside a Command's Inbox Peek pane.
- Choose `6 Logs` (`6` or `g` then `o`) to browse Command logs and Session traces.

## Driving it with Playwright

Preconditions:

- Console started with `--no-sign-in` against `seed-home.cjs` (which seeds `.spans.jsonl` and `.logs.jsonl` for `widgets#7`, `#8`, `#9` and `gadgets#10`) and `Doctor` passed.

- **Open Waterfall for a Command.** Navigate to `${origin}/waterfall?command=widgets%238`. `#span-tree` renders and `#slowest-calls li` lists ranked tool calls (`tool · Bash cargo test --test search`, `tool · Bash cargo clippy --all-targets`, etc.).
- **Expand all spans.** Click button `Every call` inside `.span-controls`. Tool items `.span-item[data-kind="tool"]` become visible inside `#span-tree`.
- **Inspect Slowest call and Zoom.** Click the first button in `#slowest-calls button[data-reveal-span]`. `#peek` opens showing the span card (`span-detail-*`), then click button `Zoom timeline around this call`. `#span-zoom` becomes visible with `Zoom out to whole Command`; click `#span-zoom` button `Zoom out to whole Command` (`#span-zoom button`) to restore the full axis (both `#span-zoom` and `#peek` render a zoom-out button while zoomed).
- **Open Session conversation.** Click link `Open in conversation` inside `#peek-content` (or navigate to `${origin}/logs`, expand `details.log-group[data-repository="acme/widgets"] > summary` when multiple repositories are logged, and click `li.log-row:has-text("8-verify-implement") a:has-text("Session trace")`). The Session conversation view renders the seeded agent message (`Inspecting acme/widgets#8`) and tool execution block without `pageerror`.
- **Proof.** Capture desktop `1440×900` `.png` with `Every call` expanded and span Peek open, and record a Phase 2 `.mp4` walkthrough of expanding -> Slowest call Peek -> Zoom -> Zoom out.

## Gotchas

- `Session` rows start folded in `#span-tree`; tool and model calls are absent from the DOM until `Every call` or `Sessions` (or `ArrowRight` / `Slowest calls` reveal) expands them.
- Navigating to `/waterfall` without `?command=` selects the newest Command automatically; pass `?command=widgets%238` when targeting a specific fixture.
- Clicking `Zoom timeline around this call` (and `Zoom out to whole Command`) morphs timeline bars via `Idiomorph.morph` (`src/console/assets/spans.js`), emitting expected `style-src 'self'` CSP inline-style console errors during Phase 1 default-CSP exploration; allow only those zoom inline-style CSP messages during `/waterfall` zoom checks while keeping strict CSP error assertions on all other routes.
- On `/logs`, repository groups fold closed whenever more than one repository has logs (`acme/widgets` and `acme/gadgets`); expand `details.log-group > summary` before clicking a log row link.
