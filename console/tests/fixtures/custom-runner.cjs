const { mkdir, writeFile } = require('node:fs/promises');
const { join } = require('node:path');
const experiments = require('./experiments.cjs');

function record() {
  const value = experiments.record('paused');
  value.title = 'Compare Custom runners';
  value.baseline_ref = value.candidate_ref = null;
  value.suites = ['process'];
  value.bar.trials_per_task = { process: 1 };
  value.tasks = [value.tasks[0]];
  value.tasks[0].suite = value.tasks[0].manifest.session = 'process';
  for (const [arm, name, digest, dirty] of [
    ['baseline', 'outside-review', 'd'.repeat(64), false],
    ['candidate', 'outside-candidate<script>', 'e'.repeat(64), true],
  ]) {
    value[arm] = { commit: null, harness: null, model: null, effort: null, runner: name, runner_digest: digest };
    value.prepared[arm] = { commit: null, binary: '', harness: null, runner: {
      name, directory: '/private/frozen', files: {},
      manifest: { command: ['outside'], files: {}, git_exclude: [], roles: { 'inspect<role>': { model: 'role-model', effort: 'high' } } },
      provenance: { 'runner.py': { repository: '/private/source', origin: 'https://example.test/workflow', path: 'runner.py', commit: 'f'.repeat(40), dirty } },
    } };
  }
  return value;
}

async function save(home) {
  const value = record();
  const folder = await experiments.save(home, value);
  await mkdir(join(folder, 'results'), { recursive: true });
  for (const arm of ['baseline', 'candidate']) {
    const result = experiments.result('alpha', arm, 0);
    result.suite = 'process';
    await writeFile(join(folder, 'results', `${experiments.key('alpha', arm, 0, 'process')}.json`), JSON.stringify(result));
  }
  await mkdir(join(folder, 'pairs'), { recursive: true });
  const side = { diff: [{ path: 'src/count.py', test: false, patch: '+def count(): return 2\n' }], test_files: [], fails_on_base: null, passes_on_other: null, disagreement: false, base: [], cross: [], error: null };
  await writeFile(join(folder, 'pairs', `${pair}.json`), JSON.stringify({ suite: 'process', task: 'alpha', trial: 0, baseline: side, candidate: side }));
  return folder;
}
const pair = experiments.key('alpha', 'pair', 0, 'process');
module.exports = { record, save, pair };
