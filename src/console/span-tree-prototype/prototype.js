// PROTOTYPE (#995), throwaway: span-tree layouts A–D and Peek layouts 1–3
// over real Session traces. No tests, no persistence; state lives in the URL.
const F = window.FIXTURE;
const NOW = F.now;
const VARIANTS = { A: 'Tree + timeline', B: 'Drill-down list', C: 'Flame chart', D: 'Waterfall grown in place', E: 'Tree + slowest calls' };
const DETAILS = { 1: 'Tabs', 2: 'Facts list', 3: 'Summary card' };
const params = new URLSearchParams(location.search);
const state = {
  variant: VARIANTS[params.get('variant')] ? params.get('variant') : 'A',
  details: DETAILS[params.get('details')] ? params.get('details') : '1',
  command: Math.min(+params.get('command') || 0, F.commands.length - 1),
  phone: params.get('frame') === '390',
  embed: params.get('embed') === '1',
};
const KIND = { command: 'Command', run: 'Run', stage: 'Stage', session: 'Session', model: 'Model call', turn: 'Turn', tool: 'Tool call', agent: 'Sub-agent', span: 'Span' };
const TAG = { command: 'cmd', run: 'run', stage: 'stage', session: 'sess', model: 'model', turn: 'turn', tool: 'tool', agent: 'agent', span: 'span' };

// ---------- model ----------
const byId = new Map();
for (const command of F.commands) prepare(command.tree, null, 0, command);
function prepare(node, parent, depth, command) {
  node.parent = parent; node.depth = depth; node.command = command;
  node.endOrNow = node.end ?? NOW;
  if (node.start == null) node.start = parent?.start ?? NOW;
  byId.set(node.id, node);
  const r = { models: 0, tools: 0, errors: 0, agents: 0, input: 0, output: 0, cacheRead: 0, cost: 0, running: 0 };
  const own = node.attrs || {};
  if (node.kind === 'model' || node.kind === 'turn') r.models++;
  if (node.kind === 'tool') r.tools++;
  if (node.kind === 'agent') r.agents++;
  if (node.status === 'error') r.errors++;
  if (node.status === 'running' && node.kind === 'tool') r.running++;
  if (node.kind !== 'session') { // a session span repeats its calls' totals
    r.input += +own['gen_ai.usage.input_tokens'] || 0;
    r.output += +own['gen_ai.usage.output_tokens'] || 0;
    r.cacheRead += +own['thirdshift.usage.cache_read_tokens'] || 0;
  }
  r.cost += node.kind === 'session' ? (+own['thirdshift.cost.usd'] || 0) : 0;
  for (const child of node.children) {
    prepare(child, node, depth + 1, command);
    for (const k in r) r[k] += child.rollup[k];
  }
  if (node.kind === 'session' && !r.input) { // Codex: usage only on the session/turns
    r.input = +own['gen_ai.usage.input_tokens'] || 0; r.output = +own['gen_ai.usage.output_tokens'] || 0;
  }
  node.rollup = r;
}
const command = () => F.commands[state.command];
const root = () => command().tree;
const ancestors = node => { const out = []; for (let n = node.parent; n; n = n.parent) out.unshift(n); return out; };
const spanCli = node => `thirdshift logs '${node.command.selector}' --span ${node.id} --json`;

// ---------- formatting ----------
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
function dur(ms) {
  if (ms == null || isNaN(ms)) return 'unknown';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)} s`;
  const m = Math.floor(s / 60), h = Math.floor(m / 60);
  if (h) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${m}m ${String(Math.round(s % 60)).padStart(2, '0')}s`;
}
const clock = ms => ms == null ? '—' : new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const num = n => n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e4 ? `${Math.round(n / 1e3)}k` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n);
const length = n => n.endOrNow - n.start;
const tag = n => `<span class="k k-${n.kind}">${TAG[n.kind]}</span>`;
const statusText = n => n.status === 'error' ? `<span class="st-error">✕ ${esc(n.statusMessage || 'error')}</span>`
  : n.status === 'running' ? `<span class="st-running">● running</span>` : '';
function rollupText(n, { short = false } = {}) {
  const r = n.rollup, bits = [dur(length(n)) + (n.estimated ? ' est.' : '')];
  if (n.kind !== 'tool' && n.kind !== 'model' && n.kind !== 'turn') {
    if (r.models) bits.push(`${r.models} model`);
    if (r.tools) bits.push(`${r.tools} tool`);
  }
  if (!short && (r.input || r.output)) bits.push(`${num(r.input)}→${num(r.output)} tok`);
  if (!short && r.cost) bits.push(`$${r.cost.toFixed(2)}`);
  if (r.errors) bits.push(`<span class="st-error">${r.errors} err</span>`);
  if (r.running) bits.push(`<span class="st-running">${r.running} running</span>`);
  return bits.join(' · ');
}
function bar(n, axis, extra = '') {
  const span = Math.max(axis.end - axis.start, 1);
  const left = Math.max(0, Math.min(100, (n.start - axis.start) / span * 100));
  const width = Math.max(0, Math.min(100 - left, (n.endOrNow - Math.max(n.start, axis.start)) / span * 100));
  const cls = [n.kind, n.status === 'error' && 'error', n.status === 'running' && 'running', n.estimated && 'est'].filter(Boolean).join(' ');
  return `<div class="bar ${extra}" aria-hidden="true"><i class="${cls}" style="left:${left}%;width:${width}%"></i></div>`;
}
const axisOf = n => ({ start: n.start, end: Math.max(n.endOrNow, n.start + 1) });
function ticks(axis, count = 5) {
  let out = '';
  for (let i = 0; i <= count; i++) {
    const at = i / count * 100;
    out += `<span style="left:${Math.min(Math.max(at, 4), 96)}%">+${dur((axis.end - axis.start) * i / count)}</span>`;
  }
  return out;
}
const legend = () => `<p class="legend"><span><span class="bar"><i class="tool" style="left:0;width:100%"></i></span> tool call</span><span><span class="bar"><i class="model" style="left:0;width:100%"></i></span> model call / turn</span><span><span class="bar"><i class="error" style="left:0;width:100%"></i></span> error</span><span><span class="bar"><i class="running" style="left:0;width:100%"></i></span> running</span><span><span class="bar"><i class="tool est" style="left:0;width:100%"></i></span> estimated time (converted Session log)</span></p>`;

