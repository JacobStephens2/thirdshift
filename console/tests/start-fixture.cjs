const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, readFile, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./playwright.cjs');
const { replaceRecord } = require('./records.cjs');

async function startFixture(extraEnv = {}) {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift-start-page-'));
  let server, exited, browser;
  const close = async () => {
    await browser?.close();
    if (server && server.exitCode === null) server.kill();
    await exited;
    await rm(home, { recursive: true, force: true });
  };
  try {
    await mkdir(join(home, '.thirdshift'));
    await mkdir(join(home, 'bin'));
    const repo = join(home, 'repo');
    const remote = join(home, 'origin.git');
    const git = args => execFileSync('git', args, { stdio: 'pipe' });
    git(['init', '--bare', '-b', 'main', remote]);
    git(['clone', remote, repo]);
    git(['-C', repo, 'config', 'user.name', 'Console fixture']);
    git(['-C', repo, 'config', 'user.email', 'fixture@example.test']);
    git(['-C', repo, 'commit', '--allow-empty', '-m', 'Fixture']);
    git(['-C', repo, 'push', 'origin', 'main']);
    git(['-C', repo, 'remote', 'set-url', 'origin', 'https://github.com/acme/widgets']);
    git(['-C', repo, 'config', `url.${remote}.insteadOf`, 'https://github.com/acme/widgets']);
    await writeFile(join(home, '.thirdshift/config.toml'), `launch_directories = [${JSON.stringify(repo)}]\n[email]\nalways = false\n[harness]\ndefault = "codex"\n[console]\nport = 0\nallowed_emails = ["owner@example.test"]\n`);
    await writeFile(join(home, '.thirdshift/console-accounts.json'), JSON.stringify({
      'owner@example.test': {
        password_hash: '$argon2id$v=19$m=19456,t=2,p=1$Yd89fw8Yx3I9TbhuY+Y/cQ$NhL+Lqg2T9nXrlM8NKBaQ3dpVpRWjRsyYbeNkQS8Q7s', sessions: {}
      }
    }), { mode: 0o600 });
    await writeFile(join(home, 'bin/crontab'), '#!/bin/sh\ncat "$HOME/crontab"\n', { mode: 0o755 });
    // No network or real Harness is used. Ticket discovery records a Command,
    // then PR selection fails. The Pass finds no Ready issue and skips.
    await writeFile(join(home, 'bin/gh'), `#!/usr/bin/env python3
import json, sys
args = sys.argv[1:]
if args[:2] == ['issue', 'view']:
    print(json.dumps({'state':'OPEN', 'title':'Fixture issue', 'labels':[], 'createdAt':'2026-01-01T00:00:00Z'}))
elif '--paginate' in args:
    pass # --jq .[] returns no items
elif args[:2] == ['api', 'graphql']:
    print(json.dumps({'data':{'repository':{'issue':{'subIssues':{'nodes':[], 'pageInfo':{'hasNextPage':False}}}}}}))
else:
    print('fixture stops at PR selection', file=sys.stderr)
    sys.exit(1)
`, { mode: 0o755 });
    await writeFile(join(home, 'bin/codex'), '#!/bin/sh\n[ "$1 $2" = "login status" ] && exit 0\nexit 1\n', { mode: 0o755 });
    const shell = join(home, 'login-shell');
    await writeFile(shell, '#!/bin/sh\n[ "$1" = -l ] || exit 91\nshift\nexec /bin/sh "$@"\n', { mode: 0o755 });
    await writeFile(join(home, 'crontab'), `*/5 * * * * cd '${repo}' && thirdshift pickup base main highest-first no-email harness codex\n`);
    const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
    const env = { ...process.env, ...extraEnv, HOME: home, SHELL: shell, PATH: `${join(home, 'bin')}:${process.env.PATH}` };
    server = spawn(binary, ['console'], { env, stdio: ['ignore', 'pipe', 'inherit'] });
    exited = once(server, 'exit');
    const lines = createInterface({ input: server.stdout });
    let origin;
    try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(15000) }); }
    finally { lines.close(); }
    browser = await chromium.launch({ headless: true });
    return { home, repo, browser, origin, close,
      async readyPullRequest(number = 42, changes = {}, github = {}) {
        const logs = join(home, '.thirdshift/logs');
        const directory = join(logs, 'acme/widgets/commands/review');
        await mkdir(directory, { recursive: true });
        const pull_request = changes.pull_request ?? `https://github.com/acme/widgets/pull/${number}`;
        const now = new Date().toISOString();
        const path = join(directory, `${number}-fixture.command.json`);
        await replaceRecord(path, JSON.stringify({
          version: 1, kind: 'Review run', repository: 'acme/widgets', hostname: 'fixture',
          issue: null, process_id: 0, process_start: 'gone', harness: 'claude', model: 'old-model', effort: 'high',
          started_at: now, ended_at: now, stages: [], outcome: 'ready for review', pull_request,
          ...changes
        }));
        const cache = join(logs, 'github-inbox.json');
        const kept = JSON.parse(await readFile(cache, 'utf8').catch(error => {
          if (error.code !== 'ENOENT') throw error;
          return '{"version":1,"subjects":{}}';
        }));
        kept.subjects[pull_request] = { state: 'Open', ci: 'red', mergeable: 'mergeable', title: 'Repair these widgets', ...github };
        await replaceRecord(cache, JSON.stringify(kept));
        await writeFile(join(logs, 'github-inbox.changed'), `${number}:${JSON.stringify(github)}`);
        return path;
      },
      cli(args, input) { return execFileSync(binary, args, { env, input, encoding: 'utf8' }); },
      scopes(value) { execFileSync(binary, ['console', 'scopes', '--scopes', value], { env }); },
      async page(options) {
        const page = await browser.newPage(options);
        await page.goto(`${origin}/sign-in`);
        await page.getByLabel('Email', { exact: true }).fill('owner@example.test');
        await page.getByLabel('Password', { exact: true }).fill('browser test password');
        await Promise.all([page.waitForURL(`${origin}/`), page.getByRole('button', { name: 'Sign in', exact: true }).click()]);
        return page;
      }
    };
  } catch (error) { await close(); throw error; }
}
module.exports = { startFixture };
