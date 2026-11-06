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
  expect(calls).toContain('"--method", "PUT", "-f", "labels[]=in-progress"');
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "ready-for-agent" }]);
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
elif args[:2] == ['api', 'repos/test/project/issues?state=open&labels=ready-for-agent&per_page=100']:
    print(json.dumps([[dict(issue, number=8), dict(issue, number=7)]]))
elif len(args) > 1 and args[0] == 'api' and '/events?' in args[1]:
    print(json.dumps([[{'event': 'labeled', 'label': {'name': 'ready-for-agent'}, 'created_at': '2026-01-01T00:00:00Z', 'actor': {'login': 'test'}}]]))
elif args[:2] == ['api', 'graphql']:
    connection = {'nodes': [], 'pageInfo': {'hasNextPage': False, 'endCursor': None}}
    print(json.dumps({'data': {'repository': {'issue': {'edits': connection, 'parent': None, 'subIssues': connection, 'blockedBy': connection, 'linked': connection}}}}))
elif args[:2] in [['api', 'repos/test/project/issues/7/comments'], ['api', 'repos/test/project/issues/8/comments']]:
    print(json.dumps([issue['comments']]))
elif args[0] == 'api' and args[1].endswith('/labels') and '--method' in args and args[args.index('--method') + 1] == 'PUT':
    labels = [arg.split('=', 1)[1] for arg in args if arg.startswith('labels[]=')]
    issue['labels'] = [{'name': x} for x in labels]
    (root / 'issue.json').write_text(json.dumps(issue))
    if 'in-progress' in labels and (root / 'interrupt-claim').exists():
        import time
        (root / 'claim-ready').write_text('ready')
        while True: time.sleep(10)
elif args[:2] == ['label', 'list']: print('[{"name":"in-progress"}]')
elif args[:2] == ['label', 'create']: (root / 'label-created').write_text('yes')
elif args[:2] == ['pr', 'list']: print('[]')
else: sys.exit('unexpected gh arguments: ' + repr(args))
`);
  await writeFile(join(directory, "claude"), `#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE'])
(root / 'session').write_text(json.dumps({'cwd': os.getcwd(), 'args': sys.argv[1:], 'stdin': sys.stdin.read()}))
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
  return { directory, git, start, run, issueChange };
}

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

test("Pickup selects the lowest Ready issue and holds a source-free reservation through Claim ending", async () => {
  const f = await fixture();
  const harness = join(f.directory, "claude");
  await writeFile(harness, (await readFile(harness, "utf8")) + `
import subprocess
ref = 'refs/heads/thirdshift/pickup/7'
token = subprocess.check_output(['git', 'ls-remote', '--refs', 'origin', ref], text=True).split()[0]
(root / 'reservation').write_text(subprocess.check_output(['git', 'cat-file', '-p', token], text=True))
assert [x['name'] for x in json.loads((root / 'issue.json').read_text())['labels']] == ['in-progress']
`);
  const result = await f.run("pickup");
  expect(result.stderr).toBe("");
  expect(result.code).toBe(0);
  const session = object(JSON.parse(await readFile(join(f.directory, "session"), "utf8")));
  expect(session.args).toEqual(expect.arrayContaining([expect.stringContaining("issues/7")]));
  const reservation = await readFile(join(f.directory, "reservation"), "utf8");
  expect(reservation).toContain("tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904");
  expect(reservation).not.toContain("parent ");
  expect(reservation).toContain("Host:");
  expect(await readFile(join(f.directory, "calls"), "utf8")).toContain('"repos/test/project/issues/7/labels", "--method", "PUT"');
  expect(f.git("ls-remote", "--refs", "origin", "refs/heads/thirdshift/pickup/*")).toBe("");
  const ended = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(ended.labels).toEqual([{ name: "ready-for-agent" }]);
}, 120_000);

test("Pickup passes over another host's reservation and takes the next Ready issue", async () => {
  const f = await fixture();
  const reference = "refs/heads/thirdshift/pickup/7";
  const owner = f.git("rev-parse", "HEAD");
  f.git("push", "origin", `${owner}:${reference}`);
  const result = await f.run("pickup");
  expect(result.code).toBe(0);
  expect(f.git("ls-remote", "--refs", "origin", reference)).toBe(`${owner}\t${reference}`);
  const session = object(JSON.parse(await readFile(join(f.directory, "session"), "utf8")));
  expect(session.args).toEqual(expect.arrayContaining([expect.stringContaining("issues/8")]));
}, 120_000);

