import { record } from "./github";
import type { Commands } from "./process";
import { checks } from "./ci";
import { session } from "./harness";
import { resolve } from "node:path";

interface Ticket {
  repository: string;
  number: string;
  branch: string;
  base: string;
  title: string;
}
interface PullRequest {
  number: number;
  head: string;
  draft: boolean;
  mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN";
}
type Repair = { kind: "ci"; names: string[] } | { kind: "conflict" };
type State = { kind: "publish" } | { kind: "sync"; pr: PullRequest }
  | { kind: "watch"; pr: PullRequest; head: string; started: number }
  | { kind: "repair"; pr: PullRequest; cause: Repair }
  | { kind: "ready"; pr: PullRequest };

export class Delivery {
  private finishing = false;
  constructor(private readonly commands: Commands, private readonly worktree: string, private readonly ticket: Ticket) {}

  private runCommand(args: string[]): Promise<string> {
    return this.commands.run(args, this.worktree, { finishing: this.finishing });
  }

  async salvage(): Promise<void> {
    this.finishing = true;
    try {
      const mergeHead = await this.runCommand(["git", "rev-parse", "--git-path", "MERGE_HEAD"]);
      if (await Bun.file(resolve(this.worktree, mergeHead)).exists()) await this.runCommand(["git", "merge", "--abort"]);
      const ahead = await this.runCommand(["git", "rev-list", "--count", `refs/heads/${this.ticket.base}..HEAD`]);
      try {
        if (ahead !== "0") {
          await this.push();
          await this.ensurePR();
        }
      } finally {
        // A publication error must still leave an existing PR as a draft.
        const pr = await this.findPR();
        if (pr && !pr.draft) await this.runCommand(["gh", "pr", "ready", String(pr.number), "--repo", this.ticket.repository, "--undo"]);
      }
    } finally {
      this.finishing = false;
    }
  }

  async run(): Promise<void> {
    let state: State = { kind: "publish" };
    let repairs = 0;
    let rounds = 0;
    const deadline = performance.now() + 10 * 60_000;
    for (;;) {
      if (performance.now() > deadline) throw new Error("Delivery timed out waiting for CI or mergeability");
      switch (state.kind) {
        case "publish": {
          await this.runCommand(["git", "fetch", "origin", this.ticket.base]);
          const ahead = await this.runCommand(["git", "rev-list", "--count", `origin/${this.ticket.base}..HEAD`]);
          if (ahead === "0") throw new Error("Harness left no commits ahead of the Base branch");
          await this.push();
          state = { kind: "sync", pr: await this.ensurePR() };
          break;
        }
        case "sync": {
          if (++rounds > 10) throw new Error("Base branch update limit reached");
          await this.view(state.pr.number);
          await this.runCommand(["git", "fetch", "origin", this.ticket.base]);
          try {
            await this.runCommand(["git", "merge", "--no-edit", `origin/${this.ticket.base}`]);
          } catch (error) {
            const conflicts = await this.runCommand(["git", "diff", "--name-only", "--diff-filter=U"]);
            if (!conflicts) throw error;
            state = { kind: "repair", pr: state.pr, cause: { kind: "conflict" } };
            break;
          }
          await this.push();
          const head = await this.runCommand(["git", "rev-parse", "HEAD"]);
          state = { kind: "watch", pr: state.pr, head, started: performance.now() };
          break;
        }
        case "watch": {
          const watched = await this.view(state.pr.number);
          if (watched.head !== state.head) throw new Error("Pull request head changed during CI watch");
          const ci = await checks(this.commands, this.worktree, this.ticket.repository, state.head);
          switch (ci.kind) {
            case "pending": await Bun.sleep(1000); break;
            case "failed": state = { kind: "repair", pr: state.pr, cause: { kind: "ci", names: ci.names } }; break;
            case "absent": case "passed": {
              // GitHub can expose the PR before checks have registered on a new head.
              if (ci.kind === "absent" && performance.now() - state.started < 3000) { await Bun.sleep(1000); break; }
              const pr = await this.view(state.pr.number);
              if (pr.head !== state.head) throw new Error("Pull request head changed during CI watch");
              if (pr.mergeable === "UNKNOWN") { await Bun.sleep(1000); break; }
              if (pr.mergeable === "CONFLICTING") { state = { kind: "sync", pr }; break; }
              state = { kind: "ready", pr };
              break;
            }
            default: ci satisfies never;
          }
          break;
        }
        case "repair": {
          await this.view(state.pr.number);
          if (repairs >= 5) throw new Error("Repair limit reached (5)");
          let cause: string;
          switch (state.cause.kind) {
            case "ci": cause = `CI red on ${state.cause.names.join(", ")}`; break;
            case "conflict": cause = `Merge conflict with Base branch ${this.ticket.base}`; break;
            default: state.cause satisfies never; throw new Error("Unhandled Repair");
          }
          console.error(`${cause}; starting Repair ${++repairs} of 5`);
          await session(this.commands, this.worktree, `Repair https://github.com/${this.ticket.repository}/pull/${state.pr.number}\n${cause}.\nWork on ${this.ticket.branch}, Base branch ${this.ticket.base}. Resolve and commit the fix. Read .thirdshift-issue.md; do not fetch issue comments. Do not push, change PR readiness, or merge the pull request; Delivery owns publication.`);
          const dirty = await this.runCommand(["git", "status", "--porcelain"]);
          if (dirty) throw new Error("Repair left uncommitted changes");
          state = { kind: "sync", pr: state.pr };
          break;
        }
        case "ready":
          {
            const pr = await this.view(state.pr.number);
            if (pr.head !== state.pr.head || pr.mergeable !== "MERGEABLE") throw new Error("Pull request changed before readiness");
            if (pr.draft) await this.runCommand(["gh", "pr", "ready", String(pr.number), "--repo", this.ticket.repository]);
          }
          return;
        default: state satisfies never;
      }
    }
  }

