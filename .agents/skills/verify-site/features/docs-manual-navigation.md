# Docs manual navigation

The generated manual (`/docs/`) starts with all heading sections folded and unfolds target sections and nested subsections on summary clicks or fragment links.

## Sub-features

- `docs-toc-reveal` follows links in `nav[aria-label="Manual sections"]` and automatically opens target `<details>` ancestors.
- `docs-nested-folds` toggles top-level `h2` and nested `h3`/`h4` sections by mouse and keyboard (`Enter`/`Space`).
- `docs-deep-link` opens nested targets directly when loading `/docs/#<fragment>`.

## How to get to it (user POV)

- Follow `Documentation` from `/` or open `$origin/docs/`.
- Click any link in `Manual sections` or click section headings directly.

## Driving it with Playwright

Preconditions:

- `npm run docs:check --prefix site` passed, `serve-site.cjs` is running on `$origin`, and `Doctor` passed.

- **Confirm initial folded state.** Navigate to `${origin}/docs/`. Zero `main details[open]` elements exist on load.
- **Navigate via Manual sections TOC.** Click link `Install` inside `nav[aria-label="Manual sections"]`. URL hash becomes `#install` and `#install > pre` becomes visible.
- **Unfold nested subsections.** Click heading `What a Ticket run does`, then `The Claim`, then `When the Claim ends`. Each nested section reveals its content cleanly while unrelated sections remain folded.
- **Verify deep link.** Navigate directly to `${origin}/docs/#ui-verification-and-visual-evidence`. The `UI verification and Visual evidence` section opens automatically and scrolls into view.
- **Proof.** Capture `1440×900` `.png` showing an unfolded manual section and record a `.mp4` walkthrough navigating `Install` -> `What a Ticket run does` -> `The Claim` -> `#ui-verification-and-visual-evidence`.

## Gotchas

- `site/docs/index.html` is generated from `docs/guide.md` by `site/generate-docs.mjs`; run `npm run docs --prefix site` before serving if `docs/guide.md` was edited.
- Test both `javaScriptEnabled: true` (fragment auto-unfold via `main.js`) and `false` (native `<details>` clicking without JS).
