const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, writeFile, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright");

let home, server, exited, browser, origin;
before(async () => {
  home = await mkdtemp(join(tmpdir(), "thirdshift-console-accounts-"));
  const root = join(home, ".thirdshift");
  await mkdir(root);
  await writeFile(join(root, "config.toml"), '[console]\nport = 0\nallowed_emails = ["owner@example.test"]\ntheme = "focus-dark"\n');
  // A fixture created with `console password` for "browser test password".
  // The Rust HTTP tests exercise account creation through the real terminal.
  await writeFile(join(root, "console-accounts.json"), JSON.stringify({
    "owner@example.test": {
      password_hash: "$argon2id$v=19$m=19456,t=2,p=1$Yd89fw8Yx3I9TbhuY+Y/cQ$NhL+Lqg2T9nXrlM8NKBaQ3dpVpRWjRsyYbeNkQS8Q7s",
      sessions: {}
    }
  }), { mode: 0o600 });
  server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift"), ["console"], {
    env: { ...process.env, HOME: home }, stdio: ["ignore", "pipe", "inherit"]
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
  if (home) await rm(home, { recursive: true, force: true });
});

test("sign-in and sign-out work on a phone without JavaScript using secure cookies", async t => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 390, height: 844 }, colorScheme: "light" });
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(origin);
  assert.equal(new URL(page.url()).pathname, "/sign-in");
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.getByLabel("Email").fill("owner@example.test");
  await page.getByLabel("Password", { exact: true }).fill("browser test password");
  await Promise.all([page.waitForURL(`${origin}/`), page.getByRole("button", { name: "Sign in", exact: true }).click()]);
  await page.getByRole("heading", { name: "Inbox", exact: true }).waitFor();
  const session = (await context.cookies()).find(cookie => cookie.name === "__Host-thirdshift-session");
  assert.ok(session);
  assert.equal(session.httpOnly, true);
  assert.equal(session.secure, true);
  assert.equal(session.sameSite, "Strict");
  assert.ok(session.expires > Date.now() / 1000 + 29 * 86400);
  assert.equal((await context.request.post(`${origin}/sign-out`)).status(), 403);
  await Promise.all([page.waitForURL(`${origin}/console-log`), page.getByRole("link", { name: "Console log", exact: true }).click()]);
  await page.getByText(/owner@example.test · sign-in/).waitFor();
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
  await Promise.all([page.waitForURL(`${origin}/account`), page.getByRole("link", { name: "Account", exact: true }).click()]);
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
  await Promise.all([page.waitForURL(`${origin}/sign-in`), page.getByRole("button", { name: "Sign out this browser" }).click()]);
  await page.goto(origin);
  assert.equal(new URL(page.url()).pathname, "/sign-in");
  assert.ok(!(await context.cookies()).some(cookie => cookie.name === "__Host-thirdshift-session"));
});
