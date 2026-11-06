import { appendFile, lstat, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import type { Commands } from "./process";

async function directory(path: string): Promise<void> {
  try { await mkdir(path); }
  catch (error) {
    if (!(error instanceof Error) || !("code" in error) || error.code !== "EEXIST") throw error;
  }
  const stat = await lstat(path);
  if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error(`Expected a real directory: ${path}`);
}

export async function createWorktree(commands: Commands, launch: string, branch: string, skills: [string, string][]): Promise<string> {
  const top = await commands.run(["git", "rev-parse", "--show-toplevel"], launch);
  const common = resolve(launch, await commands.run(["git", "rev-parse", "--git-common-dir"], launch));
  const store = join(common, "thirdshift-typescript");
  await directory(store);
  const worktrees = join(store, "worktrees");
  await directory(worktrees);
  const worktree = join(worktrees, branch);
  await commands.run(["git", "worktree", "add", "-b", branch, worktree, "HEAD"], top);
  const extracted = await mkdtemp(join(store, "skills-"));
  for (const [path, content] of skills) {
    const target = join(extracted, path);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, content);
  }
  await directory(join(worktree, ".claude"));
  const links = join(worktree, ".claude/skills");
  await directory(links);
  const names = new Set(skills.map(([path]) => path.split("/")[0]).filter((name): name is string => name !== undefined && name.startsWith("thirdshift-")));
  const exclude = join(common, "info/exclude");
  const existing = await readFile(exclude, "utf8").catch((error: unknown) => {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return "";
    throw error;
  });
  const patterns = new Set(existing.split("\n"));
  const missing = ["/.thirdshift-issue.md", ...[...names].map(name => `/.claude/skills/${name}`)].filter(pattern => !patterns.has(pattern));
  if (missing.length > 0) await appendFile(exclude, "\n" + missing.join("\n") + "\n");
  for (const name of names) await symlink(join(extracted, name), join(links, name));
  return worktree;
}
