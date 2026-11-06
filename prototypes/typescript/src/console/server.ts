import { timingSafeEqual } from "node:crypto";
import { Inbox } from "./inbox";
import { dismissRequest } from "./schema";
import { openapi, routes } from "./routes";

export function serve(port: number, scope: "read" | "act", assets: { html: string; client: string }) {
  const token = process.env.THIRDSHIFT_PROTOTYPE_TOKEN;
  if (!token) throw new Error("Set THIRDSHIFT_PROTOTYPE_TOKEN before starting the prototype Console");
  const expected = Buffer.from(`Bearer ${token}`);
  const inbox = new Inbox();
  return Bun.serve({
    hostname: "127.0.0.1", port,
    async fetch(request) {
      const url = new URL(request.url);
      const route = routes.find(route => route.path === url.pathname && route.method === request.method);
      if (!route) return new Response("Route not found", { status: routes.some(route => route.path === url.pathname) ? 405 : 404 });
      if (route.scope) {
        const supplied = Buffer.from(request.headers.get("Authorization") ?? "");
        if (supplied.length !== expected.length || !timingSafeEqual(supplied, expected)) return Response.json({ error: "Bearer token required" }, { status: 401 });
        if (route.scope === "act" && scope !== "act") return Response.json({ error: "act scope required" }, { status: 403 });
      }
      switch (route.operation) {
        case "page": return new Response(assets.html, { headers: { "Content-Type": "text/html; charset=utf-8", "Content-Security-Policy": "default-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'" } });
        case "client": return new Response(assets.client, { headers: { "Content-Type": "text/javascript; charset=utf-8" } });
        case "openapi": return Response.json(openapi());
        case "status": return Response.json(await inbox.status());
        case "dismiss": {
          if (request.headers.get("Content-Type")?.split(";")[0] !== "application/json") return Response.json({ error: "Expected application/json" }, { status: 400 });
          try { return Response.json(await inbox.dismiss(dismissRequest.parse(await request.json()))); }
          catch (error) { return Response.json({ error: error instanceof Error ? error.message : String(error) }, { status: 400 }); }
        }
        default: route.operation satisfies never; throw new Error("Unhandled Console operation");
      }
    },
  });
}
