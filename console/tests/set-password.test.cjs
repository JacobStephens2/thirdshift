const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { createServer } = require("node:http");
const { mkdtemp, mkdir, writeFile, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright");

let home, resend, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), "thirdshift-console-set-password-"));
  const root = join(home, ".thirdshift");
  await mkdir(root);
  // No theme: the default `focus-auto` follows the browser's color scheme.
  await writeFile(join(root, "config.toml"), '[console]\nport = 0\nallowed_emails = ["owner@example.test"]\n[email]\nfrom = "console@example.test"\n');
  resend = createServer((request, response) => { request.resume(); response.end("{}"); });
  resend.listen(0, "127.0.0.1");
  await once(resend, "listening");
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift"), ["console"], {
    env: { ...process.env, HOME: home, RESEND_API_KEY: "re_browser_test", THIRDSHIFT_RESEND_URL: `http://127.0.0.1:${resend.address().port}` },
    stdio: ["ignore", "pipe", "inherit"]
  });
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
  resend?.close();
  if (home) await rm(home, { recursive: true, force: true });
});

for (const [colorScheme, background] of [["dark", "rgb(18, 18, 20)"], ["light", "rgb(247, 247, 245)"]]) {
  test(`the sent-code page follows a ${colorScheme} system color scheme`, async t => {
    const context = await browser.newContext({ javaScriptEnabled: false, colorScheme });
    t.after(() => context.close());
    const page = await context.newPage();
    await page.goto(`${origin}/set-password`);
    await page.getByLabel("Email").first().fill("owner@example.test");
    await Promise.all([page.waitForURL(`${origin}/set-password/code`), page.getByRole("button", { name: "Send code" }).click()]);
    await page.getByText("a code has been sent").waitFor();
    assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), background);
  });
}
