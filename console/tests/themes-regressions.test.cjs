const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { join, resolve } = require('node:path');
const { tmpdir } = require('node:os');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { test } = require('node:test');
async function reproduce(mode) {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift824-spec-'));
  let server, browser, exited;
  try {
    const root = join(home, '.thirdshift');
    const records = join(root, 'logs/acme/widgets/commands/issue');
    await mkdir(records, { recursive: true });
    await writeFile(join(root, 'config.toml'), `[console]\nport = 0\ntheme = "${mode === 'tags' ? 'press-light' : 'garden-light'}"\n`);
    const stat = await readFile(`/proc/${process.pid}/stat`, 'utf8');
    const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
    const began = new Date(Date.now() - 60_000).toISOString();
    const record = {version: 1, kind: 'Run', name: '1-fixture', repository: 'acme/widgets', hostname: 'fixture', issue: { number: 1, url: 'https://github.com/acme/widgets/issues/1', title: 'Fixture'}, process_id: process.pid, process_start: start, harness: 'codex', started_at: began, ended_at: mode === 'tags' ? new Date().toISOString() : null, outcome: mode === 'tags' ? 'failed' : null, session_log: null, stages: [{name: 'implement', began_at: began, session_log: null}]};
    const recordPath = join(records, '1-fixture.command.json');
    await writeFile(recordPath, JSON.stringify(record));
    await writeFile(join(records, '1-fixture.log'), 'Command output\n');
    server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {env: {...process.env, HOME: home}, stdio: ['ignore', 'pipe', 'inherit']});
    exited = once(server, 'exit');
    const lines = createInterface({input: server.stdout});
    const [origin] = await once(lines, 'line', {signal: AbortSignal.timeout(10_000)});
    lines.close();
    browser = await chromium.launch({headless: true});
    if (mode === 'tags') {
      const page = await browser.newPage();
      await page.goto(origin);
      const tag = await page.locator('.command[data-state="Failed"] .state').first().evaluate(el => {
        const style = getComputedStyle(el);
        return {background: style.backgroundColor, color: style.color};
      });
      console.log({tag});
      assert.deepEqual(tag, {background: 'rgb(236, 0, 140)', color: 'rgb(0, 0, 0)'}, 'Press solid urgent tag must render the Theme token fill and its best black/white text');
    } else {
      const pages = [];
      for (const name of ['lanes', 'stages', 'waterfall']) {
        const page = await browser.newPage();
        await page.goto(`${origin}/?page=${name}`);
        await page.waitForFunction(() => document.documentElement.dataset.live === 'true');
        assert.ok(await page.evaluate(() => document.getAnimations().some(a => a.playState === 'running')), `${name} starts activity motion`);
        pages.push([name, page]);
      }
      record.ended_at = new Date().toISOString(); record.outcome = 'merged';
      await writeFile(recordPath, JSON.stringify(record));
      const fresh = await (await fetch(`${origin}/?page=lanes`)).text();
      assert.ok(!fresh.includes('data-live="true"'), 'server now observes no live Command');
      await pages[0][1].waitForTimeout(6500);
      const moving = [];
      for (const [name, page] of pages) if (await page.evaluate(() => document.getAnimations().some(a => a.playState === 'running'))) moving.push(name);
      console.log({moving});
      assert.deepEqual(moving, [], 'motion must stop after the only Command ends');
    }
  } finally {
    await browser?.close();
    if (server && server.exitCode === null) server.kill();
    if (exited) await exited;
    await rm(home, {recursive: true, force: true});
  }
}

test('time-page motion stops after the last Command ends', { skip: process.platform !== 'linux' }, () => reproduce('motion'));
test('served solid tags use their Theme fill and black or white text', { skip: process.platform !== 'linux' }, () => reproduce('tags'));
