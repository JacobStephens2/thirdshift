import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { cpus, platform, release, tmpdir, totalmem } from "node:os";
import { dirname, join, resolve } from "node:path";

// This CLI benchmarks a committed snapshot, never the caller's dirty files.
const options = { ref: "HEAD", output: resolve("typescript-benchmark.json") };
for (let index = 2; index < process.argv.length; index += 2) {
  const flag = process.argv[index];
  const value = process.argv[index + 1];
  if (!value || (flag !== "--ref" && flag !== "--output")) {
    throw new Error("Usage: bun benchmark.ts [--ref COMMIT] [--output results.json]");
  }
  if (flag === "--ref") options.ref = value;
  else options.output = resolve(value);
}

async function command(args: string[], cwd: string, env = process.env): Promise<string> {
  const child = Bun.spawn(args, { cwd, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
  ]);
  if (exitCode !== 0) throw new Error(`${args.join(" ")} exited ${exitCode}\n${stdout}\n${stderr}`);
  return stdout + stderr;
}

async function inventory(root: string, paths: string[], extensions: string[]) {
  const files: { path: string; lines: number }[] = [];
  async function visit(path: string): Promise<void> {
    const absolute = join(root, path);
    const info = await stat(absolute);
    if (info.isDirectory()) {
      for (const child of (await readdir(absolute)).sort()) await visit(`${path}/${child}`);
    } else if (extensions.some(extension => path.endsWith(extension))) {
      const text = await readFile(absolute, "utf8");
      files.push({ path, lines: text === "" ? 0 : text.split("\n").length - Number(text.endsWith("\n")) });
    }
  }
  for (const path of paths) await visit(path);
  return { lines: files.reduce((sum, file) => sum + file.lines, 0), files };
}

