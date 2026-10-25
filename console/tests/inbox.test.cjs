const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, readFile, writeFile, appendFile, utimes, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright");
const { record: traceRecord } = require("./session-trace.cjs");

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
    await page.locator('[data-group="done"][hidden]').waitFor({ state: 'attached' });
    const other = await pageFor(t);
    await other.goto(origin);
    await other.locator('[data-group="done"][hidden]').waitFor({ state: 'attached' });
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

test("Inbox folds, loads only its own resources and fits a 390 px phone", async t => {
  const page = await pageFor(t, { viewport: { width: 390, height: 844 } });
  const remote = [], errors = [];
  page.on("request", request => { if (!request.url().startsWith(origin)) remote.push(request.url()); });
  page.on("pageerror", error => errors.push(error.message));
  const response = await page.goto(origin);
  assert.equal(response.status(), 200);
  await page.evaluate(() => document.fonts.ready);
  assert.equal(await page.locator('details[open]').count(), 1);
  assert.equal(await page.locator('details[open]').getAttribute("data-group"), "needs you");
  assert.match(await page.locator(".plan").first().textContent(), /Ticket #98 · waiting on #97/);
  assert.equal(await page.locator('.command .state').first().textContent(), "Failed");
  assert.equal(await page.locator(".command script").count(), 0);
  for (const summary of await page.locator("summary").all()) {
    await summary.click();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  assert.deepEqual(remote, []);
  assert.deepEqual(errors, []);
  assert.equal(await page.evaluate(() => document.fonts.check('14px "IBM Plex Sans"') && document.fonts.check('12px "IBM Plex Mono"')), true);
});

test("all section headings fit above the fold on a 14 inch laptop with a busy Needs you", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(origin);
  await page.evaluate(() => document.fonts.ready);
  assert.equal(await page.locator('[data-group="needs you"][open] .command').count(), 10);
  for (const summary of await page.locator("summary").all()) {
    const rect = await summary.boundingBox();
    assert.ok(rect.y >= 0 && rect.y + rect.height < 900, `${await summary.textContent()}: ${JSON.stringify(rect)}`);
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
        await writeFile(path, JSON.stringify(fixture));
        await page.goto(origin);
        const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ }) });
        await row.locator('.peek-link').click();
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
  } finally { await writeFile(path, original); }
});

