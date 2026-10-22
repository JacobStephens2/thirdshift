const { test, describe, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, readFile, writeFile, appendFile, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright");

for (const storage of ['Session log', 'Session trace', 'Session trace only']) {
  describe(storage, () => {
    let home, server, exited, browser, origin, folder, log, recordPath, record;
    const encode = event => {
      const raw = JSON.stringify(event);
      return (storage === 'Session log' ? raw : JSON.stringify({ resourceLogs: [{ scopeLogs: [{ logRecords: [{ body: { stringValue: raw } }] }] }] })) + '\n';
    };
    const message = text => encode({ type: "item.completed", item: { id: text, type: "agent_message", text } });
    const tool = text => encode({ type: "item.completed", item: { id: text, type: "command_execution", command: text, aggregated_output: "output", status: "completed", exit_code: 0 } });
    before(async () => {
      home = await mkdtemp(join(tmpdir(), "thirdshift-console-logs-"));
      folder = join(home, ".thirdshift/logs/acme/widgets/commands/issue");
      await mkdir(folder, { recursive: true });
      await writeFile(join(home, ".thirdshift/config.toml"), "[console]\nport = 0\n");
      const stem = "7-20261003T120000-0400~fixture";
      const sessionLog = join(folder, `${stem}-implement.jsonl`);
      log = storage === 'Session log' ? sessionLog : join(folder, `${stem}-implement.logs.jsonl`);
      if (storage === 'Session trace') await writeFile(sessionLog, 'Stale Session log\n');
      recordPath = join(folder, `${stem}.command.json`);
      await writeFile(log, message("Initial retained line") + message("Tall initial message\n".repeat(150)) + tool("first tool"));
      await writeFile(join(folder, `${stem}.log`), "Command <plain> output\n");
      // Match the native process identity used by the Linux Console test runner.
      let processStart = "gone";
      if (process.platform === "linux") {
        const stat = await readFile(`/proc/${process.pid}/stat`, "utf8");
        const fields = stat.slice(stat.lastIndexOf(") ") + 2).trim().split(/\s+/);
        processStart = `${(await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim()}:${fields[19]}`;
      }
      record = {
        version: 1, kind: "Spec run", repository: "acme/widgets", name: "acme/widgets#7", hostname: "fixture",
        issue: { number: 7, url: "https://github.com/acme/widgets/issues/7", title: "Widgets" },
        process_id: process.pid, process_start: processStart, harness: "codex", model: "gpt-6", effort: "high",
        started_at: new Date().toISOString(), ended_at: null, stages: [], session_log: sessionLog,
        outcome: null, cause: null, issue_branch: null, pull_request: null
      };
      await writeFile(recordPath, JSON.stringify(record));
      server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift"), ["console", "--no-sign-in"], { env: { ...process.env, HOME: home }, stdio: ["ignore", "pipe", "inherit"] });
      exited = once(server, "exit");
      const lines = createInterface({ input: server.stdout });
      try { [origin] = await once(lines, "line", { signal: AbortSignal.timeout(10000) }); }
      finally { lines.close(); }
      browser = await chromium.launch({ headless: true });
    });
    after(async () => {
      await browser?.close();
      if (server && server.exitCode === null) server.kill();
      await exited;
      if (home) await rm(home, { recursive: true, force: true });
    });

    test("a live Session appends without redraw, folds across batches, resumes offsets and obeys follow", { skip: process.platform !== "linux" }, async t => {
      const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
      t.after(() => context.close());
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.goto(`${origin}/logs`);
      assert.equal(await page.locator(".log-list strong").count(), 2);
      await page.getByRole("link", { name: /Session log/ }).click();
      await page.getByRole('link', { name: 'Command details', exact: true }).click();
      await page.getByRole('button', { name: 'Stop', exact: true }).waitFor();
      await page.getByRole('button', { name: 'Close details (Esc)', exact: true }).click();
      const initial = await page.locator("#log-content .message").first().elementHandle();
      await page.locator(".tool-group").evaluate(node => { node.open = true; });
      await appendFile(log, tool("second tool"));
      await page.waitForFunction(() => document.querySelectorAll(".tool").length === 2);
      assert.equal(await page.locator(".tool-group").count(), 1);
      assert.equal(await page.locator(".tool-group").evaluate(node => node.open), true);
      assert.equal(await initial.evaluate(node => node === document.querySelector("#log-content .message")), true);
      await page.locator("#follow-log").uncheck();
      await page.evaluate(() => window.scrollTo(0, 0));
      const partial = message("Appended <script>unsafe()</script> line");
      const previousOffset = await page.locator("#log-content").getAttribute("data-offset");
      await appendFile(log, partial.slice(0, -1));
      // Observe a complete polling interval; an unfinished JSON line must wait.
      await page.waitForTimeout(650);
      assert.equal(await page.locator("#log-content").getAttribute("data-offset"), previousOffset);
      await appendFile(log, "\n");
      await page.getByText("Appended <script>unsafe()</script> line", { exact: true }).waitFor();
      assert.equal(await page.evaluate(() => scrollY), 0);
      assert.equal(await page.locator("#log-content script").count(), 0);
      assert.equal(await initial.evaluate(node => node.isConnected), true);
      await page.locator("#follow-log").check();
      await appendFile(log, message("Followed newest line\n".repeat(60)));
      await page.waitForFunction(() => document.querySelector("#log-content").textContent.includes("Followed newest line"));
      await page.waitForFunction(() => scrollY + innerHeight >= document.documentElement.scrollHeight - 2);
      const offset = Number(await page.locator("#log-content").getAttribute("data-offset"));
      assert.equal(offset, (await readFile(log)).length, 'offset counts bytes in the file being streamed');
      await appendFile(log, message("Reconnect only this line"));
      // A new connection from the saved byte offset emits only subsequent lines.
      const resumed = await page.evaluate(async offset => {
        const base = document.querySelector("#log-content").dataset.stream.replace(/\d+$/, "0");
        const response = await fetch(base, { headers: { "Last-Event-ID": String(offset) } });
        const reader = response.body.getReader();
        const chunk = await reader.read();
        await reader.cancel();
        return new TextDecoder().decode(chunk.value);
      }, offset);
      assert.match(resumed, /Reconnect only this line/);
      assert.doesNotMatch(resumed, /Initial retained line|Followed newest line/);
      // A completed Command drains a final line even without its newline.
      await appendFile(log, message("Final unterminated line").trimEnd());
      record.outcome = "ready for review";
      record.ended_at = new Date().toISOString();
      await writeFile(recordPath, JSON.stringify(record));
      await page.getByText("Final unterminated line", { exact: true }).waitFor();
      await page.waitForFunction(() => document.querySelector("#log-status").textContent === "Complete");
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      assert.deepEqual(errors, []);
    });

    test("Session log polls appended lines when EventSource is unavailable", { skip: process.platform !== "linux" }, async t => {
      await appendFile(log, "\n"); // Finish the preceding test's unterminated final event.
      record.outcome = null;
      record.ended_at = null;
      await writeFile(recordPath, JSON.stringify(record));
      const context = await browser.newContext();
      t.after(() => context.close());
      const page = await context.newPage();
      // An SSE-blocking intermediary still permits finite HTTP requests.
      await page.addInitScript(() => {
        window.EventSource = class extends EventTarget {
          constructor() {
            super();
            setTimeout(() => {
              this.dispatchEvent(new Event('error'));
              this.onerror?.(new Event('error'));
            }, 20);
          }
          close() {}
        };
      });
      await page.goto(`${origin}/logs`);
      await page.getByRole('link', { name: /Session log/ }).click();
      const initial = await page.locator('#log-content .message').first().elementHandle();
      await appendFile(log, message('Visible through polling fallback'));
      await page.getByText('Visible through polling fallback', { exact: true }).waitFor({ timeout: 12000 });
      assert.equal(await initial.evaluate(node => node === document.querySelector('#log-content .message')), true);
    });

    test("poll response cannot duplicate lines already appended by recovered SSE", { skip: process.platform !== "linux" }, async t => {
      const context = await browser.newContext();
      t.after(() => context.close());
      const page = await context.newPage();
      await page.addInitScript(() => {
        window.EventSource = class extends EventTarget {
          constructor(url) { super(); this.url = url; if (url.includes('/stream/')) window.readerSource = this; }
          close() {}
        };
      });
      let releasePoll, sawPoll, finishedPoll;
      const release = new Promise(resolve => { releasePoll = resolve; });
      const captured = new Promise(resolve => { sawPoll = resolve; });
      const completed = new Promise(resolve => { finishedPoll = resolve; });
      let held = false;
      await page.route('**/logs/*/stream/*', async route => {
        if (held || route.request().headers().accept !== 'application/json') return route.continue();
        held = true; sawPoll();
        await release;
        try { await route.fulfill({ response: await route.fetch() }); }
        catch { /* Recovery may correctly abort this superseded poll. */ }
        finally { finishedPoll(); }
      });
      await page.goto(`${origin}/logs`);
      await page.getByRole('link', { name: /Session log/ }).click();
      await page.waitForFunction(() => window.readerSource);
      const path = await page.locator('#log-content').getAttribute('data-stream');
      await appendFile(log, message('First line after disconnect'));
      await page.evaluate(() => window.readerSource.onerror(new Event('error')));
      await captured;
      const first = await (await fetch(origin + path, { headers: { accept: 'application/json' } })).json();
      await page.evaluate(batch => {
        window.readerSource.dispatchEvent(new Event('open'));
        window.readerSource.dispatchEvent(new MessageEvent('lines', { data: JSON.stringify(batch) }));
      }, first);
      await page.getByText('First line after disconnect', { exact: true }).waitFor();
      await appendFile(log, message('Second line while old poll waits'));
      releasePoll();
      await completed;
      await page.waitForTimeout(250);
      const secondPath = path.replace(/\d+$/, String(first.offset));
      const second = await (await fetch(origin + secondPath, { headers: { accept: 'application/json' } })).json();
      await page.evaluate(batch => window.readerSource.dispatchEvent(new MessageEvent('lines', { data: JSON.stringify(batch) })), second);
      // The old poll was requested before first and contains both lines. It must
      // be discarded; the recovered stream provides the second line exactly once.
      await page.getByText('Second line while old poll waits', { exact: true }).waitFor({ timeout: 12000 });
      assert.equal(await page.getByText('First line after disconnect', { exact: true }).count(), 1,
        'Recovered SSE plus an older overlapping poll must append each Session line once');
    });

    test('stable normalized messages update across stream batches and reload', { skip: process.platform !== 'linux' || storage === 'Session log' }, async t => {
      record.outcome = null;
      record.ended_at = null;
      await writeFile(recordPath, JSON.stringify(record));
      const snapshot = text => JSON.stringify({ resourceLogs: [{ scopeLogs: [{ logRecords: [{
        body: { stringValue: 'retained raw snapshot' },
        attributes: [{ key: 'thirdshift.events', value: { stringValue: JSON.stringify([
          { type: 'message', id: 'streamed-reply', parent_id: null, role: 'assistant', text }
        ]) } }]
      }] }] }] }) + '\n';
      await appendFile(log, snapshot('Snapshot first'));
      const context = await browser.newContext();
      t.after(() => context.close());
      const page = await context.newPage();
      await page.goto(`${origin}/logs`);
      await page.getByRole('link', { name: /Session log/ }).click();
      const original = await page.getByText('Snapshot first', { exact: true }).locator('..').elementHandle();
      await appendFile(log, snapshot('Snapshot first complete'));
      await page.getByText('Snapshot first complete', { exact: true }).waitFor();
      assert.equal(await page.getByText('Snapshot first', { exact: true }).count(), 0);
      assert.equal(await page.locator('.message[data-message-id]').count(), 1);
      assert.equal(await original.evaluate(node => node === document.querySelector('.message[data-message-id]')), true);
      await page.reload();
      assert.equal(await page.getByText('Snapshot first', { exact: true }).count(), 0);
      assert.equal(await page.getByText('Snapshot first complete', { exact: true }).count(), 1);
      assert.equal(await page.locator('.message[data-message-id]').count(), 1);
    });

    test('SSE recovery does not repeat polled lines', { skip: process.platform !== 'linux' }, async t => {
      const context = await browser.newContext();
      t.after(() => context.close());
      const page = await context.newPage();
      await page.addInitScript(() => {
        const NativeSource = window.EventSource;
        window.EventSource = class extends EventTarget {
          constructor(url) { super(); this.url = url; this.connect(); window.recoverySource = this; }
          connect() {
            this.native = new NativeSource(this.url);
            this.native.addEventListener('open', () => { this.opened = true; this.dispatchEvent(new Event('open')); });
            this.native.addEventListener('lines', event => this.dispatchEvent(new MessageEvent('lines', { data: event.data, lastEventId: event.lastEventId })));
            this.native.addEventListener('log-error', event => this.dispatchEvent(new MessageEvent('log-error', { data: event.data })));
          }
          close() { this.native.close(); }
          pause() { this.native.close(); }
          fail() { const event = new Event('error'); this.dispatchEvent(event); this.onerror?.(event); }
          resume() { this.connect(); }
        };
      });
      await page.goto(`${origin}/logs`);
      await page.getByRole('link', { name: /Session log/ }).click();
      await page.waitForFunction(() => window.recoverySource?.opened);
      await page.evaluate(() => window.recoverySource.pause());
      await appendFile(log, message('Delivered by polling'));
      await page.evaluate(() => window.recoverySource.fail());
      await page.getByText('Delivered by polling', { exact: true }).waitFor({ timeout: 12000 });
      await appendFile(log, message('Delivered after SSE recovery'));
      await page.evaluate(() => window.recoverySource.resume());
      await page.getByText('Delivered after SSE recovery', { exact: true }).waitFor();
      const visible = await page.locator('#log-content .message pre').allTextContents();
      assert.equal(visible.filter(text => text === 'Delivered by polling').length, 1,
        `SSE replayed a line already appended by polling: ${JSON.stringify(visible)}`);
    });
  });
}
