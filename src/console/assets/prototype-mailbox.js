// PROTOTYPE (throwaway): mailbox layouts for the Inbox, switched with ?variant=.
// Question: what should a mailbox-style Inbox look like? Not for main.
// Morphs rewrite #inbox's attributes, so state lives on <html> where it can.

const VARIANTS = [
  ['now', 'Current layout'],
  ['a', 'Three-pane mail'],
  ['b', 'Tabbed inbox, rows open in place'],
  ['c', 'Table, reading pane below']
];
const params = new URLSearchParams(location.search);
const variant = VARIANTS.some(([key]) => key === params.get('variant')) ? params.get('variant') : 'now';
const root = document.documentElement;
const inbox = document.getElementById('inbox');
root.dataset.protoVariant = variant;

function go(step) {
  const index = VARIANTS.findIndex(([key]) => key === variant);
  const [next] = VARIANTS[(index + step + VARIANTS.length) % VARIANTS.length];
  const url = new URL(location.href);
  if (next === 'now') url.searchParams.delete('variant'); else url.searchParams.set('variant', next);
  location.replace(url);
}

const bar = document.createElement('div');
bar.className = 'proto-bar';
bar.innerHTML = '<button type="button" aria-label="Previous variant">←</button><span></span><button type="button" aria-label="Next variant">→</button>';
bar.querySelector('span').textContent = `PROTOTYPE · ${variant.toUpperCase()} (${VARIANTS.find(([key]) => key === variant)[1]})`;
bar.children[0].addEventListener('click', () => go(-1));
bar.children[2].addEventListener('click', () => go(1));
document.body.append(bar);
document.addEventListener('keydown', event => {
  if (event.target.closest('input, textarea, select, [contenteditable]')) return;
  if (event.key === 'ArrowLeft') go(-1);
  if (event.key === 'ArrowRight') go(1);
});

const groups = () => [...inbox.querySelectorAll('details.group[data-group]:not([hidden])')];
const css = value => value.replace(/["\\]/g, '\\$&');
let folder = sessionStorage.getItem('proto-folder');
let expanded = null;
let peeked = null;

function openPeek(row) {
  if (!row || peeked === row.dataset.command) return;
  peeked = row.dataset.command;
  row.querySelector('.peek-link')?.click();
}

// A and B: one section at a time, chosen from folders or tabs.
function drawFolders() {
  const list = groups();
  if (!list.length) return;
  if (!list.some(group => group.dataset.group === folder)) {
    folder = (list.find(group => group.querySelector('.command')) ?? list[0]).dataset.group;
  }
  let nav = document.querySelector('.proto-folders');
  if (!nav) {
    nav = document.createElement('nav');
    nav.className = 'proto-folders';
    nav.setAttribute('aria-label', 'Inbox sections');
    nav.addEventListener('click', event => {
      const button = event.target.closest('button[data-folder]');
      if (!button) return;
      folder = button.dataset.folder;
      sessionStorage.setItem('proto-folder', folder);
      expanded = null;
      apply();
      if (variant === 'a') openPeek(inbox.querySelector(`details[data-group="${css(folder)}"] .command`));
    });
    inbox.before(nav);
  }
  nav.replaceChildren(...list.map(group => {
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.folder = group.dataset.group;
    button.setAttribute('aria-pressed', String(group.dataset.group === folder));
    const count = group.querySelectorAll(':scope > .commands > .command').length;
    button.innerHTML = '<span></span><span class="proto-count"></span>';
    button.children[0].textContent = group.querySelector('summary > span')?.textContent ?? group.dataset.group;
    button.children[1].textContent = count;
    if (group.dataset.group === 'needs you' && count) button.classList.add('proto-urgent');
    return button;
  }));
  for (const group of list) if (group.dataset.group === folder) group.open = true;
}

// The Console's CSP refuses inline styles, so state lives in attributes and
// classes that prototype-mailbox.css matches; morphs drop the row class, so
// it is put back after each one.
function apply() {
  if (variant === 'a' || variant === 'b') {
    drawFolders();
    root.dataset.protoFolder = (folder ?? '').replace(/ /g, '-');
  }
  for (const row of inbox.querySelectorAll('.proto-expanded')) if (row.id !== expanded) row.classList.remove('proto-expanded');
  const open = variant === 'b' && expanded && document.getElementById(expanded);
  // Only a real change, or the observer below would wake itself forever.
  if (open && !open.classList.contains('proto-expanded')) open.classList.add('proto-expanded');
}

inbox.addEventListener('click', event => {
  if (variant === 'now') return;
  const row = event.target.closest('.command');
  if (!row || event.target.closest('a, button, form, summary')) return;
  if (variant === 'b') {
    expanded = expanded === row.id ? null : row.id;
    apply();
  } else {
    openPeek(row);
  }
});

if (variant !== 'now') {
  document.body.classList.add('full-width');
  apply();
  let pending = false;
  new MutationObserver(() => {
    if (pending) return;
    pending = true;
    queueMicrotask(() => { pending = false; apply(); });
  }).observe(inbox, { childList: true, subtree: true, characterData: true, attributeFilter: ['class'] });
  // A and C keep a reading pane open, as a mail client does.
  if (variant === 'a' || variant === 'c') {
    const first = variant === 'a'
      ? inbox.querySelector(`details[data-group="${css(folder ?? '')}"] .command`)
      : inbox.querySelector('details.group[open] .command');
    openPeek(first);
  }
  document.querySelector('[data-close-peek]')?.addEventListener('click', () => { peeked = null; });
}
