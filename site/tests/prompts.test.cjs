const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { readFile } = require("node:fs/promises");
const { join } = require("node:path");
const { chromium } = require("playwright");

let server, browser, origin;
before(async () => {
  server = createServer(async (request, response) => {
    const path = new URL(request.url, "http://localhost").pathname;
    const file = path === "/prompts/" ? "prompts/index.html" : path.slice(1);
    try {
      const content = await readFile(join(__dirname, "..", file));
      const type = file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : "text/html";
      response.writeHead(200, { "Content-Type": type }).end(content);
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server) await new Promise(resolve => server.close(resolve));
});

async function pageForTest(t, options) {
  const context = await browser.newContext(options);
  t.after(() => context.close());
  const page = await context.newPage();
  await page.route("https://**", route => route.abort());
  return page;
}

test("every headed section starts closed and toggles independently by its heading", async (t) => {
  const page = await pageForTest(t);
  await page.goto(`${origin}/prompts/`);
  const headings = page.locator("main :is(h2, h3, h4)");
  assert.ok(await headings.count() > 30);
  const ids = await headings.evaluateAll(nodes => nodes.map(node => node.id));
  for (const id of ids) {
    assert.equal(await page.locator(`#${id}`).evaluate(heading => {
      const summary = heading.closest("summary");
      return Boolean(summary && summary.parentElement.tagName === "DETAILS" && !summary.parentElement.open);
    }), true, `${id} must control a closed section`);
  }
  assert.equal(await page.locator("main details[open]").count(), 0);
  for (const id of ids) {
    const heading = page.locator(`#${id}`);
    // Open only the containing sections, as a reader does before reaching a nested heading.
    const ancestors = await heading.evaluate(node => {
      const ids = [];
      for (let parent = node.closest("details")?.parentElement; parent; parent = parent.parentElement) {
        if (parent.tagName === "DETAILS") ids.unshift(parent.querySelector("summary :is(h2,h3,h4)").id);
      }
      return ids;
    });
    for (const parent of ancestors) {
      if (!await page.locator(`#${parent}`).evaluate(node => node.closest("details").open)) {
        await page.locator(`#${parent}`).click();
      }
    }
    await heading.click();
    assert.equal(await heading.evaluate(node => node.closest("details").open), true, id);
    const content = heading.locator("xpath=ancestor::summary/parent::details/*[not(self::summary)][1]");
    assert.equal(await content.isVisible(), true, `${id} reveals its content`);
    await heading.click();
    assert.equal(await heading.evaluate(node => node.closest("details").open), false, id);
    assert.equal(await content.isVisible(), false, `${id} hides its content`);
  }
  await page.locator("#command-title").click();
  await page.locator("#prompts-title").click();
  assert.equal(await page.locator("#command-title").evaluate(node => node.closest("details").open), true);
});

test("fragment links reveal the prompt or supporting file and its containing sections", async (t) => {
  const page = await pageForTest(t);
  await page.goto(`${origin}/prompts/#prompt-resume`);
  assert.equal(await page.locator("#prompt-resume .job-text").isVisible(), true);
  await page.goto(`${origin}/prompts/`);
  await page.locator("#licence-title").click();
  await page.locator('a[href="#file-thirdshift-pr-credits-md"]').click();
  const credits = page.locator("#file-thirdshift-pr-credits-md").locator("xpath=ancestor::details[1]/pre");
  assert.equal(await credits.isVisible(), true);
  // A link to the current fragment must reveal a section the reader closed.
  await page.locator("#file-thirdshift-pr-credits-md").click();
  assert.equal(await credits.isVisible(), false);
  await page.locator('a[href="#file-thirdshift-pr-credits-md"]').click();
  assert.equal(await credits.isVisible(), true);
});

test("mobile readers can toggle headings with the keyboard without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  await page.goto(`${origin}/prompts/`);
  assert.equal(await page.locator("main details[open]").count(), 0);
  const prompts = page.locator("#prompts-title").locator("xpath=parent::summary");
  await prompts.focus();
  await page.keyboard.press("Enter");
  assert.equal(await page.locator("#prompt-fresh-title").isVisible(), true);
  const fresh = page.locator("#prompt-fresh-title").locator("xpath=parent::summary");
  await fresh.focus();
  await page.keyboard.press("Space");
  assert.equal(await page.locator("#prompt-fresh .job-text").isVisible(), true);
  await page.keyboard.press("Space");
  assert.equal(await page.locator("#prompt-fresh .job-text").isVisible(), false);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
});