test("the Inbox works without JavaScript and Focus auto follows system light and dark", async t => {
  const page = await pageFor(t, { javaScriptEnabled: false, viewport: { width: 390, height: 844 }, colorScheme: "light" });
  await page.goto(origin);
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 1);
  await page.locator('[data-group="working"] > summary').click();
  assert.equal(await page.locator('[data-group="working"][open]').count(), 1);
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(247, 247, 245)");
  await page.emulateMedia({ colorScheme: "dark" });
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
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

test("push morphs keep the selected row, folds and scroll without a reload", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(origin);
  const row = page.locator('.command').filter({ has: page.locator('.subject', { hasText: /^acme\/widgets#1$/ }) });
  await page.locator('[data-group="done"] > summary').click();
  await row.click();
  await page.evaluate(() => {
    window.visitMarker = "same document";
    window.selectedRow = document.activeElement;
    document.querySelector('.commands').scrollTop = 120;
  });
  const path = join(home, '.thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json');
  const fixture = JSON.parse(await readFile(path, 'utf8'));
  fixture.issue.title = 'Advanced while you watched';
  await writeFile(path, JSON.stringify(fixture));
  await page.getByText('Advanced while you watched', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => document.activeElement === window.selectedRow && document.activeElement.matches('.command')), true);
  assert.equal(await page.evaluate(() => window.visitMarker), "same document");
  assert.equal(await page.locator('[data-group="done"][open]').count(), 1);
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 1);
  assert.equal(await page.locator('[data-group="needs you"] .commands').evaluate(el => el.scrollTop), 120);
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
    await writeFile(path, JSON.stringify(fixture));
    await row.locator('.issue-title').filter({ hasText: fixture.issue.title }).waitFor();
    await page.locator('#peek').getByText(fixture.issue.title, { exact: true }).waitFor();
    assert.equal(await row.getAttribute('id'), id);
    assert.equal(await row.getAttribute('aria-current'), 'true');
    assert.equal(await page.locator('#peek').isVisible(), true);
    await page.keyboard.press('Space');
    assert.equal(await page.locator('#peek').isVisible(), false);
    await page.keyboard.press('g');
    await page.keyboard.press('w');
    assert.equal(await page.locator('.is-selected').evaluate(el => el.parentElement.dataset.group), 'working');
  } finally { await writeFile(path, original); }
});

test("tab title and favicon follow Needs you through pushes", async t => {
  const page = await pageFor(t);
  await page.goto(origin);
  assert.equal(await page.title(), '10 need you · thirdshift Console');
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
      await writeFile(path, JSON.stringify(fixture));
    }
    await page.waitForFunction(() => document.title === '0 need you · thirdshift Console');
    assert.equal(await page.locator('link[rel="icon"]').getAttribute('href'), '/assets/favicon.svg');
  } finally { for (const [path, original] of fixtures) await writeFile(path, original); }
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
    await writeFile(path, JSON.stringify(fixture));
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
    await writeFile(path, JSON.stringify(fixture));
    await page.getByText('Caught up by polling', { exact: true }).waitFor({ timeout: 12000 });
    assert.equal(await page.evaluate(() => window.inboxStream.readyState), 2);
  } finally { await writeFile(path, original); }
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
      await writeFile(path, JSON.stringify(fixture));
      await page.getByText(fixture.issue.title, { exact: true }).waitFor();
      assert.equal(await page.locator('[data-group="needs you"][open]').count(), 1);
      assert.deepEqual(errors, []);
    } finally { await writeFile(path, original); }
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
    await writeFile(path, JSON.stringify(fixture));
    await writeFile(log, 'command');
    await writeFile(session, event('session'));
    try {
      const page = await pageFor(t);
      await page.goto(origin);
      await page.locator('[data-group="working"] > summary').click();
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
      assert.equal(await row.locator('.state').evaluate(state => getComputedStyle(state).animationIterationCount), '3');
      assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.waitForFunction(() => document.getAnimations().length === 0);
      assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
      await appendFile(log, ' more');
      await page.waitForFunction(id => document.getElementById(id).dataset.logging === 'true', id);
      assert.equal(await row.locator('.heartbeat').evaluate(dot => dot.getAnimations().length), 0);
      fixture.outcome = 'merged'; fixture.ended_at = new Date().toISOString();
      await writeFile(path, JSON.stringify(fixture));
      await page.waitForFunction(() => document.documentElement.dataset.live === 'false');
      await page.emulateMedia({ reducedMotion: 'no-preference' });
      assert.equal(await page.evaluate(() => document.getAnimations().length), 0, 'an ended Command must stop the garden and conveyor');
    } finally {
      await Promise.all([path, log, session].map(path => rm(path, { force: true })));
      await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\n');
    }
  });
}

