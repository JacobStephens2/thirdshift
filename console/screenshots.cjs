// Capture the real Console with sample records, never the caller's logs.
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { mkdtemp, mkdir, writeFile, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { createInterface } = require('node:readline');
const { chromium } = require('./tests/playwright.cjs');

async function main() {
  const home = await mkdtemp(join(tmpdir(), 'thirdshift-screenshots-'));
  let server, exited, browser;
  try {
    const folder = join(home, '.thirdshift/logs/acme/widgets/commands/issue');
    const output = resolve(__dirname, '../site/screenshots');
    await mkdir(folder, { recursive: true });
    await mkdir(output, { recursive: true });
    await writeFile(join(home, '.thirdshift/config.toml'), '[console]\nport = 0\ntheme = "focus-light"\n');

    // Answer the Console's read-only GitHub queries locally. No account or
    // network is needed, and no sample URL is looked up on GitHub.
    const bin = join(home, 'bin');
    await mkdir(bin);
    await writeFile(join(bin, 'gh'), `#!/usr/bin/env python3
import json, re, sys, time
if sys.argv[1:] == ['api', 'rate_limit']:
    quota = {'remaining': 5000, 'reset': int(time.time()) + 3600}
    print(json.dumps({'resources': {'graphql': quota, 'core': quota}}))
elif sys.argv[1:3] == ['api', 'graphql']:
    data = {'rateLimit': {'remaining': 5000, 'resetAt': '2099-01-01T00:00:00Z'}}
    for alias, field in re.findall(r'(pr[0-9]+): repository.*?[{] (issue|pullRequest)[(]', sys.argv[-1]):
        data[alias] = {field: {'state': 'OPEN', 'closedAt': None, 'mergedAt': None, 'labels': {'nodes': []}}}
    print(json.dumps({'data': data}))
else:
    sys.exit('Unexpected screenshot fixture gh command: ' + ' '.join(sys.argv[1:]))
`, { mode: 0o755 });

    const now = Date.now();
    const time = seconds => new Date(now - seconds * 1000).toISOString();
    async function record(number, title, outcome, cause = null) {
      const session = join(folder, `${number}-sample-implement.logs.jsonl`);
      const calls = [
        ['Read', 'Read src/search.rs', 560, 552],
        ['Edit', 'Update search behavior', 550, 538],
        ['Bash', 'cargo test --test search', 535, 475],
        ['Bash', 'cargo clippy --all-targets', 470, 440],
      ];
      const spans = calls.map(([tool, name, start, end], index) => ({
        spanId: (index + 1).toString(16).padStart(16, '0'), name,
        startTimeUnixNano: `${BigInt(Date.parse(time(start))) * 1000000n}`,
        endTimeUnixNano: `${BigInt(Date.parse(time(end))) * 1000000n}`,
        attributes: [
          { key: 'gen_ai.operation.name', value: { stringValue: 'execute_tool' } },
          { key: 'gen_ai.tool.name', value: { stringValue: tool } },
          { key: 'gen_ai.tool.call.id', value: { stringValue: `call-${index}` } },
        ],
      }));
      await writeFile(session, '');
      await writeFile(session.replace('.logs.jsonl', '.spans.jsonl'),
        JSON.stringify({ resourceSpans: [{ scopeSpans: [{ spans }] }] }) + '\n');
      await writeFile(join(folder, `${number}-sample.command.json`), JSON.stringify({
        version: 1, kind: 'Run', name: `run-${number}`, repository: 'acme/widgets', hostname: 'sample',
        issue: { number, url: `https://github.com/acme/widgets/issues/${number}`, title },
        process_id: 0, process_start: 'gone', harness: 'codex', model: 'gpt-6', effort: 'high',
        started_at: time(600), ended_at: time(120), outcome, cause, session_log: session,
        issue_branch: `issue-${number}`,
        pull_request: outcome === 'ready for review' ? `https://github.com/acme/widgets/pull/${number + 10}` : null,
        stages: [
          { name: 'implement', began_at: time(600), session_log: session },
          { name: 'review', began_at: time(420), session_log: null },
          { name: 'CI watch', began_at: time(240), session_log: null },
          { name: 'cleanup', began_at: time(130), session_log: null },
        ],
      }));
      await writeFile(join(folder, `${number}-sample.log`),
        `thirdshift: #${number}: implement\nthirdshift: #${number}: review\nthirdshift: #${number}: ${outcome}\n`);
    }
    await record(7, 'Handle an empty search query', 'failed', 'Search test failed: empty query should show all widgets.');
    await record(8, 'Keep search filters in the URL', 'ready for review');
    await record(9, 'Add keyboard navigation to search results', 'ready for review');

    const env = { ...process.env, HOME: home, PATH: `${bin}:${process.env.PATH}` };
    delete env.THIRDSHIFT_CONSOLE_ASSETS;
    server = spawn(process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../target/debug/thirdshift'),
      ['console', '--no-sign-in'], {
        env,
        stdio: ['ignore', 'pipe', 'inherit'],
      });
    exited = once(server, 'exit');
    const lines = createInterface({ input: server.stdout });
    let origin;
    try { [origin] = await once(lines, 'line', { signal: AbortSignal.timeout(10000) }); }
    finally { lines.close(); }
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1200 }, reducedMotion: 'reduce' });
    async function capture(name) {
      await page.evaluate(() => document.fonts.ready);
      // Keep the navigation and page, omitting the capturing machine's name
      // and browser notification settings in the header.
      const clip = await page.evaluate(() => {
        const y = Math.ceil(document.querySelector('header').getBoundingClientRect().bottom + scrollY);
        const bottom = Math.max(document.documentElement.scrollHeight, document.body.scrollHeight);
        return { x: 0, y, width: innerWidth, height: bottom - y };
      });
      await page.screenshot({ path: join(output, name), clip });
    }
    await page.goto(origin);
    const failed = page.locator('#inbox > [data-group="needs you"] .command').first();
    await failed.locator(':scope > .row-head > .state').click();
    await page.locator('#peek-content h2').waitFor();
    await capture('inbox.png');

    await page.goto(`${origin}/waterfall?command=widgets%238`);
    await page.getByRole('button', { name: 'Every call', exact: true }).click();
    await page.locator('.span-item[data-kind="tool"]').first().waitFor();
    await capture('waterfall.png');
    console.log(`Captured Inbox and Waterfall in ${output}`);
  } finally {
    await browser?.close();
    if (server && server.exitCode === null) server.kill();
    await exited;
    await rm(home, { recursive: true, force: true });
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
