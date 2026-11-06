const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { spawn, execFileSync } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, mkdir, readFile, writeFile, rm } = require("node:fs/promises");
const { tmpdir, hostname } = require("node:os");
const { join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { chromium } = require("./playwright.cjs");
const { createServer } = require("node:http");

let home, server, exited, browser, origin, releases;
before(async () => {
  releases = createServer((request, response) => {
    response.setHeader("Content-Type", "application/json");
    response.end(JSON.stringify({ tag_name: "v99.0.0", name: "v99.0.0", url: "http://localhost/release", prerelease: false,
      assets: [{ name: "thirdshift-installer.sh", url: "http://localhost/installer", browser_download_url: "http://localhost/installer" }] }));
  });
  releases.listen(0, "127.0.0.1");
  await once(releases, "listening");
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
    env: { ...process.env, HOME: home, THIRDSHIFT_INSTALLER_GHE_BASE_URL: `http://127.0.0.1:${releases.address().port}/` }, stdio: ["ignore", "pipe", "inherit"]
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
  if (releases) await new Promise(resolve => releases.close(resolve));
  if (home) await rm(home, { recursive: true, force: true });
});

test("Update shows versions only for admin and Theme import controls follow admin", async t => {
  const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift");
  const scopes = value => execFileSync(binary, ["console", "scopes", "--scopes", value], { env: { ...process.env, HOME: home } });
  t.after(() => scopes("read,security,act,run,admin"));
  for (const scope of ["read,act", "read,act,admin"]) {
    scopes(scope);
    for (const viewport of [{width:1440,height:900}, {width:390,height:844}]) {
      const context = await browser.newContext({viewport});
      try {
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", error => errors.push(error.message));
        await page.goto(`${origin}/sign-in`);
        await page.getByLabel("Email").fill("owner@example.test");
        await page.getByLabel("Password", {exact:true}).fill("browser test password");
        await Promise.all([page.waitForURL(`${origin}/`), page.getByRole("button", {name:"Sign in",exact:true}).click()]);
        const admin = scope.includes("admin");
        assert.equal(await page.getByRole("link", {name:"Update thirdshift",exact:true}).count(), admin ? 1 : 0);
        await page.locator("[data-open-themes]").click();
        assert.equal(await page.locator("#edit-theme").count(), admin ? 1 : 0);
        assert.equal(await page.locator("#save-theme").count(), admin ? 1 : 0);
        await page.locator("#export-theme").click();
        await page.waitForFunction(() => document.querySelector("#theme-document").value.length > 0);
        const response = await page.goto(`${origin}/update`);
        assert.equal(response.status(), admin ? 200 : 403);
        if (admin) {
          await page.getByText("Latest stable version: 99.0.0", {exact:true}).waitFor();
          assert.match(await page.locator("main").textContent(), /Current version:/);
          await page.getByRole("button", {name:"Update thirdshift",exact:true}).waitFor();
          assert.equal((await context.request.post(`${origin}/update`)).status(), 403, "CSRF is required");
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
        }
        assert.deepEqual(errors, []);
      } finally { await context.close(); }
    }
  }
});

test("User config is editable for admin sessions and read-only for read sessions", async t => {
  const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift");
  const scopes = value => execFileSync(binary, ["console", "scopes", "--scopes", value], { env: { ...process.env, HOME: home } });
  t.after(() => scopes("read,security,act,run,admin"));
  for (const scope of ["read", "admin"]) {
    scopes(scope);
    for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport, javaScriptEnabled: false });
      try {
        const page = await context.newPage();
        await page.goto(`${origin}/sign-in`);
        await page.getByLabel("Email").fill("owner@example.test");
        await page.getByLabel("Password", { exact: true }).fill("browser test password");
        await Promise.all([page.waitForURL(`${origin}/`), page.getByRole("button", { name: "Sign in", exact: true }).click()]);
        const response = await page.goto(`${origin}/config`);
        assert.equal(response.status(), 200);
        await page.getByRole("heading", { name: "User config", exact: true }).waitFor();
        await page.getByRole("link", { name: "User config", exact: true }).waitFor();
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
        const setting = page.locator("section").filter({ has: page.getByRole("heading", { name: "spec.parallel", exact: true }) });
        await setting.getByText("Default: 3", { exact: true }).waitFor();
        if (scope === "read") {
          assert.equal(await page.locator('main textarea, main button[type="submit"]').count(), 0);
          await setting.getByText("Value: 3", { exact: true }).waitFor();
        } else {
          const field = page.getByLabel("Value for spec.parallel", { exact: true });
          await field.fill("6");
          await Promise.all([page.waitForURL(`${origin}/config`), page.getByRole("button", { name: "Save spec.parallel", exact: true }).click()]);
          assert.equal(await page.getByLabel("Value for spec.parallel", { exact: true }).inputValue(), "6");
          const saved = JSON.parse(execFileSync(binary, ["config", "spec.parallel", "--json"], { env: { ...process.env, HOME: home } }));
          assert.equal(saved.value, 6);
        }
        assert.equal(await page.locator('form[action="/config/console.allowed_emails"], form[action="/config/console.address_scopes"]').count(), 0);
      } finally { await context.close(); }
    }
  }
});

