const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, readFile, readdir, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { record: traceRecord } = require('./session-trace.cjs');

const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');

test('remote Details, Waterfall and Session traces stay in the caller at desktop and phone widths', async t => {
  const root = await mkdtemp(join(tmpdir(), 'thirdshift-remote-details-'));
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
  const homes = ['caller', 'owner'].map(name => join(root, name));
  for (const [index, home] of homes.entries()) {
    const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    await mkdir(dir, { recursive: true });
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    const now = new Date().toISOString();
    const session = join(dir, '7-fixture-implement.logs.jsonl');
    await writeFile(session, traceRecord(index ? 'Remote Session output <unsafe>' : 'Local Session output'));
    await writeFile(join(dir, '7-fixture.log'), index ? 'Remote Command output <unsafe>\n' : 'Local Command output\n');
    await writeFile(join(dir, '7-fixture.command.json'), JSON.stringify({
      version: 1, kind: 'Run', repository: 'acme/widgets', hostname: 'fixture',
      issue: { number: 7, url: 'https://github.com/acme/widgets/issues/7', title: index ? 'Remote widgets' : 'Local widgets' },
      process_id: 0, process_start: 'gone', harness: 'codex', started_at: now, ended_at: now,
      stages: [{ name: 'implement', began_at: now, session_log: session }], session_log: session,
      outcome: 'failed', cause: index ? 'Remote fixture needs attention' : 'Local fixture needs attention',
    }));
  }
  const cli = (home, args, input) => execFileSync(binary, args, { env: { ...process.env, HOME: home }, input, encoding: 'utf8' });
  async function start(home) {
    const child = spawn(binary, ['console', '--no-sign-in'], {
      env: { ...process.env, HOME: home, THIRDSHIFT_FLEET_INTERVAL_MS: '100' }, stdio: ['ignore', 'pipe', 'inherit'],
    });
    children.push({ child, exited: once(child, 'exit') });
    const lines = createInterface({ input: child.stdout });
    try { return (await once(lines, 'line', { signal: AbortSignal.timeout(15000) }))[0]; }
    finally { lines.close(); }
  }
  const owner = await start(homes[1]);
  const token = cli(homes[1], ['console', 'token', 'create', 'caller', '--scopes', 'read,security']).trim();
  cli(homes[0], ['console', 'fleet', 'add', 'remote', owner], `${token}\n`);
  const caller = await start(homes[0]);
  browser = await chromium.launch({ headless: true });
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const page = await browser.newPage({ viewport });
    await page.goto(caller);
    const row = page.locator('.command[data-machine="remote"]').first();
    await row.waitFor();
    assert.equal(await row.locator('.machine-tag a').getAttribute('href'), owner);
    await row.locator('.state').click();
    const details = page.locator('#peek-content');
    await details.getByText('Remote Command output <unsafe>', { exact: true }).waitFor();
    assert.equal(new URL(page.url()).origin, caller, 'opening a remote row stays in place');
    assert.equal(new URL(await details.getByRole('link', { name: 'Open the owning Console' }).getAttribute('href')).origin, owner);
    assert.equal(await details.locator('form').count(), 0, 'details do not offer local actions on a remote Command');
    await details.getByRole('link', { name: 'Waterfall', exact: true }).click();
    await page.locator('#span-tree [role="treeitem"]').first().waitFor();
    assert.equal(new URL(page.url()).origin, caller);
    assert.equal(new URL(page.url()).searchParams.get('machine'), 'remote');
    await page.locator('#span-tree .span-row').first().click();
    const spanCopy = page.locator('#peek-content button[data-copy-cli]');
    await spanCopy.waitFor();
    assert.match(await spanCopy.getAttribute('data-copy-cli'), /--machine 'remote'/, 'copied span commands select the owning machine');
    const data = JSON.parse(await page.locator('#span-data').textContent());
    assert.ok(data.spans.some(span => span.kind === 'stage' && span.name === 'implement'));
    assert.ok(data.spans.some(span => span.kind === 'sess'));
    const session = data.spans.find(span => span.conversation_url);
    await page.goto(`${caller}${session.conversation_url}`);
    await page.locator('#log-content').getByText('Remote Session output <unsafe>', { exact: true }).waitFor();
    assert.equal(new URL(page.url()).searchParams.get('machine'), 'remote');
    assert.equal(await page.locator('#log-content script').count(), 0, 'remote text is escaped');
    await page.getByRole('link', { name: 'Command details', exact: true }).click();
    await details.getByText('Remote Command output <unsafe>', { exact: true }).waitFor();
    await details.getByRole('link', { name: 'Full Command log', exact: true }).click();
    await page.locator('#log-content').getByText('Remote Command output <unsafe>', { exact: true }).waitFor();
    assert.equal(new URL(page.url()).origin, caller);
    assert.equal(new URL(page.url()).searchParams.get('machine'), 'remote');
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'reader fits the viewport');
    await page.close();
  }
  const recordPath = join(homes[1], '.thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json');
  const security = JSON.parse(await readFile(recordPath, 'utf8'));
  security.kind = 'Security run';
  security.security_findings = [{ title: 'Private remote finding', severity: 'high', url: 'https://github.com/acme/widgets/security/advisories/GHSA-test' }];
  await writeFile(recordPath, JSON.stringify(security));
  const reader = cli(homes[1], ['console', 'token', 'create', 'reader']).trim();
  cli(homes[0], ['console', 'fleet', 'add', 'remote', owner], `${reader}\n`);
  const page = await browser.newPage();
  await page.goto(caller);
  await page.locator('.command[data-machine="remote"] .state').first().click();
  await page.locator('#peek-content').getByText('needs the `security` scope on `remote`', { exact: true }).waitFor();
  assert.equal(new URL(page.url()).origin, caller, 'scope refusals stay in the pane');
  assert.equal(await page.locator('#peek-content').getByText('Private remote finding').count(), 0);
  await page.close();
  async function noRemoteContent(root) {
    for (const entry of await readdir(root, { withFileTypes: true })) {
      const path = join(root, entry.name);
      if (entry.isDirectory()) await noRemoteContent(path);
      else assert.doesNotMatch(await readFile(path, 'utf8'), /Remote (Command|Session) output/, path);
    }
  }
  await noRemoteContent(homes[0]);
});
