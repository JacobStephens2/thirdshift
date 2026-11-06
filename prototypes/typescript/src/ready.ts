import { labelIs, readIssue, record, trusted } from "./github";
import type { Issue } from "./github";
import type { Commands } from "./process";

function object(value: unknown): Record<string, unknown> {
  if (!record(value)) throw new Error("Invalid Ready issue response");
  return value;
}
function timestamp(value: unknown): number {
  if (typeof value !== "string" || !Number.isFinite(Date.parse(value))) throw new Error("Invalid history timestamp");
  return Date.parse(value);
}
function pages(json: string): Record<string, unknown>[] {
  const value: unknown = JSON.parse(json);
  if (!Array.isArray(value) || !value.every(Array.isArray)) throw new Error("Invalid GitHub pages");
  return value.flat().map(object);
}

// Keep all history pagination at the GitHub seam. Separate connections need
// independent cursors; a single shared cursor would silently lose edits.
export class ReadyIssues {
  constructor(private readonly commands: Commands, private readonly launch: string, private readonly repository: string) {}

  async numbers(): Promise<string[]> {
    const candidates = pages(await this.commands.run(["gh", "api", `repos/${this.repository}/issues?state=open&labels=ready-for-agent&per_page=100`, "--paginate", "--slurp"], this.launch));
    return candidates.filter(issue => issue.pull_request === undefined).map(issue => {
      if (typeof issue.number !== "number" || !Number.isSafeInteger(issue.number) || issue.number < 1) throw new Error("Invalid issue number");
      return issue.number;
    }).sort((a, b) => a - b).map(String);
  }

  private async query(number: string, fields: string, after?: string): Promise<Record<string, unknown>> {
    const [owner, repo] = this.repository.split("/");
    if (!owner || !repo) throw new Error("Invalid repository");
    const query = `query($owner:String!,$repo:String!,$number:Int!${fields.includes("$after") ? ",$after:String" : ""}){repository(owner:$owner,name:$repo){issue(number:$number){${fields}}}}`;
    const args = ["gh", "api", "graphql", "-f", `query=${query}`, "-f", `owner=${owner}`, "-f", `repo=${repo}`, "-F", `number=${number}`];
    if (after !== undefined) args.push("-f", `after=${after}`);
    const response = object(JSON.parse(await this.commands.run(args, this.launch)));
    if (response.errors !== undefined) throw new Error("GitHub Ready query failed");
    return object(object(object(response.data).repository).issue);
  }

  private async connection(number: string, fields: string): Promise<Record<string, unknown>[]> {
    const result: Record<string, unknown>[] = [];
    let after: string | undefined;
    const cursors = new Set<string>();
    for (;;) {
      const connection = object((await this.query(number, fields, after)).edits);
      if (!Array.isArray(connection.nodes)) throw new Error("Invalid history nodes");
      result.push(...connection.nodes.map(object));
      const info = object(connection.pageInfo);
      if (info.hasNextPage === false) return result;
      if (info.hasNextPage !== true || typeof info.endCursor !== "string" || cursors.has(info.endCursor)) throw new Error("Invalid history pagination");
      after = info.endCursor;
      cursors.add(after);
    }
  }

  private async member(actor: unknown): Promise<boolean> {
    if (!record(actor) || typeof actor.login !== "string") return false;
    const login = actor.login;
    if (!/^[A-Za-z0-9-]+$/.test(login)) return false;
    const owner = this.repository.split("/")[0];
    if (login.toLowerCase() === owner?.toLowerCase()) return true;
    const membership = async (path: string) => {
      // gh reports HTTP 404 on stderr for nonmembers. Other failures stop
      // selection instead of granting trust.
      try {
        const response = await this.commands.run(["gh", "api", "--include", path], this.launch);
        if (/^HTTP\/\S+ 204\b/.test(response)) return true;
        throw new Error("Unexpected membership response");
      } catch (error) {
        if (error instanceof Error && /HTTP 404/.test(error.message)) return false;
        throw error;
      }
    };
    if (await membership(`repos/${this.repository}/collaborators/${login}`)) return true;
    const repo = object(JSON.parse(await this.commands.run(["gh", "api", `repos/${this.repository}`], this.launch)));
    return object(repo.owner).type === "Organization" && await membership(`orgs/${owner}/members/${login}`);
  }

