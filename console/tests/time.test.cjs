const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

let home, server, exited, browser, origin;
const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
// By repository folds each grid to its name; a reader opens each one.
async function openSections(page) {
  for (const summary of await page.locator('details.stage-section > summary').all()) if (!await summary.evaluate(node => node.parentElement.open)) await summary.click();
}
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
  // Different repository ranges must stay attributed to their own columns.
  const otherFolder = join(home, '.thirdshift/logs/aaa/first/commands/issue');
  await mkdir(otherFolder, { recursive: true });
  const other = JSON.parse(await readFile(join(folder, '3-fixture.command.json'), 'utf8'));
  other.repository = 'aaa/first';
  other.issue.number = 8;
  other.issue.url = 'https://github.com/aaa/first/issues/8';
  for (const [number, minutes] of [[8, 150], [18, 210], [19, 220]]) {
    other.issue.number = number;
    other.started_at = time(minutes);
    other.stages[0].began_at = time(minutes);
    other.stages[1].began_at = time(minutes - 30);
    other.ended_at = time(minutes - 70);
    await writeFile(join(otherFolder, `${number}-fixture.command.json`), JSON.stringify(other));
  }
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
    assert.equal(await page.locator('.lane').count(), 9);
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

test('Stages groups runs, scrolls in both views and remembers the view at phone and desktop widths', { skip: process.platform !== 'linux' }, async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${origin}/?page=stages`);
  const status = JSON.parse(execFileSync(binary, ['status', '--all', '--json'], { env: { ...process.env, HOME: home } }));
  const flatten = command => [command, ...(command.dispatched ?? []).flatMap(flatten), ...(command.children ?? []).flatMap(flatten)];
  const runs = status.commands.flatMap(flatten);
  const subjects = runs.map(run => {
    const kind = run.kind === 'Run' ? 'Ticket run' : run.kind;
    return run.issue ? `${kind} · ${run.repository}#${run.issue.number}` : `${kind} on ${run.repository}`;
  });
  assert.equal(await page.getByLabel('View', { exact: true }).inputValue(), 'all');
  const allHeadings = await page.locator('thead .peek-link').allTextContents();
  const implementCells = page.locator('tbody tr').filter({ has: page.getByRole('rowheader', { name: 'implement', exact: true }) }).locator('td');
  assert.match(await implementCells.nth(allHeadings.indexOf('Ticket run · aaa/first#8')).textContent(), /Usual range: 30–30 min/);
  assert.match(await implementCells.nth(allHeadings.indexOf('Ticket run · acme/widgets#3')).textContent(), /Usual range: 10–10 min/);
  for (const width of [390, 1280]) {
    await page.setViewportSize({ width, height: 844 });
    for (const mode of ['all', 'repository']) {
      await page.getByLabel('View', { exact: true }).selectOption(mode);
      await page.waitForFunction(count => document.querySelectorAll('.stage-grid').length === count, mode === 'all' ? 1 : 2);
      await openSections(page);
      assert.deepEqual((await page.locator('thead .peek-link').allTextContents()).sort(), [...subjects].sort());
      const grid = page.locator('.stage-section').filter({ has: page.locator('thead .peek-link', { hasText: 'acme/widgets#6' }) });
      const headings = await grid.locator('thead .peek-link').allTextContents();
      assert.equal(headings[headings.indexOf('Spec run · acme/widgets#2') + 1], 'Ticket run · acme/widgets#6');
      assert.ok(headings.indexOf('Ticket run · acme/widgets#5') < headings.indexOf('Pickup run · acme/widgets#1'));
      assert.match(await grid.locator('.run-context').filter({ hasText: 'Ticket of' }).textContent(), /Spec run · acme\/widgets#2/);
      const scroll = grid.locator('.stage-scroll');
      const label = grid.locator('tbody th').first();
      const left = (await label.boundingBox()).x;
      await scroll.evaluate(node => { node.scrollLeft = node.scrollWidth; });
      assert.ok(await scroll.evaluate(node => node.scrollLeft > 0));
      assert.ok(Math.abs((await label.boundingBox()).x - left) < 1);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await grid.getByRole('button', { name: 'Newest runs' }).click();
      assert.equal(await scroll.evaluate(node => node.scrollLeft), 0);
    }
  }
  await page.reload();
  await page.waitForFunction(() => document.querySelectorAll('.stage-grid').length === 2);
  await openSections(page);
  assert.equal(await page.getByLabel('View', { exact: true }).inputValue(), 'repository');
  const widgets = page.locator('.stage-grid').filter({ has: page.locator('thead .peek-link', { hasText: 'acme/widgets#6' }) });
  assert.match(await widgets.textContent(), /Running ·/);
  assert.match(await widgets.textContent(), /Usual range: 10–10 min/);
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

test('Stages fills a wide window, with and without the Peek open', async t => {
  const context = await browser.newContext({ viewport: { width: 1920, height: 1000 } });
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${origin}/?page=stages`);
  // The body's padding is the only margin left on either side.
  const [left, right] = await page.evaluate(() => {
    const padding = parseFloat(getComputedStyle(document.body).paddingLeft);
    return [padding, document.documentElement.clientWidth - padding];
  });
  const scroll = await page.locator('.stage-scroll').boundingBox();
  assert.ok(scroll.x <= left + 1 && scroll.x + scroll.width >= right - 1, JSON.stringify({ left, right, scroll }));
  await page.locator('thead .peek-link').first().click();
  await page.locator('#peek-content h2').waitFor();
  const main = await page.locator('main').boundingBox();
  const peek = await page.locator('#peek').boundingBox();
  assert.ok(main.x <= left + 1 && peek.x + peek.width >= right - 1, JSON.stringify({ left, right, main, peek }));
});

test('Stages live insertions preserve the visible Ticket, link focus and selected view', async () => {
  for (const width of [390, 1280]) for (const view of ['all', 'repository']) for (const position of ['left', 'older']) {
    const context = await browser.newContext({ viewport: { width, height: 844 } });
    const page = await context.newPage();
    const folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    const added = [join(folder, '20-fixture.command.json'), join(folder, '21-fixture.command.json')];
    try {
      await page.goto(`${origin}/?page=stages`);
      await page.getByLabel('View', { exact: true }).selectOption(view);
      await page.waitForFunction(count => document.querySelectorAll('.stage-grid').length === count, view === 'all' ? 1 : 2);
      await openSections(page);
      const ticket = page.locator('thead .command').filter({ has: page.getByRole('link', { name: `Ticket run · acme/widgets#${position === 'left' ? 5 : 6}`, exact: true }) });
      const link = ticket.locator('.peek-link');
      await link.focus();
      await ticket.evaluate((node, position) => {
        const scroll = node.closest('.stage-scroll');
        if (position === 'left') scroll.scrollLeft = 0;
        else scroll.scrollLeft += node.getBoundingClientRect().left - scroll.getBoundingClientRect().left - 155;
      }, position);
      const left = (await ticket.boundingBox()).x;
      const selector = await ticket.getAttribute('data-command');
      for (const [index, path] of added.entries()) {
        const fixture = JSON.parse(await readFile(join(folder, '6-fixture.command.json'), 'utf8'));
        fixture.issue.number = 20 + index;
        fixture.issue.title = `Live insertion ${index}`;
        fixture.started_at = new Date().toISOString();
        if (index === 0) delete fixture.parent_record;
        await writeFile(path, JSON.stringify(fixture));
        await page.locator('thead .command', { hasText: fixture.issue.title }).waitFor();
        assert.ok(Math.abs((await ticket.boundingBox()).x - left) < 1, `${width} ${view} ${position}: insertion ${index} moved the Ticket`);
        assert.equal(await page.evaluate(() => document.activeElement?.className), 'peek-link');
        assert.equal(await page.locator('.command[aria-current="true"]').getAttribute('data-command'), selector);
        assert.equal(await page.getByLabel('View', { exact: true }).inputValue(), view);
      }
      const section = ticket.locator('xpath=ancestor::details[contains(@class, "stage-section")]');
      await section.getByRole('button', { name: 'Newest runs' }).focus();
      await page.keyboard.press('Space');
      assert.equal(await section.locator('.stage-scroll').evaluate(node => node.scrollLeft), 0);
    } finally {
      await context.close();
      await Promise.all(added.map(path => rm(path, { force: true })));
    }
  }
});