// ---------- selection and Peek ----------
let selected = null;
const peek = document.querySelector('#peek'), peekContent = document.querySelector('#peek-content');
function select(node, { open = true } = {}) {
  selected = node;
  document.querySelectorAll('.is-selected').forEach(e => e.classList.remove('is-selected'));
  document.querySelectorAll(`[data-id="${CSS.escape(node.id)}"]`).forEach(e => e.classList.add('is-selected'));
  if (open) openPeek(node);
}
function openPeek(node) {
  peek.hidden = false;
  document.body.classList.add('peek-open');
  peekContent.innerHTML = PEEK[state.details](node);
  peekContent.querySelectorAll('[data-goto]').forEach(a => a.addEventListener('click', e => { e.preventDefault(); const n = byId.get(a.dataset.goto); reveal(n); select(n); }));
  peekContent.querySelectorAll('[data-fit]').forEach(b => b.addEventListener('click', () => { fit(byId.get(b.dataset.fit) || node); openPeek(node); showRow(node.id); }));
  peekContent.querySelectorAll('[data-unzoom]').forEach(b => b.addEventListener('click', () => { unzoom(); openPeek(node); showRow(node.id); }));
  peekContent.querySelectorAll('[data-copy]').forEach(b => b.addEventListener('click', () => { navigator.clipboard?.writeText(b.dataset.copy); b.textContent = 'Copied'; }));
  peekContent.querySelectorAll('.pd-tabs button').forEach(b => b.addEventListener('click', () => {
    peekContent.querySelectorAll('.pd-tabs button').forEach(x => x.setAttribute('aria-selected', x === b));
    peekContent.querySelectorAll('[data-tab]').forEach(p => { p.hidden = p.dataset.tab !== b.dataset.tabFor; });
  }));
}
function closePeek() { peek.hidden = true; document.body.classList.remove('peek-open'); }
document.querySelector('[data-close-peek]').addEventListener('click', closePeek);

function pretty(text) {
  if (text == null || text === '') return null;
  try { const v = JSON.parse(text); if (typeof v === 'object') return JSON.stringify(v, null, 2); return String(v); } catch { return text; }
}
function io(node) {
  const a = node.attrs || {};
  switch (node.kind) {
    case 'tool': return { input: pretty(a['gen_ai.tool.call.arguments']), output: pretty(a['gen_ai.tool.call.result']) };
    case 'session': return { input: pretty(a['thirdshift.session.prompt']) || '(No Session prompt recorded: converted from a Session log.)', output: pretty(a['thirdshift.text']) };
    case 'agent': return { input: pretty(a['thirdshift.description']), output: null };
    case 'model': case 'turn': return { input: null, output: null, note: 'The model call\'s text is in its log records, not on the span. The conversation view shows it.' };
    case 'run': case 'command': return { input: null, output: pretty(a.cause || a.outcome) };
    default: return { input: null, output: null };
  }
}
function overview(node) {
  const a = node.attrs || {}, r = node.rollup, parent = node.parent;
  const rows = [
    ['Kind', KIND[node.kind]], ['Status', node.status === 'ok' ? 'Succeeded' : node.status === 'running' ? 'Running' : node.status === 'error' ? `Error: ${node.statusMessage || ''}` : 'Unset'],
    ['Start', clock(node.start)], ['End', node.end ? clock(node.end) : 'still running'],
    ['Duration', dur(length(node)) + (node.estimated ? ' (estimated from the Command log)' : '')],
  ];
  if (parent) rows.push([`Share of ${KIND[parent.kind].toLowerCase()}`, `${Math.round(length(node) / Math.max(length(parent), 1) * 100)}%`]);
  const model = a['gen_ai.response.model'] || a['gen_ai.request.model'] || a.model;
  if (model) rows.push(['Model', model + (a['thirdshift.request.effort'] || a.effort ? ` · ${a['thirdshift.request.effort'] || a.effort}` : '')]);
  if (a.harness || a['resource thirdshift.harness']) rows.push(['Harness', (a.harness || a['resource thirdshift.harness']) + (a['resource thirdshift.harness.version'] ? ` (${a['resource thirdshift.harness.version']})` : '')]);
  if (a['gen_ai.tool.name']) rows.push(['Tool', a['gen_ai.tool.name']]);
  if (r.input || r.output) rows.push(['Tokens in / out', `${r.input.toLocaleString()} / ${r.output.toLocaleString()}` + (r.cacheRead ? ` (${r.cacheRead.toLocaleString()} cache read)` : '')]);
  if (r.cost) rows.push(['Cost', `$${r.cost.toFixed(2)}`]);
  if (node.children.length) rows.push(['Contains', [r.models && `${r.models} model calls`, r.tools && `${r.tools} tool calls`, r.agents && `${r.agents} sub-agents`, r.errors && `${r.errors} errors`].filter(Boolean).join(', ') || `${node.children.length} spans`]);
  if (a.pull_request) rows.push(['Pull request', a.pull_request]);
  if (node.file) rows.push(['Session trace', node.file]);
  rows.push(['Span ID', node.id]);
  return rows;
}
const table = rows => `<table class="pd-table">${rows.map(([k, v]) => `<tr><th>${esc(k)}</th><td>${esc(v)}</td></tr>`).join('')}</table>`;
const path = node => `<p class="pd-path">${ancestors(node).map(n => `<a href="#" data-goto="${esc(n.id)}">${esc(TAG[n.kind])} ${esc(n.name.length > 30 ? n.name.slice(0, 30) + '…' : n.name)}</a>`).join(' › ')}</p>`;
const attrRows = node => Object.entries(node.attrs || {}).sort().map(([k, v]) => [k, v]);

