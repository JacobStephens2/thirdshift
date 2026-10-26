const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('playwright');

let home, records, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), 'thirdshift-notifications-'));
  records = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  await mkdir(records, { recursive: true });
  await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
  await item(1);
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

async function item(number, outcome = 'failed') {
  const now = new Date().toISOString();
  await writeFile(join(records, `${number}.command.json`), JSON.stringify({
    version: 1, kind: 'Run', name: `${number}`, repository: 'acme/widgets', hostname: 'fixture',
    issue: { number, url: `https://github.com/acme/widgets/issues/${number}`, title: `Problem ${number}` },
    process_id: 0, process_start: 'gone', harness: 'codex', started_at: now, ended_at: now,
    stages: [], outcome, cause: `Failure ${number}`
  }));
}

async function contextFor(t, mode = 'granted') {
  const context = await browser.newContext();
  t.after(() => context.close());
  // Notification is the OS seam; real fetch, SSE, storage and Web Locks remain in use.
  await context.addInitScript(mode => {
    window.notices = [];
    window.permissionRequests = 0;
    window.focusCalls = 0;
    window.focus = () => window.focusCalls++;
    const NativeSource = EventSource;
    window.EventSource = class extends NativeSource {
      constructor(...args) { super(...args); window.notificationStream = this; }
    };
    let permission = localStorage.getItem('test:notification-permission') ?? (mode === 'prompt' ? 'default' : mode);
    window.Notification = class {
      static get permission() { return permission; }
      static async requestPermission() {
        window.permissionRequests++;
        localStorage.setItem('test:notification-permission', 'granted');
        return permission = 'granted';
      }
      constructor(title, options) { this.title = title; this.options = options; window.notices.push(this); }
      close() { this.closed = true; }
    };
  }, mode);
  return context;
}

async function count(page, value) {
  await page.waitForFunction(value => document.title === `${value} need you · thirdshift Console`, value);
}

