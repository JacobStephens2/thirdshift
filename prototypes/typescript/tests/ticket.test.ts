import { afterAll, beforeAll, expect, test } from "bun:test";
import { chmod, lstat, mkdtemp, readFile, readdir, readlink, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function object(value: unknown): Record<string, unknown> {
  if (!isObject(value)) throw new Error("Expected JSON object");
  return value;
}

const root = resolve(import.meta.dir, "..");
const binary = join(root, "build/thirdshift");
const fixtures: string[] = [];
beforeAll(async () => {
  const build = Bun.spawn([process.execPath, "run", "build"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  const [code, out, err] = await Promise.all([build.exited, new Response(build.stdout).text(), new Response(build.stderr).text()]);
  if (code !== 0) throw new Error(`build: ${out}\n${err}`);
}, 120_000);

test("standalone Ticket run claims, links embedded skills and streams a Harness session", async () => {
  const f = await fixture();
  const result = await f.run();
  expect(result.code).toBe(0);
  const session = object(JSON.parse(await readFile(join(f.directory, "session"), "utf8")));
  expect(session.stdin).toBe("");
  expect(typeof session.cwd).toBe("string");
  if (typeof session.cwd !== "string") throw new Error("Missing session cwd");
  const worktree = session.cwd;
  expect(worktree).not.toBe(f.directory);
  expect(f.git("-C", worktree, "branch", "--show-current")).toBe("issue-7");
  expect(session.args).toEqual(expect.arrayContaining(["--output-format", "stream-json", "--permission-mode", "auto"]));
  const issueText = await readFile(join(worktree, ".thirdshift-issue.md"), "utf8");
  expect(issueText).toContain("trusted context");
  expect(issueText).not.toContain("untrusted text");
  expect(issueText).toContain("left out");
  const skills = join(worktree, ".claude/skills");
  for (const name of await readdir(resolve(root, "../../skills"))) {
    if (!name.startsWith("thirdshift-")) continue;
    expect((await lstat(join(skills, name))).isSymbolicLink()).toBe(true);
    const target = await readlink(join(skills, name));
    expect(await readFile(join(target, "SKILL.md"), "utf8")).toBe(await readFile(resolve(root, "../../skills", name, "SKILL.md"), "utf8"));
  }
  expect(f.git("-C", worktree, "status", "--porcelain")).toBe("");
  expect(result.stdout).toContain("working");
  expect(result.stdout).toContain("done");
  const calls = await readFile(join(f.directory, "calls"), "utf8");
  expect(calls).toContain('"--add-label", "in-progress", "--remove-label", "ready-for-agent"');
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "in-progress" }]);
}, 120_000);
afterAll(async () => { for (const fixture of fixtures) await rm(fixture, { recursive: true, force: true }); });

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), "thirdshift-ts-"));
  fixtures.push(directory);
  const env = {
    ...process.env,
    // The compiled executable must run without Bun or this repository on PATH.
    PATH: `${directory}:/usr/bin:/bin`,
    GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "Test", GIT_AUTHOR_EMAIL: "test@example.com",
    GIT_COMMITTER_NAME: "Test", GIT_COMMITTER_EMAIL: "test@example.com",
    FIXTURE: directory,
  };
  function git(...args: string[]) {
    const result = Bun.spawnSync(["git", ...args], { cwd: directory, env });
    if (result.exitCode !== 0) throw new Error(result.stderr.toString());
    return result.stdout.toString().trim();
  }
  git("init", "-q", "-b", "main");
  git("-c", "core.hooksPath=/dev/null", "commit", "--allow-empty", "-qm", "base");
  git("clone", "--bare", ".", "origin.git");
  // InsteadOf keeps Origin match real while git fetch/push use a private bare repo.
  git("remote", "add", "origin", "https://github.com/test/project.git");
  git("config", `url.${directory}/origin.git.insteadOf`, "https://github.com/test/project.git");
  await writeFile(join(directory, "issue.json"), JSON.stringify({
    title: "A Ticket", body: "Implement it", state: "open", author_association: "OWNER",
    labels: [{ name: "ready-for-agent" }],
    comments: [{ body: "trusted context", author_association: "MEMBER" }, { body: "untrusted text", author_association: "NONE" }],
  }));
  await writeFile(join(directory, "gh"), `#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE'])
args = sys.argv[1:]
with (root / 'calls').open('a') as log: log.write(json.dumps(args) + '\\n')
issue = json.loads((root / 'issue.json').read_text())
if args[:2] in [['api', 'repos/test/project/issues/7'], ['api', 'repos/test/project/issues/8']]:
    print(json.dumps(issue))
elif args[:2] in [['api', 'repos/test/project/issues/7/comments'], ['api', 'repos/test/project/issues/8/comments']]:
    print(json.dumps([issue['comments']]))
elif args[:2] == ['issue', 'edit']:
    labels = [x['name'] for x in issue['labels']]
    for i, arg in enumerate(args):
        if arg == '--add-label' and args[i+1] not in labels: labels.append(args[i+1])
        if arg == '--remove-label' and args[i+1] in labels: labels.remove(args[i+1])
    issue['labels'] = [{'name': x} for x in labels]
    (root / 'issue.json').write_text(json.dumps(issue))
    if '--add-label' in args and args[args.index('--add-label') + 1] == 'in-progress' and (root / 'interrupt-claim').exists():
        import time
        (root / 'claim-ready').write_text('ready')
        while True: time.sleep(10)
elif args[:2] == ['label', 'list']: print('[{"name":"in-progress"}]')
elif args[:2] == ['label', 'create']: (root / 'label-created').write_text('yes')
elif args[:2] == ['pr', 'list']:
    prs = json.loads((root / 'prs.json').read_text()) if (root / 'prs.json').exists() else {}
    head = args[args.index('--head') + 1]
    print(json.dumps([prs[head]] if head in prs else []))
elif args[:2] == ['pr', 'create']:
    import subprocess
    prs = json.loads((root / 'prs.json').read_text()) if (root / 'prs.json').exists() else {}
    head = args[args.index('--head') + 1]
    sha = subprocess.check_output(['git', 'rev-parse', head], text=True).strip()
    pr = {'number': len(prs) + 1, 'state': 'OPEN', 'headRefName': head,
          'headRefOid': sha, 'baseRefName': args[args.index('--base') + 1],
          'isDraft': '--draft' in args, 'mergeable': 'MERGEABLE',
          'url': 'https://github.com/test/project/pull/' + str(len(prs) + 1)}
    prs[head] = pr
    (root / 'prs.json').write_text(json.dumps(prs))
    print(pr['url'])
elif args[:2] == ['pr', 'view'] or args[:2] == ['pr', 'ready']:
    import subprocess
    prs = json.loads((root / 'prs.json').read_text())
    pr = next(x for x in prs.values() if str(x['number']) == args[2])
    pr['headRefOid'] = subprocess.check_output(['git', 'rev-parse', 'refs/heads/' + pr['headRefName']], cwd=root / 'origin.git', text=True).strip()
    if args[1] == 'ready': pr['isDraft'] = '--undo' in args
    (root / 'prs.json').write_text(json.dumps(prs))
    print(json.dumps(pr))
elif args[0] == 'api' and '/check-runs' in args[1]:
    mode = (root / 'ci').read_text() if (root / 'ci').exists() else 'green'
    sessions = int((root / 'sessions').read_text())
    failed = mode == 'always-red' or (mode == 'recover' and sessions == 1)
    reads = int((root / 'check-reads').read_text()) + 1 if (root / 'check-reads').exists() else 1
    (root / 'check-reads').write_text(str(reads))
    if mode == 'register': failed = sessions == 1
    checks = [{'name': 'tests', 'status': 'in_progress' if mode == 'pending' and reads == 1 else 'completed', 'conclusion': 'failure' if failed else 'success'}]
    if mode == 'absent' or (mode == 'register' and reads == 1): checks = []
    print(json.dumps([{'check_runs': checks}]))
elif args[0] == 'api' and args[1].endswith('/status'):
    mode = (root / 'ci').read_text() if (root / 'ci').exists() else 'green'
    red = (root / 'sessions').read_text() == '1'
    statuses = [{'context': 'external', 'state': 'failure' if red else 'success'}] if mode == 'status-recover' else []
    print(json.dumps([{'statuses': statuses}]))
else: sys.exit('unexpected gh arguments: ' + repr(args))
`);
  await writeFile(join(directory, "claude"), `#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE'])
(root / 'session').write_text(json.dumps({'cwd': os.getcwd(), 'args': sys.argv[1:], 'stdin': sys.stdin.read()}))
count = int((root / 'sessions').read_text()) + 1 if (root / 'sessions').exists() else 1
(root / 'sessions').write_text(str(count))
if (root / 'interrupt').exists():
    import signal, time
    def log(message):
        with (root / 'signals').open('a') as output: output.write(message + '\\n')
    def handler(number, frame):
        log(('leader ' if leader else 'child ') + signal.Signals(number).name)
        if leader and (root / 'leader-exits').exists(): sys.exit(0)
    leader = True
    signal.signal(signal.SIGINT, handler)
    signal.signal(signal.SIGTERM, handler)
    pid = os.fork()
    if pid == 0:
        leader = False
        (root / 'child-pid').write_text(str(os.getpid()))
        while True: time.sleep(10)
    while not (root / 'child-pid').exists(): time.sleep(0.01)
    (root / 'ready').write_text(str(os.getpid()))
    while True: time.sleep(10)
if (root / 'stream').exists():
    import time
    for byte in '{"type":"assistant","message":"café"}\\n'.encode(): os.write(1, bytes([byte]))
    while not (root / 'continue').exists(): time.sleep(0.01)
import subprocess
if (root / 'fail-repair').exists() and count > 1: sys.exit(3)
if (root / 'conflict').exists():
    pathlib.Path('shared.txt').write_text('Issue branch change\\n' if count == 1 else 'Resolved both changes\\n')
    subprocess.run(['git', 'add', 'shared.txt'], check=True)
if not (root / 'no-commit').exists():
    subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', 'commit', '--allow-empty', '-qm', 'Harness work ' + str(count)], check=True)
if (root / 'conflict').exists() and count == 1:
    (root / 'shared.txt').write_text('Base branch change\\n')
    subprocess.run(['git', '-C', str(root), 'add', 'shared.txt'], check=True)
    subprocess.run(['git', '-C', str(root), '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'Base moved'], check=True)
    subprocess.run(['git', '-C', str(root), 'push', 'origin', 'main'], check=True)
if (root / 'fail-harness').exists(): sys.exit(3)
print('{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}', flush=True)
print('{"type":"result","is_error":false,"result":"done"}', end='', flush=True)
`);
  await chmod(join(directory, "gh"), 0o755);
  await chmod(join(directory, "claude"), 0o755);
  function start(url = "https://github.com/test/project/issues/7") {
    return Bun.spawn([binary, url], { cwd: directory, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  }
  async function run(url?: string) {
    const child = start(url);
    const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
    return { code, stdout, stderr };
  }
  async function issueChange(change: Record<string, unknown>) {
    const file = join(directory, "issue.json");
    await writeFile(file, JSON.stringify({ ...object(JSON.parse(await readFile(file, "utf8"))), ...change }));
  }
  async function pr() {
    return object(object(JSON.parse(await readFile(join(directory, "prs.json"), "utf8")))["issue-7"]);
  }
  async function conflict() {
    await writeFile(join(directory, "shared.txt"), "Original\n");
    git("add", "shared.txt");
    git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "shared base");
    git("push", "origin", "main");
    await writeFile(join(directory, "conflict"), "yes");
  }
  return { directory, git, start, run, issueChange, pr, conflict };
}

