const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, readFile, writeFile, appendFile, utimes, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("./playwright.cjs");
const { record: traceRecord } = require("./session-trace.cjs");
const { replaceRecord } = require("./records.cjs");

let home, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), "thirdshift-console-"));
  const configDir = join(home, ".thirdshift");
  const records = join(configDir, "logs/acme/widgets/commands/issue");
  await mkdir(records, { recursive: true });
  await writeFile(join(configDir, "config.toml"), '[console]\nport = 0\nother_consoles = ["https://other.example.test"]\n');
  for (let number = 1; number <= 10; number++) {
    const now = new Date().toISOString();
    await writeFile(join(records, `${number}-fixture.command.json`), JSON.stringify({
      version: 1, kind: "Spec run", repository: "acme/widgets", hostname: "fixture",
      issue: { number, url: `https://github.com/acme/widgets/issues/${number}`, title: "A long title that should wrap on a phone, even with <script>alert('unsafe')</script>" },
      process_id: 0, process_start: "gone", harness: "codex", model: "gpt-6", effort: "high",
      started_at: now, ended_at: now, stages: [], session_log: null,
      outcome: "failed", cause: `distinct fixture failure ${number}`, issue_branch: null, pull_request: null,
      tickets: [{ issue: { number: 98, url: "https://github.com/acme/widgets/issues/98", title: "Build it" }, state: "waiting", blockers: [{ number: 97, url: "https://github.com/acme/widgets/issues/97", title: "First" }] }]
    }));
    await writeFile(join(records, `${number}-fixture.log`), `Full Command output ${number}\n`);
  }
  const security = join(configDir, 'logs/acme/widgets/commands/secure');
  await mkdir(security, { recursive: true });
  const session = join(security, 'secure-fixture-security-audit.logs.jsonl');
  await writeFile(session, traceRecord('Security Session output'));
  await writeFile(join(security, 'secure-fixture.log'), 'Security Command output\n');
  await writeFile(join(security, 'secure-fixture.command.json'), JSON.stringify({
    version: 1, kind: 'Security run', name: 'secure-fixture', repository: 'acme/widgets',
    hostname: 'fixture', process_id: 0, process_start: 'gone', harness: 'codex',
    started_at: '2026-01-01T12:00:00Z', ended_at: '2026-01-01T13:00:00Z',
    stages: [], session_log: session, outcome: 'findings recorded',
    security_findings: [{ title: 'Bound <input> before storage', severity: 'high', url: 'https://github.com/acme/widgets/security/advisories/GHSA-test' }]
  }));
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift"), ["console", "--no-sign-in"], {
    env: { ...process.env, HOME: home }, stdio: ["ignore", "pipe", "inherit"]
  });
  exited = once(server, "exit");
  const lines = createInterface({ input: server.stdout });
  try {
    [origin] = await once(lines, "line", { signal: AbortSignal.timeout(10000) });
    assert.match(origin, /^http:\/\/127\.0\.0\.1:\d+$/);
  } finally { lines.close(); }
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server && server.exitCode === null) server.kill();
  await exited;
  if (home) await rm(home, { recursive: true, force: true });
});

async function pageFor(t, options) {
  const context = await browser.newContext(options);
  t.after(() => context.close());
  const page = await context.newPage();
  return page;
}

test('Paused rows and Command details copy the saved resume command', async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  const fixture = JSON.parse(original);
  const resume = "thirdshift experiment 'feature with spaces' --yes";
  fixture.outcome = 'paused';
  fixture.kind = 'Experiment';
  fixture.resume_command = resume;
  fixture.cause = 'Command paused at a plan-limit warning: Claude seven-day window at 86%, resets Fri Oct 30 22:00 UTC';
  fixture.limit = { harness: 'claude', window: 'seven_day', utilization: 0.86, resets_at: '2026-10-30T22:00:00Z' };
  const page = await pageFor(t);
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  try {
    await replaceRecord(path, JSON.stringify(fixture));
    await page.goto(`${origin}/?section=needs-you`);
    const row = page.locator('#inbox li.command[data-state="Paused"]');
    await row.getByText(/resets Fri Oct 30 22:00/).waitFor();
    await row.getByRole('button', { name: 'Copy resume command', exact: true }).click();
    await row.getByText('CLI copied.', { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), resume);
    fixture.cause += ' Updated after copying.';
    await replaceRecord(path, JSON.stringify(fixture));
    await row.getByText(fixture.cause, { exact: true }).waitFor();
    await row.getByText('CLI copied.', { exact: true }).waitFor();
    await row.locator('.state').click();
    const details = page.locator('#peek-content');
    await details.getByRole('button', { name: 'Copy resume command', exact: true }).click();
    await details.getByText('CLI copied.', { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), resume);
    assert.equal(await details.getByText(/^Failed:/).count(), 0);
  } finally {
    await replaceRecord(path, original);
  }
});

test('Inbox keeps issue numbers visible for long repository and Process subjects', async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  const fixture = JSON.parse(original);
  fixture.repository = 'Educational-Travel-Adventures/some-long-repository-name';
  fixture.issue = { number: 123, url: `https://github.com/${fixture.repository}/issues/123`, title: 'Keep the issue number visible' };
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  try {
    for (const process of [null, 'nightly']) {
      await t.test(process ? 'Process issue' : 'ordinary issue', async () => {
        fixture.kind = process ? 'Process' : 'Spec run';
        fixture.process_name = process;
        await replaceRecord(path, JSON.stringify(fixture));
        await page.goto(`${origin}/?section=needs-you`);
        await page.evaluate(() => document.fonts.ready);
        const subject = page.locator('#inbox .row-head > .subject').filter({ has: page.locator(`a[href="${fixture.issue.url}"]`) });
        const visibility = await subject.evaluate(node => {
          const texts = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
          let text;
          while ((text = texts.nextNode()) && !text.textContent.endsWith('#123')) {}
          const range = document.createRange();
          range.setStart(text, text.length - 4);
          range.setEnd(text, text.length);
          const number = range.getBoundingClientRect();
          const subject = node.getBoundingClientRect();
          return { left: number.left, right: number.right, subjectLeft: subject.left, subjectRight: subject.right };
        });
        assert.ok(visibility.left >= visibility.subjectLeft && visibility.right <= visibility.subjectRight + 1,
          `#123 must be visible inside its subject: ${JSON.stringify(visibility)}`);
        const full = `${process ? 'Process nightly · ' : ''}${fixture.repository}#123`;
        assert.equal(await subject.textContent(), full);
        assert.equal(await subject.getAttribute('title'), full);
        assert.equal(await subject.locator('a').getAttribute('href'), fixture.issue.url);
        // Insert and measure in one browser task so a live refresh cannot
        // remove the fixture tag before its layout is checked.
        const tagged = await subject.evaluate(node => {
          const tag = document.createElement('span');
          tag.className = 'account-tag';
          tag.textContent = 'conductor';
          node.prepend(tag, ' ');
          const subject = node.getBoundingClientRect();
          const account = node.querySelector('.account-tag').getBoundingClientRect();
          const issue = node.querySelector('.subject-issue').getBoundingClientRect();
          return { left: account.left, right: issue.right, subjectLeft: subject.left, subjectRight: subject.right,
            accountTop: account.top, accountBottom: account.bottom, issueTop: issue.top, issueBottom: issue.bottom };
        });
        assert.ok(tagged.left >= tagged.subjectLeft && tagged.right <= tagged.subjectRight + 1,
          `account and #123 must fit inside the subject: ${JSON.stringify(tagged)}`);
        assert.ok(tagged.accountTop < tagged.issueBottom && tagged.issueTop < tagged.accountBottom,
          `account and #123 must share one line: ${JSON.stringify(tagged)}`);
      });
    }
    fixture.issue = null;
    fixture.name = 'fixture';
    for (const process of [null, 'nightly']) {
      await t.test(process ? 'Process without an issue' : 'Command without an issue', async () => {
        fixture.kind = process ? 'Process' : 'Spec run';
        fixture.process_name = process;
        await replaceRecord(path, JSON.stringify(fixture));
        await page.goto(`${origin}/?section=needs-you`);
        const full = `${process ? 'Process nightly' : 'fixture'} on ${fixture.repository}`;
        const subject = page.locator('#inbox .row-head > .subject', { hasText: full });
        assert.equal(await subject.textContent(), full);
        assert.equal(await subject.getAttribute('title'), full);
        assert.equal(await subject.locator('a').count(), 0);
        assert.equal(await subject.evaluate(node => getComputedStyle(node).textOverflow), 'ellipsis');
      });
    }
  } finally {
    await replaceRecord(path, original);
  }
});

