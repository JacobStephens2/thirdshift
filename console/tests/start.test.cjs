const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { join } = require('node:path');
const { startFixture } = require('./start-fixture.cjs');
const { replaceRecord } = require('./records.cjs');
const { hostname } = require('node:os');

let fixture;
before(async () => { fixture = await startFixture(); });
after(async () => { await fixture?.close(); });

test('closed Start work is one line across Themes and keeps native keyboard and pointer operation', async t => {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const page = await fixture.page({ viewport });
    t.after(() => page.context().close());
    const panel = page.getByRole('region', { name: 'Start work', exact: true });
    const summary = panel.getByText('Start work', { exact: true });
    const issue = panel.getByLabel('Issue number or URL', { exact: true });
    for (const family of ['focus', 'plain', 'press', 'signal', 'garden', 'factory']) {
      for (const mode of ['light', 'dark']) {
        await t.test(`${family}-${mode} at ${viewport.width}px`, async () => {
          await page.evaluate(async theme => {
            document.documentElement.dataset.theme = theme;
            await document.fonts.ready;
          }, `${family}-${mode}`);
          const closed = await panel.boundingBox();
          assert.ok(closed.height <= 50, `closed Start work height ${closed.height}px exceeds one line`);
          assert.ok((await summary.boundingBox()).height >= 44, 'summary retains a comfortable hit area');
          assert.equal(await issue.isVisible(), false);
          await summary.focus();
          await summary.press('Enter');
          await issue.waitFor();
          assert.equal(await summary.evaluate(element => {
            const style = getComputedStyle(element);
            return element === document.activeElement && style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) > 0;
          }), true, 'keyboard focus stays visible');
          const expanded = await panel.boundingBox();
          const input = await issue.boundingBox();
          const submit = await panel.getByRole('button', { name: 'Review start', exact: true }).boundingBox();
          assert.ok(input.x - expanded.x >= 12 && expanded.x + expanded.width - input.x - input.width >= 12);
          assert.ok(expanded.y + expanded.height - submit.y - submit.height >= 12, 'expanded controls keep bottom spacing');
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
          await summary.press('Space');
          await issue.waitFor({ state: 'hidden' });
          await summary.click();
          await issue.waitFor();
          await summary.click();
          await issue.waitFor({ state: 'hidden' });
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
        });
      }
    }
    await page.context().close();
  }
});

test('Start Fix run confirms the pull request and defaults, cancels, and submits once', async t => {
  await fixture.readyPullRequest();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const page = await fixture.page({ viewport });
    t.after(() => page.context().close());
    const requests = [];
    let respond;
    let received;
    const receiving = new Promise(resolve => { received = resolve; });
    await page.route('**/api/v1/runs', async route => {
      requests.push({ body: route.request().postDataJSON(), csrf: route.request().headers()['x-csrf-token'] });
      received();
      await new Promise(resolve => { respond = resolve; });
      await route.fulfill({ json: { version: 1, name: 'acme/widgets fix #42' } });
    });
    const start = page.getByRole('button', { name: 'Start Fix run', exact: true });
    await start.click({ timeout: 5000 });
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    const request = { command: 'fix', repository: 'acme/widgets', pull_request: 'https://github.com/acme/widgets/pull/42' };
    assert.deepEqual(JSON.parse(await confirm.locator('pre').textContent()), request);
    assert.match(await confirm.textContent(), /https:\/\/github.com\/acme\/widgets\/pull\/42/);
    assert.ok((await confirm.textContent()).includes(`machine ${hostname()}`));
    assert.match(await confirm.textContent(), /User config and the CLI defaults/);
    assert.deepEqual(requests, []);
    await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
    await confirm.waitFor({ state: 'hidden' });
    await start.click();
    await page.keyboard.press('Escape');
    await confirm.waitFor({ state: 'hidden' });
    assert.deepEqual(requests, []);
    await start.click();
    await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
    try {
      await page.locator('[data-start-result]').filter({ hasText: 'Starting…' }).waitFor();
      await receiving;
      assert.equal(await start.isDisabled(), true);
      assert.equal(requests.length, 1);
      assert.deepEqual(requests[0].body, request);
      assert.ok(requests[0].csrf);
    } finally { respond?.(); }
    await page.locator('[data-start-result]').filter({ hasText: 'Started Command acme/widgets fix #42.' }).waitFor();
    await start.click();
    await confirm.waitFor();
    await page.keyboard.press('Escape');
    await confirm.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.context().close();
  }
});

