const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, readFile, readdir, rm } = require('node:fs/promises');
const { tmpdir, hostname } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { createServer } = require('node:http');

const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
async function waitUntil(check, label) {
  const deadline = Date.now() + 15000;
  while (!await check()) {
    assert.ok(Date.now() < deadline, label);
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}
async function files(root) {
  const result = {};
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) Object.assign(result, await files(path));
    else result[path] = await readFile(path, 'utf8');
  }
  return result;
}

test('Fleet Inbox polls with page subscriptions, retains rows, and fits a phone', async t => {
  const root = await mkdtemp(join(tmpdir(), 'thirdshift-fleet-inbox-'));
  const children = [];
  let browser;
  t.after(async () => {
    await browser?.close();
    for (const child of children) {
      if (child.process.exitCode === null) child.process.kill();
      await child.exited;
    }
    await rm(root, { recursive: true, force: true });
  });
  const homes = ['caller', 'remote'].map(name => join(root, name));
  for (const [index, home] of homes.entries()) {
    const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    await mkdir(dir, { recursive: true });
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    const now = new Date().toISOString();
    await writeFile(join(dir, '1-fixture.command.json'), JSON.stringify({
      version: 1, kind: 'Run', name: 'fixture', repository: 'acme/widgets', hostname: 'fixture',
      issue: null, process_id: 0, process_start: 'gone', harness: 'codex',
      started_at: now, ended_at: now, stages: [], session_log: null,
      outcome: 'failed', cause: index ? 'Remote fixture needs your attention' : 'Local fixture needs your attention',
      tickets: index ? ['merged', 'waiting'].map((state, i) => ({
        issue: { number: 10 + i, url: `https://github.com/acme/widgets/issues/${10 + i}`, title: `Ticket ${i}` },
        state, blockers: [],
      })) : [],
    }));
  }
  const cli = (home, args, input) => execFileSync(binary, args, { env: { ...process.env, HOME: home }, input, encoding: 'utf8' });
  async function start(home) {
    const process = spawn(binary, ['console', '--no-sign-in'], {
      env: { ...global.process.env, HOME: home, THIRDSHIFT_FLEET_INTERVAL_MS: '100', THIRDSHIFT_FLEET_STALE_MS: '600' },
      stdio: ['ignore', 'pipe', 'inherit'],
    });
    const child = { process, exited: once(process, 'exit') };
    children.push(child);
    const lines = createInterface({ input: process.stdout });
    try { [child.origin] = await once(lines, 'line', { signal: AbortSignal.timeout(15000) }); }
    finally { lines.close(); }
    return child;
  }
  const remote = await start(homes[1]);
  const token = cli(homes[1], ['console', 'token', 'create', 'caller', '--scopes', 'read,act']).trim();
  cli(homes[0], ['console', 'fleet', 'add', 'remote-workstation-with-a-long-name', remote.origin], `${token}\n`);
  const caller = await start(homes[0]);
  const before = await files(homes[0]);
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  await context.addInitScript(() => {
    window.notices = [];
    window.Notification = class {
      static permission = 'granted';
      static async requestPermission() { return 'granted'; }
      constructor(title, options) { window.notices.push({ title, options }); }
    };
  });
  const page = await context.newPage();
  await page.goto(caller.origin);
  await page.locator('.command[data-machine="remote-workstation-with-a-long-name"]').first().waitFor();
  if (process.env.FLEET_CAPTURE_BEFORE) {
    for (const [width, height, path] of [[1440, 900, process.env.BEFORE_DESKTOP], [390, 844, process.env.BEFORE_MOBILE]]) {
      await page.setViewportSize({ width, height });
      await page.screenshot({ path, fullPage: true });
      console.log(`Before capture: ${path}`);
    }
    return;
  }
  const remoteRows = page.locator('.command[data-machine="remote-workstation-with-a-long-name"]');
  await remoteRows.first().waitFor();
  assert.match(await page.title(), /^2 need you/);
  assert.equal(await page.locator('.command').count(), 2);
  await remoteRows.getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
  assert.match(await remoteRows.locator('.machine-tag').textContent(), /remote-workstation/);
  await remoteRows.locator(':scope > .row-head > .ticket-progress').getByText('1 / 2 Tickets complete', { exact: true }).waitFor();
  assert.equal(await page.locator('.command[data-console=""] .ticket-progress').count(), 0, 'local records supply no progress');
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  if (process.env.AFTER_DESKTOP) {
    for (const [width, height, path] of [[1440, 900, process.env.AFTER_DESKTOP], [390, 844, process.env.AFTER_MOBILE]]) {
      await page.setViewportSize({ width, height });
      await page.screenshot({ path, fullPage: true });
    }
  }
  await page.goto(`${caller.origin}/?machine=remote-workstation-with-a-long-name`);
  await remoteRows.first().waitFor();
  assert.equal(await page.locator('.command').count(), 1);
  await page.getByRole('link', { name: /Ready for review/ }).first().click();
  assert.equal(new URL(page.url()).searchParams.get('machine'), 'remote-workstation-with-a-long-name');
  await page.goto(`${caller.origin}/?machine=${encodeURIComponent(hostname())}`);
  await page.locator('.command').first().waitFor();
  assert.equal(await page.locator('.command').count(), 1, 'local machine filter');
  await page.goto(caller.origin);
  await remoteRows.first().waitFor();
  await remoteRows.getByRole('button', { name: 'Dismiss', exact: true }).click();
  await waitUntil(async () => await remoteRows.locator('button').filter({ hasText: /^Dismiss$/ }).count() === 0, 'remote row leaves Needs you');
  await page.locator('#dismissal-notices').getByRole('button', { name: 'Undo', exact: true }).click();
  await remoteRows.getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  // The enabled observer establishes its baseline through a real Inbox read.
  await page.waitForFunction(() => localStorage.getItem('thirdshift:console:notification-count'));
  const remotePath = join(homes[1], '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const added = JSON.parse(await readFile(remotePath, 'utf8'));
  added.tickets[1].state = 'merged';
  added.kind = 'Process'; added.process_name = 'custom-build';
  await writeFile(remotePath, JSON.stringify(added));
  await remoteRows.locator('.subject[title="Process custom-build on acme/widgets"]').waitFor();
  await remoteRows.locator(':scope > .row-head > .ticket-progress').getByText('2 / 2 Tickets complete', { exact: true }).waitFor();
  added.name = 'another-remote-command'; added.cause = 'Another remote problem';
  await writeFile(remotePath.replace('1-fixture', '2-fixture'), JSON.stringify(added));
  await page.waitForFunction(() => notices.length === 1);
  assert.match(await page.evaluate(() => notices[0].title), /^3 need you/);
  assert.match(await page.evaluate(() => notices[0].options.body), /^remote-workstation/);
  cli(homes[1], ['console', 'token', 'revoke', 'caller']);
  await page.locator('[data-reachability="refused"]').waitFor();
  assert.equal(await remoteRows.count(), 2, 'refused keeps the last good rows');
  remote.process.kill();
  await remote.exited;
  await page.locator('[data-reachability="unreachable"]').waitFor();
  await remoteRows.first().locator('.machine-tag').waitFor();
  await waitUntil(() => remoteRows.evaluateAll(rows => rows.every(row => row.classList.contains('stale'))), 'last good rows turn grey');
  assert.deepEqual(await files(homes[0]), before, 'remote answers stay in memory');
});

test('Fleet makes no requests without a page and a slow or old machine cannot block a good one', async t => {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift-fleet-idle-'));
  const root = join(home, '.thirdshift');
  await mkdir(root);
  await writeFile(join(root, 'config.toml'), '[console]\nport = 0\n');
  const cli = args => execFileSync(binary, args, { env: { ...process.env, HOME: home }, encoding: 'utf8' });
  const document = JSON.parse(cli(['status', '--local', '--json']));
  document.active_runs = 3;
  let requests = 0;
  const fixture = createServer((request, response) => {
    if (request.url.endsWith('/api/v1/whoami')) {
      response.end(JSON.stringify({ version: 1, api_version: 'v1', hostname: 'remote-host', thirdshift_version: 'fixture', scopes: ['read'] }));
    } else {
      requests++;
      if (request.url.startsWith('/slow/')) return;
      if (request.url.startsWith('/old/')) { response.writeHead(404); response.end(); return; }
      response.end(JSON.stringify(document));
    }
  });
  await new Promise(resolve => fixture.listen(0, '127.0.0.1', resolve));
  let server, exited, browser;
  t.after(async () => {
    await browser?.close();
    if (server?.exitCode === null) server.kill();
    await exited;
    fixture.closeAllConnections();
    await new Promise(resolve => fixture.close(resolve));
    await rm(home, { recursive: true, force: true });
  });
  const address = `http://127.0.0.1:${fixture.address().port}`;
  await writeFile(join(root, 'config.toml'), `[console]\nport = 0\nfleet = [${['good', 'slow', 'old'].map(name => `{name = "${name}", address = "${address}/${name}"}`).join(',')}]\n`);
  await writeFile(join(root, 'credentials.toml'), '[fleet]\ngood = "fixture"\nslow = "fixture"\nold = "fixture"\n', { mode: 0o600 });
  server = spawn(binary, ['console', '--no-sign-in'], {
    env: { ...process.env, HOME: home, THIRDSHIFT_FLEET_INTERVAL_MS: '100' }, stdio: ['ignore', 'pipe', 'inherit'],
  });
  exited = once(server, 'exit');
  const lines = createInterface({ input: server.stdout });
  let origin;
  try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(15000) }); }
  finally { lines.close(); }
  await new Promise(resolve => setTimeout(resolve, 400));
  assert.equal(requests, 0, 'no page subscription, no Fleet requests');
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  await page.goto(origin);
  await page.locator('.fleet-machines li').filter({ hasText: /^good · ok/ }).waitFor();
  await page.locator('[data-reachability="needs update"]').waitFor();
  assert.match(await page.locator('.active-runs strong').textContent(), /^3 active runs/);
  assert.match(await page.locator('.fleet-machines li').filter({ hasText: /^good/ }).textContent(), /3 active/);
  assert.equal(await page.locator('.fleet-machines li').filter({ hasText: /^slow/ }).count(), 0, 'good answer arrives before the slow timeout');
  await page.close();
  await new Promise(resolve => setTimeout(resolve, 400));
  const stopped = requests;
  await new Promise(resolve => setTimeout(resolve, 400));
  assert.equal(requests, stopped, 'closing the last page stops new requests');
});
