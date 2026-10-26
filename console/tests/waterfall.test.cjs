const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

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
  const session = join(folder, '99-20261001T120000+0000-implement.logs.jsonl');
  const spans = [
    ['a', 'first <unsafe>', '1790856015000000000', '1790856030000000000'],
    ['b', 'unmatched', '0', '0'],
    ['c', 'last', '1790856030000000000', '1790856040000000000'],
  ].map(([id, command, start, end], index) => ({
    spanId: (index + 1).toString(16).padStart(16, '0'), name: `execute_tool Bash ${command}`,
    startTimeUnixNano: start, endTimeUnixNano: end,
    attributes: [
      { key: 'gen_ai.operation.name', value: { stringValue: 'execute_tool' } },
      { key: 'gen_ai.tool.call.id', value: { stringValue: id } },
      { key: 'gen_ai.tool.name', value: { stringValue: 'Bash' } },
    ]
  }));
  await writeFile(session, '');
  await writeFile(session.replace('.logs.jsonl', '.spans.jsonl'), JSON.stringify({ resourceSpans: [{ scopeSpans: [{ spans }] }] }) + '\n');
  await writeFile(join(folder, '7-fixture.issue-99.child.json'), JSON.stringify({
    ...record, kind: 'Run', name: 'ticket-99', issue: { ...record.issue, number: 99 }, parent_record: parent,
    started_at: '2026-10-01T12:00:10+00:00', ended_at: '2026-10-01T12:00:40+00:00', session_log: session,
    stages: [{ name: 'implement', began_at: '2026-10-01T12:00:10+00:00', session_log: session }]
  }));
  await writeFile(join(folder, '7-fixture.log'), 'thirdshift: 12:00:00 starting\nthirdshift: 12:00:15 #99: implement: $ first <unsafe>\nthirdshift: 12:00:30 #99: implement: $ last\nthirdshift: 12:01:00 finished\n');
  const traceSession = join(folder, '10-20261001T120000+0000-implement.logs.jsonl');
  await writeFile(traceSession, await readFile(resolve(__dirname, '../../tests/fixtures/waterfall-logs.jsonl')));
  await writeFile(traceSession.replace('.logs.jsonl', '.spans.jsonl'), await readFile(resolve(__dirname, '../../tests/fixtures/waterfall-spans.jsonl')));
  await writeFile(join(folder, '10-fixture.command.json'), JSON.stringify({
    ...record, kind: 'Run', name: 'trace-10', harness: 'muse', issue: { ...record.issue, number: 10 },
    session_log: traceSession,
    stages: [{ name: 'implement', began_at: record.started_at, session_log: traceSession }]
  }));
  const liveStart = new Date(Date.now() - 10000).toISOString();
  const liveSession = join(folder, '9-20261001T120000+0000-implement.logs.jsonl');
  await writeFile(liveSession, JSON.stringify({ resourceLogs: [{ scopeLogs: [{ logRecords: [{
    observedTimeUnixNano: `${BigInt(Date.parse(liveStart)) * 1000000n}`,
    attributes: [{ key: 'thirdshift.events', value: { stringValue: JSON.stringify([
      { type: 'tool_call_started', id: 'live', parent_id: null, tool: 'Bash', input: { command: 'still running' } }
    ]) } }]
  }] }] }] }) + '\n');
  for (const number of [8, 9]) {
    await writeFile(join(folder, `${number}-fixture.command.json`), JSON.stringify({
      ...record, kind: 'Run', name: `run-${number}`, issue: { ...record.issue, number },
      process_id: number === 9 ? process.pid : 0, process_start: number === 9 ? processStart : 'gone',
      outcome: null, ended_at: null,
      ...(number === 9 ? { started_at: liveStart, session_log: liveSession,
        stages: [{ name: 'implement', began_at: liveStart, session_log: liveSession }] } : {})
    }));
  }
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], { env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit'] });
  exited = once(server, 'exit');
  const lines = createInterface({ input: server.stdout });
  try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) }); }
  finally { lines.close(); }
  browser = await chromium.launch({ headless: true });
});