// A and E: zoom to a span with children; a call zooms to its parent, where it can be read.
function zoomButton(node) {
  if (state.variant === 'C') return '<button type="button" data-fit>Zoom to this span</button>';
  if (state.variant === 'B') return '<button type="button" data-fit>Open this span\'s children</button>';
  if (state.variant === 'D') return '';
  if (!node.parent) return '';
  if (A.fitNode === node) return '<button type="button" data-unzoom>Zoom out to whole Command</button>';
  return `<button type="button" data-fit="${esc(node.id)}">${node.children.length ? `Zoom timeline to this ${esc(KIND[node.kind].toLowerCase())}` : 'Zoom timeline around this call'}</button>`;
}
const PEEK = {
  1(node) { // Tabs, after MLflow
    const { input, output, note } = io(node);
    return `<section aria-label="Span details"><div class="pd-head">${tag(node)}<h2>${esc(node.name)}</h2></div>${path(node)}<p>${statusText(node) || ''} ${dur(length(node))}</p>
      <div class="pd-tabs" role="tablist"><button role="tab" aria-selected="true" data-tab-for="o">Overview</button><button role="tab" aria-selected="false" data-tab-for="io">Inputs / Outputs</button><button role="tab" aria-selected="false" data-tab-for="a">Attributes (${Object.keys(node.attrs || {}).length})</button></div>
      <div data-tab="o">${table(overview(node))}<p><code>${esc(spanCli(node))}</code></p></div>
      <div data-tab="io" hidden>${note ? `<p class="dim">${esc(note)}</p>` : ''}${input ? `<h3>Input</h3><pre class="pd-pre">${esc(input)}</pre>` : ''}${output ? `<h3>Output</h3><pre class="pd-pre">${esc(output)}</pre>` : ''}${!input && !output && !note ? '<p class="dim">None recorded.</p>' : ''}</div>
      <div data-tab="a" hidden>${table(attrRows(node))}</div></section>`;
  },
  2(node) { // The Console's own Peek: one scroll of facts
    const { input, output, note } = io(node);
    const facts = overview(node).map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('');
    return `<section aria-label="Span details"><h2>${esc(KIND[node.kind])} details</h2><code>${esc(spanCli(node))}</code>
      <p><a href="#">Conversation at this ${esc(KIND[node.kind].toLowerCase())}</a> · <a href="#">Full Command log</a></p>${path(node)}
      <dl><dt>name</dt><dd>${esc(node.name)}</dd>${facts}
      ${note ? `<dt>text</dt><dd>${esc(note)}</dd>` : ''}${input ? `<dt>input</dt><dd><pre class="pd-pre">${esc(input)}</pre></dd>` : ''}${output ? `<dt>output</dt><dd><pre class="pd-pre">${esc(output)}</pre></dd>` : ''}
      <dt>attributes</dt><dd><dl>${attrRows(node).map(([k, v]) => `<dt style="text-transform:none">${esc(k)}</dt><dd>${esc(v)}</dd>`).join('')}</dl></dd></dl></section>`;
  },
  3(node) { // Summary card: numbers first, raw behind disclosures
    const { input, output, note } = io(node), r = node.rollup, parent = node.parent;
    const tiles = [[dur(length(node)) + (node.estimated ? '*' : ''), 'duration'], parent && [`${Math.round(length(node) / Math.max(length(parent), 1) * 100)}%`, `of its ${KIND[parent.kind].toLowerCase()}`],
      (r.input || r.output) && [`${num(r.input)} / ${num(r.output)}`, 'tokens in / out'], r.cost && [`$${r.cost.toFixed(2)}`, 'cost'],
      node.children.length && [`${r.models} / ${r.tools}`, 'model / tool calls'], r.errors && [String(r.errors), 'errors']].filter(Boolean);
    return `<section aria-label="Span details">${path(node)}<div class="pd-head">${tag(node)}<h2>${esc(node.name)}</h2></div><p>${statusText(node)} <span class="dim">${clock(node.start)} → ${node.end ? clock(node.end) : 'now'}${node.estimated ? ' · *estimated' : ''}</span></p>
      <div class="pd-tiles">${tiles.map(([v, l]) => `<div class="pd-tile"><b>${esc(v)}</b><span>${esc(l)}</span></div>`).join('')}</div>
      ${parent ? `<p class="dim" style="font-size:12px;margin:0">Within its ${esc(KIND[parent.kind].toLowerCase())}</p>${bar(node, axisOf(parent))}` : ''}
      <p class="dim" style="font-size:12px;margin:6px 0 0">Within the Command</p>${bar(node, axisOf(node.command.tree))}
      <div class="pd-actions">${zoomButton(node)}<a href="#">Open in conversation</a><button type="button" data-copy="${esc(spanCli(node))}">Copy CLI</button></div>
      ${note ? `<p class="dim">${esc(note)}</p>` : ''}
      ${input ? `<details open><summary>Input</summary><pre class="pd-pre">${esc(input)}</pre></details>` : ''}
      ${output ? `<details><summary>Output</summary><pre class="pd-pre">${esc(output)}</pre></details>` : ''}
      <details><summary>All attributes (${Object.keys(node.attrs || {}).length})</summary>${table(attrRows(node))}</details>
      <details><summary>Overview table</summary>${table(overview(node))}</details></section>`;
  },
};

