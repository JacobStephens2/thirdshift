import { expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const runner = resolve(import.meta.dir, "../benchmark.ts");

test("benchmark CLI measures a pinned fresh worktree and removes it, with tool failures propagated", async () => {
  const root = await mkdtemp(join(tmpdir(), "benchmark-test-"));
  const repo = join(root, "repo");
  const bin = join(root, "bin");
  await mkdir(join(repo, "prototypes/typescript/src"), { recursive: true });
  await mkdir(join(repo, "src"));
  await mkdir(join(repo, "tests"));
  await mkdir(join(repo, "prototypes/typescript/tests"));
  await mkdir(bin);
  const git = (...args: string[]) => {
    const result = Bun.spawnSync(["git", ...args], { cwd: repo });
    if (result.exitCode) throw new Error(result.stderr.toString());
    return result.stdout.toString().trim();
  };
  try {
    await writeFile(join(repo, "prototypes/typescript/src/main.ts"), "// example\nexport {};\n");
    await writeFile(join(repo, "src/main.rs"), "fn main() {}\n");
    await writeFile(join(repo, "tests/pickup_model.rs"), "// model\n");
    await writeFile(join(repo, "prototypes/typescript/tests/pickup-model.test.ts"), "// model\n");
    git("init", "-q");
    git("add", ".");
    git("-c", "user.name=Benchmark", "-c", "user.email=benchmark@example.com", "commit", "-qm", "fixture");
    // Only the external toolchain is substituted; Git worktree creation,
    // measurement, source inventory, JSON publication and cleanup are real.
    const tool = `#!/bin/sh
set -eu
if [ "\${1:-}" = --version ]; then echo fixture-1; exit; fi
case "$*" in *install*|fetch*) exit;; esac
if [ -n "\${FAIL_BENCHMARK:-}" ]; then echo compiler-failed >&2; exit 23; fi
case "$PWD" in */worktree|*/worktree/prototypes/typescript) ;; *) echo not-a-worktree >&2; exit 24;; esac
case "$*" in
  *install*) exit;;
  *build.ts*) mkdir -p build; printf binary > build/thirdshift;;
  *'build --release'*) mkdir -p "$CARGO_TARGET_DIR/release"; printf rust-binary > "$CARGO_TARGET_DIR/release/thirdshift";;
esac
echo measured
`;
    for (const name of ["bun", "cargo", "rustc"]) await writeFile(join(bin, name), tool, { mode: 0o755 });
    const output = join(root, "results.json");
    const run = (extraEnv = {}) => Bun.spawnSync([process.execPath, runner, "--ref", "HEAD", "--output", output], {
      cwd: repo, env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, ...extraEnv },
    });
    const result = run();
    expect(result.stderr.toString()).toBe("");
    expect(result.exitCode).toBe(0);
    const report = JSON.parse(await readFile(output, "utf8"));
    expect(report.commit).toBe(git("rev-parse", "HEAD"));
    expect(report.measurements).toHaveLength(16);
    expect(report.measurements.every((row: { seconds: number; exitCode: number }) => row.seconds >= 0 && row.exitCode === 0)).toBe(true);
    expect(report.binaryBytes).toEqual({ typescript: 6, rust: 11 });
    expect(report.sources.typescript.production.lines).toBe(2);
    expect(report.sources.rust.production.lines).toBe(1);
    expect(report.sources.rust.model.lines).toBe(1);
    expect(git("worktree", "list", "--porcelain").match(/^worktree /gm)).toHaveLength(1);
    await rm(output);
    const failed = run({ FAIL_BENCHMARK: "1" });
    expect(failed.exitCode).not.toBe(0);
    expect(failed.stderr.toString()).toContain("compiler-failed");
    expect(await readFile(join(output + ".logs", "typescript-typecheck-cold.log"), "utf8")).toContain("compiler-failed");
    expect(await Bun.file(output).exists()).toBe(false);
    expect(git("worktree", "list", "--porcelain").match(/^worktree /gm)).toHaveLength(1);
  } finally { await rm(root, { recursive: true, force: true }); }
}, 120_000);

test("empirical research report covers measured commands, sizes, scope and architectural tradeoffs", async () => {
  const docs = resolve(import.meta.dir, "../../../docs/research");
  const report = await readFile(join(docs, "typescript-vs-rust.md"), "utf8");
  const results = JSON.parse(await readFile(join(docs, "typescript-vs-rust.json"), "utf8"));
  for (const topic of ["Reproduction", "Measured results", "Source size", "Subprocess and signal", "Exhaustiveness", "Console velocity", "Model checking", "Limitations", "ADR-0001"]) {
    expect(report).toContain(topic);
  }
  expect(results.commit).toMatch(/^[a-f0-9]{40}$/);
  for (const language of ["typescript", "rust"]) {
    for (const operation of ["typecheck", "test", "build", "model"]) {
      for (const cache of ["cold", "incremental"]) {
        const matches = results.measurements.filter((row: { language: string; operation: string; cache: string }) =>
          row.language === language && row.operation === operation && row.cache === cache);
        expect(matches).toHaveLength(1);
        const row = matches[0];
        expect(row.exitCode).toBe(0);
        expect(Number.isFinite(row.seconds) && row.seconds > 0).toBe(true);
        expect(row.commands.length).toBeGreaterThan(0);
        expect(row.commands.every((args: string[]) => args.length > 1)).toBe(true);
        expect(report).toContain(row.seconds.toFixed(3));
      }
    }
    expect(report).toContain(String(results.binaryBytes[language]));
    expect(report).toContain(String(results.sources[language].production.lines));
  }
});
