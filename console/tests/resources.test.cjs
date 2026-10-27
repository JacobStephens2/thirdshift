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
const settings = '[console]\nport = 0\n';
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-resources-'));
  await mkdir(join(home, '.thirdshift'));
  await writeFile(join(home, '.thirdshift/config.toml'), `${settings}[resources]\nmin_available_memory_mb = 9007199254740991\n`);
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
    env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit']
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

test('Resources shows admission, reloads limits, supports keyboard navigation and fits a phone', async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  await page.getByRole('link', { name: '9 Resources', exact: true }).click();
  await page.getByRole('heading', { name: 'Resources', exact: true }).waitFor();
  assert.match(await page.locator('main').textContent(), /Waiting for host resources/);
  assert.match(await page.locator('main').textContent(), /9007199254740991 MB/);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await writeFile(join(home, '.thirdshift/config.toml'), settings);
  await page.getByRole('link', { name: 'Reload', exact: true }).click();
  assert.match(await page.locator('main').textContent(), /Ready to start another session/);
  await page.goto(origin);
  await page.keyboard.press('9');
  await page.waitForURL(`${origin}/resources`);
  await page.goto(origin);
  await page.keyboard.press('g');
  await page.locator('#jumps:not([hidden])').waitFor();
  await page.keyboard.press('e');
  await page.waitForURL(`${origin}/resources`);
  await page.screenshot({ path: '/tmp/thirdshift-1152-resources.png', fullPage: true });
  assert.deepEqual(errors, []);
});
