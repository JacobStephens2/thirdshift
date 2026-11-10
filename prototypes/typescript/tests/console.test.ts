import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { inboxStatus, dismissResponse, type InboxStatus } from "../src/console/schema";
import { routes } from "../src/console/routes";
import { z } from "zod";
import { chromium } from "playwright";
import { buildBinary, runBinary, startBinary } from "./binary";

const directories: string[] = [];
const initial = {
  activeRuns: 1,
  rows: [
    { id: "failed-7", title: "Ticket #7 failed", group: "needs-you", active: false, dismissal: null },
    { id: "ready-8", title: "Ticket #8 ready", group: "ready-for-review", active: false, dismissal: null },
    { id: "working-9", title: "Ticket #9 working", group: "working", active: true, dismissal: null },
  ],
} satisfies InboxStatus;

beforeAll(buildBinary, 120_000);

test("every registered Console route has OpenAPI, page, CLI and scope parity", async () => {
  const f = await fixture();
  const server = await f.serve();
  try {
    const result = await f.cli("console", "openapi");
    expect(result.code).toBe(0);
    const raw: unknown = await (await server.request("/api/v1/openapi.json")).json();
    expect(JSON.parse(result.stdout)).toEqual(raw);
    const operation = z.object({
      operationId: z.string(), "x-cli": z.string(), "x-console-page": z.string(),
      "x-required-scope": z.enum(["read", "act"]).nullable(),
      responses: z.record(z.string(), z.unknown()),
    });
    const doc = z.object({
      openapi: z.literal("3.1.0"), paths: z.record(z.string(), z.record(z.string(), operation)),
      components: z.object({ schemas: z.record(z.string(), z.object({ type: z.literal("object"), required: z.array(z.string()) })) }),
    }).parse(raw);
    expect(Object.entries(doc.paths).flatMap(([path, methods]) => Object.keys(methods).map(method => `${method.toUpperCase()} ${path}`)).sort()).toEqual(routes.map(route => `${route.method} ${route.path}`).sort());
    expect(routes.map(route => route.operation).sort()).toEqual(["client", "dismiss", "openapi", "page", "status"]);
    for (const route of routes) {
      const documented = doc.paths[route.path]?.[route.method.toLowerCase()];
      expect(documented).toMatchObject({ operationId: route.operation, "x-cli": route.cli, "x-console-page": route.page, "x-required-scope": route.scope });
      const response = await server.request(route.path, route.request ? { method: route.method, headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: "failed-7", undo: true }) } : undefined);
      expect(response.status).toBe(200);
      expect(response.headers.get("Content-Type")).toContain(route.contentType);
    }
    expect(doc.components.schemas.InboxStatus?.required).toEqual(["activeRuns", "rows"]);
    expect(doc.components.schemas.DismissRequest?.required).toEqual(["id", "undo"]);
    expect(doc.components.schemas.DismissResponse).toEqual(doc.components.schemas.InboxStatus);
    expect((await server.request("/not-a-route")).status).toBe(404);
  } finally { await server.stop(); }
}, 120_000);

