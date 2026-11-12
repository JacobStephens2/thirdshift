const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

const FAMILIES = ['focus', 'plain', 'press', 'signal', 'garden', 'factory'];
const THEMES = FAMILIES.flatMap(family => ['light', 'dark'].map(mode => `${family}-${mode}`));
const CUSTOM_URGENT = '#123456';

let home, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-metrics-'));
  const root = join(home, '.thirdshift');
  await mkdir(join(root, 'console/themes'), { recursive: true });
  await writeFile(join(root, 'config.toml'), '[console]\nport = 0\n');
  await writeFile(join(root, 'console/themes/my-metrics.json'), JSON.stringify({
    name: 'my-metrics', base: 'garden-dark', tokens: { urgent: CUSTOM_URGENT }
  }));
  const directory = join(root, 'logs/acme/a-repository-with-a-long-name/commands/issue');
  await mkdir(directory, { recursive: true });
  const kinds = ['Run', 'Spec run', 'Review run', 'Triage run'];
  const outcomes = ['merged', 'failed', 'paused', 'ready for review', null, 'interrupted'];
  let number = 1;
  for (let days = 0; days < 14; days += 1) {
    for (const kind of kinds) {
      const started = new Date(Date.now() - days * 86400000).toISOString();
      const outcome = outcomes[(days + number) % outcomes.length];
      await writeFile(join(directory, `${number}-fixture.command.json`), JSON.stringify({
        version: 1, kind, repository: 'acme/a-repository-with-a-long-name', hostname: 'fixture',
        issue: { number, url: `https://github.com/acme/a-repository-with-a-long-name/issues/${number}` },
        process_id: 0, process_start: 'gone', harness: 'codex', model: 'gpt-6', effort: 'high',
        started_at: started, ended_at: outcome && started, stages: [], session_log: null,
        outcome, cause: null, issue_branch: null, pull_request: null
      }));
      number += 1;
    }
  }
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
    env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit']
  });
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

async function fills(page) {
  return page.evaluate(() => Object.fromEntries(['succeeded', 'failed', 'paused', 'stopped'].map(name => {
    const rect = document.querySelector(`.metrics-legend rect.${name}`);
    return [name, getComputedStyle(rect).fill];
  })));
}

test('Metrics opens by keyboard, fits a phone and stacks outcomes in every Theme without motion', async t => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  await page.keyboard.press('g');
  await page.keyboard.press('m');
  await page.getByRole('heading', { name: 'Metrics', exact: true }).waitFor();
  await page.locator('select[name="window"]').selectOption('7d');
  await Promise.all([page.waitForNavigation(), page.getByRole('button', { name: 'Show', exact: true }).click()]);
  assert.match(page.url(), /window=7d/);
  await page.getByRole('heading', { name: 'Compared with the preceding window' }).waitFor();
  assert.ok(await page.locator('svg.metrics-bar rect').count() > 0);
  for (const name of ['Yield and pull-request survival', 'Post-merge quality and interventions',
    'Efficiency, wasted effort and Repair churn', 'Stage bottlenecks and session friction']) {
    await page.getByRole('heading', { name, exact: true }).waitFor();
  }
  assert.ok(await page.locator('svg.metrics-spark path').count() > 0);
  assert.equal(await page.locator('main script').count(), 0);
  for (const theme of [...THEMES, 'my-metrics']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
    if (theme === 'my-metrics') {
      await page.addStyleTag({ url: '/assets/user-themes.css' });
    }
    const colours = await fills(page);
    assert.equal(new Set(Object.values(colours)).size, 4, `${theme}: ${JSON.stringify(colours)}`);
    for (const colour of Object.values(colours)) assert.notEqual(colour, 'none', theme);
    if (theme === 'my-metrics') assert.equal(colours.failed, 'rgb(18, 52, 86)');
    const spark = await page.evaluate(() => {
      const style = getComputedStyle(document.querySelector('svg.metrics-spark path'));
      return { stroke: style.stroke, fill: style.fill };
    });
    assert.notEqual(spark.stroke, 'none', theme);
    assert.equal(spark.fill, 'none', theme);
    const moving = await page.evaluate(() => [...document.querySelectorAll('main *')].filter(element => {
      const style = getComputedStyle(element);
      return style.animationName !== 'none' || (style.transitionDuration.split(',').some(value => parseFloat(value) > 0));
    }).length);
    assert.equal(moving, 0, theme);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, theme);
  }
  await page.goto(`${origin}/metrics?window=12w`);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  assert.deepEqual(errors, []);
});
