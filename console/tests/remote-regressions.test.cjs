const { test } = require('node:test');
const { execFileSync } = require('node:child_process');
const { resolve } = require('node:path');

const binary = process.env.THIRDSHIFT_BINARY ?? resolve(__dirname, '../../target/debug/thirdshift');
for (const [name, file, ...args] of [
  ['remote Peek fetches at most a 4 KiB log tail', 'remote-preview-budget.py'],
  ['remote child Ticket rows open the exact owning record', 'remote-child-details.py'],
  ['remote preview preserves normalized UTF-8 text', 'remote-child-details.py', '--utf8'],
]) {
  test(name, () => execFileSync('python3', [resolve(__dirname, '../reproductions', file), ...args], {
    env: { ...process.env, THIRDSHIFT_BINARY: binary }, encoding: 'utf8', timeout: 120000,
  }));
}
