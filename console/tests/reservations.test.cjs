const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

let home, server, exited, browser, origin;
const reference = 'refs/heads/thirdshift/pickup/ci-red';
const sha = '0123456789012345678901234567890123456789';

before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-reservation-inbox-'));
  const repo = join(home, '.thirdshift/logs/acme/widgets');
  await mkdir(repo, { recursive: true });
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  await writeFile(join(repo, 'reservations.json'), JSON.stringify({
    repository: 'acme/widgets', launch: '/unavailable/fixture', observations: [{
      reference, sha, owner: { host: 'retired <host>', launch: '/old/checkout', process: 201924,
        started: new Date(Date.now() - 25 * 3600 * 1000).toISOString() },
    }],
  }));
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'),
    ['console', '--no-sign-in'], { env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'pipe'] });
  exited = once(server, 'exit');
  const lines = createInterface({ input: server.stdout });
  try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(15000) }); }
  finally { lines.close(); }
  browser = await chromium.launch({ headless: true });
});

after(async () => {
  await browser?.close();
  if (server?.exitCode === null) server.kill();
  if (exited) await exited;
  if (home) await rm(home, { recursive: true, force: true });
});

test('manual reservation recovery is readable and copyable on desktop and phone without a Command record', async t => {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    for (const javaScriptEnabled of [true, false]) {
      const context = await browser.newContext({ viewport, javaScriptEnabled });
      t.after(() => context.close());
      const page = await context.newPage();
      await page.goto(`${origin}/?section=needs-you`);
      const row = page.locator('#inbox [data-group="needs you"] .command[data-state="Reservation"]');
      await row.waitFor({ state: 'visible' });
      assert.equal(await row.locator('.issue-title').textContent(), reference);
      const detail = await row.locator('.now').textContent();
      assert.match(detail, /retired <host>/);
      assert.match(detail, /Process: 201924/);
      assert.match(detail, /Base branch CI/);
      assert.match(await row.locator('.age').textContent(), /Held for 25 hr/);
      const command = await row.locator('.peek-cli code').textContent();
      assert.equal(command, `git -C '/unavailable/fixture' push --no-verify --force-with-lease=${reference}:${sha} origin :${reference}`);
      assert.equal(await row.locator('.peek-link').count(), 0);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal overflow');
      if (javaScriptEnabled) {
        await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
        await row.getByRole('button', { name: 'Copy release command' }).click();
        assert.equal(await page.evaluate(() => navigator.clipboard.readText()), command);
      }
      await context.close();
    }
  }
});

test('Dismiss and Undo keep the exact reservation observation without releasing it', async t => {
  const page = await browser.newPage();
  t.after(() => page.close());
  await page.goto(`${origin}/?section=needs-you`);
  const waiting = page.locator('#inbox [data-group="needs you"] .command[data-state="Reservation"]');
  await waiting.getByRole('button', { name: 'Dismiss', exact: true }).click();
  await waiting.waitFor({ state: 'detached' });
  await page.goto(`${origin}/?section=resolved`);
  const resolved = page.locator('#inbox [data-group="resolved"] .command[data-state="Reservation"]');
  await resolved.getByRole('button', { name: 'Undo', exact: true }).click();
  await resolved.waitFor({ state: 'detached' });
  await page.goto(`${origin}/?section=needs-you`);
  await waiting.waitFor({ state: 'visible' });
});