test('Command details times out a held request and can be opened again', async t => {
  const page = await pageFor(t);
  await page.clock.install();
  let release;
  const held = new Promise(resolve => { release = resolve; });
  await page.route('**/peek?*', async route => {
    await held;
    await route.continue();
  });
  try {
    await page.goto(`${origin}/?page=security`);
    const requested = page.waitForRequest('**/peek?*');
    await page.locator('.command .state').first().click();
    await requested;
    await page.clock.fastForward(15000);
    await page.getByText('Details unavailable. Reload to try again.', { exact: true })
      .waitFor({ timeout: 2000 });
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await page.locator('[data-close-peek]').click();
  await page.locator('.command .state').first().click();
  await page.locator('#peek dt').first().waitFor();
});

// Overview combines the current groups; folders can filter to one section.
const folder = (page, group) => page.locator(`#inbox .folders a[data-group="${group}"]`);
const shownGroup = page => page.locator('#inbox > .group:not([hidden])');
const shownGroups = page => shownGroup(page).evaluateAll(groups => groups.map(group => group.dataset.group));
const overviewGroups = ['needs you', 'ready for review', 'landing', 'working'];
async function showDetails(page, row) {
  const group = await row.evaluate(node => node.closest('.group').dataset.group);
  if (await folder(page, group).getAttribute('aria-current') !== 'page') await folder(page, group).click();
  await row.locator(':scope > .row-head > .state').click();
}

test('Command details shows recent Command output and follows new output on desktop and phone', async t => {
  const log = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.log');
  const original = await readFile(log, 'utf8');
  try {
    for (const width of [1280, 390]) {
      await writeFile(log, 'Old output\n' + 'Earlier progress\n'.repeat(1000)
        + 'Checking CI\nWaiting for checks\nStill waiting\nChecks complete\nLatest <script>unsafe()</script>\n');
      const page = await pageFor(t, { viewport: { width, height: 844 } });
      await page.goto(origin);
      await showDetails(page, page.locator('#inbox .command').filter({
        has: page.locator(':scope > .row-head > .subject', { hasText: /^acme\/widgets#1$/ })
      }));
      const preview = page.getByRole('region', { name: 'Recent Command output', exact: true });
      await preview.waitFor();
      assert.match(await preview.textContent(), /Latest <script>unsafe\(\)<\/script>/);
      assert.equal(await preview.locator('pre').textContent(),
        'Checking CI\nWaiting for checks\nStill waiting\nChecks complete\nLatest <script>unsafe()</script>');
      assert.equal(await preview.locator('script').count(), 0);
      assert.equal(await preview.evaluate(el => el.scrollWidth <= el.clientWidth), true);
      await appendFile(log, 'New progress ' + 'x'.repeat(500) + '\n');
      await preview.getByText('New progress ', { exact: false }).waitFor();
      assert.equal(await preview.locator('pre').evaluate(el => el.textContent.split('\n').length), 5);
      assert.equal(await preview.evaluate(el => el.scrollWidth <= el.clientWidth), true);
      assert.equal(await preview.locator('pre').evaluate(el =>
        el.scrollHeight - el.scrollTop <= el.clientHeight + 1), true);
      await page.getByRole('link', { name: 'Full Command log', exact: true }).click();
      await page.locator('#log-content').getByText(/Old output/).waitFor();
      await page.close();
    }
  } finally {
    await writeFile(log, original);
  }
});

test('Command details opens with the Inbox subject and full title above its actions', async t => {
  for (const width of [1280, 390]) {
    const page = await pageFor(t, { viewport: { width, height: 844 } });
    await page.goto(origin);
    const row = page.locator('#inbox > [data-group="needs you"] .command').first();
    const subject = await row.locator(':scope > .row-head > .subject').textContent();
    const title = await row.locator(':scope > .row-head > .issue-title').textContent();
    await showDetails(page, row);
    const pane = page.getByRole('region', { name: 'Command details', exact: true });
    const heading = pane.getByRole('heading', { level: 2 });
    await heading.getByText(title, { exact: true }).waitFor();
    assert.equal(await heading.getByRole('link').textContent(), subject);
    assert.equal(await heading.getByRole('link').getAttribute('href'),
      await row.locator(':scope > .row-head > .subject a').getAttribute('href'));
    assert.equal(await heading.textContent(), `${subject} ${title}`);
    assert.equal(await heading.locator('script').count(), 0);
    assert.equal(await pane.getByRole('link', { name: /^Issue #/ }).count(), 0);
    assert.equal(await heading.evaluate(el => Boolean(el.compareDocumentPosition(
      el.parentElement.querySelector('form[data-dismiss]')) & Node.DOCUMENT_POSITION_FOLLOWING)), true);
    // A live refresh replaces the pane, so look the title up and measure it in one step.
    const wrapped = await page.evaluate(() => {
      const el = document.querySelector('#peek-content > section[aria-label="Command details"] h2 span');
      return {
        height: el.getBoundingClientRect().height,
        lineHeight: parseFloat(getComputedStyle(el).lineHeight),
        fits: el.scrollWidth <= el.clientWidth,
        whiteSpace: getComputedStyle(el).whiteSpace,
      };
    });
    assert.ok(wrapped.height > wrapped.lineHeight, JSON.stringify({ width, wrapped }));
    assert.equal(wrapped.fits, true);
    assert.equal(wrapped.whiteSpace, 'normal');
    await page.close();
  }
});

test('Command details shows the full failure reason below its heading on desktop and phone', async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  const fixture = JSON.parse(original);
  const cause = 'codex exited 1: Selected model is at capacity. Please try a different model. See https://example.test/status <script>unsafe</script>';
  fixture.cause = cause;
  try {
    await replaceRecord(path, JSON.stringify(fixture));
    for (const width of [1280, 390]) {
      const page = await pageFor(t, { viewport: { width, height: 844 } });
      await page.goto(`${origin}/?section=needs-you`);
      const row = page.locator('#inbox .command').filter({
        has: page.locator('.subject a[href="https://github.com/acme/widgets/issues/1"]'),
      });
      await showDetails(page, row);
      const pane = page.getByRole('region', { name: 'Command details', exact: true });
      const reason = pane.locator('h2 + .peek-failure');
      await reason.waitFor();
      assert.equal(await reason.textContent(), `Failed: ${cause}`);
      assert.equal(await reason.getByRole('link').getAttribute('href'), 'https://example.test/status');
      assert.equal(await reason.locator('script, details').count(), 0);
      assert.equal(await reason.evaluate(el => Boolean(el.compareDocumentPosition(
        el.parentElement.querySelector('form[data-dismiss]')) & Node.DOCUMENT_POSITION_FOLLOWING)), true);
      const fits = await reason.evaluate(el => el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight);
      assert.equal(fits, true, `full failure reason fits at width ${width}`);
      await page.close();
    }
  } finally { await replaceRecord(path, original); }
});

test('inline Dismiss leaves Needs you without opening details and offers Undo on a phone', async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  await page.goto(origin);
  const waiting = '#inbox > [data-group="needs you"] .command';
  const row = page.locator(waiting).first();
  const command = await row.getAttribute('data-command');
  const subject = await row.locator(':scope > .row-head > .subject').textContent();
  const count = await page.locator(waiting).count();
  await row.getByRole('button', { name: 'Dismiss', exact: true }).click();
  await page.waitForFunction(({ waiting, count }) => document.querySelectorAll(waiting).length === count - 1, { waiting, count });
  assert.equal(await page.locator('#peek').isVisible(), false);
  const notice = page.locator('#dismissal-notices > [role="status"]').filter({ hasText: `Dismissed ${subject}` });
  await notice.waitFor();
  assert.equal(await notice.locator('[name="command"]').inputValue(), command);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await notice.getByRole('button', { name: 'Undo', exact: true }).click();
  await page.waitForFunction(({ waiting, count }) => document.querySelectorAll(waiting).length === count, { waiting, count });
  await notice.waitFor({ state: 'detached' });
  assert.equal(await page.locator('#peek').isVisible(), false);
});

test('j then d dismisses and selects the next row, respecting typing, dialogs and modifiers', async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  const rows = page.locator('#inbox > [data-group="needs you"] .command');
  const count = await rows.count();
  const next = await rows.nth(1).getAttribute('data-command');
  await page.keyboard.press('j');
  const command = await page.locator('.command[aria-current="true"]').getAttribute('data-command');
  for (const key of ['Control+d', 'Meta+d', 'Alt+d', 'Shift+d']) await page.keyboard.press(key);
  await page.keyboard.press('?');
  await page.locator('#keys[open]').waitFor();
  assert.equal(await page.locator('#keys dt').getByText('d', { exact: true }).count(), 1);
  await page.keyboard.press('d');
  await page.keyboard.press('Escape');
  await page.keyboard.press('Shift+T');
  await page.locator('#themes[open]').waitFor();
  await page.getByRole('button', { name: 'Make a Theme from this' }).click();
  await page.waitForFunction(() => document.querySelector('#theme-document').value.includes('"tokens"'));
  await page.locator('#theme-document').fill('typing ');
  await page.locator('#theme-document').press('d');
  assert.equal(await page.locator('#theme-document').inputValue(), 'typing d');
  await page.getByRole('button', { name: 'Close Themes' }).click();
  assert.equal(await rows.count(), count);
  await rows.first().focus();
  await page.keyboard.press('d');
  await page.waitForFunction(next => document.querySelector('.command[aria-current="true"]')?.dataset.command === next, next);
  await page.waitForFunction(count => document.querySelectorAll('#inbox > [data-group="needs you"] .command').length === count - 1, count);
  assert.equal(await page.locator('#peek').isVisible(), false);
  await folder(page, 'done').click();
  const doneRow = page.locator(`#inbox > [data-group="done"] .command[data-command="${command}"]`);
  await doneRow.focus();
  assert.equal(await doneRow.getByRole('button', { name: 'Dismiss', exact: true }).count(), 0);
  await page.keyboard.press('d');
  assert.equal(await page.locator('#dismissal-notices form').count(), 1);
  await page.locator('#dismissal-notices').getByRole('button', { name: 'Undo', exact: true }).click();
  await folder(page, 'needs you').click();
  await page.waitForFunction(count => document.querySelectorAll('#inbox > [data-group="needs you"] .command').length === count, count);
});

test('inline Dismiss and notice Undo keep pending and error feedback through live updates and retry', async t => {
  const page = await pageFor(t);
  await page.clock.install();
  await page.goto(origin);
  const row = page.locator('#inbox > [data-group="needs you"] .command').first();
  await showDetails(page, row);
  const form = row.locator(':scope > .row-head form[data-dismiss]');
  const recordPath = await form.locator('[name="command"]').inputValue();
  const original = await readFile(recordPath, 'utf8');
  await page.route(`${origin}/dismiss`, route => route.fulfill({ status: 409, body: 'Please retry Dismiss' }));
  try {
    await form.getByRole('button', { name: 'Dismiss', exact: true }).click();
    await form.getByText('Could not save the dismissal: Please retry Dismiss', { exact: true }).waitFor();
    assert.equal(await form.locator('button').isEnabled(), true);
    assert.equal(await page.locator('#dismissal-notices form').count(), 0);
    await page.unrouteAll({ behavior: 'wait' });
    await form.getByRole('button', { name: 'Dismiss', exact: true }).click();
    const notice = page.locator('#dismissal-notices > [role="status"]');
    await notice.waitFor();
    await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).waitFor();
    await page.route(`${origin}/dismiss/undo`, route => route.fulfill({ status: 409, body: 'Please retry Undo' }));
    await notice.getByRole('button', { name: 'Undo', exact: true }).click();
    await notice.getByText('Could not save the dismissal: Please retry Undo', { exact: true }).waitFor();
    await page.mouse.move(0, 0);
    await page.clock.fastForward(30000);
    assert.equal(await notice.isVisible(), true);
    assert.equal(await notice.locator('button').isEnabled(), true);
    await page.unrouteAll({ behavior: 'wait' });
    let release;
    const held = new Promise(resolve => { release = resolve; });
    let requests = 0;
    await page.route(`${origin}/dismiss/undo`, async route => {
      requests++;
      await held;
      await route.continue();
    });
    try {
      await notice.getByRole('button', { name: 'Undo', exact: true }).click();
      await notice.getByText('Taking back dismissal…', { exact: true }).waitFor();
      assert.equal(await notice.locator('button').isDisabled(), true);
      const updated = JSON.parse(original);
      updated.cause = 'Live refresh while notice Undo is pending';
      await replaceRecord(recordPath, JSON.stringify(updated));
      await page.locator('#peek-content').getByText(updated.cause, { exact: true }).waitFor();
      assert.equal(await notice.locator('button').isDisabled(), true);
      await notice.locator('form').evaluate(form => form.requestSubmit());
      release();
      await notice.waitFor({ state: 'detached' });
      await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
      assert.equal(requests, 1);
    } finally { release(); }
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await replaceRecord(recordPath, original);
  }
});

test('the Inbox opens with Details closed, and Dismiss works by keyboard from Details beside a clicked row', async t => {
  const page = await pageFor(t);
  const observer = await pageFor(t);
  for (const current of [page, observer]) await current.goto(`${origin}/?page=inbox`);
  const waiting = '#inbox > [data-group="needs you"] .command';
  const row = page.locator(waiting).first();
  const count = await page.locator(waiting).count();
  await page.evaluate(() => { window.sameDocument = true; });
  assert.equal(await page.locator('#peek').isVisible(), false);
  assert.equal(await page.locator('.is-selected').count(), 0);
  await showDetails(page, row);
  assert.equal(await row.getAttribute('aria-current'), 'true');
  const dismiss = page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true });
  await dismiss.waitFor({ timeout: 5000 });
  assert.equal(await page.locator('#peek-content form[data-dismiss] [name="command"]').inputValue(), await row.getAttribute('data-command'));
  await dismiss.focus();
  await page.keyboard.press('Enter');
  for (const current of [page, observer]) {
    await current.waitForFunction(({ waiting, count }) => document.querySelectorAll(waiting).length === count - 1, { waiting, count });
  }
  assert.equal(await page.evaluate(() => window.sameDocument), true);
  await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).click();
  await page.waitForFunction(({ waiting, count }) => document.querySelectorAll(waiting).length === count, { waiting, count });
});

for (const [outcome, group] of [['failed', 'needs you'], ['ready for review', 'ready for review']]) {
  for (const javaScriptEnabled of [false, true]) {
    test(`Inbox row Dismiss works in ${group} ${javaScriptEnabled ? 'with' : 'without'} JavaScript on a phone`, async t => {
      const directory = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
      const recordPath = join(directory, '223-fixture.command.json');
      const fixture = JSON.parse(await readFile(join(directory, '1-fixture.command.json'), 'utf8'));
      fixture.issue = { number: 223, url: 'https://github.com/acme/widgets/issues/223', title: 'Dismiss from the Inbox' };
      fixture.outcome = outcome;
      fixture.cause = null;
      fixture.pull_request = outcome === 'ready for review' ? 'https://github.com/acme/widgets/pull/224' : null;
      await writeFile(recordPath, JSON.stringify(fixture));
      t.after(async () => {
        await rm(recordPath, { force: true });
        await rm(join(directory, '223-fixture.command.dismiss.json'), { force: true });
      });
      const page = await pageFor(t, { javaScriptEnabled, viewport: { width: 390, height: 844 } });
      if (javaScriptEnabled) await page.clock.install();
      await page.goto(`${origin}/?page=inbox`);
      const section = page.locator(`#inbox > [data-group="${group}"]`);
      if (group === 'ready for review') await folder(page, group).click();
      const row = section.locator('.command').filter({ has: page.locator('.subject a[href$="/issues/223"]') });
      await row.getByRole('button', { name: 'Dismiss', exact: true }).click();
      if (javaScriptEnabled) {
        await row.waitFor({ state: 'detached' });
        const notice = page.locator('#dismissal-notices > [role="status"]');
        await notice.waitFor();
        await notice.getByRole('button', { name: 'Undo', exact: true }).click();
        await row.waitFor();
        await row.getByRole('button', { name: 'Dismiss', exact: true }).click();
        await row.waitFor({ state: 'detached' });
        await page.waitForFunction(() => !document.querySelector('#dismissal-notices button').disabled);
        await page.mouse.move(0, 0);
        await page.clock.fastForward(10001);
        await notice.waitFor({ state: 'detached' });
      } else await page.waitForURL(`${origin}/`);
      assert.equal(await section.locator('.subject a[href$="/issues/223"]').count(), 0);
      assert.equal(await page.locator('#peek').isVisible(), false);
      await folder(page, 'done').click();
      const dismissed = page.locator('#inbox > [data-group="done"] .command').filter({ has: page.locator('.subject a[href$="/issues/223"]') });
      assert.equal(await dismissed.getByRole('button', { name: 'Dismiss', exact: true }).count(), 0);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      if (javaScriptEnabled) {
        await showDetails(page, dismissed);
        await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).waitFor();
      } else {
        await dismissed.locator('.peek-link').click();
        await page.getByRole('button', { name: 'Undo', exact: true }).waitFor();
      }
    });
  }
}

