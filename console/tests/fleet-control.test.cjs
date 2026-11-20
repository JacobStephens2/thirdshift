const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { utimes, writeFile } = require('node:fs/promises');
const { hostname } = require('node:os');
const { basename, join } = require('node:path');
const { record: traceRecord } = require('./session-trace.cjs');
const { startFixture } = require('./start-fixture.cjs');

test('Ready for review groups Fleet Commands and routes Dismiss and Undo to the selected member', async t => {
  const owner = await startFixture();
  t.after(() => owner.close());
  const caller = await startFixture({ THIRDSHIFT_FLEET_INTERVAL_MS: '150' });
  t.after(() => caller.close());
  const token = owner.cli(['console', 'token', 'create', 'watcher', '--scopes', 'read,act']).trim();
  caller.cli(['console', 'fleet', 'add', 'relay', owner.origin], `${token}\n`);

  const pr = 'https://github.com/acme/widgets/pull/116';
  const localPath = await caller.readyPullRequest(116, { name: 'forge-review' }, { ci: 'green', title: 'Latest completion' });
  const session = join(owner.home, '.thirdshift/logs/acme/widgets/commands/review/116-review.logs.jsonl');
  const remotePath = await owner.readyPullRequest(116, {
    name: 'relay-review', pull_request: 'https://github.com/Acme/Widgets/pull/116/', session_log: session,
    started_at: new Date(Date.now() - 120_000).toISOString(), ended_at: new Date(Date.now() - 60_000).toISOString(),
  }, { ci: 'red', mergeable: 'conflicting', title: 'Earlier completion' });
  await writeFile(session, traceRecord('Relay Session output'));
  await writeFile(remotePath.replace('.command.json', '.log'), 'Relay Command output\n');
  await caller.readyPullRequest(117);
  await caller.readyPullRequest(118);
  await caller.readyPullRequest(119, { repository: 'acme/gadgets', pull_request: 'https://github.com/acme/gadgets/pull/119' });

  const page = await caller.page({ viewport: { width: 390, height: 844 } });
  t.after(() => page.context().close());
  const ready = page.locator('#inbox > [data-group="ready for review"]');
  const folder = page.locator('.folders [data-group="ready for review"]');
  const rows = ready.locator(':scope > .commands > li');
  const grouped = rows.filter({ has: page.getByRole('heading', { name: 'acme/widgets#116' }) });
  await folder.click();
  const fold = grouped.getByText('2 contributing Commands', { exact: true });
  await fold.waitFor({ timeout: 15000 });
  assert.equal(await rows.count(), 4);
  assert.equal(await folder.locator('.count').textContent(), '4');
  assert.equal(await ready.locator(':scope > summary .count').textContent(), '4');
  assert.equal(await grouped.getByRole('link', { name: 'acme/widgets#116', exact: true }).getAttribute('href'), pr);
  assert.match(await grouped.locator('.review-heading').textContent(), /Latest completion/);
  assert.deepEqual(await grouped.locator('.review-warnings span').allTextContents(), ['CI red', 'Merge conflict']);
  const groupId = await grouped.getAttribute('id');
  assert.equal(await grouped.getByRole('button', { name: /Dismiss/ }).count(), 0);
  await fold.focus();
  await page.keyboard.press('Enter');
  const local = grouped.locator('.review-members > .command[data-console=""]');
  const remote = grouped.locator('.review-members > .command[data-machine="relay"]');
  const dismissRemote = remote.getByRole('button', { name: 'Dismiss Command on relay' });
  await dismissRemote.waitFor();
  await page.keyboard.press('j');
  assert.equal(await local.getAttribute('aria-current'), 'true');
  assert.equal(await local.getAttribute('data-command'), `${localPath}::forge-review`);
  assert.equal(await remote.getAttribute('data-command'), `${remotePath}::relay-review`);
  assert.equal(await remote.getByText('Merge conflict', { exact: true }).count(), 1);
  const fullLog = new URL(await remote.getByRole('link', { name: 'Full log' }).getAttribute('href'), caller.origin);
  assert.equal(fullLog.searchParams.get('machine'), 'relay');
  assert.match(await (await page.request.get(fullLog.href)).text(), /Relay Command output/);
  await remote.getByRole('link', { name: 'Details', exact: true }).click();
  const details = page.locator('#peek-content');
  const owningConsole = details.getByRole('link', { name: 'Open the owning Console' });
  await owningConsole.waitFor();
  const owningPeek = new URL(await owningConsole.getAttribute('href'));
  assert.equal(owningPeek.origin, owner.origin);
  assert.equal(owningPeek.searchParams.get('command'), `${remotePath}::relay-review`);
  await details.getByText('Session traces', { exact: false }).first().click();
  const trace = await details.getByRole('link', { name: /116-review.logs.jsonl/ }).getAttribute('href');
  assert.match(await (await page.request.get(new URL(trace, caller.origin).href)).text(), /Relay Session output/);
  await page.locator('[data-close-peek]').click();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  const ids = await page.locator('#inbox [id]').evaluateAll(nodes => nodes.map(node => node.id));
  assert.equal(new Set(ids).size, ids.length);

  const dismissed = page.waitForRequest(request => request.method() === 'POST' && request.url() === `${caller.origin}/dismiss`);
  await dismissRemote.click();
  const undo = page.getByRole('button', { name: 'Undo dismissal on relay' });
  await undo.waitFor();
  assert.equal((await dismissed).postDataJSON().machine, 'relay');
  // Wait for the owning-machine decision to replace the group with its one
  // remaining Command, even though the top-level count never changed.
  const remaining = rows.and(page.locator('.command')).filter({ has: page.locator(`a[href="${pr}"]`) });
  await remaining.getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
  assert.equal(await remaining.getAttribute('id'), groupId);
  assert.equal(await remaining.getAttribute('data-command'), `${localPath}::forge-review`);
  const rawRow = (fixture, path) => JSON.parse(fixture.cli(['status', '--local', '--json'])).commands.find(row => row.record_path === path);
  assert.equal(rawRow(owner, remotePath).dismissal.kinds[0], 'Ready');
  assert.equal(rawRow(caller, localPath).group, 'ready for review');
  await undo.click();
  await fold.waitFor();
  assert.equal(await grouped.getAttribute('id'), groupId);
  assert.equal(rawRow(owner, remotePath).group, 'ready for review');
  assert.equal(await rows.count(), 4);

  await page.goto(`${caller.origin}/?section=ready-for-review&machine=relay`);
  await rows.locator('.row-head > .subject > a[href$="/pull/116/"]').waitFor();
  assert.equal(await rows.count(), 1);
  assert.equal(await rows.first().getAttribute('id'), groupId);
  await page.goto(`${caller.origin}/?section=ready-for-review&repository=acme%2Fgadgets`);
  assert.equal(await rows.count(), 1);
  assert.match(await rows.first().textContent(), /acme\/gadgets#119/);
  await page.goto(`${caller.origin}/?section=ready-for-review`);
  await fold.click();
  owner.cli(['console', 'token', 'revoke', 'watcher']);
  await grouped.getByText('Stale or unavailable facts', { exact: true }).waitFor();
  await remote.getByText('Owning machine unavailable', { exact: true }).waitFor();
  assert.equal(await remote.getByRole('button', { name: /Dismiss|Start Fix run/ }).count(), 0);
  assert.equal(await local.getByRole('button', { name: /Dismiss/ }).count(), 1);
});

test('Text status pairs stale GitHub facts with the Fleet Command that supplied them', async t => {
  const env = { THIRDSHIFT_GITHUB_INTERVAL_MS: '3600000' };
  const peer = await startFixture(env);
  t.after(() => peer.close());
  const caller = await startFixture(env);
  t.after(() => caller.close());
  const token = peer.cli(['console', 'token', 'create', 'watcher', '--scopes', 'read']);
  caller.cli(['console', 'fleet', 'add', 'relay', peer.origin], token);
  await peer.readyPullRequest(116, { name: 'older-peer-command' }, { ci: 'green' });
  await caller.readyPullRequest(116, { name: 'newer-local-command' }, { ci: 'green' });
  const old = new Date(Date.now() - 360_000);
  await utimes(join(peer.home, '.thirdshift/logs/github-inbox.json'), old, old);
  const page = await peer.page();
  const status = await (await page.request.get(`${peer.origin}/api/v1/status`)).json();
  assert.ok(status.github_answer_age_seconds >= 300, JSON.stringify(status));
  const output = caller.cli(['status']);
  const lines = output.split('\n');
  assert.match(lines.find(line => line.startsWith('acme/widgets#116 · ')), /2 Commands/);
  const remote = lines.findIndex(line => line.startsWith('  [relay]'));
  assert.ok(remote >= 0, output);
  assert.match(lines.slice(remote, remote + 5).join('\n'), /stale/i, output);
});

test('Ready members with identical qualified names show different record names', async t => {
  const fixture = await startFixture({ THIRDSHIFT_GITHUB_INTERVAL_MS: '3600000' });
  t.after(() => fixture.close());
  const timestamp = new Date().toISOString();
  const facts = { name: 'acme/widgets#116', pull_request: 'https://github.com/acme/widgets/pull/116', started_at: timestamp, ended_at: timestamp };
  const records = [await fixture.readyPullRequest(116, facts), await fixture.readyPullRequest(216, facts)];
  const page = await fixture.page({ viewport: { width: 390, height: 844 } });
  const group = page.locator('.review-group');
  await group.locator('summary').click({ timeout: 15000 });
  const members = group.locator('.review-members > .command');
  assert.equal(await members.count(), 2);
  const labels = await members.locator('.member-command code').allTextContents();
  assert.deepEqual(labels.sort(), records.map(record => `${basename(record)}::acme/widgets#116`).sort());
});

test('A failed Dismiss restores a navigable selection when another Ready member arrives', async t => {
  const fixture = await startFixture();
  t.after(() => fixture.close());
  const started_at = new Date(Date.now() - 120_000).toISOString();
  const ended_at = new Date(Date.now() - 60_000).toISOString();
  const pull_request = 'https://github.com/acme/widgets/pull/116';
  await fixture.readyPullRequest(116, { name: 'first-review', started_at, ended_at });
  const page = await fixture.page();
  const ready = page.locator('#inbox > [data-group="ready for review"] > .commands');
  const dismiss = ready.locator(':scope > .command').getByRole('button', { name: 'Dismiss', exact: true });
  let failSave;
  const pendingSave = new Promise(resolve => { failSave = resolve; });
  await page.route('**/dismiss', async route => {
    await pendingSave;
    await route.fulfill({ status: 500, body: 'Fixture write failed' });
  });
  const group = ready.locator(':scope > .review-group');
  try {
    await dismiss.click();
    await fixture.readyPullRequest(117, { name: 'second-review', pull_request, started_at, ended_at });
    await group.locator(':scope > details > summary').click();
  } finally {
    failSave();
  }
  await group.getByText(/Could not save the dismissal/).waitFor();
  const selected = await group.evaluate(node => {
    if (node.matches('.is-selected')) return 'PR group wrapper';
    return node.querySelector('.is-selected')?.tagName ?? 'nothing';
  });
  assert.ok(['SUMMARY', 'LI'].includes(selected), `Expected a navigable selection, got ${selected}`);
});

test('Fleet starts, Passes and config keep their machine through confirmation and scopes', async t => {
  const release = createServer((request, response) => {
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({tag_name: 'v99.0.0', name: 'v99.0.0', url: 'http://127.0.0.1/release', prerelease: false, assets: []}));
  });
  await new Promise(resolve => release.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => release.close(resolve)));
  const owner = await startFixture({ THIRDSHIFT_INSTALLER_GHE_BASE_URL: `http://127.0.0.1:${release.address().port}/` });
  t.after(() => owner.close());
  const caller = await startFixture();
  t.after(() => caller.close());
  function pair(scopes) {
    const token = owner.cli(['console', 'token', 'create', `caller-${scopes}`, '--scopes', scopes]).trim();
    caller.cli(['console', 'fleet', 'add', 'owner', owner.origin], `${token}\n`);
  }
  pair('read,run,admin');
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const page = await caller.page({ viewport });
    t.after(() => page.context().close());
    await page.getByText('Start work', { exact: true }).click();
    await page.getByLabel('Repository', { exact: true }).selectOption({ label: 'acme/widgets · owner' });
    await page.getByLabel('Issue number or URL', { exact: true }).fill(String(viewport.width));
    await page.getByRole('button', { name: 'Review start', exact: true }).click();
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    assert.match(await confirm.textContent(), /machine owner/);
    await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
    await page.locator('[data-start-result]').filter({ hasText: `Started Command acme/widgets#${viewport.width}.` }).waitFor({ timeout: 15000 });
    assert.match(JSON.stringify(JSON.parse(owner.cli(['console', 'log', '--json']))), /owner@example.test from the Console on caller-read,run,admin/);
    await page.goto(`${caller.origin}/cronjobs?machine=owner`);
    await page.getByRole('button', { name: 'Run now', exact: true }).click();
    await confirm.waitFor();
    assert.match(await confirm.textContent(), /machine owner/);
    await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
    await page.locator('[data-start-result]').filter({ hasText: /Pass skipped/ }).waitFor({ timeout: 15000 });
    await page.goto(`${caller.origin}/config?machine=owner`);
    await page.getByLabel('Value for spec.parallel', { exact: true }).fill('5');
    await Promise.all([page.waitForURL(`${caller.origin}/config?machine=owner&saved=spec.parallel#config-spec.parallel`), page.getByRole('button', { name: 'Save spec.parallel', exact: true }).click()]);
    assert.equal(JSON.parse(owner.cli(['config', 'spec.parallel', '--json'])).value, 5);
    assert.equal(JSON.parse(caller.cli(['config', 'spec.parallel', '--json'])).value, 3);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.goto(`${caller.origin}/update?machine=owner`);
    await page.getByRole('button', { name: 'Update thirdshift', exact: true }).waitFor();
    assert.match(await page.locator('main').textContent(), /Machine: owner/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'Update errors fit the viewport');
    await page.context().close();
  }
  // The caller's own privileges do not make remote controls appear.
  pair('read');
  const page = await caller.page();
  t.after(() => page.context().close());
  assert.equal(await page.locator('#start-repository option[data-machine="owner"]').count(), 0);
  await page.goto(`${caller.origin}/cronjobs?machine=owner`);
  assert.equal(await page.getByRole('button', { name: 'Run now', exact: true }).count(), 0);
  await page.goto(`${caller.origin}/config?machine=owner`);
  assert.match(await page.textContent('body'), /403/);
});