test("green Delivery pushes the committed head and leaves a ready pull request", async () => {
  const f = await fixture();
  const result = await f.run();
  expect(result.code).toBe(0);
  expect(await Bun.file(join(f.directory, "prs.json")).exists()).toBe(true);
  const pr = await f.pr();
  expect(pr.isDraft).toBe(false);
  expect(pr.baseRefName).toBe("main");
  expect(pr.headRefOid).toBe(f.git("rev-parse", "issue-7"));
  expect(f.git("ls-remote", "--heads", "origin", "issue-7")).toContain(String(pr.headRefOid));
  const calls = await readFile(join(f.directory, "calls"), "utf8");
  expect(calls).toContain(`commits/${String(pr.headRefOid)}/check-runs`);
  expect(calls).toContain('"--draft"');
}, 120_000);

test("CI failure starts a Repair and watches the repaired head before readiness", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "ci"), "recover");
  const result = await f.run();
  expect(result.code).toBe(0);
  expect(await readFile(join(f.directory, "sessions"), "utf8")).toBe("2");
  const pr = await f.pr();
  expect(pr.isDraft).toBe(false);
  const head = f.git("rev-parse", "issue-7");
  expect(pr.headRefOid).toBe(head);
  expect(await readFile(join(f.directory, "calls"), "utf8")).toContain(`commits/${head}/check-runs`);
  const session = object(JSON.parse(await readFile(join(f.directory, "session"), "utf8")));
  expect(JSON.stringify(session.args)).toContain("CI red on tests");
}, 120_000);

