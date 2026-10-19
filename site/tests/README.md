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

The user manual's source is `docs/guide.md` at the repository root. After editing
it or `site/generate-docs.mjs`, run `cd site && npm run docs` and commit the generated
`site/docs/index.html` in the same pull request. `npm test` first runs
`npm run docs:check` to reject stale output. The browser tests check navigation,
section anchors, command escaping and reading at mobile widths without JavaScript.
