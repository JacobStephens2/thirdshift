const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, rm, chmod } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
const HOLDER = 'laptop-vm-fixture';
const hours = count => new Date(Date.now() + count * 3600000).toISOString();
// Grok in cooldown on both its routes, agy above the 85% ceiling, and a
// Claude login shared with the remote machine.
const ROUTES = '[harness]\ndefault = "grok"\nroutes = ["grok", "agy"]\nmax_utilization = 85\n';
const claude = (ago, used) => ({
  observed_at: hours(-ago), source: 'session',
  account: { signed_in: true, label: 'me@example.test', organization: 'org-1' },
  windows: [{ name: 'five_hour', used_percent: used, resets_at: hours(3) }]
});

let local, remote, browser;

async function machine(name, config, harnesses) {
  const home = await mkdtemp(join(tmpdir(), `thirdshift-plan-usage-${name}-`));
  const bin = join(home, 'bin');
  await mkdir(join(home, '.thirdshift'), { recursive: true });
  await mkdir(bin);
  for (const cli of ['grok', 'agy']) {
    await writeFile(join(bin, cli), '#!/bin/sh\nexit 0\n');
    await chmod(join(bin, cli), 0o755);
  }
  await writeFile(join(home, '.thirdshift/config.toml'), `[console]\nport = 0\n${config}`);
  await writeFile(join(home, '.thirdshift/plan-usage.json'), JSON.stringify({ harnesses }));
  const env = { ...process.env, HOME: home, PATH: `${bin}:${process.env.PATH}` };
  const server = spawn(binary, ['console', '--no-sign-in'], { env, stdio: ['ignore', 'pipe', 'inherit'] });
  const exited = once(server, 'exit');
  const lines = createInterface({ input: server.stdout });
  let origin;
  try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) }); }
  finally { lines.close(); }
  const cli = (args, input) => {
    const result = spawnSync(binary, args, { env, input, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  return { home, server, exited, origin, cli };
}

async function stop(machine) {
  if (!machine) return;
  if (machine.server.exitCode === null) machine.server.kill();
  await machine.exited;
  await rm(machine.home, { recursive: true, force: true });
}

before(async () => {
  local = await machine('local', ROUTES, {
    grok: { observed_at: hours(0), source: 'session',
      cooldown: { since: hours(0), until: hours(1), backoff_minutes: 60 } },
    agy: { observed_at: hours(0), source: 'session',
      windows: [{ name: 'weekly', used_percent: 90, resets_at: hours(24) }] },
    claude: claude(2, 10)
  });
  remote = await machine('remote', '', { claude: claude(0, 70) });
  browser = await chromium.launch({ headless: true });
});

after(async () => {
  await browser?.close();
  await stop(local);
  await stop(remote);
});

test('the header warns of limited routes, Plan usage opens by keyboard, clears a cooldown and fits a phone', async t => {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const context = await browser.newContext({ viewport });
    t.after(() => context.close());
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(local.origin);
    const warning = page.locator('header #plan-usage-warning a');
    await warning.waitFor();
    assert.equal(await warning.textContent(), 'Plan usage · 3 routes limited');
    assert.match(await warning.getAttribute('title'), /grok harness\.default, ceiling 85%: in cooldown/);
    const [pill, host] = await Promise.all([warning.boundingBox(), page.locator('#host-resources').boundingBox()]);
    assert.ok(pill.y >= host.y + host.height - 1, 'the warning sits below host admission');
    if (viewport.width === 1440) {
      await page.screenshot({ path: '/tmp/thirdshift-2182-header-desktop.png' });
    }
    await Promise.all([page.waitForURL(`${local.origin}/plan-usage`), warning.click()]);
    await page.getByRole('heading', { level: 1, name: /^Plan usage · / }).waitFor();
    const grok = page.locator('section.plan-usage-login', { has: page.getByRole('heading', { name: /^grok/ }) });
    assert.match(await grok.textContent(), /harness\.routes 1, ceiling 85%: in cooldown/);
    assert.match(await page.locator('main').textContent(), /90% used/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `${viewport.width}px`);
    await page.screenshot({ path: `/tmp/thirdshift-2182-plan-usage-${viewport.width}.png`, fullPage: true });
    // Nine Resources stays the host's own.
    await page.goto(`${local.origin}/resources`);
    assert.doesNotMatch(await page.locator('main').textContent(), /Plan usage/);
    await page.goto(local.origin);
    await page.keyboard.press('g');
    await page.locator('#jumps:not([hidden])').waitFor();
    await page.keyboard.press('p');
    await page.waitForURL(`${local.origin}/plan-usage`);
    assert.deepEqual(errors, []);
  }
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${local.origin}/plan-usage`);
  await page.locator('select[name="harness"]').selectOption('grok');
  await Promise.all([page.waitForURL(`${local.origin}/plan-usage`), page.getByRole('button', { name: 'Clear cooldowns' }).click()]);
  assert.equal(await page.locator('header #plan-usage-warning a').textContent(), 'Plan usage · 1 route limited');
  const grok = page.locator('section.plan-usage-login', { has: page.getByRole('heading', { name: /^grok/ }) });
  assert.match(await grok.textContent(), /harness\.routes 1, ceiling 85%: available/);
});

test('Fleet Plan usage groups a shared login, shows a pushed machine read-only and an offline one cached', async t => {
  const hostname = (await (await fetch(`${local.origin}/api/v1/whoami`)).json()).hostname;
  const token = remote.cli(['console', 'token', 'create', 'watcher']);
  local.cli(['console', 'fleet', 'add', 'remote', remote.origin], `${token}\n`);
  const report = local.cli(['console', 'token', 'create', HOLDER, '--scopes', 'report']);
  const pushed = await fetch(`${local.origin}/api/v1/plan-usage/report`, {
    method: 'POST', headers: { authorization: `Bearer ${report}`, 'content-type': 'application/json' },
    body: JSON.stringify({ version: 1, machine: HOLDER, view: { version: 1, harnesses: [
      { harness: 'agy', installed: true, active: false, routes: [], observed_at: hours(0), source: 'session',
        windows: [{ name: 'weekly', used_percent: 33 }] }
    ] } })
  });
  assert.equal(pushed.status, 200, await pushed.text());

  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${local.origin}/plan-usage`);
  await page.getByRole('heading', { level: 1, name: 'Plan usage · Fleet' }).waitFor();
  const shared = page.getByRole('heading', { level: 2, name: `claude · shared by ${hostname}, remote` });
  await shared.waitFor();
  const login = page.locator('section.plan-usage-login', { has: shared });
  assert.match(await login.textContent(), /organization org-1 · me@example\.test/);
  assert.match(await login.textContent(), /70% used/);
  assert.match(await login.textContent(), /on remote/);
  const machines = page.locator('[aria-labelledby="plan-usage-machines"]');
  assert.match(await machines.textContent(), /remote: ok/);
  assert.match(await machines.textContent(), new RegExp(`${HOLDER}: reported, read-only · pushed`));
  await page.getByRole('heading', { level: 2, name: `agy · ${HOLDER}` }).waitFor();
  await page.screenshot({ path: '/tmp/thirdshift-2182-plan-usage-fleet.png', fullPage: true });

  await page.locator('select[name="machine"]').selectOption('remote');
  await Promise.all([page.waitForNavigation(), page.getByRole('button', { name: 'Show', exact: true }).click()]);
  await page.getByRole('heading', { level: 1, name: 'Plan usage · remote' }).waitFor();

  remote.server.kill();
  await remote.exited;
  await page.goto(`${local.origin}/plan-usage`);
  assert.match(await page.locator('[aria-labelledby="plan-usage-machines"]').textContent(), /remote: cached, offline · last seen/);
  await page.getByRole('heading', { level: 2, name: `claude · shared by ${hostname}, remote` }).waitFor();
  assert.deepEqual(errors, []);
});
