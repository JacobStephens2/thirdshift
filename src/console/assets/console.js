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
  const position = [scrollX, scrollY];
  const selection = selected?.id || selected?.closest('details')?.id;
  const selectedSummary = selected?.matches('summary');
  const previousGrowth = growth();
  Idiomorph.morph(current, replacement, {
    callbacks: {
      beforeNodeMorphed(node) {
        // Copy feedback belongs to this browser, not the refreshed server row.
        return !node.matches?.('[data-copy-status]');
      },
      beforeAttributeUpdated(name, node) {
        if (name === "open" && node.matches("details")) return false;
      }
    }
  });
  const focusedNode = document.getElementById(focused);
  (wasSummary ? focusedNode?.querySelector("summary") : focusedNode)?.focus({ preventScroll: true });
  // Focus cannot enter a folded section. Keep the logical row selected so
  // it remains selected, and regains focus, when that section is reopened.
  const selectedNode = document.getElementById(selection);
  select(selectedSummary ? selectedNode?.querySelector('summary') : selectedNode, { scroll: false });
  window.scrollTo(...position);
  document.title = next.title;
  document.querySelector('link[rel="icon"]').href = next.querySelector('link[rel="icon"]').getAttribute("href");
  markNew();
  showDismissal();
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
      const url = stages?.url() ?? location.href;
      const response = await fetch(url, { cache: "no-store" });
      if (!response.ok) throw new Error("Inbox unavailable");
      const html = await response.text();
      if (timePage) morphTimePage(html);
      else {
        morph(html);
        // A folder opened while this answer was on its way outranks it.
        if (url !== location.href) showFolder(urlFolder()?.dataset.group);
      }
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
// Inbox rows and details can be replaced while a dismissal request is in flight.
const dismissals = new Map();

function showDismissal() {
  for (const form of document.querySelectorAll('form[data-dismiss]')) {
    const state = dismissals.get(form.elements.command.value);
    form.querySelector('button').disabled = state?.pending ?? false;
    form.querySelector('[role="status"]').textContent = state && (state.pending || state.action === form.action)
      ? state.message : '';
  }
}

function removeDismissalNotice(command) {
  for (const form of document.querySelectorAll('#dismissal-notices form')) {
    if (form.elements.command.value === command) form.parentElement.remove();
  }
}

