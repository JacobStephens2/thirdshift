// Run the existing notification assertion with a write held after truncation:
// node --require ./console/reproductions/notification-record-write.cjs --test --test-name-pattern '^several tabs notify' console/tests/notifications.test.cjs
const fs = require('node:fs/promises');
const { chromium } = require('../tests/playwright.cjs');
const pages = [];
const launch = chromium.launch.bind(chromium);
chromium.launch = async (...args) => {
  const browser = await launch(...args);
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (...args) => {
    const context = await newContext(...args);
    context.on('page', page => pages.push(page));
    return context;
  };
  return browser;
};
const writeFile = fs.writeFile;
fs.writeFile = async (path, text, ...options) => {
  if (typeof text !== 'string' || !text.includes('Changed without a count rise')) {
    return writeFile(path, text, ...options);
  }
  // This deliberately exposes writeFile's truncate/write interval. With
  // replaceRecord only the temporary file is empty; the old record stays whole.
  await writeFile(path, '', ...options);
  const expected = String(path).endsWith('.command.json') ? 1 : 2;
  try {
    for (const page of pages) {
      const inbox = new URL('/', page.url()).href;
      const refreshed = page.waitForResponse(response => response.url() === inbox && response.ok());
      await page.evaluate(() => notificationStream.dispatchEvent(new Event('commands')));
      await (await refreshed).finished();
      await page.waitForFunction(expected => document.title.startsWith(`${expected} need you`), expected);
    }
    await pages[0].waitForFunction(expected => JSON.parse(localStorage.getItem('thirdshift:console:notification-count'))?.count === expected, expected);
    console.log(`Record write held: ${pages.length} pages and shared notification count observed ${expected}`);
  } finally {
    await writeFile(path, text, ...options);
  }
};
