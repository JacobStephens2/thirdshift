import type { Issue } from "./github";
import type { Commands } from "./process";
import { ReadyIssues } from "./ready";
import { Reservation } from "./reservation";

type Attempt = { kind: "unready" } | { kind: "busy" } | { kind: "stale" } | { kind: "dispatched" };
function assertNever(value: never): never { throw new Error(`Unhandled Pickup outcome: ${JSON.stringify(value)}`); }

export async function pickup(commands: Commands, launch: string, repository: string, dispatch: (number: string, issue: Issue) => Promise<void>): Promise<void> {
  const ready = new ReadyIssues(commands, launch, repository);
  for (const number of await ready.numbers()) {
    const outcome = await attempt(number);
    switch (outcome.kind) {
      case "unready": case "busy": case "stale": break;
      case "dispatched": return;
      default: assertNever(outcome);
    }
  }
  console.log("Pickup skipped: no reservable Ready issue");

  async function attempt(number: string): Promise<Attempt> {
    const selected = await ready.read(number);
    if (!selected) return { kind: "unready" };
    const reservation = await Reservation.take(commands, launch, number);
    if (!reservation) return { kind: "busy" };
    try {
      const refreshed = await ready.read(number);
      if (!refreshed || refreshed.generation !== selected.generation) return { kind: "stale" };
      await dispatch(number, refreshed.issue);
      return { kind: "dispatched" };
    } finally { await reservation.release(); }
  }
}