test("Playwright Inbox rows match status and inline Dismissal/Undo persist at desktop and mobile", async () => {
  const f = await fixture();
  const server = await f.serve();
  try {
    const browser = await chromium.launch({ headless: true });
    try {
      for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
        const page = await browser.newPage({ viewport });
        const errors: string[] = [];
        page.on("pageerror", error => errors.push(error.message));
        await page.goto(server.url);
        await page.getByLabel("Console token").fill(f.env.THIRDSHIFT_PROTOTYPE_TOKEN);
        await page.getByRole("button", { name: "Open Inbox" }).click();
        await page.getByRole("heading", { name: "Needs you", exact: true }).waitFor();
        const status = inboxStatus.parse(await (await server.request("/api/v1/status")).json());
        for (const row of status.rows) {
          const visible = page.locator(`[data-row-id="${row.id}"]`);
          expect(await visible.textContent()).toContain(row.title);
          expect(await visible.getAttribute("data-group")).toBe(row.group);
        }
        expect(await page.locator("#active-runs").textContent()).toBe("1 active run");
        const row = page.locator('[data-row-id="failed-7"]');
        await row.getByRole("button", { name: "Dismiss", exact: true }).click();
        await page.locator('[data-row-id="failed-7"][data-group="resolved"]').waitFor();
        expect(inboxStatus.parse(await (await server.request("/api/v1/status")).json()).rows[0]?.dismissal).toEqual({ from: "needs-you" });
        await page.reload();
        await row.getByRole("button", { name: "Undo", exact: true }).click();
        await page.locator('[data-row-id="failed-7"][data-group="needs-you"]').waitFor();
        expect(inboxStatus.parse(JSON.parse((await f.cli("status", "--json")).stdout))).toEqual(initial);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        expect(errors).toEqual([]);
        const capture = viewport.width === 1440 ? process.env.AFTER_DESKTOP : process.env.AFTER_MOBILE;
        if (capture) await page.screenshot({ path: capture });
        await page.close();
      }
    } finally { await browser.close(); }
  } finally { await server.stop(); }
}, 120_000);
afterAll(async () => { for (const directory of directories) await rm(directory, { recursive: true, force: true }); });

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), "thirdshift-ts-console-"));
  directories.push(directory);
  const state = join(directory, "inbox.json");
  await writeFile(state, JSON.stringify(initial));
  const env = { ...process.env, PATH: "/usr/bin:/bin", THIRDSHIFT_PROTOTYPE_INBOX: state, THIRDSHIFT_PROTOTYPE_TOKEN: "test-console-token" };
  function cli(...args: string[]) {
    return runBinary(args, directory, env);
  }
  async function serve(scope = "act") {
    const child = startBinary(["console", "--port", "0", "--scope", scope], directory, env);
    const reader = child.stdout.getReader();
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const first = await Promise.race([
        reader.read(),
        new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error("Console startup timed out")), 30_000); }),
      ]);
      const url = new TextDecoder().decode(first.value).trim();
      if (!/^http:\/\/127\.0\.0\.1:[0-9]+$/.test(url)) throw new Error(`Console did not start: ${url}; ${await new Response(child.stderr).text()}`);
      const request = (path: string, init: RequestInit = {}) => fetch(url + path, {
        ...init, headers: { Authorization: `Bearer ${env.THIRDSHIFT_PROTOTYPE_TOKEN}`, ...init.headers },
      });
      return { url, request, async stop() { child.kill(); await child.exited; reader.releaseLock(); } };
    } catch (error) {
      child.kill(); await child.exited; reader.releaseLock(); throw error;
    } finally { clearTimeout(timer); }
  }
  return { directory, env, cli, serve };
}

test("standalone status --json reads the shared Inbox payload", async () => {
  const f = await fixture();
  const result = await f.cli("status", "--json");
  expect(result.stderr).toBe("");
  expect(result.code).toBe(0);
  expect(inboxStatus.parse(JSON.parse(result.stdout))).toEqual(initial);
}, 120_000);

test("Console status and Dismissal/Undo have identical CLI twins and enforce scopes", async () => {
  const f = await fixture();
  const server = await f.serve();
  try {
    expect((await fetch(server.url + "/api/v1/status")).status).toBe(401);
    const status = inboxStatus.parse(await (await server.request("/api/v1/status")).json());
    expect(status).toEqual(initial);
    const body = { id: "failed-7", undo: false };
    const dismissed = dismissResponse.parse(await (await server.request("/api/v1/dismiss", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) })).json());
    expect(dismissed.rows[0]).toEqual({ id: "failed-7", title: "Ticket #7 failed", active: false, group: "resolved", dismissal: { from: "needs-you" } });
    expect(inboxStatus.parse(JSON.parse((await f.cli("status", "--json")).stdout))).toEqual(dismissed);
    const undo = await f.cli("dismiss", "failed-7", "--undo", "--json");
    expect(undo.code).toBe(0);
    expect(dismissResponse.parse(JSON.parse(undo.stdout))).toEqual(initial);
    expect(inboxStatus.parse(await (await server.request("/api/v1/status")).json())).toEqual(initial);
    const cliDismiss = await f.cli("dismiss", "ready-8", "--json");
    expect(cliDismiss.code).toBe(0);
    const httpUndo = await server.request("/api/v1/dismiss", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: "ready-8", undo: true }) });
    expect(dismissResponse.parse(await httpUndo.json())).toEqual(initial);
    expect((await server.request("/api/v1/dismiss", { method: "POST", headers: { "Content-Type": "application/json" }, body: '{"id":7}' })).status).toBe(400);
    expect((await server.request("/api/v1/dismiss", { method: "GET" })).status).toBe(405);
  } finally { await server.stop(); }
  const readOnly = await f.serve("read");
  try {
    expect((await readOnly.request("/api/v1/status")).status).toBe(200);
    expect((await readOnly.request("/api/v1/dismiss", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: "failed-7", undo: false }) })).status).toBe(403);
  } finally { await readOnly.stop(); }
}, 120_000);