const repo = (await command(["git", "rev-parse", "--show-toplevel"], process.cwd())).trim();
const commit = (await command(["git", "rev-parse", "--verify", `${options.ref}^{commit}`], repo)).trim();
const temporary = await mkdtemp(join(tmpdir(), "thirdshift-benchmark-"));
const worktree = join(temporary, "worktree");
const measurements: {
  language: string; operation: string; cache: string; command: string[]; cwd: string;
  seconds: number; exitCode: number; output: string;
}[] = [];
const binaryBytes: Record<string, number> = {};
const logs = options.output + ".logs";
let added = false;
try {
  await command(["git", "worktree", "add", "--detach", worktree, commit], repo);
  added = true;
  await mkdir(logs, { recursive: true });
  const ts = join(worktree, "prototypes/typescript");
  const versions = {
    bun: (await command(["bun", "--version"], repo)).trim(),
    rustc: (await command(["rustc", "--version"], repo)).trim(),
    cargo: (await command(["cargo", "--version"], repo)).trim(),
    git: (await command(["git", "--version"], repo)).trim(),
  };
  // Installation/downloads are setup, outside measured wall time.
  await command(["bun", "install", "--frozen-lockfile"], ts);
  await command(["cargo", "fetch", "--locked"], worktree);
  async function pair(language: "typescript" | "rust", operation: string, args: string[], target?: string) {
    const cwd = language === "typescript" ? ts : worktree;
    const env = target ? {
      ...process.env, CARGO_TARGET_DIR: target, CARGO_BUILD_JOBS: "4",
      RUSTC_WRAPPER: "", RUSTC_WORKSPACE_WRAPPER: "", RUSTFLAGS: "", CARGO_ENCODED_RUSTFLAGS: "",
    } : process.env;
    for (const cache of ["cold", "incremental"]) {
      console.log(`${language} ${operation} ${cache}: ${args.join(" ")}`);
      const log = join(logs, `${language}-${operation}-${cache}.log`);
      const start = performance.now();
      let output: string;
      try { output = await command(args, cwd, env); }
      catch (error) {
        await writeFile(log, String(error) + "\n");
        throw new Error(`Measurement failed; retained output: ${log}`, { cause: error });
      }
      const seconds = (performance.now() - start) / 1000;
      await writeFile(log, output);
      measurements.push({ language, operation, cache, command: args, cwd: language === "typescript" ? "prototypes/typescript" : ".",
        seconds, exitCode: 0, output });
      console.log(`  ${measurements.at(-1)?.seconds.toFixed(3)}s`);
    }
  }
  await mkdir(join(ts, "build"), { recursive: true });
  await pair("typescript", "typecheck", ["bun", "node_modules/typescript/bin/tsc", "--noEmit", "--incremental", "--tsBuildInfoFile", "build/typecheck.tsbuildinfo"]);
  await rm(join(ts, "build"), { recursive: true, force: true });
  await pair("typescript", "build", ["bun", "run", "build.ts"]);
  binaryBytes.typescript = (await stat(join(ts, "build/thirdshift"))).size;
  await rm(join(ts, "build"), { recursive: true, force: true });
  // Explicit slice tests avoid recursively benchmarking this runner's own tests.
  await pair("typescript", "test", ["bun", "test", "tests/ticket.test.ts", "tests/ready.test.ts", "tests/console.test.ts", "tests/pickup-model.test.ts"]);
  await pair("typescript", "model", ["bun", "test", "tests/pickup-model.test.ts"]);

  const targets = join(temporary, "targets");
  const checkTarget = join(targets, "check");
  await pair("rust", "typecheck", ["cargo", "check", "--locked", "--bin", "thirdshift"], checkTarget);
  await rm(checkTarget, { recursive: true, force: true });
  const buildTarget = join(targets, "build");
  await pair("rust", "build", ["cargo", "build", "--release", "--locked", "--bin", "thirdshift"], buildTarget);
  binaryBytes.rust = (await stat(join(buildTarget, "release/thirdshift"))).size;
  await rm(buildTarget, { recursive: true, force: true });
  const testTarget = join(targets, "test");
  await pair("rust", "test", ["cargo", "test", "--locked", "--test", "fresh_run", "--test", "pickup_run", "--test", "console", "--test", "pickup_model", "--", "--test-threads=3"], testTarget);
  await pair("rust", "model", ["cargo", "test", "--locked", "--test", "pickup_model", "--", "--test-threads=1", "--nocapture"], testTarget);

  const sources = {
    typescript: {
      production: await inventory(worktree, ["prototypes/typescript/src"], [".ts", ".html"]),
      model: await inventory(worktree, ["prototypes/typescript/tests/pickup-model.test.ts"], [".ts"]),
    },
    rust: {
      production: await inventory(worktree, ["src"], [".rs", ".html", ".js", ".css"]),
      model: await inventory(worktree, ["tests/pickup_model.rs"], [".rs"]),
    },
  };
  // Owner-file inventories are auditable lower bounds, not dependency closures.
  // Rust embeds unit tests in owner files; raw physical lines include them.
  const sliceOwners = {
    typescript: {
      ticket: ["main.ts", "ticket.ts", "claim.ts", "delivery.ts", "ci.ts", "worktree.ts", "harness.ts", "github.ts", "process.ts"],
      pickup: ["pickup.ts", "ready.ts", "reservation.ts"],
      console: ["console/"],
    },
    rust: {
      ticket: ["run.rs", "claim.rs", "delivery.rs", "delivery/", "ci.rs", "worktree.rs", "worktree/", "harness.rs", "harness/", "github.rs", "github/", "process.rs", "process/", "interrupt.rs"],
      pickup: ["pickup.rs", "pickup/", "ready.rs", "reservation.rs"],
      console: ["console.rs", "console/", "logs/status.rs", "logs/status/", "logs/dismiss.rs", "logs/reading/inbox.rs"],
    },
  };
  const slices: Record<string, Record<string, { lines: number; files: { path: string; lines: number }[] }>> = {};
  for (const language of ["typescript", "rust"] as const) {
    const prefix = language === "typescript" ? "prototypes/typescript/src/" : "src/";
    const groups: Record<string, { lines: number; files: { path: string; lines: number }[] }> = {};
    for (const [name, owners] of Object.entries(sliceOwners[language])) {
      const files = sources[language].production.files.filter(file => owners.some(owner => owner.endsWith("/")
        ? file.path.startsWith(prefix + owner) : file.path === prefix + owner));
      groups[name] = { lines: files.reduce((sum, file) => sum + file.lines, 0), files };
    }
    slices[language] = groups;
  }
  const result = {
    schemaVersion: 1, commit, timestamp: new Date().toISOString(), versions,
    host: { platform: platform(), release: release(), cpu: cpus()[0]?.model, logicalCpus: cpus().length, memoryBytes: totalmem() },
    methodology: {
      cold: "Fresh detached worktree; dependencies installed/fetched first; independent empty build outputs for typecheck, build and test. OS and download caches retained.",
      incremental: "Immediate no-edit repeat in the same worktree, retaining that operation's outputs. tsc explicitly enables incremental metadata; Bun rebuilds/retests; Cargo reuses artifacts.",
      model: "Two standalone process runs after test compilation; excludes Rust compilation, includes runner startup and all model assertions/mutations. Not a cold compilation measurement.",
      rust: "Full production binary, release size/build; dev check/test; four build jobs; three test threads, one model thread; compiler wrappers and Rust flags cleared.",
      scope: "TypeScript Ticket/Pickup/Console slice versus broader Rust fresh_run/pickup_run/console/pickup_model suites. Not feature-equivalent workloads.",
      samples: 1,
    },
    measurements, binaryBytes, sources, slices,
  };
  await mkdir(dirname(options.output), { recursive: true });
  await writeFile(options.output, JSON.stringify(result, null, 2) + "\n");
  console.log(`Results: ${options.output}`);
} finally {
  if (added) await command(["git", "worktree", "remove", "--force", worktree], repo);
  await rm(temporary, { recursive: true, force: true });
}
