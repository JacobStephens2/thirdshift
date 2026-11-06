import { constants } from "node:fs";
import { open } from "node:fs/promises";
import { join } from "node:path";
import { issueFile, readIssue } from "./github";
import { Claim } from "./claim";
import { Delivery } from "./delivery";
import { session } from "./harness";
import { Commands } from "./process";
import { createWorktree } from "./worktree";

type Outcome = { kind: "ready" } | { kind: "failed"; cause: string } | { kind: "interrupted" };

export async function main(skills: [string, string][]): Promise<void> {
  const commands = new Commands();
  const launch = process.cwd();
  let claim: Claim | undefined;
  let delivery: Delivery | undefined;
  let outcome: Outcome = { kind: "failed", cause: "Ticket run did not complete" };
  try {
    const url = new URL(process.argv[2] ?? "");
    const match = /^\/([^/]+)\/([^/]+)\/issues\/([1-9][0-9]*)$/.exec(url.pathname);
    const owner = match?.[1];
    const repo = match?.[2];
    const number = match?.[3];
    const name = /^[A-Za-z0-9._-]+$/;
    // Compare the whole URL, not only the path. Userinfo, a port, a query, or a
    // fragment would otherwise reach the issue file and the auto-permission prompt.
    if (url.protocol !== "https:" || url.hostname !== "github.com" || url.port !== "" || !owner || !repo || !number || !name.test(owner) || !name.test(repo) || owner === "." || owner === ".." || repo === "." || repo === ".." || owner.startsWith("-") || repo.startsWith("-") || url.href !== `https://github.com/${owner}/${repo}/issues/${number}`) throw new Error("Expected a GitHub Issue URL");
    const repository = `${owner}/${repo}`;
    const canonical = `https://github.com/${owner}/${repo}/issues/${number}`;
    const branch = `issue-${number}`;
    const origin = await commands.run(["git", "config", "--get", "remote.origin.url"], launch);
    const originMatch = /^(?:https:\/\/github.com\/|git@github.com:|ssh:\/\/git@github.com\/)([^/]+\/[^/]+?)(?:\.git)?\/?$/.exec(origin);
    if (originMatch?.[1]?.toLowerCase() !== repository.toLowerCase()) throw new Error("Origin match failed");
    const issue = readIssue(await commands.run(["gh", "api", `repos/${repository}/issues/${number}`], launch));
    const comments = await commands.run(["gh", "api", `repos/${repository}/issues/${number}/comments`, "--paginate", "--slurp"], launch);
    const text = issueFile(issue, canonical, comments);
    claim = new Claim(commands, launch, { repository, number, branch });
    await claim.make(issue.labels);
    const base = await commands.run(["git", "branch", "--show-current"], launch);
    if (!base) throw new Error("Expected a checked-out Base branch");
    const worktree = await createWorktree(commands, launch, branch, skills);
    await writeIssue(join(worktree, ".thirdshift-issue.md"), text);
    delivery = new Delivery(commands, worktree, { repository, number, branch, base, title: issue.title });
    await session(commands, worktree, `/thirdshift-implement ${canonical}\nThe Base branch is ${base}. Work on ${branch}. Read .thirdshift-issue.md; do not fetch issue comments yourself.\nCommit your work. Do not push, open a PR, change PR readiness, or merge the pull request; Delivery owns publication.`);
    await delivery.run();
    outcome = { kind: "ready" };
  } catch (error) {
    outcome = { kind: "failed", cause: error instanceof Error ? error.message : String(error) };
  } finally {
    if (commands.interrupted) outcome = { kind: "interrupted" };
    switch (outcome.kind) {
      case "ready": break;
      case "failed": console.error(outcome.cause); process.exitCode = 1; break;
      case "interrupted": console.error("Command interrupted"); process.exitCode = 130; break;
      default: outcome satisfies never;
    }
    if (outcome.kind !== "ready") {
      try { await delivery?.salvage(); }
      catch (error) { console.error(`Failed ticket run salvage failed; Worktree retained: ${error instanceof Error ? error.message : String(error)}`); }
    }
    try {
      await claim?.end();
    } catch (error) {
      console.error(`Claim ending failed: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
    if (commands.interrupted) process.exitCode = 130;
    commands.close();
  }
}

// O_NOFOLLOW applies to the final path component, so a symlink checked out at
// .thirdshift-issue.md cannot replace a file outside the worktree.
async function writeIssue(path: string, text: string): Promise<void> {
  const handle = await open(path, constants.O_WRONLY | constants.O_CREAT | constants.O_TRUNC | constants.O_NOFOLLOW, 0o644).catch((error: unknown) => {
    if (typeof error === "object" && error !== null && "code" in error && error.code === "ELOOP") {
      throw new Error("Refusing to follow a symlink for .thirdshift-issue.md");
    }
    throw error;
  });
  try {
    await handle.writeFile(text);
  } finally {
    await handle.close();
  }
}
