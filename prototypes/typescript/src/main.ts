import { main as ticket } from "./ticket";
import { Inbox } from "./console/inbox";
import { openapi } from "./console/routes";
import { serve } from "./console/server";

export async function main(skills: [string, string][], assets: { html: string; client: string }): Promise<void> {
  const args = process.argv.slice(2);
  try {
    if (args[0] === "status" && args.length === 2 && args[1] === "--json") {
      console.log(JSON.stringify(await new Inbox().status()));
    } else if (args[0] === "dismiss") {
      const id = args[1];
      if (!id || !(args.length === 3 && args[2] === "--json" || args.length === 4 && args[2] === "--undo" && args[3] === "--json")) throw new Error("Usage: dismiss <id> [--undo] --json");
      console.log(JSON.stringify(await new Inbox().dismiss({ id, undo: args.includes("--undo") })));
    } else if (args[0] === "console") {
      if (args.length === 2 && args[1] === "openapi") { console.log(JSON.stringify(openapi())); return; }
      let port = 3939;
      let scope: "read" | "act" = "act";
      for (let i = 1; i < args.length; i += 2) {
        const value = args[i + 1];
        if (args[i] === "--port" && value && /^\d+$/.test(value) && Number(value) <= 65535) port = Number(value);
        else if (args[i] === "--scope" && (value === "read" || value === "act")) scope = value;
        else throw new Error("Usage: console [--port <0..65535>] [--scope read|act]");
      }
      const server = serve(port, scope, assets);
      console.log(`http://127.0.0.1:${server.port}`);
      const stop = () => { server.stop(true); };
      process.once("SIGINT", stop);
      process.once("SIGTERM", stop);
      process.once("SIGHUP", stop);
    } else {
      await ticket(skills);
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
