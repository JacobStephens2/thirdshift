# Browser tests

The site still serves static files with no build step. To run its browser tests:

```sh
cd site
npm ci
npx playwright install --with-deps chromium
npm test
```

The tests start a local server on an unused port and close it and the browser
before exiting. CI runs them in the `site-test` job.

`npm test` also checks that the static documentation pages match `docs/guide/*.md`.
After editing those Markdown sources or `site/build-docs.mjs`, run `npm run docs`
from `site/` and commit the generated `site/docs/*/index.html` pages (and
`site/docs/index.html`). `npm run docs:check` checks freshness without writing.
The published site needs no build step or client-side Markdown renderer.