test("Failed ticket run salvages committed work as a draft and preserves the Claim", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "fail-harness"), "yes");
  const result = await f.run();
  expect(result.code).toBe(1);
  expect(await Bun.file(join(f.directory, "prs.json")).exists()).toBe(true);
  const pr = await f.pr();
  expect(pr.isDraft).toBe(true);
  expect(f.git("ls-remote", "--heads", "origin", "issue-7")).toContain(f.git("rev-parse", "issue-7"));
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "in-progress" }]);
}, 120_000);

test("merge-conflict Repair commits the Base branch merge and re-verifies CI", async () => {
  const f = await fixture();
  await f.conflict();
  const result = await f.run();
  expect(result.code, result.stderr).toBe(0);
  expect(await readFile(join(f.directory, "sessions"), "utf8")).toBe("2");
  expect(f.git("show", "issue-7:shared.txt")).toBe("Resolved both changes");
  expect(f.git("rev-list", "--parents", "-n", "1", "issue-7").split(" ")).toHaveLength(3);
  f.git("merge-base", "--is-ancestor", "main", "issue-7");
  const pr = await f.pr();
  expect(pr.isDraft).toBe(false);
  expect(pr.headRefOid).toBe(f.git("rev-parse", "issue-7"));
  expect(await readFile(join(f.directory, "calls"), "utf8")).toContain(`commits/${String(pr.headRefOid)}/check-runs`);
}, 120_000);

