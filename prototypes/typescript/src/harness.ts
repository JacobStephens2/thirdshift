import { record } from "./github";
import type { Commands } from "./process";

export async function session(commands: Commands, worktree: string, prompt: string): Promise<void> {
  let resultSeen = false;
  await commands.run(["claude", "--print", "--verbose", "--output-format", "stream-json", "--permission-mode", "auto", prompt], worktree, {
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
}