test('Start Fix run requires an eligible row, current run permission and a known Launch directory', async t => {
  const configPath = join(fixture.home, '.thirdshift/config.toml');
  const config = await readFile(configPath, 'utf8');
  const cronPath = join(fixture.home, 'crontab');
  const cron = await readFile(cronPath, 'utf8');
  t.after(async () => {
    await replaceRecord(configPath, config);
    await writeFile(cronPath, cron);
    fixture.scopes('read,security,act,run,admin');
    await fixture.readyPullRequest();
  });
  const page = await fixture.page();
  t.after(() => page.context().close());
  for (const [outcome, ci, mergeable, eligible] of [
    ['ready for review', 'red', 'mergeable', true],
    ['ready for review', 'green', 'conflicting', true],
    ['ready for review', 'pending', 'conflicting', true],
    ['ready for review', 'green', 'mergeable', false],
    ['ready for review', 'unknown', 'unknown', false],
    ['failed', 'red', 'conflicting', false],
    ['merged', 'red', 'conflicting', false],
  ]) {
    await fixture.readyPullRequest(42, { outcome }, { ci, mergeable });
    await page.goto(`${fixture.origin}/?section=${outcome === 'failed' ? 'needs-you' : outcome === 'merged' ? 'done' : 'ready-for-review'}`);
    assert.equal(await page.getByRole('button', { name: 'Start Fix run', exact: true }).count(), Number(eligible), `${outcome}:${ci}:${mergeable}`);
  }
  await fixture.readyPullRequest();
  for (const scopes of ['read', 'read,act', 'read,run']) {
    fixture.scopes(scopes);
    await page.goto(fixture.origin);
    assert.equal(await page.getByRole('button', { name: 'Start Fix run', exact: true }).count(), Number(scopes.includes('run')));
    assert.equal(await page.getByRole('button', { name: 'Copy fix command', exact: true }).count(), 1);
  }
  await replaceRecord(configPath, config.replace(/^launch_directories = .*\n/, ''));
  await writeFile(cronPath, '');
  await page.goto(fixture.origin);
  assert.equal(await page.getByRole('button', { name: 'Start Fix run', exact: true }).count(), 0);
  assert.equal(await page.getByRole('button', { name: 'Copy fix command', exact: true }).count(), 1);
});

