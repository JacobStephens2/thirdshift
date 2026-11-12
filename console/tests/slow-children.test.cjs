const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { replaceRecord } = require('./records.cjs');
const { liveRunWithUsualRange } = require('./fixtures/run-health.cjs');

test('compact Spec and Pass rows keep step-1 Slow child text visible', { skip: process.platform !== 'linux' }, async () => {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift-child-slow-'));
  let server, exited, browser;
  try {
    const records = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    await mkdir(records, { recursive: true });
    await mkdir(join(home, 'bin'));
    await writeFile(join(home, 'bin/gh'), '#!/bin/sh\nexit 1\n', { mode: 0o755 });
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    const base = await liveRunWithUsualRange(records);
    const began = new Date(Date.now() - 1200000).toISOString();
    const parentPath = join(records, '20-parent.command.json');
    const childPath = join(records, '20-parent.command.issue-21.child.json');
    const ticket = { ...base, name: 'widgets#21',
      issue: { number: 21, title: 'Slow child', url: 'https://github.com/acme/widgets/issues/21' },
      started_at: began, stages: [{ name: 'implement', began_at: began, session_log: null }] };
    const spec = { ...base, kind: 'Spec run', name: 'widgets#20',
      issue: { number: 20, title: 'Spec parent', url: 'https://github.com/acme/widgets/issues/20' },
      started_at: began, stages: [], tickets: [{ issue: ticket.issue, state: 'running', blockers: [] }] };
    await writeFile(join(records, '20-parent.log'), 'still working\n');
    server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
      env: { ...process.env, HOME: home, PATH: `${join(home, 'bin')}:${process.env.PATH}` },
      stdio: ['ignore', 'pipe', 'inherit']
    });
    exited = once(server, 'exit');
    const lines = createInterface({ input: server.stdout });
    let origin;
    try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) }); }
    finally { lines.close(); }
    browser = await chromium.launch({ headless: true });
    for (const tree of ['Spec', 'Pass with Spec', 'Pass with Ticket']) {
      await rm(childPath, { force: true });
      const parent = tree === 'Spec' ? spec : {
        ...base, kind: 'Pickup run', name: 'pickup-parent', started_at: began, stages: [],
        dispatched: [tree === 'Pass with Spec' ? spec : ticket]
      };
      await replaceRecord(parentPath, JSON.stringify(parent));
      if (tree !== 'Pass with Ticket') {
        await replaceRecord(childPath, JSON.stringify({ ...ticket, parent_record: parentPath,
          ...(tree === 'Pass with Spec' ? { parent_dispatch: spec.issue.url } : {}) }));
      }
      for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
        const page = await browser.newPage({ viewport });
        await page.goto(origin);
        const row = page.locator('#inbox .group[data-group="working"] > ul > .command');
        await row.waitFor();
        assert.equal(await row.count(), 1, tree);
        await row.locator(':scope > .row-head > .gist').getByText(/slower than usual \(12–17 min\), still working/).waitFor();
        assert.ok((await row.innerText()).includes('acme/widgets#21'), tree);
        assert.equal(await page.locator('#inbox .group[data-group="needs you"] > ul > .command').count(), 0, tree);
        await page.close();
      }
    }
  } finally {
    await browser?.close();
    if (server && server.exitCode === null) server.kill();
    if (exited) await exited;
    await rm(home, { recursive: true, force: true });
  }
});
