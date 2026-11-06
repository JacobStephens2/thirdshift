const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { createServer } = require('node:http');
const { chromium } = require('./playwright.cjs');
const binary = process.env.THIRDSHIFT_BINARY || resolve(__dirname, '../../target/debug/thirdshift');

async function fixture(t, commands) {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift-spec-review-'));
  const root = join(home, '.thirdshift');
  await mkdir(root);
  const remote = createServer((req, res) => {
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({version: 2, hostname: 'fixture', active_runs: 0, commands, machines: []}));
  });
  let child, exited, browser;
  t.after(async () => {
    await browser?.close();
    if (child?.exitCode === null) child.kill();
    await exited;
    remote.closeAllConnections();
    await new Promise(resolve => remote.close(resolve));
    await rm(home, {recursive: true, force: true});
  });
  await new Promise(resolve => remote.listen(0, '127.0.0.1', resolve));
  await writeFile(join(root, 'config.toml'), `[console]\nport = 0\nfleet = [{ name = "remote", address = "http://127.0.0.1:${remote.address().port}" }]\n`);
  await writeFile(join(root, 'credentials.toml'), '[fleet]\nremote = "fixture"\n', {mode: 0o600});
  child = spawn(binary, ['console', '--no-sign-in'], {
    env: {...process.env, HOME: home, THIRDSHIFT_FLEET_INTERVAL_MS: '100'}, stdio: ['ignore', 'pipe', 'inherit']
  });
  exited = once(child, 'exit').catch(error => { if (error.code !== 'ENOENT') throw error; });
  const lines = createInterface({input: child.stdout});
  let origin;
  try { [origin] = await once(lines, 'line', {signal: AbortSignal.timeout(15000)}); }
  finally { lines.close(); }
  browser = await chromium.launch({headless: true});
  return { page: await browser.newPage(), origin };
}
function command(name, outcome = 'ready for review') {
  const now = new Date().toISOString();
  return {version: 1, kind: 'Run', name, repository: 'acme/widgets', issue: null,
    hostname: 'fixture', process_id: 0, process_start: 'gone', harness: 'codex',
    started_at: now, ended_at: now, stages: [], session_log: null, outcome,
    pull_request: 'https://github.com/acme/widgets/pull/42', source: 'command_record',
    state: 'ended', stage: null, health: null, children: [], dispatched: [],
    command_log: '/fixture/pass.log'};
}
function item(facts, group) {
  return {...facts, group, needs_you_kind: null, landing: false, stopping: false, count: 1};
}

test('remote dispatched Ready row appears once, with one DOM identity', async t => {
  const run = command('dispatched-42');
  const pass = {...command('security-pass', 'merged'), kind: 'Security run', pull_request: null, dispatched: [run]};
  const {page, origin} = await fixture(t, [item(pass, 'done'), item(run, 'ready for review')]);
  await page.goto(origin);
  await page.locator('[data-machine="remote"]').first().waitFor();
  const sameCommand = page.locator('.command[data-machine="remote"][data-command$="::dispatched-42"]');
  assert.equal(await sameCommand.count(), 1, 'a separately listed Ready row must be removed from its parent tree');
  const ids = await page.locator('#inbox .command').evaluateAll(rows => rows.map(row => row.id));
  assert.equal(new Set(ids).size, ids.length, 'each rendered Command must have a unique DOM ID');
});

test('Ready-for-review link in idle notice retains the machine filter', async t => {
  const {page, origin} = await fixture(t, [item(command('ready-42'), 'ready for review')]);
  await page.goto(`${origin}/?machine=remote`);
  const link = page.locator('.idle-notice a[href*="ready-for-review"]');
  await link.waitFor();
  await link.click();
  assert.equal(new URL(page.url()).searchParams.get('machine'), 'remote', 'folder links must preserve the chosen machine');
});