test('Stages handles invalid or unavailable preferences and preserves selector focus on a live update', async () => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/3-fixture.command.json');
  const before = await readFile(path, 'utf8');
  for (const storage of ['invalid', 'unavailable', 'unwritable']) {
    const context = await browser.newContext();
    await context.addInitScript(storage => {
      if (storage === 'invalid') localStorage.setItem('thirdshift:console:stages-view', 'unexpected');
      if (storage === 'unavailable') Object.defineProperty(window, 'localStorage', { get() { throw new Error('Storage denied'); } });
      if (storage === 'unwritable') Storage.prototype.setItem = () => { throw new Error('Storage full'); };
    }, storage);
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    try {
      await page.goto(`${origin}/?page=stages`);
      const selector = page.getByLabel('View', { exact: true });
      assert.equal(await selector.inputValue(), 'all');
      await selector.focus();
      // Not ArrowDown: a closed <select> ignores it in Chromium on macOS (#1395).
      await selector.selectOption('repository');
      assert.equal(await page.evaluate(() => document.activeElement.id), 'stages-view');
      await page.waitForFunction(() => document.querySelectorAll('.stage-grid').length === 2);
      const fixture = JSON.parse(before);
      fixture.issue.title = `Focus during ${storage} preference storage`;
      await writeFile(path, JSON.stringify(fixture));
      await page.locator('thead .command', { hasText: fixture.issue.title }).waitFor({ state: 'attached' });
      assert.equal(await page.evaluate(() => document.activeElement.id), 'stages-view');
      assert.equal(await selector.inputValue(), 'repository');
      assert.deepEqual(errors, []);
    } finally {
      await context.close();
      await writeFile(path, before);
    }
  }
});

