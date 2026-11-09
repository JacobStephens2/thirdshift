const { test } = require('node:test');
const assert = require('node:assert/strict');
const { hostname } = require('node:os');
const { startFixture } = require('./start-fixture.cjs');

test('local config save confirms the value and machine at the saved setting', async t => {
  const fixture = await startFixture();
  t.after(() => fixture.close());
  const page = await fixture.page({ viewport: { width: 1440, height: 900 } });
  await page.goto(`${fixture.origin}/config`);
  await page.getByLabel('Value for spec.parallel', { exact: true }).fill('10');
  await page.getByRole('button', { name: 'Save spec.parallel', exact: true }).click();
  await page.getByRole('status').filter({ hasText: `Saved spec.parallel = 10 on ${hostname()}` }).waitFor();
  assert.equal(new URL(page.url()).hash, '#config-spec.parallel');
  const section = page.locator('[id="config-spec.parallel"]');
  assert.equal(await section.getByLabel('Value for spec.parallel', { exact: true }).inputValue(), '10');
  await page.waitForFunction(() => {
    const top = document.getElementById('config-spec.parallel').getBoundingClientRect().top;
    return top >= -1 && top < innerHeight;
  });
});

test('config feedback stays visible and redacted on local and Fleet machines without requiring JavaScript', async t => {
  const owner = await startFixture();
  t.after(() => owner.close());
  const caller = await startFixture();
  t.after(() => caller.close());
  const token = owner.cli(['console', 'token', 'create', 'config-browser', '--scopes', 'admin']).trim();
  caller.cli(['console', 'fleet', 'add', 'orchestrate', owner.origin], `${token}\n`);
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    for (const javaScriptEnabled of [true, false]) {
      for (const machine of ['', 'orchestrate']) {
        const page = await caller.page({ viewport, javaScriptEnabled });
        const suffix = machine ? `?machine=${machine}` : '';
        const target = machine || hostname();
        await page.goto(`${caller.origin}/config${suffix}`);
        await page.getByLabel('Value for spec.parallel', { exact: true }).fill('10');
        await page.getByRole('button', { name: 'Save spec.parallel', exact: true }).click();
        const status = page.locator('[id="config-spec.parallel"] [role="status"]');
        await status.waitFor();
        assert.equal(await status.textContent(), `Saved spec.parallel = 10 on ${target}.`);
        const savedURL = new URL(page.url());
        assert.equal(savedURL.hash, '#config-spec.parallel');
        assert.equal(savedURL.searchParams.get('machine') || '', machine);
        await page.waitForFunction(() => {
          const rect = document.getElementById('config-spec.parallel').querySelector('[role="status"]').getBoundingClientRect();
          return rect.top >= 0 && rect.bottom <= innerHeight;
        });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);

        await page.getByLabel('Value for otlp.endpoint', { exact: true }).fill('https://collector.example.test/private-fixture?token=secret-fixture');
        await page.getByRole('button', { name: 'Save otlp.endpoint', exact: true }).click();
        const redacted = page.locator('[id="config-otlp.endpoint"] [role="status"]');
        await redacted.waitFor();
        assert.equal(await redacted.textContent(), `Saved otlp.endpoint = "[redacted]" on ${target}.`);
        assert.equal(new URL(page.url()).hash, '#config-otlp.endpoint');
        assert.equal(new URL(page.url()).searchParams.get('machine') || '', machine);
        assert.doesNotMatch(await page.content(), /private-fixture|secret-fixture/);
        assert.doesNotMatch(page.url(), /collector|private-fixture|secret-fixture/);

        await page.getByLabel('Value for spec.parallel', { exact: true }).fill('0');
        const [invalid] = await Promise.all([
          page.waitForNavigation(),
          page.getByRole('button', { name: 'Save spec.parallel', exact: true }).click()
        ]);
        assert.equal(invalid.status(), machine ? 422 : 400);
        assert.match(await page.textContent('body'), /spec.parallel must be a whole number from 1 up/);
        assert.doesNotMatch(await page.textContent('body'), /Saved spec.parallel/);
        await page.context().close();
      }
    }
  }
});
