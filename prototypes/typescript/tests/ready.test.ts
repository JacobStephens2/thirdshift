import { expect, test } from "bun:test";
import { Commands } from "../src/process";
import { ReadyIssues } from "../src/ready";

test("Pickup rejects an active Claim whose GitHub label uses another case", async () => {
  class FakeCommands extends Commands {
    override async run(args: string[]): Promise<string> {
      if (args[0] === "git") return "";
      if (args[1] === "pr") return "[]";
      if (args[2] === "repos/test/project/issues/7") return JSON.stringify({
        title: "Already claimed", body: "", state: "open", author_association: "OWNER",
        labels: [{ name: "ready-for-agent" }, { name: "In-Progress" }],
      });
      if (args[2]?.includes("/events?")) return JSON.stringify([[
        { event: "labeled", label: { name: "ready-for-agent" }, created_at: "2026-01-01T00:00:00Z", actor: { login: "test" } },
        { event: "labeled", label: { name: "In-Progress" }, created_at: "2026-01-01T00:00:01Z", actor: { login: "test" } },
      ]]);
      if (args[2] === "graphql") {
        const connection = { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } };
        return JSON.stringify({ data: { repository: { issue: {
          edits: connection, parent: null, subIssues: connection, blockedBy: connection,
        } } } });
      }
      throw new Error(`Unexpected request: ${JSON.stringify(args)}`);
    }
  }
  const commands = new FakeCommands();
  try {
    expect(await new ReadyIssues(commands, ".", "test/project").read("7")).toBeUndefined();
  } finally { commands.close(); }
});
