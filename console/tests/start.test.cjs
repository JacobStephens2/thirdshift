const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { mkdir, writeFile } = require('node:fs/promises');
const { join } = require('node:path');
const { startFixture } = require('./start-fixture.cjs');

let fixture;
before(async () => { fixture = await startFixture(); });
after(async () => { await fixture?.close(); });

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
    `*/5 * * * * cd '${fixture.repo}' && thirdshift pickup base "$BRANCH"`
  ].join('\n'));
  const page = await fixture.page();
  t.after(() => page.context().close());
  await page.goto(`${fixture.origin}/cronjobs`);
  assert.equal(await page.getByRole('button', { name: 'Run now', exact: true }).count(), 3);
  for (const [index, kind, harness] of [[0, 'architect', 'codex'], [1, 'secure', 'codex'], [2, 'review', 'codex']]) {
    await page.getByRole('button', { name: 'Run now', exact: true }).nth(index).click();
    const confirm = page.getByRole('dialog', { name: 'Confirm start', exact: true });
    await confirm.waitFor();
    assert.match(await confirm.locator('[data-start-description]').textContent(), new RegExp(`${kind}.*Harness ${harness}`));
    const request = JSON.parse(await confirm.locator('pre').textContent());
    if (kind === 'architect') { assert.equal(request.focus, 'Console starts'); assert.equal(request.plan_only, true); }
    if (kind === 'secure') assert.equal(request.security_fix, false);
    if (kind === 'review') { assert.equal(request.author, 'fixture'); assert.equal(request.recommend_only, true); }
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