test('Dismiss keeps Undo in Command details and updates another open Inbox', async t => {
  const page = await pageFor(t);
  const observer = await pageFor(t);
  for (const current of [page, observer]) await current.goto(origin);
  const waiting = '[data-group="needs you"] .command';
  const count = await page.locator(waiting).count();
  await showDetails(page, page.locator(waiting).first());
  await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
  await page.evaluate(() => { window.sameDocument = true; });
  await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).click();
  await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).waitFor({ timeout: 3000 });
  assert.equal(await page.evaluate(() => window.sameDocument), true);
  await observer.waitForFunction(({ waiting, count }) => document.querySelectorAll(waiting).length === count - 1, { waiting, count });
  assert.match(await page.locator('#peek-content').textContent(), /thirdshift dismiss .* --undo/);
  await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).click();
  await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
  await observer.waitForFunction(({ waiting, count }) => document.querySelectorAll(waiting).length === count, { waiting, count });
  assert.equal(await page.evaluate(() => window.sameDocument), true);
});

test('Dismiss and Undo show pending feedback through live refresh and prevent duplicate submissions', async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  await showDetails(page, page.locator('#inbox > [data-group="needs you"] .command').first());
  const form = page.locator('#peek-content form[data-dismiss]');
  await form.waitFor();
  const recordPath = await form.locator('[name="command"]').inputValue();
  const original = await readFile(recordPath, 'utf8');
  try {
    for (const [path, label, pending, next] of [
      ['/dismiss', 'Dismiss', 'Dismissing…', 'Undo'],
      ['/dismiss/undo', 'Undo', 'Taking back dismissal…', 'Dismiss']
    ]) {
      let release;
      const held = new Promise(resolve => { release = resolve; });
      let requests = 0;
      await page.route(`${origin}${path}`, async route => {
        requests++;
        await held;
        await route.continue();
      });
      try {
        await page.locator('#peek-content').getByRole('button', { name: label, exact: true }).click();
        await page.waitForFunction(pending => document.querySelector('#peek-content form[data-dismiss] [role="status"]').textContent === pending, pending, { timeout: 2000 });
        assert.equal(await form.locator(':scope > button').isDisabled(), true);
        const updated = JSON.parse(original);
        updated.cause = `Live refresh while ${label} is pending`;
        await replaceRecord(recordPath, JSON.stringify(updated));
        await page.locator('#peek-content').getByText(updated.cause, { exact: true }).waitFor();
        assert.equal(await form.locator(':scope > [role="status"]').textContent(), pending);
        assert.equal(await form.locator(':scope > button').isDisabled(), true);
        // A second submit event must be ignored even if a browser dispatches it directly.
        await form.evaluate(form => form.requestSubmit());
        release();
        await page.locator('#peek-content').getByRole('button', { name: next, exact: true }).waitFor({ timeout: 3000 });
        await page.waitForFunction(() => !document.querySelector('#peek-content form[data-dismiss] button').disabled);
        assert.equal(requests, 1);
      } finally {
        release();
        await page.unrouteAll({ behavior: 'wait' });
      }
    }
  } finally { await replaceRecord(recordPath, original); }
});

test('A failed Dismiss shows the error through live refresh and allows retry', async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  await showDetails(page, page.locator('#inbox > [data-group="needs you"] .command').first());
  const form = page.locator('#peek-content form[data-dismiss]');
  await form.waitFor();
  const recordPath = await form.locator('[name="command"]').inputValue();
  const original = await readFile(recordPath, 'utf8');
  await page.route(`${origin}/dismiss`, route => route.fulfill({ status: 409, body: 'Please try again' }));
  try {
    await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).click();
    const error = 'Could not save the dismissal: Please try again';
    await form.locator(':scope > [role="status"]').getByText(error, { exact: true }).waitFor();
    assert.equal(await form.locator(':scope > button').isEnabled(), true);
    const updated = JSON.parse(original);
    updated.cause = 'Live refresh after failed dismissal';
    await replaceRecord(recordPath, JSON.stringify(updated));
    await page.locator('#peek-content').getByText(updated.cause, { exact: true }).waitFor();
    assert.equal(await form.locator(':scope > [role="status"]').textContent(), error);
    await page.unrouteAll({ behavior: 'wait' });
    await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).click();
    await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).waitFor();
    await page.waitForFunction(() => !document.querySelector('#peek-content form[data-dismiss] button').disabled);
    assert.equal(await form.locator(':scope > [role="status"]').textContent(), '');
    await page.locator('#peek-content').getByRole('button', { name: 'Undo', exact: true }).click();
    await page.locator('#peek-content').getByRole('button', { name: 'Dismiss', exact: true }).waitFor();
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await replaceRecord(recordPath, original);
  }
});

test("Shift+T switches all built-in Themes and saves hidden sections across browsers", async t => {
  const page = await pageFor(t);
  const remote = [];
  page.on('request', request => { if (!request.url().startsWith(origin)) remote.push(request.url()); });
  await page.goto(origin);
  await page.keyboard.press('Shift+T');
  await page.locator('#themes[open]').waitFor({ timeout: 2000 });
  assert.equal(await page.locator('#look-status').textContent(), '');
  try {
    for (const family of ['focus', 'plain', 'press', 'signal', 'garden', 'factory']) {
      for (const mode of ['auto', 'light', 'dark']) {
        const name = `${family}-${mode}`;
        await page.selectOption('#theme-name', name);
        const saved = page.waitForResponse(response => response.url() === `${origin}/theme`);
        await page.getByRole('button', { name: 'Apply Theme', exact: true }).click();
        const response = await saved;
        assert.equal(response.status(), 200, await response.text());
        assert.equal((await response.json()).theme, name);
        await page.waitForFunction(name => document.documentElement.dataset.theme === name, name);
        assert.match(await readFile(join(home, '.thirdshift/config.toml'), 'utf8'), new RegExp(`theme = "${name}"`));
        await page.evaluate(() => document.fonts.ready);
        assert.equal(await page.evaluate(() => [...document.fonts].filter(f => f.status === 'error').length), 0);
      }
    }
    await page.getByRole('button', { name: 'Close Themes' }).click();
    await page.getByRole('button', { name: 'Sections', exact: true }).click();
    await page.getByLabel('Show Done', { exact: true }).uncheck();
    await page.getByRole('button', { name: 'Save sections' }).click();
    await page.locator('#inbox .folders a[data-group="done"][hidden]').waitFor({ state: 'attached' });
    const other = await pageFor(t);
    await other.goto(origin);
    await other.locator('#inbox .folders a[data-group="done"][hidden]').waitFor({ state: 'attached' });
    assert.equal(await other.locator('html').getAttribute('data-theme'), 'factory-dark');
    assert.deepEqual(remote, [], 'Themes must load fonts and assets locally');
  } finally { await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n'); }
});

test("every built-in Theme text pair meets AA in both system modes, including solid tags", async t => {
  const page = await pageFor(t);
  const failures = [];
  const { APCAcontrast, sRGBtoY } = await import('apca-w3');
  const luminance = rgb => rgb.map(v => v / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
  const ratio = (a, b) => (Math.max(luminance(a), luminance(b)) + .05) / (Math.min(luminance(a), luminance(b)) + .05);
  await page.goto(origin);
  const backgrounds = { focus: ['rgb(247, 247, 245)', 'rgb(18, 18, 20)'], plain: ['rgb(252, 252, 251)', 'rgb(17, 18, 20)'], press: ['rgb(232, 230, 225)', 'rgb(10, 10, 10)'], signal: ['rgb(250, 250, 250)', 'rgb(17, 19, 21)'], garden: ['rgb(246, 240, 225)', 'rgb(22, 26, 20)'], factory: ['rgb(227, 230, 232)', 'rgb(28, 29, 31)'] };
  for (const family of ['focus', 'plain', 'press', 'signal', 'garden', 'factory']) {
    for (const mode of ['auto', 'light', 'dark']) {
      for (const system of ['light', 'dark']) {
        await page.emulateMedia({ colorScheme: system });
        const theme = `${family}-${mode}`;
        const { pairs, background } = await page.evaluate(theme => {
          document.documentElement.dataset.theme = theme;
          const probe = document.createElement('span');
          document.body.append(probe);
          const color = token => {
            probe.style.color = `var(--${token})`;
            return getComputedStyle(probe).color.match(/[\d.]+/g).slice(0, 3).map(Number);
          };
          const pairs = [];
          for (const background of ['bg', 'panel', 'panel-2', 'sel-bg']) {
            for (const text of ['text', 'head', 'dim', 'link', 'urgent-text', 'look-text', 'working-text', 'ready-text']) {
              pairs.push({ name: `${text} on ${background}`, text: color(text), background: color(background) });
            }
          }
          for (const state of ['urgent', 'look', 'working', 'landing', 'ready', 'done']) {
            pairs.push({ name: `on-${state}`, text: color(`on-${state}`), background: color(state) });
          }
          probe.remove();
          const texts = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
          while (texts.nextNode()) {
            const node = texts.currentNode;
            if (!node.textContent.trim()) continue;
            const range = document.createRange(); range.selectNode(node);
            if (!range.getBoundingClientRect().width || !range.getBoundingClientRect().height) continue;
            const style = getComputedStyle(node.parentElement);
            let parent = node.parentElement;
            while (parent && getComputedStyle(parent).backgroundColor === 'rgba(0, 0, 0, 0)') parent = parent.parentElement;
            if (!parent) continue;
            pairs.push({ name: `rendered ${node.textContent.trim().slice(0, 30)}`, text: style.color.match(/[\d.]+/g).slice(0, 3).map(Number), background: getComputedStyle(parent).backgroundColor.match(/[\d.]+/g).slice(0, 3).map(Number) });
          }
          return { pairs, background: getComputedStyle(document.body).backgroundColor };
        }, theme);
        assert.equal(background, backgrounds[family][(mode === 'auto' ? system : mode) === 'dark' ? 1 : 0], `${theme} must respect the chosen mode on ${system}`);
        for (const pair of pairs) {
          if (ratio(pair.text, pair.background) < 4.5) failures.push(`${theme} (${system}): ${pair.name} = ${ratio(pair.text, pair.background)}`);
          if (family === 'focus' && !pair.name.startsWith('on-')) {
            assert.ok(Math.abs(APCAcontrast(sRGBtoY(pair.text), sRGBtoY(pair.background))) >= 45, `${theme}: ${pair.name} below APCA Lc 45`);
          }
          if (pair.name.startsWith('on-')) {
            const best = Math.max(ratio([0, 0, 0], pair.background), ratio([255, 255, 255], pair.background));
            assert.equal(ratio(pair.text, pair.background), best, `${theme}: ${pair.name} must use the better black or white`);
          }
        }
      }
    }
  }
  assert.deepEqual(failures, []);
  assert.equal(await page.evaluate(() => document.getAnimations().length), 0, 'idle Commands must not animate');
});

test("Inbox folders load only their own resources and fit a 390 px phone", async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  const remote = [], errors = [];
  page.on("request", request => { if (!request.url().startsWith(origin)) remote.push(request.url()); });
  page.on("pageerror", error => errors.push(error.message));
  const response = await page.goto(origin);
  assert.equal(response.status(), 200);
  await page.evaluate(() => document.fonts.ready);
  assert.deepEqual(await shownGroups(page), overviewGroups);
  assert.equal(await page.locator('#peek').isVisible(), false, 'a phone opens Details only on a tap');
  assert.match(await page.locator(".plan").first().textContent(), /Ticket #98 · waiting on #97/);
  assert.equal(await page.locator('.command .state').first().textContent(), "Failed");
  assert.equal(await page.locator(".command script").count(), 0);
  for (const link of await page.locator("#inbox .folders a:not([hidden])").all()) {
    await link.click();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  assert.deepEqual(remote, []);
  assert.deepEqual(errors, []);
  assert.equal(await page.evaluate(() => document.fonts.check('14px "IBM Plex Sans"') && document.fonts.check('12px "IBM Plex Mono"')), true);
});

test("finish information is visible without JavaScript on a narrow phone", async t => {
  const page = await pageFor(t, { javaScriptEnabled: false, viewport: { width: 320, height: 844 } });
  await page.goto(origin);
  const age = page.locator('[data-group="needs you"] .command > .row-head > .age').first();
  assert.equal(await age.isVisible(), true);
  assert.match(await age.innerText(), /^Ended \d{4}-\d\d-\d\d \d\d:\d\d [A-Z][A-Za-z0-9+:-]* · \d+ (sec|min|hr \d+ min) ago$/);
  assert.ok(Number.isFinite(Date.parse(await age.locator('time').getAttribute('datetime'))));
  const box = await age.boundingBox();
  assert.ok(box.x >= 0 && box.x + box.width <= 320, JSON.stringify(box));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  assert.equal(await page.locator('.command .peek-link').first().isVisible(), true);
});

test("existing refresh advances finish age while keeping its recorded instant", async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/71-fixture.command.json');
  const end = new Date(Date.now() - 55000).toISOString();
  const fixture = JSON.parse(await readFile(join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json'), 'utf8'));
  fixture.issue = { number: 71, url: 'https://github.com/acme/widgets/issues/71', title: 'Finish refresh fixture' };
  fixture.started_at = new Date(Date.parse(end) - 140 * 60000).toISOString();
  fixture.ended_at = end;
  fixture.cause = 'finish refresh fixture';
  await replaceRecord(path, JSON.stringify(fixture));
  t.after(() => rm(path, { force: true }));
  const page = await pageFor(t);
  await page.goto(origin);
  const row = page.locator('.command').filter({ has: page.locator('a[href="https://github.com/acme/widgets/issues/71"]') });
  const time = row.locator('time');
  assert.equal(await time.getAttribute('datetime'), end);
  assert.match(await row.locator('.age').textContent(), /\d+ sec ago$/);
  const absolute = await time.textContent();
  // Cross the server's minute boundary, then wake the ordinary filesystem refresh.
  await page.waitForTimeout(Math.max(0, Date.parse(end) + 61000 - Date.now()));
  await appendFile(join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.log'), 'Refresh finish age\n');
  await page.waitForFunction(() => [...document.querySelectorAll('.command')].some(row =>
    row.querySelector('a[href="https://github.com/acme/widgets/issues/71"]') && row.querySelector('.age')?.textContent.includes('1 min ago')));
  assert.equal(await time.getAttribute('datetime'), end);
  assert.equal(await time.textContent(), absolute);
  await page.reload();
  assert.match(await row.locator('.age').textContent(), /1 min ago$/);
  assert.equal(await time.getAttribute('datetime'), end);
});

test("every section folder fits above the fold on a 14 inch laptop and a busy Needs you never scrolls inside itself", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(origin);
  await page.evaluate(() => document.fonts.ready);
  const rows = page.locator('#inbox > [data-group="needs you"]:not([hidden]) > .commands > .command');
  assert.equal(await rows.count(), 10);
  for (const row of await rows.all()) {
    assert.ok((await row.boundingBox()).height < 60, 'one line per row');
  }
  assert.equal(await page.locator('#inbox .commands').evaluateAll(lists => lists.every(list => list.scrollHeight <= list.clientHeight)), true);
  for (const link of await page.locator("#inbox .folders a").all()) {
    const rect = await link.boundingBox();
    assert.ok(rect.y >= 0 && rect.y + rect.height < 900, `${await link.textContent()}: ${JSON.stringify(rect)}`);
  }
});

test("Run metrics show present and missing signals in details, including at 390 px", async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  const complete = {
    outcome: 'merged', repairs: 0, ci_rounds: 2, total_minutes: 12.5,
    unaddressed_findings: { standards: 1, spec: 0, security: 2 },
    stages: [{ name: 'implement', seconds: 750, minutes: 12.5,
      usual_range: { lower_seconds: 600, upper_seconds: 900 }, position: 'within',
      review_findings: { standards: 3, spec: 4 } }]
  };
  const partial = structuredClone(complete);
  partial.total_minutes = null;
  partial.unaddressed_findings.security = null;
  partial.stages[0].usual_range = null;
  partial.stages[0].position = 'too_few_runs';
  partial.stages[0].review_findings.spec = null;
  const fact = (scope, label) => scope.locator('dt').filter({ hasText: new RegExp(`^${label}$`) }).locator('xpath=following-sibling::dd[1]');
  try {
    for (const width of [1440, 390]) {
      const page = await pageFor(t, { viewport: { width, height: 844 } });
      for (const [name, metrics] of [['complete', complete], ['partial', partial], ['older', undefined], ['unfinished', undefined]]) {
        const fixture = JSON.parse(original);
        fixture.kind = 'Run';
        fixture.run_metrics = metrics;
        if (name === 'unfinished') {
          fixture.outcome = null;
          fixture.ended_at = null;
        }
        await replaceRecord(path, JSON.stringify(fixture));
        await page.goto(origin);
        const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ }) });
        await showDetails(page, row);
        const details = page.locator('#peek');
        const shown = fact(details, 'run metrics');
        await shown.waitFor();
        if (name === 'unfinished') {
          assert.equal(await shown.textContent(), 'Missing');
        } else if (name === 'older') {
          assert.equal(await fact(shown, 'outcome').textContent(), 'failed');
          assert.equal(await fact(shown, 'repairs').textContent(), '0');
          assert.equal(await fact(shown, 'ci rounds').textContent(), '0');
          assert.equal(await fact(shown, 'total minutes').textContent(), '0.0');
          assert.equal(await fact(shown, 'unaddressed findings').textContent(), 'Missing');
        } else {
          assert.equal(await fact(shown, 'outcome').textContent(), 'merged');
          assert.equal(await fact(shown, 'repairs').textContent(), '0');
          assert.equal(await fact(shown, 'ci rounds').textContent(), '2');
          assert.equal(await fact(shown, 'total minutes').textContent(), name === 'partial' ? 'Missing' : '12.5');
          const unaddressed = fact(shown, 'unaddressed findings');
          assert.equal(await fact(unaddressed, 'standards').textContent(), '1');
          assert.equal(await fact(unaddressed, 'spec').textContent(), '0');
          assert.equal(await fact(unaddressed, 'security').textContent(), name === 'partial' ? 'Missing' : '2');
          const stages = fact(shown, 'stages');
          assert.equal(await fact(stages, 'name').textContent(), 'implement');
          assert.equal(await fact(stages, 'seconds').textContent(), '750.0');
          assert.equal(await fact(stages, 'minutes').textContent(), '12.5');
          assert.equal(await fact(stages, 'position').textContent(), name === 'partial' ? 'too_few_runs' : 'within');
          if (name === 'partial') assert.equal(await fact(stages, 'usual range').textContent(), 'Missing');
          else {
            assert.equal(await fact(stages, 'lower seconds').textContent(), '600.0');
            assert.equal(await fact(stages, 'upper seconds').textContent(), '900.0');
          }
          const review = fact(stages, 'review findings');
          assert.equal(await fact(review, 'standards').textContent(), '3');
          assert.equal(await fact(review, 'spec').textContent(), name === 'partial' ? 'Missing' : '4');
        }
        if (name !== 'unfinished') assert.equal(await fact(shown, 'confirmed bug origins').textContent(), '0');
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `${name} at ${width}px`);
        assert.equal(await details.evaluate(el => el.scrollWidth <= el.clientWidth), true, `${name} details at ${width}px`);
        await page.keyboard.press('Escape');
      }
      await page.close();
    }
  } finally { await replaceRecord(path, original); }
});