test.each(["no-ready", "intervening-claim"])("Pickup re-verifies after reservation: %s", async change => {
  const f = await fixture();
  // The origin hook changes GitHub's state precisely after the reservation's
  // creation, with no sleeps or timing assumptions.
  const hook = join(f.directory, "origin.git/hooks/post-receive");
  await writeFile(hook, `#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE'])
for line in sys.stdin:
    if line.split()[2] == 'refs/heads/thirdshift/pickup/7' and line.split()[0] == '0' * 40:
        (root / 'changed').write_text('${change}')
`);
  await chmod(hook, 0o755);
  const gh = join(f.directory, "gh");
  await writeFile(gh, (await readFile(gh, "utf8")).replace("issue = json.loads((root / 'issue.json').read_text())", `issue = json.loads((root / 'issue.json').read_text())
if (root / 'changed').exists() and args[:2] == ['api', 'repos/test/project/issues/7']:
    if (root / 'changed').read_text() == 'no-ready': issue['labels'] = []
`).replace("print(json.dumps([[{'event': 'labeled'", `if (root / 'changed').exists() and '/7/events?' in args[1]:
        print(json.dumps([[{'event':'labeled','label':{'name':'in-progress'},'created_at':'2025-12-31T23:59:58Z','actor':{'login':'test'}}, {'event':'unlabeled','label':{'name':'in-progress'},'created_at':'2025-12-31T23:59:59Z','actor':{'login':'test'}}, {'event':'labeled','label':{'name':'ready-for-agent'},'created_at':'2026-01-01T00:00:00Z','actor':{'login':'test'}}]]))
        sys.exit(0)
    print(json.dumps([[{'event': 'labeled'`));
  const result = await f.run("pickup");
  expect(result.code).toBe(0);
  expect(f.git("ls-remote", "--refs", "origin", "refs/heads/thirdshift/pickup/*")).toBe("");
  const session = object(JSON.parse(await readFile(join(f.directory, "session"), "utf8")));
  expect(session.args).toEqual(expect.arrayContaining([expect.stringContaining("issues/8")]));
}, 120_000);

test.each(["author", "ready-actor", "body-edit", "title-edit", "active-history", "cased-history", "unready-label"])("Pickup rejects unsafe Ready input: %s", async reason => {
  const f = await fixture();
  if (reason === "author") await f.issueChange({ author_association: "NONE" });
  if (reason === "unready-label") await f.issueChange({ labels: [{ name: "ready-for-agent" }, { name: "needs-info" }] });
  const gh = join(f.directory, "gh");
  let source = await readFile(gh, "utf8");
  if (reason === "ready-actor") source = source.replace("'actor': {'login': 'test'}", "'actor': None");
  if (reason === "active-history") source = source.replace("'name': 'ready-for-agent'}, 'created_at'", "'name': 'in-progress'}, 'created_at'");
  if (reason === "cased-history") source = source.replace("'name': 'ready-for-agent'}, 'created_at'", "'name': 'In-Progress'}, 'created_at'");
  if (reason === "body-edit" || reason === "title-edit") source = source.replace("connection = {'nodes': [],", `query = args[args.index('-f') + 1]
    edits = [{'editedAt':'2026-02-01T00:00:00Z','createdAt':'2026-02-01T00:00:00Z','actor':None}] if '${reason === "body-edit" ? "userContentEdits" : "RENAMED_TITLE_EVENT"}' in query else []
    connection = {'nodes': edits,`);
  await writeFile(gh, source);
  const result = await f.run("pickup");
  expect(result.code).toBe(0);
  expect(result.stdout).toContain("Pickup skipped");
  expect(await Bun.file(join(f.directory, "session")).exists()).toBe(false);
  expect(f.git("ls-remote", "--refs", "origin", "refs/heads/thirdshift/pickup/*")).toBe("");
}, 120_000);

