#!/usr/bin/env node
'use strict';

// Seed an isolated temporary HOME for verify-console with faux Command records,
// Command logs, Session traces, Waterfall spans, and a local stub `gh`.
const { mkdirSync, writeFileSync } = require('node:fs');
const { join } = require('node:path');

const args = process.argv.slice(2);
const signIn = args.includes('--sign-in');
const positional = args.filter(arg => !arg.startsWith('--'));
const home = positional[0];

if (!home) {
  console.error('Usage: seed-home.cjs <mktemp-home-dir> [--sign-in]');
  process.exit(1);
}

const configDir = join(home, '.thirdshift');
const widgetsFolder = join(configDir, 'logs/acme/widgets/commands/issue');
const gadgetsFolder = join(configDir, 'logs/acme/gadgets/commands/issue');
const binDir = join(home, 'bin');

mkdirSync(widgetsFolder, { recursive: true });
mkdirSync(gadgetsFolder, { recursive: true });
mkdirSync(binDir, { recursive: true });

let config = '[console]\nport = 0\n';
if (signIn) {
  config += 'allowed_emails = ["owner@example.test"]\n';
  writeFileSync(
    join(configDir, 'console-accounts.json'),
    JSON.stringify({
      'owner@example.test': {
        // Committed faux test password: "browser test password"
        password_hash: '$argon2id$v=19$m=19456,t=2,p=1$Yd89fw8Yx3I9TbhuY+Y/cQ$NhL+Lqg2T9nXrlM8NKBaQ3dpVpRWjRsyYbeNkQS8Q7s',
        sessions: {},
      },
    }),
    { mode: 0o600 }
  );
}
writeFileSync(join(configDir, 'config.toml'), config);

writeFileSync(
  join(binDir, 'gh'),
  `#!/usr/bin/env python3
import json, re, sys, time
args = sys.argv[1:]
if args[:2] == ['api', 'user']:
    print(json.dumps({'login': 'faux-operator'}))
elif args[:2] == ['api', 'rate_limit']:
    quota = {'remaining': 5000, 'reset': int(time.time()) + 3600}
    print(json.dumps({'resources': {'graphql': quota, 'core': quota}}))
elif args[:2] == ['api', 'graphql']:
    data = {'rateLimit': {'limit': 5000, 'cost': 1, 'remaining': 5000, 'resetAt': '2099-01-01T00:00:00Z'}}
    query = args[-1]
    for alias, owner, repo, field, number in re.findall(
        r'(pr[0-9]+): repository[(]owner: "([^"]+)", name: "([^"]+)"[)] [{] (issue|pullRequest)[(]number: ([0-9]+)[)]',
        query,
    ):
        if field == 'pullRequest':
            data[alias] = {
                'pullRequest': {
                    'url': f'https://github.com/{owner}/{repo}/pull/{number}',
                    'title': 'Keep search filters in the URL',
                    'state': 'OPEN',
                    'mergedAt': None,
                    'closedAt': None,
                    'mergeable': 'MERGEABLE',
                    'commits': {'nodes': [{'commit': {'statusCheckRollup': {'state': 'SUCCESS'}}}]},
                }
            }
        else:
            data[alias] = {'issue': {'state': 'OPEN', 'closedAt': None, 'labels': {'nodes': []}}}
    print(json.dumps({'data': data}))
else:
    sys.exit('Unexpected verify-console stub gh command: ' + ' '.join(args))
`,
  { mode: 0o755 }
);

const now = Date.now();
const isoSecondsAgo = seconds => new Date(now - seconds * 1000).toISOString();

function traceRecord(raw) {
  return JSON.stringify({
    resourceLogs: [{ scopeLogs: [{ logRecords: [{ body: { stringValue: raw } }] }] }],
  }) + '\n';
}

