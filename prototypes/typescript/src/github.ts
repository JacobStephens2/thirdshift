export function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function trusted(association: unknown): boolean {
  return association === "OWNER" || association === "MEMBER" || association === "COLLABORATOR";
}

export interface Issue {
  title: string;
  body: string;
  state: string;
  labels: string[];
}

export function readLabels(value: unknown): string[] {
  if (!Array.isArray(value)) throw new Error("Invalid GitHub labels");
  const labels: string[] = [];
  for (const label of value) {
    if (!record(label) || typeof label.name !== "string") throw new Error("Invalid GitHub label");
    labels.push(label.name);
  }
  return labels;
}

export function readIssue(json: string): Issue {
  const value: unknown = JSON.parse(json);
  if (!record(value) || value.state !== "open" || !trusted(value.author_association)) {
    throw new Error("pre-flight: issue must be open and written by a Trusted account");
  }
  if (value.pull_request !== undefined) throw new Error("pre-flight: expected an issue, received a pull request");
  if (typeof value.title !== "string" || (value.body !== null && typeof value.body !== "string") || !Array.isArray(value.labels)) {
    throw new Error("Invalid GitHub issue response");
  }
  const labels = readLabels(value.labels);
  return { title: value.title, body: value.body ?? "", state: value.state, labels };
}

export function issueFile(issue: Issue, url: string, commentsJson: string): string {
  const pages: unknown = JSON.parse(commentsJson);
  if (!Array.isArray(pages)) throw new Error("Invalid GitHub comment pages");
  const comments: string[] = [];
  for (const page of pages) {
    if (!Array.isArray(page)) throw new Error("Invalid GitHub comment page");
    for (const comment of page) {
      if (!record(comment) || typeof comment.body !== "string") throw new Error("Invalid GitHub comment");
      comments.push(trusted(comment.author_association) ? comment.body : "[Comment left out: author is not a Trusted account.]");
    }
  }
  return `# ${issue.title}\n\n${url}\n\n${issue.body}\n\n${comments.join("\n\n")}\n`;
}