test('Stages orders starts within the same second by their recorded time', async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const paths = [3, 4].map(number => join(folder, `${number}-fixture.command.json`));
  const before = await Promise.all(paths.map(path => readFile(path, 'utf8')));
  try {
    const second = Math.floor(Date.now() / 1000) * 1000;
    for (const [index, path] of paths.entries()) {
      const fixture = JSON.parse(before[index]);
      fixture.started_at = new Date(second + [100, 900][index]).toISOString();
      await writeFile(path, JSON.stringify(fixture));
    }
    const page = await context.newPage();
    await page.goto(`${origin}/?page=stages`);
    const headings = await page.locator('thead .peek-link').allTextContents();
    assert.ok(headings.indexOf('Ticket run · acme/widgets#4') < headings.indexOf('Ticket run · acme/widgets#3'));
  } finally {
    await Promise.all(paths.map((path, index) => writeFile(path, before[index])));
  }
});

test('Stages keeps its scrolling table and keyboard access in every Theme', async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${origin}/?page=stages`);
  for (const theme of ['focus', 'plain', 'press', 'signal', 'garden', 'factory']) for (const mode of ['light', 'dark']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, `${theme}-${mode}`);
    const scroll = page.locator('.stage-scroll');
    await scroll.focus();
    await scroll.evaluate(node => { node.scrollLeft = 0; });
    const label = page.locator('tbody th').first();
    const left = (await label.boundingBox()).x;
    await page.locator('thead .command').last().focus();
    assert.ok(await scroll.evaluate(node => node.scrollLeft > 0), `${theme}-${mode}`);
    assert.ok(Math.abs((await label.boundingBox()).x - left) < 1);
    assert.equal(await label.evaluate(node => getComputedStyle(node).display), 'table-cell');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
});
