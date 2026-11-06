import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { issueFile, readIssue, record } from "./github";
import { Claim } from "./claim";
import { Commands } from "./process";
import { createWorktree } from "./worktree";

export async function main(skills: [string, string][]): Promise<void> {
  const commands = new Commands();
  const launch = process.cwd();
  let claim: Claim | undefined;
  try {
    const url = new URL(process.argv[2] ?? "");
    const match = /^\/([^/]+)\/([^/]+)\/issues\/([1-9][0-9]*)$/.exec(url.pathname);
    if (url.protocol !== "https:" || url.hostname !== "github.com" || !match) throw new Error("Expected a GitHub Issue URL");
    const repository = `${match[1]}/${match[2]}`;
    const number = match[3] ?? "";
    const branch = `issue-${number}`;
    const origin = await commands.run(["git", "config", "--get", "remote.origin.url"], launch);
    const originMatch = /^(?:https:\/\/github.com\/|git@github.com:|ssh:\/\/git@github.com\/)([^/]+\/[^/]+?)(?:\.git)?\/?$/.exec(origin);
    if (originMatch?.[1]?.toLowerCase() !== repository.toLowerCase()) throw new Error("Origin match failed");
    const issue = readIssue(await commands.run(["gh", "api", `repos/${repository}/issues/${number}`], launch));
    const comments = await commands.run(["gh", "api", `repos/${repository}/issues/${number}/comments`, "--paginate", "--slurp"], launch);
    const text = issueFile(issue, url.href, comments);
    claim = new Claim(commands, launch, { repository, number, branch });
    await claim.make(issue.labels);
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
      await claim?.end();
    } catch (error) {
      console.error(`Claim ending failed: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 1;
    }
    if (commands.interrupted) process.exitCode = 130;
    commands.close();
  }
}
