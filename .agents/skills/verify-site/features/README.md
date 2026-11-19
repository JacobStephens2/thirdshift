# Site verification map

Maintained source for verifying the `thirdshift` static website (`site/`). Read [../SKILL.md](../SKILL.md) and `verify-ui` (Factory: `thirdshift-verify-ui`) for isolation, walkthrough recording, and transcoding rules, then follow the matching feature recipe.

## Site preconditions

- Follow [../SKILL.md](../SKILL.md) for `docs:check`, `serve-site.cjs`, `Doctor`, offline route stubbing/aborting, and `Cleanup`, and `verify-ui` (Factory: `thirdshift-verify-ui`) for recording/transcoding mechanics.
- Verify `javaScriptEnabled: false` alongside JavaScript behavior where `<details>` or anchor reveal applies, and check `document.documentElement.scrollWidth <= innerWidth` at `390×844` mobile.
- Record any unmet precondition in the feature's `Gotchas` rather than weakening assertions.

## Feature entry contract

Each feature file starts with an H1 title, a summary paragraph, and four H2 headings in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with Playwright`, and `Gotchas`.

## Features

- [Homepage and press line](./homepage-press-line.md) covers install Copy, reading disclosures, workflow folds, and the interactive 4-step press line.
- [Docs manual navigation](./docs-manual-navigation.md) covers `/docs/` section disclosures, nested folds, and anchor reveal.
- [Prompts and skills](./prompts-and-skills.md) covers `/prompts/` section toggles, skill `SKILL.md` folds, and fragment links.
- [How it works and Architecture](./how-it-works-and-architecture.md) covers `/how-it-works/` and `/architecture/` navigation, folds, and diagram containers.
- [Videos lazy embed](./videos-lazy-embed.md) covers `/videos/` zero-preload thumbnails and click/keyboard YouTube-nocookie iframe activation.