test("the Inbox works without JavaScript and Focus auto follows system light and dark", async t => {
  const page = await pageFor(t, { javaScriptEnabled: false, viewport: { width: 390, height: 844 }, colorScheme: "light" });
  await page.goto(origin);
  assert.deepEqual(await shownGroups(page), overviewGroups);
  await folder(page, 'working').click();
  await page.waitForURL('**/?section=working');
  assert.equal(await shownGroup(page).getAttribute('data-group'), 'working');
  await folder(page, 'overview').click();
  assert.deepEqual(await shownGroups(page), overviewGroups);
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(247, 247, 245)");
  await page.emulateMedia({ colorScheme: "dark" });
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
});

test("idle skipped Passes start folded and can be opened on phones with or without JavaScript", async t => {
  const config = join(home, '.thirdshift/config.toml');
  const activity = join(home, '.thirdshift/logs/acme/widgets/activity.log');
  const ready = join(home, '.thirdshift/logs/acme/widgets/commands/issue/203-fixture.command.json');
  const fired = join(home, '.thirdshift/logs/acme/widgets/pickup.last-fired');
  const fixture = JSON.parse(await readFile(join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json'), 'utf8'));
  Object.assign(fixture, { outcome: 'ready for review', pull_request: 'https://github.com/acme/widgets/pull/203' });
  fixture.issue.number = 203;
  await replaceRecord(ready, JSON.stringify(fixture));
  await writeFile(config, '[console]\nport = 0\nhidden_sections = ["needs-you", "ready-for-review", "landing", "working", "done", "resolved"]\n');
  await writeFile(activity, '2026-01-01 12:00:00 Pickup run skipped: no Ready issue on acme/widgets <script>unsafe</script>\n');
  await writeFile(fired, '2026-01-02T15:04:05Z');
  try {
    for (const javaScriptEnabled of [false, true]) {
      const page = await pageFor(t, { javaScriptEnabled, viewport: { width: 390, height: 844 } });
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(origin);
      const notice = page.getByRole('region', { name: 'No running Commands' });
      assert.equal(await notice.isVisible(), true);
      const reason = notice.getByText('Pickup run idle on acme/widgets: no Ready issue', { exact: false });
      await reason.waitFor({ state: 'hidden' });
      const summary = notice.locator('summary').filter({ hasText: 'Last recorded skipped Passes:' });
      await summary.focus();
      await page.keyboard.press('Enter');
      await reason.waitFor({ state: 'visible' });
      assert.match(await notice.locator('#idle-passes li').textContent(), /^2026-01-01 12:00 · Pickup run idle/);
      await notice.locator('#pass-starts summary').click();
      const started = notice.locator('#pass-starts li');
      await started.waitFor({ state: 'visible' });
      assert.equal(await started.locator('time').getAttribute('datetime'), '2026-01-02T15:04:05Z');
      assert.match(await started.textContent(), /^\d{4}-\d{2}-\d{2} \d{2}:\d{2} \S+ · Pickup last fired/);
      assert.equal(await notice.locator('script').count(), 0);
      assert.equal((await notice.textContent()).includes('Needs you'), false);
      for (const [label, section] of [['Ready for review', 'ready-for-review']]) {
        assert.equal(await notice.getByRole('link', { name: label, exact: true }).count(), 0, 'hidden sections must not have dead links');
        assert.ok((await notice.textContent()).includes(`${label} (hidden; show it with Sections or thirdshift console sections --show ${section})`));
      }
      assert.equal(await page.locator('#inbox > .group:visible').count(), 0);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      if (javaScriptEnabled) {
        await page.evaluate(() => { window.visitMarker = 'same idle Inbox'; });
        await replaceRecord(activity, '2026-01-01 12:01:00 Pickup run skipped: Claim limit reached\n');
        const updated = notice.getByText('Pickup run idle on acme/widgets: Claim limit reached', { exact: true });
        await updated.waitFor({ state: 'visible' });
        assert.match(await notice.locator('#idle-passes li').textContent(), /^2026-01-01 12:01 · Pickup run idle/);
        await replaceRecord(fired, '2026-01-03T15:04:05Z');
        await page.waitForFunction(() => document.querySelector('#pass-starts time')?.dateTime === '2026-01-03T15:04:05Z');
        assert.match(await started.textContent(), /^\d{4}-\d{2}-\d{2} \d{2}:\d{2} \S+ · Pickup last fired/);
        assert.equal(await page.evaluate(() => window.visitMarker), 'same idle Inbox');
        await summary.click();
        await updated.waitFor({ state: 'hidden' });
        await replaceRecord(activity, '2026-01-01 12:02:00 Pickup run skipped: no Ready issue on acme/widgets\n');
        await reason.waitFor({ state: 'attached' });
        await reason.waitFor({ state: 'hidden' });
        await summary.click();
        await reason.waitFor({ state: 'visible' });
      }
      assert.deepEqual(errors, []);
      await page.reload();
      await reason.waitFor({ state: 'hidden' });
    }
  } finally {
    await rm(activity, { force: true });
    await rm(fired, { force: true });
    await rm(ready, { force: true });
    await writeFile(config, '[console]\nport = 0\n');
  }
});

test("skipped Pass disclosure keeps native keyboard controls after selecting an Inbox row", async t => {
  const activity = join(home, '.thirdshift/logs/acme/widgets/activity.log');
  await writeFile(activity, '2026-01-01 12:00:00 Pickup run skipped: no Ready issue on acme/widgets\n');
  t.after(() => rm(activity, { force: true }));
  const page = await pageFor(t);
  await page.goto(origin);
  await page.keyboard.press('j');
  await page.keyboard.press('j');
  await page.locator('.command.is-selected').waitFor();
  const notice = page.getByRole('region', { name: 'No running Commands' });
  const reason = notice.getByText('Pickup run idle on acme/widgets: no Ready issue on acme/widgets', { exact: true });
  await notice.locator('summary').focus();
  await page.keyboard.press('Space');
  await reason.waitFor({ state: 'visible' });
  await page.keyboard.press('Enter');
  await reason.waitFor({ state: 'hidden' });
  assert.equal(await page.locator('#peek').isVisible(), false);
});

test("explicit Focus light and dark override the system preference", async t => {
  const page = await pageFor(t, { javaScriptEnabled: false });
  const config = join(home, ".thirdshift/config.toml");
  try {
    for (const [theme, system, expected] of [
      ["focus-light", "dark", "rgb(247, 247, 245)"],
      ["focus-dark", "light", "rgb(18, 18, 20)"]
    ]) {
      await writeFile(config, `[console]\nport = 0\ntheme = "${theme}"\n`);
      await page.emulateMedia({ colorScheme: system });
      await page.goto(origin);
      assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), expected);
    }
  } finally { await writeFile(config, "[console]\nport = 0\n"); }
});

