const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('playwright');
const reproduceReviewFinding = require('../reproductions/span-tree-review.cjs');

for (const mode of ['empty', 'chips', 'totals']) {
  test(`Spec regression: ${mode}`, { skip: process.platform !== 'linux' }, async () => {
    await reproduceReviewFinding(mode);
  });
}

let home, server, exited, browser, origin, folder, recordPath, session;
const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
const attr = (key, value) => ({ key, value: typeof value === 'boolean' ? { boolValue: value } : { stringValue: String(value) } });
const nano = ms => String(BigInt(ms) * 1000000n);
const span = (id, parent, name, start, end, attributes = [], status) => ({
  spanId: id, parentSpanId: parent, name, startTimeUnixNano: nano(start),
  endTimeUnixNano: end ? nano(end) : '0', attributes, status
});
const ids = { session: '1111111111111111', agent: '2222222222222222', tool: '3333333333333333', model: '4444444444444444', turn: '5555555555555555' };
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-spans-'));
  folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  await mkdir(folder, { recursive: true });
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  const start = Date.parse('2026-10-01T12:00:00Z');
  session = join(folder, '99-20261001T120000+0000-implement.jsonl');
  const record = {
    version: 1, kind: 'Spec run', repository: 'acme/widgets', name: 'spec-7', hostname: 'fixture',
    issue: { number: 7, url: 'https://github.com/acme/widgets/issues/7', title: 'Widgets <unsafe>' },
    process_id: 0, process_start: 'gone', harness: 'codex', started_at: new Date(start).toISOString(),
    ended_at: new Date(start + 60000).toISOString(), outcome: 'merged', session_log: null,
    stages: [{ name: 'Tickets', began_at: new Date(start).toISOString(), session_log: null }]
  };
  recordPath = join(folder, '7-fixture.command.json');
  await writeFile(recordPath, JSON.stringify(record));
  await writeFile(join(folder, '7-fixture.issue-99.child.json'), JSON.stringify({
    ...record, kind: 'Run', name: 'ticket-99', issue: { ...record.issue, number: 99 }, parent_record: recordPath,
    session_log: session, stages: [
      { name: 'implement', began_at: new Date(start + 1000).toISOString(), session_log: session },
      { name: 'pull request', began_at: new Date(start + 55000).toISOString(), session_log: session }
    ]
  }));
  const spans = [
    span(ids.session, undefined, 'invoke_agent implement', start + 1000, start + 55000, [attr('gen_ai.operation.name', 'invoke_agent'), attr('thirdshift.session.prompt', 'Build <widgets>'), attr('thirdshift.session.kind', 'implement')]),
    span(ids.agent, ids.session, 'agent reviewer', start + 3000, start + 50000, [attr('thirdshift.type', 'subagent_started'), attr('thirdshift.description', 'Review widgets')]),
    span(ids.tool, ids.agent, 'execute_tool Bash', start + 5000, start + 25000, [attr('gen_ai.operation.name', 'execute_tool'), attr('gen_ai.tool.name', 'Bash'), attr('gen_ai.tool.call.arguments', '{"command":"cargo test <unsafe>"}'), attr('gen_ai.tool.call.result', 'Tests failed'), attr('thirdshift.time.estimated', true)], { code: 2, message: 'test failed' }),
    span(ids.model, ids.session, 'model response', start + 30000, start + 45000, [attr('gen_ai.operation.name', 'chat'), attr('gen_ai.response.model', 'test-model'), attr('gen_ai.usage.input_tokens', 100), attr('gen_ai.usage.output_tokens', 25), attr('thirdshift.cost.usd', '0.01')]),
    span(ids.turn, ids.session, 'turn response', start + 1000, start + 55000, [attr('thirdshift.type', 'turn_ended')]),
    ...Array.from({ length: 300 }, (_, i) => span((1000 + i).toString(16).padStart(16, '0'), ids.session, `execute_tool Read ${i}`, start + 46000, start + 46010, [attr('gen_ai.operation.name', 'execute_tool')]))
  ];
  await writeFile(session.replace('.jsonl', '.logs.jsonl'), '');
  await writeFile(session.replace('.jsonl', '.spans.jsonl'), JSON.stringify({ resourceSpans: [{ scopeSpans: [{ spans }] }] }) + '\n');
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

