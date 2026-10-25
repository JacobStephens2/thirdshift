import { activity, loadLook, motionAllowed } from './look.js';
import { watchCommands } from './changes.js';
import { initSpanTree } from './spans.js';
import { initStages } from './stages.js';
let spanTree;

// Elapsed Pass ages keep advancing even when no Command or file changes.
function agePassStarts() {
  for (const element of document.querySelectorAll('[data-pass-fired-at]')) {
    const seconds = Math.max(0, Math.floor((Date.now() - Date.parse(element.dataset.passFiredAt)) / 1000));
    if (!Number.isFinite(seconds)) continue;
    element.textContent = seconds < 60 ? `${seconds} sec`
      : seconds < 3600 ? `${Math.floor(seconds / 60)} min`
      : `${Math.floor(seconds / 3600)} hr ${Math.floor(seconds % 3600 / 60)} min`;
  }
}
// Inbox refreshes can introduce the first observed Pass after page load.
if (document.getElementById('inbox') || document.querySelector('[data-pass-fired-at]')) {
  agePassStarts();
  setInterval(agePassStarts, 1000);
}

// Browser preferences are conveniences; malformed or unavailable storage hides nothing.
function stored(key, fallback) {
  try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; }
}
function remember(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage is optional */ }
}
const visitKey = "thirdshift:console:last-look";
const visit = stored(visitKey, Date.now());
const lastLook = typeof visit === "number" && Number.isFinite(visit) && visit >= 0 && visit <= Date.now() ? visit : Date.now();
function markNew() {
  for (const row of document.querySelectorAll("#inbox .command")) {
    row.querySelector(":scope > .row-head > .new").hidden = Number(row.dataset.startedMs) <= lastLook;
  }
}
markNew();
remember(visitKey, Date.now());
window.addEventListener("pagehide", () => remember(visitKey, Date.now()));
const foldKey = "thirdshift:console:folded";
let folded = stored(foldKey, {});
if (typeof folded !== "object" || Array.isArray(folded)) folded = {};
for (const section of document.querySelectorAll("details[data-group]")) {
  if (typeof folded[section.dataset.group] === "boolean") section.open = folded[section.dataset.group];
}
document.addEventListener("toggle", event => {
  const section = event.target;
  if (!section.matches("details[data-group]")) return;
  folded[section.dataset.group] = section.open;
  remember(foldKey, folded);
  if (section.open && selected && section.contains(selected)) selected.focus({ preventScroll: true });
}, true);
document.addEventListener("focusin", event => {
  const row = event.target.closest(".command");
  if (row) select(row, { scroll: false });
});
document.addEventListener("click", event => {
  if (!event.target.closest('a, button, input, select, textarea')) {
    event.target.closest(".command")?.focus({ preventScroll: true });
  }
});

function logLengths(row) {
  return row.dataset.logGrowth.split(":").map(Number);
}
function growth() {
  return new Map([...document.querySelectorAll(".command")].map(row => [row.id, logLengths(row)]));
}

function morph(html) {
  const next = new DOMParser().parseFromString(html, "text/html");
  const current = document.getElementById("inbox");
  const replacement = next.getElementById("inbox");
  if (!replacement) throw new Error("Missing Inbox");
  const active = document.activeElement;
  const focused = active?.id || active?.closest("details")?.id;
  const wasSummary = active?.matches("summary");
  const scroll = [...current.querySelectorAll("details")].map(section => [section.id, section.querySelector(".commands").scrollTop]);
  const position = [scrollX, scrollY];
  const selection = selected?.id || selected?.closest('details')?.id;
  const selectedSummary = selected?.matches('summary');
  const previousGrowth = growth();
  Idiomorph.morph(current, replacement, {
    callbacks: {
      beforeAttributeUpdated(name, node) {
        if (name === "open" && node.matches("details[data-group]")) return false;
      }
    }
  });
  const focusedNode = document.getElementById(focused);
  (wasSummary ? focusedNode?.querySelector("summary") : focusedNode)?.focus({ preventScroll: true });
  // Focus cannot enter a folded section. Keep the logical row selected so
  // it remains selected, and regains focus, when that section is reopened.
  const selectedNode = document.getElementById(selection);
  select(selectedSummary ? selectedNode?.querySelector('summary') : selectedNode, { scroll: false });
  for (const [id, top] of scroll) document.getElementById(id).querySelector(".commands").scrollTop = top;
  window.scrollTo(...position);
  document.title = next.title;
  document.querySelector('link[rel="icon"]').href = next.querySelector('link[rel="icon"]').getAttribute("href");
  markNew();
  activity();
  loadLook();
  for (const row of document.querySelectorAll('.command[data-logging="true"]')) {
    const previous = previousGrowth.get(row.id) ?? [0, 0];
    const increased = logLengths(row).some((size, index) => size > previous[index]);
    if (increased && row.dataset.live === 'true' && motionAllowed()) {
      row.querySelector(":scope > .row-head > .heartbeat")?.animate([
        { transform: "scale(1)", opacity: 0.4 },
        { transform: "scale(1.8)", opacity: 1, offset: 0.3 },
        { transform: "scale(1)", opacity: 0.4 }
      ], { duration: 900 });
      row.querySelector(':scope > .now')?.animate([
        { outline: '2px solid var(--working)' }, { outline: '2px solid transparent' }
      ], { duration: 900 });
    }
  }
}

