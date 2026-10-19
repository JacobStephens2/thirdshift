// Folded sections are a per-browser convenience. A fresh visit opens Needs you.
const key = "thirdshift:console:folded";
let folded = {};
try { folded = JSON.parse(localStorage.getItem(key) ?? "{}"); } catch { /* storage is optional */ }
for (const section of document.querySelectorAll("details[data-group]")) {
  if (typeof folded?.[section.dataset.group] === "boolean") section.open = folded[section.dataset.group];
  section.addEventListener("toggle", () => {
    folded ??= {};
    folded[section.dataset.group] = section.open;
    try { localStorage.setItem(key, JSON.stringify(folded)); } catch { /* storage is optional */ }
  });
}

const peek = document.querySelector('#peek');
const content = document.querySelector('#peek-content');
const help = document.querySelector('#keys');
const jumps = document.querySelector('#jumps');
let selected = null;
let peekCommand = null;
let pendingPeek = null;

function select(element) {
  selected?.classList.remove('is-selected');
  selected = element;
  selected?.classList.add('is-selected');
  selected?.scrollIntoView({ block: 'nearest' });
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
  return [...document.querySelectorAll('main .group > summary, main .command')]
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
    go: () => { section.open = true; select(section.querySelector('summary')); }
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
  const row = event.target.closest('main .command');
  if (link) { event.preventDefault(); select(row); togglePeek(row); }
  else if (row && !event.target.closest('a, button, input, select, textarea')) select(row);
  else if (event.target.closest('main .group > summary')) select(event.target.closest('summary'));
});

document.addEventListener('keydown', event => {
  if (event.ctrlKey || event.metaKey || event.altKey || event.target.closest('input, select, textarea, [contenteditable="true"]')) return;
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
    section.open = false;
    select(section.querySelector('summary'));
  } else if (event.key === 'Enter' && selected) {
    event.preventDefault();
    const row = selected.matches('.command') ? selected : selected.parentElement.querySelector('.command');
    if (row?.dataset.command) location.assign(row.dataset.log);
  }
});
