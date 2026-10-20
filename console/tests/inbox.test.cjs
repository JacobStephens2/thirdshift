const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, readFile, writeFile, appendFile, utimes, rm } = require("node:fs/promises");
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

test("push morphs keep the selected row, folds and scroll without a reload", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(origin);
  const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ }) });
  await page.locator('[data-group="done"] > summary').click();
  await row.click();
  await page.evaluate(() => {
    window.visitMarker = "same document";
    window.selectedRow = document.activeElement;
    document.querySelector('.commands').scrollTop = 120;
  });
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const fixture = JSON.parse(await readFile(path, 'utf8'));
  fixture.issue.title = 'Advanced while you watched';
  await writeFile(path, JSON.stringify(fixture));
  await page.getByText('Advanced while you watched', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => document.activeElement === window.selectedRow && document.activeElement.matches('.command')), true);
  assert.equal(await page.evaluate(() => window.visitMarker), "same document");
  assert.equal(await page.locator('[data-group="done"][open]').count(), 1);
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 1);
  assert.equal(await page.locator('[data-group="needs you"] .commands').evaluate(el => el.scrollTop), 120);
  assert.deepEqual(errors, []);
});

test("tab title and favicon follow Needs you through pushes", async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  assert.equal(await page.title(), '10 need you · thirdshift Console');
  assert.equal(await page.locator('link[rel="icon"]').getAttribute('href'), '/assets/favicon-urgent.svg');
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const fixtures = [];
  try {
    for (let number = 1; number <= 10; number++) {
      const path = join(dir, `${number}-fixture.command.json`);
      const original = await readFile(path, 'utf8');
      fixtures.push([path, original]);
      const fixture = JSON.parse(original);
      fixture.outcome = 'merged';
      await writeFile(path, JSON.stringify(fixture));
    }
    await page.waitForFunction(() => document.title === '0 need you · thirdshift Console');
    assert.equal(await page.locator('link[rel="icon"]').getAttribute('href'), '/assets/favicon.svg');
  } finally { for (const [path, original] of fixtures) await writeFile(path, original); }
});

test("new since the last visit survives browser storage and clears after looking", async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  await page.waitForFunction(() => localStorage.getItem('thirdshift:console:last-look') !== null);
  await page.goto('about:blank');
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const path = join(dir, '201-fixture.command.json');
  try {
    const fixture = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
    fixture.issue.number = 201;
    fixture.started_at = fixture.ended_at = new Date().toISOString();
    fixture.cause = 'New problem after the last visit';
    await writeFile(path, JSON.stringify(fixture));
    await page.goto(origin);
    const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#201$/ }) });
    await row.locator('.new').waitFor();
    assert.equal(await page.locator('.new:visible').count(), 1);
    await page.reload();
    assert.equal(await page.locator('.new:visible').count(), 0);
  } finally { await rm(path, { force: true }); }
});

test("cutting the open stream falls back to polling and catches up", async t => {
  const page = await pageFor(t);
  await page.addInitScript(() => {
    const NativeSource = EventSource;
    window.EventSource = class extends NativeSource {
      constructor(...args) { super(...args); window.inboxStream = this; }
    };
  });
  await page.goto(origin);
  await page.waitForFunction(() => window.inboxStream?.readyState === EventSource.OPEN);
  await page.evaluate(() => {
    window.inboxStream.close();
    window.inboxStream.dispatchEvent(new Event('error'));
  });
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  try {
    const fixture = JSON.parse(original);
    fixture.issue.title = 'Caught up by polling';
    await writeFile(path, JSON.stringify(fixture));
    await page.getByText('Caught up by polling', { exact: true }).waitFor({ timeout: 12000 });
    assert.equal(await page.evaluate(() => window.inboxStream.readyState), 2);
  } finally { await writeFile(path, original); }
});

for (const storage of ['blocked', 'malformed']) {
  test(`live Inbox works with ${storage} browser storage`, async t => {
    const page = await pageFor(t);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(mode => {
      if (mode === 'blocked') Object.defineProperty(window, 'localStorage', { get() { throw new Error('Storage blocked'); } });
      else {
        localStorage.setItem('thirdshift:console:folded', 'false');
        localStorage.setItem('thirdshift:console:last-look', '{broken');
      }
    }, storage);
    await page.goto(origin);
    const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
    const original = await readFile(path, 'utf8');
    try {
      const fixture = JSON.parse(original);
      fixture.issue.title = `Still live with ${storage} storage`;
      await writeFile(path, JSON.stringify(fixture));
      await page.getByText(fixture.issue.title, { exact: true }).waitFor();
      assert.equal(await page.locator('[data-group="needs you"][open]').count(), 1);
      assert.deepEqual(errors, []);
    } finally { await writeFile(path, original); }
  });
}