test('a Fleet name matching the local hostname keeps config changes on the owner', async t => {
  const owner = await startFixture();
  t.after(() => owner.close());
  const caller = await startFixture();
  t.after(() => caller.close());
  const machine = hostname();
  const token = owner.cli(['console', 'token', 'create', 'caller', '--scopes', 'admin']).trim();
  caller.cli(['console', 'fleet', 'add', machine, owner.origin], `${token}\n`);
  const page = await caller.page({ viewport: { width: 390, height: 844 } });
  t.after(() => page.context().close());
  await page.getByText('Start work', { exact: true }).click();
  assert.match(await page.locator('#start-repository option').first().textContent(), /\(local\)/);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.goto(`${caller.origin}/config?machine=${encodeURIComponent(machine)}`);
  await page.getByLabel('Value for spec.parallel', { exact: true }).fill('5');
  await page.getByRole('button', { name: 'Save spec.parallel', exact: true }).click();
  await page.waitForLoadState();
  assert.equal(JSON.parse(owner.cli(['config', 'spec.parallel', '--json'])).value, 5);
  assert.equal(JSON.parse(caller.cli(['config', 'spec.parallel', '--json'])).value, 3);
  caller.cli(['config', 'spec.parallel', '6', '--machine', machine, '--json']);
  assert.equal(JSON.parse(owner.cli(['config', 'spec.parallel', '--json'])).value, 6);
  assert.equal(JSON.parse(caller.cli(['config', 'spec.parallel', '--json'])).value, 3);
});
