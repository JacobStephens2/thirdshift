import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { issueFile, readIssue, readLabels, record } from "./github";
import { Commands } from "./process";
import { createWorktree } from "./worktree";

type Claim = { kind: "unchanged" } | { kind: "changed"; addedProgress: boolean; removedReady: boolean };
function assertNever(value: never): never { throw new Error(`Unhandled Claim: ${JSON.stringify(value)}`); }

export async function main(skills: [string, string][]): Promise<void> {
  const commands = new Commands();
  const launch = process.cwd();
  let claim: Claim = { kind: "unchanged" };
  let repository = "";
  let number = "";
  let branch = "";
  try {
    const url = new URL(process.argv[2] ?? "");
    const match = /^\/([^/]+)\/([^/]+)\/issues\/([1-9][0-9]*)$/.exec(url.pathname);
    if (url.protocol !== "https:" || url.hostname !== "github.com" || !match) throw new Error("Expected a GitHub Issue URL");
    repository = `${match[1]}/${match[2]}`;
    number = match[3] ?? "";
    branch = `issue-${number}`;
    const origin = await commands.run(["git", "config", "--get", "remote.origin.url"], launch);
    const originMatch = /^(?:https:\/\/github.com\/|git@github.com:|ssh:\/\/git@github.com\/)([^/]+\/[^/]+?)(?:\.git)?\/?$/.exec(origin);
    if (originMatch?.[1]?.toLowerCase() !== repository.toLowerCase()) throw new Error("Origin match failed");
    const issue = readIssue(await commands.run(["gh", "api", `repos/${repository}/issues/${number}`], launch));
    const comments = await commands.run(["gh", "api", `repos/${repository}/issues/${number}/comments`, "--paginate", "--slurp"], launch);
    const text = issueFile(issue, url.href, comments);
    const addedProgress = !issue.labels.includes("in-progress");
    const removedReady = issue.labels.includes("ready-for-agent");
    if (addedProgress || removedReady) {
      const args = ["gh", "issue", "edit", number, "--repo", repository];
      if (addedProgress) args.push("--add-label", "in-progress");
      if (removedReady) args.push("--remove-label", "ready-for-agent");
      // A label mutation may reach GitHub before cancellation or a transport
      // error is reported. Record the intended changes before starting it;
      // Claim ending reloads labels before deciding what is still ours to undo.
      claim = { kind: "changed", addedProgress, removedReady };
      await commands.run(args, launch);
    }
    const base = await commands.run(["git", "branch", "--show-current"], launch);
    if (!base) throw new Error("Expected a checked-out Base branch");
    const worktree = await createWorktree(commands, launch, branch, skills);
    await writeFile(join(worktree, ".thirdshift-issue.md"), text);
    let resultSeen = false;
    await commands.run(["claude", "--print", "--verbose", "--output-format", "stream-json", "--permission-mode", "auto",
      `/thirdshift-implement ${url.href}\nThe Base branch is ${base}. Work on ${branch}. Read .thirdshift-issue.md; do not fetch issue comments yourself.`], worktree, {
      line(json) {
        const event: unknown = JSON.parse(json);
        if (!record(event) || typeof event.type !== "string") throw new Error("Invalid Harness JSON event");
        if (event.type === "result") {
          resultSeen = true;
          if (event.is_error === true) throw new Error("Harness reported an error result");
        }
        console.log(JSON.stringify(event));
      },
    });
    if (!resultSeen) throw new Error("Harness ended without a result event");
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  } finally {
    try {
      switch (claim.kind) {
        case "unchanged": break;
        case "changed": {
          // Finishing commands ignore recorded interruption, so Claim ending
          // still completes after the Harness process group has been stopped.
          const current: unknown = JSON.parse(await commands.run(["gh", "api", `repos/${repository}/issues/${number}`], launch, { finishing: true }));
          if (!record(current)) throw new Error("Invalid GitHub issue response at Claim ending");
          const labels = readLabels(current.labels);
          if (!labels.includes("in-progress")) break;
          if (current.state === "closed") {
            await commands.run(["gh", "issue", "edit", number, "--repo", repository, "--remove-label", "in-progress"], launch, { finishing: true });
            break;
          }
          if (current.state !== "open") throw new Error("Invalid issue state at Claim ending");
          const remote = await commands.run(["git", "ls-remote", "--heads", "origin", `refs/heads/${branch}`], launch, { finishing: true });
          const prs: unknown = JSON.parse(await commands.run(["gh", "pr", "list", "--repo", repository, "--head", branch, "--state", "all", "--json", "number"], launch, { finishing: true }));
          if (!Array.isArray(prs)) throw new Error("Invalid pull-request response");
          if (!remote && prs.length === 0) {
            const args = ["gh", "issue", "edit", number, "--repo", repository];
            if (claim.addedProgress) args.push("--remove-label", "in-progress");
            if (claim.removedReady) args.push("--add-label", "ready-for-agent");
            await commands.run(args, launch, { finishing: true });
          }
          break;
        }
        default: assertNever(claim);
      }
    } catch (error) {
      console.error(`Claim ending failed: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
    if (commands.interrupted) process.exitCode = 130;
    commands.close();
  }
}
