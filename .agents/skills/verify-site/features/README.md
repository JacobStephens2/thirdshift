# Site verification map

Maintained source for verifying the `thirdshift` static website (`site/`). Read this index before driving, then follow the matching feature recipe.

## Baseline preconditions

- Run `npm run docs:check --prefix site` before serving.
- Run inside one `( ... )` subshell with `trap cleanup EXIT` registered before launching `serve-site.cjs` or any browser daemon.
- Read `$origin` (`http://127.0.0.1:<port>`) from `serve-site.cjs` stdout and run `Doctor` before driving.
- Intercept external `https://**` requests in Playwright (`route.abort()`, or fulfill YouTube embeds locally) so verification stays local and deterministic.

## Driving conventions

- Prefer ARIA roles, accessible names, and explicit headings over positional selectors.
- Test both `1440×900` desktop and `390×844` mobile when layout, disclosures, or diagrams change, and verify `document.documentElement.scrollWidth <= innerWidth`.
- Test progressive enhancement (`javaScriptEnabled: false`) alongside interactive JavaScript behavior where pages use `<details>` or anchor navigation.
- Wait for visible state (`locator.waitFor({ state: 'visible' })`) or URL changes rather than arbitrary sleeps outside brief `~300–500 ms` walkthrough pacing beats.

## Proof and skip reporting

- Write proof captures with `mktemp` outside `$state_dir` (`/tmp/after-desktop.XXXXXX.png` and H.264 `yuv420p` `+faststart` `<= 8 MB` `/tmp/after-desktop.XXXXXX.mp4`).
- Confirm captured PNGs and transcoded MP4s survive cleanup and remain valid media files.
- Record any unreachable prerequisite in the feature's `Gotchas` rather than weakening assertions.

## Feature entry contract

Each feature file starts with an H1 title, a summary paragraph, and four H2 headings in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with Playwright`, and `Gotchas`.

## Features

- [Homepage and press line](./homepage-press-line.md) covers install Copy, reading disclosures, workflow folds, and the interactive 4-step press line.
- [Docs manual navigation](./docs-manual-navigation.md) covers `/docs/` section disclosures, nested folds, and anchor reveal.
- [Prompts and skills](./prompts-and-skills.md) covers `/prompts/` section toggles, skill `SKILL.md` folds, and fragment links.
- [How it works and Architecture](./how-it-works-and-architecture.md) covers `/how-it-works/` and `/architecture/` navigation, folds, and diagram containers.
- [Videos lazy embed](./videos-lazy-embed.md) covers `/videos/` zero-preload thumbnails and click/keyboard YouTube-nocookie iframe activation.