test("push morphs keep the selected row, folder and scroll without a reload", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 500 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(origin);
  const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ }) });
  await row.click();
  await page.evaluate(() => {
    window.visitMarker = "same document";
    window.selectedRow = document.activeElement;
    window.scrollTo(0, 120);
  });
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const fixture = JSON.parse(await readFile(path, 'utf8'));
  fixture.issue.title = 'Advanced while you watched';
  await replaceRecord(path, JSON.stringify(fixture));
  await row.locator('.issue-title').filter({ hasText: 'Advanced while you watched' }).waitFor();
  assert.equal(await page.evaluate(() => document.activeElement === window.selectedRow && document.activeElement.matches('.command')), true);
  assert.equal(await page.evaluate(() => window.visitMarker), "same document");
  assert.deepEqual(await shownGroups(page), overviewGroups);
  assert.equal(await page.evaluate(() => scrollY), 120);
  assert.deepEqual(errors, []);
});

test("keyboard selection and the open peek survive an Inbox push", async t => {
  const page = await pageFor(t);
  await page.goto(`${origin}/?page=inbox`);
  await page.keyboard.press('j');
  await page.keyboard.press('j');
  const row = page.locator('.is-selected.command');
  const id = await row.getAttribute('id');
  const selector = await row.getAttribute('data-command');
  const path = selector.split('::')[0];
  const original = await readFile(path, 'utf8');
  try {
    await page.keyboard.press('Space');
    await page.locator('#peek dt').first().waitFor();
    const fixture = JSON.parse(original);
    fixture.issue.title = 'Keyboard-selected Command after push';
    await replaceRecord(path, JSON.stringify(fixture));
    await row.locator('.issue-title').filter({ hasText: fixture.issue.title }).waitFor();
    // The folded issue fact previews its title.
    await page.locator('#peek summary').getByText(fixture.issue.title, { exact: true }).waitFor();
    assert.equal(await row.getAttribute('id'), id);
    assert.equal(await row.getAttribute('aria-current'), 'true');
    assert.equal(await page.locator('#peek').isVisible(), true);
    await page.keyboard.press('Space');
    assert.equal(await page.locator('#peek').isVisible(), false);
    await page.keyboard.press('g');
    await page.keyboard.press('w');
    await page.waitForURL('**/?section=working');
    assert.equal(await shownGroup(page).getAttribute('data-group'), 'working');
    assert.equal(await page.locator('.is-selected').count(), 0);
  } finally { await replaceRecord(path, original); }
});

test("tab title keeps the host while Needs you and favicon follow pushes", async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  const host = await page.locator('header h1').textContent();
  assert.equal(await page.title(), `10 need you · ${host} · thirdshift Console`);
  assert.equal(await page.locator('link[rel="icon"]').getAttribute('href'), '/assets/favicon-urgent.svg');
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const fixtures = [];
  try {
    for (let number = 1; number <= 10; number++) {
      const path = join(dir, `${number}-fixture.command.json`);
      const original = await readFile(path, 'utf8');
      fixtures.push([path, original]);
      const fixture = JSON.parse(original);
      fixture.outcome = 'merged';
      await replaceRecord(path, JSON.stringify(fixture));
    }
    await page.waitForFunction(host => document.title === `0 need you · ${host} · thirdshift Console`, host);
    assert.equal(await page.locator('link[rel="icon"]').getAttribute('href'), '/assets/favicon.svg');
  } finally { for (const [path, original] of fixtures) await replaceRecord(path, original); }
});

test('reader page titles identify the host without JavaScript', async t => {
  const page = await pageFor(t, { javaScriptEnabled: false });
  await page.goto(origin);
  const host = await page.locator('header h1').textContent();
  for (const [path, title] of [
    ['/?page=history', 'History'], ['/?page=security', 'Security'],
    ['/lanes', 'Lanes'], ['/stages', 'Stages'], ['/?page=waterfall', 'Waterfall'],
    ['/logs', 'Logs'], ['/cronjobs', 'Cron jobs'], ['/resources', 'Resources'],
    ['/scores', 'Scores'], ['/bug-origins', 'Bug origins'], ['/experiments', 'Experiments']
  ]) {
    const response = await page.goto(`${origin}${path}`);
    assert.equal(response.status(), 200, path);
    assert.equal(await page.title(), `${title} · ${host} · thirdshift`, path);
  }
  await page.goto(`${origin}/logs`);
  await page.locator('.log-list a').filter({ hasText: 'Command log' }).first().click();
  assert.equal(await page.title(), `Command log · ${host} · thirdshift`);
  await page.goto(origin);
  const command = await page.locator('.command[data-command]').first().getAttribute('data-command');
  const response = await page.goto(`${origin}/stop?${new URLSearchParams({ command })}`);
  assert.equal(response.status(), 200);
  assert.equal(await page.title(), `Stop · ${host} · thirdshift`);
});

test("new since the last visit survives browser storage and clears after looking", async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  await page.waitForFunction(() => localStorage.getItem('thirdshift:console:last-look') !== null);
  await page.goto('about:blank');
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const path = join(dir, '201-fixture.command.json');
  try {
    const fixture = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
    fixture.issue.number = 201;
    fixture.started_at = fixture.ended_at = new Date().toISOString();
    fixture.cause = 'New problem after the last visit';
    await replaceRecord(path, JSON.stringify(fixture));
    await page.goto(origin);
    const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#201$/ }) });
    await row.locator('.new').waitFor();
    assert.equal(await page.locator('.new:visible').count(), 1);
    await page.reload();
    assert.equal(await page.locator('.new:visible').count(), 0);
  } finally { await rm(path, { force: true }); }
});

test("cutting the open stream falls back to polling and catches up", async t => {
  const page = await pageFor(t);
  await page.addInitScript(() => {
    const NativeSource = EventSource;
    window.EventSource = class extends NativeSource {
      constructor(...args) { super(...args); window.inboxStream = this; }
    };
  });
  await page.goto(origin);
  await page.waitForFunction(() => window.inboxStream?.readyState === EventSource.OPEN);
  await page.evaluate(() => {
    window.inboxStream.close();
    window.inboxStream.dispatchEvent(new Event('error'));
  });
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  try {
    const fixture = JSON.parse(original);
    fixture.issue.title = 'Caught up by polling';
    await replaceRecord(path, JSON.stringify(fixture));
    await page.getByText('Caught up by polling', { exact: true }).waitFor({ timeout: 12000 });
    assert.equal(await page.evaluate(() => window.inboxStream.readyState), 2);
  } finally { await replaceRecord(path, original); }
});

for (const storage of ['blocked', 'malformed']) {
  test(`live Inbox works with ${storage} browser storage`, async t => {
    const page = await pageFor(t);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(mode => {
      if (mode === 'blocked') Object.defineProperty(window, 'localStorage', { get() { throw new Error('Storage blocked'); } });
      else {
        localStorage.setItem('thirdshift:console:folded', 'false');
        localStorage.setItem('thirdshift:console:last-look', '{broken');
      }
    }, storage);
    await page.goto(origin);
    const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
    const original = await readFile(path, 'utf8');
    try {
      const fixture = JSON.parse(original);
      fixture.issue.title = `Still live with ${storage} storage`;
      await replaceRecord(path, JSON.stringify(fixture));
      await page.getByText(fixture.issue.title, { exact: true }).waitFor();
      assert.deepEqual(await shownGroups(page), overviewGroups);
      assert.deepEqual(errors, []);
    } finally { await replaceRecord(path, original); }
  });
}

{
  test('heartbeat watches Session trace records and is flat when Silent', { skip: process.platform !== 'linux' }, async t => {
    // This fixture borrows the Console's real native process identity. HTTP
    // coverage uses an actual held Run on both Linux and macOS.
    const stat = await readFile(`/proc/${server.pid}/stat`, 'utf8');
    const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
    const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    const path = join(dir, '202-fixture.command.json');
    const log = join(dir, '202-fixture.log');
    const session = join(home, 'heartbeat-session.logs.jsonl');
    const event = traceRecord;
    const fixture = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
    Object.assign(fixture, { kind: 'Run', process_id: server.pid, process_start: start, outcome: null, ended_at: null, session_log: session });
    fixture.issue.number = 202;
    const began = new Date(Date.now() - 20 * 60 * 1000).toISOString();
    fixture.stages = [{ name: 'implement', began_at: began, session_log: session }];
    await replaceRecord(path, JSON.stringify(fixture));
    await writeFile(log, 'command');
    await writeFile(session, event('session'));
    try {
      const page = await pageFor(t);
      await page.goto(origin);
      assert.equal(await page.getByRole('region', { name: 'No running Commands' }).count(), 0);
      await folder(page, 'working').click();
      const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#202$/ }) });
      const id = await row.getAttribute('id');
      await page.waitForFunction(id => document.getElementById(id).querySelector('.state').getAnimations({ subtree: true }).some(a => a.playState === 'running'), id, { timeout: 2000 });
      await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
      assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
      await page.reload();
      assert.equal(await page.getByRole('button', { name: 'Pause motion', exact: true }).getAttribute('aria-pressed'), 'true');
      assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
      await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
      assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
      await appendFile(session, event(' growth'));
      await page.waitForFunction(id => document.getElementById(id).querySelector('.heartbeat').getAnimations().some(animation => animation.playState === 'running'), id);
      await page.waitForFunction(id => document.getElementById(id).querySelector('.heartbeat').getAnimations().length === 0, id);
      for (const family of ['garden', 'factory']) {
        await writeFile(join(home, '.thirdshift/config.toml'), `[console]\nport = 0\ntheme = "${family}-auto"\n`);
        await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, `${family}-auto`);
        assert.notEqual(await page.locator('header').evaluate(header => getComputedStyle(header, '::after').animationName), 'none');
        await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
        await appendFile(session, event(' paused growth'));
        await page.waitForTimeout(350);
        assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
        await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
        await page.emulateMedia({ reducedMotion: 'reduce' });
        await page.waitForFunction(() => document.getAnimations().length === 0);
        assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
        await page.emulateMedia({ reducedMotion: 'no-preference' });
      }
      const old = new Date(Date.now() - 20 * 60 * 1000);
      await utimes(log, old, old);
      await utimes(session, old, old);
      await page.waitForFunction(id => document.getElementById(id).dataset.state === 'Silent', id);
      assert.equal(await page.getByRole('region', { name: 'No running Commands' }).count(), 0);
      assert.equal(await row.locator('.state').evaluate(state => getComputedStyle(state).animationIterationCount), '3');
      assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.waitForFunction(() => document.getAnimations().length === 0);
      assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
      await appendFile(log, ' more');
      await page.waitForFunction(id => document.getElementById(id).dataset.logging === 'true', id);
      assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
      fixture.outcome = 'merged'; fixture.ended_at = new Date().toISOString();
      await replaceRecord(path, JSON.stringify(fixture));
      await page.waitForFunction(() => document.documentElement.dataset.live === 'false');
      await page.getByRole('region', { name: 'No running Commands' }).waitFor();
      await page.emulateMedia({ reducedMotion: 'no-preference' });
      assert.equal(await page.evaluate(() => document.getAnimations().length), 0, 'an ended Command must stop the garden and conveyor');
    } finally {
      await Promise.all([path, log, session].map(path => rm(path, { force: true })));
      await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    }
  });
}