test("heartbeat beats only for server-observed log growth and is flat when Silent", { skip: process.platform !== 'linux' }, async t => {
  // This fixture borrows the Console's real native process identity. HTTP
  // coverage uses an actual held Run on both Linux and macOS.
  const stat = await readFile(`/proc/${server.pid}/stat`, 'utf8');
  const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const path = join(dir, '202-fixture.command.json');
  const log = join(dir, '202-fixture.log');
  const session = join(home, 'heartbeat-session.log');
  const fixture = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
  Object.assign(fixture, { kind: 'Run', process_id: server.pid, process_start: start, outcome: null, ended_at: null, session_log: session });
  fixture.issue.number = 202;
  const began = new Date(Date.now() - 20 * 60 * 1000).toISOString();
  fixture.stages = [{ name: 'implement', began_at: began, session_log: session }];
  await writeFile(path, JSON.stringify(fixture));
  await writeFile(log, 'command');
  await writeFile(session, 'session');
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    await page.locator('[data-group="working"] > summary').click();
    const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#202$/ }) });
    const id = await row.getAttribute('id');
    assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
    await appendFile(session, ' growth');
    await page.waitForFunction(id => document.getElementById(id).querySelector('.heartbeat').getAnimations().some(animation => animation.playState === 'running'), id);
    await page.waitForFunction(id => document.getElementById(id).querySelector('.heartbeat').getAnimations().length === 0, id);
    const old = new Date(Date.now() - 20 * 60 * 1000);
    await utimes(log, old, old);
    await utimes(session, old, old);
    await page.waitForFunction(id => document.getElementById(id).dataset.state === 'Silent', id);
    assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await appendFile(log, ' more');
    await page.waitForFunction(id => document.getElementById(id).dataset.logging === 'true', id);
    assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
  } finally { await Promise.all([path, log, session].map(path => rm(path, { force: true }))); }
});

test("a push keeps a Pass dispatch selected instead of its parent", async t => {
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/security');
  await mkdir(dir, { recursive: true });
  const path = join(dir, '20261019T110000+0000~security.command.json');
  const now = new Date().toISOString();
  const fixture = {
    version: 1, kind: 'Security run', name: 'secure-fixture', repository: 'acme/widgets', hostname: 'fixture',
    issue: null, process_id: 0, process_start: 'gone', harness: 'codex', model: null, effort: null,
    started_at: now, ended_at: now, stages: [], session_log: null, outcome: 'failed', cause: 'Pass failure', issue_branch: null, pull_request: null
  };
  fixture.dispatched = [{ ...fixture, kind: 'Run', name: 'issue-303', issue: { number: 303, url: 'https://github.com/acme/widgets/issues/303', title: 'Dispatch before push' } }];
  await writeFile(path, JSON.stringify(fixture));
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    const child = page.locator('.command').filter({ has: page.locator(':scope > .row-head > .subject', { hasText: /^acme\/widgets#303$/ }) });
    await child.click();
    fixture.dispatched[0].issue.title = 'Dispatch after push';
    await writeFile(path, JSON.stringify(fixture));
    await page.getByText('Dispatch after push', { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.activeElement.querySelector(':scope > .row-head > .subject')?.textContent), 'acme/widgets#303');
    const ids = await page.locator('.command').evaluateAll(rows => rows.map(row => row.id));
    assert.equal(new Set(ids).size, ids.length);
  } finally { await rm(path, { force: true }); }
});

test("selection survives a move into folded Done and regains focus when opened", async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    const row = page.locator('.command').filter({ has: page.locator(':scope > .row-head > .subject', { hasText: /^acme\/widgets#1$/ }) });
    await row.click();
    const id = await row.getAttribute('id');
    const fixture = JSON.parse(original);
    fixture.outcome = 'merged';
    await writeFile(path, JSON.stringify(fixture));
    await page.waitForFunction(id => document.getElementById(id)?.closest('details').dataset.group === 'done', id);
    assert.equal(await page.evaluate(() => document.querySelector('.command[aria-current="true"]')?.id ?? document.activeElement.id), id);
    assert.equal(await page.locator('[data-group="done"][open]').count(), 0);
    await page.locator('[data-group="done"] > summary').click();
    await page.waitForFunction(id => document.activeElement.id === id, id);
  } finally { await writeFile(path, original); }
});