test("Repair limit bounds permanently failing CI and keeps a draft with its Claim", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "ci"), "always-red");
  const result = await f.run();
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("Repair limit reached (5)");
  expect(await readFile(join(f.directory, "sessions"), "utf8")).toBe("6");
  const pr = await f.pr();
  expect(pr.isDraft).toBe(true);
  expect(pr.headRefOid).toBe(f.git("rev-parse", "issue-7"));
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "in-progress" }]);
}, 120_000);

test.each(["pending", "register", "status-recover", "absent"])("Delivery watches %s CI before readiness", async (mode) => {
  const f = await fixture();
  await writeFile(join(f.directory, "ci"), mode);
  const result = await f.run();
  expect(result.code, result.stderr).toBe(0);
  expect(await readFile(join(f.directory, "sessions"), "utf8")).toBe(mode === "pending" || mode === "absent" ? "1" : "2");
  const pr = await f.pr();
  expect(pr.isDraft).toBe(false);
}, 120_000);

test("failed conflict Repair aborts the merge and salvages the original committed head", async () => {
  const f = await fixture();
  await f.conflict();
  await writeFile(join(f.directory, "fail-repair"), "yes");
  const result = await f.run();
  expect(result.code).toBe(1);
  const session = object(JSON.parse(await readFile(join(f.directory, "session"), "utf8")));
  if (typeof session.cwd !== "string") throw new Error("Missing Worktree");
  const mergeHead = f.git("-C", session.cwd, "rev-parse", "--git-path", "MERGE_HEAD");
  expect(await Bun.file(resolve(session.cwd, mergeHead)).exists()).toBe(false);
  expect(f.git("-C", session.cwd, "status", "--porcelain")).toBe("");
  expect(f.git("show", "issue-7:shared.txt")).toBe("Issue branch change");
  const pr = await f.pr();
  expect(pr.isDraft).toBe(true);
  expect(pr.headRefOid).toBe(f.git("rev-parse", "issue-7"));
}, 120_000);

