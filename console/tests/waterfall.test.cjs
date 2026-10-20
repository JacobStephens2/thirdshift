const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('playwright');

let home, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-waterfall-'));
  const folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  await mkdir(folder, { recursive: true });
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  let processStart = 'gone';
  if (process.platform === 'linux') {
    const stat = await readFile(`/proc/${process.pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(') ') + 2).trim().split(/\s+/);
    processStart = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${fields[19]}`;
  }
  const record = {
    version: 1, kind: 'Spec run', repository: 'acme/widgets', name: 'spec-7', hostname: 'fixture',
    issue: { number: 7, url: 'https://github.com/acme/widgets/issues/7', title: 'Widgets <script>unsafe()</script>' },
    process_id: 0, process_start: 'gone', harness: 'codex', started_at: '2026-10-01T12:00:00+00:00',
    ended_at: '2026-10-01T12:01:00+00:00', outcome: 'merged', session_log: null,
    stages: [{ name: 'Tickets', began_at: '2026-10-01T12:00:00+00:00', session_log: null }]
  };
  const parent = join(folder, '7-fixture.command.json');
  await writeFile(parent, JSON.stringify(record));
  const session = join(folder, '99-20261001T120000+0000-implement.jsonl');
  const item = (id, command) => JSON.stringify({ type: 'item.completed', item: { id, type: 'command_execution', command, status: 'completed' } }) + '\n';
  await writeFile(session, item('a', 'first <unsafe>') + item('b', 'unmatched') + item('c', 'last'));
  await writeFile(join(folder, '7-fixture.issue-99.child.json'), JSON.stringify({
    ...record, kind: 'Run', name: 'ticket-99', issue: { ...record.issue, number: 99 }, parent_record: parent,
    started_at: '2026-10-01T12:00:10+00:00', ended_at: '2026-10-01T12:00:40+00:00', session_log: session,
    stages: [{ name: 'implement', began_at: '2026-10-01T12:00:10+00:00', session_log: session }]
  }));
  await writeFile(join(folder, '7-fixture.log'), 'thirdshift: 12:00:00 starting\nthirdshift: 12:00:15 #99: implement: $ first <unsafe>\nthirdshift: 12:00:30 #99: implement: $ last\nthirdshift: 12:01:00 finished\n');
  for (const number of [8, 9]) {
    await writeFile(join(folder, `${number}-fixture.command.json`), JSON.stringify({
      ...record, kind: 'Run', name: `run-${number}`, issue: { ...record.issue, number },
      process_id: number === 9 ? process.pid : 0, process_start: number === 9 ? processStart : 'gone',
      outcome: null, ended_at: null
    }));
  }
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], { env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit'] });
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

test('Waterfall chips, nested scale and unknown tools work at desktop and 390 px', async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await page.goto(`${origin}/?page=waterfall`);
    const chips = page.getByRole('navigation', { name: 'Choose a Command' });
    assert.match(await chips.textContent(), /acme\/widgets#8 · died/);
    if (process.platform === 'linux') assert.match(await chips.textContent(), /acme\/widgets#9 · live/);
    await chips.getByRole('link', { name: 'acme/widgets#7 · merged' }).click();
    assert.equal(await page.locator('.waterfall-run').count(), 2);
    const child = page.locator('.waterfall-run[data-depth="1"]');
    assert.equal(await child.locator('.waterfall-tool').count(), 3);
    assert.match(await child.locator('.waterfall-tool').nth(1).textContent(), /Time unknown/);
    const track = await child.locator(':scope > .timeline').boundingBox();
    const run = await child.locator(':scope > .timeline rect').boundingBox();
    const stageTrack = await child.locator('.waterfall-stage > .timeline').boundingBox();
    const toolTrack = await child.locator('.waterfall-tool .timeline').first().boundingBox();
    const first = await child.locator('.waterfall-tool .timeline rect').first().boundingBox();
    assert.ok(Math.abs(run.width - track.width / 2) <= 1.01);
    assert.ok(Math.abs(first.width - track.width / 4) <= 1.01);
    assert.ok(Math.abs((first.x - track.x) / track.width - 0.25) < 0.002);
    assert.equal(track.x, stageTrack.x);
    assert.equal(track.x, toolTrack.x);
    assert.equal(track.width, toolTrack.width);
    assert.equal(await page.locator('main script, main unsafe').count(), 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await child.locator('.peek-link').click();
    await page.locator('#peek-content h2').waitFor();
    assert.match(await page.locator('#peek-content').textContent(), /ticket-99/);
    await page.keyboard.press('Escape');
    await page.keyboard.press('1');
    await page.waitForURL('**page=inbox');
    await page.keyboard.press('4');
    await page.waitForURL('**page=waterfall');
  }
  assert.deepEqual(errors, []);
});
