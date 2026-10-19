const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, writeFile, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright");

let home, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), "thirdshift-console-"));
  const configDir = join(home, ".thirdshift");
  const records = join(configDir, "logs/acme/widgets/commands/issue");
  await mkdir(records, { recursive: true });
  await writeFile(join(configDir, "config.toml"), '[console]\nport = 0\nother_consoles = ["https://other.example.test"]\n');
  for (let number = 1; number <= 10; number++) {
    const now = new Date().toISOString();
    await writeFile(join(records, `${number}-fixture.command.json`), JSON.stringify({
      version: 1, kind: "Spec run", repository: "acme/widgets", hostname: "fixture",
      issue: { number, url: `https://github.com/acme/widgets/issues/${number}`, title: "A long title that should wrap on a phone, even with <script>alert('unsafe')</script>" },
      process_id: 0, process_start: "gone", harness: "codex", model: "gpt-6", effort: "high",
      started_at: now, ended_at: now, stages: [], session_log: null,
      outcome: "failed", cause: `distinct fixture failure ${number}`, issue_branch: null, pull_request: null,
      tickets: [{ issue: { number: 98, url: "https://github.com/acme/widgets/issues/98", title: "Build it" }, state: "waiting", blockers: [{ number: 97, url: "https://github.com/acme/widgets/issues/97", title: "First" }] }]
    }));
  }
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift"), ["console", "--no-sign-in"], {
    env: { ...process.env, HOME: home }, stdio: ["ignore", "pipe", "inherit"]
  });
  exited = once(server, "exit");
  const lines = createInterface({ input: server.stdout });
  try {
    [origin] = await once(lines, "line", { signal: AbortSignal.timeout(10000) });
    assert.match(origin, /^http:\/\/127\.0\.0\.1:\d+$/);
  } finally { lines.close(); }
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server && server.exitCode === null) server.kill();
  await exited;
  if (home) await rm(home, { recursive: true, force: true });
});

async function pageFor(t, options) {
  const context = await browser.newContext(options);
  t.after(() => context.close());
  const page = await context.newPage();
  return page;
}

test("Inbox folds, loads only its own resources and fits a 390 px phone", async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  const remote = [], errors = [];
  page.on("request", request => { if (!request.url().startsWith(origin)) remote.push(request.url()); });
  page.on("pageerror", error => errors.push(error.message));
  const response = await page.goto(origin);
  assert.equal(response.status(), 200);
  await page.evaluate(() => document.fonts.ready);
  assert.equal(await page.locator('details[open]').count(), 1);
  assert.equal(await page.locator('details[open]').getAttribute("data-group"), "needs you");
  assert.match(await page.locator(".plan").first().textContent(), /Ticket #98 · waiting on #97/);
  assert.equal(await page.locator('.command .state').first().textContent(), "Failed");
  assert.equal(await page.locator(".command script").count(), 0);
  for (const summary of await page.locator("summary").all()) {
    await summary.click();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  assert.deepEqual(remote, []);
  assert.deepEqual(errors, []);
  assert.equal(await page.evaluate(() => document.fonts.check('14px "IBM Plex Sans"') && document.fonts.check('12px "IBM Plex Mono"')), true);
});

test("all section headings fit above the fold on a 14 inch laptop with a busy Needs you", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(origin);
  await page.evaluate(() => document.fonts.ready);
  assert.equal(await page.locator('[data-group="needs you"][open] .command').count(), 10);
  for (const summary of await page.locator("summary").all()) {
    const rect = await summary.boundingBox();
    assert.ok(rect.y >= 0 && rect.y + rect.height < 900, `${await summary.textContent()}: ${JSON.stringify(rect)}`);
  }
});

test("the Inbox works without JavaScript and Focus auto follows system light and dark", async t => {
  const page = await pageFor(t, { javaScriptEnabled: false, viewport: { width: 390, height: 844 }, colorScheme: "light" });
  await page.goto(origin);
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 1);
  await page.locator('[data-group="working"] > summary').click();
  assert.equal(await page.locator('[data-group="working"][open]').count(), 1);
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(247, 247, 245)");
  await page.emulateMedia({ colorScheme: "dark" });
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
});
