const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { startFixture } = require('./start-fixture.cjs');

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
    await Promise.all([page.waitForURL(`${caller.origin}/config?machine=owner`), page.getByRole('button', { name: 'Save spec.parallel', exact: true }).click()]);
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
