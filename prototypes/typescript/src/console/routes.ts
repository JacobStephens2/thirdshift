import { z } from "zod";
import { dismissRequest, dismissResponse, inboxStatus } from "./schema";

type Route = {
  method: "GET" | "POST";
  path: string;
  operation: "page" | "client" | "status" | "dismiss" | "openapi";
  page: "/";
  cli: string;
  scope: "read" | "act" | null;
  contentType: string;
  response?: "InboxStatus" | "DismissResponse";
  request?: "DismissRequest";
};

// Dispatch, documentation and CLI/page/scope pairing all come from this table.
export const routes: readonly Route[] = [
  { method: "GET", path: "/", operation: "page", page: "/", cli: "console", scope: null, contentType: "text/html" },
  { method: "GET", path: "/client.js", operation: "client", page: "/", cli: "console", scope: null, contentType: "text/javascript" },
  { method: "GET", path: "/api/v1/status", operation: "status", page: "/", cli: "status --json", scope: "read", contentType: "application/json", response: "InboxStatus" },
  { method: "POST", path: "/api/v1/dismiss", operation: "dismiss", page: "/", cli: "dismiss <id> [--undo] --json", scope: "act", contentType: "application/json", request: "DismissRequest", response: "DismissResponse" },
  { method: "GET", path: "/api/v1/openapi.json", operation: "openapi", page: "/", cli: "console openapi", scope: "read", contentType: "application/json" },
];

export function openapi() {
  const paths: Record<string, Record<string, unknown>> = {};
  for (const route of routes) {
    const schema = route.response ? { $ref: `#/components/schemas/${route.response}` }
      : { type: route.contentType === "application/json" ? "object" : "string" };
    paths[route.path] = {
      ...paths[route.path],
      [route.method.toLowerCase()]: {
        operationId: route.operation,
        "x-console-page": route.page,
        "x-cli": route.cli,
        "x-required-scope": route.scope,
        security: route.scope ? [{ bearer: [] }] : [],
        ...(route.request ? { requestBody: { required: true, content: { "application/json": { schema: { $ref: `#/components/schemas/${route.request}` } } } } } : {}),
        responses: {
          "200": { description: "Success", content: { [route.contentType]: { schema } } },
          ...(route.scope ? {
            "401": { description: "Missing or invalid bearer token" },
            "403": { description: "Token lacks required scope" },
          } : {}),
          ...(route.request ? { "400": { description: "Invalid JSON, Dismissal, or Inbox row" } } : {}),
        },
      },
    };
  }
  return {
    openapi: "3.1.0", info: { title: "TypeScript prototype Console", version: "1.0.0" }, paths,
    components: {
      securitySchemes: { bearer: { type: "http", scheme: "bearer" } },
      schemas: {
        InboxStatus: z.toJSONSchema(inboxStatus),
        DismissRequest: z.toJSONSchema(dismissRequest),
        DismissResponse: z.toJSONSchema(dismissResponse),
      },
    },
  };
}