test('span tree, summary Peek, zoom and keyboard work at laptop and 390 px', async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await page.goto(`${origin}/waterfall?command=widgets%237`);
    const tree = page.getByRole('tree', { name: 'Command spans' });
    await tree.waitFor();
    assert.equal(await tree.getByRole('treeitem').count(), 7, 'calls stay out of the DOM while their session is folded');
    const sess = tree.locator(`[data-span-id="${ids.session}"]`);
    assert.equal(await sess.getAttribute('aria-expanded'), 'false');
    assert.match(await sess.textContent(), /#1/);
    assert.match(await sess.textContent(), /1 error/);
    const slow = page.locator('#slowest-calls');
    assert.equal(await slow.locator('[data-reveal-span]').count(), 10);
    assert.match(await slow.locator('[data-reveal-span]').first().textContent(), /Bash/);
    assert.equal(await slow.locator(`[data-reveal-span="${ids.turn}"]`).count(), 0);
    await slow.locator(`[data-reveal-span="${ids.tool}"]`).click();
    const tool = tree.locator(`[data-span-id="${ids.tool}"]`);
    await page.locator('#peek-content h2').waitFor();
    assert.equal(await tool.getAttribute('aria-selected'), 'true');
    assert.match(await page.locator('#peek-content').textContent(), /cargo test <unsafe>/);
    assert.match(await page.locator('#peek-content').textContent(), /test failed/);
    assert.equal(await page.locator('#peek-content details[open] summary').textContent(), 'Input');
    const copy = page.getByRole('button', { name: 'Copy CLI', exact: true });
    assert.match(await copy.getAttribute('data-copy-cli'), /show .* --span 3333333333333333 --json/);
    const data = JSON.parse(execFileSync(binary, ['show', 'widgets#7', '--span', ids.tool, '--json'], { env: { ...process.env, HOME: home }, encoding: 'utf8' }));
    assert.equal(data.error, 'test failed');
    await page.getByRole('button', { name: 'Zoom timeline around this call', exact: true }).click();
    assert.match(await page.locator('#span-axis').textContent(), /Timeline zoomed around/);
    assert.ok(await tree.locator('.outside-window').count() > 0);
    const row = await tool.boundingBox();
    const axis = await page.locator('#span-axis').boundingBox();
    const peek = await page.locator('#peek').boundingBox();
    assert.ok(row.y >= Math.max(0, axis.y + axis.height) - 1);
    assert.ok(row.y + row.height <= (width === 390 ? peek.y : 844) + 1);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    assert.equal(await page.locator('#peek-content unsafe, main unsafe').count(), 0);
    await page.locator('#span-axis').getByRole('button', { name: 'Zoom out to whole Command' }).click();
    await tool.focus();
    await page.keyboard.press('ArrowLeft');
    assert.equal(await page.evaluate(() => document.activeElement.dataset.spanId), ids.agent);
    await page.keyboard.press('ArrowLeft');
    assert.equal(await tree.locator(`[data-span-id="${ids.agent}"]`).getAttribute('aria-expanded'), 'false');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Enter');
    assert.equal(await tool.getAttribute('aria-selected'), 'true');
    if (width === 390) {
      // Even the last row needs scroll space above the bottom sheet.
      const last = tree.locator('.span-item[data-kind="tool"]').last();
      await last.locator(':scope > .span-row').click();
      await page.waitForFunction(() => {
        const row = document.querySelector('#span-tree [role="treeitem"][aria-selected="true"] > .span-row').getBoundingClientRect();
        const sheet = document.getElementById('peek').getBoundingClientRect();
        const axis = document.getElementById('span-axis').getBoundingClientRect();
        return row.top >= Math.max(0, axis.bottom) - 1 && row.bottom <= sheet.top + 1;
      });
    }
    await page.getByRole('button', { name: 'Command details', exact: true }).click();
    await page.locator('#peek-content .facts, #peek-content dl').first().waitFor();
    assert.match(await page.locator('#peek-content').textContent(), /ticket-99/);
    await page.keyboard.press('Escape');
    await page.keyboard.press('1');
    await page.waitForURL('**page=inbox');
  }
  assert.deepEqual(errors, []);
});

test('refresh preserves folds, selected span, zoom and open Peek', async t => {
  const context = await browser.newContext();
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(`${origin}/waterfall?command=widgets%237`);
  await page.locator(`#slowest-calls [data-reveal-span="${ids.model}"]`).click();
  await page.getByRole('button', { name: 'Zoom timeline around this call' }).click();
  await page.locator('#peek-content details').filter({ hasText: /^Output/ }).locator('summary').click();
  const original = await readFile(recordPath, 'utf8');
  const record = JSON.parse(original);
  record.issue.title = 'Widgets refreshed';
  if (process.platform === 'linux') {
    const stat = await readFile(`/proc/${process.pid}/stat`, 'utf8');
    record.process_id = process.pid;
    record.process_start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(') ') + 2).trim().split(/\s+/)[19]}`;
    record.outcome = null;
    record.ended_at = null;
  } else {
    // The observer publishes changes in its Inbox; this old fixture otherwise
    // lives only in History. Move it into the recent Done group on other OSes.
    record.ended_at = new Date().toISOString();
  }
  await writeFile(recordPath, JSON.stringify(record));
  await page.waitForFunction(() => document.querySelector('#span-tree')?.textContent.includes('Widgets refreshed'));
  assert.equal(await page.locator(`[data-span-id="${ids.session}"]`).getAttribute('aria-expanded'), 'true');
  assert.equal(await page.locator(`[data-span-id="${ids.model}"]`).getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#span-axis').textContent(), /Timeline zoomed around/);
  assert.equal(await page.locator('#peek').isVisible(), true);
  assert.equal(await page.locator('#peek-content details').filter({ hasText: /^Output/ }).getAttribute('open'), '');
  await writeFile(recordPath, original);
});