test('notifications ask on click, survive reload and open the Inbox for a rise', async t => {
  const context = await contextFor(t, 'prompt');
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const response = await page.goto(origin);
  assert.equal(response.headers()['content-security-policy'], "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'");
  assert.equal(await page.locator('script:not([src])').count(), 0);
  await count(page, 1);
  assert.equal(await page.evaluate(() => permissionRequests), 0);
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click({ timeout: 2000 });
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  assert.equal(await page.evaluate(() => permissionRequests), 1);
  assert.equal(await page.evaluate(() => notices.length), 0);
  await page.reload();
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  assert.equal(await page.evaluate(() => notices.length), 0);
  try {
    await item(2);
    await page.waitForFunction(() => notices.length === 1);
    const notice = await page.evaluate(() => ({ title: notices[0].title, body: notices[0].options.body }));
    const host = await page.locator('header h1').textContent();
    assert.equal(notice.title, `2 need you · ${host}`);
    assert.match(notice.body, /acme\/widgets#2.*Problem 2/);
    const focused = await page.evaluate(() => { notices[0].onclick({ preventDefault() {} }); return focusCalls; });
    assert.equal(focused, 1);
    await page.waitForURL(`${origin}/?page=inbox`);
    assert.deepEqual(errors, []);
  } finally { await rm(join(records, '2.command.json'), { force: true }); }
});

test('several tabs notify once per rise, including a rise after a fall; off stays off', async t => {
  const context = await contextFor(t);
  const pages = await Promise.all([context.newPage(), context.newPage(), context.newPage()]);
  await Promise.all(pages.map(page => page.goto(origin)));
  await pages[0].getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  for (const page of pages) await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  const total = async () => (await Promise.all(pages.map(page => page.evaluate(() => notices.length)))).reduce((a, b) => a + b, 0);
  const settle = async value => {
    for (const page of pages) await count(page, value);
    await pages[0].waitForTimeout(1200);
  };
  await settle(1);
  assert.equal(await total(), 0);
  try {
    await item(3);
    await settle(2);
    assert.equal(await total(), 1);
    // A changed title updates both tabs without changing Needs you.
    const path = join(records, '3.command.json');
    const record = JSON.parse(await readFile(path, 'utf8'));
    record.issue.title = 'Changed without a count rise';
    await writeFile(path, JSON.stringify(record));
    for (const page of pages) await page.locator('#inbox .issue-title').getByText(record.issue.title, { exact: true }).waitFor();
    await settle(2);
    assert.equal(await total(), 1);
    await item(3, 'merged');
    await settle(1);
    assert.equal(await total(), 1);
    await item(3);
    await settle(2);
    assert.equal(await total(), 2);
    await pages[1].getByRole('button', { name: 'Turn notifications off', exact: true }).click();
    for (const page of pages) await page.getByRole('button', { name: 'Turn notifications on', exact: true }).waitFor();
    await pages[0].reload();
    await pages[0].getByRole('button', { name: 'Turn notifications on', exact: true }).waitFor();
    const offTotal = await total();
    await item(4);
    await settle(3);
    assert.equal(await total(), offTotal);
  } finally {
    await rm(join(records, '3.command.json'), { force: true });
    await rm(join(records, '4.command.json'), { force: true });
  }
});

for (const mode of ['denied', 'missing', 'insecure', 'storage', 'locks']) {
  test(`notifications explain ${mode} support and leave the Console usable`, async t => {
    const context = await contextFor(t, mode === 'denied' ? 'denied' : 'granted');
    await context.addInitScript(mode => {
      if (mode === 'missing') delete window.Notification;
      if (mode === 'insecure') Object.defineProperty(window, 'isSecureContext', { value: false });
      if (mode === 'storage') Object.defineProperty(window, 'localStorage', { get() { throw new Error('blocked'); } });
      if (mode === 'locks') Object.defineProperty(navigator, 'locks', { value: undefined });
    }, mode);
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(origin);
    const control = page.getByRole('button', { name: 'Turn notifications on', exact: true });
    await control.waitFor({ timeout: 2000 });
    assert.equal(await control.isDisabled(), true);
    const reason = { denied: /blocked/, missing: /no Notification API/, insecure: /HTTPS/, storage: /storage/, locks: /across tabs/ }[mode];
    assert.match(await page.locator('#notifications-status').textContent(), reason);
    await count(page, 1);
    assert.deepEqual(errors, []);
  });
}

test('a non-Inbox tab notifies for the newest arriving item, even if its Command started earlier', async t => {
  const context = await contextFor(t);
  const page = await context.newPage();
  await page.goto(`${origin}/?page=history`);
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  await page.waitForTimeout(1200);
  // Gate fetches so both arrivals are observed together, independently of the
  // server observer's one-second cadence.
  let release;
  const ready = new Promise(resolve => { release = resolve; });
  await page.route(`${origin}/`, async route => { await ready; await route.continue(); });
  try {
    await item(5);
    await item(6);
    const path = join(records, '6.command.json');
    const record = JSON.parse(await readFile(path, 'utf8'));
    record.started_at = new Date(Date.now() - 3600000).toISOString();
    record.ended_at = new Date(Date.now() + 1000).toISOString();
    await writeFile(path, JSON.stringify(record));
    release();
    await page.waitForFunction(() => notices.length > 0);
    assert.match(await page.evaluate(() => notices.at(-1).options.body), /acme\/widgets#6.*Problem 6/);
    assert.equal(new URL(page.url()).searchParams.get('page'), 'history');
  } finally {
    release();
    await rm(join(records, '5.command.json'), { force: true });
    await rm(join(records, '6.command.json'), { force: true });
  }
});

test('notifications recover after the Inbox is temporarily unavailable', async t => {
  const context = await contextFor(t);
  const page = await context.newPage();
  await page.goto(`${origin}/?page=history`);
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  await page.waitForTimeout(1200);
  await page.route(`${origin}/`, route => route.fulfill({ status: 503, body: 'temporarily unavailable' }));
  try {
    const failed = page.waitForResponse(response => response.status() === 503);
    await item(7);
    await failed;
    await page.waitForTimeout(100);
    await page.unroute(`${origin}/`);
    await item(8);
    await page.waitForFunction(() => notices.length === 1, null, { timeout: 12000 });
    await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  } finally {
    await rm(join(records, '7.command.json'), { force: true });
    await rm(join(records, '8.command.json'), { force: true });
  }
});

test('opening another tab during a rise does not swallow the existing tab notification', async t => {
  const context = await contextFor(t);
  const older = await context.newPage();
  await older.goto(`${origin}/?page=history`);
  await older.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  await older.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  await older.waitForTimeout(1200);
  // Delay this tab's next notice until the new tab has established its baseline.
  await older.evaluate(() => notificationStream.close());
  try {
    await item(9);
    const newer = await context.newPage();
    await newer.goto(`${origin}/?page=history`);
    await newer.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
    await newer.waitForTimeout(1200);
    assert.equal(await newer.evaluate(() => notices.length), 0);
    await older.evaluate(() => notificationStream.dispatchEvent(new Event('commands')));
    await older.waitForFunction(() => notices.length === 1, null, { timeout: 5000 });
    assert.equal(await newer.evaluate(() => notices.length), 0);
  } finally {
    await rm(join(records, '9.command.json'), { force: true });
  }
});

test('a delayed tab does not notify for a fall already observed by another tab', async t => {
  const context = await contextFor(t);
  const active = await context.newPage();
  await active.goto(`${origin}/?page=history`);
  await active.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  const delayed = await context.newPage();
  await delayed.goto(`${origin}/?page=history`);
  await delayed.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  await delayed.waitForTimeout(1200);
  await delayed.evaluate(() => notificationStream.close());
  try {
    await item(10);
    await item(11);
    await active.waitForFunction(() => notices.at(-1)?.title.startsWith('3 need you'));
    await rm(join(records, '11.command.json'));
    // Wait for the active tab's successful fall sample to complete.
    await active.waitForResponse(response => response.url() === `${origin}/` && response.ok());
    await active.waitForTimeout(100);
    const fetched = delayed.waitForResponse(response => response.url() === `${origin}/` && response.ok());
    await delayed.evaluate(() => notificationStream.dispatchEvent(new Event('commands')));
    await fetched;
    await delayed.waitForTimeout(100);
    assert.equal(await delayed.evaluate(() => notices.length), 0);
  } finally {
    await rm(join(records, '10.command.json'), { force: true });
    await rm(join(records, '11.command.json'), { force: true });
  }
});

test('notifications resume when a page returns from the back-forward cache', async t => {
  const context = await contextFor(t);
  const page = await context.newPage();
  await page.goto(`${origin}/?page=history`);
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  await page.waitForTimeout(1200);
  await page.evaluate(() => {
    dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }));
    dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true }));
  });
  await page.waitForTimeout(1200);
  try {
    await item(12);
    await page.waitForFunction(() => notices.length === 1, null, { timeout: 5000 });
  } finally { await rm(join(records, '12.command.json'), { force: true }); }
});

