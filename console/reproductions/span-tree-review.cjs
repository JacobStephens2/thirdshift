const assert = require('node:assert/strict');
const { mkdtemp, mkdir, writeFile, readFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { createInterface } = require('node:readline');
const { chromium } = require('playwright');
const repo = resolve(__dirname, '../..');
const binary = process.env.THIRDSHIFT_BINARY || join(repo, 'target/debug/thirdshift');
async function reproduce(mode) {
  const home = await mkdtemp(join(tmpdir(), 'spec-996-'));
  let server, browser, exited;
  try {
    const folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    await mkdir(folder, { recursive: true });
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    const start = Date.now() - 30000;
    const stat = await readFile(`/proc/${process.pid}/stat`, 'utf8');
    const identity = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(') ') + 2).trim().split(/\s+/)[19]}`;
    const session = join(folder, '7-fixture-implement.jsonl');
    const path = join(folder, '7-fixture.command.json');
    const record = { version: 1, kind: 'Run', repository: 'acme/widgets', name: 'issue-7', hostname: 'fixture', issue: { number: 7, url: 'https://github.com/acme/widgets/issues/7', title: 'Widgets' }, process_id: process.pid, process_start: identity, harness: 'codex', started_at: new Date(start).toISOString(), ended_at: null, outcome: null, session_log: ['totals', 'phone'].includes(mode) ? session : null, stages: [{ name: 'implement', began_at: new Date(start).toISOString(), session_log: ['totals', 'phone'].includes(mode) ? session : null }] };
    if (mode !== 'empty') await writeFile(path, JSON.stringify(record));
    if (['totals', 'phone'].includes(mode)) {
      const attr = (key, value) => ({ key, value: { stringValue: String(value) } });
      const span = (id, parentSpanId, name, operation, attrs = []) => ({ status: mode === 'phone' && operation === 'execute_tool' ? { code: 2, message: 'tool failed' } : undefined, spanId: id, parentSpanId, name, startTimeUnixNano: String(BigInt(start) * 1000000n), endTimeUnixNano: String(BigInt(start + 20000) * 1000000n), attributes: [attr('gen_ai.operation.name', operation), ...attrs] });
      await writeFile(session.replace('.jsonl', '.logs.jsonl'), '');
      await writeFile(session.replace('.jsonl', '.spans.jsonl'), JSON.stringify({ resourceSpans: [{ scopeSpans: [{ spans: [span('1111111111111111', undefined, 'invoke_agent implement', 'invoke_agent', [attr('gen_ai.usage.input_tokens', 100), attr('gen_ai.usage.output_tokens', 25), attr('thirdshift.cost.usd', 0.01)]), ...(mode === 'phone' ? Array.from({ length: 10 }, (_, i) => span((2000+i).toString(16).padStart(16,'0'), '1111111111111111', 'execute_tool Bash', 'execute_tool')) : [span('2222222222222222', '1111111111111111', 'model response', 'chat', [attr('gen_ai.usage.input_tokens', 100), attr('gen_ai.usage.output_tokens', 25)])])] }] }] }) + '\n');
    }
    server = spawn(binary, ['console', '--no-sign-in'], { env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit'] });
    exited = once(server, 'exit');
    const lines = createInterface({ input: server.stdout });
    const [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) });
    lines.close();
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${origin}/waterfall`);
    if (mode === 'empty') {
      assert.match(await page.locator('main').textContent(), /No Commands yet/);
      await writeFile(path, JSON.stringify(record));
      await page.waitForSelector('#span-tree', { state: 'attached', timeout: 10000 });
      await page.waitForFunction(() => document.querySelectorAll('#span-tree [role="treeitem"]').length > 0, null, { timeout: 5000 });
    } else if (mode === 'chips') {
      await page.locator('#span-tree [role="treeitem"]').first().waitFor();
      record.ended_at = new Date().toISOString(); record.outcome = 'merged';
      await writeFile(path, JSON.stringify(record));
      await page.waitForFunction(() => document.querySelector('#span-tree')?.textContent.includes('merged') || !document.querySelector('#span-tree .span-status-running'), null, { timeout: 10000 });
      const tree = await (await page.request.get(`${origin}/spans?command=widgets%237`)).json();
      assert.equal(tree.spans[0].status, 'merged');
      assert.match(await page.locator('.command-chips [aria-current]').textContent(), /merged/, 'Command chip must reflect the completed Command after the span tree refreshes');
    } else if (mode === 'phone') {
      await page.locator('#span-tree > .span-item').waitFor();
      const widths = await page.evaluate(() => ({ actual: document.documentElement.scrollWidth, expected: innerWidth }));
      assert.ok(widths.actual <= widths.expected, `390 px phone overflows: ${JSON.stringify(widths)}`);
    } else if (mode === 'totals') {
      await page.locator('#span-tree > .span-item > .span-row').click();
      await page.locator('#peek-content h2').waitFor();
      const tiles = await page.locator('#peek-content .span-tile').allTextContents();
      assert.ok(tiles.some(text => /100\s*Tokens in/.test(text)), `Command summary must include its 100 input tokens once, got ${JSON.stringify(tiles)}`);
      assert.ok(tiles.some(text => /25\s*Tokens out/.test(text)), `Command summary must include its 25 output tokens once, got ${JSON.stringify(tiles)}`);
      assert.ok(tiles.some(text => /0\.01\s*Cost/.test(text)), `Command summary must include its session cost, got ${JSON.stringify(tiles)}`);
    } else throw Error('Expected empty, chips, or totals');
    console.log(`PASS ${mode}`);
  } finally {
    await browser?.close();
    if (server?.exitCode === null) server.kill();
    await exited;
    await rm(home, { recursive: true, force: true });
  }
}
module.exports = reproduce;
if (require.main === module) reproduce(process.argv[2]).catch(error => { console.error(error); process.exitCode = 1; });
