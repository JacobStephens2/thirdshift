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
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-cronjobs-'));
  await mkdir(join(home, '.thirdshift'));
  await mkdir(join(home, 'bin'));
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  await writeFile(join(home, 'bin/crontab'), '#!/bin/sh\n[ "$1" = -l ] || exit 99\ncat "$HOME/crontab"\n', { mode: 0o755 });
  await writeFile(join(home, 'crontab'), '*/5 * * * * cd /srv/a-repository-with-a-long-name && /home/owner/.cargo/bin/thirdshift pickup base main >> /home/owner/.thirdshift/logs/cron.log 2>&1\n@reboot thirdshift secure base main # <script>unsafe()</script>\n');
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
    env: { ...process.env, HOME: home, PATH: `${join(home, 'bin')}:${process.env.PATH}` },
    stdio: ['ignore', 'pipe', 'inherit']
  });
  exited = once(server, 'exit');
  const lines = createInterface({ input: server.stdout });
  try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) }); }
  finally { lines.close(); }
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server && server.exitCode === null) server.kill();
  await exited;
  if (home) await rm(home, { recursive: true, force: true });
});

test('Cron jobs is reachable by link and keyboard, escapes commands and fits a phone', async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  await page.getByRole('link', { name: '8 Cron jobs', exact: true }).click();
  await page.getByRole('heading', { name: 'Cron jobs', exact: true }).waitFor();
  assert.match(await page.locator('main').textContent(), /\*\/5 \* \* \* \*/);
  assert.match(await page.locator('main').textContent(), /<script>unsafe\(\)<\/script>/);
  assert.equal(await page.locator('main script').count(), 0);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.goto(origin);
  await page.keyboard.press('8');
  await page.waitForURL(`${origin}/cronjobs`);
  await page.goto(origin);
  await page.keyboard.press('g');
  await page.locator('#jumps:not([hidden])').waitFor();
  await page.keyboard.press('j');
  await page.waitForURL(`${origin}/cronjobs`);
  await writeFile(join(home, 'crontab'), '0 2 * * * thirdshift architect base main\n');
  await page.reload();
  assert.match(await page.locator('main').textContent(), /0 2 \* \* \*/);
  assert.doesNotMatch(await page.locator('main').textContent(), /@reboot/);
  assert.deepEqual(errors, []);
});

test('last-fired ages advance on a quiet Inbox and Cron jobs page without another Pass', async t => {
  const root = join(home, '.thirdshift/logs/acme/widgets');
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'pickup.last-fired'), new Date().toISOString());
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  for (const path of ['/', '/cronjobs']) {
    await page.goto(`${origin}${path}`);
    const lastFired = page.locator('li').filter({ hasText: /Pickup last fired/ });
    const before = (await lastFired.textContent()).trim();
    assert.match(before, /Pickup last fired \d+ sec ago on acme\/widgets$/);
    await page.waitForTimeout(3000);
    const after = (await lastFired.textContent()).trim();
    assert.notEqual(after, before, 'The open page must advance elapsed last-fired age without another Pass');
  }
});
