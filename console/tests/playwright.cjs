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

module.exports = { ...playwright, chromium, DEFAULT_TIMEOUT };
