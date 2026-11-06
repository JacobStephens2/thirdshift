import { record } from "./github";
import type { Commands } from "./process";

type Check = { kind: "pending"; name: string } | { kind: "passed"; name: string } | { kind: "failed"; name: string };
export type CI = { kind: "absent" } | { kind: "pending" } | { kind: "passed" } | { kind: "failed"; names: string[] };

export async function checks(commands: Commands, worktree: string, repository: string, head: string): Promise<CI> {
  const checks: Check[] = [];
  for (const source of ["check-runs", "status"]) {
    const pages: unknown = JSON.parse(await commands.run(["gh", "api", `repos/${repository}/commits/${head}/${source}`, "--paginate", "--slurp"], worktree));
    if (!Array.isArray(pages)) throw new Error("Invalid CI pages");
    for (const page of pages) {
      if (!record(page)) throw new Error("Invalid CI page");
      const entries: unknown = source === "check-runs" ? page.check_runs : page.statuses;
      if (!Array.isArray(entries)) throw new Error("Invalid CI entries");
      for (const entry of entries) {
        if (!record(entry)) throw new Error("Invalid CI entry");
        const name = source === "check-runs" ? entry.name : entry.context;
        if (typeof name !== "string") throw new Error("Invalid CI name");
        if (source === "check-runs") {
          if (entry.status !== "completed") checks.push({ kind: "pending", name });
          else if (["success", "neutral", "skipped"].includes(String(entry.conclusion))) checks.push({ kind: "passed", name });
          else checks.push({ kind: "failed", name });
        } else {
          switch (entry.state) {
            case "pending": checks.push({ kind: "pending", name }); break;
            case "success": checks.push({ kind: "passed", name }); break;
            case "error": case "failure": checks.push({ kind: "failed", name }); break;
            default: throw new Error("Invalid commit status");
          }
        }
      }
    }
  }
  let pending = false;
  const failed: string[] = [];
  for (const check of checks) {
    switch (check.kind) {
      case "pending": pending = true; break;
      case "passed": break;
      case "failed": failed.push(check.name); break;
      default: check satisfies never;
    }
  }
  if (pending) return { kind: "pending" };
  if (failed.length) return { kind: "failed", names: failed };
  return checks.length ? { kind: "passed" } : { kind: "absent" };
}
