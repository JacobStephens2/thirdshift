/* PROTOTYPE (#1019), throwaway. Five layouts for an Experiment's page
   (?variant=A–E) and three for the Experiments list (?list=1–3), over the
   invented Experiments in fixture.js. Not served by the Console; see README.md. */
(function () {
  'use strict';

  const F = window.PROTO_FIXTURE;
  const VARIANTS = [
    ['A', 'Compare table'],
    ['B', 'Report'],
    ['C', 'Trial grid'],
    ['D', 'Differences'],
    ['E', 'Combined'],
  ];
  const LISTS = [
    ['1', 'Log list'],
    ['2', 'Table'],
    ['3', 'Inbox groups'],
  ];
  const THEMES = ['focus-auto', 'focus-light', 'focus-dark', 'plain-auto', 'press-auto', 'signal-auto', 'garden-auto', 'factory-auto'];

  // ---- State, kept in the URL so a layout is shareable and reload-stable.
  const params = new URLSearchParams(location.search);
  const pick = (value, options, fallback) => (options.includes(value) ? value : fallback);
  const state = {
    page: pick(params.get('page'), ['list', 'experiment'], 'experiment'),
    variant: pick(params.get('variant'), VARIANTS.map(v => v[0]), 'A'),
    list: pick(params.get('list'), LISTS.map(v => v[0]), '1'),
    x: pick(params.get('x'), F.experiments.map(e => e.proto.key), 'waiting'),
    theme: pick(params.get('theme'), THEMES, 'focus-auto'),
    phone: params.get('frame') === '390',
    embed: params.get('embed') === '1',
    live: params.get('live') !== 'off',
    open: new Set(),
    peek: null,
    suite: null,
    rep: 'all',
    filter: 'all',
    metric: 'accuracy',
    same: new Set(),
    pendingOnly: false,
  };
  function url(extra = {}) {
    const p = new URLSearchParams({ page: state.page, variant: state.variant, list: state.list, x: state.x });
    if (state.theme !== 'focus-auto') p.set('theme', state.theme);
    if (!state.live) p.set('live', 'off');
    if (state.phone) p.set('frame', '390');
    for (const [key, value] of Object.entries(extra)) value == null ? p.delete(key) : p.set(key, value);
    return `${location.pathname}?${p}`;
  }

  // ---- Formatting.
  const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const prose = s => esc(s).replace(/`([^`]+)`/g, '<code>$1</code>');
  const signed = n => (n > 0 ? '+' : n < 0 ? '−' : '') + Math.abs(n);
  const pts = v => signed(Math.round(v * 100));
  const share = v => `${Math.round(v * 100)}%`;
  const short = c => c.slice(0, 7);
  const cap = s => s[0].toUpperCase() + s.slice(1);
  const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  const NOW = Date.parse(F.now);
  const clock = iso => new Date(iso).toLocaleString('en-US', { timeZone: 'America/New_York', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  const ago = iso => {
    const m = Math.max(1, Math.round((NOW - Date.parse(iso)) / 60000));
    if (m < 60) return `${m} min ago`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h} h ago`;
    const d = Math.round(h / 24);
    return d === 1 ? 'yesterday' : `${d} days ago`;
  };
  const mins = m => { const t = Math.round(m); return t < 60 ? `${t} min` : `${Math.floor(t / 60)} h ${String(t % 60).padStart(2, '0')} min`; };
  const dur = ms => (ms >= 60000 ? mins(ms / 60000) : `${Math.round(ms / 1000)} s`);
  const tok = n => (n == null ? '—' : n >= 1e7 ? `${Math.round(n / 1e6)}M` : n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : String(n));
  const usd = n => (n == null ? 'not reported' : `$${n.toFixed(2)}`);
  const change = (b, c) => (b ? `${signed(Math.round(((c - b) / b) * 100))}%` : '');

  // ---- Domain helpers.
  const SUITE = { 'code-review': 'Code review', 'spec-review': 'Spec review' };
  const ARM = { baseline: 'Baseline', candidate: 'Candidate' };
  const SIDE = { b: 'baseline', c: 'candidate' };
  const current = () => F.experiments.find(e => e.proto.key === state.x);
  const suitesOf = e => Object.entries(e.bar.trials_per_task).map(([name, n]) => ({ name, n, tasks: F.tasks[name] }));
  const totalOf = e => suitesOf(e).reduce((sum, s) => sum + s.tasks.length * s.n * 2, 0);
  const same = (a, b) => a.suite === b.suite && a.task === b.task && a.arm === b.arm && a.trial === b.trial;
  const keyOf = x => `${x.suite}|${x.task}|${x.arm}|${x.trial}`;
  const armLine = s => `${s.harness} · ${s.model || 'default Model'} · ${s.effort || 'default Effort'}`;
  const totalTokens = u => Object.values(u.models).reduce((sum, m) => sum + (m.input_tokens || 0) + (m.output_tokens || 0), 0);

  function at(e, suite, task, arm, trial) {
    const key = { suite, task, arm, trial };
    const result = e.results.find(x => same(x, key));
    if (result) return result;
    const run = e.proto.running.find(x => same(x, key));
    if (run && e.state === 'running') return { ...key, running: true, started_ms_ago: run.started_ms_ago };
    return { ...key, queued: true, paused: e.state === 'paused' };
  }
  const isResult = x => !x.queued && !x.running;
  const graded = x => isResult(x) && !!(x.score && (x.score.model || !x.score.checks.every(c => c.passed)));
  const pendingOf = (e, x) => e.pending_rechecks.find(p => same(p, x));
  const recheckOf = x => (x.score && x.score.rechecks && x.score.rechecks.length ? x.score.rechecks[x.score.rechecks.length - 1] : null);
  const effective = x => F.effective(x.score);

  function mark(e, x) {
    if (x.queued) return { sym: '·', tone: 'queued', label: x.paused ? 'Not run yet: the Experiment paused' : 'Queued' };
    if (x.running) return { sym: '◌', tone: 'running', label: `Running for ${dur(x.started_ms_ago)}` };
    if (x.outcome === 'void') return { sym: '⊘', tone: 'void', label: `Void: ${F.voidText[x.attempts[x.attempts.length - 1].void_cause]}` };
    if (!graded(x)) return { sym: '–', tone: 'ungraded', label: 'Code checks passed; the model Grader scores it after the last Trial' };
    const s = effective(x), p = pendingOf(e, x), r = recheckOf(x);
    const bits = [s.passed ? 'Passed' : 'Failed'];
    if (x.outcome === 'failed') bits.push(x.attempts[x.attempts.length - 1].cause);
    if (s.wrong_finding) bits.push('wrong finding');
    if (x.score.borderline) bits.push('borderline');
    if (r) bits.push('you rechecked it');
    if (p) bits.push('waiting on your recheck');
    if (x.attempts.length > 1) bits.push(`after ${plural(x.attempts.length - 1, 'Void attempt')}`);
    return { sym: s.passed ? '✓' : '✗', tone: s.passed ? 'pass' : 'fail', label: bits.join(', '), wrong: s.wrong_finding, borderline: x.score.borderline, pending: !!p, rechecked: !!r, rerun: x.attempts.length > 1 };
  }
  function glyph(m) {
    return `<span class="x-mark" data-tone="${m.tone}"${m.pending ? ' data-pending' : ''}${m.rechecked ? ' data-rechecked' : ''} title="${esc(m.label)}">`
      + `<span aria-hidden="true">${m.sym}</span>${m.wrong ? '<sup aria-hidden="true">!</sup>' : ''}${m.borderline ? '<sup aria-hidden="true">?</sup>' : ''}`
      + `<span class="x-sr">${esc(m.label)}</span></span>`;
  }

  function taskInfo(e, suite, task) {
    const n = e.bar.trials_per_task[suite];
    const arm = name => Array.from({ length: n }, (_, i) => at(e, suite, task, name, i));
    const b = arm('baseline'), c = arm('candidate'), all = b.concat(c);
    const dropped = all.some(x => x.outcome === 'void');
    const complete = all.every(graded);
    const passes = xs => xs.filter(x => graded(x) && x.outcome !== 'void' && effective(x).passed).length;
    const wrongs = xs => xs.filter(x => graded(x) && effective(x).wrong_finding).length;
    const pb = passes(b), pc = passes(c);
    return {
      suite, task, n, b, c, dropped, complete, pb, pc,
      diff: complete && !dropped ? (pc - pb) / n : null,
      wrongDiff: complete && !dropped ? (wrongs(b) - wrongs(c)) / n : null,
      pending: all.filter(x => isResult(x) && pendingOf(e, x)).length,
      key: `${suite}/${task}`,
    };
  }
  const tasksOf = (e, suite) => F.tasks[suite].map(t => taskInfo(e, suite, t));

  function armStats(e, suite) {
    const infos = tasksOf(e, suite).filter(i => !i.dropped && i.complete);
    const n = e.bar.trials_per_task[suite];
    const stat = side => {
      const xs = infos.flatMap(i => i[side]);
      return {
        passed: xs.filter(x => effective(x).passed).length,
        wrong: xs.filter(x => effective(x).wrong_finding).length,
        trials: xs.length,
        all: n === 3 ? infos.filter(i => i[side].every(x => effective(x).passed)).length : null,
      };
    };
    return { b: stat('b'), c: stat('c'), tasks: infos.length, n };
  }

  function status(e) {
    if (e.state === 'running') return e.proto.stage === 'Model Graders' ? { word: 'Grading', tone: 'working' } : { word: 'Running', tone: 'working' };
    if (e.state === 'paused') return { word: 'Paused', tone: 'look' };
    return {
      pass: { word: 'Pass', tone: 'pass' },
      fail: { word: 'Fail', tone: 'fail' },
      inconclusive: { word: 'Inconclusive', tone: 'look' },
      waiting_on_rechecks: { word: 'Waiting on your rechecks', tone: 'look' },
    }[e.verdict.verdict];
  }
  const tone = v => (v === 'pass' ? 'pass' : v === 'fail' ? 'fail' : 'look');
  function tag(e) {
    const s = status(e);
    return `<span class="x-tag" data-tone="${s.tone}">${s.tone === 'working' ? '<span class="x-spin" aria-hidden="true"></span>' : ''}${esc(s.word)}</span>`;
  }
  const small = (v, word = cap(v)) => `<span class="x-tag x-small" data-tone="${tone(v)}">${esc(word)}</span>`;

  const observed = a => {
    const t = (a.difference_counts.length - 1) / 2;
    let n = 0, sum = 0;
    a.difference_counts.forEach((c, i) => { n += c; sum += (c * (i - t)) / t; });
    return n ? sum / n : 0;
  };
  const range = a => `${pts(a.interval.lower)} to ${pts(a.interval.upper)}`;
  const METRIC = { accuracy: 'accuracy', wrong_findings: 'wrong findings' };

  // Why a suite's Verdict is what it is, in the Bar's own words.
  function suiteWhy(e, name, v) {
    const total = v.retained_tasks + v.dropped_tasks;
    if (v.dropped_tasks > Math.floor(total / 10)) return `${v.dropped_tasks} of ${total} Eval tasks dropped out as void, more than a tenth, so it is inconclusive whatever the intervals say.`;
    const margin = pts(e.bar.margin);
    const parts = [];
    for (const key of ['accuracy', 'wrong_findings']) {
      const a = v[key];
      if (a.verdict === 'fail') parts.push(`The whole ${METRIC[key]} interval is below zero.`);
      if (a.verdict === 'inconclusive') {
        let s = `The ${METRIC[key]} interval reaches below the ${margin}-point margin and above zero.`;
        if (a.more_tasks && a.more_tasks.additional_tasks) s += ` About ${a.more_tasks.additional_tasks} more Eval tasks would settle it (an ${share(a.more_tasks.target_probability)} chance).`;
        parts.push(s);
      }
    }
    if (!parts.length) parts.push(`Both intervals sit above the ${margin}-point margin and reach zero or above.`);
    return parts.join(' ');
  }

  function explain(e) {
    const done = e.results.length, total = totalOf(e);
    if (e.state === 'running' && e.proto.stage === 'Model Graders') {
      const tasks = suitesOf(e).reduce((s, x) => s + x.tasks.length, 0);
      return { head: `Grading: the model Graders have scored ${e.proto.graded || 0} of ${tasks} Eval tasks.`, lines: ['Each Grader reads one blind packet per Eval task, holding both Arms\' Trials with their labels shuffled. The Verdict comes once every Eval task is scored.'] };
    }
    if (e.state === 'running') {
      return {
        head: `Running: ${done} of ${total} Trials have finished.`,
        lines: [
          `${plural(e.proto.running.length, 'Trial')} running now; two workers take the Arms in turn, so a slow hour reaches both.`,
          'The model Graders score each Eval task after the last Trial; until then a Trial shows only its code checks.',
        ],
      };
    }
    if (e.state === 'paused') {
      const p = e.proto.pause;
      return {
        head: `Paused at a ${cap(p.harness)} plan-limit warning, ${clock(p.at)}.`,
        lines: [
          `${done} of ${total} Trials finished before it and are kept. ${p.text}`,
          `Run the same command again to resume: <code>${esc(e.proto.command_line)}</code>`,
        ],
      };
    }
    const v = e.verdict, lines = [];
    if (v.verdict === 'waiting_on_rechecks') {
      const b = e.pending_rechecks.filter(p => p.borderline).length, d = e.pending_rechecks.filter(p => p.decisive).length;
      lines.push(`${plural(e.pending_rechecks.length, 'Score')} wait on your recheck: ${b} borderline, ${d} decisive (some are both). The Verdict is final once you have rechecked them all.`);
    }
    for (const [name, sv] of Object.entries(v.suites)) lines.push(`<strong>${esc(SUITE[name])}</strong>: ${small(sv.verdict)} ${suiteWhy(e, name, sv)}`);
    const heads = {
      pass: 'Pass: the Candidate is no worse than the Baseline, under the Bar.',
      fail: 'Fail: the Candidate is surely worse than the Baseline.',
      inconclusive: 'Inconclusive: these Trials can\'t say whether the Candidate is worse.',
      waiting_on_rechecks: `Provisionally ${v.provisional_verdict === 'inconclusive' ? 'inconclusive' : `a ${v.provisional_verdict}`}, until you recheck ${plural(e.pending_rechecks.length, 'Score')}.`,
    };
    return { head: heads[v.verdict], lines };
  }

  // ---- Shared pieces: interval figure, CLI line, Session links.
  const AXIS = 0.5;
  const xpos = (v, axis = AXIS) => Math.max(0, Math.min(100, ((v + axis) / (2 * axis)) * 100));
  function iv(e, a, label, axis = AXIS) {
    const lo = a.interval.lower, hi = a.interval.upper;
    return `<span class="x-iv" data-tone="${tone(a.verdict)}" role="img" aria-label="${esc(`${label}: ${range(a)} points; margin ${pts(e.bar.margin)}`)}">`
      + `<span class="x-iv-margin" style="left:${xpos(e.bar.margin, axis)}%"></span>`
      + `<span class="x-iv-zero" style="left:${xpos(0, axis)}%"></span>`
      + `<span class="x-iv-bar${lo < -axis ? ' x-clip-l' : ''}${hi > axis ? ' x-clip-r' : ''}" style="left:${xpos(lo, axis)}%;width:${(xpos(hi, axis) - xpos(lo, axis)).toFixed(2)}%"></span>`
      + `<span class="x-iv-mean" style="left:${xpos(observed(a), axis)}%"></span></span>`;
  }
  const axisKey = (e, axis = AXIS) => `<span class="x-axis" aria-hidden="true"><span style="left:0%">${pts(-axis)}</span><span style="left:${xpos(e.bar.margin, axis)}%">${pts(e.bar.margin)}</span><span style="left:${xpos(0, axis)}%">0</span><span style="left:100%">${pts(axis)}</span></span>`;
  const cli = cmd => `<span class="x-cli"><code>${esc(cmd)}</code><button type="button" data-copy="${esc(cmd)}">Copy</button></span>`;
  const sessionUrl = (e, attempt) => `/log?command=${encodeURIComponent(e.proto.command)}&session=${encodeURIComponent(attempt.session_id || '')}`;
  const sessionLink = (e, x, text = 'Session trace') => {
    const a = x.attempts && [...x.attempts].reverse().find(t => t.session_id);
    return a ? `<a href="${esc(sessionUrl(e, a))}" data-dead="Opens this Trial's Session trace on the Logs page.">${esc(text)}</a>` : '<span class="x-dim">no session</span>';
  };
  const recheckCli = (e, p, x) => `thirdshift recheck ${e.id} ${p.id} ${effective(x).passed ? 'pass' : 'fail'} --reason "…"`;
  function decisiveText(x, p) {
    const flips = p.proto_flips || [];
    const wrongNow = effective(x).wrong_finding;
    const wrong = wrongNow ? 'without its wrong finding' : 'with one wrong finding';
    if (flips.length === 2) return 'flipping its pass or fail, or its wrong-finding flag, would change the Verdict';
    if (flips[0] === 'passed') return `as a ${effective(x).passed ? 'fail' : 'pass'}, it would change the Verdict`;
    return `${wrong}, it would change the Verdict`;
  }
  const why = (x, p) => [p.borderline ? 'the Grader marked it borderline' : '', p.decisive ? decisiveText(x, p) : ''].filter(Boolean).join('; and ');

  function setupRows(e) {
    const prep = e.prepared;
    const differs = f => e.baseline[f] !== e.candidate[f];
    const cell = (side, f, text) => `<td${differs(f) ? ' class="x-differs"' : ''}>${esc(text)}</td>`;
    return [
      ['Build', `<td>${esc(e.proto.baseline_ref)} · <code>${short(e.baseline.commit)}</code></td><td${differs('commit') ? ' class="x-differs"' : ''}>${esc(e.proto.candidate_ref)} · <code>${short(e.candidate.commit)}</code></td>`],
      ['Harness', cell('b', 'harness', e.baseline.harness) + cell('c', 'harness', e.candidate.harness)],
      ['Model', cell('b', 'model', e.baseline.model) + cell('c', 'model', e.candidate.model)],
      ['Effort', cell('b', 'effort', e.baseline.effort) + cell('c', 'effort', e.candidate.effort)],
      ['Harness version', `<td>${esc(prep.baseline.harness.version)}</td><td>${esc(prep.candidate.harness.version)}</td>`],
    ];
  }

  function barFacts(e) {
    const b = e.bar, v = e.verdict && Object.values(e.verdict.suites)[0];
    return `<dl class="x-bar">`
      + `<dt>Margin</dt><dd>${pts(b.margin)} points. A pass needs the whole interval above it and some of it at zero or above; a fail is an interval wholly below zero.</dd>`
      + `<dt>Credible level</dt><dd>${share(b.credible_level)}</dd>`
      + `<dt>Prior</dt><dd>${b.prior} pseudo Eval task, spread evenly over the values a difference can take</dd>`
      + `<dt>Seed and draws</dt><dd>Seed ${b.seed}, ${b.draws.toLocaleString('en-US')} draws</dd>`
      + `<dt>Trials per Eval task</dt><dd>${Object.entries(b.trials_per_task).map(([s, n]) => `${SUITE[s]}: ${n} per Arm`).join(' · ')}</dd>`
      + `<dt>Timeout per Trial</dt><dd>${b.timeout_seconds / 60} minutes; a timeout is a fail</dd>`
      + `<dt>Void Trials</dt><dd>Re-run fresh up to ${b.void_rules.retries} times. Void causes: ${b.void_rules.causes.map(c => c.replace(/_/g, ' ')).join(', ')}. An Eval task still void after its re-runs drops out for both Arms; if more than a tenth drop out, the Verdict is inconclusive.</dd>`
      + `<dt>Efficiency</dt><dd>Shown beside accuracy, outside the Bar</dd>`
      + `<dt>Recorded</dt><dd>${clock(e.created_at)}, before the first Trial. It can't change afterwards.</dd>`
      + (v ? `<dt>Method</dt><dd><code>${esc(v.method)}</code> · thirdshift ${esc(v.thirdshift_version)}</dd>` : '')
      + `</dl>`;
  }

  function header(e) {
    return `<p class="x-crumb"><a href="${esc(url({ page: 'list' }))}" data-nav="list">← Experiments</a></p>`
      + `<div class="page-title"><h2>${esc(e.proto.name)}</h2><code>thirdshift experiments ${e.id} --json</code></div>`
      + `<p class="x-title">${esc(e.proto.title)} · started ${esc(clock(e.created_at))}${e.proto.ended_at ? `, ended ${esc(clock(e.proto.ended_at))}` : ''} · <a href="#" data-dead="Opens the Experiment's Command log.">Command log</a></p>`;
  }
  const findingsBlock = e => (e.findings.length ? `<div class="x-callout" data-tone="look">${e.findings.map(f => `<p><strong>Finding</strong>: ${esc(f)}. Runs have no memory cap, so this Arm may need more memory than a Trial gets.</p>`).join('')}</div>` : '');

  // ---- Peek: Trial details, an Eval task's Trials, the Bar.
  function trialPeek(e, x) {
    const head = `<h2>${esc(ARM[x.arm])} · Trial ${x.trial + 1}</h2><p class="x-sub">${esc(SUITE[x.suite])} · <button type="button" class="x-linkish" data-peek-task="${esc(x.suite)}|${esc(x.task)}">${esc(x.task)}</button></p>`;
    if (x.running) return `<section aria-label="Trial details">${head}<p>${glyph(mark(e, x))} Running for ${dur(x.started_ms_ago)}. Its session writes its Session trace as it goes.</p><p><a href="#" data-dead="Opens the live Session trace.">Watch the Session trace</a></p></section>`;
    if (x.queued) return `<section aria-label="Trial details">${head}<p>${glyph(mark(e, x))} ${x.paused ? 'Not run yet: the Experiment paused before it.' : 'Queued. It starts when one of the two workers is free.'}</p></section>`;
    const m = mark(e, x), p = pendingOf(e, x), r = recheckOf(x), model = x.score.model, s = effective(x);
    let h = `<section aria-label="Trial details">${head}<p class="x-big">${glyph(m)} <strong>${s.passed ? 'Passed' : 'Failed'}</strong>${r ? ', by your recheck' : ''}${x.outcome === 'void' ? ' · void' : ''}</p>`;
    if (p) h += `<div class="x-callout" data-tone="look"><p><strong>Waiting on your recheck</strong>: ${esc(why(x, p))}.</p>${cli(recheckCli(e, p, x))}</div>`;
    h += '<dl>';
    if (model) h += `<dt>Model Grader</dt><dd>${model.passed ? 'Pass' : 'Fail'}${model.borderline ? ', borderline' : ''}${model.wrong_finding ? ', wrong finding' : ''}. ${prose(model.reason)}<br><span class="x-dim">Scored as ${esc(model.label)} in the Eval task's blind packet.</span></dd>`;
    else if (x.outcome !== 'void') h += `<dt>Model Grader</dt><dd>${graded(x) ? 'Not needed: a code check failed.' : 'Not yet. It scores the Eval task after the last Trial.'}</dd>`;
    if (r) h += `<dt>Your recheck</dt><dd>${r.passed ? 'Pass' : 'Fail'}. ${prose(r.reason)}<br><span class="x-dim">${esc(clock(r.recorded_at))}. Kept beside the Grader's Score; the Verdict uses yours.</span></dd>`;
    h += `<dt>Wrong finding</dt><dd>${s.wrong_finding ? 'Yes' : 'No'}</dd>`;
    h += `<dt>Code checks</dt><dd><ul class="x-checks">${x.score.checks.map(c => `<li><span aria-hidden="true">${c.passed ? '✓' : '✗'}</span> ${esc(c.command ? `${c.command.join(' ')}: ` : '')}${esc(c.reason)}</li>`).join('')}</ul></dd>`;
    h += `<dt>Attempts</dt><dd><ol class="x-attempts">${x.attempts.map(a => `<li>${esc(cap(a.outcome))}${a.void_cause ? `: ${esc(F.voidText[a.void_cause])}` : a.cause ? `: ${esc(a.cause)}` : ''} · ${dur(a.timings.total_ms)} · ${a.session_id ? `<a href="${esc(sessionUrl(e, a))}" data-dead="Opens this attempt's Session trace.">Session trace</a>` : 'no session started'}</li>`).join('')}</ol>`;
    if (x.attempts.length > 1 && x.outcome !== 'void') h += '<span class="x-dim">A Void attempt runs again fresh, up to twice, and is counted against its Arm.</span>';
    if (x.outcome === 'void') h += '<span class="x-dim">Still void after its re-runs, so this Eval task drops out for both Arms.</span>';
    h += `</dd><dt>Arm</dt><dd>${esc(ARM[x.arm])}: ${esc(armLine(e[x.arm]))} · <code>${short(e[x.arm].commit)}</code></dd>`;
    h += `<dt>Trial ID</dt><dd><code>${esc(F.trialKey(x, e.id))}</code></dd></dl>`;
    return `${h}</section>`;
  }
  function taskPeek(e, suite, task) {
    const info = taskInfo(e, suite, task);
    const side = (k, arm) => `<h3>${ARM[arm]}: ${info.complete || info.dropped ? `${info[k === 'b' ? 'pb' : 'pc']} of ${info.n}` : '…'}</h3><ul class="x-trials">${info[k].map(x => `<li><button type="button" class="x-linkish" data-peek-trial="${esc(keyOf(x))}">${glyph(mark(e, x))} Trial ${x.trial + 1}</button> ${x.score && x.score.model ? prose(x.score.model.reason) : esc(mark(e, x).label)}</li>`).join('')}</ul>`;
    return `<section aria-label="Eval task details"><h2>${esc(task)}</h2><p class="x-sub">${esc(SUITE[suite])} · ${plural(info.n, 'Trial')} per Arm</p><p>${diffText(info)}</p>${side('b', 'baseline')}${side('c', 'candidate')}</section>`;
  }
  const barPeek = e => `<section aria-label="The Bar"><h2>The Bar</h2><p class="x-sub">Fixed before the first Trial.</p>${barFacts(e)}</section>`;
  function diffText(info) {
    if (info.dropped) return 'Dropped: a Trial stayed void after its re-runs, so this Eval task leaves the interval for both Arms.';
    if (!info.complete) return 'Not every Trial is finished and graded yet.';
    const d = info.diff;
    return d === 0 ? 'Same pass share in both Arms.' : `The Candidate passed ${plural(Math.abs(info.pc - info.pb), 'more Trial', 'more Trials').replace('more', d > 0 ? 'more' : 'fewer')} than the Baseline: ${pts(d)} points.`;
  }
  const diffCell = info => (info.dropped ? '<span class="x-dim">dropped</span>' : info.diff == null ? '<span class="x-dim">…</span>' : `<span class="x-diff" data-sign="${Math.sign(info.diff)}">${pts(info.diff)}</span>`);

  // ======== A · Compare table: Arms side by side, then Eval tasks by Arm (Phoenix's compare view).
  function variantA(e) {
    let h = header(e) + `<div class="va-status">${tag(e)}<span>${explain(e).head}</span></div>`;
    if (e.state !== 'completed') h += `<p class="x-dim">${explain(e).lines.join(' ')}</p>`;
    h += findingsBlock(e) + armsTableA(e);
    if (e.verdict) h += verdictA(e);
    if (e.pending_rechecks.length) h += rechecksA(e);
    h += compareA(e);
    h += `<details class="va-bar"><summary>The Bar <span class="x-dim">· margin ${pts(e.bar.margin)} points, ${share(e.bar.credible_level)}, fixed ${esc(clock(e.created_at))}</span></summary>${barFacts(e)}</details>`;
    return h;
  }
  function armsTableA(e) {
    const rows = setupRows(e);
    for (const s of suitesOf(e)) {
      const st = armStats(e, s.name);
      const cell = x => (x.trials ? `${x.passed}/${x.trials} <span class="x-dim">${share(x.passed / x.trials)}</span>` : '<span class="x-dim">—</span>');
      rows.push([`${SUITE[s.name]} pass share`, `<td>${cell(st.b)}</td><td>${cell(st.c)}</td>`]);
      if (st.n === 3 && st.tasks) rows.push(['pass³ <span class="x-dim">all three passed</span>', `<td>${st.b.all}/${st.tasks}</td><td>${st.c.all}/${st.tasks}</td>`]);
      rows.push([`${SUITE[s.name]} wrong findings`, `<td>${st.b.trials ? `${st.b.wrong} of ${st.b.trials}` : '—'}</td><td>${st.c.trials ? `${st.c.wrong} of ${st.c.trials}` : '—'}</td>`]);
    }
    rows.push(['Minutes', `<td>${mins(e.minutes.baseline)}</td><td>${mins(e.minutes.candidate)} <span class="x-dim">${change(e.minutes.baseline, e.minutes.candidate)}</span></td>`]);
    rows.push(['Tokens <span class="x-dim">in + out</span>', `<td>${tok(totalTokens(e.usage.baseline))}</td><td>${tok(totalTokens(e.usage.candidate))} <span class="x-dim">${change(totalTokens(e.usage.baseline), totalTokens(e.usage.candidate))}</span></td>`]);
    rows.push(['Cost', `<td>${usd(e.usage.baseline.cost_usd)}</td><td>${usd(e.usage.candidate.cost_usd)} <span class="x-dim">${e.usage.baseline.cost_usd ? change(e.usage.baseline.cost_usd, e.usage.candidate.cost_usd) : ''}</span></td>`]);
    const voids = v => (v.count ? `${v.count} <span class="x-dim">${Object.entries(v.causes).map(([c, n]) => `${c.replace(/_/g, ' ')} ${n}`).join(', ')}</span>` : '0');
    rows.push(['Void Trials', `<td>${voids(e.voids.baseline)}</td><td>${voids(e.voids.candidate)}</td>`]);
    return `<table class="va-arms"><caption class="x-sr">Arms side by side</caption><thead><tr><td></td><th scope="col">Baseline</th><th scope="col">Candidate</th></tr></thead><tbody>${rows.map(([k, v]) => `<tr><th scope="row">${k}</th>${v}</tr>`).join('')}</tbody></table>`;
  }
  function verdictA(e) {
    let h = '<section class="va-verdict" aria-label="Verdict by suite">';
    for (const [name, v] of Object.entries(e.verdict.suites)) {
      h += `<div class="va-suite"><h3>${esc(SUITE[name])} ${small(v.verdict)} <span class="x-dim">${v.retained_tasks} Eval tasks${v.dropped_tasks ? `, ${v.dropped_tasks} dropped` : ''}</span></h3>`;
      for (const key of ['accuracy', 'wrong_findings']) {
        const a = v[key];
        h += `<div class="va-metric"><span class="va-mname">${cap(METRIC[key])}</span>${iv(e, a, cap(METRIC[key]))}<span class="va-num">${pts(observed(a))} <span class="x-dim">${range(a)}</span></span>${small(a.verdict)}</div>`;
      }
      h += `<div class="va-metric va-key"><span></span>${axisKey(e)}<span></span><span></span></div>`;
      h += `<p class="va-why">${suiteWhy(e, name, v)}${v.pass_cubed ? ` pass³: Baseline ${share(v.pass_cubed.baseline)}, Candidate ${share(v.pass_cubed.candidate)}.` : ''}</p></div>`;
    }
    return `${h}</section><p class="x-dim va-legend">Points of pass share, Candidate minus Baseline. Bars are ${share(e.bar.credible_level)} intervals; the tick is the observed difference; the dashed line is the margin. For wrong findings, plus means fewer in the Candidate.</p>`;
  }
  function rechecksA(e) {
    const open = state.open.has('rechecks');
    const b = e.pending_rechecks.filter(p => p.borderline).length;
    let h = `<section class="va-rechecks x-callout" data-tone="look"><p><strong>${plural(e.pending_rechecks.length, 'Score')} wait on your recheck</strong> (${b} borderline). <button type="button" class="x-linkish" data-toggle="rechecks" aria-expanded="${open}">${open ? 'Hide them' : 'Show them'}</button> · <button type="button" class="x-linkish" data-filter="pending">Show only their Eval tasks</button></p>`;
    if (open) h += `<ol class="va-recheck-list">${e.pending_rechecks.map(p => { const x = at(e, p.suite, p.task, p.arm, p.trial); return `<li><button type="button" class="x-linkish" data-peek-trial="${esc(keyOf(x))}">${glyph(mark(e, x))} ${esc(p.task)} · ${ARM[p.arm]} Trial ${p.trial + 1}</button> <span class="x-dim">${esc(why(x, p))}</span></li>`; }).join('')}</ol><p class="x-dim">Record each with <code>thirdshift recheck &lt;experiment&gt; &lt;trial&gt; pass|fail --reason …</code>; Details has the exact line.</p>`;
    return `${h}</section>`;
  }
  function compareA(e) {
    const suites = suitesOf(e);
    const suite = suites.find(s => s.name === state.suite) || suites[0];
    const infos = tasksOf(e, suite.name);
    const better = infos.filter(i => i.diff > 0).length, worse = infos.filter(i => i.diff < 0).length, even = infos.filter(i => i.diff === 0).length, dropped = infos.filter(i => i.dropped).length, waiting = infos.filter(i => !i.complete && !i.dropped).length;
    const shown = infos.filter(i => (state.filter === 'differs' ? i.diff : state.filter === 'pending' ? i.pending : state.filter === 'void' ? i.dropped || i.b.concat(i.c).some(x => x.attempts && x.attempts.length > 1) : true));
    const rep = state.rep === 'all' || +state.rep >= suite.n ? 'all' : +state.rep;
    let h = '<section class="va-compare" aria-label="Eval tasks by Arm">';
    if (suites.length > 1) h += `<div class="va-tabs" role="tablist">${suites.map(s => `<button type="button" role="tab" aria-selected="${s.name === suite.name}" data-suite="${s.name}">${esc(SUITE[s.name])} <span class="x-dim">${s.tasks.length}</span></button>`).join('')}</div>`;
    h += '<div class="va-tools">';
    if (suite.n > 1) h += `<span class="x-seg" role="group" aria-label="Trials shown">${['all', ...Array.from({ length: suite.n }, (_, i) => String(i))].map(k => `<button type="button" data-rep="${k}" aria-pressed="${String(rep) === k}">${k === 'all' ? 'All Trials' : `Trial ${+k + 1}`}</button>`).join('')}</span>`;
    h += `<span class="x-seg" role="group" aria-label="Filter Eval tasks">${[['all', 'All'], ['differs', 'Differs'], ['pending', 'Needs recheck'], ['void', 'Void or re-run']].map(([k, t]) => `<button type="button" data-filter="${k}" aria-pressed="${state.filter === k}">${t}</button>`).join('')}</span></div>`;
    h += `<p class="va-counts">Candidate better on ${better} · worse on ${worse} · same on ${even}${dropped ? ` · dropped ${dropped}` : ''}${waiting ? ` · ${waiting} not finished` : ''}</p>`;
    h += `<table class="va-table${rep === 'all' ? '' : ' va-one'}"><thead><tr><th scope="col">Eval task</th><th scope="col">Baseline</th><th scope="col">Candidate</th><th scope="col">Difference</th></tr></thead><tbody>`;
    for (const i of shown) {
      const open = state.open.has(i.key);
      const cell = (side, passes) => (rep === 'all'
        ? `${i[side].map(x => glyph(mark(e, x))).join('')} <span class="va-share">${graded(i[side][0]) || i.complete ? `${passes}/${i.n}` : ''}</span>`
        : oneCell(e, i[side][rep]));
      h += `<tr class="va-row${open ? ' va-open' : ''}" data-open-task="${esc(i.key)}"><th scope="row"><button type="button" aria-expanded="${open}" data-focus="a-${esc(i.key)}">${esc(i.task)}</button></th><td data-label="Baseline">${cell('b', i.pb)}</td><td data-label="Candidate">${cell('c', i.pc)}</td><td data-label="Difference">${diffCell(i)}</td></tr>`;
      if (open) h += `<tr class="va-detail"><td colspan="4">${pairsA(e, i)}</td></tr>`;
    }
    if (!shown.length) h += '<tr><td colspan="4" class="x-dim">No Eval tasks match.</td></tr>';
    return `${h}</tbody></table></section>`;
  }
  function oneCell(e, x) {
    const m = mark(e, x);
    const text = x.score && x.score.model ? prose(x.score.model.reason) : esc(m.label);
    return `<div class="va-one-cell">${glyph(m)}<span>${text}</span></div>`;
  }
  function pairsA(e, i) {
    let h = '<div class="va-pairs">';
    for (let t = 0; t < i.n; t++) {
      for (const side of ['b', 'c']) {
        const x = i[side][t], m = mark(e, x);
        h += `<div class="va-pair"><p class="va-pair-head">${glyph(m)} <strong>${ARM[SIDE[side]]} · Trial ${t + 1}</strong></p><p>${x.score && x.score.model ? prose(x.score.model.reason) : esc(m.label)}</p><p class="x-dim">${isResult(x) ? `${dur(x.attempts.reduce((s, a) => s + a.timings.total_ms, 0))} · ${sessionLink(e, x)} · ` : ''}<button type="button" class="x-linkish" data-peek-trial="${esc(keyOf(x))}">Details</button></p></div>`;
      }
    }
    return `${h}</div>`;
  }

  // ======== B · Report: one column of sentences and facts, like Command details.
  function variantB(e) {
    const ex = explain(e);
    let h = header(e) + `<section class="vb-lead">${tag(e)}<p class="vb-head">${ex.head}</p>${ex.lines.map(l => `<p>${l}</p>`).join('')}</section>`;
    if (e.verdict) {
      h += '<section class="vb-figures" aria-label="Intervals">';
      for (const [name, v] of Object.entries(e.verdict.suites)) {
        for (const key of ['accuracy', 'wrong_findings']) h += `<p class="vb-figure"><span>${esc(SUITE[name])}, ${METRIC[key]}</span>${iv(e, v[key], `${SUITE[name]} ${METRIC[key]}`)}<span>${range(v[key])}</span></p>`;
      }
      h += `<p class="vb-figure vb-key"><span></span>${axisKey(e)}<span></span></p></section>`;
    }
    h += findingsBlock(e);
    if (e.pending_rechecks.length) h += rechecksB(e);
    h += `<h3>What was compared</h3><dl class="vb-dl">`
      + `<dt>Baseline</dt><dd>${esc(e.proto.baseline_ref)} · <code>${short(e.baseline.commit)}</code> · ${esc(armLine(e.baseline))} · ${esc(e.prepared.baseline.harness.version)}</dd>`
      + `<dt>Candidate</dt><dd>${esc(e.proto.candidate_ref)} · <code>${short(e.candidate.commit)}</code> · ${esc(armLine(e.candidate))} · ${esc(e.prepared.candidate.harness.version)}</dd>`
      + `<dt>Eval suites</dt><dd>${suitesOf(e).map(s => `${SUITE[s.name]}: ${s.tasks.length} Eval tasks, ${plural(s.n, 'Trial')} per Arm on each`).join('; ')}</dd>`
      + `<dt>Command</dt><dd><code>${esc(e.proto.command_line)}</code></dd>`
      + `<dt>Trials</dt><dd>${e.results.length} of ${totalOf(e)} finished${e.state === 'running' ? `, ${e.proto.running.length} running` : ''}</dd></dl>`;
    h += `<h3>Efficiency</h3><p class="vb-eff">${efficiencyText(e)}</p>`;
    h += '<h3>Eval tasks</h3>';
    for (const s of suitesOf(e)) {
      h += `<h4>${esc(SUITE[s.name])} <span class="x-dim">${s.tasks.length} Eval tasks × ${s.n} per Arm</span></h4><ul class="vb-tasks">`;
      for (const i of tasksOf(e, s.name)) {
        const open = state.open.has(i.key);
        const line = `${i.complete || i.dropped ? `Baseline ${i.pb} of ${i.n}, Candidate ${i.pc} of ${i.n}` : 'in progress'}${i.pending ? ` · ${plural(i.pending, 'recheck')} waiting` : ''}`;
        h += `<li><details data-key="${esc(i.key)}"${open ? ' open' : ''}><summary><span class="vb-task">${esc(i.task)}</span> <span class="x-dim">${line}</span> ${diffCell(i)}</summary><ol class="vb-trials">`;
        for (const side of ['b', 'c']) for (const x of i[side]) {
          const m = mark(e, x);
          h += `<li>${glyph(m)} <strong>${ARM[x.arm]} Trial ${x.trial + 1}</strong>: ${x.score && x.score.model ? prose(x.score.model.reason) : esc(m.label)} ${isResult(x) ? sessionLink(e, x) : ''} · <button type="button" class="x-linkish" data-peek-trial="${esc(keyOf(x))}">Details</button></li>`;
        }
        h += '</ol></details></li>';
      }
      h += '</ul>';
    }
    return `${h}<h3>The Bar</h3>${barFacts(e)}`;
  }
  function efficiencyText(e) {
    const m = e.minutes, u = e.usage, tb = totalTokens(u.baseline), tc = totalTokens(u.candidate);
    let s = `The Candidate took ${mins(m.candidate)} of Trial time to the Baseline's ${mins(m.baseline)} (${change(m.baseline, m.candidate)}), and used ${tok(tc)} tokens in and out to ${tok(tb)} (${change(tb, tc)}).`;
    s += u.candidate.cost_usd != null && u.baseline.cost_usd != null ? ` It cost ${usd(u.candidate.cost_usd)} to ${usd(u.baseline.cost_usd)} (${change(u.baseline.cost_usd, u.candidate.cost_usd)}).` : ` ${cap(e.candidate.harness)} doesn't report cost.`;
    const v = e.voids;
    if (v.baseline.count || v.candidate.count) s += ` Void attempts: Baseline ${v.baseline.count}, Candidate ${v.candidate.count}; they count in the minutes.`;
    return `${s} Efficiency stays out of the Bar.`;
  }
  function rechecksB(e) {
    const groups = [['Borderline and decisive', p => p.borderline && p.decisive], ['Borderline', p => p.borderline && !p.decisive], ['Decisive', p => !p.borderline && p.decisive]];
    let h = `<section class="vb-rechecks"><h3>Waiting on you <span class="x-dim">${e.pending_rechecks.length}</span></h3><p>A borderline Score is one the Grader marked as a close call. A decisive one would change the Verdict if it flipped. Record each recheck from the CLI; it is kept beside the Grader's Score.</p>`;
    for (const [title, test] of groups) {
      const list = e.pending_rechecks.filter(test);
      if (!list.length) continue;
      h += `<h4>${title} <span class="x-dim">${list.length}</span></h4><ol class="vb-recheck-list">`;
      for (const p of list) {
        const x = at(e, p.suite, p.task, p.arm, p.trial);
        h += `<li><p>${glyph(mark(e, x))} <strong>${esc(p.task)}</strong>, ${ARM[p.arm]} Trial ${p.trial + 1}: ${prose(x.score.model ? x.score.model.reason : '')}</p><p class="x-dim">${esc(cap(why(x, p)))}.</p>${cli(recheckCli(e, p, x))}</li>`;
      }
      h += '</ol>';
    }
    return `${h}</section>`;
  }

  // ======== C · Trial grid: every Trial a cell; details open in the Peek.
  function variantC(e) {
    const st = status(e);
    let h = header(e) + `<div class="vc-strip">${tag(e)}`;
    if (e.verdict) for (const [name, v] of Object.entries(e.verdict.suites)) h += `<span class="vc-stat"><span class="x-dim">${esc(SUITE[name])}</span> ${iv(e, v.accuracy, `${SUITE[name]} accuracy`)} <span>${range(v.accuracy)}</span></span>`;
    h += `<span class="vc-stat"><span class="x-dim">Trials</span> ${e.results.length}/${totalOf(e)}</span>`;
    h += `<span class="vc-stat"><span class="x-dim">Minutes</span> ${Math.round(e.minutes.baseline)} · ${Math.round(e.minutes.candidate)}</span>`;
    h += `<button type="button" data-peek-bar>The Bar</button>`;
    if (e.pending_rechecks.length) h += `<button type="button" data-pending-only aria-pressed="${state.pendingOnly}">${plural(e.pending_rechecks.length, 'recheck')}</button>`;
    h += `</div><p class="vc-head">${explain(e).head}${st.tone === 'working' || e.state === 'paused' ? ` ${explain(e).lines[0]}` : ''}</p>`;
    h += findingsBlock(e) + gridC(e);
    h += effBarsC(e);
    return h;
  }
  function gridC(e) {
    let h = `<p class="vc-legend" aria-label="Legend"><span>${glyph({ sym: '✓', tone: 'pass', label: 'Passed' })} passed</span><span>${glyph({ sym: '✗', tone: 'fail', label: 'Failed' })} failed</span><span>${glyph({ sym: '✓', tone: 'pass', label: 'Wrong finding', wrong: true })} wrong finding</span><span>${glyph({ sym: '✓', tone: 'pass', label: 'Borderline', borderline: true })} borderline</span><span>${glyph({ sym: '✓', tone: 'pass', label: 'Waiting on your recheck', pending: true })} recheck waiting</span><span>${glyph({ sym: '✓', tone: 'pass', label: 'You rechecked it', rechecked: true })} you rechecked</span><span>${glyph({ sym: '⊘', tone: 'void', label: 'Void' })} void</span>${e.state !== 'completed' ? `<span>${glyph({ sym: '–', tone: 'ungraded', label: 'Not graded yet' })} not graded</span><span>${glyph({ sym: '◌', tone: 'running', label: 'Running' })} running</span><span>${glyph({ sym: '·', tone: 'queued', label: 'Queued' })} queued</span>` : ''}</p>`;
    for (const s of suitesOf(e)) {
      const v = e.verdict && e.verdict.suites[s.name];
      h += `<section class="vc-suite"><h3>${esc(SUITE[s.name])} ${v ? small(v.verdict) : ''} <span class="x-dim">${s.tasks.length} Eval tasks × ${s.n} per Arm</span></h3>`;
      h += `<div class="vc-grid" role="table" aria-label="${esc(SUITE[s.name])} Trials" style="--n:${s.n}"><div class="vc-row vc-colhead" role="row"><span role="columnheader">Eval task</span><span role="columnheader" class="vc-armhead">Baseline</span><span role="columnheader" class="vc-armhead">Candidate</span><span role="columnheader">Diff.</span></div>`;
      for (const i of tasksOf(e, s.name)) {
        if (state.pendingOnly && !i.pending) continue;
        h += `<div class="vc-row${i.dropped ? ' vc-dropped' : ''}" role="row"><span role="rowheader"><button type="button" class="vc-task" data-peek-task="${esc(s.name)}|${esc(i.task)}" data-focus="ct-${esc(i.key)}">${esc(i.task)}</button></span>`;
        for (const side of ['b', 'c']) {
          h += `<span class="vc-cells" role="cell">${i[side].map(x => { const m = mark(e, x); const k = keyOf(x); return `<button type="button" class="vc-cell" data-peek-trial="${esc(k)}" data-focus="c-${esc(k)}" aria-label="${esc(`${ARM[x.arm]} Trial ${x.trial + 1}: ${m.label}`)}"${state.peek && state.peek.key === k ? ' aria-current="true"' : ''}>${glyph(m)}</button>`; }).join('')}</span>`;
        }
        h += `<span role="cell" class="vc-diff">${diffCell(i)}</span></div>`;
      }
      h += '</div></section>';
    }
    return h;
  }
  function effBarsC(e) {
    const rows = [
      ['Minutes', e.minutes.baseline, e.minutes.candidate, v => mins(v)],
      ['Tokens in + out', totalTokens(e.usage.baseline), totalTokens(e.usage.candidate), tok],
      ['Cost', e.usage.baseline.cost_usd, e.usage.candidate.cost_usd, usd],
    ];
    let h = '<section class="vc-eff" aria-label="Efficiency"><h3>Efficiency <span class="x-dim">outside the Bar</span></h3>';
    for (const [name, b, c, fmt] of rows) {
      if (b == null || c == null) { h += `<p class="vc-effrow"><span>${name}</span><span class="x-dim">${fmt(null)} by ${esc(e.candidate.harness)}</span></p>`; continue; }
      const max = Math.max(b, c) || 1;
      h += `<div class="vc-effrow"><span>${name}</span><span class="vc-bars"><span class="vc-bar" style="width:${((b / max) * 100).toFixed(1)}%"><span>Baseline ${fmt(b)}</span></span><span class="vc-bar vc-bar-c" style="width:${((c / max) * 100).toFixed(1)}%"><span>Candidate ${fmt(c)} <span class="x-dim">${change(b, c)}</span></span></span></span></div>`;
    }
    return `${h}</section><details class="vc-bardetails"><summary>The Bar</summary>${barFacts(e)}</details>`;
  }

  // ======== D · Differences: each Eval task on the interval's axis, as a forest plot.
  function variantD(e) {
    let h = header(e) + `<div class="vd-lead">${tag(e)}<p>${explain(e).head}</p></div>`;
    if (e.state !== 'completed') h += `<p class="x-dim">${explain(e).lines.join(' ')}</p>`;
    h += findingsBlock(e);
    if (e.pending_rechecks.length) h += `<p class="x-callout" data-tone="look"><strong>${plural(e.pending_rechecks.length, 'Score')} wait on your recheck.</strong> Rows marked ${glyph({ sym: '•', tone: 'look', label: 'has rechecks waiting' })} hold them; open a row, then a Trial's Details for its command.</p>`;
    h += `<div class="vd-tools"><span class="x-seg" role="group" aria-label="Metric">${[['accuracy', 'Accuracy'], ['wrong_findings', 'Wrong findings']].map(([k, t]) => `<button type="button" data-metric="${k}" aria-pressed="${state.metric === k}">${t}</button>`).join('')}</span></div>`;
    for (const s of suitesOf(e)) h += forestD(e, s);
    h += dumbbellsD(e);
    h += `<details class="vd-bar"><summary>The Bar</summary>${barFacts(e)}</details>`;
    return h;
  }
  function forestD(e, s) {
    const v = e.verdict && e.verdict.suites[s.name];
    const metric = state.metric;
    const infos = tasksOf(e, s.name);
    const value = i => (metric === 'accuracy' ? i.diff : i.wrongDiff);
    const scored = infos.filter(i => value(i) != null);
    const differing = scored.filter(i => value(i) !== 0).sort((a, b) => value(a) - value(b) || a.task.localeCompare(b.task));
    const even = scored.filter(i => value(i) === 0);
    const dropped = infos.filter(i => i.dropped), waiting = infos.filter(i => !i.dropped && value(i) == null);
    const AX = 1;
    const plot = (inner, cls = '') => `<span class="vd-plot ${cls}"><span class="vd-margin" style="left:${xpos(e.bar.margin, AX)}%"></span><span class="vd-zero" style="left:${xpos(0, AX)}%"></span>${inner}</span>`;
    const row = i => {
      const open = state.open.has(i.key);
      return `<li class="vd-row${open ? ' vd-open' : ''}"><button type="button" class="vd-label" data-open-task="${esc(i.key)}" aria-expanded="${open}" data-focus="d-${esc(i.key)}">${esc(i.task)}${i.pending ? ` ${glyph({ sym: '•', tone: 'look', label: `${plural(i.pending, 'recheck')} waiting` })}` : ''}</button>${plot(`<span class="vd-dot" data-sign="${Math.sign(value(i))}" style="left:${xpos(value(i), AX)}%"></span>`)}<span class="vd-val">${pts(value(i))}</span>${open ? `<div class="vd-trials">${trialsD(e, i)}</div>` : ''}</li>`;
    };
    let h = `<section class="vd-suite" aria-label="${esc(SUITE[s.name])}"><h3>${esc(SUITE[s.name])} ${v ? small(v.verdict) : ''} <span class="x-dim">${METRIC[metric]}, Candidate minus Baseline</span></h3>`;
    h += `<div class="vd-axisrow"><span></span><span class="vd-axis">${[-1, -0.5, e.bar.margin, 0, 0.5, 1].map(t => `<span style="left:${xpos(t, AX)}%"${t === e.bar.margin ? ' class="vd-axis-margin"' : Math.abs(t) === 0.5 ? ' class="vd-axis-half"' : ''}>${t === e.bar.margin ? `margin ${pts(t)}` : pts(t)}</span>`).join('')}</span><span></span></div><ol class="vd-rows">`;
    h += differing.map(row).join('');
    if (even.length) {
      const open = state.same.has(s.name);
      h += `<li class="vd-row vd-even"><button type="button" class="vd-label" data-same="${s.name}" aria-expanded="${open}">${plural(even.length, 'Eval task')} with no difference</button>${plot(`<span class="vd-dot vd-stack" style="left:${xpos(0, AX)}%"><span>${even.length}</span></span>`)}<span class="vd-val">0</span></li>`;
      if (open) h += even.map(row).join('');
    }
    // Until the model Graders score a task it has no difference to plot, so its row shows its Trials so far.
    if (waiting.length) {
      if (!scored.length) h += `<li class="vd-row vd-note"><span class="vd-label x-dim">Each Eval task gets its point once the model Graders score it, after the last Trial. Trials so far, Baseline then Candidate:</span></li>`;
      h += waiting.map(i => `<li class="vd-row vd-wait"><button type="button" class="vd-label" data-peek-task="${esc(s.name)}|${esc(i.task)}" data-focus="dw-${esc(i.key)}">${esc(i.task)}</button><span class="vd-progress">${i.b.map(x => glyph(mark(e, x))).join('')}<span class="vd-gap"></span>${i.c.map(x => glyph(mark(e, x))).join('')}</span><span class="vd-val x-dim">${i.b.concat(i.c).filter(isResult).length}/${2 * i.n}</span></li>`).join('');
    }
    if (dropped.length) h += `<li class="vd-row vd-note"><span class="vd-label x-dim">Dropped as void: ${dropped.map(i => esc(i.task)).join(', ')}</span></li>`;
    if (v) {
      const a = v[metric];
      h += `<li class="vd-row vd-summary"><span class="vd-label"><strong>Interval</strong> ${small(a.verdict)}</span>${plot(`<span class="vd-iv" data-tone="${tone(a.verdict)}" style="left:${xpos(a.interval.lower, AX)}%;width:${(xpos(a.interval.upper, AX) - xpos(a.interval.lower, AX)).toFixed(2)}%"></span><span class="vd-mean" style="left:${xpos(observed(a), AX)}%"></span>`)}<span class="vd-val">${range(a)}</span></li>`;
    }
    h += '</ol>';
    if (v) {
      h += `<p class="vd-why">${suiteWhy(e, s.name, v)}${v.pass_cubed ? ` pass³: Baseline ${share(v.pass_cubed.baseline)}, Candidate ${share(v.pass_cubed.candidate)}.` : ''}</p>`;
      const a = v[metric];
      if (a.more_tasks && a.more_tasks.clear_probabilities.length) h += moreD(a.more_tasks);
    }
    return `${h}</section>`;
  }
  function trialsD(e, i) {
    const col = side => `<div><p class="vd-armname">${ARM[SIDE[side]]}: ${side === 'b' ? i.pb : i.pc} of ${i.n}</p><ul>${i[side].map(x => { const m = mark(e, x); return `<li>${glyph(m)} <button type="button" class="x-linkish" data-peek-trial="${esc(keyOf(x))}">Trial ${x.trial + 1}</button> ${x.score && x.score.model ? prose(x.score.model.reason) : esc(m.label)}</li>`; }).join('')}</ul></div>`;
    return col('b') + col('c');
  }
  function moreD(more) {
    const probs = more.clear_probabilities, n = probs.length, w = 300, hgt = 64;
    const pt = (k, p) => `${((k / n) * w).toFixed(1)},${(hgt - p * hgt).toFixed(1)}`;
    const line = probs.map((p, i) => pt(i + 1, p)).join(' ');
    return `<figure class="vd-more"><svg viewBox="0 0 ${w} ${hgt}" preserveAspectRatio="none" role="img" aria-label="Chance of a clear Verdict with more Eval tasks: ${share(more.target_probability)} after ${more.additional_tasks}"><line x1="0" x2="${w}" y1="${hgt - more.target_probability * hgt}" y2="${hgt - more.target_probability * hgt}" class="vd-target"/><polyline points="0,${hgt} ${line}" class="vd-curve"/></svg><figcaption>Chance of a clear Verdict with more Eval tasks: ${more.additional_tasks ? `${share(more.target_probability)} at about ${more.additional_tasks} more` : `under ${share(more.target_probability)} even at ${n} more`}.</figcaption></figure>`;
  }
  function dumbbellsD(e) {
    const rows = [
      ['Minutes', e.minutes.baseline, e.minutes.candidate, v => mins(v)],
      ['Tokens', totalTokens(e.usage.baseline), totalTokens(e.usage.candidate), tok],
      ['Cost', e.usage.baseline.cost_usd, e.usage.candidate.cost_usd, usd],
    ];
    let h = '<section class="vd-eff" aria-label="Efficiency"><h3>Efficiency <span class="x-dim">outside the Bar · ● Baseline ◆ Candidate</span></h3><ol class="vd-rows">';
    for (const [name, b, c, fmt] of rows) {
      if (b == null || c == null) { h += `<li class="vd-row vd-note"><span class="vd-label">${name}</span><span class="x-dim">${fmt(null)} by ${esc(e.candidate.harness)}</span></li>`; continue; }
      const max = Math.max(b, c) * 1.1 || 1, xb = (b / max) * 100, xc = (c / max) * 100;
      h += `<li class="vd-row"><span class="vd-label">${name}</span><span class="vd-plot vd-dumb"><span class="vd-link" style="left:${Math.min(xb, xc)}%;width:${Math.abs(xc - xb)}%"></span><span class="vd-b" style="left:${xb}%" title="Baseline ${fmt(b)}"></span><span class="vd-c" style="left:${xc}%" title="Candidate ${fmt(c)}"></span></span><span class="vd-val">${fmt(b)} → ${fmt(c)} <span class="x-dim">${change(b, c)}</span></span></li>`;
    }
    return `${h}</ol></section>`;
  }

  // ======== E · Combined: B's lead and rechecks, D's suite intervals, A's Arms table, C's grid.
  function variantE(e) {
    const ex = explain(e);
    let h = header(e) + `<section class="vb-lead">${tag(e)}<p class="vb-head">${ex.head}</p>`;
    if (!e.verdict) h += ex.lines.map(l => `<p>${l}</p>`).join('');
    h += '</section>';
    if (e.verdict) h += intervalsE(e);
    h += findingsBlock(e);
    if (e.pending_rechecks.length) h += rechecksE(e);
    // While Trials run, the grid is what changes, so it comes before the Arms.
    const arms = `<h3 class="ve-h">What was compared</h3>${armsTableA(e)}`, trials = `<h3 class="ve-h">Trials</h3>${gridC(e)}`;
    h += e.state === 'completed' ? arms + trials : trials + arms;
    h += `<details class="va-bar"><summary>The Bar <span class="x-dim">· margin ${pts(e.bar.margin)} points, ${share(e.bar.credible_level)}, fixed ${esc(clock(e.created_at))}</span></summary>${barFacts(e)}</details>`;
    return h;
  }
  function intervalsE(e) {
    let h = '<section class="ve-verdict" aria-label="Verdict by suite">';
    for (const [name, v] of Object.entries(e.verdict.suites)) {
      h += `<h3>${esc(SUITE[name])} ${small(v.verdict)} <span class="x-dim">${v.retained_tasks} Eval tasks${v.dropped_tasks ? `, ${v.dropped_tasks} dropped` : ''}</span></h3>`;
      for (const key of ['accuracy', 'wrong_findings']) h += `<p class="vb-figure"><span>${cap(METRIC[key])} ${small(v[key].verdict)}</span>${iv(e, v[key], `${SUITE[name]} ${METRIC[key]}`)}<span>${range(v[key])}</span></p>`;
      h += `<p class="vb-figure vb-key"><span></span>${axisKey(e)}<span></span></p><p class="ve-why">${suiteWhy(e, name, v)}</p>`;
    }
    return `${h}</section><p class="x-dim ve-legend">Points of pass share, Candidate minus Baseline, at ${share(e.bar.credible_level)}. The dashed line is the ${pts(e.bar.margin)}-point margin. For wrong findings, plus means fewer in the Candidate.</p>`;
  }
  function rechecksE(e) {
    const all = e.pending_rechecks, open = state.open.has('rechecks'), FIRST = 5;
    const shown = open ? all : all.slice(0, FIRST);
    let h = `<section class="ve-rechecks x-callout" data-tone="look"><p><strong>Waiting on you: ${plural(all.length, 'recheck')}</strong> (${all.filter(p => p.borderline).length} borderline, ${all.filter(p => p.decisive).length} decisive). The Verdict is final once you have rechecked ${all.length === 1 ? 'it' : 'them all'}. Read the Trial, then record yours from the CLI; it is kept beside the Grader's Score. <button type="button" class="x-linkish" data-pending-only aria-pressed="${state.pendingOnly}">${state.pendingOnly ? 'Show every Eval task below' : 'Show only their Eval tasks below'}</button></p><ol>`;
    for (const p of shown) {
      const x = at(e, p.suite, p.task, p.arm, p.trial);
      h += `<li><button type="button" class="x-linkish" data-peek-trial="${esc(keyOf(x))}">${glyph(mark(e, x))} ${esc(p.task)} · ${ARM[p.arm]} Trial ${p.trial + 1}</button><span class="x-dim">${esc(cap(why(x, p)))}</span><button type="button" data-copy="${esc(recheckCli(e, p, x))}">Copy command</button></li>`;
    }
    h += '</ol>';
    if (all.length > FIRST) h += `<p><button type="button" class="x-linkish" data-toggle="rechecks" aria-expanded="${open}">${open ? 'Show fewer' : `Show all ${all.length}`}</button></p>`;
    return `${h}</section>`;
  }

  // ======== Experiments list layouts.
  const experimentHref = e => esc(url({ page: 'experiment', x: e.proto.key }));
  const listHead = () => '<div class="page-title"><h2>Experiments</h2><code>thirdshift experiments --json</code></div>'
    + '<p class="x-dim">Each Experiment compares a Baseline arm with a Candidate arm on the Eval suites a change touches. Start one from the CLI with <code>thirdshift experiment &lt;candidate&gt;</code>; <code>scripts/release.sh</code> starts its own. The Console only shows them.</p>';
  const ordered = () => F.experiments.slice().sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at));
  const changeLine = e => (e.proto.kind === 'flags'
    ? `${['harness', 'model', 'effort'].filter(f => e.baseline[f] !== e.candidate[f]).map(f => `${f}: ${e.baseline[f]} → ${e.candidate[f]}`).join(', ')} · one build, <code>${short(e.candidate.commit)}</code>`
    : `${esc(e.proto.baseline_ref)} <code>${short(e.baseline.commit)}</code> → ${esc(e.proto.candidate_ref)} <code>${short(e.candidate.commit)}</code>`);
  // What kept a finished Experiment from passing, by suite and metric.
  const decider = e => {
    if (!e.verdict || e.verdict.provisional_verdict === 'pass') return '';
    const bits = [];
    for (const [name, v] of Object.entries(e.verdict.suites)) {
      const total = v.retained_tasks + v.dropped_tasks;
      if (v.dropped_tasks > Math.floor(total / 10)) { bits.push(`${SUITE[name]}: ${v.dropped_tasks} of ${total} Eval tasks dropped`); continue; }
      for (const key of ['accuracy', 'wrong_findings']) if (v[key].verdict !== 'pass') bits.push(`${SUITE[name]} ${METRIC[key]}: ${v[key].verdict}`);
    }
    return bits.join(' · ');
  };
  const nowLine = e => {
    if (e.state === 'running') return `${e.proto.stage === 'Model Graders' ? `Model Graders ${e.proto.graded || 0} of ${suitesOf(e).reduce((s, x) => s + x.tasks.length, 0)}` : `Trials ${e.results.length} of ${totalOf(e)}, ${e.proto.running.length} running`}`;
    if (e.state === 'paused') return `Paused at a ${cap(e.proto.pause.harness)} plan-limit warning · ${e.results.length} of ${totalOf(e)} Trials kept`;
    const v = e.verdict;
    const bits = decider(e) ? [decider(e)] : Object.entries(v.suites).map(([name, sv]) => `${SUITE[name]} ${sv.verdict}`);
    if (v.verdict === 'waiting_on_rechecks') bits.unshift(`${plural(e.pending_rechecks.length, 'recheck')} waiting, provisionally ${v.provisional_verdict}`);
    return bits.join(' · ');
  };

  function list1() {
    let h = `${listHead()}<ul class="log-list l1">`;
    for (const e of ordered()) {
      h += `<li><p class="l1-head"><a href="${experimentHref(e)}" data-nav="experiment:${e.proto.key}">${esc(e.proto.name)}</a> ${tag(e)}</p><dl>`
        + `<dt>Change</dt><dd>${esc(e.proto.title)}</dd>`
        + `<dt>Arms</dt><dd>${changeLine(e)}</dd>`
        + `<dt>Eval suites</dt><dd>${suitesOf(e).map(s => `${SUITE[s.name]} (${s.tasks.length} × ${s.n})`).join(', ')}</dd>`
        + `<dt>Now</dt><dd>${esc(nowLine(e))}</dd>`
        + (e.verdict ? `<dt>Intervals</dt><dd>${Object.entries(e.verdict.suites).map(([name, v]) => `${SUITE[name]}: accuracy ${range(v.accuracy)}, wrong findings ${range(v.wrong_findings)}`).join('; ')}</dd>` : '')
        + `<dt>Started</dt><dd>${esc(clock(e.created_at))}${e.proto.ended_at ? `, ended ${esc(clock(e.proto.ended_at))}` : ''}</dd>`
        + `</dl></li>`;
    }
    return `${h}</ul>`;
  }

  function list2() {
    let h = `${listHead()}<table class="l2"><thead><tr><th scope="col">Verdict</th><th scope="col">Experiment</th><th scope="col">Accuracy, Candidate minus Baseline</th><th scope="col">Trials</th><th scope="col">Started</th></tr></thead><tbody>`;
    for (const e of ordered()) {
      const ivs = e.verdict
        ? Object.entries(e.verdict.suites).map(([name, v]) => `<span class="l2-iv"><span class="x-dim">${esc(SUITE[name])}</span>${iv(e, v.accuracy, `${SUITE[name]} accuracy`)}<span>${range(v.accuracy)}</span></span>`).join('')
        : `<span class="x-dim">${e.state === 'running' ? 'after the Graders' : 'after resuming'}</span>`;
      const prog = e.results.length / totalOf(e);
      h += `<tr data-nav="experiment:${e.proto.key}"><td data-label="Verdict">${tag(e)}${e.pending_rechecks.length ? `<br><span class="x-dim">${plural(e.pending_rechecks.length, 'recheck')}, provisionally ${e.verdict.provisional_verdict}</span>` : decider(e) ? `<br><span class="x-dim">${esc(decider(e))}</span>` : ''}</td>`
        + `<td data-label="Experiment"><a href="${experimentHref(e)}" data-nav="experiment:${e.proto.key}">${esc(e.proto.name)}</a><br><span class="l2-title">${esc(e.proto.title)}</span><br><span class="x-dim">${changeLine(e)}</span></td>`
        + `<td data-label="Accuracy">${ivs}</td>`
        + `<td data-label="Trials"><span class="l2-prog" style="--p:${prog}"></span>${e.results.length}/${totalOf(e)}</td>`
        + `<td data-label="Started"><time datetime="${esc(e.created_at)}" title="${esc(clock(e.created_at))}">${esc(ago(e.created_at))}</time></td></tr>`;
    }
    return `${h}</tbody></table><p class="x-dim l2-key">${axisKey(F.experiments[0])}</p>`;
  }

  function list3() {
    const groups = [
      ['Running', e => e.state === 'running'],
      ['Needs you', e => e.state === 'paused' || (e.verdict && e.verdict.verdict !== 'pass')],
      ['Done', e => e.verdict && e.verdict.verdict === 'pass'],
    ];
    let h = listHead();
    for (const [title, test] of groups) {
      const rows = ordered().filter(test);
      h += `<details class="group" open><summary><span>${title}</span><span class="count">${rows.length}</span></summary><ul class="commands l3">`;
      if (!rows.length) h += '<p class="empty">Nothing here.</p>';
      for (const e of rows) {
        const st = status(e);
        h += `<li class="command l3-row" data-tone="${st.tone}" data-live="${e.state === 'running'}"><div class="row-head">${e.state === 'running' ? '<span class="heartbeat" aria-label="Log activity"></span>' : ''}${tag(e)} <span class="subject"><a href="${experimentHref(e)}" data-nav="experiment:${e.proto.key}">${esc(e.proto.name)}</a></span> <span class="issue-title">${esc(e.proto.title)}</span><span class="age">${e.proto.ended_at ? `Ended ${esc(ago(e.proto.ended_at))} · ` : ''}${esc(ago(e.created_at))}</span></div>`
          + `<p class="now">${esc(nowLine(e))}</p>`
          + (e.verdict ? `<p class="l3-ivs">${Object.entries(e.verdict.suites).map(([name, v]) => `<span><span class="x-dim">${esc(SUITE[name])}</span> ${iv(e, v.accuracy, `${SUITE[name]} accuracy`)} ${range(v.accuracy)}</span>`).join('')}</p>` : '')
          + `<p class="harness">${changeLine(e)}</p>`
          + `<p><a href="${experimentHref(e)}" data-nav="experiment:${e.proto.key}">Experiment</a> · <a href="#" data-dead="Opens the Command details in the Peek, as on the Inbox.">Details</a> · <a href="#" data-dead="Opens the Experiment's Command log.">Full log</a></p></li>`;
      }
      h += '</ul></details>';
    }
    return h;
  }

  const RENDER = { A: variantA, B: variantB, C: variantC, D: variantD, E: variantE };
  const LIST_RENDER = { 1: list1, 2: list2, 3: list3 };

  // ---- Rendering, the Peek and the switcher.
  const main = document.getElementById('proto-main');
  const peek = document.getElementById('peek');
  const peekContent = document.getElementById('peek-content');

  function renderPeek(e) {
    if (!state.peek || state.page !== 'experiment') return closePeek(false);
    let html;
    if (state.peek.type === 'trial') {
      const [suite, task, arm, trial] = state.peek.key.split('|');
      html = trialPeek(e, at(e, suite, task, arm, +trial));
    } else if (state.peek.type === 'task') html = taskPeek(e, state.peek.suite, state.peek.task);
    else html = barPeek(e);
    peek.hidden = false;
    document.body.classList.add('peek-open');
    peekContent.innerHTML = html;
  }
  function closePeek(clear = true) {
    if (clear) state.peek = null;
    peek.hidden = true;
    document.body.classList.remove('peek-open');
  }

  function render() {
    document.documentElement.dataset.theme = state.theme;
    document.body.classList.toggle('proto-embed', state.embed);
    if (state.phone && !state.embed) {
      document.body.classList.add('proto-phone-host');
      let frame = document.getElementById('proto-frame');
      if (!frame) {
        frame = document.createElement('iframe');
        frame.id = 'proto-frame';
        frame.title = '390 px phone preview';
        document.body.prepend(frame);
      }
      const src = url({ embed: '1', frame: null });
      if (frame.dataset.src !== src) { frame.dataset.src = src; frame.src = src; }
      updateBar();
      history.replaceState(null, '', url());
      syncLive();
      return;
    }
    document.body.classList.remove('proto-phone-host');
    const frame = document.getElementById('proto-frame');
    if (frame) frame.remove();
    const e = current();
    const focused = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.focus : null;
    main.innerHTML = state.page === 'list' ? LIST_RENDER[state.list]() : RENDER[state.variant](e);
    document.title = `${state.page === 'list' ? 'Experiments' : e.proto.name} · prototype · thirdshift`;
    document.documentElement.dataset.live = String(state.page === 'list' ? F.experiments.some(x => x.state === 'running') : e.state === 'running');
    renderPeek(e);
    if (focused) {
      const again = main.querySelector(`[data-focus="${CSS.escape(focused)}"]`) || peekContent.querySelector(`[data-focus="${CSS.escape(focused)}"]`);
      if (again) again.focus({ preventScroll: true });
    }
    updateBar();
    history.replaceState(null, '', url());
    if (state.embed && parent !== window) parent.postMessage({ protoState: { page: state.page, x: state.x, variant: state.variant, list: state.list } }, '*');
    syncLive();
  }

  function updateBar() {
    const v = VARIANTS.find(x => x[0] === state.variant), l = LISTS.find(x => x[0] === state.list);
    document.getElementById('proto-variant').textContent = `${v[0]} · ${v[1]}`;
    document.getElementById('proto-list').textContent = `${l[0]} · ${l[1]}`;
    document.getElementById('proto-variant').toggleAttribute('data-active', state.page === 'experiment');
    document.getElementById('proto-list').toggleAttribute('data-active', state.page === 'list');
    document.getElementById('proto-x').value = state.x;
    document.getElementById('proto-theme').value = state.theme;
    document.getElementById('proto-phone').setAttribute('aria-pressed', String(state.phone));
    const run = F.experiments.find(x => x.proto.key === 'running');
    const live = document.getElementById('proto-live');
    live.setAttribute('aria-pressed', String(state.live));
    live.textContent = run.state === 'completed' ? 'Replay live run' : state.live ? 'Live: on' : 'Live: off';
  }

  function step(axis, dir) {
    const list = axis === 'variant' ? VARIANTS : LISTS;
    const i = list.findIndex(x => x[0] === state[axis]);
    state[axis] = list[(i + dir + list.length) % list.length][0];
    state.page = axis === 'variant' ? 'experiment' : 'list';
    render();
  }

  // ---- Live: replay the running Experiment's remaining Trials, Graders and Verdict.
  let timer = null;
  function tick() {
    const run = F.experiments.find(x => x.proto.key === 'running');
    if (run.state !== 'running') return syncLive();
    if (run.proto.stage === 'Trials') {
      if (!F.live.advance(run)) { run.proto.stage = 'Model Graders'; run.proto.graded = 0; run.proto.running = []; }
    } else {
      const tasks = suitesOf(run).flatMap(s => s.tasks.map(t => [s.name, t]));
      const [suite, task] = tasks[run.proto.graded];
      F.live.grade(run, suite, task);
      run.proto.graded++;
      if (run.proto.graded >= tasks.length) {
        F.live.finish(run);
        run.proto.stage = null;
        run.proto.ended_at = new Date(NOW).toISOString();
      }
    }
    render();
  }
  function syncLive() {
    const run = F.experiments.find(x => x.proto.key === 'running');
    const visible = state.page === 'list' || state.x === 'running';
    const want = state.live && !(state.phone && !state.embed) && run.state === 'running' && visible;
    if (want && !timer) timer = setInterval(tick, params.get('speed') === 'fast' ? 120 : 1700);
    if (!want && timer) { clearInterval(timer); timer = null; }
  }

  // ---- Events.
  let toastTimer = null;
  function toast(text) {
    const t = document.getElementById('proto-toast');
    t.textContent = text;
    t.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { t.hidden = true; }, 2600);
  }

  document.addEventListener('click', ev => {
    const t = ev.target.closest('[data-nav],[data-open-task],[data-peek-trial],[data-peek-task],[data-peek-bar],[data-copy],[data-dead],[data-suite],[data-rep],[data-filter],[data-metric],[data-same],[data-close-peek],[data-step],[data-toggle],[data-pending-only],[data-proto-elsewhere]');
    if (!t) return;
    if (t.matches('[data-step]')) return step(t.dataset.step, +t.dataset.dir);
    if (t.matches('[data-close-peek]')) return closePeek();
    if (t.matches('[data-dead]')) { ev.preventDefault(); return toast(`Not in this prototype. ${t.dataset.dead}`); }
    if (t.matches('[data-proto-elsewhere]')) { ev.preventDefault(); return toast('Other Console pages are outside this prototype.'); }
    if (t.matches('[data-copy]')) {
      const done = () => toast('Copied');
      if (navigator.clipboard) navigator.clipboard.writeText(t.dataset.copy).then(done, () => toast(t.dataset.copy));
      else toast(t.dataset.copy);
      return;
    }
    ev.preventDefault();
    if (t.matches('[data-nav]')) {
      const [page, key] = t.dataset.nav.split(':');
      state.page = page;
      if (key) state.x = key;
      state.peek = null; state.open.clear(); state.suite = null; state.filter = 'all'; state.pendingOnly = false;
      window.scrollTo(0, 0);
    } else if (t.matches('[data-open-task]')) {
      const key = t.dataset.openTask;
      state.open.has(key) ? state.open.delete(key) : state.open.add(key);
    } else if (t.matches('[data-peek-trial]')) state.peek = { type: 'trial', key: t.dataset.peekTrial };
    else if (t.matches('[data-peek-task]')) { const [suite, task] = t.dataset.peekTask.split('|'); state.peek = { type: 'task', suite, task }; }
    else if (t.matches('[data-peek-bar]')) state.peek = { type: 'bar' };
    else if (t.matches('[data-suite]')) state.suite = t.dataset.suite;
    else if (t.matches('[data-rep]')) state.rep = t.dataset.rep;
    else if (t.matches('[data-filter]')) { state.filter = t.dataset.filter; state.open.delete('rechecks'); }
    else if (t.matches('[data-metric]')) state.metric = t.dataset.metric;
    else if (t.matches('[data-same]')) { const s = t.dataset.same; state.same.has(s) ? state.same.delete(s) : state.same.add(s); }
    else if (t.matches('[data-toggle]')) { const k = t.dataset.toggle; state.open.has(k) ? state.open.delete(k) : state.open.add(k); }
    else if (t.matches('[data-pending-only]')) state.pendingOnly = !state.pendingOnly;
    render();
  });
  // Report-layout folds are native <details>; remember them across live renders.
  main.addEventListener('toggle', ev => {
    const d = ev.target;
    if (!d.dataset || !d.dataset.key) return;
    d.open ? state.open.add(d.dataset.key) : state.open.delete(d.dataset.key);
  }, true);

  document.getElementById('proto-x').addEventListener('change', ev => { state.x = ev.target.value; state.page = 'experiment'; state.peek = null; state.open.clear(); state.suite = null; state.filter = 'all'; render(); });
  document.getElementById('proto-theme').addEventListener('change', ev => { state.theme = ev.target.value; render(); });
  document.getElementById('proto-phone').addEventListener('click', () => { state.phone = !state.phone; render(); });
  document.getElementById('proto-live').addEventListener('click', () => {
    const run = F.experiments.find(x => x.proto.key === 'running');
    if (run.state === 'completed') { F.reset('running'); state.live = true; state.x = 'running'; state.page = state.page === 'list' ? 'list' : 'experiment'; }
    else state.live = !state.live;
    render();
    if (state.phone && !state.embed) { const frame = document.getElementById('proto-frame'); if (frame) { frame.dataset.src = ''; render(); } }
  });
  addEventListener('message', ev => {
    if (!ev.data || !ev.data.protoState || state.embed) return;
    Object.assign(state, ev.data.protoState);
    const frame = document.getElementById('proto-frame');
    if (frame) frame.dataset.src = url({ embed: '1', frame: null });
    updateBar();
    history.replaceState(null, '', url());
  });
  document.addEventListener('keydown', ev => {
    if (ev.target.closest && ev.target.closest('input, textarea, select, [contenteditable]')) return;
    if (ev.key === 'Escape' && state.peek) { closePeek(); render(); return; }
    if ((ev.key !== 'ArrowLeft' && ev.key !== 'ArrowRight') || ev.altKey || ev.ctrlKey || ev.metaKey) return;
    ev.preventDefault();
    step(ev.shiftKey ? 'list' : 'variant', ev.key === 'ArrowLeft' ? -1 : 1);
  });

  const xs = document.getElementById('proto-x');
  for (const e of F.experiments) xs.add(new Option(`${status(e).word} · ${e.proto.name}`, e.proto.key));
  const ts = document.getElementById('proto-theme');
  for (const t of THEMES) ts.add(new Option(t, t));
  render();
})();
