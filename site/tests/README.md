# Browser tests

The site serves checked-in static files; deployment needs no build step. To run its browser tests:

```sh
cd site
npm ci
npx playwright install --with-deps chromium
npm test
```

The tests start a local server on an unused port and close it and the browser
before exiting. CI runs them in the `site-test` job.

After a click, key press or navigation, a test waits for the state it expects
rather than reading it once: `locator.waitFor({ state })` for visibility, an open
fold or an inserted element, `page.waitForURL` for the address, and the suite's
`waitUntil` for a reading no locator expresses, such as scroll position. A
snapshot read such as `isVisible()` races the page's handlers. The waits use
Playwright's default timeout, and the runner adds no retries.

The page tests also consume the site without JavaScript to check canonical URLs,
distinct search metadata, homepage site-name structured data and sitemap discovery
through `robots.txt`. When adding a public page, give it a canonical URL and unique
title and description, then include it in `site/sitemap.xml` and the page tests.

The user manual's source is `docs/guide.md` at the repository root. After editing
it or `site/generate-docs.mjs`, run `cd site && npm run docs` and commit the generated
`site/docs/index.html` in the same pull request. `npm test` first runs
`npm run docs:check` to reject stale output. The browser tests check navigation,
section anchors, command escaping and reading at mobile widths without JavaScript.
