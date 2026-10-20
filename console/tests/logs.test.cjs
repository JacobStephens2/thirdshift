const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, readFile, writeFile, appendFile, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright");

let home, server, exited, browser, origin, folder, log, recordPath, record;
const message = text => JSON.stringify({ type: "item.completed", item: { id: text, type: "agent_message", text } }) + "\n";
const tool = text => JSON.stringify({ type: "item.completed", item: { id: text, type: "command_execution", command: text, aggregated_output: "output", status: "completed", exit_code: 0 } }) + "\n";
before(async () => {
  home = await mkdtemp(join(tmpdir(), "thirdshift-console-logs-"));
  folder = join(home, ".thirdshift/logs/acme/widgets/commands/issue");
  await mkdir(folder, { recursive: true });
  await writeFile(join(home, ".thirdshift/config.toml"), "[console]\nport = 0\n");
  const stem = "7-20261003T120000-0400~fixture";
  log = join(folder, `${stem}-implement.jsonl`);
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
    started_at: new Date().toISOString(), ended_at: null, stages: [], session_log: log,
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
