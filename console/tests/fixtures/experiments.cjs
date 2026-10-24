const { mkdir, writeFile } = require('node:fs/promises');
const { join } = require('node:path');
const { createHash } = require('node:crypto');
const id = 'c'.repeat(64);
const key = (task, arm, trial, suite = 'code-review') => `${createHash('sha256').update(`${suite}\0${task}`).digest('hex')}--${arm}--${trial}`;
const setup = (commit, harness) => ({ commit: commit.repeat(40), harness, model: 'test-model', effort: 'high' });
const tasks = ['alpha', 'beta', 'gamma'].map(name => ({ suite: 'code-review', name, directory: '/private/task', manifest: {
  format: 1, state: 'approved', session: 'code-review', repository: 'private', base: 'a'.repeat(40), change: 'b'.repeat(40), criteria: 'Private criteria', inputs: {}, trial: null
} }));
function record(state = 'running') {
  return { format: 1, id, state, created_at: '2026-10-24T10:00:00Z',
    baseline_ref: 'v1.0', candidate_ref: 'issue-1020', title: 'Improve <script>unsafe()</script> review',
    resume_command: 'thirdshift experiment issue-1020 --baseline v1.0 --suite code-review',
    baseline: setup('a', 'claude'), candidate: setup('b', 'codex'), suites: ['code-review'],
    prepared: { format: 1, baseline: { commit: 'a'.repeat(40), binary: '/private/baseline', harness: { name: 'claude', path: '/private/claude', version: '1.2.3' } }, candidate: { commit: 'b'.repeat(40), binary: '/private/candidate', harness: { name: 'codex', path: '/private/codex', version: '4.5.6' } } },
    bar: { margin: -0.1, pass_level: 0.8, fail_level: 0.95, prior: 1, seed: 7, draws: 20000, trials_per_task: { 'code-review': 3 }, timeout_seconds: 1800, void_rules: { retries: 2, causes: ['memory_cap', 'usage_limit'] } }, tasks: structuredClone(tasks)
  };
}
const assessment = (verdict, lower, upper) => ({ verdict, pass_interval: { lower, upper }, fail_interval: { lower: lower - 0.05, upper: upper + 0.05 }, difference_counts: [0, 0, 1, 2, 0, 0, 0], more_tasks: verdict === 'inconclusive' ? { target_probability: 0.8, additional_tasks: 8, clear_probabilities: [] } : null });
function finish(value) {
  value.state = 'completed'; value.ended_at = '2026-10-24T12:30:00Z';
  value.verdict = { verdict: 'waiting_on_rechecks', provisional_verdict: 'inconclusive', borderline_trials: 2, suites: {
    'code-review': { verdict: 'inconclusive', bar: { ...value.bar, trials_per_task: 3, prediction: { simulations: 200, interval_draws: 512, max_additional_tasks: 2000 } },
      thirdshift_version: 'test-version', method: 'paired-dirichlet-v2', retained_tasks: 3, dropped_tasks: 0, pass_cubed: { baseline: 1/3, candidate: 2/3 },
      accuracy: assessment('pass', -0.07, 0.21), wrong_findings: assessment('inconclusive', -0.16, 0.04)
    }
  } };
  value.pending_rechecks = ['alpha', 'beta'].flatMap(task => [0, 1, 2].map(trial => ({ id: key(task, 'candidate', trial), suite: 'code-review', task, arm: 'candidate', trial, borderline: task === 'alpha' && trial < 2, decisive: true, decisive_flips: ['wrong_finding'] })));
  return value;
}
function result(task, arm, trial) {
  const wrong = arm === 'candidate' && task === 'alpha';
  const passed = task !== 'beta' || arm === 'candidate';
  return { suite: 'code-review', task, arm, trial, outcome: 'succeeded', score: {
    passed, wrong_finding: wrong, borderline: wrong, model: { label: `${task}-blind-${trial}`, passed, wrong_finding: wrong, borderline: wrong, reason: `Grader reason for ${task}: <script>unsafe()</script>` },
    checks: [{ command: ['test', '-f', 'report.md'], passed: true, reason: 'Report exists' }]
  }, attempts: [{ version: 1, outcome: 'succeeded', timings: { total_ms: 60000, session_ms: 59000 }, void_cause: null, cause: null, session_id: null, log: null, model_evidence: null, reports: { standards: null, spec: null } }] };
}
async function save(home, value) {
  const folder = join(home, '.thirdshift/evals/experiments', id);
  await mkdir(folder, { recursive: true });
  await writeFile(join(folder, 'experiment.tmp'), JSON.stringify(value));
  const { rename } = require('node:fs/promises');
  await rename(join(folder, 'experiment.tmp'), join(folder, 'experiment.json'));
  return folder;
}
async function results(home) {
  const folder = join(home, '.thirdshift/evals/experiments', id, 'results');
  await mkdir(folder, { recursive: true });
  for (const task of tasks) for (const arm of ['baseline', 'candidate']) for (let trial = 0; trial < 3; trial++) {
    await writeFile(join(folder, `${key(task.name, arm, trial)}.json`), JSON.stringify(result(task.name, arm, trial)));
  }
}
module.exports = { id, key, record, finish, result, save, results };
