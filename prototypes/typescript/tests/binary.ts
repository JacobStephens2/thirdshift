import { join, resolve } from "node:path";

export const root = resolve(import.meta.dir, "..");
const binary = join(root, "build/thirdshift");

export async function buildBinary(): Promise<void> {
  const build = Bun.spawn([process.execPath, "run", "build"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  const [code, out, err] = await Promise.all([build.exited, new Response(build.stdout).text(), new Response(build.stderr).text()]);
  if (code !== 0) throw new Error(`build: ${out}\n${err}`);
}

export function startBinary(args: string[], cwd: string, env: NodeJS.ProcessEnv) {
  return Bun.spawn([binary, ...args], { cwd, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
}

export async function runBinary(args: string[], cwd: string, env: NodeJS.ProcessEnv) {
  const child = startBinary(args, cwd, env);
  const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  return { code, stdout, stderr };
}