function dismissalNotice(form, subject) {
  const command = form.elements.command.value;
  removeDismissalNotice(command);
  const notices = document.querySelector('#dismissal-notices');
  const notice = document.createElement('div');
  notice.setAttribute('role', 'status');
  const undo = document.createElement('form');
  undo.method = 'post';
  undo.action = '/dismiss/undo';
  undo.dataset.dismiss = '';
  for (const name of ['command', 'csrf']) undo.append(form.elements[name].cloneNode(true));
  const button = document.createElement('button');
  button.textContent = 'Undo';
  const status = document.createElement('span');
  status.setAttribute('role', 'status');
  undo.append(button, status);
  notices.append(notice);
  notice.append(`Dismissed ${subject} · `, undo);
  // Keep Undo reachable while someone is using it, saving it, or retrying it.
  function expire() {
    if (!notice.isConnected) return;
    if (notice.matches(':hover, :focus-within') || dismissals.has(command)) setTimeout(expire, 10000);
    else notice.remove();
  }
  setTimeout(expire, 10000);
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

function showPeek(row) {
  if (peekCommand !== row.dataset.command) togglePeek(row);
}

// The Inbox is a mailbox: one section, a folder, shows at a time, and once
// opened, Details stays beside it for the selected row.
const mailbox = document.body.classList.contains('mailbox');
function currentFolder() {
  return document.querySelector('#inbox .folders a[aria-current="page"]');
}
// The folder this page's URL names, if it names one.
function urlFolder() {
  const section = new URLSearchParams(location.search).get('section');
  return [...document.querySelectorAll('#inbox .folders a[data-group]')]
    .find(link => new URL(link.href).searchParams.get('section') === section);
}
function showFolder(group) {
  for (const link of document.querySelectorAll('#inbox .folders a[data-group]')) {
    if (link.dataset.group === group) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  }
  for (const section of document.querySelectorAll('#inbox > .group[data-group]')) section.hidden = section.dataset.group !== group;
}
function openFolder(link, { push = true } = {}) {
  showFolder(link.dataset.group);
  if (push && link.href !== location.href) history.pushState(null, '', link.href);
  // A selected row that moved into this folder keeps its selection and focus.
  const kept = selected?.closest('#inbox > .group:not([hidden])') && selected.matches('.command') ? selected : null;
  const row = kept ?? document.querySelector('#inbox > .group:not([hidden]) .command');
  select(row ?? null, { scroll: false });
  kept?.focus({ preventScroll: true });
  // Open Details move to the folder's row; closed ones stay closed.
  if (!row) closePeek();
  else if (!peek.hidden) showPeek(row);
}
if (mailbox) {
  const folder = currentFolder();
  if (folder && !new URLSearchParams(location.search).has('section')) history.replaceState(null, '', folder.href);
  addEventListener('popstate', () => {
    const link = urlFolder();
    // Only the server knows the default folder of a URL that names none.
    if (link) openFolder(link, { push: false });
    else location.reload();
  });
}

async function refreshPeek() {
  if (peekCommand && !pendingPeek) {
    await loadPeek(`/peek?${new URLSearchParams({ command: peekCommand })}`);
  }
}

// A live refresh replaces the pane; keep each copy's status and its by-hand selection.
function keepCopy() {
  const selection = getSelection();
  const kept = [...content.querySelectorAll('.peek-cli')].map(line => ({
    status: line.querySelector('[role="status"]').textContent,
    selected: line.contains(selection.anchorNode) && selection.toString() === line.querySelector('code').textContent
  }));
  return () => {
    content.querySelectorAll('.peek-cli').forEach((fresh, index) => {
      if (!kept[index]) return;
      fresh.querySelector('[role="status"]').textContent = kept[index].status;
      if (kept[index].selected) selection.selectAllChildren(fresh.querySelector('code'));
    });
  };
}

// Command details open folded; a refresh of the same Command keeps what was opened.
let foldsCommand = null;
function keepFolds(url) {
  const command = new URL(url, location.href).searchParams.get('command');
  const open = command === foldsCommand
    ? new Set([...content.querySelectorAll('details[data-fact][open]')].map(fold => fold.dataset.fact))
    : new Set();
  foldsCommand = command;
  return () => {
    for (const fold of content.querySelectorAll('details[data-fact]')) if (open.has(fold.dataset.fact)) fold.open = true;
  };
}

async function loadPeek(url) {
  pendingPeek?.abort();
  const request = new AbortController();
  pendingPeek = request;
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    request.abort();
  }, 10000);
  try {
    const response = await fetch(url, { signal: request.signal, cache: 'no-store' });
    if (!response.ok) throw new Error('Details unavailable');
    const html = await response.text();
    if (request.signal.aborted) return;
    const restoreCopy = keepCopy();
    const restoreFolds = keepFolds(url);
    // Only the server's escaped show template is inserted, never a log.
    content.replaceChildren(document.createRange().createContextualFragment(html));
    restoreCopy();
    restoreFolds();
    showDismissal();
    if (content.querySelector('[data-live="true"]')) startUpdates();
    else stopPeekUpdates();
  } catch (error) {
    if (pendingPeek === request && (timedOut || (!request.signal.aborted && error.name !== 'AbortError'))) {
      content.textContent = 'Details unavailable. Reload to try again.';
    }
  } finally {
    clearTimeout(timeout);
    if (pendingPeek === request) pendingPeek = null;
  }
}

function items() {
  return [...document.querySelectorAll('main .group > summary, main .command, main .log-row')]
    .filter(element => {
      for (let parent = element.parentElement; parent; parent = parent.parentElement) {
        if (parent.hidden || (parent.tagName === 'DETAILS' && !parent.open && element !== parent.firstElementChild)) return false;
      }
      // One-line Inbox rows keep their child rows for Details, undrawn.
      return element.getClientRects().length > 0;
    });
}

