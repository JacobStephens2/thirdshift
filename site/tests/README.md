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
