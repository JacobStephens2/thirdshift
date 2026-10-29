const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');

let home, server, exited, browser, origin, folder;
async function firstLine(child) {
  const lines = createInterface({ input: child.stdout });
  try { return (await once(lines, 'line', { signal: AbortSignal.timeout(10000) }))[0]; }
  finally { lines.close(); }
}
before(async () => {
  home = await mkdtemp(join(tmpdir(), "thirdshift-console-stop's-"));
  folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  await mkdir(folder, { recursive: true });
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
    env: { ...process.env, HOME: home }, stdio: ['ignore', 'pipe', 'inherit']
  });
  exited = once(server, 'exit');
  origin = await firstLine(server);
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server && server.exitCode === null) server.kill();
  await exited;
  if (home) await rm(home, { recursive: true, force: true });
});

for (const requeue of [false, true]) {
  test(`Stop confirms, cancels and streams completion (${requeue ? 'requeue' : 'triage'})`, { skip: process.platform !== 'linux' }, async t => {
    // A disposable live process accepts TERM; HTTP tests separately exercise
    // real Command cleanup, fake Harness and fake GitHub label changes.
    const command = spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); console.log('ready'); setInterval(() => {}, 1000)"], { stdio: ['ignore', 'pipe', 'inherit'] });
    const ended = once(command, 'exit');
    t.after(async () => { command.kill('SIGKILL'); await ended; });
    await firstLine(command);
    const stat = await readFile(`/proc/${command.pid}/stat`, 'utf8');
    const ticks = stat.slice(stat.lastIndexOf(') ') + 2).trim().split(/\s+/)[19];
    const identity = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${ticks}`;
    const path = join(folder, '7-fixture.command.json');
    const request = join(folder, '7-fixture.command.stop.json');
    const record = {
      version: 1, kind: 'Run', name: 'acme/widgets#7', repository: 'acme/widgets', hostname: 'fixture',
      issue: { number: 7, url: 'https://github.com/acme/widgets/issues/7', title: 'Work to stop' },
      process_id: command.pid, process_start: identity, harness: 'codex',
      started_at: new Date().toISOString(), ended_at: null, stages: [], outcome: null
    };
    await rm(request, { force: true });
    await writeFile(path, JSON.stringify(record));
    await writeFile(join(folder, '7-fixture.log'), 'Waiting for Stop\n');
    const context = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: requeue ? 'reduce' : 'no-preference' });
    t.after(() => context.close());
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    // Each existing page can reach the same exact Command's Stop control.
    for (const route of ['/', '/lanes', '/stages', '/waterfall', '/history', '/logs', '/security']) {
      await page.goto(`${origin}${route}`);
      if (route === '/history' || route === '/security') {
        // These lists contain ended/Security Commands; use their shared peek
        // route directly to verify the live Command's action is available.
        await page.goto(`${origin}/?page=waterfall`);
      }
      if (route === '/') {
        await page.locator('#inbox .folders a[data-group="working"]').click();
        if (!requeue) await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
      }
      if (await page.locator('#span-tree').count()) {
        await page.locator('.span-row').first().click();
        await page.getByRole('button', { name: 'Command details', exact: true }).click();
      } else if (route === '/') await page.locator('#inbox > .group:not([hidden]) .command .state').first().click();
      else await page.locator('.peek-link').first().click();
      await page.getByRole('button', { name: 'Stop', exact: true }).waitFor();
      if (route === '/') {
        const action = page.locator('#peek form[action="/stop"]');
        const selector = await action.locator('input[name="command"]').inputValue();
        const expected = `thirdshift stop '${selector.replaceAll("'", "'\\''")}'`;
        const copy = action.getByRole('button', { name: 'Copy CLI', exact: true });
        await copy.waitFor();
        for (const width of [1280, 390]) {
          await page.setViewportSize({ width, height: 844 });
          const code = action.locator('code');
          assert.equal(await code.textContent(), expected);
          assert.equal(await code.getAttribute('title'), expected);
          const [stopBox, codeBox, copyBox] = await Promise.all([
            action.getByRole('button', { name: 'Stop', exact: true }).boundingBox(),
            code.boundingBox(), copy.boundingBox()
          ]);
          assert.ok(codeBox.height < 36 && Math.abs(codeBox.y + codeBox.height / 2 - (copyBox.y + copyBox.height / 2)) < 8,
            JSON.stringify({ width, codeBox, copyBox }));
          assert.ok(codeBox.x >= stopBox.x + stopBox.width && copyBox.x > codeBox.x + codeBox.width);
          assert.equal(await code.evaluate(node => node.scrollWidth > node.clientWidth), true);
          assert.ok(copyBox.x + copyBox.width <= width);
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
          await copy.click();
          await action.getByRole('status').filter({ hasText: 'CLI copied.' }).waitFor();
          assert.equal(await page.evaluate(() => navigator.clipboard.readText()), expected);
          assert.equal(new URL(page.url()).pathname, '/');
          assert.equal(await readFile(request, 'utf8').catch(() => null), null);
        }
      }
    }
    await page.getByRole('button', { name: 'Stop', exact: true }).click();
    await page.getByRole('heading', { name: 'Stop this Command?' }).waitFor();
    assert.match(await page.locator('main').textContent(), /needs-triage/);
    await page.getByRole('link', { name: 'Cancel', exact: true }).click();
    assert.equal(await readFile(request, 'utf8').catch(() => null), null);
    await page.goto(`${origin}/stop?command=widgets%237`);
    const observer = await context.newPage();
    await observer.goto(`${origin}/waterfall`);
    await observer.locator('.span-row').first().click();
    await observer.getByRole('button', { name: 'Command details', exact: true }).click();
    await observer.getByRole('button', { name: 'Stop', exact: true }).waitFor();
    assert.equal((await context.request.post(`${origin}/stop`, { form: { command: 'widgets#7', confirmed: 'yes', ending: 'triage' } })).status(), 403);
    await page.getByRole('button', { name: requeue ? 'Requeue instead' : 'Confirm Stop', exact: true }).click();
    await page.getByText('still ending', { exact: true }).waitFor();
    assert.equal(await page.locator('.stopping-bar').evaluate(node => getComputedStyle(node).animationName), 'none');
    assert.equal(await page.getByRole('button', { name: /Stop|Requeue/ }).count(), 0);
    const asked = JSON.parse(await readFile(request, 'utf8'));
    assert.equal(asked.requeue, requeue);
    assert.equal(asked.from_console, true);
    await page.evaluate(() => { window.sameDocument = true; });
    record.outcome = 'interrupted';
    record.ended_at = new Date().toISOString();
    record.issue_branch = 'issue-7';
    record.pull_request = 'https://github.com/acme/widgets/pull/10';
    await writeFile(path, JSON.stringify(record));
    await page.getByText('Issue branch left: issue-7', { exact: true }).waitFor();
    await page.getByText('Pull request left: https://github.com/acme/widgets/pull/10', { exact: true }).waitFor();
    await observer.getByRole('button', { name: 'Stop', exact: true }).waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => window.sameDocument), true);
    assert.equal(await page.getByText('still ending', { exact: true }).count(), 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    assert.deepEqual(errors, []);
  });
}
