import type { Issue } from "./github";
import type { Commands } from "./process";
import { ReadyIssues } from "./ready";
import { Reservation } from "./reservation";

export async function pickup(commands: Commands, launch: string, repository: string, dispatch: (number: string, issue: Issue) => Promise<void>): Promise<void> {
  const ready = new ReadyIssues(commands, launch, repository);
  for (const number of await ready.numbers()) {
    const selected = await ready.read(number);
    if (!selected) continue;
    const reservation = await Reservation.take(commands, launch, number);
    if (!reservation) continue;
    try {
      const refreshed = await ready.read(number);
      if (!refreshed || refreshed.generation !== selected.generation) continue;
      await dispatch(number, refreshed.issue);
      return;
    } finally { await reservation.release(); }
  }
  console.log("Pickup skipped: no reservable Ready issue");
}
