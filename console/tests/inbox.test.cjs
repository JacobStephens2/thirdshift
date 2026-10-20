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
    await writeFile(join(records, `${number}-fixture.log`), `Full Command output ${number}\n`);
  }
  const security = join(configDir, 'logs/acme/widgets/commands/secure');
  await mkdir(security, { recursive: true });
  const session = join(security, 'secure-fixture-security-audit.jsonl');
  await writeFile(session, 'Security Session output\n');
  await writeFile(join(security, 'secure-fixture.log'), 'Security Command output\n');
  await writeFile(join(security, 'secure-fixture.command.json'), JSON.stringify({
    version: 1, kind: 'Security run', name: 'secure-fixture', repository: 'acme/widgets',
    hostname: 'fixture', process_id: 0, process_start: 'gone', harness: 'codex',
    started_at: '2026-01-01T12:00:00Z', ended_at: '2026-01-01T13:00:00Z',
    stages: [], session_log: session, outcome: 'findings recorded',
    security_findings: [{ title: 'Bound <input> before storage', severity: 'high', url: 'https://github.com/acme/widgets/security/advisories/GHSA-test' }]
  }));
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

test("explicit Focus light and dark override the system preference", async t => {
  const page = await pageFor(t, { javaScriptEnabled: false });
  const config = join(home, ".thirdshift/config.toml");
  try {
    for (const [theme, system, expected] of [
      ["focus-light", "dark", "rgb(247, 247, 245)"],
      ["focus-dark", "light", "rgb(18, 18, 20)"]
    ]) {
      await writeFile(config, `[console]\nport = 0\ntheme = "${theme}"\n`);
      await page.emulateMedia({ colorScheme: system });
      await page.goto(origin);
      assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), expected);
    }
  } finally { await writeFile(config, "[console]\nport = 0\n"); }
});

test("every keyboard action preserves list nodes and Escape closes one layer at a time", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(origin);
  await page.evaluate(() => {
    window.rowsBefore = [...document.querySelectorAll('.command')];
    window.listChanges = [];
    for (const list of document.querySelectorAll('.commands')) {
      new MutationObserver(records => window.listChanges.push(...records.map(r => r.type)))
        .observe(list, { childList: true, subtree: true });
    }
  });
  const selected = page.locator('.is-selected');
  await page.keyboard.press('j');
  assert.equal(await selected.evaluate(el => el.tagName), 'SUMMARY');
  await page.keyboard.press('k');
  assert.equal(await selected.textContent().then(s => s.trim().startsWith('Resolved')), true);
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 0);
  await page.keyboard.press('Space');
  await page.keyboard.press('j');
  assert.equal(await selected.getAttribute('class').then(s => s.includes('command')), true);
  await page.keyboard.press('k');
  assert.equal(await selected.evaluate(el => el.tagName), 'SUMMARY');
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  await page.locator('#peek dt').first().waitFor();
  const peekBox = await page.locator('#peek').boundingBox();
  const mainBox = await page.locator('main').boundingBox();
  assert.ok(peekBox.x >= mainBox.x + mainBox.width, 'peek is beside the list');
  await page.keyboard.press('?');
  assert.equal(await page.locator('#keys').evaluate(el => el.open), true);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#keys').evaluate(el => el.open), false);
  assert.equal(await page.locator('#peek').isVisible(), true);
  await page.keyboard.press('g');
  assert.equal(await page.locator('#jumps').isVisible(), true);
  assert.match(await page.locator('#jumps').textContent(), /History/);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#jumps').isVisible(), false);
  assert.equal(await page.locator('#peek').isVisible(), true);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#peek').isVisible(), false);
  assert.equal(await selected.count(), 1);
  await page.keyboard.press('Escape');
  assert.equal(await selected.count(), 0);
  for (const [letter, group] of [['r', 'ready for review'], ['l', 'landing'], ['d', 'done'], ['v', 'resolved']]) {
    await page.keyboard.press('g');
    await page.keyboard.press(letter);
    assert.equal(await selected.evaluate(el => el.parentElement.dataset.group), group);
    assert.equal(await selected.evaluate(el => el.parentElement.open), true);
  }
  await page.keyboard.press('g');
  await page.keyboard.press('w');
  assert.equal(await page.locator('[data-group="working"][open]').count(), 1);
  await page.keyboard.press('g');
  await page.keyboard.press('n');
  await page.keyboard.press('j');
  await page.keyboard.press('z');
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 0);
  await page.keyboard.press('Space');
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  await page.locator('#peek dt').first().waitFor();
  await page.keyboard.press('Space');
  assert.equal(await page.locator('#peek').isVisible(), false);
  assert.deepEqual(await page.evaluate(() => window.listChanges), []);
  assert.equal(await page.evaluate(() => window.rowsBefore.every((row, i) => row === document.querySelectorAll('.command')[i])), true);
  await page.keyboard.press('Enter');
  await page.waitForURL('**/log?*');
  assert.match(await page.locator('body').textContent(), /Full Command output/);
});