test('Start Fix run recognizes GitHub repository identity across URL casing', async t => {
  const pull_request = 'https://github.com/ACME/Widgets/pull/42';
  await fixture.readyPullRequest(42, { pull_request });
  t.after(() => fixture.readyPullRequest());
  const page = await fixture.page();
  t.after(() => page.context().close());
  await page.getByRole('button', { name: 'Start Fix run', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
  await confirm.waitFor();
  assert.deepEqual(JSON.parse(await confirm.locator('pre').textContent()), {
    command: 'fix', repository: 'ACME/Widgets', pull_request
  });
  await page.keyboard.press('Escape');
  await confirm.waitFor({ state: 'hidden' });
});

test('Start Fix run works after live row and permission changes and preserves uncertain-response feedback', async t => {
  await fixture.readyPullRequest(42, {}, { ci: 'green' });
  fixture.scopes('read');
  t.after(async () => { fixture.scopes('read,security,act,run,admin'); await fixture.readyPullRequest(); });
  const page = await fixture.page();
  t.after(() => page.context().close());
  const start = page.locator('.command[data-command$="42-fixture.command.json"]').getByRole('button', { name: 'Start Fix run', exact: true });
  assert.equal(await start.count(), 0);
  fixture.scopes('read,run');
  await fixture.readyPullRequest(42, {}, { ci: 'green', mergeable: 'conflicting' });
  await start.waitFor();
  await start.click();
  const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
  await confirm.waitFor();
  await page.keyboard.press('Escape');
  await confirm.waitFor({ state: 'hidden' });
  let requests = 0, release, received;
  const receiving = new Promise(resolve => { received = resolve; });
  await page.route('**/api/v1/runs', async route => {
    requests++;
    received();
    await new Promise(resolve => { release = resolve; });
    await route.abort('failed');
  });
  await start.click();
  await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
  await receiving;
  try {
    await fixture.readyPullRequest(42, { cause: 'Updated while start is pending' });
    await page.getByText('Updated while start is pending', { exact: true }).waitFor({ state: 'attached' });
    assert.equal(await start.isDisabled(), true);
    assert.equal(requests, 1);
  } finally { release(); }
  const result = page.locator('[data-start-result]');
  await result.filter({ hasText: 'Check Inbox and logs before retrying.' }).waitFor();
  await fixture.readyPullRequest(42, { cause: 'Updated after uncertain response' });
  await page.getByText('Updated after uncertain response', { exact: true }).waitFor({ state: 'attached' });
  assert.match(await result.textContent(), /Check Inbox and logs before retrying/);
  assert.equal(requests, 1);
  const inserted = await fixture.readyPullRequest(43);
  t.after(() => rm(inserted, { force: true }));
  const newStart = page.locator('.command[data-command$="43-fixture.command.json"]').getByRole('button', { name: 'Start Fix run', exact: true });
  await newStart.click();
  await confirm.waitFor();
  assert.equal(JSON.parse(await confirm.locator('pre').textContent()).pull_request, 'https://github.com/acme/widgets/pull/43');
  await page.keyboard.press('Escape');
  await confirm.waitFor({ state: 'hidden' });
  fixture.scopes('read');
  await start.waitFor({ state: 'detached' });
});

test('Start Fix run forwards only to the owning machine with its current run permission', async t => {
  const owner = await startFixture();
  t.after(() => owner.close());
  const caller = await startFixture({ THIRDSHIFT_FLEET_POLL_MS: '100' });
  t.after(() => caller.close());
  await owner.readyPullRequest();
  const token = owner.cli(['console', 'token', 'create', 'fix-caller', '--scopes', 'read,run']).trim();
  caller.cli(['console', 'fleet', 'add', 'owner', owner.origin], `${token}\n`);
  caller.scopes('read');
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const page = await caller.page({ viewport });
    t.after(() => page.context().close());
    const row = page.locator('.command[data-machine="owner"]');
    const start = row.getByRole('button', { name: 'Start Fix run', exact: true });
    await start.click();
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    assert.match(await confirm.textContent(), /machine owner/);
    const requests = [];
    page.on('request', request => {
      if (request.method() === 'POST' && /\/runs(?:\?|$)/.test(request.url())) {
        requests.push({ path: new URL(request.url()).pathname + new URL(request.url()).search, body: request.postDataJSON() });
      }
    });
    const forwarding = page.waitForResponse(response => response.url().includes('/fleet/runs?machine=owner'));
    await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
    // The fake GitHub refuses PR admission. This still exercises real forwarding,
    // destination authorization, CSRF and attribution, without starting a Harness.
    const response = await forwarding;
    const failure = (await response.json()).error;
    assert.equal(response.status(), 422, failure);
    await page.locator('[data-start-result]').filter({ hasText: failure }).waitFor();
    assert.deepEqual(requests, [{ path: '/fleet/runs?machine=owner', body: {
      command: 'fix', repository: 'acme/widgets', pull_request: 'https://github.com/acme/widgets/pull/42'
    } }]);
    assert.match(owner.cli(['console', 'log', '--json']), /owner@example.test from the Console on fix-caller/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.context().close();
  }
  const page = await caller.page();
  t.after(() => page.context().close());
  const row = page.locator('.command[data-machine="owner"]');
  await row.getByRole('button', { name: 'Start Fix run', exact: true }).waitFor();
  owner.scopes('read');
  await row.getByRole('button', { name: 'Start Fix run', exact: true }).waitFor({ state: 'detached' });
  await row.getByRole('button', { name: 'Copy fix command', exact: true }).waitFor();
});

test('Start requires confirmation, names its Harness and shows the Command in Inbox', async t => {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    await t.test(`${viewport.width}px`, async t => {
      const page = await fixture.page({ viewport });
      t.after(() => page.context().close());
      const requests = [];
      page.on('request', request => { if (request.method() === 'POST' && request.url().endsWith('/api/v1/runs')) requests.push(request.postDataJSON()); });
      await page.getByText('Start work', { exact: true }).click();
      await page.getByLabel('Repository', { exact: true }).selectOption('acme/widgets');
      await page.getByLabel('Issue number or URL', { exact: true }).fill(String(viewport.width));
      await page.getByRole('button', { name: 'Review start', exact: true }).click();
      const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
      await confirm.waitFor();
      assert.match(await confirm.textContent(), /codex/);
      assert.match(await confirm.textContent(), new RegExp(`acme/widgets.*${viewport.width}`));
      assert.deepEqual(requests, []);
      await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
      await confirm.waitFor({ state: 'hidden' });
      assert.deepEqual(requests, []);
      await page.getByRole('button', { name: 'Review start', exact: true }).click();
      await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
      await page.locator('[data-start-result]').filter({ hasText: `Started Command acme/widgets#${viewport.width}.` }).waitFor();
      assert.equal(requests.length, 1);
      assert.equal(requests[0].repository, 'acme/widgets');
      assert.equal(requests[0].issue, String(viewport.width));
      await page.locator('#inbox .command').filter({ hasText: `#${viewport.width}` }).waitFor();
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    });
  }
});

test('Run now confirms the cron Pass options and reports its skip outcome', async t => {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const page = await fixture.page({ viewport });
    t.after(() => page.context().close());
    await page.goto(`${fixture.origin}/cronjobs`);
    const requests = [];
    page.on('request', request => { if (request.method() === 'POST' && request.url().endsWith('/api/v1/runs')) requests.push(request.postDataJSON()); });
    await page.getByRole('button', { name: 'Run now', exact: true }).click();
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    assert.match(await confirm.textContent(), /codex/);
    assert.match(await confirm.textContent(), /highest-first/);
    assert.match(await confirm.textContent(), /main/);
    assert.deepEqual(requests, []);
    await page.keyboard.press('Escape');
    await confirm.waitFor({ state: 'hidden' });
    assert.deepEqual(requests, []);
    await page.getByRole('button', { name: 'Run now', exact: true }).click();
    await confirm.getByRole('button', { name: 'Start now', exact: true }).click();
    await page.locator('[data-start-result]').filter({ hasText: /Pass skipped:.*no ready issue/i }).waitFor({ timeout: 15000 });
    assert.deepEqual(requests, [{ repository: 'acme/widgets', command: 'pickup', base: 'main', order: 'highest-first', email: { mode: 'never' }, harness: 'codex' }]);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
});

test('read and act sessions see neither Start nor Run now', async t => {
  t.after(() => fixture.scopes('read,security,act,run,admin'));
  for (const scopes of ['read', 'read,act']) {
    fixture.scopes(scopes);
    const page = await fixture.page();
    try {
      assert.equal(await page.locator('[data-start-area], #start-confirm').count(), 0);
      await page.goto(`${fixture.origin}/cronjobs`);
      await page.getByRole('heading', { name: 'Cron jobs', exact: true }).waitFor();
      assert.equal(await page.getByRole('button', { name: 'Run now', exact: true }).count(), 0);
      assert.equal(await page.locator('[data-start-area], #start-confirm').count(), 0);
    } finally { await page.context().close(); }
  }
});

test('Start reviews every shared option before submission and remains inert without JavaScript', async t => {
  const page = await fixture.page({ viewport: { width: 390, height: 844 } });
  t.after(() => page.context().close());
  await page.getByText('Start work', { exact: true }).click();
  await page.getByText('Shared options', { exact: true }).click();
  await page.getByLabel('Issue number or URL', { exact: true }).fill('https://github.com/acme/widgets/issues/7');
  await page.getByLabel('Harness', { exact: true }).selectOption('claude');
  await page.getByLabel('Model', { exact: true }).fill('opus');
  await page.getByLabel('Effort', { exact: true }).fill('high');
  await page.getByLabel('Parallel Tickets', { exact: true }).fill('2');
  await page.getByLabel('Email', { exact: true }).selectOption('unless_merged');
  await page.getByLabel('Email address', { exact: true }).fill('owner@example.test');
  for (const name of ['Merge', 'Base fix', 'Security review', 'Security fix', 'Close Security findings']) {
    await page.getByLabel(name, { exact: true }).selectOption('false');
  }
  await page.getByRole('button', { name: 'Review start', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
  await confirm.waitFor();
  assert.deepEqual(JSON.parse(await confirm.locator('pre').textContent()), {
    repository: 'acme/widgets', issue: 'https://github.com/acme/widgets/issues/7',
    harness: 'claude', model: 'opus', effort: 'high', parallel: 2,
    merge: false, base_fix: false, security_review: false, security_fix: false, security_close: false,
    email: { mode: 'unless_merged', address: 'owner@example.test' }
  });
  assert.match(await confirm.locator('[data-start-description]').textContent(), /Harness claude/);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  const noScript = await fixture.page({ javaScriptEnabled: false });
  t.after(() => noScript.context().close());
  await noScript.getByText('Start work', { exact: true }).click();
  assert.equal(await noScript.getByRole('button', { name: 'Review start', exact: true }).isDisabled(), true);
  assert.equal(await noScript.getByRole('button', { name: 'Start Fix run', exact: true }).isDisabled(), true);
  await noScript.goto(`${fixture.origin}/cronjobs`);
  assert.equal(await noScript.getByRole('button', { name: 'Run now', exact: true }).isDisabled(), true);
});

test('Run now recognizes all Pass kinds and leaves shell expressions display-only', async t => {
  const original = `*/5 * * * * cd '${fixture.repo}' && thirdshift pickup base main highest-first no-email harness codex\n`;
  t.after(() => writeFile(join(fixture.home, 'crontab'), original));
  await writeFile(join(fixture.home, 'crontab'), [
    `*/5 * * * * cd '${fixture.repo}' && thirdshift architect 'Console starts' base main --plan-only >> /tmp/fixture-cron.log 2>&1`,
    `*/5 * * * * cd '${fixture.repo}' && thirdshift secure base main no-security-fix`,
    `*/5 * * * * cd '${fixture.repo}' && thirdshift review --author fixture --recommend-only`,
    `*/5 * * * * cd '${fixture.repo}' && thirdshift fix base main author fixture no-merge no-base-fix no-email harness claude`,
    `*/5 * * * * cd '${fixture.repo}' && thirdshift fix https://github.com/acme/widgets/pull/1`,
    `*/5 * * * * cd '${fixture.repo}' && thirdshift pickup base "$BRANCH"`
  ].join('\n'));
  const page = await fixture.page();
  t.after(() => page.context().close());
  await page.goto(`${fixture.origin}/cronjobs`);
  assert.equal(await page.getByRole('button', { name: 'Run now', exact: true }).count(), 4);
  for (const [index, kind, harness] of [[0, 'architect', 'codex'], [1, 'secure', 'codex'], [2, 'review', 'codex'], [3, 'fix', 'claude']]) {
    await page.getByRole('button', { name: 'Run now', exact: true }).nth(index).click();
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    assert.match(await confirm.locator('[data-start-description]').textContent(), new RegExp(`${kind}.*Harness ${harness}`));
    const request = JSON.parse(await confirm.locator('pre').textContent());
    if (kind === 'architect') { assert.equal(request.focus, 'Console starts'); assert.equal(request.plan_only, true); }
    if (kind === 'secure') assert.equal(request.security_fix, false);
    if (kind === 'review') { assert.equal(request.author, 'fixture'); assert.equal(request.recommend_only, true); }
    if (kind === 'fix') {
      assert.equal(request.base, 'main'); assert.equal(request.author, 'fixture');
      assert.equal(request.merge, false); assert.equal(request.base_fix, false);
      assert.deepEqual(request.email, { mode: 'never' });
    }
    await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  }
});

test('Run now preserves numeric arguments beside shell redirections', async t => {
  const original = `*/5 * * * * cd '${fixture.repo}' && thirdshift pickup base main highest-first no-email harness codex\n`;
  t.after(() => writeFile(join(fixture.home, 'crontab'), original));
  const page = await fixture.page();
  t.after(() => page.context().close());
  for (const redirection of ['', ' >> /tmp/fixture-cron.log 2>&1', ' 2>&1', ' 2>> /tmp/fixture-errors.log']) {
    await writeFile(join(fixture.home, 'crontab'), `*/5 * * * * cd '${fixture.repo}' && thirdshift pickup base main no-email harness codex parallel 2${redirection}\n`);
    await page.goto(`${fixture.origin}/cronjobs`);
    await page.getByRole('button', { name: 'Run now', exact: true }).click();
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    const request = JSON.parse(await confirm.locator('pre').textContent());
    assert.equal(request.parallel, 2);
    assert.equal(request.harness, 'codex');
    await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  }
});

test('Run now offers Custom-process Passes without adding an issue target', async t => {
  const original = `*/5 * * * * cd '${fixture.repo}' && thirdshift pickup base main highest-first no-email harness codex\n`;
  t.after(() => writeFile(join(fixture.home, 'crontab'), original));
  const processes = join(fixture.home, '.thirdshift/processes');
  await mkdir(processes, { recursive: true });
  await writeFile(join(processes, 'inspect.toml'), 'version = 1\ndescription = "Inspect"\n[[steps]]\nname = "inspect"\ncommand = "true"\n');
  await writeFile(join(fixture.home, 'crontab'), `*/5 * * * * cd '${fixture.repo}' && thirdshift process inspect no-email harness codex\n`);
  const page = await fixture.page();
  t.after(() => page.context().close());
  await page.goto(`${fixture.origin}/cronjobs`);
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
  await confirm.waitFor();
  const request = JSON.parse(await confirm.locator('pre').textContent());
  assert.equal(request.command, 'process');
  assert.equal(request.name, 'inspect');
  assert.equal(request.issue, undefined);
  assert.equal(request.harness, 'codex');
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
});
