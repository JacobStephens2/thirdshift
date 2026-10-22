// The reader shared with `show --spans` owns trace interpretation and ancestry.
function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
function duration(ms) {
  if (ms === null || !Number.isFinite(ms)) return 'Time unknown';
  if (ms < 1000) return `${Math.round(ms * 1000) / 1000} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${Math.round(seconds * 100) / 100} s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`;
}
function clock(ms) { return ms === null ? 'unknown' : new Date(ms).toLocaleTimeString([], { hour12: false }); }
function pretty(text) {
  if (text === null || text === undefined) return 'No text retained.';
  try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return text; }
}
const quote = text => `'${text.replaceAll("'", "'\\''")}'`;
const kindName = { cmd: 'Command', run: 'Run', stage: 'stage', sess: 'session', model: 'model call', tool: 'tool call', turn: 'turn', agent: 'sub-agent' };

export function initSpanTree({ openCommand, openDetails }) {
  const tree = document.getElementById('span-tree');
  if (!tree) return null;
  let data = JSON.parse(document.getElementById('span-data').textContent);
  let nodes, children, roots, axis, ranks, descendants;
  const expanded = new Map();
  let selected = data.spans[0]?.id, detail = null, zoom = null;
  const content = document.getElementById('peek-content');
  const peek = document.getElementById('peek');
  const header = document.getElementById('span-axis');
  const slow = document.querySelector('#slowest-calls ol');
  const status = document.getElementById('span-status');
  const end = span => span.status === 'running' ? Date.now() : span.end;
  const elapsed = span => span.status === 'running' && span.start !== null ? Math.max(0, Date.now() - span.start) : span.duration_ms;
  const durationText = span => `${duration(elapsed(span))}${span.estimated ? '*' : ''}`;
  const rowFor = id => document.getElementById(`span-${id}`);
  const isOpen = span => expanded.get(span.id) ?? ['cmd', 'run', 'stage'].includes(span.kind);
  const inScope = span => span.start !== null && end(span) !== null && end(span) >= axis.start && span.start <= axis.end;
  const button = (text, action) => {
    const node = el('button', '', text);
    node.type = 'button';
    node.addEventListener('click', action);
    return node;
  };
  function ancestors(span) {
    const path = [], seen = new Set([span.id]);
    while (nodes.has(span.parent_id) && !seen.has(span.parent_id)) {
      span = nodes.get(span.parent_id);
      seen.add(span.id);
      path.unshift(span);
    }
    return path;
  }
  function index() {
    nodes = new Map(data.spans.map(span => [span.id, span]));
    children = new Map(); roots = [];
    for (const span of data.spans) {
      if (nodes.has(span.parent_id)) {
        if (!children.has(span.parent_id)) children.set(span.parent_id, []);
        children.get(span.parent_id).push(span);
      } else roots.push(span);
    }
    for (const list of children.values()) list.sort((a, b) => (a.start ?? Infinity) - (b.start ?? Infinity));
    if (!nodes.has(selected)) selected = roots[0]?.id;
    if (!nodes.has(detail)) detail = null;
    if (!nodes.has(zoom)) zoom = null;
  }
  function wholeAxis() {
    const starts = data.spans.map(span => span.start).filter(time => time !== null);
    const ends = data.spans.map(span => end(span) ?? span.start).filter(time => time !== null);
    const start = starts.length ? Math.min(...starts) : Date.now();
    return { start, end: Math.max(start + 1, ...ends) };
  }
  function setAxis() {
    axis = wholeAxis();
    const span = nodes.get(zoom);
    if (!span || span.start === null) return;
    const finish = end(span) ?? span.start;
    if (children.has(span.id)) axis = { start: span.start, end: Math.max(finish, span.start + 1) };
    else {
      const parent = nodes.get(span.parent_id), pad = Math.max(elapsed(span) ?? 0, 1000) * 1.5;
      axis = { start: Math.max(parent?.start ?? axis.start, span.start - pad), end: Math.min(parent ? end(parent) ?? Date.now() : axis.end, finish + pad) };
      if (axis.end <= axis.start) axis = { start: span.start - pad, end: finish + pad };
    }
  }
  function rankCalls() {
    const calls = data.spans.filter(span => ['tool', 'model'].includes(span.kind) && elapsed(span) !== null && inScope(span))
      .sort((a, b) => elapsed(b) - elapsed(a) || a.id.localeCompare(b.id)).slice(0, 10);
    ranks = new Map(calls.map((span, i) => [span.id, i + 1]));
    descendants = new Map(data.spans.map(span => [span.id, { errors: 0, ranks: [] }]));
    for (const span of data.spans) for (const parent of ancestors(span)) {
      const summary = descendants.get(parent.id);
      if (span.error !== null) summary.errors++;
      if (ranks.has(span.id)) summary.ranks.push(ranks.get(span.id));
    }
    const fragment = document.createDocumentFragment();
    for (const span of calls) {
      const li = el('li'), reveal = button('', () => revealSpan(span.id));
      reveal.dataset.revealSpan = span.id;
      reveal.append(el('span', 'span-rank', `#${ranks.get(span.id)}`), el('span', 'slow-name', `${span.kind} · ${span.name}`), el('span', 'slow-clock', `${clock(span.start)} → ${span.status === 'running' ? 'now' : clock(span.end)}`), el('span', 'slow-duration', durationText(span)));
      const parent = nodes.get(span.parent_id);
      const fraction = parent && elapsed(parent) > 0 ? `${Math.round(elapsed(span) / elapsed(parent) * 100)}% of its ${kindName[parent.kind]}` : 'Parent duration unknown';
      reveal.append(el('span', 'slow-where', `${fraction}${span.run ? ` · #${span.run}` : ''}${span.session_kind ? ` · ${span.session_kind} session` : ''}`));
      li.append(reveal); fragment.append(li);
    }
    if (!calls.length) fragment.append(el('li', '', 'No timed tool or model calls in this window.'));
    slow.replaceChildren(fragment);
  }
  function bar(span, range = axis) {
    const track = el('span', 'span-track');
    const fill = el('i', `span-bar kind-${span.kind}${span.error !== null ? ' span-error' : ''}${span.status === 'running' ? ' span-running' : ''}${span.estimated ? ' span-estimate' : ''}${ranks.has(span.id) ? ' span-slow' : ''}`);
    if (span.start !== null && end(span) !== null) {
      const scale = 100 / Math.max(1, range.end - range.start);
      const left = Math.min(100, Math.max(0, (span.start - range.start) * scale));
      const width = Math.min(100 - left, Math.max(0, (Math.min(end(span), range.end) - Math.max(span.start, range.start)) * scale));
      fill.style.left = `${left}%`; fill.style.width = `${width}%`;
    } else fill.style.width = '0';
    track.append(fill); return track;
  }
  function renderTree() {
    const focused = tree.contains(document.activeElement) ? document.activeElement.closest('[data-span-id]')?.dataset.spanId : null;
    function item(span, depth, position, count, path) {
      if (path.has(span.id)) return document.createDocumentFragment();
      const nextPath = new Set(path).add(span.id), nested = children.get(span.id) ?? [], open = isOpen(span);
      const wrapper = el('div', 'span-item');
      wrapper.id = `span-${span.id}`; wrapper.dataset.spanId = span.id; wrapper.dataset.kind = span.kind;
      wrapper.setAttribute('role', 'treeitem'); wrapper.setAttribute('aria-level', depth + 1);
      wrapper.setAttribute('aria-labelledby', `span-label-${span.id}`);
      wrapper.setAttribute('aria-posinset', position + 1); wrapper.setAttribute('aria-setsize', count);
      wrapper.setAttribute('aria-selected', span.id === selected); wrapper.tabIndex = span.id === selected ? 0 : -1;
      if (nested.length) wrapper.setAttribute('aria-expanded', open);
      const row = el('div', `span-row${span.id === selected ? ' is-selected' : ''}${zoom && !inScope(span) ? ' outside-window' : ''}`);
      const label = el('span', 'span-label'); label.id = `span-label-${span.id}`; label.style.setProperty('--span-depth', depth);
      const twist = el('span', 'span-twist', nested.length ? open ? '▾' : '▸' : '·');
      twist.setAttribute('aria-hidden', 'true'); twist.dataset.discloseSpan = span.id;
      label.append(twist, el('span', 'span-kind', span.kind));
      if (ranks.has(span.id)) label.append(el('span', 'span-rank', `#${ranks.get(span.id)}`));
      const name = el('span', 'span-name', span.name); name.title = span.name; label.append(name);
      if (span.error !== null || span.status === 'running') {
        const mark = el('span', span.error !== null ? 'span-status-error' : 'span-status-running', span.error !== null ? '✕' : '●');
        mark.title = span.error ?? 'running'; mark.setAttribute('aria-label', span.error ?? 'running'); label.append(mark);
      }
      if (!open && nested.length) {
        const summary = descendants.get(span.id);
        if (summary.errors) label.append(el('span', 'span-fold-hint', `${summary.errors} error${summary.errors === 1 ? '' : 's'}`));
        if (summary.ranks.length) label.append(el('span', 'span-fold-hint', summary.ranks.sort((a, b) => a - b).map(rank => `#${rank}`).join(' ')));
      }
      row.append(label, el('span', 'span-duration', durationText(span)), bar(span)); wrapper.append(row);
      if (open && nested.length) {
        const group = el('div', 'span-children'); group.setAttribute('role', 'group');
        nested.forEach((child, i) => group.append(item(child, depth + 1, i, nested.length, nextPath))); wrapper.append(group);
      }
      return wrapper;
    }
    tree.replaceChildren(...roots.map((span, i) => item(span, 0, i, roots.length, new Set())));
    if (focused) rowFor(focused)?.focus({ preventScroll: true });
  }
  function renderAxis() {
    document.getElementById('span-ticks').replaceChildren(...[0, 0.5, 1].map(fraction => {
      const tick = el('span', '', `+${duration((axis.end - axis.start) * fraction)}`); tick.style.left = `${fraction * 100}%`; return tick;
    }));
    const notice = document.getElementById('span-zoom'); notice.hidden = !zoom; notice.replaceChildren();
    if (zoom) notice.append(el('b', '', `Timeline zoomed around ${nodes.get(zoom).name}: ${clock(axis.start)} → ${clock(axis.end)} (${duration(axis.end - axis.start)})`), button('Zoom out to whole Command', () => zoomTo(null)));
  }
  function render() { setAxis(); rankCalls(); renderAxis(); renderTree(); if (detail) renderDetails(); }
  function centerRow(id) {
    requestAnimationFrame(() => {
      const row = rowFor(id)?.querySelector(':scope > .span-row'); if (!row) return;
      const top = Math.max(0, header.getBoundingClientRect().bottom);
      const bottom = innerWidth <= 760 && !peek.hidden ? peek.getBoundingClientRect().top : innerHeight;
      const rect = row.getBoundingClientRect();
      window.scrollBy({ top: rect.top + rect.height / 2 - (top + bottom) / 2, behavior: 'instant' });
    });
  }
  function choose(id, focus = true) { selected = id; renderTree(); if (focus) rowFor(id)?.focus({ preventScroll: true }); }
  function revealSpan(id) {
    const span = nodes.get(id); if (!span) return;
    ancestors(span).forEach(parent => expanded.set(parent.id, true)); choose(id); detail = id; renderDetails(); centerRow(id);
  }
  function zoomTo(id) { zoom = id; render(); centerRow(selected); }
  function renderDetails() {
    const span = nodes.get(detail); if (!span) return;
    openDetails();
    const folds = new Map([...content.querySelectorAll('details')].map(node => [node.dataset.detailSection, node.open]));
    const sameSpan = content.dataset.detailSpanId === span.id, scroll = sameSpan ? peek.scrollTop : 0;
    content.dataset.detailSpanId = span.id;
    const card = el('article', 'span-card'), path = el('nav', 'span-path'); path.setAttribute('aria-label', 'Span ancestors');
    for (const parent of ancestors(span)) {
      const link = el('a', '', parent.name); link.href = `#span-${parent.id}`;
      link.addEventListener('click', event => { event.preventDefault(); revealSpan(parent.id); }); path.append(link);
    }
    card.append(path, el('h2', '', `${span.kind} · ${span.name}`), el('p', '', `${span.status}${span.error !== null ? ` · ${span.error}` : ''} · ${clock(span.start)} → ${span.status === 'running' ? 'now' : clock(span.end)}${span.estimated ? ' · estimated*' : ''}`));
    const tiles = el('div', 'span-tiles');
    const tile = (label, value) => {
      if (value === null || value === undefined) return;
      const node = el('div', 'span-tile'); node.append(el('b', '', String(value)), el('span', '', label)); tiles.append(node);
    };
    tile('Duration', elapsed(span) === null ? null : durationText(span));
    const parent = nodes.get(span.parent_id);
    if (parent && elapsed(parent) > 0 && elapsed(span) !== null) tile(`% of its ${kindName[parent.kind]}`, `${Math.round(elapsed(span) / elapsed(parent) * 100)}%`);
    const attrs = span.attributes;
    tile('Tokens in', attrs['gen_ai.usage.input_tokens']); tile('Tokens out', attrs['gen_ai.usage.output_tokens']); tile('Cost (USD)', attrs['thirdshift.cost.usd']);
    const nested = data.spans.filter(child => ancestors(child).some(parent => parent.id === span.id));
    for (const [kind, label] of [['model', 'Model calls'], ['tool', 'Tool calls']]) {
      const count = nested.filter(child => child.kind === kind).length; if (count) tile(label, count);
    }
    const errors = descendants.get(span.id).errors + (span.error !== null ? 1 : 0); if (errors) tile('Errors', errors);
    card.append(tiles);
    for (const [label, owner] of [['Within parent', parent], ['Within Command', roots[0]]]) {
      if (owner && owner.start !== null && end(owner) !== null) {
        const position = el('div', 'span-position', label); position.append(bar(span, { start: owner.start, end: end(owner) })); card.append(position);
      }
    }
    const actions = el('div', 'span-actions');
    const zoomButton = button(zoom ? 'Zoom out to whole Command' : children.has(span.id) ? `Zoom timeline to this ${kindName[span.kind]}` : 'Zoom timeline around this call', () => zoomTo(zoom ? null : span.id));
    zoomButton.disabled = span.start === null; actions.append(zoomButton);
    if (span.conversation_url) { const link = el('a', '', 'Open in conversation'); link.href = span.conversation_url; actions.append(link); }
    const cli = `thirdshift show ${quote(data.command)} --span ${span.id} --json`;
    const copy = button('Copy CLI', async () => {
      try { await navigator.clipboard.writeText(cli); status.textContent = 'CLI copied.'; } catch { status.textContent = cli; }
    });
    copy.dataset.copyCli = cli; actions.append(copy, button('Command details', () => { detail = null; openCommand(span); })); card.append(actions);
    function section(label, body, open = false) {
      const disclosure = el('details'); disclosure.dataset.detailSection = label;
      disclosure.open = sameSpan ? folds.get(label) ?? open : open;
      disclosure.append(el('summary', '', label), body); card.append(disclosure);
    }
    const noModelText = span.kind === 'model' || span.kind === 'turn';
    section('Input', el('pre', '', noModelText ? 'The text is in the conversation view.' : pretty(span.input)), true);
    section('Output', el('pre', '', noModelText ? 'The text is in the conversation view.' : pretty(span.output)));
    section('All attributes', el('pre', '', JSON.stringify(attrs, null, 2)));
    const table = el('table', 'span-overview'), body = el('tbody');
    const overview = { Kind: kindName[span.kind], Status: span.status, Start: span.start === null ? 'unknown' : new Date(span.start).toISOString(), End: span.status === 'running' ? 'now' : span.end === null ? 'unknown' : new Date(span.end).toISOString(), Model: attrs['gen_ai.response.model'] ?? attrs['gen_ai.request.model'], Harness: attrs['thirdshift.harness'], 'Tokens in': attrs['gen_ai.usage.input_tokens'], 'Tokens out': attrs['gen_ai.usage.output_tokens'], 'Cache read': attrs['thirdshift.usage.cache_read_tokens'], 'Cache write': attrs['thirdshift.usage.cache_write_tokens'], 'Cost (USD)': attrs['thirdshift.cost.usd'], 'Span ID': span.id };
    for (const [key, value] of Object.entries(overview)) {
      if (value === undefined || value === null) continue;
      const row = el('tr'); row.append(el('th', '', key), el('td', '', String(value))); body.append(row);
    }
    table.append(body); section('Overview table', table); content.replaceChildren(card);
    peek.hidden = false; document.body.classList.add('peek-open'); peek.scrollTop = scroll;
  }
  tree.addEventListener('click', event => {
    const row = event.target.closest('.span-row'); if (!row) return;
    const id = row.parentElement.dataset.spanId;
    if (event.target.closest('[data-disclose-span]') && children.has(id)) { expanded.set(id, !isOpen(nodes.get(id))); choose(id); }
    else revealSpan(id);
  });
  tree.addEventListener('focusin', event => {
    const id = event.target.closest('[data-span-id]')?.dataset.spanId;
    if (id && selected !== id) {
      selected = id;
      for (const node of tree.querySelectorAll('[role="treeitem"]')) {
        node.tabIndex = node.dataset.spanId === id ? 0 : -1; node.setAttribute('aria-selected', node.dataset.spanId === id);
        node.querySelector(':scope > .span-row').classList.toggle('is-selected', node.dataset.spanId === id);
      }
    }
  });
  tree.addEventListener('keydown', event => {
    const span = nodes.get(selected); if (!span || event.altKey || event.ctrlKey || event.metaKey) return;
    const visible = [...tree.querySelectorAll('[role="treeitem"]')].map(node => node.dataset.spanId), index = visible.indexOf(selected);
    let target;
    switch (event.key) {
      case 'ArrowDown': case 'j': target = visible[Math.min(index + 1, visible.length - 1)]; break;
      case 'ArrowUp': case 'k': target = visible[Math.max(0, index - 1)]; break;
      case 'Home': target = visible[0]; break;
      case 'End': target = visible.at(-1); break;
      case 'ArrowRight':
        if (children.has(selected)) { if (isOpen(span)) target = children.get(selected)[0].id; else { expanded.set(selected, true); renderTree(); } }
        break;
      case 'ArrowLeft':
        if (children.has(selected) && isOpen(span)) { expanded.set(selected, false); renderTree(); } else target = span.parent_id;
        break;
      case 'Enter': case ' ': revealSpan(selected); break;
      default: return;
    }
    event.preventDefault(); event.stopPropagation();
    if (target) { choose(target); centerRow(target); }
  });
  for (const control of document.querySelectorAll('[data-expand]')) control.addEventListener('click', () => {
    const level = control.dataset.expand;
    for (const span of nodes.values()) expanded.set(span.id, level === 'all' || span.kind === 'cmd' || (['stage', 'sess'].includes(level) && span.kind === 'run') || (level === 'sess' && span.kind === 'stage'));
    const folded = ancestors(nodes.get(selected)).find(parent => !isOpen(parent)); if (folded) selected = folded.id;
    render();
  });
  index(); render();
  const timer = setInterval(() => { if (data.spans.some(span => span.status === 'running')) render(); }, 1000);
  window.addEventListener('pagehide', () => clearInterval(timer), { once: true });
  return {
    closeDetails() { detail = null; delete content.dataset.detailSpanId; },
    update(document) {
      const next = JSON.parse(document.getElementById('span-data').textContent);
      const chips = document.querySelector('.command-chips');
      tree.closest('main').querySelector('.command-chips').replaceChildren(...chips.childNodes);
      const changed = JSON.stringify(next.spans) !== JSON.stringify(data.spans);
      data = next;
      if (changed) { index(); render(); }
    }
  };
}