test('Account and Console log tabs keep notifications available', async t => {
  const context = await contextFor(t);
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  try {
    for (const [path, number] of [['/account', 13], ['/console-log', 14]]) {
      await page.goto(`${origin}${path}`);
      await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor({ timeout: 2000 });
      await page.waitForTimeout(1200);
      await item(number);
      await page.waitForFunction(() => notices.length === 1);
    }
  } finally {
    await rm(join(records, '13.command.json'), { force: true });
    await rm(join(records, '14.command.json'), { force: true });
  }
});

test('the Inbox keeps updating when an in-flight notification request is turned off', async t => {
  const context = await contextFor(t);
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('button', { name: 'Turn notifications on', exact: true }).click();
  await page.getByRole('button', { name: 'Turn notifications off', exact: true }).waitFor();
  await page.waitForTimeout(1200);
  let release, held;
  const gate = new Promise(resolve => { release = resolve; });
  const blocked = new Promise(resolve => { held = resolve; });
  const delayed = await context.newPage();
  // History only fetches the Inbox for notifications, so this holds that
  // observation's cross-tab lock without delaying the visible Inbox's fetch.
  await delayed.route(`${origin}/`, async route => {
    held();
    await gate;
    await route.continue();
  });
  try {
    await delayed.goto(`${origin}/?page=history`);
    let deadline;
    try {
      await Promise.race([blocked, new Promise((_, reject) => {
        deadline = setTimeout(() => reject(new Error('Notification request did not start')), 5000);
      })]);
    } finally { clearTimeout(deadline); }
    await item(15);
    await count(page, 2);
    await page.getByRole('button', { name: 'Turn notifications off', exact: true }).click();
    await page.getByRole('button', { name: 'Turn notifications on', exact: true }).waitFor();
    await item(16);
    await page.waitForFunction(() => document.title === '3 need you · thirdshift Console', null, { timeout: 6000 });
    assert.equal(await page.evaluate(() => notices.length), 0);
  } finally {
    release();
    await rm(join(records, '15.command.json'), { force: true });
    await rm(join(records, '16.command.json'), { force: true });
  }
});