test("sign-in and sign-out work on a phone without JavaScript using secure cookies", async t => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 390, height: 844 }, colorScheme: "light" });
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(origin);
  assert.equal(new URL(page.url()).pathname, "/sign-in");
  assert.equal(await page.title(), `Sign in · ${hostname()} · thirdshift`);
  await page.goto(`${origin}/set-password`);
  assert.equal(await page.title(), `Set your password · ${hostname()} · thirdshift`);
  await page.goto(`${origin}/sign-in`);
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
  await page.getByText(/owner@example.test · sign-in/).first().waitFor();
  assert.equal(await page.title(), `Console log · ${hostname()} · thirdshift`);
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
  await Promise.all([page.waitForURL(`${origin}/account`), page.getByRole("link", { name: "Account", exact: true }).click()]);
  assert.equal(await page.title(), `Account · ${hostname()} · thirdshift`);
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(18, 18, 20)");
  await Promise.all([page.waitForURL(`${origin}/sign-in`), page.getByRole("button", { name: "Sign out this browser" }).click()]);
  await page.goto(origin);
  assert.equal(new URL(page.url()).pathname, "/sign-in");
  assert.ok(!(await context.cookies()).some(cookie => cookie.name === "__Host-thirdshift-session"));
});

test("read-only sessions hide actions on desktop and phone with and without JavaScript", async t => {
  const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, "../../target/debug/thirdshift");
  const scopes = value => execFileSync(binary, ["console", "scopes", "--scopes", value], { env: { ...process.env, HOME: home } });
  scopes("read");
  t.after(() => scopes("read,security,act,run,admin"));
  const folder = join(home, ".thirdshift/logs/acme/widgets/commands/issue");
  await mkdir(folder, { recursive: true });
  const stat = await readFile(`/proc/${process.pid}/stat`, "utf8");
  const ticks = stat.slice(stat.lastIndexOf(") ") + 2).trim().split(/\s+/)[19];
  const identity = `${(await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim()}:${ticks}`;
  for (const number of [7, 8]) {
    await writeFile(join(folder, `${number}-fixture.command.json`), JSON.stringify({
      version: 1, kind: "Run", name: `acme/widgets#${number}`, repository: "acme/widgets", hostname: "fixture",
      issue: { number, url: `https://github.com/acme/widgets/issues/${number}`, title: "Check address scopes" },
      process_id: number === 7 ? process.pid : 0, process_start: number === 7 ? identity : "gone", harness: "codex",
      started_at: new Date().toISOString(), ended_at: number === 7 ? null : new Date().toISOString(),
      stages: [], outcome: number === 7 ? null : "failed"
    }));
    await writeFile(join(folder, `${number}-fixture.log`), "Checking Console access\n");
  }
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    for (const javaScriptEnabled of [false, true]) {
      const context = await browser.newContext({ viewport, javaScriptEnabled, reducedMotion: "reduce" });
      try {
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", error => errors.push(error.message));
        await page.goto(origin);
        await page.getByLabel("Email").fill("owner@example.test");
        await page.getByLabel("Password", { exact: true }).fill("browser test password");
        await Promise.all([page.waitForURL(`${origin}/`), page.getByRole("button", { name: "Sign in", exact: true }).click()]);
        await page.getByRole("heading", { name: "Inbox", exact: true }).waitFor();
        await page.getByRole("button", { name: "Dismiss", exact: true }).waitFor({ state: "hidden" });
        if (javaScriptEnabled) {
          await page.locator('.command[data-live="true"] .state').first().click();
          await page.locator("#peek-content h2").waitFor();
          await page.getByRole("button", { name: "Stop", exact: true }).waitFor({ state: "hidden" });
          await page.keyboard.press("Escape");
          await page.keyboard.press("Shift+T");
          await page.locator("#themes[open]").waitFor({ state: "hidden" });
        } else {
          await page.goto(`${origin}/peek?command=widgets%237`);
          await page.getByRole("heading", { name: /acme\/widgets#7/ }).waitFor();
          await page.getByRole("button", { name: "Stop", exact: true }).waitFor({ state: "hidden" });
        }
        for (const route of ["/", "/lanes", "/stages", "/logs", "/resources", "/experiments", "/scores", "/bug-origins", "/console-log"]) {
          assert.equal((await page.goto(`${origin}${route}`)).status(), 200, route);
          await page.locator("main").waitFor();
          await page.locator('button[data-open-themes], button[data-open-sections], form[action="/scores/mine"], form[action="/bug-origins/settle"]').waitFor({ state: "hidden" });
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, route);
        }
        assert.deepEqual(errors, []);
      } finally { await context.close(); }
    }
  }
});
