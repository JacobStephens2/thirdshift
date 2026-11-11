const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, readFile, rm } = require('node:fs/promises');
const { tmpdir, hostname } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { replaceRecord } = require('./records.cjs');

test('Working identifies live Review and Fix targets locally and across the Fleet', { skip: process.platform !== 'linux' }, async t => {
  const root = await mkdtemp(join(tmpdir(), 'thirdshift-working-targets-'));
  const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
  const children = [];
  let browser;
  t.after(async () => {
    await browser?.close();
    for (const { child, exited } of children) {
      if (child.exitCode === null) child.kill();
      await exited;
    }
    await rm(root, { recursive: true, force: true });
  });
  const homes = ['local', 'remote'].map(name => join(root, name));
  const cli = (home, args, input) => execFileSync(binary, args, { env: { ...process.env, HOME: home }, input, encoding: 'utf8' });
  const records = [];
  for (const home of homes) {
    const dir = join(home, '.thirdshift/logs/acme/widgets/commands/fix');
    await mkdir(dir, { recursive: true });
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    const child = spawn(binary, ['console', '--no-sign-in'], {
      env: { ...process.env, HOME: home, THIRDSHIFT_FLEET_INTERVAL_MS: '100' }, stdio: ['ignore', 'pipe', 'inherit'],
    });
    const running = { child, exited: once(child, 'exit') };
    children.push(running);
    const lines = createInterface({ input: child.stdout });
    try { [running.origin] = await once(lines, 'line', { signal: AbortSignal.timeout(15000) }); }
    finally { lines.close(); }
    const stat = await readFile(`/proc/${child.pid}/stat`, 'utf8');
    const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
    for (const [name, kind, number] of [
      ['review-explicit', 'Review run', 41], ['review-pass', 'Review run', 42],
      ['fix-explicit', 'Fix run', 43], ['fix-pass', 'Fix run', 45],
      ['fix-base', 'Fix run', null], ['fix-legacy', 'Fix run', null],
    ]) {
      const now = new Date().toISOString();
      const value = { version: 1, name, kind, repository: 'acme/widgets', hostname: 'fixture',
        process_id: child.pid, process_start: start, harness: 'codex',
        started_at: now, ended_at: null, outcome: null, issue: null,
        stages: [{ name: 'ci-fix', began_at: now, session_log: null }],
        pull_request: number && `https://github.com/acme/widgets/pull/${number}`,
        ...(name === 'fix-base' ? { base_branch: 'release/2026.11', issue_branch: 'unrelated-repair-name' } : {}),
      };
      const path = join(dir, `${name}.command.json`);
      await replaceRecord(path, JSON.stringify(value));
      if (name === 'fix-base') records.push({ path, value });
    }
  }
  const token = cli(homes[1], ['console', 'token', 'create', 'caller', '--scopes', 'read']).trim();
  cli(homes[0], ['console', 'fleet', 'add', 'remote', children[1].origin], `${token}\n`);
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  for (const [width, height, capture] of [[1440, 900, 'DESKTOP'], [390, 844, 'MOBILE']]) {
    await page.setViewportSize({ width, height });
    for (const section of ['overview', 'working']) {
      await page.goto(`${children[0].origin}/?section=${section}`);
      await page.locator('.command[data-machine="remote"]').first().waitFor();
      if (section === 'overview' && process.env[`BEFORE_${capture}`]) {
        await page.screenshot({ path: process.env[`BEFORE_${capture}`], fullPage: true });
      }
      if (process.env.CAPTURE_BEFORE) continue;
      const rows = page.locator('#inbox > [data-group="working"] .command');
      assert.equal(await rows.count(), 12);
      for (const machine of [hostname(), 'remote']) {
        for (const [name, kind, number] of [
          ['review-explicit', 'Review run', 41], ['review-pass', 'Review run', 42],
          ['fix-explicit', 'Fix run', 43], ['fix-pass', 'Fix run', 45],
        ]) {
          const row = page.locator(`.command[data-machine="${machine}"][data-command*="${name}.command.json"]`);
          const subject = row.locator('.row-head .subject');
          const link = subject.getByRole('link', { name: `PR #${number}`, exact: true });
          assert.equal(await link.isVisible(), true, `${width}px ${machine} ${name}: visible PR target`);
          assert.equal(await link.getAttribute('href'), `https://github.com/acme/widgets/pull/${number}`);
          assert.equal(await subject.getByText('acme/widgets', { exact: true }).isVisible(), true);
          assert.match(await subject.textContent(), new RegExp(kind));
        }
        const base = page.locator(`.command[data-machine="${machine}"][data-command*="fix-base.command.json"]`);
        assert.equal(await base.locator('.row-head').getByText('Base branch release/2026.11', { exact: true }).isVisible(), true);
        assert.equal(await base.locator('.row-head').getByRole('link', { name: 'PR #44', exact: true }).count(), 0);
        const legacy = page.locator(`.command[data-machine="${machine}"][data-command*="fix-legacy.command.json"]`);
        assert.match(await legacy.locator('.subject').textContent(), /acme\/widgets/);
        assert.equal(await legacy.locator('.row-head').getByText(/^Base branch /).count(), 0);
      }
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      if (section === 'overview' && process.env[`AFTER_${capture}`]) {
        await page.screenshot({ path: process.env[`AFTER_${capture}`], fullPage: true });
      }
    }
  }
  if (process.env.CAPTURE_BEFORE) return;
  for (const { path, value } of records) {
    await replaceRecord(path, JSON.stringify({ ...value, pull_request: 'https://github.com/acme/widgets/pull/44' }));
  }
  // Both local notifications and Fleet polling update the open compact rows.
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    for (const machine of [hostname(), 'remote']) {
      const base = page.locator(`.command[data-machine="${machine}"][data-command*="fix-base.command.json"] .row-head`);
      await base.getByRole('link', { name: 'PR #44', exact: true }).waitFor();
      assert.equal(await base.getByText('Base branch release/2026.11', { exact: true }).isVisible(), true);
    }
  }
});
