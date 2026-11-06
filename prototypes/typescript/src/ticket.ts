import { constants } from "node:fs";
import { open } from "node:fs/promises";
import { join } from "node:path";
import { issueFile, readIssue, record } from "./github";
import type { Issue } from "./github";
import { Claim } from "./claim";
import { Commands } from "./process";
import { createWorktree } from "./worktree";
import { pickup } from "./pickup";

export async function main(skills: [string, string][]): Promise<void> {
  const commands = new Commands();
  const launch = process.cwd();
  try {
    if (process.argv[2] === "pickup") {
      const repository = await originRepository(commands, launch);
      await pickup(commands, launch, repository, (number, issue) => runTicket(commands, launch, skills, repository, number, issue));
      return;
    }
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
    if ((await originRepository(commands, launch)).toLowerCase() !== repository.toLowerCase()) throw new Error("Origin match failed");
    const issue = readIssue(await commands.run(["gh", "api", `repos/${repository}/issues/${number}`], launch));
    await runTicket(commands, launch, skills, repository, number, issue);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  } finally {
    if (commands.interrupted) process.exitCode = 130;
    commands.close();
  }
}

async function originRepository(commands: Commands, launch: string): Promise<string> {
  const origin = await commands.run(["git", "config", "--get", "remote.origin.url"], launch);
  const match = /^(?:https:\/\/github.com\/|git@github.com:|ssh:\/\/git@github.com\/)([A-Za-z0-9._-]+\/[A-Za-z0-9._-]+?)(?:\.git)?\/?$/.exec(origin);
  if (!match?.[1]) throw new Error("Origin match failed: expected a GitHub origin");
  if (match[1].split("/").some(name => name === "." || name === ".." || name.startsWith("-"))) throw new Error("Origin match failed: invalid repository name");
  return match[1];
}

async function runTicket(commands: Commands, launch: string, skills: [string, string][], repository: string, number: string, issue: Issue): Promise<void> {
  const canonical = `https://github.com/${repository}/issues/${number}`;
  const branch = `issue-${number}`;
  const claim = new Claim(commands, launch, { repository, number, branch });
  try {
    // Pickup makes the Claim before dispatching the Ticket session. A failure
    // before this write has dispatched nothing; a write advances the generation.
    await claim.make(issue.labels);
    const comments = await commands.run(["gh", "api", `repos/${repository}/issues/${number}/comments`, "--paginate", "--slurp"], launch);
    const text = issueFile(issue, canonical, comments);
    const base = await commands.run(["git", "branch", "--show-current"], launch);
    if (!base) throw new Error("Expected a checked-out Base branch");
    const worktree = await createWorktree(commands, launch, branch, skills);
    await writeIssue(join(worktree, ".thirdshift-issue.md"), text);
    let resultSeen = false;
    await commands.run(["claude", "--print", "--verbose", "--output-format", "stream-json", "--permission-mode", "auto",
      `/thirdshift-implement ${canonical}\nThe Base branch is ${base}. Work on ${branch}. Read .thirdshift-issue.md; do not fetch issue comments yourself.`], worktree, {
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
  } finally {
    await claim.end();
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