test('Waterfall draws overlapping Session-trace spans and running start records', async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await page.goto(`${origin}/waterfall?command=widgets%2310`);
    await page.getByRole('button', { name: 'Every call', exact: true }).click();
    const tools = page.locator('.span-item[data-kind="tool"]');
    assert.equal(await tools.count(), 4);
    const data = await (await context.request.get(`${origin}/spans?command=widgets%2310`)).json();
    const spans = data.spans.filter(span => span.kind === 'tool').sort((a, b) => a.start - b.start);
    assert.equal(spans[0].start, Date.parse('2026-10-01T12:00:15.250+00:00'));
    assert.equal(spans[0].duration_ms, 3500);
    assert.equal(spans[1].start, Date.parse('2026-10-01T12:00:16.500+00:00'));
    assert.equal(spans[1].duration_ms, 625);
    const track = await tools.nth(0).locator('.span-track').boundingBox();
    const first = await tools.nth(0).locator('.span-bar').boundingBox();
    const second = await tools.nth(1).locator('.span-bar').boundingBox();
    assert.ok(Math.abs(first.width - track.width * 3.5 / 60) <= 1.01);
    assert.ok(Math.abs(second.width - track.width * 0.625 / 60) <= 1.01);
    assert.ok(Math.abs((first.x - track.x) / track.width - 15.25 / 60) < 0.002);
    assert.ok(first.x + first.width > second.x);
    assert.equal(spans[2].start, Date.parse('2026-10-01T12:00:20.125+00:00'));
    assert.equal(spans[2].duration_ms, null);
    assert.match(await tools.nth(2).textContent(), /Time unknown/);
    assert.equal(await page.locator('main script:not([type="application/json"]), main unsafe').count(), 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (process.platform === 'linux') {
      await page.goto(`${origin}/waterfall?command=widgets%239`);
      await page.getByRole('button', { name: 'Every call', exact: true }).click();
      const running = page.locator('.span-item[data-kind="tool"]');
      assert.equal(await running.count(), 1);
      assert.match(await running.textContent(), /still running/);
      const live = await (await context.request.get(`${origin}/spans?command=widgets%239`)).json();
      const call = live.spans.find(span => span.kind === 'tool');
      assert.ok(call.start > Date.now() - 120000);
      const seconds = (Date.now() - call.start) / 1000;
      assert.ok(seconds >= 10 && seconds < 120, `${seconds} seconds`);
      const initial = (await running.locator('.span-duration').textContent());
      await page.waitForFunction(initial => document.querySelector('.span-item[data-kind="tool"] .span-duration')?.textContent !== initial, initial);
      assert.ok((await running.locator('.span-bar').boundingBox()).width > 0);
    }
  }
  const recordPath = join(home, '.thirdshift/logs/acme/widgets/commands/issue/10-fixture.command.json');
  const died = JSON.parse(await readFile(recordPath, 'utf8'));
  died.outcome = null;
  died.ended_at = null;
  await writeFile(recordPath, JSON.stringify(died));
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await page.goto(`${origin}/waterfall?command=widgets%2310`);
    await page.getByRole('button', { name: 'Every call', exact: true }).click();
    assert.match(await page.locator('.command-chips [aria-current]').textContent(), /died/);
    for (const index of [0, 1]) {
      assert.ok((await page.locator('.span-item[data-kind="tool"] .span-bar').nth(index).boundingBox()).width > 0);
    }
  }
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
    const runs = page.locator('details.command-choices');
    assert.equal(await runs.evaluate(node => node.open), false, 'runs start folded');
    assert.match(await runs.locator('summary').textContent(), /^Runs \d+ · acme\/widgets#\d+ · \w+$/);
    await runs.locator('summary').click();
    const chips = page.getByRole('navigation', { name: 'Choose a Command' });
    assert.match(await chips.textContent(), /acme\/widgets#8 · died/);
    if (process.platform === 'linux') assert.match(await chips.textContent(), /acme\/widgets#9 · live/);
    await chips.getByRole('link', { name: 'acme/widgets#7 · merged' }).click();
    await runs.locator('summary', { hasText: / · acme\/widgets#7 · merged$/ }).waitFor({ state: 'attached' });
    await page.getByRole('button', { name: 'Every call', exact: true }).click();
    assert.equal(await page.locator('.span-item[data-kind="run"]').count(), 2);
    const child = page.locator('.span-item[data-kind="run"]').nth(1);
    assert.equal(await child.locator('.span-item[data-kind="tool"]').count(), 3);
    assert.match(await child.locator('.span-item[data-kind="tool"]').last().textContent(), /Time unknown/);
    const track = await child.locator(':scope > .span-row > .span-track').boundingBox();
    const run = await child.locator(':scope > .span-row .span-bar').boundingBox();
    const stageTrack = await child.locator('.span-item[data-kind="stage"] > .span-row > .span-track').boundingBox();
    const toolTrack = await child.locator('.span-item[data-kind="tool"] .span-track').first().boundingBox();
    const first = await child.locator('.span-item[data-kind="tool"] .span-bar').first().boundingBox();
    assert.ok(Math.abs(run.width - track.width / 2) <= 1.01);
    assert.ok(Math.abs(first.width - track.width / 4) <= 1.01);
    assert.ok(Math.abs((first.x - track.x) / track.width - 0.25) < 0.002);
    assert.equal(track.x, stageTrack.x);
    assert.equal(track.x, toolTrack.x);
    assert.equal(track.width, toolTrack.width);
    assert.equal(await page.locator('main script:not([type="application/json"]), main unsafe').count(), 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await child.locator(':scope > .span-row').click();
    await page.getByRole('button', { name: 'Command details', exact: true }).click();
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
