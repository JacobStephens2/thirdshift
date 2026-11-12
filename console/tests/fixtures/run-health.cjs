const { readFile } = require('node:fs/promises');
const { join } = require('node:path');
const { replaceRecord } = require('../records.cjs');

// A live writer plus completed implement stages with a 12–17 min Usual range.
async function liveRunWithUsualRange(records) {
  const stat = await readFile(`/proc/${process.pid}/stat`, 'utf8');
  const start = (await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim()
    + ':' + stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19];
  const live = {
    version: 1, kind: 'Run', repository: 'acme/widgets', hostname: 'fixture',
    process_id: process.pid, process_start: start, harness: 'codex', ended_at: null, outcome: null
  };
  for (const [number, minutes] of [[10, 10], [11, 14], [12, 20]]) {
    const ended = new Date(Date.now() - 7200000).toISOString();
    const began = new Date(Date.parse(ended) - minutes * 60000).toISOString();
    await replaceRecord(join(records, `${number}.command.json`), JSON.stringify({
      ...live, name: `widgets#${number}`,
      issue: { number, title: 'History', url: `https://github.com/acme/widgets/issues/${number}` },
      started_at: began, ended_at: ended, outcome: 'merged',
      stages: [{ name: 'implement', began_at: began, session_log: null }]
    }));
  }
  return live;
}

module.exports = { liveRunWithUsualRange };