const timePage = document.querySelector('main[data-time-page]');
const stages = initStages(timePage, refresh);
function morphTimePage(html) {
  const next = new DOMParser().parseFromString(html, 'text/html');
  const replacement = next.querySelector('main[data-time-page]');
  if (!replacement) throw new Error('Missing time page');
  if (stages && !stages.accepts(replacement)) return;
  const selection = selected?.dataset.command;
  const focused = timePage.contains(document.activeElement);
  const position = [scrollX, scrollY];
  const hours = timePage.querySelector('[name="hours"]')?.value;
  const reading = stages?.capture();
  Idiomorph.morph(timePage, replacement);
  // A push refreshes the rows while the person may still be editing this form.
  if (hours !== undefined) timePage.querySelector('[name="hours"]').value = hours;
  if (selection) {
    const row = [...timePage.querySelectorAll('.command')].find(row => row.dataset.command === selection);
    select(row, { scroll: false });
    if (focused && !stages) row?.focus({ preventScroll: true });
  }
  if (reading) stages.restore(reading);
  window.scrollTo(...position);
  activity();
  loadLook();
}

async function refresh() {
  if (observesCommands) {
    if (timePage?.matches('[data-span-page]')) {
      const response = await fetch(location.href, { cache: 'no-store' });
      if (!response.ok) throw new Error('Span tree unavailable');
      const html = await response.text();
      if (spanTree) spanTree.update(new DOMParser().parseFromString(html, 'text/html'));
      else {
        morphTimePage(html);
        spanTree = initializeSpanTree();
      }
      activity();
      loadLook();
    } else {
      const response = await fetch(stages?.url() ?? (timePage ? location.href : '/'), { cache: "no-store" });
      if (!response.ok) throw new Error("Inbox unavailable");
      const html = await response.text();
      if (timePage) morphTimePage(html);
      else morph(html);
    }
  }
  await refreshPeek();
}
let stopWatching;
function startUpdates() {
  stopWatching ??= watchCommands(refresh);
}
const observesCommands = document.getElementById('inbox') || timePage;
if (observesCommands) startUpdates();

function stopPeekUpdates() {
  if (observesCommands) return;
  stopWatching?.();
  stopWatching = undefined;
}

const peek = document.querySelector('#peek');
const content = document.querySelector('#peek-content');
const help = document.querySelector('#keys');
const jumps = document.querySelector('#jumps');
let selected = null;
let peekCommand = null;
let pendingPeek = null;
// Details can be replaced or reopened while a dismissal request is in flight.
const dismissals = new Map();

function showDismissal() {
  const form = content.querySelector('form[data-dismiss]');
  if (!form) return;
  const state = dismissals.get(form.elements.command.value);
  form.querySelector('button').disabled = state?.pending ?? false;
  form.querySelector('[role="status"]').textContent = state && (state.pending || state.action === form.action)
    ? state.message : '';
}

function initializeSpanTree() { return initSpanTree({
  openDetails() {
    peekCommand = null;
    pendingPeek?.abort();
    pendingPeek = null;
  },
  openCommand: span => {
    peekCommand = null;
    togglePeek({ dataset: { command: span.command, peek: span.command_url } });
  }
}); }
spanTree = initializeSpanTree();
activity();

