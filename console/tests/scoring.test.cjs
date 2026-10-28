const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

let home, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-scoring-'));
  execFileSync('python3', [resolve(__dirname, '../../tests/console/scoring_fixture.py'), home]);
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
    env: { ...process.env, HOME: home, PATH: `${join(home, 'bin')}:${process.env.PATH}` },
    stdio: ['ignore', 'pipe', 'inherit']
  });
  exited = once(server, 'exit');
  const lines = createInterface({ input: server.stdout });
  try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) }); }
  finally { lines.close(); }
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server && server.exitCode === null) {
    server.kill();
    const deadline = setTimeout(() => server.kill('SIGKILL'), 2000);
    await exited;
    clearTimeout(deadline);
  }
  await exited;
  if (home) await rm(home, { recursive: true, force: true });
});

test('Scores and Bug origins navigate by keyboard, submit their CLI actions and fit a phone', async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  await page.getByRole('link', { name: 'Scores', exact: true }).click();
  await page.getByRole('heading', { name: 'Scores', exact: true }).waitFor();
  assert.match(await page.locator('main').textContent(), /2026-W43/);
  assert.doesNotMatch(await page.content(), /Model secret/);
  const form = page.locator('form[action="/scores/mine"]');
  const criteria = ['Correct', 'Scoped', 'Tested', 'Honest', 'Standards'];
  for (const criterion of criteria.slice(0, 4)) {
    await form.locator(`[name="${criterion}"]`).selectOption('pass');
    await form.locator(`[name="${criterion}_reason"]`).fill(`My ${criterion} reason`);
  }
  await form.getByRole('button', { name: 'Save all five Scores' }).click();
  assert.doesNotMatch(await page.content(), /Model secret/);
  assert.equal(await form.count(), 1);
  await form.locator('[name="Standards"]').selectOption('fail');
  await form.locator('[name="Standards_reason"]').fill('My <script>reason</script>');
  await Promise.all([
    page.waitForNavigation(),
    form.getByRole('button', { name: 'Save all five Scores' }).click()
  ]);
  assert.match(await page.locator('main').textContent(), /Model secret <script>unsafe\(\)<\/script>/);
  assert.match(await page.locator('main').textContent(), /My <script>reason<\/script>/);
  assert.match(await page.getByRole('region', { name: 'Calibration' }).textContent(), /0\/1 agreements/);
  assert.equal(await page.locator('main script').count(), 0);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.getByRole('link', { name: 'Score again', exact: true }).click();
  assert.doesNotMatch(await page.content(), /Model secret/);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.getByRole('link', { name: 'Bug origins', exact: true }).click();
  for (const [issue, action] of [[8, 'Confirm'], [9, 'Correct'], [10, 'Reject']]) {
    const row = page.getByRole('article', { name: `https://github.com/acme/widgets/issues/${issue}`, exact: true });
    if (action === 'Correct') await row.getByRole('textbox', { name: 'Correct pull request' }).fill('24');
    await Promise.all([page.waitForNavigation(), row.getByRole('button', { name: action, exact: true }).click()]);
    const updated = page.getByRole('article', { name: `https://github.com/acme/widgets/issues/${issue}`, exact: true });
    assert.match(await updated.textContent(), action === 'Reject' ? /State: rejected/ : /State: confirmed/);
    if (action === 'Correct') assert.match(await updated.textContent(), /pull\/24.*corrected/s);
  }
  assert.equal(await page.locator('main script').count(), 0);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.goto(origin);
  await page.keyboard.press('g');
  await page.keyboard.press('q');
  await page.waitForURL(`${origin}/scores`);
  await page.keyboard.press('g');
  await page.keyboard.press('u');
  await page.waitForURL(`${origin}/bug-origins`);
  assert.deepEqual(errors, []);
});