test("Pickup accepts a differently cased Ready label and preserves unrelated spelling", async () => {
  const f = await fixture();
  await f.issueChange({ labels: [{ name: "Ready-For-Agent" }, { name: "Bug" }] });
  const gh = join(f.directory, "gh");
  await writeFile(gh, (await readFile(gh, "utf8")).replace("'name': 'ready-for-agent'}, 'created_at'", "'name': 'Ready-For-Agent'}, 'created_at'"));
  const result = await f.run("pickup");
  expect(result.code).toBe(0);
  expect(await Bun.file(join(f.directory, "session")).exists()).toBe(true);
  const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
  expect(issue.labels).toEqual([{ name: "Bug" }, { name: "ready-for-agent" }]);
}, 120_000);

test("Pickup reads later edit-history pages before trusting the issue", async () => {
  const f = await fixture();
  const gh = join(f.directory, "gh");
  await writeFile(gh, (await readFile(gh, "utf8")).replace("connection = {'nodes': [], 'pageInfo': {'hasNextPage': False, 'endCursor': None}}", `connection = {'nodes': [], 'pageInfo': {'hasNextPage': False, 'endCursor': None}}
    if 'userContentEdits' in args[args.index('-f') + 1]:
        if 'after=second' not in args: connection['pageInfo'] = {'hasNextPage': True, 'endCursor': 'second'}
        else: connection['nodes'] = [{'editedAt': '2026-02-01T00:00:00Z', 'actor': None}]`));
  const result = await f.run("pickup");
  expect(result.code).toBe(0);
  expect(result.stdout).toContain("Pickup skipped");
  expect(await readFile(join(f.directory, "calls"), "utf8")).toContain("after=second");
  expect(await Bun.file(join(f.directory, "session")).exists()).toBe(false);
}, 120_000);

test("Pickup fails closed on an origin error without a competing token", async () => {
  const f = await fixture();
  const hook = join(f.directory, "origin.git/hooks/pre-receive");
  await writeFile(hook, "#!/bin/sh\nexit 1\n");
  await chmod(hook, 0o755);
  const result = await f.run("pickup");
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("check origin reservation refs/heads/thirdshift/pickup/7 by hand (our token");
  expect(await Bun.file(join(f.directory, "session")).exists()).toBe(false);
}, 120_000);

test("Pickup release preserves a changed reservation and names recovery evidence", async () => {
  const f = await fixture();
  const harness = join(f.directory, "claude");
  await writeFile(harness, (await readFile(harness, "utf8")) + `
import subprocess
subprocess.run(['git', 'push', '--force', 'origin', 'HEAD:refs/heads/thirdshift/pickup/7'], check=True)
`);
  const result = await f.run("pickup");
  expect(result.code).toBe(1);
  expect(result.stderr).toContain("could not release refs/heads/thirdshift/pickup/7");
  expect(result.stderr).toContain("our token");
  expect(f.git("ls-remote", "--refs", "origin", "refs/heads/thirdshift/pickup/7")).toContain(f.git("rev-parse", "HEAD"));
}, 120_000);

test("Pickup interruption ends the Claim before releasing its reservation", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "interrupt-claim"), "yes");
  const child = f.start("pickup");
  const output = Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
  try {
    await waitForFile(join(f.directory, "claim-ready"));
    child.kill("SIGINT");
    expect(await child.exited).toBe(130);
    expect(f.git("ls-remote", "--refs", "origin", "refs/heads/thirdshift/pickup/7")).toBe("");
    const issue = object(JSON.parse(await readFile(join(f.directory, "issue.json"), "utf8")));
    expect(issue.labels).toEqual([{ name: "ready-for-agent" }]);
  } finally {
    if (child.exitCode === null) child.kill("SIGINT");
    await child.exited;
    await output;
  }
}, 120_000);

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
    "elif args[0] == 'api' and args[1].endswith('/labels')",
    `elif args[:2] == ['label', 'list']: print('[]')
elif args[:2] == ['label', 'create']:
    (root / 'label-created').write_text('yes')
elif args[0] == 'api' and 'labels[]=in-progress' in args and not (root / 'label-created').exists():
    sys.exit("'in-progress' not found")
elif args[0] == 'api' and args[1].endswith('/labels')`
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