test("page keys, jump letters and URLs survive reload and the peek is a phone bottom sheet", async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  await page.goto(origin);
  for (const [key, name] of [['2', 'lanes'], ['3', 'stages'], ['5', 'history'], ['6', 'logs'], ['7', 'security'], ['1', 'inbox']]) {
    await page.keyboard.press(key);
    await page.waitForURL(`**/?page=${name}`);
    await page.reload();
    assert.equal(await page.locator('main h1, main h2').first().textContent(), name[0].toUpperCase() + name.slice(1));
    assert.equal(await page.locator('#peek').isVisible(), false);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  for (const key of ['4']) {
    await page.keyboard.press(key);
    assert.equal(new URL(page.url()).searchParams.get('page'), 'inbox');
    assert.equal(await page.locator(`[data-key="${key}"]`).count(), 0);
  }
  for (const [letter, name] of [['a', 'lanes'], ['t', 'stages'], ['h', 'history'], ['o', 'logs'], ['s', 'security'], ['i', 'inbox']]) {
    await page.keyboard.press('g');
    await page.keyboard.press(letter);
    await page.waitForURL(`**/?page=${name}`);
  }
  await page.keyboard.press('j');
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  await page.locator('#peek dt').first().waitFor();
  const box = await page.locator('#peek').boundingBox();
  assert.ok(box.y > 0 && Math.abs(box.y + box.height - 844) < 2);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.reload();
  assert.equal(await page.locator('#peek').isVisible(), false);
});

test('History filters and Security findings and sessions work from a phone', async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  await page.goto(`${origin}/?page=history`);
  assert.equal(await page.locator('.command').count(), 11);
  await page.locator('select[name=outcome]').selectOption('failed');
  await page.locator('select[name=outcome]').focus();
  await page.keyboard.press('7');
  assert.equal(new URL(page.url()).searchParams.get('page'), 'history', 'select keeps normal keys');
  await page.locator('select[name=repository]').selectOption('acme/widgets');
  await page.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.waitForURL('**/\?page=history&outcome=failed&repository=*');
  assert.equal(await page.locator('.command').count(), 10);
  await page.goto(`${origin}/?page=security`);
  assert.equal(await page.locator('.command').count(), 1);
  assert.match(await page.locator('[aria-label="Security findings"]').textContent(), /Bound <input> before storage.*high/);
  assert.equal(await page.locator('.command input').count(), 0);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.locator('.peek-link').click();
  await page.locator('#peek dt').first().waitFor();
  assert.match(await page.locator('#peek').textContent(), /Bound <input> before storage/);
  await page.locator('#peek a').filter({ hasText: 'Session log:' }).click();
  await page.waitForURL('**/log?*');
  assert.match(await page.locator('body').textContent(), /Security Session output/);
});

test("Enter activates the focused Security session link", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(`${origin}/?page=security`);
  await page.locator('.peek-link').click();
  const sessionLink = page.locator('#peek a').filter({ hasText: 'Session log:' });
  await sessionLink.waitFor();
  await sessionLink.focus();
  await page.keyboard.press('Enter');
  await page.waitForURL('**/log?*');
  assert.ok(new URL(page.url()).searchParams.has('session'),
    `focused Session link opened the Command log instead: ${page.url()}`);
  assert.match(await page.locator('body').textContent(), /Security Session output/);
});

test('Enter and Space activate focused peek buttons', async t => {
  const page = await pageFor(t);
  await page.goto(`${origin}/?page=security`);
  for (const key of ['Enter', 'Space']) {
    await page.locator('.peek-link').click();
    await page.locator('#peek dt').first().waitFor();
    await page.locator('[data-close-peek]').focus();
    await page.keyboard.press(key);
    assert.equal(await page.locator('#peek').isVisible(), false);
    assert.equal(new URL(page.url()).searchParams.get('page'), 'security');
  }
});

test('Logs rows open the shared reader and page keys work from the reader', async t => {
  const page = await pageFor(t);
  await page.goto(`${origin}/?page=logs`);
  await page.keyboard.press('j');
  assert.equal(await page.locator('.is-selected.log-row').count(), 1);
  await page.keyboard.press('z');
  await page.keyboard.press('Enter');
  await page.waitForURL(/\/logs\/[a-f0-9]+$/);
  await page.keyboard.press('5');
  await page.waitForURL('**/?page=history');
  await page.keyboard.press('g');
  await page.keyboard.press('o');
  await page.waitForURL('**/?page=logs');
});