  async read(number: string): Promise<{ issue: Issue; generation: number } | undefined> {
    const raw = object(JSON.parse(await this.commands.run(["gh", "api", `repos/${this.repository}/issues/${number}`], this.launch)));
    if (raw.state !== "open" || !trusted(raw.author_association) || raw.pull_request !== undefined) return undefined;
    const issue = readIssue(JSON.stringify(raw));
    if (!issue.labels.some(label => labelIs(label, "ready-for-agent")) || issue.labels.some(label => ["in-progress", "ready-for-human", "needs-info", "wontfix", "needs-triage", "base-fix", "security-fix"].includes(label.toLowerCase()))) return undefined;
    const events = pages(await this.commands.run(["gh", "api", `repos/${this.repository}/issues/${number}/events?per_page=100`, "--paginate", "--slurp"], this.launch));
    const labels = events.filter(event => event.event === "labeled" || event.event === "unlabeled");
    // Normalize a same-second swap, preserving order among Claim events.
    labels.sort((a, b) => timestamp(a.created_at) - timestamp(b.created_at)
      || Number(labelIs(object(a.label).name, "ready-for-agent")) - Number(labelIs(object(b.label).name, "ready-for-agent")));
    let generation = 0;
    let claimed = false;
    let lastRelease = -Infinity;
    let readyAt = -Infinity;
    let readyActor: unknown;
    for (const event of labels) {
      const name = object(event.label).name;
      const applied = event.event === "labeled";
      if (labelIs(name, "in-progress")) {
        if (applied) { generation++; claimed = true; }
        else { claimed = false; lastRelease = timestamp(event.created_at); }
      }
      if (labelIs(name, "ready-for-agent") && applied) { readyAt = timestamp(event.created_at); readyActor = event.actor; }
    }
    if (claimed || !Number.isFinite(readyAt) || !await this.member(readyActor)) return undefined;
    const body = await this.connection(number, "edits:userContentEdits(first:100,after:$after){nodes{editedAt actor:editor{login}} pageInfo{hasNextPage endCursor}}");
    const titles = await this.connection(number, "edits:timelineItems(first:100,after:$after,itemTypes:[RENAMED_TITLE_EVENT]){nodes{... on RenamedTitleEvent{createdAt actor{login}}} pageInfo{hasNextPage endCursor}}");
    for (const edit of [...body, ...titles]) {
      if (timestamp(edit.editedAt ?? edit.createdAt) >= readyAt && !await this.member(edit.actor)) return undefined;
    }
    const facts = await this.query(number, "parent{url} subIssues(first:1){nodes{state}} blockedBy(first:100){nodes{state} pageInfo{hasNextPage}}");
    // This Ticket-only slice leaves Specs to their own prototype ticket.
    const children = object(facts.subIssues).nodes;
    if (facts.parent !== null || !Array.isArray(children) || children.length !== 0) return undefined;
    const blockers = object(facts.blockedBy);
    if (!Array.isArray(blockers.nodes) || object(blockers.pageInfo).hasNextPage !== false || blockers.nodes.some(node => object(node).state !== "CLOSED")) return undefined;
    const linked = await this.connection(number, "edits:timelineItems(first:100,after:$after,itemTypes:[SUB_ISSUE_ADDED_EVENT,SUB_ISSUE_REMOVED_EVENT,BLOCKED_BY_ADDED_EVENT,BLOCKED_BY_REMOVED_EVENT]){nodes{... on SubIssueAddedEvent{createdAt} ... on SubIssueRemovedEvent{createdAt} ... on BlockedByAddedEvent{createdAt} ... on BlockedByRemovedEvent{createdAt}} pageInfo{hasNextPage endCursor}}");
    if (Math.max(readyAt, ...linked.map(event => timestamp(event.createdAt))) > Date.now() - 5 * 60_000) return undefined;
    const branch = `issue-${number}`;
    const remote = await this.commands.run(["git", "ls-remote", "--heads", "origin", `refs/heads/${branch}`], this.launch);
    const prs: unknown = JSON.parse(await this.commands.run(["gh", "pr", "list", "--repo", this.repository, "--head", branch, "--state", "all", "--json", "number"], this.launch));
    if (!Array.isArray(prs)) throw new Error("Invalid pull-request response");
    if ((remote || prs.length) && !(generation > 0 && readyAt >= lastRelease && lastRelease !== -Infinity)) return undefined;
    return { issue, generation };
  }
}
