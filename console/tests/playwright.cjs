const playwright = require('playwright');

// Playwright waits 30 s by default before failing a locator. Console pages
// answer in well under a second, so a broken test would otherwise stall the
// run for half a minute per step. Every context and page these tests open
// waits at most `DEFAULT_TIMEOUT` unless a call asks for longer.
const DEFAULT_TIMEOUT = 5000;

function quick(browser) {
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (...args) => {
    const context = await newContext(...args);
    context.setDefaultTimeout(DEFAULT_TIMEOUT);
    return context;
  };
  const newPage = browser.newPage.bind(browser);
  browser.newPage = async (...args) => {
    const page = await newPage(...args);
    page.context().setDefaultTimeout(DEFAULT_TIMEOUT);
    page.setDefaultTimeout(DEFAULT_TIMEOUT);
    return page;
  };
  return browser;
}

const chromium = Object.create(playwright.chromium);
chromium.launch = async (...args) => quick(await playwright.chromium.launch(...args));

// Keep the viewer active across full-page navigation when testing retained answers.
async function keepInboxSubscribed(page, origin) {
  const context = page.context();
  const watching = await context.browser().newContext({ storageState: await context.storageState() });
  const subscriber = await watching.newPage();
  const [events] = await Promise.all([
    subscriber.waitForResponse(response => response.url() === `${origin}/events`),
    subscriber.goto(origin),
  ]);
  if (!events.ok()) throw new Error(`Inbox subscription returned HTTP ${events.status()}`);
  await page.bringToFront();
}

module.exports = { ...playwright, chromium, DEFAULT_TIMEOUT, keepInboxSubscribed };