  private async push(): Promise<void> {
    await this.findPR();
    const branch = await this.runCommand(["git", "branch", "--show-current"]);
    if (branch !== this.ticket.branch) throw new Error("Harness changed the Issue branch");
    await this.runCommand(["git", "push", "origin", `HEAD:refs/heads/${this.ticket.branch}`]);
  }

  private async ensurePR(): Promise<PullRequest> {
    const existing = await this.findPR();
    if (existing) return existing;
    const { repository, branch, base, title, number } = this.ticket;
    await this.runCommand(["gh", "pr", "create", "--repo", repository, "--head", branch, "--base", base, "--draft", "--title", title, "--body", `Closes #${number}`]);
    const created = await this.findPR();
    if (!created) throw new Error("Created pull request was not found");
    return created;
  }

  private async findPR(): Promise<PullRequest | undefined> {
    const { repository, branch } = this.ticket;
    const prs: unknown = JSON.parse(await this.runCommand(["gh", "pr", "list", "--repo", repository, "--head", branch, "--state", "all", "--json", "number"]));
    if (!Array.isArray(prs) || prs.length > 1) throw new Error("Ambiguous pull request for Issue branch");
    if (prs.length === 0) return undefined;
    const pr: unknown = prs[0];
    if (!record(pr) || typeof pr.number !== "number") throw new Error("Invalid pull request number");
    return this.view(pr.number);
  }

  private async view(number: number): Promise<PullRequest> {
    const value: unknown = JSON.parse(await this.runCommand(["gh", "pr", "view", String(number), "--repo", this.ticket.repository,
      "--json", "number,state,headRefName,baseRefName,headRefOid,isDraft,mergeable"]));
    if (!record(value) || value.number !== number || value.state !== "OPEN" || value.headRefName !== this.ticket.branch || value.baseRefName !== this.ticket.base) throw new Error("Pull request is no longer open with the expected Issue branch and Base branch");
    if (typeof value.headRefOid !== "string" || typeof value.isDraft !== "boolean" || (value.mergeable !== "MERGEABLE" && value.mergeable !== "CONFLICTING" && value.mergeable !== "UNKNOWN")) throw new Error("Invalid pull request response");
    return { number, head: value.headRefOid, draft: value.isDraft, mergeable: value.mergeable };
  }
}