test("a session with no commits fails Delivery and releases its Claim with nothing on origin", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "no-commit"), "yes");
  const result = await f.run();
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("no commits ahead");
  expect(f.git("ls-remote", "--heads", "origin", "issue-7")).toBe("");
  expect(await Bun.file(join(f.directory, "prs.json")).exists()).toBe(false);
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "ready-for-agent" }]);
}, 120_000);

test("Origin match fails before claiming or starting a session", async () => {
  const f = await fixture();
  const result = await f.run("https://github.com/other/project/issues/7");
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("Origin match");
  expect(await Bun.file(join(f.directory, "calls")).exists()).toBe(false);
}, 120_000);

test("a non-canonical Issue URL is rejected before any GitHub call", async () => {
  const f = await fixture();
  const result = await f.run("https://user:ghp_testsecret@github.com/test/project/issues/7?inject=run-this");
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("Expected a GitHub Issue URL");
  expect(`${result.stdout}\n${result.stderr}`).not.toContain("ghp_testsecret");
  expect(`${result.stdout}\n${result.stderr}`).not.toContain("inject=run-this");
  expect(await Bun.file(join(f.directory, "calls")).exists()).toBe(false);
  expect(await Bun.file(join(f.directory, "session")).exists()).toBe(false);
}, 120_000);

test("issue file write does not follow a symlink outside the worktree", async () => {
  const f = await fixture();
  await symlink("../../../config", join(f.directory, ".thirdshift-issue.md"));
  f.git("add", ".thirdshift-issue.md");
  f.git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "symlink");
  const result = await f.run();
  const config = await readFile(join(f.directory, ".git/config"), "utf8");
  expect(config).toContain('[remote "origin"]');
  expect(config).toContain("https://github.com/test/project.git");
  expect(config).not.toContain("Implement it");
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("Refusing to follow a symlink");
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "ready-for-agent" }]);
}, 120_000);

async function waitForFile(path: string): Promise<string> {
  const deadline = performance.now() + 30_000;
  while (performance.now() < deadline) {
    if (await Bun.file(path).exists()) {
      const content = await readFile(path, "utf8");
      if (content) return content;
    }
    await Bun.sleep(20);
  }
  throw new Error(`Timed out waiting for ${path}`);
}

test.each([false, true])("interruption stops descendants and ends the Claim (leader exits: %p)", async (exitLeader) => {
  const f = await fixture();
  await writeFile(join(f.directory, "interrupt"), "");
  if (exitLeader) await writeFile(join(f.directory, "leader-exits"), "yes");
  const child = f.start();
  const stdout = new Response(child.stdout).text();
  const stderr = new Response(child.stderr).text();
  let leader: number | undefined;
  let descendant: number | undefined;
  try {
    leader = Number(await waitForFile(join(f.directory, "ready")));
    descendant = Number(await waitForFile(join(f.directory, "child-pid")));
    child.kill("SIGINT");
    expect(await child.exited).toBe(130);
    expect(await readFile(join(f.directory, "signals"), "utf8")).toContain("leader SIGINT");
    const signals = await readFile(join(f.directory, "signals"), "utf8");
    if (!exitLeader) expect(signals).toContain("leader SIGTERM");
    expect(signals).toContain("child SIGINT");
    expect(signals).toContain("child SIGTERM");
    for (const pid of [leader, descendant]) {
      // A killed orphan can remain a zombie briefly until its adopter reaps it.
      const stat = Bun.file(`/proc/${pid}/stat`);
      if (await stat.exists()) expect((await stat.text()).split(") ")[1]?.startsWith("Z")).toBe(true);
    }
    const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
    expect(issue.labels).toEqual([{ name: "ready-for-agent" }]);
  } finally {
    if (child.exitCode === null) child.kill("SIGKILL");
    if (leader !== undefined) {
      try { process.kill(-leader, "SIGKILL"); } catch { /* already gone */ }
    }
    if (descendant !== undefined) {
      try { process.kill(descendant, "SIGKILL"); } catch { /* already gone */ }
    }
    await child.exited;
    await Promise.all([stdout, stderr]);
  }
}, 120_000);

