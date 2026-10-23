const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('playwright');

let home, server, exited, browser, origin;
const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-time-'));
  const folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  await mkdir(folder, { recursive: true });
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  let processStart = 'gone';
  if (process.platform === 'linux') {
    const stat = await readFile(`/proc/${process.pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(') ') + 2).trim().split(/\s+/);
    processStart = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${fields[19]}`;
  }
  const now = Date.now();
  const time = minutes => new Date(now - minutes * 60000).toISOString();
  for (let number = 1; number <= 7; number++) {
    const live = number === 1;
    const record = {
      version: 1, kind: number === 2 ? 'Spec run' : 'Run', name: `run-${number}`, repository: 'acme/widgets', hostname: 'fixture',
      issue: { number, url: `https://github.com/acme/widgets/issues/${number}`, title: `Run ${number} <script>unsafe()</script>` },
      process_id: live ? process.pid : 0, process_start: live ? processStart : 'gone', harness: 'codex',
      started_at: time(number === 7 ? 1500 : 180 - number * 10),
      ended_at: live ? null : time(number === 7 ? 1480 : 150 - number * 10),
      outcome: live ? null : 'merged', session_log: null,
      stages: [
        { name: 'implement', began_at: time(number === 7 ? 1500 : 180 - number * 10), session_log: null },
        { name: 'CI watch', began_at: time(number === 7 ? 1490 : 170 - number * 10), session_log: null }
      ],
      ...(number === 6 ? { parent_record: join(folder, '2-fixture.command.json') } : {})
    };
    if (live) {
      const dispatched = { ...record };
      record.kind = 'Pickup run';
      record.name = 'pickup-fixture';
      record.stages = [{ name: 'dispatched Run', began_at: time(170), session_log: null }];
      record.dispatched = [dispatched];
    }
    await writeFile(join(folder, `${number}-fixture.command.json`), JSON.stringify(record));
    await writeFile(join(folder, `${number}-fixture.log`), `Command ${number}\n`);
  }
  const security = JSON.parse(await readFile(join(folder, '3-fixture.command.json'), 'utf8'));
  security.kind = 'Security run';
  security.name = 'security-fixture';
  security.issue = null;
  await writeFile(join(folder, '10-fixture.command.json'), JSON.stringify(security));
  // A finished Run in an alphabetically earlier repository must not precede
  // this repository's live Run, or reorder the finished columns by repository.
  const otherFolder = join(home, '.thirdshift/logs/aaa/first/commands/issue');
  await mkdir(otherFolder, { recursive: true });
  const other = JSON.parse(await readFile(join(folder, '3-fixture.command.json'), 'utf8'));
  other.repository = 'aaa/first';
  other.issue.number = 8;
  other.issue.url = 'https://github.com/aaa/first/issues/8';
  await writeFile(join(otherFolder, '8-fixture.command.json'), JSON.stringify(other));
  await writeFile(join(home, '.thirdshift/logs/acme/widgets/activity.log'),
    '2026-10-01 12:00:00 Run #9 started: commands/issue/9-20261001T120000+0000.log\n2026-10-01 12:20:00 Run #9 ended: merged\n');
  server = spawn(binary, ['console', '--no-sign-in'], { env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit'] });
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

test('Lanes scales every window, nests child Runs and fits 390 px without CSP violations', async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  for (const hours of [3.5, 12, 24]) {
    await page.goto(`${origin}/?page=lanes&hours=${hours}`);
    assert.equal(await page.locator('[name="hours"]').inputValue(), String(hours));
    assert.equal(await page.locator('.lane').count(), 7);
    assert.equal(await page.locator('.lane').filter({ hasText: 'acme/widgets#2' }).locator('.time-run').count(), 2);
    const complete = page.locator('.time-run').filter({ hasText: 'acme/widgets#3' });
    const track = await complete.locator('.timeline').boundingBox();
    const bars = await complete.locator('.timeline rect').all();
    const first = await bars[0].boundingBox();
    const second = await bars[1].boundingBox();
    // Painted bounds include the one-pixel stroke around each scaled interval.
    assert.ok(Math.abs(first.width - track.width * 10 / (hours * 60)) <= 1.01, JSON.stringify({ hours, track, first, second }));
    assert.ok(Math.abs(second.width - track.width * 20 / (hours * 60)) <= 1.01);
    // #3 starts 150 minutes ago; its first bar is placed on the same time axis.
    assert.ok(Math.abs((first.x - track.x) / track.width - (1 - 150 / (hours * 60))) < 0.002);
    assert.match(await complete.textContent(), /implement: Complete · 10 min/);
    assert.match(await complete.textContent(), /CI watch: Complete · 20 min/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  await page.locator('[name="hours"]').selectOption('12');
  await page.getByRole('button', { name: 'Show', exact: true }).click();
  await page.waitForURL('**hours=12');
  assert.equal(await page.locator('.time-run script').count(), 0);
  assert.deepEqual(errors, []);
});

test('a live Lanes push preserves an unsubmitted hours selection', async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${origin}/?page=lanes&hours=24`);
  await page.locator('[name="hours"]').selectOption('12');
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/3-fixture.command.json');
  const before = await readFile(path, 'utf8');
  const fixture = JSON.parse(before);
  fixture.issue.title = 'Changed while the Lanes filter is being edited';
  try {
    await writeFile(path, JSON.stringify(fixture));
    await page.getByText(fixture.issue.title, { exact: true }).waitFor();
    assert.equal(await page.locator('[name="hours"]').inputValue(), '12');
    await page.getByRole('button', { name: 'Show', exact: true }).click();
    await page.waitForURL('**hours=12');
  } finally { await writeFile(path, before); }
});

test('time headings distinguish a live Pickup run and its Run and name kinds once at 390 px', { skip: process.platform !== 'linux' }, async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  for (const view of ['lanes', 'stages']) {
    await page.goto(`${origin}/?page=${view}`);
    const headings = view === 'lanes' ? page.locator('.time-run h3') : page.locator('thead .command');
    for (const heading of ['Pickup run · acme/widgets#1', 'Ticket run · acme/widgets#1', 'Spec run · acme/widgets#2', 'Ticket run · acme/widgets#6', 'Security run on acme/widgets']) {
      assert.equal(await headings.getByRole('link', { name: heading, exact: true }).count(), 1);
      if (view === 'stages') {
        assert.ok((await page.locator('.cell-run').allTextContents()).includes(`${heading}: `));
      }
    }
    for (const heading of ['Pickup run · acme/widgets#1', 'Ticket run · acme/widgets#1']) {
      const link = headings.getByRole('link', { name: heading, exact: true });
      assert.match(await link.locator('..').textContent(), /live/);
      await link.click();
      await page.locator('#peek-content h2').waitFor();
      const details = await page.locator('#peek-content').textContent();
      assert.ok(details.includes(heading.startsWith('Pickup') ? 'pickup-fixture' : 'run-1'));
      await page.keyboard.press('Escape');
    }
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
});

test('Stages pins live Runs, matches CLI order and opens the selected child Run at 390 px', { skip: process.platform !== 'linux' }, async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${origin}/?page=stages`);
  const status = JSON.parse(execFileSync(binary, ['status', '--all', '--json'], { env: { ...process.env, HOME: home } }));
  const flatten = command => [command, ...(command.dispatched ?? []).flatMap(flatten), ...(command.children ?? []).flatMap(flatten)];
  const runs = status.commands.flatMap(flatten);
  runs.sort((a, b) => (a.state !== 'live') - (b.state !== 'live') || Date.parse(b.ended_at ?? b.started_at) - Date.parse(a.ended_at ?? a.started_at));
  assert.deepEqual(await page.locator('thead .peek-link').allTextContents(), runs.map(run => {
    const kind = run.kind === 'Run' ? 'Ticket run' : run.kind;
    return run.issue ? `${kind} · ${run.repository}#${run.issue.number}` : `${kind} on ${run.repository}`;
  }));
  assert.equal(await page.locator('thead .command').first().getAttribute('data-command'), runs[0].record_path ?? `${join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json')}::pickup-fixture`);
  assert.match(await page.locator('.stage-grid').first().textContent(), /Running ·/);
  assert.match(await page.locator('.stage-grid').first().textContent(), /Usual range: 10–10 min/);
  const historical = page.locator('.stage-grid').filter({ has: page.locator('thead .peek-link', { hasText: 'acme/widgets#9' }) });
  const headers = await historical.locator('thead .peek-link').allTextContents();
  const historicalCell = historical.locator('tbody tr').first().locator('td').nth(headers.indexOf('Ticket run · acme/widgets#9'));
  assert.match(await historicalCell.textContent(), /Unknown · no stage times retained/);
  assert.doesNotMatch(await historicalCell.textContent(), /Not reached/);
  const child = page.locator('thead .peek-link').filter({ hasText: 'acme/widgets#6' });
  await child.click();
  await page.locator('#peek-content h2').waitFor();
  assert.match(await page.locator('#peek-content').textContent(), /run-6/);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#peek').isVisible(), false);
});