test('Ready rows show CI and merge conflicts across themes and widths and update live', async t => {
  const logs = join(home, '.thirdshift/logs');
  const dir = join(logs, 'acme/widgets/commands/issue');
  const cache = join(logs, 'github-inbox.json');
  const changed = join(logs, 'github-inbox.changed');
  const previous = await readFile(cache, 'utf8').catch(error => {
    if (error.code !== 'ENOENT') throw error;
    return null;
  });
  const base = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
  const states = [
    [229, 'green', 'CI green', 'ready', 'conflicting'], [230, 'red', 'CI red', 'urgent', 'mergeable'],
    [231, 'pending', 'CI pending', 'look', 'unknown'], [232, 'no_checks', 'No CI checks', 'done', 'conflicting'],
    [233, undefined, 'CI unknown', 'done'], [234, null, 'CI unknown', 'done'],
  ];
  const subjects = {};
  const paths = [];
  try {
    for (const [number, ci, , , mergeable] of states) {
      const path = join(dir, `${number}-fixture.command.json`);
      paths.push(path);
      const url = `https://github.com/acme/widgets/pull/${number}`;
      await replaceRecord(path, JSON.stringify({ ...base, kind: 'Run', outcome: 'ready for review',
        cause: null, tickets: [], pull_request: url,
        issue: { ...base.issue, number, url: `https://github.com/acme/widgets/issues/${number}`, title: 'Review this pull request with a long issue title' } }));
      // Undefined represents an old cache; null represents an unread PR.
      if (ci !== null) subjects[url] = { state: 'Open', merged_at: null, closed_at: null, ci, mergeable };
    }
    await replaceRecord(cache, JSON.stringify({ version: 1, subjects }));
    await writeFile(changed, 'CI fixture');
    const rowFor = (page, number) => page.locator(`.command:has(> .row-head > .subject a[href$="/issues/${number}"])`);
    for (const javaScriptEnabled of [true, false]) {
      const page = await pageFor(t, { javaScriptEnabled });
      await page.goto(`${origin}/?section=ready-for-review`);
      for (const width of [1440, 768, 390]) {
        await page.setViewportSize({ width, height: 900 });
        for (const theme of ['focus-light', 'focus-dark', 'signal-light', 'signal-dark']) {
          await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
          for (const [number, , label, tag, mergeable] of states) {
            const row = rowFor(page, number);
            assert.equal(await row.getAttribute('data-state'), 'Ready');
            assert.equal(await row.locator('.merge-conflict').count(), mergeable === 'conflicting' ? 1 : 0);
            const badges = [[row.locator('.ci'), label, tag]];
            if (mergeable === 'conflicting') badges.push([row.locator('.merge-conflict'), 'Merge conflict', 'urgent']);
            for (const [badge, label, tag] of badges) {
              await badge.getByText(label, { exact: true }).waitFor();
              const seen = await badge.evaluate((node, tag) => {
                const probe = document.createElement('span');
                document.body.append(probe);
                const token = name => { probe.style.color = `var(--${name})`; return getComputedStyle(probe).color; };
                const expected = { color: token(`tag-${tag}-text`), background: token(`tag-${tag}-bg`) };
                probe.remove();
                const style = getComputedStyle(node), rect = node.getBoundingClientRect();
                return { expected, color: style.color, background: style.backgroundColor,
                  clipped: node.scrollWidth > node.clientWidth, fits: rect.left >= 0 && rect.right <= innerWidth };
              }, tag);
              assert.equal(seen.color, seen.expected.color, `${theme}: ${label}`);
              assert.equal(seen.background, seen.expected.background, `${theme}: ${label}`);
              assert.equal(seen.clipped, false, `${width}: ${label} must be readable`);
              assert.equal(seen.fits, true, `${width}: ${label} must fit`);
            }
          }
        }
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      }
    }
    const page = await pageFor(t);
    await page.goto(`${origin}/?section=ready-for-review`);
    const row = rowFor(page, 229);
    await row.locator('.merge-conflict').waitFor();
    subjects['https://github.com/acme/widgets/pull/229'].mergeable = 'mergeable';
    await replaceRecord(cache, JSON.stringify({ version: 1, subjects }));
    await writeFile(changed, 'Mergeability changed');
    await row.locator('.merge-conflict').waitFor({ state: 'detached' });
    await row.locator('.ci').getByText('CI green', { exact: true }).waitFor();
    subjects['https://github.com/acme/widgets/pull/229'].ci = 'red';
    await replaceRecord(cache, JSON.stringify({ version: 1, subjects }));
    await writeFile(changed, 'CI changed');
    await row.locator('.ci').getByText('CI red', { exact: true }).waitFor();
    assert.equal(await row.getAttribute('data-state'), 'Ready');
  } finally {
    for (const path of paths) await rm(path, { force: true });
    if (previous === null) await rm(cache, { force: true });
    else await replaceRecord(cache, previous);
    await writeFile(changed, 'CI fixture removed');
  }
});

test('Landing, Ready and Stopping rows carry their state, Theme tags and still-able motion', { skip: process.platform !== 'linux' }, async t => {
  // The live fixtures borrow the Console's own process identity, as above.
  const stat = await readFile(`/proc/${server.pid}/stat`, 'utf8');
  const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const base = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
  const now = new Date().toISOString();
  const live = stage => ({ kind: 'Run', process_id: server.pid, process_start: start, ended_at: null, outcome: null, cause: null, stages: [{ name: stage, began_at: now, session_log: null }] });
  const fixtures = {
    211: live('CI watch'),
    212: { ended_at: now, outcome: 'ready for review', cause: null, pull_request: 'https://github.com/acme/widgets/pull/212' },
    213: live('implement'),
  };
  const stop = join(dir, '213-fixture.command.stop.json');
  const paths = [stop];
  for (const [number, change] of Object.entries(fixtures)) {
    const path = join(dir, `${number}-fixture.command.json`);
    paths.push(path, join(dir, `${number}-fixture.log`));
    await replaceRecord(path, JSON.stringify({ ...base, ...change, issue: { ...base.issue, number: Number(number), url: `https://github.com/acme/widgets/issues/${number}` } }));
    await writeFile(join(dir, `${number}-fixture.log`), 'command\n');
  }
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await folder(page, 'done').click();
      await folder(page, 'overview').click();
      assert.deepEqual(await shownGroups(page), overviewGroups);
      for (const [group, title, number] of [
        ['needs you', 'Needs you', 1], ['ready for review', 'Ready for review', 212],
        ['landing', 'Landing', 211], ['working', 'Working', 213]
      ]) {
        const section = page.getByRole('region', { name: title, exact: true });
        assert.equal(await section.getByRole('heading', { name: new RegExp(`^${title} \\d+$`) }).isVisible(), true);
        assert.equal(await section.locator(`.subject a[href$="/issues/${number}"]`).isVisible(), true, group);
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
    for (const group of ['ready for review', 'landing', 'working']) await folder(page, group).click();
    const row = number => page.locator(`[data-group] > .commands > .command:has(> .row-head > .subject a[href$="/issues/${number}"])`);
    const look = (number, tag) => row(number).evaluate((node, tag) => {
      const probe = document.createElement('span');
      document.body.append(probe);
      const token = name => { probe.style.color = `var(--${name})`; return getComputedStyle(probe).color; };
      const want = { text: token(`tag-${tag}-text`), bg: token(`tag-${tag}-bg`), done: token('tag-done-bg') };
      probe.remove();
      const label = node.querySelector(':scope > .row-head > .state');
      const style = getComputedStyle(label);
      return { ...want, state: node.dataset.state, word: label.textContent, color: style.color, background: style.backgroundColor, marker: getComputedStyle(label, '::before').content, spin: getComputedStyle(label, '::before').animationName };
    }, tag);
    for (const theme of ['focus-light', 'signal-light']) {
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      for (const [number, state, tag, group] of [[211, 'Landing', 'landing', 'landing'], [212, 'Ready', 'ready', 'ready for review']]) {
        assert.equal(await row(number).evaluate(node => node.closest('[data-group]').dataset.group), group);
        const seen = await look(number, tag);
        assert.equal(seen.state, state, `${theme}: ${number}`);
        assert.equal(seen.word, state, `${theme}: ${number}`);
        assert.equal(seen.color, seen.text, `${theme}: ${state} tag text`);
        assert.equal(seen.background, seen.bg, `${theme}: ${state} tag background`);
        if (theme === 'signal-light') assert.notEqual(seen.background, seen.done, `${theme}: ${state} must not fall back to Done`);
      }
      const landing = await look(211, 'landing');
      assert.notEqual(landing.marker, 'none', 'Landing keeps the live-activity marker');
      assert.equal(landing.spin, 'working');
      const working = await look(213, 'working');
      assert.deepEqual([working.state, working.word, working.background, working.spin], ['live', 'live', working.bg, 'working']);
    }
    // A Stop request reaches the open Inbox through its live update.
    await replaceRecord(stop, JSON.stringify({ who: 'fixture', requeue: false }));
    await page.waitForFunction(() => document.querySelector('.command:has(> .row-head > .subject a[href$="/issues/213"])')?.dataset.state === 'Stopping');
    assert.equal(await row(213).evaluate(node => node.closest('[data-group]').dataset.group), 'working');
    const stripe = () => row(213).evaluate(node => ({ word: node.querySelector(':scope > .row-head > .state').textContent, height: getComputedStyle(node, '::after').height, motion: getComputedStyle(node, '::after').animationName, running: node.getAnimations({ subtree: true }).length }));
    assert.deepEqual(await stripe(), { word: 'Stopping', height: '3px', motion: 'activity-drift', running: 1 });
    await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
    assert.deepEqual(await stripe(), { word: 'Stopping', height: '3px', motion: 'none', running: 0 });
    assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
    await page.getByRole('button', { name: 'Pause motion', exact: true }).click();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    assert.deepEqual(await stripe(), { word: 'Stopping', height: '3px', motion: 'none', running: 0 });
    // Needs you rows keep their kind beside the new states.
    assert.equal(await row(1).getAttribute('data-state'), 'Failed');
  } finally {
    await Promise.all(paths.map(path => rm(path, { force: true })));
  }
});