function seedCommand(folder, repository, number, title, outcome, startAgo, endAgo, cause = null) {
  const stem = `${number}-verify`;
  const sessionPath = join(folder, `${stem}-implement.logs.jsonl`);
  const spansPath = sessionPath.replace('.logs.jsonl', '.spans.jsonl');
  const commandLogPath = join(folder, `${stem}.log`);
  const recordPath = join(folder, `${stem}.command.json`);

  const calls = [
    ['Read', 'src/search.rs', startAgo - 20, startAgo - 35],
    ['Edit', 'src/search.rs (update search behavior)', startAgo - 40, startAgo - 65],
    ['Bash', 'cargo test --test search', startAgo - 70, startAgo - 130],
    ['Bash', 'cargo clippy --all-targets', startAgo - 135, startAgo - 165],
  ];
  const spans = calls.map(([tool, args, start, end], index) => ({
    spanId: (index + 1).toString(16).padStart(16, '0'),
    name: `${tool} ${args}`,
    startTimeUnixNano: `${BigInt(Date.parse(isoSecondsAgo(start))) * 1000000n}`,
    endTimeUnixNano: `${BigInt(Date.parse(isoSecondsAgo(end))) * 1000000n}`,
    attributes: [
      { key: 'gen_ai.operation.name', value: { stringValue: 'execute_tool' } },
      { key: 'gen_ai.tool.name', value: { stringValue: tool } },
      { key: 'gen_ai.tool.call.arguments', value: { stringValue: args } },
      { key: 'gen_ai.tool.call.id', value: { stringValue: `call-${number}-${index}` } },
    ],
  }));

  const events = [
    traceRecord(JSON.stringify({ type: 'item.completed', item: { id: `m-${number}-1`, type: 'agent_message', text: `Inspecting ${repository}#${number}: ${title}` } })),
    traceRecord(JSON.stringify({ type: 'item.completed', item: { id: `t-${number}-1`, type: 'command_execution', command: 'cargo test --test search', aggregated_output: 'test result: ok.', status: 'completed', exit_code: 0 } })),
  ].join('');

  writeFileSync(sessionPath, events);
  writeFileSync(spansPath, JSON.stringify({ resourceSpans: [{ scopeSpans: [{ spans }] }] }) + '\n');
  writeFileSync(commandLogPath, `thirdshift: #${number}: implement\nthirdshift: #${number}: review\nthirdshift: #${number}: ${outcome}\n`);
  writeFileSync(
    recordPath,
    JSON.stringify({
      version: 1,
      kind: 'Run',
      name: `run-${number}`,
      repository,
      hostname: 'verify-host',
      issue: { number, url: `https://github.com/${repository}/issues/${number}`, title },
      process_id: 0,
      process_start: 'gone',
      harness: 'codex',
      model: 'gpt-6',
      effort: 'high',
      started_at: isoSecondsAgo(startAgo),
      ended_at: isoSecondsAgo(endAgo),
      outcome,
      cause,
      session_log: sessionPath,
      issue_branch: `issue-${number}`,
      pull_request: outcome === 'ready for review' ? `https://github.com/${repository}/pull/${number + 10}` : null,
      stages: [
        { name: 'implement', began_at: isoSecondsAgo(startAgo), session_log: sessionPath },
        { name: 'review', began_at: isoSecondsAgo(Math.max(endAgo + 60, startAgo - 180)), session_log: null },
        { name: 'CI watch', began_at: isoSecondsAgo(endAgo + 30), session_log: null },
        { name: 'cleanup', began_at: isoSecondsAgo(endAgo + 5), session_log: null },
      ],
    })
  );
}

seedCommand(
  widgetsFolder,
  'acme/widgets',
  7,
  'Handle an empty search query',
  'failed',
  900,
  120,
  'Search test failed: empty query should show all widgets.'
);
seedCommand(
  widgetsFolder,
  'acme/widgets',
  8,
  'Keep search filters in the URL',
  'ready for review',
  1200,
  180
);
seedCommand(
  widgetsFolder,
  'acme/widgets',
  9,
  'Add keyboard navigation to search results',
  'merged',
  1800,
  300
);
seedCommand(
  gadgetsFolder,
  'acme/gadgets',
  10,
  'Preserve query params across reloads',
  'merged',
  2100,
  420
);
