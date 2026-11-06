import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { homedir } from "node:os";
import { dismissRequest, inboxStatus, type DismissRequest, type InboxStatus } from "./schema";

export class Inbox {
  private pending: Promise<unknown> = Promise.resolve();
  constructor(private readonly path = process.env.THIRDSHIFT_PROTOTYPE_INBOX ?? join(homedir(), ".local/state/thirdshift-typescript/inbox.json")) {}

  async status(): Promise<InboxStatus> {
    let text: string;
    try { text = await readFile(this.path, "utf8"); }
    catch (error) {
      if (error instanceof Error && "code" in error && error.code === "ENOENT") return { activeRuns: 0, rows: [] };
      throw error;
    }
    const status = inboxStatus.parse(JSON.parse(text));
    status.activeRuns = status.rows.filter(row => row.active).length;
    return status;
  }

  dismiss(request: DismissRequest): Promise<InboxStatus> {
    // Serialize read/modify/write within this Console; atomic replacement keeps readers
    // from seeing partial JSON. This prototype does not support concurrent writers.
    const result = this.pending.then(async () => {
      const { id, undo } = dismissRequest.parse(request);
      const status = await this.status();
      const row = status.rows.find(row => row.id === id);
      if (!row) throw new Error("Unknown Inbox row");
      if (undo) {
        if (row.dismissal) { row.group = row.dismissal.from; row.dismissal = null; }
      } else if (!row.dismissal) {
        if (row.group !== "needs-you" && row.group !== "ready-for-review") throw new Error("Row is not waiting for the Day shift");
        row.dismissal = { from: row.group };
        row.group = row.active ? "working" : "resolved";
      }
      await mkdir(dirname(this.path), { recursive: true });
      const temporary = `${this.path}.${crypto.randomUUID()}.tmp`;
      await writeFile(temporary, JSON.stringify(status) + "\n", { mode: 0o600 });
      await rename(temporary, this.path);
      return status;
    });
    this.pending = result.catch(() => {});
    return result;
  }
}
