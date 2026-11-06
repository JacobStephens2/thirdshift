import { randomUUID } from "node:crypto";
import { hostname } from "node:os";
import type { Commands } from "./process";

export class Reservation {
  private constructor(
    private readonly commands: Commands,
    private readonly launch: string,
    private readonly reference: string,
    private readonly token: string,
  ) {}

  static async take(commands: Commands, launch: string, number: string): Promise<Reservation | undefined> {
    const reference = `refs/heads/thirdshift/pickup/${number}`;
    const tree = await commands.run(["git", "mktree"], launch);
    const message = `thirdshift Pickup reservation for #${number}\n\nToken: ${randomUUID()}\nHost: ${hostname()}\nLaunch: ${launch}\nProcess: ${process.pid}\nStarted: ${new Date().toISOString()}`;
    const token = await commands.run(["git", "-c", "commit.gpgSign=false", "commit-tree", tree, "-m", message], launch);
    try {
      await commands.run(["git", "push", "--no-verify", `--force-with-lease=${reference}:`, "origin", `${token}:${reference}`], launch);
    } catch (error) {
      // A lost response is not permission to dispatch. Only a different token
      // proves contention; our token or no ref is an uncertain remote error.
      const recovery = `check origin reservation ${reference} by hand (our token ${token})`;
      let remote: string;
      try { remote = await commands.run(["git", "ls-remote", "--refs", "origin", reference], launch, { finishing: true }); }
      catch { throw new Error(`${String(error)}; ${recovery}`); }
      const other = remote.split(/\s+/)[0];
      if (other && other !== token) return undefined;
      throw new Error(`${String(error)}; ${recovery}`);
    }
    return new Reservation(commands, launch, reference, token);
  }

  async release(): Promise<void> {
    try {
      await this.commands.run(["git", "push", "--no-verify", `--force-with-lease=${this.reference}:${this.token}`, "origin", `:${this.reference}`], this.launch, { finishing: true });
    } catch (error) {
      throw new Error(`${String(error)}; could not release ${this.reference}; check origin by hand (our token ${this.token})`);
    }
  }
}