test("a push keeps a Pass row selected, with its dispatch in Details rather than the list", async t => {
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/security');
  await mkdir(dir, { recursive: true });
  const path = join(dir, '20261019T110000+0000~security.command.json');
  const now = new Date().toISOString();
  const fixture = {
    version: 1, kind: 'Security run', name: 'secure-fixture', repository: 'acme/widgets', hostname: 'fixture',
    issue: null, process_id: 0, process_start: 'gone', harness: 'codex', model: null, effort: null,
    started_at: now, ended_at: now, stages: [], session_log: null, outcome: 'failed', cause: 'Pass failure', issue_branch: null, pull_request: null
  };
  fixture.dispatched = [{ ...fixture, kind: 'Run', name: 'issue-303', issue: { number: 303, url: 'https://github.com/acme/widgets/issues/303', title: 'Dispatch before push' } }];
  await replaceRecord(path, JSON.stringify(fixture));
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    const child = page.locator('.command').filter({ has: page.locator(':scope > .row-head > .subject', { hasText: /^acme\/widgets#303$/ }) });
    const pass = page.locator('#inbox > .group > .commands > .command').filter({ has: child });
    assert.equal(await child.isVisible(), false);
    await pass.locator(':scope > .row-head > .state').click();
    const id = await pass.getAttribute('id');
    fixture.dispatched[0].issue.title = 'Dispatch after push';
    await replaceRecord(path, JSON.stringify(fixture));
    await child.locator('.issue-title').filter({ hasText: 'Dispatch after push' }).waitFor({ state: 'attached' });
    assert.equal(await page.evaluate(() => document.activeElement.id), id);
    const ids = await page.locator('.command').evaluateAll(rows => rows.map(row => row.id));
    assert.equal(new Set(ids).size, ids.length);
  } finally { await rm(path, { force: true }); }
});

test("selection survives a move into the Done folder and regains focus when it is opened", async t => {
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(path, 'utf8');
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    const row = page.locator('.command').filter({ has: page.locator(':scope > .row-head > .subject', { hasText: /^acme\/widgets#1$/ }) });
    await row.click();
    const id = await row.getAttribute('id');
    const fixture = JSON.parse(original);
    fixture.outcome = 'merged';
    await replaceRecord(path, JSON.stringify(fixture));
    await page.waitForFunction(id => document.getElementById(id)?.closest('.group').dataset.group === 'done', id);
    assert.equal(await page.evaluate(() => document.querySelector('.command[aria-current="true"]')?.id ?? document.activeElement.id), id);
    assert.deepEqual(await shownGroups(page), overviewGroups);
    await folder(page, 'done').click();
    await page.waitForFunction(id => document.activeElement.id === id, id);
  } finally { await replaceRecord(path, original); }
});

test("a push during a record replacement keeps the selected row selected and focused", async t => {
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const path = join(dir, '2-fixture.command.json');
  const otherLog = join(dir, '3-fixture.log');
  const [original, originalLog] = await Promise.all([readFile(path, 'utf8'), readFile(otherLog, 'utf8')]);
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    const rowFor = number => page.locator('.command').filter({ has: page.locator(':scope > .row-head > .subject', { hasText: new RegExp(`^acme/widgets#${number}$`) }) });
    const row = rowFor(2);
    await row.click();
    const id = await row.getAttribute('id');
    const other = await rowFor(3).getAttribute('id');
    // Each morph restores selection before its mutations are delivered here.
    await page.evaluate(id => {
      window.unselected = [];
      new MutationObserver(() => {
        const node = document.getElementById(id);
        if (node?.getAttribute('aria-current') !== 'true' || document.activeElement !== node) {
          window.unselected.push(node ? `${document.activeElement.tagName} focused` : 'row missing');
        }
      }).observe(document.getElementById('inbox'), { subtree: true, childList: true, characterData: true, attributes: true });
    }, id);
    const fixture = JSON.parse(original);
    const title = fixture.issue.title;
    fixture.issue.title = 'Replaced while a push arrived';
    let pushedMidway = false;
    await replaceRecord(path, JSON.stringify(fixture), {
      // Another row's log growth pushes the page while the record is replaced.
      async midway() {
        const growth = await page.locator(`[id="${other}"]`).getAttribute('data-log-growth');
        await appendFile(otherLog, 'Growth during a replacement\n');
        await page.waitForFunction(([other, growth]) => document.getElementById(other).dataset.logGrowth !== growth, [other, growth]);
        // That push still read the whole earlier record.
        assert.deepEqual(await page.evaluate(() => window.unselected), []);
        assert.equal(await row.locator('.issue-title').textContent(), title);
        pushedMidway = true;
      }
    });
    assert.equal(pushedMidway, true);
    await row.locator('.issue-title').filter({ hasText: fixture.issue.title }).waitFor();
    assert.deepEqual(await page.evaluate(() => window.unselected), []);
    assert.equal(await page.evaluate(() => document.activeElement.id), id);
  } finally {
    await replaceRecord(path, original);
    await writeFile(otherLog, originalLog);
  }
});

test("folders switch in place and Back and Forward return to them, including a URL that names none", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(`${origin}/?page=inbox`);
  await page.evaluate(() => { window.sameDocument = true; });
  await page.waitForURL('**/?section=overview');
  await folder(page, 'done').click();
  await page.waitForURL('**/?section=done');
  assert.equal(await shownGroup(page).getAttribute('data-group'), 'done');
  await page.goBack();
  await page.waitForURL('**/?section=overview');
  assert.deepEqual(await shownGroups(page), overviewGroups);
  await page.goForward();
  await page.waitForURL('**/?section=done');
  assert.equal(await shownGroup(page).getAttribute('data-group'), 'done');
  assert.equal(await page.evaluate(() => window.sameDocument), true, 'switching folders never reloads');
  // An entry that names no folder gets the server's default.
  await page.evaluate(() => history.pushState(null, '', '/'));
  await folder(page, 'resolved').click();
  await page.goBack();
  // The reload names the default folder in the URL as soon as it loads.
  await page.waitForURL('**/?section=overview');
  assert.deepEqual(await shownGroups(page), overviewGroups);
});

test("hiding the open folder with Sections returns to Overview", async t => {
  const config = join(home, '.thirdshift/config.toml');
  const directory = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const merged = join(directory, '225-fixture.command.json');
  const fixture = JSON.parse(await readFile(join(directory, '1-fixture.command.json'), 'utf8'));
  fixture.issue = { number: 225, url: 'https://github.com/acme/widgets/issues/225', title: 'Merged fixture' };
  fixture.outcome = 'merged';
  await replaceRecord(merged, JSON.stringify(fixture));
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  try {
    await page.goto(`${origin}/?section=needs-you`);
    assert.equal(await folder(page, 'ready for review').getAttribute('data-empty'), '');
    await page.getByRole('button', { name: 'Sections', exact: true }).click();
    await page.getByLabel('Show Needs you', { exact: true }).uncheck();
    await page.getByRole('button', { name: 'Save sections' }).click();
    await page.locator('#inbox .folders a[data-group="needs you"][hidden]').waitFor({ state: 'attached' });
    await page.waitForURL('**/?section=overview');
    assert.deepEqual(await shownGroups(page), ['ready for review', 'landing', 'working']);
  } finally {
    await writeFile(config, '[console]\nport = 0\n');
    await rm(merged, { force: true });
  }
});

test("every keyboard action preserves list nodes and Escape closes one layer at a time", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(origin);
  await page.evaluate(() => {
    window.rowsBefore = [...document.querySelectorAll('.command')];
    window.listChanges = [];
    for (const list of document.querySelectorAll('.commands')) {
      new MutationObserver(records => window.listChanges.push(...records.map(r => r.type)))
        .observe(list, { childList: true, subtree: true });
    }
  });
  const selected = page.locator('.is-selected');
  const detailsShow = () => page.waitForFunction(() => document.querySelector('#peek-content [name="command"]')?.value === document.querySelector('.is-selected')?.dataset.command);
  // Nothing is selected or open on load; Space opens Details, which then follows j and k.
  assert.equal(await selected.count(), 0);
  assert.equal(await page.locator('#peek').isVisible(), false);
  await page.keyboard.press('j');
  const first = await selected.getAttribute('id');
  await page.keyboard.press('Space');
  await detailsShow();
  await page.keyboard.press('j');
  assert.notEqual(await selected.getAttribute('id'), first);
  await detailsShow();
  await page.keyboard.press('k');
  assert.equal(await selected.getAttribute('id'), first);
  await page.keyboard.press('k');
  assert.equal(await selected.evaluate(el => el === el.parentElement.lastElementChild), true, 'k wraps to the last row');
  await page.keyboard.press('j');
  assert.equal(await selected.getAttribute('id'), first);
  await page.keyboard.press('Space');
  assert.equal(await page.locator('#peek').isVisible(), false);
  await page.keyboard.press('Space');
  await page.locator('#peek dt').first().waitFor();
  const peekBox = await page.locator('#peek').boundingBox();
  const mainBox = await page.locator('main').boundingBox();
  assert.ok(peekBox.x >= mainBox.x + mainBox.width, 'peek is beside the list');
  await page.keyboard.press('?');
  assert.equal(await page.locator('#keys').evaluate(el => el.open), true);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#keys').evaluate(el => el.open), false);
  assert.equal(await page.locator('#peek').isVisible(), true);
  await page.keyboard.press('g');
  assert.equal(await page.locator('#jumps').isVisible(), true);
  assert.match(await page.locator('#jumps').textContent(), /History/);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#jumps').isVisible(), false);
  assert.equal(await page.locator('#peek').isVisible(), true);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#peek').isVisible(), false);
  assert.equal(await selected.count(), 1);
  await page.keyboard.press('Escape');
  assert.equal(await selected.count(), 0);
  for (const [letter, group, slug] of [['r', 'ready for review', 'ready-for-review'], ['l', 'landing', 'landing'], ['w', 'working', 'working'], ['d', 'done', 'done'], ['v', 'resolved', 'resolved'], ['n', 'needs you', 'needs-you']]) {
    await page.keyboard.press('g');
    await page.keyboard.press(letter);
    await page.waitForURL(`**/?section=${slug}`);
    assert.equal(await shownGroup(page).getAttribute('data-group'), group);
    assert.equal(await folder(page, group).getAttribute('aria-current'), 'page');
  }
  assert.equal(await selected.getAttribute('id'), first, 'opening a folder selects its first row');
  await page.keyboard.press('g');
  await page.keyboard.press('u');
  await page.waitForURL('**/?section=overview');
  assert.deepEqual(await shownGroups(page), overviewGroups);
  assert.deepEqual(await page.evaluate(() => window.listChanges), []);
  assert.equal(await page.evaluate(() => window.rowsBefore.every((row, i) => row === document.querySelectorAll('.command')[i])), true);
  await page.keyboard.press('Enter');
  await page.waitForURL('**/log?*');
  assert.match(await page.locator('body').textContent(), /Full Command output/);
});

test("page keys, jump letters and URLs survive reload and the peek is a phone bottom sheet", async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  await page.goto(origin);
  for (const [key, name] of [['2', 'lanes'], ['3', 'stages'], ['4', 'waterfall'], ['5', 'history'], ['6', 'logs'], ['7', 'security'], ['1', 'inbox']]) {
    await page.keyboard.press(key);
    await page.waitForURL(`**/?page=${name}`);
    await page.reload();
    assert.equal(await page.locator('main h1, main h2').first().textContent(), name[0].toUpperCase() + name.slice(1));
    assert.equal(await page.locator('#peek').isVisible(), false);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  for (const [letter, name] of [['b', 'lanes'], ['t', 'stages'], ['f', 'waterfall'], ['h', 'history'], ['o', 'logs'], ['s', 'security'], ['i', 'inbox']]) {
    await page.keyboard.press('g');
    await page.keyboard.press(letter);
    await page.waitForURL(`**/?page=${name}`);
  }
  for (const [letter, path] of [['a', '/account'], ['c', '/console-log']]) {
    await page.keyboard.press('g');
    await page.keyboard.press(letter);
    await page.waitForURL(`${origin}${path}`);
    await page.goto(`${origin}/?page=inbox`);
  }
  await page.keyboard.press('j');
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  await page.locator('#peek dt').first().waitFor();
  const box = await page.locator('#peek').boundingBox();
  assert.ok(box.y > 0 && Math.abs(box.y + box.height - 844) < 2);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.reload();
  assert.equal(await page.locator('#peek').isVisible(), false);
});

test('History filters and Security findings and sessions work from a phone', async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  const streams = [];
  const errors = [];
  page.on('request', request => { if (new URL(request.url()).pathname === '/events') streams.push(request.url()); });
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${origin}/?page=history`);
  assert.match(await page.title(), /^History ·/);
  assert.equal(await page.locator('.command').count(), 11);
  await page.locator('select[name=outcome]').selectOption('failed');
  await page.locator('select[name=outcome]').focus();
  await page.keyboard.press('7');
  assert.equal(new URL(page.url()).searchParams.get('page'), 'history', 'select keeps normal keys');
  await page.locator('select[name=repository]').selectOption('acme/widgets');
  await page.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.waitForURL('**/\?page=history&outcome=failed&repository=*');
  assert.equal(await page.locator('.command').count(), 10);
  await page.goto(`${origin}/?page=security`);
  assert.equal(await page.locator('.command').count(), 1);
  assert.match(await page.locator('[aria-label="Security findings"]').textContent(), /Bound <input> before storage.*high/);
  assert.equal(await page.locator('.command input').count(), 0);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.locator('.command .state').first().click();
  await page.locator('#peek dt').first().waitFor();
  assert.match(await page.locator('#peek').textContent(), /Bound <input> before storage/);
  assert.match(await page.title(), /^Security ·/);
  assert.deepEqual(streams, []);
  assert.deepEqual(errors, []);
  await page.locator('#peek .peek-traces > summary').click();
  await page.locator('#peek a').filter({ hasText: 'Session trace:' }).click();
  await page.waitForURL('**/log?*');
  assert.match(await page.locator('body').textContent(), /Security Session output/);
});

test("Enter activates the focused Security session link", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(`${origin}/?page=security`);
  await page.locator('.command .state').first().click();
  await page.locator('#peek .peek-traces > summary').click();
  const sessionLink = page.locator('#peek a').filter({ hasText: 'Session trace:' });
  await sessionLink.waitFor();
  await sessionLink.focus();
  await page.keyboard.press('Enter');
  await page.waitForURL('**/log?*');
  assert.ok(new URL(page.url()).searchParams.has('session'),
    `focused Session link opened the Command log instead: ${page.url()}`);
  assert.match(await page.locator('body').textContent(), /Security Session output/);
});

test('Enter and Space activate focused peek buttons', async t => {
  const page = await pageFor(t);
  await page.goto(`${origin}/?page=security`);
  for (const key of ['Enter', 'Space']) {
    await page.locator('.command .state').first().click();
    await page.locator('#peek dt').first().waitFor();
    await page.locator('[data-close-peek]').focus();
    await page.keyboard.press(key);
    assert.equal(await page.locator('#peek').isVisible(), false);
    assert.equal(new URL(page.url()).searchParams.get('page'), 'security');
  }
});

test('Needs you copies problem context and links for an agent by keyboard', async t => {
  const directory = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const recordPath = join(directory, '1365-copy.command.json');
  const logPath = join(directory, '1365-copy.log');
  const fixture = JSON.parse(await readFile(join(directory, '1-fixture.command.json'), 'utf8'));
  fixture.issue = { number: 1365, url: 'https://github.com/acme/widgets/issues/1365', title: "Copy <script>alert('unsafe')</script> as text" };
  fixture.cause = 'Copy fixture failure';
  fixture.started_at = fixture.ended_at = new Date().toISOString();
  await replaceRecord(recordPath, JSON.stringify(fixture));
  await writeFile(logPath, 'Copy fixture output\n');
  t.after(async () => { await rm(recordPath, { force: true }); await rm(logPath, { force: true }); });
  const page = await pageFor(t);
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  await page.goto(origin);
  const row = page.locator('[data-group="needs you"] > .commands > .command').filter({
    has: page.locator('.subject', { hasText: /^acme\/widgets#1365$/ })
  });
  const selector = await row.getAttribute('data-command');
  const copy = row.getByRole('button', { name: 'Copy', exact: true });
  // The one-line row offers Copy without opening Command details.
  assert.equal(await copy.isVisible(), true);
  await copy.focus({ timeout: 3000 });
  await page.keyboard.press('Enter');
  await row.getByRole('status').filter({ hasText: 'Copied.' }).waitFor();
  const text = await page.evaluate(() => navigator.clipboard.readText());
  assert.match(text, /Needs you/);
  assert.ok(text.includes(`Machine: ${await page.locator('header h1').textContent()}`));
  assert.ok(text.includes('acme/widgets#1365'));
  assert.ok(text.includes("Copy <script>alert('unsafe')</script> as text"));
  assert.ok(text.includes('Failed'));
  assert.ok(text.includes('Copy fixture failure'));
  assert.ok(text.includes('codex · gpt-6 · high'));
  assert.ok(text.includes('https://github.com/acme/widgets/issues/1365'));
  assert.ok(text.includes('Ticket #98 · waiting on #97'));
  assert.ok(text.includes(`${origin}/peek?`));
  assert.ok(text.includes(`${origin}/log?`));
  assert.ok(text.includes(`${origin}/logs/command/`));
  assert.ok(text.includes(`thirdshift show '${selector}' --json`));
  assert.doesNotMatch(text, /Dismiss|New since you last looked|csrf|Copy CLI/);
  assert.equal(await page.locator('#peek').isVisible(), false);
  assert.equal(await page.locator('[data-group="resolved"] .subject', { hasText: /^acme\/widgets#1365$/ }).count(), 0);
  fixture.cause = 'Updated after copying';
  await replaceRecord(recordPath, JSON.stringify(fixture));
  await row.getByText(fixture.cause, { exact: true }).waitFor({ state: 'attached' });
  assert.equal(await row.locator('[data-copy-status]').textContent(), 'Copied.');
  await page.goto(`${origin}/?page=history`);
  assert.equal(await page.getByRole('button', { name: 'Copy', exact: true }).count(), 0);
});

for (const clipboard of ['refused', 'unavailable']) {
  test(`Needs you keeps selected context through live updates when clipboard is ${clipboard}`, async t => {
    const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
    await page.addInitScript(clipboard => {
      if (clipboard === 'unavailable') Object.defineProperty(navigator, 'clipboard', { value: undefined });
      else navigator.clipboard.writeText = () => Promise.reject(new DOMException('refused', 'NotAllowedError'));
    }, clipboard);
    await page.goto(origin);
    const row = page.locator('[data-group="needs you"] > .commands > .command').filter({
      has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ })
    });
    const recordPath = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
    const original = await readFile(recordPath, 'utf8');
    await row.getByRole('button', { name: 'Copy', exact: true }).focus();
    await page.keyboard.press('Space');
    const dialog = page.getByRole('dialog', { name: 'Needs you information' });
    await dialog.waitFor();
    const field = dialog.getByRole('textbox', { name: 'Information to share with an agent' });
    const text = await field.inputValue();
    assert.ok(text.includes('distinct fixture failure 1'));
    assert.ok(text.includes('https://github.com/acme/widgets/issues/1'));
    assert.ok(text.includes('thirdshift show'));
    assert.equal(await field.evaluate(field => field.value.slice(field.selectionStart, field.selectionEnd)), text);
    assert.equal(await field.evaluate(field => document.activeElement === field), true);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    try {
      const updated = JSON.parse(original);
      updated.cause = 'Updated while the copy dialog is open';
      await replaceRecord(recordPath, JSON.stringify(updated));
      await row.getByText(updated.cause, { exact: true }).waitFor({ state: 'attached' });
      assert.equal(await field.inputValue(), text);
      assert.equal(await field.evaluate(field => field.value.slice(field.selectionStart, field.selectionEnd)), text);
      await dialog.getByRole('button', { name: 'Close', exact: true }).focus();
      await page.keyboard.press('5');
      assert.equal(new URL(page.url()).searchParams.get('page'), null);
      await page.keyboard.press('Enter');
      await dialog.waitFor({ state: 'hidden' });
      assert.equal(await page.locator('#peek').isVisible(), false);
    } finally { await replaceRecord(recordPath, original); }
  });
}

test('Command details copies its show command to the clipboard', async t => {
  const page = await pageFor(t);
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  await page.goto(`${origin}/?page=security`);
  const selector = await page.locator('.command').first().getAttribute('data-command');
  await page.locator('.command .state').first().click();
  await page.locator('#peek dt').first().waitFor();
  const expected = `thirdshift show '${selector}' --json`;
  assert.equal(await page.locator('#peek code').filter({ hasText: 'thirdshift show' }).textContent(), expected);
  // The CLI line stays one line, cut off, with Copy CLI at its end.
  const line = page.locator('#peek .peek-cli').filter({ hasText: 'thirdshift show' });
  const [code, button] = [await line.locator('code').boundingBox(), await line.getByRole('button', { name: 'Copy CLI' }).boundingBox()];
  assert.ok(code.height < 2 * 18 && Math.abs(code.y + code.height / 2 - (button.y + button.height / 2)) < 8, JSON.stringify({ code, button }));
  assert.ok(button.x > code.x + code.width);
  await page.locator('#peek').getByRole('button', { name: 'Copy CLI', exact: true }).click();
  await page.locator('#peek').getByRole('status').filter({ hasText: 'CLI copied.' }).waitFor();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), expected);
});

test('Command details open folded, keep opened facts through a push and copy a path from its icon', async t => {
  const page = await pageFor(t);
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  await page.goto(origin);
  const row = page.locator('[data-group="needs you"] > .commands > .command').filter({
    has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ })
  });
  await showDetails(page, row);
  const issue = page.locator('#peek details[data-fact="issue"]');
  await issue.waitFor();
  assert.equal(await page.locator('#peek details[data-fact][open]').count(), 0);
  await issue.locator('> summary').click();
  const recordPath = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const original = await readFile(recordPath, 'utf8');
  try {
    const updated = JSON.parse(original);
    updated.cause = 'Pushed while a fact is open';
    await replaceRecord(recordPath, JSON.stringify(updated));
    await page.locator('#peek-content').getByText(updated.cause, { exact: true }).waitFor();
    assert.equal(await page.locator('#peek details[data-fact="issue"]').getAttribute('open'), '');
  } finally { await replaceRecord(recordPath, original); }
  // A path keeps one line and copies whole from its icon.
  const file = page.locator('#peek dd.file').first();
  const path = await file.locator('.tail').getAttribute('title');
  const box = await file.boundingBox();
  assert.ok(box.height < 2 * 18, JSON.stringify(box));
  await file.locator('.copy-icon').click();
  await file.locator('.copy-icon[data-copied]').waitFor();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), path);
});

test('Command details selects its show command when the clipboard is refused', async t => {
  const page = await pageFor(t);
  await page.addInitScript(() => {
    navigator.clipboard.writeText = () => Promise.reject(new DOMException('refused', 'NotAllowedError'));
  });
  await page.goto(`${origin}/?page=security`);
  const selector = await page.locator('.command').first().getAttribute('data-command');
  await page.locator('.command .state').first().click();
  await page.locator('#peek dt').first().waitFor();
  await page.locator('#peek').getByRole('button', { name: 'Copy CLI', exact: true }).click();
  await page.locator('#peek').getByRole('status').filter({ hasText: 'Selected; copy it with your keyboard.' }).waitFor();
  assert.equal(await page.evaluate(() => getSelection().toString()), `thirdshift show '${selector}' --json`);
});

test('Command details keeps its fallback selection across a live refresh', { skip: process.platform !== 'linux' }, async t => {
  const stat = await readFile(`/proc/${server.pid}/stat`, 'utf8');
  const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const path = join(dir, '203-fixture.command.json');
  const session = join(home, 'copy-session.logs.jsonl');
  const fixture = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
  Object.assign(fixture, { kind: 'Run', process_id: server.pid, process_start: start, outcome: null, ended_at: null, session_log: session });
  fixture.issue.number = 203;
  fixture.stages = [{ name: 'implement', began_at: new Date().toISOString(), session_log: session }];
  await replaceRecord(path, JSON.stringify(fixture));
  await writeFile(join(dir, '203-fixture.log'), 'command');
  await writeFile(session, traceRecord('session'));
  try {
    const page = await pageFor(t);
    await page.addInitScript(() => {
      navigator.clipboard.writeText = () => Promise.reject(new DOMException('refused', 'NotAllowedError'));
    });
    await page.goto(origin);
    await folder(page, 'working').click();
    const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#203$/ }) });
    await row.locator('.state').click();
    await page.locator('#peek [data-live="true"]').waitFor();
    const line = page.locator('#peek .peek-cli').filter({ hasText: 'thirdshift show' });
    await line.getByRole('button', { name: 'Copy CLI', exact: true }).click();
    await page.locator('#peek').getByRole('status').filter({ hasText: 'Selected; copy it with your keyboard.' }).waitFor();
    const cli = await page.evaluate(() => getSelection().toString());
    assert.match(cli, /^thirdshift show '.*' --json$/);
    const before = await line.elementHandle();
    await appendFile(session, traceRecord(' growth'));
    await appendFile(join(dir, '203-fixture.log'), ' more');
    await page.waitForFunction(old => !old.isConnected, before, { timeout: 5000 });
    assert.equal(await page.evaluate(() => getSelection().toString()), cli);
  } finally { await rm(path, { force: true }); }
});

test('Logs rows open the shared reader and page keys work from the reader', async t => {
  const page = await pageFor(t);
  await page.goto(`${origin}/?page=logs`);
  // One repository's logs show open under its name; j/k step through both.
  assert.equal(await page.locator('.log-group[open]').count(), 1);
  await page.keyboard.press('j');
  assert.equal(await page.locator('.log-group > summary.is-selected').count(), 1);
  await page.keyboard.press('j');
  assert.equal(await page.locator('.is-selected.log-row').count(), 1);
  await page.keyboard.press('z');
  assert.equal(await page.locator('.log-group > summary.is-selected').count(), 1);
  await page.keyboard.press('Enter');
  await page.waitForURL(/\/logs\/[a-f0-9]+$/);
  await page.keyboard.press('5');
  await page.waitForURL('**/?page=history');
  await page.keyboard.press('g');
  await page.keyboard.press('o');
  await page.waitForURL('**/?page=logs');
});

test('the active-run total follows live trees, stays visible and fits every Theme on a phone', { skip: process.platform !== 'linux' }, async t => {
  // The live fixtures borrow the Console's own process identity, as above.
  const stat = await readFile(`/proc/${server.pid}/stat`, 'utf8');
  const start = `${(await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()}:${stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]}`;
  const dir = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
  const config = join(home, '.thirdshift/config.toml');
  const base = JSON.parse(await readFile(join(dir, '1-fixture.command.json'), 'utf8'));
  const now = new Date().toISOString();
  const live = { kind: 'Run', process_id: server.pid, process_start: start, ended_at: null, outcome: null, cause: null, tickets: [], stages: [{ name: 'implement', began_at: now, session_log: null }] };
  const issue = number => ({ ...base.issue, number, url: `https://github.com/acme/widgets/issues/${number}` });
  const spec = join(dir, '221-fixture.command.json');
  const tickets = [222, 223].map(number => join(dir, `221-fixture.command.issue-${number}.child.json`));
  const separate = join(dir, '224-fixture.command.json');
  const write = (path, number, change = {}) => replaceRecord(path, JSON.stringify({ ...base, ...live, issue: issue(number), ...change }));
  const ended = { outcome: 'merged', ended_at: now };
  try {
    const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
    await page.goto(origin);
    const total = page.locator('#inbox > .active-runs');
    assert.equal(await total.locator('strong').textContent(), '0 active runs');
    await write(spec, 221, { kind: 'Spec run' });
    for (const [index, path] of tickets.entries()) await write(path, 222 + index, { parent_record: spec });
    await page.waitForFunction(() => document.querySelector('#inbox > .active-runs strong').textContent === '1 active run');
    assert.equal(await page.getByRole('region', { name: 'No running Commands' }).count(), 0);
    await write(separate, 224);
    await page.waitForFunction(() => document.querySelector('#inbox > .active-runs strong').textContent === '2 active runs');
    // Moving between folders, into and out of the live work, leaves the total alone.
    for (const group of ['working', 'needs you']) {
      await folder(page, group).click();
      assert.equal(await shownGroup(page).getAttribute('data-group'), group);
      assert.equal(await total.locator('strong').textContent(), '2 active runs');
    }
    // A Spec's Ticket ending while the Spec continues keeps its one run.
    await write(tickets[0], 222, { parent_record: spec, ...ended });
    await page.waitForTimeout(1500);
    assert.equal(await total.locator('strong').textContent(), '2 active runs');
    // Hidden sections and every built-in Theme keep the total readable on a phone.
    await writeFile(config, '[console]\nport = 0\nhidden_sections = ["needs-you", "ready-for-review", "landing", "working", "done", "resolved"]\n');
    await page.waitForFunction(() => document.querySelectorAll('#inbox > details:not([hidden])').length === 0);
    for (const family of ['focus', 'plain', 'press', 'signal', 'garden', 'factory']) {
      for (const mode of ['light', 'dark']) {
        await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, `${family}-${mode}`);
        assert.equal(await total.isVisible(), true, `${family}-${mode}`);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `${family}-${mode}`);
      }
    }
    await write(separate, 224, ended);
    await page.waitForFunction(() => document.querySelector('#inbox > .active-runs strong').textContent === '1 active run');
    await write(tickets[1], 223, { parent_record: spec, ...ended });
    await write(spec, 221, { kind: 'Spec run', ...ended });
    await page.waitForFunction(() => document.querySelector('#inbox > .active-runs strong').textContent === '0 active runs');
    await page.getByRole('region', { name: 'No running Commands' }).waitFor();
  } finally {
    await Promise.all([spec, separate, ...tickets].map(path => rm(path, { force: true })));
    await writeFile(config, '[console]\nport = 0\n');
  }
});
