/* PROTOTYPE (#1019), throwaway.
   Invented Experiments in the shape `thirdshift experiments --json` prints (the
   View in src/evals/experiment/store.rs on origin/issue-977), plus a `proto`
   block for facts the record doesn't hold yet: refs, a title, the end time and
   running Trials. Every name, commit, reason and number here is made up; real
   Eval tasks stay private on the host (docs/eval-tasks.md). */
(function () {
  'use strict';

  // ---- Seeded randomness, so every load shows the same Experiments.
  function prng(seed) {
    let a = seed >>> 0;
    return function () {
      a = (a + 0x6D2B79F5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  const between = (r, lo, hi) => lo + (hi - lo) * r();
  const hex = (r, n) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join('');
  const uuid = r => `${hex(r, 8)}-${hex(r, 4)}-4${hex(r, 3)}-a${hex(r, 3)}-${hex(r, 12)}`;

  // ---- The Verdict, by src/verdict.rs's method: a Dirichlet over the values
  // one Eval task's difference can take. A different generator from the Rust
  // one, so bounds differ in the third decimal; the rules are the same.
  // Box-Muller, keeping the second value of each pair.
  let spare = null;
  function normal(r) {
    if (spare !== null) { const z = spare; spare = null; return z; }
    let u = 0;
    while (u === 0) u = r();
    const radius = Math.sqrt(-2 * Math.log(u)), angle = 2 * Math.PI * r();
    spare = radius * Math.sin(angle);
    return radius * Math.cos(angle);
  }
  function gamma(r, shape) {
    if (shape < 1) return gamma(r, shape + 1) * Math.pow(r(), 1 / shape);
    const d = shape - 1 / 3, c = 1 / Math.sqrt(9 * d);
    for (;;) {
      let x, v;
      do { x = normal(r); v = 1 + c * x; } while (v <= 0);
      v = v * v * v;
      const u = r();
      if (u < 1 - 0.0331 * x ** 4 || Math.log(u) < 0.5 * x * x + d * (1 - v + Math.log(v))) return d * v;
    }
  }
  function intervalOf(alpha, values, draws, level, r) {
    const out = new Float64Array(draws);
    for (let i = 0; i < draws; i++) {
      let sum = 0, mean = 0;
      for (let j = 0; j < alpha.length; j++) { const g = gamma(r, alpha[j]); sum += g; mean += g * values[j]; }
      out[i] = mean / sum;
    }
    out.sort();
    const q = p => { const h = (draws - 1) * p, lo = Math.floor(h), hi = Math.min(lo + 1, draws - 1); return out[lo] + (h - lo) * (out[hi] - out[lo]); };
    const tail = (1 - level) / 2;
    return { lower: q(tail), upper: q(1 - tail) };
  }
  const decide = (interval, margin) =>
    interval.upper < 0 ? 'fail' : interval.lower > margin && interval.upper >= 0 ? 'pass' : 'inconclusive';

  // "How many more Eval tasks": simulate more tasks from the posterior and find
  // the fewest with at least a 0.8 chance of a clear Verdict. Coarse budget.
  function moreTasks(counts, alpha, values, bar, r) {
    const k = counts.length, probe = [1, 2, 4, 6, 8, 12, 16, 20, 25, 30, 40, 50, 60, 80, 100, 130, 160, 200];
    const chance = n => {
      let clear = 0;
      const sims = 20;
      for (let s = 0; s < sims; s++) {
        let sum = 0;
        const pi = alpha.map(a => { const g = gamma(r, a); sum += g; return g; }).map(g => g / sum);
        const extra = counts.slice();
        for (let t = 0; t < n; t++) {
          let u = r(), j = 0;
          while (j < k - 1 && u > pi[j]) { u -= pi[j]; j++; }
          extra[j]++;
        }
        const post = extra.map(c => c + bar.prior / k);
        if (decide(intervalOf(post, values, 100, bar.credible_level, r), bar.margin) !== 'inconclusive') clear++;
      }
      return clear / sims;
    };
    const points = [];
    let found = null;
    for (const n of probe) {
      const p = chance(n);
      points.push([n, p]);
      if (p >= 0.8) { found = n; break; }
    }
    const last = points[points.length - 1][0];
    const clear = [];
    for (let n = 1; n <= last; n++) {
      const after = points.findIndex(([m]) => m >= n);
      const [n1, p1] = after > 0 ? points[after - 1] : [0, 0];
      const [n2, p2] = points[after];
      clear.push(+(p1 + (p2 - p1) * (n - n1) / (n2 - n1 || 1)).toFixed(3));
    }
    return { target_probability: 0.8, additional_tasks: found, clear_probabilities: clear };
  }

  // The prototype draws 2,500 times, not the Bar's 20,000, to load quickly;
  // the bounds move by a few tenths of a point.
  const DRAWS = 2500;
  // Experiments share count vectors (every 9-task suite with no wrong findings
  // has the same one), so assessments are memoized.
  const memo = new Map();
  function assess(counts, bar, seed, predictions) {
    // The interval comes first from the same draws either way, so a decision
    // can reuse an assessment made with predictions.
    const key = flag => `${seed}:${flag}:${counts.join(',')}`;
    const known = memo.get(key(predictions)) || (!predictions && memo.get(key(true))) || assessOnce(counts, bar, seed, predictions);
    memo.set(key(predictions), known);
    return { ...known, difference_counts: counts.slice() };
  }
  function assessOnce(counts, bar, seed, predictions) {
    const k = counts.length, trials = (k - 1) / 2;
    const values = counts.map((_, i) => (i - trials) / trials);
    const alpha = counts.map(c => c + bar.prior / k);
    const r = prng(seed * 7919 + 17);
    const interval = intervalOf(alpha, values, Math.min(bar.draws, DRAWS), bar.credible_level, r);
    const verdict = decide(interval, bar.margin);
    return {
      difference_counts: counts.slice(),
      interval,
      verdict,
      more_tasks: predictions && verdict === 'inconclusive' ? moreTasks(counts, alpha, values, bar, r) : null,
    };
  }

  const effective = score => {
    const latest = score.rechecks && score.rechecks[score.rechecks.length - 1];
    return {
      passed: latest ? latest.passed : score.passed,
      wrong_finding: latest && latest.wrong_finding != null ? latest.wrong_finding : score.wrong_finding,
    };
  };

  // One suite's difference counts. `flip` names one result whose raw Score
  // flips, as rechecks::assess tries each in turn.
  function tally(experiment, tasks, suite, trials, flip) {
    const counts = new Array(2 * trials + 1).fill(0), wrong = new Array(2 * trials + 1).fill(0);
    let retained = 0, dropped = 0, allB = 0, allC = 0;
    for (const task of tasks) {
      const results = experiment.results.filter(x => x.suite === suite && x.task === task);
      if (results.some(x => x.outcome === 'void')) { dropped++; continue; }
      const scoreOf = x => {
        const s = effective(x.score);
        return flip && flip.result === x ? { ...s, [flip.field]: !x.score[flip.field] } : s;
      };
      const arm = name => results.filter(x => x.arm === name).map(scoreOf);
      const b = arm('baseline'), c = arm('candidate');
      const pb = b.filter(s => s.passed).length, pc = c.filter(s => s.passed).length;
      counts[pc + trials - pb]++;
      wrong[b.filter(s => s.wrong_finding).length + trials - c.filter(s => s.wrong_finding).length]++;
      allB += pb === trials; allC += pc === trials;
      retained++;
    }
    return { counts, wrong, retained, dropped, allB, allC };
  }

  const combine = (decisions, dropped, total) => dropped > Math.floor(total / 10) ? 'inconclusive'
    : decisions.includes('fail') ? 'fail' : decisions.includes('inconclusive') ? 'inconclusive' : 'pass';

  // grading::evaluate plus verdict::evaluate, per suite and for the Experiment.
  function judge(experiment, tasksBySuite, predictions = true) {
    const bar = experiment.bar;
    const suites = {};
    let total = 0, dropped = 0;
    for (const [suite, trials] of Object.entries(bar.trials_per_task)) {
      const t = tally(experiment, tasksBySuite[suite], suite, trials, null);
      total += tasksBySuite[suite].length; dropped += t.dropped;
      const statistics = { ...bar, trials_per_task: trials, prediction: { simulations: 200, interval_draws: 512, max_additional_tasks: 2000 } };
      delete statistics.timeout_seconds; delete statistics.void_rules;
      const accuracy = assess(t.counts, bar, bar.seed, predictions);
      const wrongFindings = assess(t.wrong, bar, bar.seed + 1, predictions);
      suites[suite] = {
        bar: statistics,
        thirdshift_version: experiment.proto.version,
        method: 'paired-dirichlet-v1; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles',
        accuracy,
        wrong_findings: wrongFindings,
        verdict: combine([accuracy.verdict, wrongFindings.verdict], t.dropped, tasksBySuite[suite].length),
        retained_tasks: t.retained,
        dropped_tasks: t.dropped,
        pass_cubed: trials === 3 && t.retained > 0 ? { baseline: t.allB / t.retained, candidate: t.allC / t.retained } : null,
      };
    }
    const decision = combine(Object.values(suites).map(s => s.verdict), dropped, total);
    const borderline = experiment.results.filter(x => x.score && x.score.borderline).length;
    return { verdict: decision, provisional_verdict: decision, suites, borderline_trials: borderline };
  }

  // grading::decision with one flipped Score. A flip moves one Eval task's
  // difference by one step, so decisions are cached by counts.
  function decisionWith(experiment, tasksBySuite, flip, cache) {
    const bar = experiment.bar, decisions = [];
    let total = 0, dropped = 0;
    for (const [suite, trials] of Object.entries(bar.trials_per_task)) {
      const t = tally(experiment, tasksBySuite[suite], suite, trials, flip);
      total += tasksBySuite[suite].length; dropped += t.dropped;
      const decide = (counts, seed) => {
        const key = `${seed}:${counts.join(',')}`;
        if (!cache.has(key)) cache.set(key, assess(counts, bar, seed, false).verdict);
        return cache.get(key);
      };
      decisions.push(combine([decide(t.counts, bar.seed), decide(t.wrong, bar.seed + 1)], t.dropped, tasksBySuite[suite].length));
    }
    return combine(decisions, dropped, total);
  }

  // ---- Invented Eval tasks.
  const CODE = [
    ['pager-off-by-one', "names next_page's `<=` and the empty last page it returns", 'said the pager change looked correct', 'a leak in the page cache', 'questions the loop bound, but never says the last page comes back empty'],
    ['cache-key-collision', 'shows two tenants sharing one cache key once the prefix is dropped', 'reviewed the TTL change and never looked at the key', 'a race in cache warm-up', 'asks whether keys can collide, without showing that they do'],
    ['stale-lock-cleanup', 'points out that cleanup deletes a lock a live process still holds', 'approved the cleanup loop', "that the lock file's mode is too open", 'calls cleanup "risky" without naming the live holder'],
    ['utf8-truncation', 'finds the byte slice that splits a multi-byte character', 'checked lengths but not character boundaries', 'SQL injection in the title query', 'mentions Unicode titles, but not the panic'],
    ['retry-without-backoff', 'names the retry loop that never waits, and what it does under rate limits', 'commented only on log wording', 'a missing timeout on the first request', 'suggests backoff "for politeness" without calling it a bug'],
    ['midnight-rollover', 'shows reports dated a day early for runs that end after midnight UTC', 'tested only daytime timestamps', 'that the locale format is wrong', 'flags time zones in general, not the rollover'],
    ['unawaited-flush', 'notices the buffer flush is never awaited before exit', 'approved the shutdown path', 'a deadlock in the writer', 'asks about shutdown order without finding the lost writes'],
    ['path-join-traversal', 'shows `../` in an upload name escaping the storage folder', 'called the join safe', 'a symlink race in cleanup', 'recommends cleaning names without showing the escape'],
    ['double-close-handle', 'finds the second close on the error path', 'reviewed only the happy path', 'a file-descriptor leak on success', 'notes two closes but calls them harmless'],
  ].map(([name, caught, missed, wrong, close]) => ({ name, caught, missed, wrong, close }));

  const GAPS = ['what happens when the token expires mid-export', 'who can see a draft before it is shared', 'how the migration rolls back', 'which time zone the daily digest uses', 'the empty state for a new account', 'a rate limit on bulk export', "how a deleted user's comments read", 'the size cap on uploads', 'behaviour while offline', 'the retry limit', 'who hears when a payment fails', 'how long audit entries are kept', 'what an archived project still sends', 'the order of webhook deliveries', 'the error a reader sees on a stale link', 'how two editors merge changes', 'what the import does with duplicates', 'whether search covers attachments', 'how a renamed field reaches old clients', 'what a cancelled invite leaves behind'];
  const SPEC = ['widgets-12', 'widgets-19', 'widgets-27', 'widgets-33', 'widgets-41', 'ledger-4', 'ledger-9', 'ledger-15', 'ledger-22', 'ledger-31', 'atlas-103', 'atlas-118', 'atlas-126', 'atlas-140', 'atlas-152', 'orchard-7', 'orchard-11', 'orchard-18', 'orchard-24', 'orchard-30', 'beacon-56', 'beacon-61', 'beacon-70', 'beacon-77', 'beacon-85', 'quill-2', 'quill-6', 'quill-13', 'quill-21', 'quill-28']
    .map((name, i) => ({ name, a: GAPS[i % 20], b: GAPS[(i * 7 + 3) % 20], wrong: GAPS[(i * 3 + 11) % 20] }));
  const TASKS = { 'code-review': CODE, 'spec-review': SPEC };
  const TASK_NAMES = { 'code-review': CODE.map(t => t.name), 'spec-review': SPEC.map(t => t.name) };

  function reason(suite, task, code) {
    if (suite === 'code-review') {
      switch (code) {
        case 'P': case 'R': case 'M': return `Caught it: ${task.caught}.`;
        case 'p': return `Probably caught it: ${task.close}. Borderline.`;
        case 'F': case 'r': return `Missed it: ${task.missed}.`;
        case 'f': return `Close, but not a catch: ${task.close}. Borderline.`;
        case 'W': return `Caught it, but also reported ${task.wrong}, which isn't a bug. The criteria allow no wrong findings.`;
        case 'w': return `Caught it, and also reported ${task.wrong}, which isn't a bug.`;
        default: return 'Left no review report to grade.';
      }
    }
    switch (code) {
      case 'P': case 'R': case 'M': return `Found both known real gaps: ${task.a}, and ${task.b}.`;
      case 'p': return `Found ${task.a}; its note on ${task.b} is vague enough to argue. Borderline.`;
      case 'F': case 'r': return `Found ${task.a}, but missed ${task.b}.`;
      case 'f': return `Hints at ${task.a} without saying the Spec leaves it open. Borderline.`;
      case 'W': return `Missed ${task.b}, and flagged ${task.wrong} as missing, though the Spec covers it.`;
      case 'w': return `Found both known gaps, but also flagged ${task.wrong} as missing, though the Spec covers it.`;
      default: return 'Left no review report to grade.';
    }
  }

  const VOID_TEXT = {
    session_unavailable: 'the provider returned a server error before any work',
    usage_limit: 'a plan-limit warning stopped the session',
    rate_limit: 'the provider rate-limited the session before any work',
    interrupted: 'the Experiment was interrupted',
    repository_changed: "the Trial's repository changed underneath it",
    sandbox_unavailable: 'the sandbox did not start',
    host_killed: 'the host killed the session',
    memory_cap: "the session hit the Trial's memory cap",
  };

  // One Trial result, as workers.rs writes it. Codes:
  //   P pass · p borderline pass · F fail · f borderline fail
  //   W fail with a wrong finding · w pass with a wrong finding
  //   T timed out (a fail) · X no report (a fail)
  //   R pass after a rate-limit Void attempt · r fail after a server-error Void attempt
  //   M pass after a memory-cap Void attempt · V void after every re-run
  // A trailing `*` marks a Score you rechecked.
  function trial(ctx, suite, task, arm, index, code, graded) {
    const r = ctx.r;
    const rechecked = code.includes('*');
    code = code[0];
    const minutes = suite === 'code-review' ? between(r, 4, 14) : between(r, 6, 21);
    const attempt = (outcome, voidCause, cause, ms) => {
      const session = outcome === 'void' && ['sandbox_unavailable'].includes(voidCause) ? null : uuid(r);
      const total = Math.round(ms ?? minutes * 60000);
      return {
        version: 1,
        outcome,
        timings: { total_ms: total, session_ms: Math.max(0, total - Math.round(between(r, 9000, 40000))) },
        void_cause: voidCause,
        cause,
        session_id: session,
        log: session ? `${ctx.dir}/trials/${task.name}--${arm}--${index}/attempt/${session}.logs.jsonl` : null,
        model_evidence: null,
        reports: outcome === 'succeeded' && code !== 'X'
          ? { standards: `${ctx.dir}/trials/${task.name}--${arm}--${index}/standards.md`, spec: suite === 'spec-review' ? `${ctx.dir}/trials/${task.name}--${arm}--${index}/spec.md` : null }
          : { standards: null, spec: null },
      };
    };
    const attempts = [];
    if (code === 'R') attempts.push(attempt('void', 'rate_limit', VOID_TEXT.rate_limit, between(r, 20000, 90000)));
    if (code === 'r') attempts.push(attempt('void', 'session_unavailable', VOID_TEXT.session_unavailable, between(r, 15000, 60000)));
    if (code === 'M') attempts.push(attempt('void', 'memory_cap', VOID_TEXT.memory_cap, between(r, 180000, 420000)));
    if (code === 'V') for (let i = 0; i < 3; i++) attempts.push(attempt('void', 'sandbox_unavailable', VOID_TEXT.sandbox_unavailable, between(r, 2000, 6000)));
    else if (code === 'T') attempts.push(attempt('failed', null, 'timed out after 30 minutes', 1800000));
    else attempts.push(attempt('succeeded', null, null));
    const last = attempts[attempts.length - 1];
    const passes = 'PpRMw'.includes(code);
    const checks = [
      { command: null, passed: last.outcome === 'succeeded', reason: `Trial ${last.outcome}: ${last.cause || 'session completed'}` },
      { command: null, passed: !!(last.reports.standards || last.reports.spec), reason: 'review must leave at least one report' },
    ];
    const score = {
      passed: checks.every(c => c.passed),
      wrong_finding: false,
      borderline: false,
      model: null,
      checks,
    };
    if (graded && last.outcome !== 'void') {
      score.model = { label: null, passed: passes, reason: reason(suite, task, code), wrong_finding: 'Ww'.includes(code), borderline: 'pf'.includes(code) };
      score.passed = checks.every(c => c.passed) && passes;
      score.wrong_finding = score.model.wrong_finding;
      score.borderline = score.model.borderline;
    }
    if (rechecked) {
      score.rechecks = [{ passed: !score.passed, reason: ctx.recheckReason || 'Rechecked against the criteria.', wrong_finding: null, recorded_at: ctx.recheckAt }];
    }
    const result = { suite, task: task.name, arm, trial: index, outcome: last.outcome, score, attempts };
    Object.defineProperty(result, 'proto_code', { value: code, enumerable: false });
    return result;
  }

  // Blind packet labels: trial-1 … trial-n, shuffled across both Arms per task.
  function label(results, r) {
    const byTask = {};
    for (const x of results) (byTask[`${x.suite}/${x.task}`] ||= []).push(x);
    for (const group of Object.values(byTask)) {
      const labels = group.map((_, i) => `trial-${i + 1}`);
      for (let i = labels.length - 1; i > 0; i--) { const j = Math.floor(r() * (i + 1)); [labels[i], labels[j]] = [labels[j], labels[i]]; }
      group.forEach((x, i) => { if (x.score.model) x.score.model.label = labels[i]; });
    }
  }

  const trialKey = (x, id) => {
    // The real key hashes suite NUL task with SHA-256; a stand-in with its shape.
    let h = 0x811c9dc5;
    for (const ch of `${id}${x.suite}\0${x.task}`) { h ^= ch.charCodeAt(0); h = Math.imul(h, 16777619) >>> 0; }
    const r = prng(h);
    return `${hex(r, 64)}--${x.arm}--${x.trial}`;
  };

  const MODELS = {
    'claude-opus-5-5': { input: [140e3, 420e3], output: [7e3, 26e3], cacheRead: [0.6e6, 2.8e6], cacheWrite: [40e3, 160e3], cost: [0.9, 3.1] },
    'claude-sonnet-5-5': { input: [150e3, 460e3], output: [8e3, 30e3], cacheRead: [0.7e6, 3.1e6], cacheWrite: [40e3, 170e3], cost: [0.28, 1.05] },
    'gpt-6.1-sol': { input: [180e3, 520e3], output: [9e3, 34e3], cacheRead: [0.9e6, 3.6e6], cacheWrite: [0, 0], cost: null },
  };

  function usageOf(results, setup, r) {
    const m = MODELS[setup.model], models = {};
    const row = models[setup.model] = { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: m.cacheWrite[1] ? 0 : null, cost_usd: m.cost ? 0 : null };
    for (const x of results) for (const a of x.attempts) {
      if (!a.session_id) continue;
      const share = a.outcome === 'void' ? 0.15 : 1;
      row.input_tokens += Math.round(between(r, ...m.input) * share);
      row.output_tokens += Math.round(between(r, ...m.output) * share);
      row.cache_read_tokens += Math.round(between(r, ...m.cacheRead) * share);
      if (row.cache_write_tokens != null) row.cache_write_tokens += Math.round(between(r, ...m.cacheWrite) * share);
      if (row.cost_usd != null) row.cost_usd += between(r, ...m.cost) * share;
    }
    if (row.cost_usd != null) row.cost_usd = +row.cost_usd.toFixed(2);
    return { models, cost_usd: row.cost_usd };
  }

  function minutesOf(results) {
    return +(results.reduce((sum, x) => sum + x.attempts.reduce((s, a) => s + a.timings.total_ms, 0), 0) / 60000).toFixed(3);
  }

  function voidsOf(results) {
    const arm = () => ({ count: 0, causes: {} });
    const voids = { baseline: arm(), candidate: arm() };
    for (const x of results) for (const a of x.attempts) if (a.outcome === 'void') {
      voids[x.arm].count++;
      voids[x.arm].causes[a.void_cause] = (voids[x.arm].causes[a.void_cause] || 0) + 1;
    }
    return voids;
  }

  const CAUSES = ['session_unavailable', 'usage_limit', 'rate_limit', 'interrupted', 'repository_changed', 'sandbox_unavailable', 'host_killed', 'memory_cap'];
  const HARNESS = {
    claude: { path: '/home/jacob/.local/bin/claude', version: '2.4.12 (Claude Code)' },
    codex: { path: '/home/jacob/.local/bin/codex', version: 'codex-cli 0.131.0' },
  };

  // ---- The Experiments.
  function experiment(spec) {
    const r = prng(spec.seed);
    const id = hex(r, 64);
    const dir = `/home/jacob/.thirdshift/evals/experiments/${id}`;
    const trialsPerTask = {};
    for (const suite of spec.suites) trialsPerTask[suite] = suite === 'code-review' ? 3 : 1;
    const setups = {
      baseline: { commit: spec.baseline.commit, harness: spec.baseline.harness, model: spec.baseline.model, effort: spec.baseline.effort },
      candidate: { commit: spec.candidate.commit, harness: spec.candidate.harness, model: spec.candidate.model, effort: spec.candidate.effort },
    };
    const ctx = { r, dir, recheckReason: spec.recheckReason, recheckAt: spec.recheckAt };
    const graded = spec.state === 'completed';
    const all = [];
    const order = [];
    for (const suite of spec.suites) {
      const n = trialsPerTask[suite];
      TASKS[suite].forEach((task, t) => {
        const codes = spec.codes[suite](t, task);
        for (let i = 0; i < n; i++) for (const arm of ['baseline', 'candidate']) {
          const code = codes[arm][i];
          order.push({ suite, task, arm, index: i, code });
        }
      });
    }
    const done = spec.done ?? order.length;
    for (const item of order.slice(0, done)) all.push(trial(ctx, item.suite, item.task, item.arm, item.index, item.code, graded));
    label(all, r);
    const tasks = {};
    for (const suite of spec.suites) tasks[suite] = TASK_NAMES[suite];
    const view = {
      id,
      baseline: setups.baseline,
      candidate: setups.candidate,
      suites: spec.suites.slice(),
      bar: {
        margin: -0.1, credible_level: 0.95, prior: 1, seed: 0, draws: 20000,
        trials_per_task: trialsPerTask, timeout_seconds: 1800,
        void_rules: { retries: 2, causes: CAUSES.slice() },
      },
      prepared: {
        format: 1,
        baseline: { commit: setups.baseline.commit, binary: `/home/jacob/.thirdshift/evals/builds/${setups.baseline.commit}/thirdshift`, harness: { name: setups.baseline.harness, ...HARNESS[setups.baseline.harness] } },
        candidate: { commit: setups.candidate.commit, binary: `/home/jacob/.thirdshift/evals/builds/${setups.candidate.commit}/thirdshift`, harness: { name: setups.candidate.harness, ...HARNESS[setups.candidate.harness] } },
      },
      state: spec.state,
      created_at: spec.created,
      results: all,
      verdict: null,
      pending_rechecks: [],
      minutes: { baseline: minutesOf(all.filter(x => x.arm === 'baseline')), candidate: minutesOf(all.filter(x => x.arm === 'candidate')) },
      usage: { baseline: usageOf(all.filter(x => x.arm === 'baseline'), setups.baseline, r), candidate: usageOf(all.filter(x => x.arm === 'candidate'), setups.candidate, r) },
      voids: voidsOf(all),
      findings: [],
      proto: {
        key: spec.key,
        kind: spec.kind,
        name: spec.name,
        title: spec.title,
        candidate_ref: spec.candidate.ref,
        baseline_ref: spec.baseline.ref,
        ended_at: spec.ended || null,
        command: `experiment-${spec.created.slice(0, 19).replace(/[-:]/g, '')}`,
        command_line: spec.cli,
        version: spec.version,
        tasks,
        order: order.map(item => ({ suite: item.suite, task: item.task.name, arm: item.arm, trial: item.index, code: item.code })),
        running: spec.state === 'running' ? order.slice(done, done + 2).map(item => ({ suite: item.suite, task: item.task.name, arm: item.arm, trial: item.index, started_ms_ago: Math.round(between(r, 60000, 420000)) })) : [],
        stage: spec.stage || null,
        pause: spec.pause || null,
        ctx,
      },
    };
    if (spec.pause) {
      // The Trial in flight at the warning is void; pending.json keeps it, so
      // the View counts it though it has no result.
      const cut = order[done];
      view.voids[cut.arm].count++;
      view.voids[cut.arm].causes.usage_limit = (view.voids[cut.arm].causes.usage_limit || 0) + 1;
      view.proto.pause.trial = { suite: cut.suite, task: cut.task.name, arm: cut.arm, trial: cut.index };
    }
    for (const [arm, voids] of Object.entries(view.voids)) {
      const cap = voids.causes.memory_cap || 0;
      if (cap > 1) view.findings.push(`${arm[0].toUpperCase()}${arm.slice(1)} Arm repeatedly hit its memory cap (${cap} Void Trials)`);
    }
    if (graded) finish(view);
    if (graded && spec.agree) {
      // Rechecks you already recorded, agreeing with the Grader.
      const agreed = spec.agree === 'all' ? view.pending_rechecks : view.pending_rechecks.slice(0, spec.agree);
      const notes = ['Agree with the Grader.', 'Agree: the report says it in its first finding.', 'Agree; no wrong findings in the report.', 'Agree, though the wording is loose.'];
      const start = Date.parse(spec.ended);
      agreed.forEach((pending, i) => {
        const x = view.results.find(o => o.suite === pending.suite && o.task === pending.task && o.arm === pending.arm && o.trial === pending.trial);
        x.score.rechecks = [{ passed: x.score.passed, reason: notes[i % notes.length], wrong_finding: x.score.wrong_finding, recorded_at: new Date(start + (i + 3) * 97000).toISOString() }];
      });
      finish(view);
    }
    return view;
  }

  // Grade, judge and list what waits on a recheck, as store.rs and rechecks.rs do.
  function finish(view) {
    view.verdict = judge(view, view.proto.tasks);
    const original = view.verdict.provisional_verdict, cache = new Map();
    view.pending_rechecks = [];
    for (const x of view.results) {
      if (view.results.some(o => o.suite === x.suite && o.task === x.task && o.outcome === 'void')) continue;
      if (!x.score || (x.score.rechecks && x.score.rechecks.length)) continue;
      const flips = ['passed', 'wrong_finding'].filter(field => decisionWith(view, view.proto.tasks, { result: x, field }, cache) !== original);
      if (x.score.borderline || flips.length) {
        const pending = { id: trialKey(x, view.id), suite: x.suite, task: x.task, arm: x.arm, trial: x.trial, borderline: x.score.borderline, decisive: flips.length > 0 };
        // Which flip would change the Verdict. Pending doesn't record it yet.
        Object.defineProperty(pending, 'proto_flips', { value: flips, enumerable: false });
        view.pending_rechecks.push(pending);
      }
    }
    if (view.pending_rechecks.length) view.verdict.verdict = 'waiting_on_rechecks';
    view.state = 'completed';
  }

  const ARMS = {
    opus: { harness: 'claude', model: 'claude-opus-5-5', effort: 'high' },
    sonnet: { harness: 'claude', model: 'claude-sonnet-5-5', effort: 'high' },
    sol: { harness: 'codex', model: 'gpt-6.1-sol', effort: 'max' },
  };
  // One string per Arm: a letter per Trial, each followed by any modifiers.
  const tokens = s => s.match(/[A-Za-z]\*?/g);
  const both = (b, c) => ({ baseline: tokens(b), candidate: tokens(c) });

  const specs = [
    {
      key: 'running', seed: 1302, kind: 'branch', name: 'issue-1302', state: 'running', stage: 'Trials', cli: 'thirdshift experiment issue-1302',
      title: 'Ask the code review for a failing test before a finding (#1302)',
      created: '2026-10-24T15:26:04-04:00', version: '0.15.0',
      baseline: { ref: 'main at the branch point', commit: 'd07948c1e2b4a6f1c3d5e7f9a1b3c5d7e9f1a3b5', ...ARMS.opus },
      candidate: { ref: 'issue-1302', commit: '7be20d4a9c1f3e5d7b9a1c3e5f7a9b1d3c5e7f90', ...ARMS.opus },
      suites: ['code-review'], done: 31,
      codes: { 'code-review': t => [both('PPF', 'PPP'), both('PPP', 'PPP'), both('FFP', 'PFP'), both('PFP', 'PPP'), both('PPP', 'PPp'), both('FFF', 'FFP'), both('PPP', 'PPP'), both('PPP', 'PPP'), both('PFF', 'PPF')][t] },
    },
    {
      key: 'paused', seed: 1311, kind: 'branch', name: 'issue-1311', state: 'paused', stage: 'Trials', cli: 'thirdshift experiment issue-1311 --baseline-harness codex --candidate-harness codex',
      title: 'Give Codex reviews the implement session sub-agent rules (#1311)',
      created: '2026-10-24T11:02:51-04:00', version: '0.15.0',
      baseline: { ref: 'main at the branch point', commit: 'c4a9e1f0b2d3c4e5f6a7b8c9d0e1f2a3b4c5d6e7', ...ARMS.sol },
      candidate: { ref: 'issue-1311', commit: '0f3d8b6a2c4e6f8a0b2c4d6e8f0a2b4c6d8e0f1a', ...ARMS.sol },
      suites: ['code-review'], done: 14,
      pause: { cause: 'usage_limit', harness: 'codex', at: '2026-10-24T11:58:40-04:00', text: 'Codex reported a plan-limit warning, so no new Trial started and the one in flight became void.' },
      codes: { 'code-review': t => [both('PPP', 'PPF'), both('PFP', 'PPP'), both('FPF', 'FFP'), both('PPP', 'PPP'), both('PPP', 'PPP'), both('FFF', 'FFF'), both('PPP', 'PPP'), both('PPP', 'PPP'), both('PPF', 'PFF')][t] },
    },
    {
      key: 'waiting', seed: 1320, kind: 'branch', name: 'issue-1320', state: 'completed', cli: 'thirdshift experiment issue-1320',
      title: 'Have the Spec review read the ADRs a Spec cites (#1320)',
      created: '2026-10-24T08:47:37-04:00', ended: '2026-10-24T12:20:09-04:00', version: '0.15.0',
      baseline: { ref: 'main at the branch point', commit: 'd07948c1e2b4a6f1c3d5e7f9a1b3c5d7e9f1a3b5', ...ARMS.opus },
      candidate: { ref: 'issue-1320', commit: 'b58d2e0a7c4f1b8e5d2a9c6f3b0e7d4a1c8f5b2e', ...ARMS.opus },
      suites: ['spec-review'],
      recheckReason: 'It names the missing rollback in its third finding; that is the known gap.', recheckAt: '2026-10-24T12:41:52-04:00',
      agree: 6,
      codes: {
        'spec-review': t => both(tokens('PPFPPFPPPPF*PPPPPFPPPPPFPPPPPFP')[t], tokens('PPPPPFPPPPPPPpPPFPPPpPPPPPPFPP')[t]),
      },
    },
    {
      key: 'release', seed: 160, kind: 'release', name: 'release v0.16.0', state: 'completed', cli: 'thirdshift experiment --release a41f07b',
      title: 'Release v0.16.0: 23 changes since v0.15.0, 2 of them to Factory skills',
      created: '2026-10-23T13:10:37-04:00', ended: '2026-10-23T17:42:09-04:00', version: '0.16.0',
      baseline: { ref: 'v0.15.0', commit: 'd07948c1e2b4a6f1c3d5e7f9a1b3c5d7e9f1a3b5', ...ARMS.opus },
      candidate: { ref: 'v0.16.0 (being released)', commit: 'a41f07b3c9e2d4f6a8b0c2e4f6a8b0d2c4e6f8a0', ...ARMS.opus },
      suites: ['code-review', 'spec-review'],
      recheckReason: 'The report names the boundary in its second paragraph; that is a catch.', recheckAt: '2026-10-23T17:55:12-04:00',
      codes: {
        'code-review': t => [both('PPF', 'PPP'), both('PPP', 'PPP'), both('FFP', 'PPF'), both('PFP', 'PPP'), both('PPP', 'PPP'), both('FFF', 'FFF'), both('PPP', 'PPP'), both('PPR', 'PPP'), both('PF*F', 'PPF')][t],
        'spec-review': t => both('PPFPPFPPPPFPPPPPFPPPPPFPPPPPFP'[t], 'PPPPPFPPPPPPPPPPFPPPPPPPPPPPPP'[t]),
      },
    },
    {
      key: 'inconclusive', seed: 55, kind: 'flags', name: 'Sonnet in place of Opus', state: 'completed', cli: 'thirdshift experiment main --suite code-review --candidate-model claude-sonnet-5-5',
      title: 'Same build, two sets of flags: claude-sonnet-5-5 in place of claude-opus-5-5',
      created: '2026-10-22T19:40:15-04:00', ended: '2026-10-23T00:52:33-04:00', version: '0.15.0',
      baseline: { ref: 'main', commit: 'd07948c1e2b4a6f1c3d5e7f9a1b3c5d7e9f1a3b5', ...ARMS.opus },
      candidate: { ref: 'main', commit: 'd07948c1e2b4a6f1c3d5e7f9a1b3c5d7e9f1a3b5', ...ARMS.sonnet },
      suites: ['code-review'],
      codes: { 'code-review': t => [both('PPP', 'PFP'), both('PPP', 'PPF'), both('PFP', 'PFP'), both('PPP', 'PMP'), both('PPP', 'PPP'), both('FFP', 'FPP'), both('PPP', 'PPM'), both('PPR', 'PPP'), both('PPF', 'PFF')][t] },
    },
    {
      key: 'fail', seed: 1288, kind: 'branch', name: 'issue-1288', state: 'completed', cli: 'thirdshift experiment issue-1288',
      title: 'Shorten the Spec review Session prompt (#1288)',
      created: '2026-10-22T10:05:44-04:00', ended: '2026-10-22T16:37:02-04:00', version: '0.14.1',
      baseline: { ref: 'main at the branch point', commit: '6a2e9f1c3b5d7e9f1a3c5e7a9b1d3f5a7c9e1b3d', ...ARMS.opus },
      candidate: { ref: 'issue-1288', commit: 'e19c4b7d2f5a8c1e4b7d0a3f6c9e2b5d8a1f4c7e', ...ARMS.opus },
      suites: ['spec-review'],
      codes: {
        'spec-review': t => both('PPFPPPFPPPPFPPPPPFPPRPPFPPPPFP'[t], 'FPFwPFFPFPwFPFFPFFwPPFPFFPwFFP'[t]),
      },
    },
    {
      key: 'dropped', seed: 1264, kind: 'branch', name: 'issue-1264', state: 'completed', cli: 'thirdshift experiment issue-1264',
      title: 'Let the Spec review read linked Tickets (#1264)',
      created: '2026-10-21T08:31:09-04:00', ended: '2026-10-21T14:12:55-04:00', version: '0.14.1',
      baseline: { ref: 'main at the branch point', commit: '9d4b2a7e1c6f3a8d5b2e9c6f3a0d7b4e1c8f5a2d', ...ARMS.opus },
      candidate: { ref: 'issue-1264', commit: '3c8e1a6d4f9b2e7c5a0d3f8b6e1c4a9d7b2f5e0c', ...ARMS.opus },
      suites: ['spec-review'],
      codes: {
        'spec-review': t => [4, 11, 19, 26].includes(t) ? both('V', 'P') : both('PPFPPPFPPPPPPPPFPPPPPPFPPPPPPP'[t], 'PPPPPPFPPPPPPPPPPPPPPPPPPPPFPP'[t]),
      },
    },
    {
      key: 'pass', seed: 150, kind: 'release', name: 'release v0.15.0', state: 'completed', cli: 'thirdshift experiment --release d07948c',
      title: 'Release v0.15.0: 31 changes since v0.14.1, 1 of them to the Spec review Session prompt',
      created: '2026-10-20T16:20:03-04:00', ended: '2026-10-20T21:58:47-04:00', version: '0.15.0',
      baseline: { ref: 'v0.14.1', commit: '6a2e9f1c3b5d7e9f1a3c5e7a9b1d3f5a7c9e1b3d', ...ARMS.opus },
      candidate: { ref: 'v0.15.0 (being released)', commit: 'd07948c1e2b4a6f1c3d5e7f9a1b3c5d7e9f1a3b5', ...ARMS.opus },
      suites: ['spec-review'],
      recheckReason: 'The second finding is the missing retention rule. A catch.', recheckAt: '2026-10-20T22:06:31-04:00',
      agree: 'all',
      codes: {
        'spec-review': t => both(tokens('PPFPPPPFPPPPPPPPPPFPPPPPPPPFPP')[t], tokens('PPPPPPPF*PPPPPPPPPPPPPPPPPPPPPP')[t]),
      },
    },
  ];

  const experiments = specs.map(experiment);

  // The live demo replays the running Experiment's remaining Trials, then its
  // model Graders, then its Verdict.
  function advance(view) {
    const p = view.proto;
    const finished = view.results.length;
    if (finished >= p.order.length) return false;
    const item = p.order[finished];
    const task = TASKS[item.suite].find(t => t.name === item.task);
    view.results.push(trial(p.ctx, item.suite, task, item.arm, item.trial, item.code, false));
    const next = p.order.slice(finished + 1, finished + 3);
    p.running = next.map((x, i) => ({ suite: x.suite, task: x.task, arm: x.arm, trial: x.trial, started_ms_ago: i ? 20000 : 140000 }));
    view.minutes = { baseline: minutesOf(view.results.filter(x => x.arm === 'baseline')), candidate: minutesOf(view.results.filter(x => x.arm === 'candidate')) };
    view.usage = { baseline: usageOf(view.results.filter(x => x.arm === 'baseline'), view.baseline, p.ctx.r), candidate: usageOf(view.results.filter(x => x.arm === 'candidate'), view.candidate, p.ctx.r) };
    view.voids = voidsOf(view.results);
    return true;
  }
  function grade(view, suite, task) {
    const t = TASKS[suite].find(x => x.name === task);
    view.results.forEach((x, i) => {
      if (x.suite !== suite || x.task !== task) return;
      const item = view.proto.order.find(o => o.suite === suite && o.task === task && o.arm === x.arm && o.trial === x.trial);
      const graded = trial(view.proto.ctx, suite, t, x.arm, x.trial, item.code, true);
      graded.attempts = x.attempts;
      view.results[i] = graded;
    });
    label(view.results.filter(x => x.suite === suite && x.task === task), view.proto.ctx.r);
  }

  window.PROTO_FIXTURE = {
    now: '2026-10-24T15:40:00-04:00',
    experiments,
    tasks: TASK_NAMES,
    voidText: VOID_TEXT,
    trialKey,
    effective,
    live: { advance, grade, finish },
    reset(key) {
      const i = experiments.findIndex(x => x.proto.key === key);
      experiments[i] = experiment(specs.find(s => s.key === key));
      return experiments[i];
    },
  };
})();