function select(element, { scroll = true } = {}) {
  selected?.classList.remove('is-selected');
  selected?.removeAttribute('aria-current');
  selected = element;
  selected?.classList.add('is-selected');
  if (selected?.matches('.command')) selected.setAttribute('aria-current', 'true');
  if (scroll) selected?.scrollIntoView({ block: 'nearest' });
}

function closePeek() {
  spanTree?.closeDetails();
  stopPeekUpdates();
  pendingPeek?.abort();
  pendingPeek = null;
  peekCommand = null;
  peek.hidden = true;
  document.body.classList.remove('peek-open');
}

async function togglePeek(row) {
  if (!row.dataset.command) return;
  if (peekCommand === row.dataset.command) { closePeek(); return; }
  peekCommand = row.dataset.command;
  content.textContent = 'Loading details…';
  peek.hidden = false;
  document.body.classList.add('peek-open');
  await loadPeek(row.dataset.peek);
}

async function refreshPeek() {
  if (peekCommand && !pendingPeek) {
    await loadPeek(`/peek?${new URLSearchParams({ command: peekCommand })}`);
  }
}

// A live refresh replaces the pane; keep a copy's status and its by-hand selection.
function keepCopy() {
  const old = content.querySelector('.peek-cli');
  if (!old) return () => {};
  const status = old.querySelector('[role="status"]').textContent;
  const selection = getSelection();
  const selected = old.contains(selection.anchorNode) && selection.toString() === old.querySelector('code').textContent;
  return () => {
    const fresh = content.querySelector('.peek-cli');
    if (!fresh) return;
    fresh.querySelector('[role="status"]').textContent = status;
    if (selected) selection.selectAllChildren(fresh.querySelector('code'));
  };
}

async function loadPeek(url) {
  pendingPeek?.abort();
  const request = new AbortController();
  pendingPeek = request;
  try {
    const response = await fetch(url, { signal: request.signal, cache: 'no-store' });
    if (!response.ok) throw new Error('Details unavailable');
    const html = await response.text();
    if (request.signal.aborted) return;
    const restoreCopy = keepCopy();
    // Only the server's escaped show template is inserted, never a log.
    content.replaceChildren(document.createRange().createContextualFragment(html));
    restoreCopy();
    showDismissal();
    if (content.querySelector('[data-live="true"]')) startUpdates();
    else stopPeekUpdates();
  } catch (error) {
    if (!request.signal.aborted && error.name !== 'AbortError') content.textContent = 'Details unavailable. Reload to try again.';
  } finally {
    if (pendingPeek === request) pendingPeek = null;
  }
}

function items() {
  return [...document.querySelectorAll('main .group > summary, main .command, main .log-row')]
    .filter(element => {
      for (let parent = element.parentElement; parent; parent = parent.parentElement) {
        if (parent.hidden || (parent.tagName === 'DETAILS' && !parent.open && element !== parent.firstElementChild)) return false;
      }
      return true;
    });
}

const sectionLetters = { 'needs you': 'n', 'ready for review': 'r', landing: 'l', working: 'w', done: 'd', resolved: 'v' };
const targets = new Map();
for (const link of document.querySelectorAll('nav[aria-label="Pages"] a')) {
  targets.set(link.dataset.jump, { label: link.textContent.replace(/^\d+ /, ''), go: () => link.click() });
}
for (const section of document.querySelectorAll('.group')) {
  const letter = sectionLetters[section.dataset.group];
  if (letter) targets.set(letter, {
    label: section.querySelector('summary span').textContent,
    go: () => {
      const current = section.id ? document.getElementById(section.id) : section;
      current.open = true;
      select(current.querySelector('summary'));
    }
  });
}
for (const [letter, target] of targets) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = `${letter} · ${target.label}`;
  button.addEventListener('click', () => { jumps.hidden = true; target.go(); });
  jumps.append(button);
}

