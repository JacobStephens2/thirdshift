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
  for (const row of document.querySelectorAll('.command[data-logging="true"]')) {
    const previous = previousGrowth.get(row.id) ?? [0, 0];
    const increased = logLengths(row).some((size, index) => size > previous[index]);
    if (increased && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
      row.querySelector(":scope > .row-head > .heartbeat")?.animate([
        { transform: "scale(1)", opacity: 0.4 },
        { transform: "scale(1.8)", opacity: 1, offset: 0.3 },
        { transform: "scale(1)", opacity: 0.4 }
      ], { duration: 900 });
    }
  }
}

// Coalesce notices, but fetch again if a change arrives during the request.
let refreshing = false, pending = false;
async function refresh() {
  pending = true;
  if (refreshing) return;
  refreshing = true;
  try {
    while (pending) {
      pending = false;
      const response = await fetch("/", { cache: "no-store" });
      if (!response.ok) throw new Error("Inbox unavailable");
      morph(await response.text());
    }
  } catch {
    startPolling();
  } finally { refreshing = false; }
}
let polling;
function startPolling() {
  polling ??= setInterval(refresh, 5000);
}
if (document.getElementById('inbox') && typeof EventSource === "function") {
  const stream = new EventSource("/events");
  stream.addEventListener("commands", refresh);
  stream.addEventListener("error", startPolling);
  stream.addEventListener("open", () => {
    clearInterval(polling);
    polling = undefined;
    refresh();
  });
} else if (document.getElementById('inbox')) { startPolling(); }

const peek = document.querySelector('#peek');
const content = document.querySelector('#peek-content');
const help = document.querySelector('#keys');
const jumps = document.querySelector('#jumps');
let selected = null;
let peekCommand = null;
let pendingPeek = null;

function select(element, { scroll = true } = {}) {
  selected?.classList.remove('is-selected');
  selected?.removeAttribute('aria-current');
  selected = element;
  selected?.classList.add('is-selected');
  if (selected?.matches('.command')) selected.setAttribute('aria-current', 'true');
  if (scroll) selected?.scrollIntoView({ block: 'nearest' });
}

function closePeek() {
  pendingPeek?.abort();
  pendingPeek = null;
  peekCommand = null;
  peek.hidden = true;
  document.body.classList.remove('peek-open');
}

async function togglePeek(row) {
  if (!row.dataset.command) return;
  if (peekCommand === row.dataset.command) { closePeek(); return; }
  pendingPeek?.abort();
  const request = new AbortController();
  pendingPeek = request;
  peekCommand = row.dataset.command;
  content.textContent = 'Loading details…';
  peek.hidden = false;
  document.body.classList.add('peek-open');
  try {
    const response = await fetch(row.dataset.peek, { signal: request.signal });
    if (!response.ok) throw new Error('Details unavailable');
    const html = await response.text();
    if (request.signal.aborted) return;
    // Only the server's escaped show template is inserted, never a log.
    content.replaceChildren(document.createRange().createContextualFragment(html));
  } catch (error) {
    if (!request.signal.aborted && error.name !== 'AbortError') content.textContent = 'Details unavailable. Reload to try again.';
  }
}

function items() {
  return [...document.querySelectorAll('main .group > summary, main .command, main .log-row')]
    .filter(element => {
      for (let parent = element.parentElement; parent; parent = parent.parentElement) {
        if (parent.tagName === 'DETAILS' && !parent.open && element !== parent.firstElementChild) return false;
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