test("a push keeps a Pass dispatch selected instead of its parent", async t => {
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
  await writeFile(path, JSON.stringify(fixture));
  try {
    const page = await pageFor(t);
    await page.goto(origin);
    const child = page.locator('.command').filter({ has: page.locator(':scope > .row-head > .subject', { hasText: /^acme\/widgets#303$/ }) });
    await child.click();
    fixture.dispatched[0].issue.title = 'Dispatch after push';
    await writeFile(path, JSON.stringify(fixture));
    await page.getByText('Dispatch after push', { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.activeElement.querySelector(':scope > .row-head > .subject')?.textContent), 'acme/widgets#303');
    const ids = await page.locator('.command').evaluateAll(rows => rows.map(row => row.id));
    assert.equal(new Set(ids).size, ids.length);
  } finally { await rm(path, { force: true }); }
});

test("selection survives a move into folded Done and regains focus when opened", async t => {
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
    await writeFile(path, JSON.stringify(fixture));
    await page.waitForFunction(id => document.getElementById(id)?.closest('details').dataset.group === 'done', id);
    assert.equal(await page.evaluate(() => document.querySelector('.command[aria-current="true"]')?.id ?? document.activeElement.id), id);
    assert.equal(await page.locator('[data-group="done"][open]').count(), 0);
    await page.locator('[data-group="done"] > summary').click();
    await page.waitForFunction(id => document.activeElement.id === id, id);
  } finally { await writeFile(path, original); }
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
  await page.keyboard.press('j');
  assert.equal(await selected.evaluate(el => el.tagName), 'SUMMARY');
  await page.keyboard.press('k');
  assert.equal(await selected.textContent().then(s => s.trim().startsWith('Resolved')), true);
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 0);
  await page.keyboard.press('Space');
  await page.keyboard.press('j');
  assert.equal(await selected.getAttribute('class').then(s => s.includes('command')), true);
  await page.keyboard.press('k');
  assert.equal(await selected.evaluate(el => el.tagName), 'SUMMARY');
  await page.keyboard.press('j');
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
  for (const [letter, group] of [['r', 'ready for review'], ['l', 'landing'], ['d', 'done'], ['v', 'resolved']]) {
    await page.keyboard.press('g');
    await page.keyboard.press(letter);
    assert.equal(await selected.evaluate(el => el.parentElement.dataset.group), group);
    assert.equal(await selected.evaluate(el => el.parentElement.open), true);
  }
  await page.keyboard.press('g');
  await page.keyboard.press('w');
  assert.equal(await page.locator('[data-group="working"][open]').count(), 1);
  await page.keyboard.press('g');
  await page.keyboard.press('n');
  await page.keyboard.press('j');
  await page.keyboard.press('z');
  assert.equal(await page.locator('[data-group="needs you"][open]').count(), 0);
  await page.keyboard.press('Space');
  await page.keyboard.press('j');
  await page.keyboard.press('Space');
  await page.locator('#peek dt').first().waitFor();
  await page.keyboard.press('Space');
  assert.equal(await page.locator('#peek').isVisible(), false);
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
  await page.locator('.peek-link').click();
  await page.locator('#peek dt').first().waitFor();
  assert.match(await page.locator('#peek').textContent(), /Bound <input> before storage/);
  assert.match(await page.title(), /^Security ·/);
  assert.deepEqual(streams, []);
  assert.deepEqual(errors, []);
  await page.locator('#peek a').filter({ hasText: 'Session trace:' }).click();
  await page.waitForURL('**/log?*');
  assert.match(await page.locator('body').textContent(), /Security Session output/);
});

test("Enter activates the focused Security session link", async t => {
  const page = await pageFor(t, { viewport: { width: 1440, height: 900 } });
  await page.goto(`${origin}/?page=security`);
  await page.locator('.peek-link').click();
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
    await page.locator('.peek-link').click();
    await page.locator('#peek dt').first().waitFor();
    await page.locator('[data-close-peek]').focus();
    await page.keyboard.press(key);
    assert.equal(await page.locator('#peek').isVisible(), false);
    assert.equal(new URL(page.url()).searchParams.get('page'), 'security');
  }
});

test('Logs rows open the shared reader and page keys work from the reader', async t => {
  const page = await pageFor(t);
  await page.goto(`${origin}/?page=logs`);
  await page.keyboard.press('j');
  assert.equal(await page.locator('.is-selected.log-row').count(), 1);
  await page.keyboard.press('z');
  await page.keyboard.press('Enter');
  await page.waitForURL(/\/logs\/[a-f0-9]+$/);
  await page.keyboard.press('5');
  await page.waitForURL('**/?page=history');
  await page.keyboard.press('g');
  await page.keyboard.press('o');
  await page.waitForURL('**/?page=logs');
});
