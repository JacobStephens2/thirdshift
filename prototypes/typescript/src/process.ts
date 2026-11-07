import { spawn } from "node:child_process";
import { StringDecoder } from "node:string_decoder";

const graceMs = 1_000;
function signalGroup(pid: number, signal: NodeJS.Signals): void {
  try { process.kill(-pid, signal); }
  catch (error) {
    if (!(error instanceof Error) || !("code" in error) || error.code !== "ESRCH") throw error;
  }
}

async function stopGroup(pid: number): Promise<void> {
  // Keep escalating even when the group leader has exited: descendants can
  // ignore signals and keep the session's output descriptors open.
  signalGroup(pid, "SIGINT");
  await Bun.sleep(graceMs);
  signalGroup(pid, "SIGTERM");
  await Bun.sleep(graceMs);
  signalGroup(pid, "SIGKILL");
}

export class Commands {
  interrupted = false;
  private stop: (() => void) | undefined;
  private readonly interrupt = () => { this.interrupted = true; this.stop?.(); };

  constructor() {
    process.on("SIGINT", this.interrupt);
    process.on("SIGTERM", this.interrupt);
    process.on("SIGHUP", this.interrupt);
  }

  close(): void {
    process.off("SIGINT", this.interrupt);
    process.off("SIGTERM", this.interrupt);
    process.off("SIGHUP", this.interrupt);
  }

  async run(args: string[], cwd: string, options: {
    finishing?: boolean;
    line?: (line: string) => void;
  } = {}): Promise<string> {
    if (this.interrupted && !options.finishing) throw new Error("Command interrupted");
    const executable = args[0];
    if (!executable) throw new Error("Empty command");
    const child = spawn(executable, args.slice(1), { cwd, detached: true, stdio: ["ignore", "pipe", "pipe"] });
    let stopping: Promise<void> | undefined;
    let streamError: unknown;
    const stop = () => {
      if (child.pid !== undefined && !stopping) stopping = stopGroup(child.pid);
    };
    if (!options.finishing) this.stop = stop;
    let output = "";
    let pending = "";
    let stderr = "";
    const decoder = new StringDecoder("utf8");
    const line = options.line;
    const consume = (chunk: string) => {
      if (!line) { output += chunk; return; }
      pending += chunk;
      for (;;) {
        const end = pending.indexOf("\n");
        if (end < 0) break;
        const value = pending.slice(0, end);
        pending = pending.slice(end + 1);
        try { if (value.trim()) line(value); }
        catch (error) { streamError = error; stop(); }
      }
    };
    child.stdout.on("data", (chunk: Buffer) => consume(decoder.write(chunk)));
    child.stderr.on("data", (chunk: Buffer) => { stderr = (stderr + chunk.toString()).slice(-65_536); });
    try {
      const code = await new Promise<number | null>((resolve, reject) => {
        child.once("error", reject);
        child.once("close", resolve);
      });
      consume(decoder.end());
      if (line && pending.trim() && !streamError) {
        try { line(pending); } catch (error) { streamError = error; stop(); }
      }
      await stopping;
      if (streamError) throw streamError;
      if (this.interrupted && !options.finishing) throw new Error("Command interrupted");
      if (code !== 0) throw new Error(`${args.join(" ")} exited ${code}: ${stderr}`);
      return output.trim();
    } finally {
      if (this.stop === stop) this.stop = undefined;
      await stopping;
    }
  }
}