content.addEventListener('click', async event => {
  const copy = event.target.closest('.peek-cli [data-copy-cli]');
  if (!copy) return;
  const line = copy.closest('.peek-cli');
  const status = line.querySelector('[role="status"]');
  try {
    await navigator.clipboard.writeText(copy.dataset.copyCli);
    status.textContent = 'CLI copied.';
  } catch {
    // An insecure origin or refused permission still leaves the text ready to copy by hand.
    getSelection().selectAllChildren(line.querySelector('code'));
    status.textContent = 'Selected; copy it with your keyboard.';
  }
});
content.addEventListener('submit', async event => {
  const form = event.target.closest('form[data-dismiss]');
  if (!form) return;
  event.preventDefault();
  const command = form.elements.command.value;
  if (dismissals.get(command)?.pending) return;
  const undo = new URL(form.action).pathname === '/dismiss/undo';
  const state = {
    action: form.action, pending: true,
    message: undo ? 'Taking back dismissal…' : 'Dismissing…'
  };
  dismissals.set(command, state);
  showDismissal();
  let saved = false;
  try {
    const response = await fetch(form.action, {
      method: 'POST', body: new URLSearchParams(new FormData(form))
    });
    if (!response.ok) throw new Error(await response.text());
    saved = true;
    pendingPeek?.abort();
    pendingPeek = null;
    await refresh();
    dismissals.delete(command);
  } catch (error) {
    state.message = saved
      ? `Dismissal saved. Could not refresh details: ${error.message}`
      : `Could not save the dismissal: ${error.message}`;
  } finally {
    state.pending = false;
    showDismissal();
  }
});
document.querySelector('[data-close-peek]').addEventListener('click', closePeek);
document.querySelector('[data-close-keys]').addEventListener('click', () => help.close());
document.querySelector('[data-open-keys]').addEventListener('click', () => help.showModal());
document.addEventListener('click', event => {
  const link = event.target.closest('.peek-link');
  const row = event.target.closest('main .command, main .log-row');
  if (link) { event.preventDefault(); select(row); togglePeek(row); }
  else if (row && !event.target.closest('a, button, input, select, textarea')) select(row);
  else if (event.target.closest('main .group > summary')) {
    const summary = event.target.closest('summary');
    if (summary.parentElement.open || !selected?.matches('.command') || !summary.parentElement.contains(selected)) select(summary);
  }
});

document.addEventListener('keydown', event => {
  if (event.ctrlKey || event.metaKey || event.altKey || event.target.closest('input, select, textarea, [contenteditable="true"]')) return;
  if (event.key === 'Enter' && event.target.closest('a, button')) return;
  if (event.key === ' ' && event.target.closest('button')) return;
  if (document.querySelector('#themes[open], #sections-chooser[open]')) return;
  if (event.key === 'T' && event.shiftKey) { event.preventDefault(); document.querySelector('[data-open-themes]').click(); return; }
  if (event.key === 'Escape') {
    event.preventDefault();
    if (help.open) help.close();
    else if (!jumps.hidden) jumps.hidden = true;
    else if (!peek.hidden) closePeek();
    else select(null);
    return;
  }
  if (help.open) return;
  if (!jumps.hidden) {
    event.preventDefault();
    jumps.hidden = true;
    targets.get(event.key.toLowerCase())?.go();
    return;
  }
  const page = document.querySelector(`nav[aria-label="Pages"] a[data-key="${CSS.escape(event.key)}"]`);
  if (page) { event.preventDefault(); page.click(); return; }
  if (event.key === '?') { event.preventDefault(); help.showModal(); return; }
  if (event.key === 'g') { event.preventDefault(); jumps.hidden = false; return; }
  if (event.key === 'j' || event.key === 'k') {
    event.preventDefault();
    const visible = items();
    const index = visible.indexOf(selected);
    const next = index < 0 ? (event.key === 'j' ? 0 : visible.length - 1)
      : (index + (event.key === 'j' ? 1 : -1) + visible.length) % visible.length;
    select(visible[next] ?? null);
  } else if (event.key === ' ' && selected) {
    event.preventDefault();
    if (selected.tagName === 'SUMMARY') selected.parentElement.open = !selected.parentElement.open;
    else togglePeek(selected);
  } else if (event.key === 'z' && selected) {
    event.preventDefault();
    const section = selected.closest('.group');
    if (!section) return;
    section.open = false;
    select(section.querySelector('summary'));
  } else if (event.key === 'Enter' && selected) {
    event.preventDefault();
    const row = selected.matches('.command, .log-row') ? selected : selected.parentElement.querySelector('.command');
    if (row?.dataset.log) location.assign(row.dataset.log);
  }
});
