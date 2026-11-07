import { afterEach, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { serve } from "../src/console/server";

const token = "test-console-token";
const directories: string[] = [];
const previous = {
  inbox: process.env.THIRDSHIFT_PROTOTYPE_INBOX,
  token: process.env.THIRDSHIFT_PROTOTYPE_TOKEN,
  nodeEnv: process.env.NODE_ENV,
};

afterEach(async () => {
  restore("THIRDSHIFT_PROTOTYPE_INBOX", previous.inbox);
  restore("THIRDSHIFT_PROTOTYPE_TOKEN", previous.token);
  restore("NODE_ENV", previous.nodeEnv);
  for (const directory of directories.splice(0)) await rm(directory, { recursive: true, force: true });
});

test("an authenticated status failure stays JSON and omits the server working directory", async () => {
  const directory = await mkdtemp(join(tmpdir(), "thirdshift-ts-console-error-"));
  directories.push(directory);
  const inbox = join(directory, "inbox.json");
  await writeFile(inbox, "{not-json");
  process.env.THIRDSHIFT_PROTOTYPE_INBOX = inbox;
  process.env.THIRDSHIFT_PROTOTYPE_TOKEN = token;
  // Operator and compiled-binary runs leave NODE_ENV unset, which is when Bun
  // enables its development error page by default.
  delete process.env.NODE_ENV;

  const server = serve(0, "read", { html: "<p>page</p>", client: "export {}" });
  try {
    const url = `http://127.0.0.1:${server.port}/api/v1/status`;
    const unauthenticated = await fetch(url);
    expect(unauthenticated.status).toBe(401);
    expect(unauthenticated.headers.get("content-type")).toContain("application/json");
    const unauthenticatedBody = await unauthenticated.text();
    expect(unauthenticatedBody).not.toContain(process.cwd());
    expect(unauthenticatedBody).not.toContain(directory);

    const authenticated = await fetch(url, { headers: { Authorization: `Bearer ${token}` } });
    const body = await authenticated.text();
    expect(authenticated.status).toBe(500);
    expect(authenticated.headers.get("content-type")).toContain("application/json");
    expect(authenticated.headers.get("content-type")).not.toContain("text/html");
    expect(body).not.toContain(process.cwd());
    expect(body).not.toContain(directory);
    expect(body).not.toContain(token);
    expect(body).not.toContain("inbox.ts");
    expect(body.toLowerCase()).not.toContain("<script");
    expect(body.toLowerCase()).not.toContain("doctype");
    expect(JSON.parse(body)).toEqual({ error: "Internal Server Error" });
  } finally {
    server.stop(true);
  }
});

function restore(name: string, value: string | undefined) {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}