test("Harness JSON is decoded and streamed before the session ends", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "stream"), "yes");
  const child = f.start();
  const stderr = new Response(child.stderr).text();
  let output = "";
  const reader = child.stdout.getReader();
  const reading = (async () => {
    const decoder = new TextDecoder();
    for (;;) {
      const next = await reader.read();
      if (next.done) break;
      output += decoder.decode(next.value, { stream: true });
    }
    output += decoder.decode();
  })();
  try {
    const deadline = performance.now() + 30_000;
    while (!output.includes("café") && performance.now() < deadline) await Bun.sleep(20);
    expect(output).toContain("café");
    expect(child.exitCode).toBe(null);
    await writeFile(join(f.directory, "continue"), "yes");
    expect(await child.exited).toBe(0);
    await reading;
    expect(output).toContain("done");
  } finally {
    if (child.exitCode === null) child.kill("SIGINT");
    await child.exited;
    await Promise.all([reading, stderr]);
  }
}, 120_000);

test("Harness failures release a new Claim and preserve other issue labels", async () => {
  for (const script of [
    'print("not JSON", flush=True)',
    'print(\'{"type":"result","is_error":true}\', flush=True)',
    'print(\'{"type":"assistant"}\', flush=True)',
    'import sys; sys.exit(3)',
  ]) {
    const f = await fixture();
    await f.issueChange({ labels: [{ name: "bug" }, { name: "ready-for-agent" }] });
    await writeFile(join(f.directory, "claude"), `#!/usr/bin/env python3\n${script}\n`);
    const result = await f.run();
    expect(result.code).toBe(1);
    const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
    expect(issue.labels).toEqual([{ name: "bug" }, { name: "ready-for-agent" }]);
  }
}, 120_000);

test("a pushed Issue branch keeps its Claim", async () => {
  const f = await fixture();
  const harness = join(f.directory, "claude");
  await writeFile(harness, (await readFile(harness, "utf8")) + '\nimport subprocess\nsubprocess.run(["git", "push", "origin", "HEAD"], check=True)\n');
  expect((await f.run()).code).toBe(0);
  expect(f.git("ls-remote", "--heads", "origin", "issue-7")).toContain("refs/heads/issue-7");
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "in-progress" }]);
}, 120_000);

test("an existing Claim is left in place", async () => {
  const f = await fixture();
  await f.issueChange({ labels: [{ name: "in-progress" }] });
  expect((await f.run()).code).toBe(0);
  expect(await readFile(join(f.directory, "calls"), "utf8")).not.toContain('"edit"');
}, 120_000);

test("interruption while applying the Claim still restores its prior labels", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "interrupt-claim"), "yes");
  const child = f.start();
  const output = Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
  try {
    await waitForFile(join(f.directory, "claim-ready"));
    child.kill("SIGINT");
    expect(await child.exited).toBe(130);
    const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
    expect(issue.labels).toEqual([{ name: "ready-for-agent" }]);
  } finally {
    if (child.exitCode === null) child.kill("SIGINT");
    await child.exited;
    await output;
  }
}, 120_000);

