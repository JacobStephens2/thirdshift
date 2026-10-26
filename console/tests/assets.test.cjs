const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, cp, readFile, writeFile, rename, rm } = require('node:fs/promises');
const { join, resolve } = require('node:path');
const { tmpdir } = require('node:os');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { test } = require('node:test');

test('development assets restyle open pages and reload changed scripts over one stream', async t => {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift-assets-'));
  let server, exited, browser;
  try {
    const assets = join(home, 'assets');
    await cp(resolve(__dirname, '../../src/console/assets'), assets, { recursive: true });
    const cssPath = join(assets, 'console.css');
    const css = await readFile(cssPath, 'utf8');
    await writeFile(cssPath, `${css}\nbody { --asset-test: before; }\n`);
    await mkdir(join(home, '.thirdshift'));
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift'), ['console', '--no-sign-in'], {
      env: { ...process.env, HOME: home, THIRDSHIFT_CONSOLE_ASSETS: assets }, stdio: ['ignore', 'pipe', 'inherit'],
    });
    exited = once(server, 'exit');
    const lines = createInterface({ input: server.stdout });
    const [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) });
    lines.close();
    if (!(await (await fetch(`${origin}/assets/console.css`)).text()).includes('--asset-test: before')) {
      t.skip('binary does not enable debug development assets');
      return;
    }
    browser = await chromium.launch({ headless: true });
    const paths = ['/', '/logs', '/account', '/console-log'];
    const pages = await Promise.all(paths.map(() => browser.newPage()));
    const violations = [], streams = paths.map(() => 0), errors = [];
    for (const [index, page] of pages.entries()) {
      await page.exposeFunction('cspViolation', value => violations.push(value));
      page.on('pageerror', error => errors.push(error.message));
      await page.addInitScript(() => document.addEventListener('securitypolicyviolation', event => window.cspViolation(event.violatedDirective)));
      page.on('request', request => { if (new URL(request.url()).pathname === '/events') streams[index]++; });
      const look = page.waitForResponse(response => response.url() === `${origin}/look`);
      await page.goto(`${origin}${paths[index]}`);
      await look;
      await page.waitForFunction(() => getComputedStyle(document.body).getPropertyValue('--asset-test').trim() === 'before');
      await page.evaluate(() => { window.preservedThroughStyleChange = true; });
    }
    const began = Date.now();
    // Atomic replacement is how editors commonly save a stylesheet.
    await writeFile(`${cssPath}.tmp`, `${css}\nbody { --asset-test: changed; }\n`);
    await rename(`${cssPath}.tmp`, cssPath);
    for (const page of pages) {
      await page.waitForFunction(() => getComputedStyle(document.body).getPropertyValue('--asset-test').trim() === 'changed', null, { timeout: 2000 }).catch(async error => {
        t.diagnostic(JSON.stringify({ errors, streams }));
        throw error;
      });
      assert.equal(await page.evaluate(() => window.preservedThroughStyleChange), true);
    }
    assert.ok(Date.now() - began < 2000, 'stylesheet update should arrive in about a second');
    assert.deepEqual(streams, paths.map(() => 1));
    // Imported Theme styles are refreshed through their containing stylesheet.
    const themesPath = join(assets, 'themes.css');
    await writeFile(themesPath, `${await readFile(themesPath, 'utf8')}\nbody { --import-test: changed; }\n`);
    for (const page of pages) await page.waitForFunction(() => getComputedStyle(document.body).getPropertyValue('--import-test').trim() === 'changed');
    // Delay one real response across a newer edit. It must never win later.
    const page = pages[0];
    let captured, release;
    const held = new Promise(resolve => { captured = resolve; });
    const released = new Promise(resolve => { release = resolve; });
    let first = true;
    await page.route('**/assets/console.css?revision=*', async route => {
      if (!first) return route.continue();
      first = false;
      const response = await route.fetch();
      captured();
      await released;
      await route.fulfill({ response });
    });
    try {
      await writeFile(cssPath, `${css}\nbody { --asset-test: stale; }\n`);
      await Promise.race([held, new Promise((_, reject) => setTimeout(() => reject(new Error('stylesheet request missing')), 5000))]);
      await writeFile(cssPath, `${css}\nbody { --asset-test: newest; }\n`);
      await page.waitForFunction(() => getComputedStyle(document.body).getPropertyValue('--asset-test').trim() === 'newest');
    } finally { release(); }
    await page.waitForFunction(() => !document.querySelector('link[media="not all"]'));
    assert.equal(await page.evaluate(() => getComputedStyle(document.body).getPropertyValue('--asset-test').trim()), 'newest');
    await page.unroute('**/assets/console.css?revision=*');
    const scriptPath = join(assets, 'changes.js');
    const navigations = pages.map(page => page.waitForEvent('load'));
    await writeFile(scriptPath, `${await readFile(scriptPath, 'utf8')}\nwindow.changedScriptRan = true;\n`);
    await Promise.all(navigations);
    for (const page of pages) {
      await page.waitForFunction(() => window.changedScriptRan === true);
      assert.equal(await page.evaluate(() => window.preservedThroughStyleChange), undefined);
    }
    assert.deepEqual(violations, []);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    if (server && server.exitCode === null) server.kill();
    if (exited) await exited;
    await rm(home, { recursive: true, force: true });
  }
});