const sectionLetters = { 'needs you': 'n', 'ready for review': 'r', landing: 'l', working: 'w', done: 'd', resolved: 'v' };
const targets = new Map();
for (const link of document.querySelectorAll('nav[aria-label="Pages"] a')) {
  targets.set(link.dataset.jump, { label: link.textContent.replace(/^\d+ /, ''), go: () => link.click() });
}
for (const link of document.querySelectorAll('#inbox .folders a[data-group]')) {
  const letter = sectionLetters[link.dataset.group];
  if (letter) targets.set(letter, {
    label: link.querySelector('span').textContent,
    go: () => {
      const current = document.querySelector(`#inbox .folders a[data-group="${CSS.escape(link.dataset.group)}"]`);
      if (current && !current.hidden) openFolder(current);
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

// Copy the rendered facts, including child Commands, without fetching or exposing logs.
function needsYouContext(row) {
  const facts = row.cloneNode(true);
  for (const control of facts.querySelectorAll('button, form, [role="status"], .new, .heartbeat, .gist')) control.remove();
  for (const link of facts.querySelectorAll('a')) {
    link.replaceWith(link.textContent === link.href ? link.href : `${link.textContent} (${link.href})`);
  }
  for (const block of facts.querySelectorAll('p, li, ul, .row-head, .row-head > span')) block.append('\n');
  const text = facts.textContent.split('\n').map(line => line.trim()).filter(Boolean).join('\n');
  const command = row.dataset.command.replaceAll("'", "'\\''");
  return `thirdshift · Needs you\nMachine: ${document.querySelector('header h1').textContent}\nCommand: ${row.dataset.command}\n${text}\nthirdshift show '${command}' --json`;
}

document.addEventListener('click', async event => {
  const copy = event.target.closest('[data-copy-context]');
  if (!copy) return;
  const row = copy.closest('.command');
  const text = needsYouContext(row);
  try {
    await navigator.clipboard.writeText(text);
    document.getElementById(row.id)?.querySelector('[data-copy-status]')?.replaceChildren('Copied.');
  } catch {
    // Keep a selectable snapshot outside the live Inbox so updates cannot erase it.
    const dialog = document.getElementById('copy-context');
    const field = dialog.querySelector('textarea');
    field.value = text;
    dialog.showModal();
    field.focus();
    field.select();
  }
});

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
// A copy icon copies its one value, such as a log path, and shows a tick when done.
content.addEventListener('click', async event => {
  const copy = event.target.closest('[data-copy-text]');
  if (!copy) return;
  try {
    await navigator.clipboard.writeText(copy.dataset.copyText);
    copy.dataset.copied = '';
    copy.setAttribute('aria-label', 'Copied');
  } catch {
    getSelection().selectAllChildren(copy.previousElementSibling);
  }
});

document.addEventListener('submit', async event => {
  const form = event.target.closest('form[data-dismiss]');
  if (!form) return;
  event.preventDefault();
  const command = form.elements.command.value;
  if (dismissals.get(command)?.pending) return;
  const undo = new URL(form.action).pathname === '/dismiss/undo';
  const row = form.closest('.one-line .command');
  const subject = row?.querySelector(':scope > .row-head > .subject').textContent;
  const visible = row ? items().filter(item => item.matches('.command')) : [];
  const next = visible[(visible.indexOf(row) + 1) % visible.length];
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
    if (row && !undo) {
      dismissalNotice(form, subject);
      if (selected?.dataset.command === command) {
        const nextRow = next !== row ? document.getElementById(next?.id) : null;
        select(nextRow);
        nextRow?.focus({ preventScroll: true });
      }
    }
    if (undo) removeDismissalNotice(command);
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
  const folder = event.target.closest('#inbox .folders a[data-group]');
  if (folder && event.button === 0 && !(event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)) { event.preventDefault(); openFolder(folder); }
  else if (link) { event.preventDefault(); select(row); togglePeek(row); }
  else if (row && !event.target.closest('a, button, input, select, textarea')) {
    select(row);
    // One-line rows, in the Inbox, History and Security, open their Details.
    if (row.closest('.one-line')) showPeek(row);
  }
  else if (event.target.closest('main .group > summary')) {
    const summary = event.target.closest('summary');
    if (summary.parentElement.open || !selected?.matches('.command') || !summary.parentElement.contains(selected)) select(summary);
  }
});

document.addEventListener('keydown', event => {
  if (document.querySelector('#copy-context[open]')) return;
  if (event.ctrlKey || event.metaKey || event.altKey || event.target.closest('input, select, textarea, [contenteditable="true"]')) return;
  if (event.target.matches('summary') && !event.target.closest('.group') && ['Enter', ' '].includes(event.key)) return;
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
    if (!peek.hidden && selected?.matches('.one-line .command')) showPeek(selected);
  } else if (event.key === 'd' && !event.shiftKey && selected) {
    const form = selected.querySelector(':scope > .row-head form[data-dismiss]');
    if (!form || !selected.matches('.one-line .command')) return;
    event.preventDefault();
    form.requestSubmit();
  } else if (event.key === ' ' && selected) {
    event.preventDefault();
    if (selected.tagName === 'SUMMARY') selected.parentElement.open = !selected.parentElement.open;
    else togglePeek(selected);
  } else if (event.key === 'z' && selected) {
    event.preventDefault();
    const section = selected.closest('details.group');
    if (!section) return;
    section.open = false;
    select(section.querySelector('summary'));
  } else if (event.key === 'Enter' && selected) {
    event.preventDefault();
    const row = selected.matches('.command, .log-row') ? selected : selected.parentElement.querySelector('.command, .log-row');
    if (row?.dataset.log) location.assign(row.dataset.log);
  }
});