test("Claim ending respects the Day shift removing its Claim", async () => {
  const f = await fixture();
  const harness = join(f.directory, "claude");
  await writeFile(harness, (await readFile(harness, "utf8")) + `
issue = json.loads((root / 'issue.json').read_text())
issue['labels'] = [{'name': 'ready-for-human'}]
(root / 'issue.json').write_text(json.dumps(issue))
`);
  expect((await f.run()).code).toBe(0);
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "ready-for-human" }]);
}, 120_000);

test("closed and untrusted issues fail before applying a Claim", async () => {
  for (const change of [{ state: "closed" }, { author_association: "NONE" }]) {
    const f = await fixture();
    const file = join(f.directory, "issue.json");
    const issue = object(JSON.parse(await readFile(file, "utf8")));
    await writeFile(file, JSON.stringify({ ...issue, ...change }));
    const result = await f.run();
    expect(result.code).toBe(1);
    expect(result.stderr).toContain("pre-flight");
    expect(await readFile(join(f.directory, "calls"), "utf8")).not.toContain('"edit"');
    expect(await Bun.file(join(f.directory, "session")).exists()).toBe(false);
  }
}, 120_000);

test("shared skill exclusions are added once across Ticket runs", async () => {
  const f = await fixture();
  expect((await f.run()).code).toBe(0);
  expect((await f.run("https://github.com/test/project/issues/8")).code).toBe(0);
  const exclusions = await readFile(join(f.directory, ".git/info/exclude"), "utf8");
  expect(exclusions.split("\n").filter(line => line === "/.claude/skills/thirdshift-implement")).toHaveLength(1);
  expect(exclusions.split("\n").filter(line => line === "/.thirdshift-issue.md")).toHaveLength(1);
}, 120_000);

test("missing repository Claim label is created before applying it", async () => {
  const f = await fixture();
  const gh = join(f.directory, "gh");
  await writeFile(gh, (await readFile(gh, "utf8")).replace(
    "elif args[:2] == ['issue', 'edit']:",
    `elif args[:2] == ['label', 'list']: print('[]')
elif args[:2] == ['label', 'create']:
    (root / 'label-created').write_text('yes')
elif args[:2] == ['issue', 'edit']:
    if '--add-label' in args and args[args.index('--add-label') + 1] == 'in-progress' and not (root / 'label-created').exists():
        sys.exit("'in-progress' not found")`
  ));
  expect((await f.run()).code).toBe(0);
}, 120_000);

test("closed issue clears an existing Claim", async () => {
  const f = await fixture();
  await f.issueChange({ labels: [{ name: "in-progress" }] });
  const harness = join(f.directory, "claude");
  await writeFile(harness, (await readFile(harness, "utf8")) + `
issue = json.loads((root / 'issue.json').read_text())
issue['state'] = 'closed'
(root / 'issue.json').write_text(json.dumps(issue))
`);
  expect((await f.run()).code).toBe(0);
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([]);
}, 120_000);

test("Failed salvage drafts an existing ready PR even when repaired commits cannot push", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "ci"), "recover");
  const gh = join(f.directory, "gh");
  const header = "elif args[0] == 'api' and '/check-runs' in args[1]:\n";
  await writeFile(gh, (await readFile(gh, "utf8")).replace(header, header + `
    prs = json.loads((root / 'prs.json').read_text())
    for pr in prs.values(): pr['isDraft'] = False
    (root / 'prs.json').write_text(json.dumps(prs))
`));
  const git = join(f.directory, "git");
  await writeFile(git, `#!/usr/bin/env python3
import os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE'])
if sys.argv[1:2] == ['push'] and (root / 'sessions').exists() and int((root / 'sessions').read_text()) >= 2:
    sys.exit('simulated transport failure publishing repaired head')
os.execv('/usr/bin/git', ['git', *sys.argv[1:]])
`);
  await chmod(git, 0o755);
  const result = await f.run();
  expect(result.code).toBe(1);
  const pr = await f.pr();
  expect(pr.isDraft).toBe(true);
}, 120_000);
