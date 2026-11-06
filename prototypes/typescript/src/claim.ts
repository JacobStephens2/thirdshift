import { labelIs, readLabels, record } from "./github";
import type { Commands } from "./process";

type State = { kind: "unchanged" } | { kind: "changed"; addedProgress: boolean; removedReady: boolean };
function assertNever(value: never): never { throw new Error(`Unhandled Claim: ${JSON.stringify(value)}`); }

export class Claim {
  private state: State = { kind: "unchanged" };

  constructor(
    private readonly commands: Commands,
    private readonly launch: string,
    private readonly issue: { repository: string; number: string; branch: string },
  ) {}

  async make(labels: string[]): Promise<void> {
    const addedProgress = !labels.some(label => labelIs(label, "in-progress"));
    const removedReady = labels.some(label => labelIs(label, "ready-for-agent"));
    if (!addedProgress && !removedReady) return;
    if (addedProgress) {
      const existing: unknown = JSON.parse(await this.commands.run(["gh", "label", "list", "--repo", this.issue.repository,
        "--search", "in-progress", "--limit", "100", "--json", "name"], this.launch));
      if (!Array.isArray(existing)) throw new Error("Invalid repository labels response");
      if (!existing.some(label => record(label) && labelIs(label.name, "in-progress"))) {
        await this.commands.run(["gh", "label", "create", "in-progress", "--repo", this.issue.repository,
          "--color", "FBCA04", "--description", "A Ticket run has taken this issue"], this.launch);
      }
    }
    // GitHub may apply the mutation before cancellation or a transport error.
    // Ending reloads labels before deciding what is still ours to undo.
    this.state = { kind: "changed", addedProgress, removedReady };
    await this.setLabels([...labels.filter(label => !labelIs(label, "ready-for-agent") && !labelIs(label, "in-progress")), "in-progress"]);
  }

  private async setLabels(labels: string[], finishing = false): Promise<void> {
    // One GitHub PUT swaps Ready and Claim atomically, as modeled in ADR-0017.
    const args = ["gh", "api", `repos/${this.issue.repository}/issues/${this.issue.number}/labels`, "--method", "PUT"];
    if (labels.length === 0) args.push("-F", "labels[]");
    else for (const label of labels) args.push("-f", `labels[]=${label}`);
    await this.commands.run(args, this.launch, { finishing });
  }

  async end(): Promise<void> {
    const { commands, launch, issue } = this;
    // Finishing ignores recorded interruption. Closed issues lose even a
    // pre-existing Claim; open issues keep any Claim this run did not change.
    const current: unknown = JSON.parse(await commands.run(["gh", "api", `repos/${issue.repository}/issues/${issue.number}`], launch, { finishing: true }));
    if (!record(current)) throw new Error("Invalid GitHub issue response at Claim ending");
    const labels = readLabels(current.labels);
    if (!labels.some(label => labelIs(label, "in-progress"))) return;
    if (current.state === "closed") {
      await this.setLabels(labels.filter(label => !labelIs(label, "in-progress")), true);
      return;
    }
    if (current.state !== "open") throw new Error("Invalid issue state at Claim ending");
    switch (this.state.kind) {
      case "unchanged": return;
      case "changed": {
        const { addedProgress, removedReady } = this.state;
        const remote = await commands.run(["git", "ls-remote", "--heads", "origin", `refs/heads/${issue.branch}`], launch, { finishing: true });
        const prs: unknown = JSON.parse(await commands.run(["gh", "pr", "list", "--repo", issue.repository, "--head", issue.branch, "--state", "all", "--json", "number"], launch, { finishing: true }));
        if (!Array.isArray(prs)) throw new Error("Invalid pull-request response");
        if (!remote && prs.length === 0) {
          const restored = labels.filter(label => !addedProgress || !labelIs(label, "in-progress"));
          if (removedReady && !restored.some(label => labelIs(label, "ready-for-agent"))) restored.push("ready-for-agent");
          await this.setLabels(restored, true);
        }
        return;
      }
      default: assertNever(this.state);
    }
  }
}