// ---------- variants ----------
const host = document.querySelector('#variant-root');
let fit = () => {}, reveal = () => {}, unzoom = () => {};
// A and E: centre a row in what's left visible between the sticky axis
// header and, on a phone, the Peek's bottom sheet.
function showRow(id) {
  const row = host.querySelector(`.ta-row[data-id="${CSS.escape(id)}"]`), axis = host.querySelector('.ta-axis');
  if (!row || !axis) return;
  const top = axis.offsetHeight, sheet = !peek.hidden && getComputedStyle(peek).position === 'fixed';
  const bottom = sheet ? peek.getBoundingClientRect().top : innerHeight, box = row.getBoundingClientRect();
  scrollBy(0, box.top - (top + Math.max(0, (bottom - top - box.height) / 2)));
}

// A: one indented tree with a timeline column on a shared axis (MLflow's trace view).
const A = { expanded: new Set(), axis: null };
function variantA(slow = false) {
  const r = root();
  if (!A.seeded) { A.seeded = true; (function seed(n) { if (['command', 'run', 'stage'].includes(n.kind)) { A.expanded.add(n.id); n.children.forEach(seed); } })(r); }
  const axis = A.axis || axisOf(r);
  const rows = [];
  (function walk(n) { rows.push(n); if (A.expanded.has(n.id)) n.children.forEach(walk); })(r);
  // E: rank the slowest model and tool calls inside the timeline's scope.
  const rank = new Map(), contains = new Map();
  let slowest = [];
  if (slow) {
    const fitted = A.fitNode && A.fitNode.command === command() ? A.fitNode : null;
    const scope = fitted ? (fitted.children.length ? fitted : fitted.parent) : r, leaves = [];
    // A Codex turn spans its tool calls, so it would crowd them out.
    (function walk(n) { if (n.kind === 'tool' || n.kind === 'model') leaves.push(n); n.children.forEach(walk); })(scope);
    slowest = leaves.filter(n => !fitted || (n.endOrNow > axis.start && n.start < axis.end)).sort((a, b) => length(b) - length(a)).slice(0, 10);
    slowest.forEach((n, i) => { rank.set(n, i + 1); ancestors(n).forEach(a => contains.set(a, [...(contains.get(a) || []), i + 1])); });
  }
  const where = n => { const s = ancestors(n).find(a => a.kind === 'session'), run = [...ancestors(n)].reverse().find(a => a.kind === 'run'); return [run && run.name.replace(/^.*#/, '#'), s && `${s.name} session`].filter(Boolean).join(' · '); };
  const ranks = n => rank.has(n) ? ` <span class="slow-rank">#${rank.get(n)}</span>` : !A.expanded.has(n.id) && contains.has(n) ? ` <span class="slow-hint" title="Contains slowest calls ${contains.get(n).map(x => '#' + x).join(', ')}">▸ ${contains.get(n).slice(0, 3).map(x => '#' + x).join(' ')}${contains.get(n).length > 3 ? ' …' : ''}</span>` : '';
  host.innerHTML = `<div class="toolbar"><span>Expand to</span>${['run', 'stage', 'session', 'every call'].map(l => `<button type="button" data-level="${l}">${l === 'run' ? 'Runs' : l === 'stage' ? 'stages' : l === 'session' ? 'sessions' : l}</button>`).join('')}
      <span class="dim">·</span><span>Timeline: ${A.axis ? `<b>${esc(A.fitName)}</b> <button type="button" data-unfit>Whole Command</button>` : '<b>whole Command</b>'}</span></div>${legend()}
    ${slow ? `<details class="slow-list" open><summary>Slowest calls ${A.axis ? `in ${esc(A.fitName)}` : 'in this Command'}</summary><ol>${slowest.map(n => `<li data-slow="${esc(n.id)}" class="${n === selected ? 'is-selected' : ''}"><span class="slow-rank">#${rank.get(n)}</span><span class="slow-name">${tag(n)} <span class="name" title="${esc(n.name)}">${esc(n.name)}</span></span><span class="slow-when">${clock(n.start)} → ${n.end ? clock(n.end) : '<span class="st-running">now</span>'}</span><b class="slow-dur">${dur(length(n))}${n.estimated ? '*' : ''}</b><span class="slow-where">${Math.round(length(n) / Math.max(length(n.parent), 1) * 100)}% of its ${KIND[n.parent.kind].toLowerCase()} · ${esc(where(n))}</span></li>`).join('')}</ol></details>` : ''}
    <div class="ta-axis">${A.axis ? `<div class="ta-zoom">Timeline zoomed ${A.fitNode?.children.length ? 'to ' : ''}<b>${esc(A.fitName)}</b>: ${clock(axis.start)} → ${clock(axis.end)} (${dur(axis.end - axis.start)}) <button type="button" data-unfit>Zoom out to whole Command</button></div>` : ''}<span>Span</span><span class="ta-dur">Time</span><div class="ta-ticks">${ticks(axis, innerWidth < 760 ? 3 : 5)}</div></div>
    <ul class="ta-tree" role="tree" tabindex="0" aria-label="Span tree">${rows.map(n => `<li class="ta-row${A.axis && (n.endOrNow <= axis.start || n.start >= axis.end) ? ' outside' : ''}" role="treeitem" data-id="${esc(n.id)}" aria-level="${n.depth + 1}" ${n.children.length ? `aria-expanded="${A.expanded.has(n.id)}"` : ''}>
      <div class="ta-label" style="padding-left:${Math.min(n.depth, 12) * (innerWidth < 760 ? 8 : 14)}px">${n.children.length ? `<button type="button" class="ta-twist" data-twist tabindex="-1">${A.expanded.has(n.id) ? '▾' : '▸'}</button>` : '<span class="ta-twist"></span>'}${tag(n)} <span class="name" title="${esc(n.name)}">${esc(n.name)}</span> ${n.status === 'error' ? `<span class="st-error" title="${esc(n.statusMessage || 'error')}">✕</span>` : n.status === 'running' ? '<span class="st-running" title="running">●</span>' : ''}${ranks(n)}${n.children.length && !A.expanded.has(n.id) && n.rollup.errors && n.status !== 'error' ? ` <span class="st-error rollup">${n.rollup.errors} err</span>` : ''}</div>
      <span class="rollup ta-dur">${dur(length(n))}${n.estimated ? '*' : ''}</span>${bar(n, axis, rank.has(n) ? 'slow' : '')}</li>`).join('')}</ul>`;
  host.querySelectorAll('[data-slow]').forEach(li => li.addEventListener('click', () => {
    const n = byId.get(li.dataset.slow); reveal(n); select(n); showRow(n.id);
  }));
  host.querySelectorAll('[data-level]').forEach(b => b.addEventListener('click', () => {
    const order = ['command', 'run', 'stage', 'session'];
    const upTo = b.dataset.level === 'every call' ? Infinity : order.indexOf(b.dataset.level);
    A.expanded.clear();
    (function walk(n) { const i = order.indexOf(n.kind); if (n.children.length && (upTo === Infinity || (i >= 0 && i < upTo) || (i < 0 && upTo === Infinity))) A.expanded.add(n.id); n.children.forEach(walk); })(r);
    if (upTo !== Infinity) (function walk(n) { if (n.kind === 'run' || n.kind === 'command') A.expanded.add(n.id); n.children.forEach(walk); })(r);
    render();
  }));
  // Rows scrolled into view stop below the sticky axis and zoom banner.
  host.style.setProperty('--ta-axis-h', `${host.querySelector('.ta-axis').offsetHeight + 8}px`);
  unzoom = () => { A.axis = null; A.fitNode = null; render(); };
  host.querySelectorAll('[data-unfit]').forEach(b => b.addEventListener('click', () => { unzoom(); if (selected && !peek.hidden) openPeek(selected); }));
  const tree = host.querySelector('.ta-tree');
  tree.addEventListener('click', e => {
    const row = e.target.closest('.ta-row'); if (!row) return;
    const n = byId.get(row.dataset.id);
    if (e.target.closest('[data-twist]')) { toggle(n); return; }
    select(n);
  });
  tree.addEventListener('keydown', e => {
    const list = rows, i = list.indexOf(selected);
    const go = n => { if (n) { select(n, { open: !peek.hidden }); host.querySelector(`[data-id="${CSS.escape(n.id)}"]`)?.scrollIntoView({ block: 'nearest' }); } };
    if (e.key === 'ArrowDown' || e.key === 'j') { e.preventDefault(); go(list[i + 1] || list[0]); }
    else if (e.key === 'ArrowUp' || e.key === 'k') { e.preventDefault(); go(list[i - 1] || list[0]); }
    else if (e.key === 'ArrowRight' && selected) { e.preventDefault(); if (selected.children.length && !A.expanded.has(selected.id)) toggle(selected); else go(selected.children[0]); }
    else if (e.key === 'ArrowLeft' && selected) { e.preventDefault(); if (A.expanded.has(selected.id)) toggle(selected); else go(selected.parent); }
    else if ((e.key === 'Enter' || e.key === ' ') && selected) { e.preventDefault(); openPeek(selected); }
  });
  function toggle(n) { A.expanded.has(n.id) ? A.expanded.delete(n.id) : A.expanded.add(n.id); render(); tree.focus?.(); }
  fit = n => {
    if (n.children.length) { A.axis = axisOf(n); A.fitName = `${KIND[n.kind].toLowerCase()} ${n.name.slice(0, 40)}`; }
    else {
      // A call alone would fill the axis; show it with what ran just before and after it.
      const pad = Math.max(length(n), 1000) * 1.5, p = n.parent;
      A.axis = { start: Math.max(p.start, n.start - pad), end: Math.min(p.endOrNow, n.endOrNow + pad) };
      A.fitName = `around ${KIND[n.kind].toLowerCase()} ${n.name.slice(0, 40)}`;
    }
    A.fitNode = n;
    ancestors(n).forEach(a => A.expanded.add(a.id)); A.expanded.add(n.id); render();
    showRow(n.id);
  };
  reveal = n => { ancestors(n).forEach(a => A.expanded.add(a.id)); render(); };
}

// B: one level at a time, a breadcrumb back up, and the axis rescaled to the focus.
const B = { focus: null, sort: 'time', show: 'all', limit: 150 };
function variantB() {
  const focus = B.focus && B.focus.command === command() ? B.focus : root();
  const axis = axisOf(focus);
  let kids = focus.children.slice();
  if (B.show === 'model') kids = kids.filter(n => n.kind === 'model' || n.kind === 'turn');
  if (B.show === 'tool') kids = kids.filter(n => n.kind === 'tool' || n.kind === 'agent');
  if (B.show === 'errors') kids = kids.filter(n => n.rollup.errors);
  if (B.sort === 'slowest') kids.sort((a, b) => length(b) - length(a));
  const shown = kids.slice(0, B.limit);
  const crumbs = [...ancestors(focus), focus];
  host.innerHTML = `<nav class="crumbs" aria-label="Span path">${crumbs.map((n, i) => `${i ? '<span>›</span>' : ''}<button type="button" data-focus="${esc(n.id)}" ${n === focus ? 'aria-current="true"' : ''}>${esc(TAG[n.kind])} ${esc(n.name.length > 26 ? n.name.slice(0, 26) + '…' : n.name)}</button>`).join('')}</nav>
    <div class="tb-focus" data-id="${esc(focus.id)}"><h3>${tag(focus)} ${esc(focus.name)} ${statusText(focus)}</h3><p class="rollup" style="white-space:normal">${clock(focus.start)} → ${focus.end ? clock(focus.end) : 'now'} · ${rollupText(focus)}</p>
      ${focus.parent ? `<p class="dim" style="font-size:12px;margin:6px 0 2px">Within its ${KIND[focus.parent.kind].toLowerCase()}</p>${bar(focus, axisOf(focus.parent))}` : ''}<p style="margin:6px 0 0"><button type="button" data-details>Details</button></p></div>
    <div class="toolbar"><span>Sort</span><button type="button" data-sort="time" aria-pressed="${B.sort === 'time'}">by start</button><button type="button" data-sort="slowest" aria-pressed="${B.sort === 'slowest'}">slowest first</button>
      <span>Show</span>${[['all', 'all'], ['model', 'model calls'], ['tool', 'tool calls'], ['errors', 'errors']].map(([k, l]) => `<button type="button" data-show="${k}" aria-pressed="${B.show === k}">${l}</button>`).join('')}</div>
    <ul class="tb-list">${shown.map(n => `<li class="tb-item" data-id="${esc(n.id)}">${tag(n)}<span class="name" title="${esc(n.name)}">${esc(n.name)} ${statusText(n)}</span><span class="rollup">${rollupText(n, { short: true })}${n.children.length ? ' <span class="go">›</span>' : ''}</span>${bar(n, axis)}</li>`).join('') || '<li class="dim">Nothing here.</li>'}</ul>
    ${kids.length > shown.length ? `<button type="button" class="tb-more" data-more>Show ${kids.length - shown.length} more</button>` : ''}`;
  host.querySelectorAll('[data-focus]').forEach(b => b.addEventListener('click', () => { B.focus = byId.get(b.dataset.focus); B.limit = 150; render(); }));
  host.querySelectorAll('[data-sort]').forEach(b => b.addEventListener('click', () => { B.sort = b.dataset.sort; render(); }));
  host.querySelectorAll('[data-show]').forEach(b => b.addEventListener('click', () => { B.show = b.dataset.show; render(); }));
  host.querySelector('[data-more]')?.addEventListener('click', () => { B.limit += 500; render(); });
  host.querySelector('[data-details]').addEventListener('click', () => select(focus));
  host.querySelectorAll('.tb-item').forEach(li => li.addEventListener('click', () => {
    const n = byId.get(li.dataset.id);
    if (n.children.length) { B.focus = n; B.limit = 150; render(); select(n, { open: !peek.hidden }); } else select(n);
  }));
  fit = n => { B.focus = n.children.length ? n : n.parent; render(); };
  reveal = n => { B.focus = n.children.length ? n : n.parent; render(); };
}

// C: the whole Command as a flame chart; zoom into any block.
const C = { zoom: null };
function variantC() {
  const zoom = C.zoom && C.zoom.command === command() ? C.zoom : root();
  const axis = axisOf(zoom), span = axis.end - axis.start;
  const width = Math.min(host.clientWidth || innerWidth - 24, 2000);
  let blocks = '', hidden = 0, maxDepth = 0;
  const all = [];
  (function walk(n, d) {
    if (n.endOrNow < axis.start || n.start > axis.end) return;
    const left = (Math.max(n.start, axis.start) - axis.start) / span * 100;
    const w = (Math.min(n.endOrNow, axis.end) - Math.max(n.start, axis.start)) / span * 100;
    if (w / 100 * width < 1.5) { hidden += 1; return; }
    maxDepth = Math.max(maxDepth, d);
    all.push(n);
    const cls = ['tc-block', `k2-${n.kind}`, n.status === 'error' && 'error', n.status === 'running' && 'running', n.estimated && 'est'].filter(Boolean).join(' ');
    blocks += `<div class="${cls}" data-id="${esc(n.id)}" style="left:${left}%;width:${w}%;top:${d * 21}px" title="${esc(`${KIND[n.kind]}: ${n.name} · ${dur(length(n))}`)}">${esc(n.name)}</div>`;
    n.children.forEach(c => walk(c, d + 1));
  })(zoom, 0);
  const slowest = all.filter(n => n !== zoom && !n.children.length).sort((a, b) => length(b) - length(a)).slice(0, 10);
  host.innerHTML = `<nav class="crumbs" aria-label="Zoom">${[...ancestors(zoom), zoom].map((n, i) => `${i ? '<span>›</span>' : ''}<button type="button" data-zoom="${esc(n.id)}">${esc(TAG[n.kind])} ${esc(n.name.length > 22 ? n.name.slice(0, 22) + '…' : n.name)}</button>`).join('')}</nav>
    <div class="toolbar"><button type="button" data-zoom-selected ${selected ? '' : 'disabled'}>Zoom into selected</button><span class="dim">Double-click a block to zoom. ${hidden ? `${hidden} spans too narrow to draw at this zoom.` : ''}</span></div>${legend()}
    <div class="tc-axis">${ticks(axis, innerWidth < 760 ? 3 : 6)}</div>
    <div class="tc-chart" style="height:${(maxDepth + 1) * 21 + 2}px">${blocks}</div>
    <section class="tc-top"><h3>Slowest calls in view</h3><ol>${slowest.map(n => `<li data-id="${esc(n.id)}">${tag(n)} ${esc(n.name.slice(0, 90))} <span class="rollup">${dur(length(n))}</span></li>`).join('')}</ol></section>`;
  const chart = host.querySelector('.tc-chart');
  chart.addEventListener('click', e => { const b = e.target.closest('.tc-block'); if (b) select(byId.get(b.dataset.id)); });
  chart.addEventListener('dblclick', e => { const b = e.target.closest('.tc-block'); if (b) { C.zoom = byId.get(b.dataset.id); render(); } });
  host.querySelectorAll('.tc-top li').forEach(li => li.addEventListener('click', () => select(byId.get(li.dataset.id))));
  host.querySelectorAll('[data-zoom]').forEach(b => b.addEventListener('click', () => { C.zoom = byId.get(b.dataset.zoom); render(); }));
  host.querySelector('[data-zoom-selected]').addEventListener('click', () => { if (selected) { C.zoom = selected.children.length ? selected : selected.parent; render(); } });
  fit = n => { C.zoom = n.children.length ? n : n.parent; render(); };
  reveal = n => { if (!(ancestors(n).includes(zoom) || n === zoom)) { C.zoom = n.parent || n; render(); } };
}

// D: today's Waterfall markup, with sessions and their calls folded inside each stage.
const D = { open: new Set() };
function variantD() {
  const r = root(), axis = axisOf(r);
  const runs = [];
  (function walk(n, d) { if (n.kind === 'command' || n.kind === 'run') runs.push([n, d]); n.children.forEach(c => walk(c, n.kind === 'run' || n.kind === 'command' ? d + 1 : d)); })(r, 0);
  const svg = (n, h = 20) => { const s = Math.max(axis.end - axis.start, 1); const l = (n.start - axis.start) / s * 100, w = Math.max((n.endOrNow - n.start) / s * 100, .2); return `<svg class="timeline" width="100%" height="${h}" aria-hidden="true"><svg x="${l}%" width="${w}%" height="${h}"><rect width="100%" height="${h}"/></svg></svg>`; };
  const calls = (n, sAxis) => `<ul class="td-calls">${n.children.map(c => c.kind === 'agent' || (c.children.length && c.kind === 'tool')
    ? `<li><details class="td-agent" ${D.open.has(c.id) ? 'open' : ''} data-fold="${esc(c.id)}"><summary>${tag(c)} ${esc(c.name.slice(0, 80))} <span class="rollup">${rollupText(c, { short: true })}</span> <a href="#" class="td-peek" data-id="${esc(c.id)}">Details</a></summary>${calls(c, sAxis)}</details></li>`
    : `<li class="td-call" data-id="${esc(c.id)}"><span class="name">${tag(c)} ${esc(c.name)} ${statusText(c)}</span><span class="rollup">${dur(length(c))}</span>${bar(c, sAxis)}</li>`).join('')}</ul>`;
  host.innerHTML = `<p>Shared time axis: ${clock(axis.start)} to ${clock(axis.end)}. Each stage folds out its sessions; each session folds out its model and tool calls on the session's own axis.</p>${legend()}` + runs.map(([run, d]) => `
    <section class="waterfall-run" data-depth="${Math.min(d, 2)}">
      <div class="command" data-id="${esc(run.id)}"><h3><a href="#" class="td-peek" data-id="${esc(run.id)}">${esc(run.name)}</a> · ${run.status === 'running' ? 'live' : esc(run.attrs.outcome || run.status)}</h3><p>${esc(run.title || '')}</p></div>
      ${svg(run)}<p>${clock(run.start)} · ${dur(length(run))}</p>
      <ol class="waterfall-stages">${run.children.filter(s => s.kind === 'stage').map(s => `
        <li class="waterfall-stage${s.status === 'running' ? ' running-stage' : ''}"><h4 data-id="${esc(s.id)}">${esc(s.name)}</h4><p>${clock(s.start)} · ${dur(length(s))}</p>${svg(s)}
          ${s.children.filter(x => x.kind === 'session').map(x => `<details class="td-session" ${D.open.has(x.id) ? 'open' : ''} data-fold="${esc(x.id)}"><summary>${tag(x)} ${esc(x.name)} session ${statusText(x)} <span class="rollup">${rollupText(x)}</span> <a href="#" class="td-peek" data-id="${esc(x.id)}">Details</a></summary>${D.open.has(x.id) ? calls(x, axisOf(x)) : ''}</details>`).join('')}
          ${s.children.filter(x => x.kind === 'run').map(x => `<p class="dim" style="margin-left:24px">↳ child Run ${esc(x.name)} below</p>`).join('')}
        </li>`).join('')}</ol>
    </section>`).join('');
  host.querySelectorAll('details[data-fold]').forEach(d => d.addEventListener('toggle', () => {
    const was = D.open.has(d.dataset.fold);
    if (d.open && !was) { D.open.add(d.dataset.fold); const y = scrollY; render(); scrollTo(0, y); }
    else if (!d.open && was) D.open.delete(d.dataset.fold);
  }));
  if (!D.listening) {
    D.listening = true;
    host.addEventListener('click', e => {
      if (state.variant !== 'D') return;
      const el = e.target.closest('.td-peek, .td-call, .command, h4[data-id]'); if (!el) return;
      e.preventDefault(); select(byId.get(el.dataset.id));
    });
  }
  fit = () => {};
  reveal = n => { ancestors(n).forEach(a => D.open.add(a.id)); render(); };
}

// ---------- shell ----------
function chipName(c) {
  let run = null;
  (function walk(n) { if (!run && n.kind === 'run') run = n; n.children.forEach(walk); })(c.tree);
  return run ? `${run.name.replace(/ · .*#/, ' #')}` : c.subject;
}
const RENDER = { A: () => variantA(), B: variantB, C: variantC, D: variantD, E: () => variantA(true) };
function render() {
  document.querySelector('#chips').innerHTML = F.commands.map((c, i) => `<a href="#" data-command="${i}" ${i === state.command ? 'aria-current="true"' : ''}>${esc(chipName(c))} · ${esc(c.tree.status === 'running' ? 'live' : c.tree.attrs.outcome || c.tree.status)}</a>`).join('');
  document.querySelector('#page-cli').textContent = `thirdshift show <command> --json · thirdshift logs <command> --spans --json (proposed)`;
  RENDER[state.variant]();
  if (selected && selected.command === command()) select(selected, { open: false });
  labels();
}
function labels() {
  const narrow = innerWidth < 600;
  document.querySelector('#proto-variant').textContent = narrow ? state.variant : `${state.variant} · ${VARIANTS[state.variant]}`;
  document.querySelector('#proto-details').textContent = narrow ? `Peek ${state.details}` : `Peek ${state.details} · ${DETAILS[state.details]}`;
  document.querySelector('#proto-phone').setAttribute('aria-pressed', state.phone);
}
document.querySelector('#chips').addEventListener('click', e => {
  const a = e.target.closest('[data-command]'); if (!a) return;
  e.preventDefault(); state.command = +a.dataset.command; selected = null; closePeek(); A.seeded = false; A.expanded.clear(); A.axis = null; A.fitNode = null; B.focus = null; C.zoom = null; sync();
});
function url() {
  const p = new URLSearchParams({ variant: state.variant, details: state.details, command: state.command });
  if (state.phone) p.set('frame', '390');
  return `${location.pathname}?${p}`;
}
function sync() {
  history.replaceState(null, '', url());
  if (state.phone && !state.embed) {
    document.body.classList.add('proto-phone-host');
    let frame = document.querySelector('#proto-frame');
    if (!frame) { frame = document.createElement('iframe'); frame.id = 'proto-frame'; frame.title = '390 px phone preview'; document.body.prepend(frame); }
    const p = new URLSearchParams({ variant: state.variant, details: state.details, command: state.command, embed: '1' });
    frame.src = `${location.pathname}?${p}`;
    labels();
    return;
  }
  document.body.classList.remove('proto-phone-host');
  document.querySelector('#proto-frame')?.remove();
  render();
  if (selected && !peek.hidden) openPeek(selected);
}
function step(kind, dir) {
  const keys = Object.keys(kind === 'variant' ? VARIANTS : DETAILS);
  state[kind] = keys[(keys.indexOf(state[kind]) + dir + keys.length) % keys.length];
  sync();
}
document.querySelectorAll('#proto-bar [data-step]').forEach(b => b.addEventListener('click', () => step(b.dataset.step, +b.dataset.dir)));
document.querySelector('#proto-phone').addEventListener('click', () => { state.phone = !state.phone; sync(); });
document.addEventListener('keydown', e => {
  if (e.target.closest('input, textarea, select, [contenteditable], [role="tree"]') || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === 'Escape') { closePeek(); return; }
  if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); step(e.shiftKey ? 'details' : 'variant', e.key === 'ArrowLeft' ? -1 : 1); }
});
if (state.embed) document.body.classList.add('proto-embed');
let resized; addEventListener('resize', () => { clearTimeout(resized); resized = setTimeout(() => { if (!state.phone || state.embed) render(); }, 150); });
sync();
