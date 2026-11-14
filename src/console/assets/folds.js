// Group disclosures: a heading this browser folded or opened keeps that
// choice; otherwise the server's default for the current folder applies.
const foldKey = 'thirdshift:console:folds';
// Older Consoles stored every group's state here, and rendered every group
// open, so only a closed group there records a choice.
const legacyKey = 'thirdshift:console:folded';

// Browser preferences are conveniences; malformed or unavailable storage hides nothing.
function stored(key) {
  try {
    const value = JSON.parse(localStorage.getItem(key));
    return value && typeof value === 'object' && !Array.isArray(value) ? value : null;
  } catch { return null; }
}
function remember(value) {
  try { localStorage.setItem(foldKey, JSON.stringify(value)); } catch { /* storage is optional */ }
}
function choicesIn(value) {
  return Object.fromEntries(Object.entries(value ?? {}).filter(([, open]) => typeof open === 'boolean'));
}
function load() {
  const current = stored(foldKey);
  if (current) return choicesIn(current);
  const legacy = Object.fromEntries(Object.entries(choicesIn(stored(legacyKey))).filter(([, open]) => !open));
  if (Object.keys(legacy).length) remember(legacy);
  return legacy;
}
const choices = load();

const groups = () => [...document.querySelectorAll('details[data-group]')];
const currentFolder = () => document.querySelector('#inbox .folders a[aria-current="page"]')?.dataset.group;
// The folder whose defaults the groups follow, once this script has applied them.
let applied = false;
let folder;
// Overview folds Done and Resolved; their own folders, and every other group, open.
function expected(section) {
  return choices[section.dataset.group]
    ?? !(folder === 'overview' && section.hasAttribute('data-overview-folded'));
}

// Saves only these groups, so another tab's newer choices survive.
function choose(sections) {
  const chosen = Object.fromEntries(sections.map(section => [section.dataset.group, section.open]));
  Object.assign(choices, chosen);
  remember({ ...stored(foldKey), ...chosen });
}
// A group left unlike its expected state was changed by hand, even when its
// native toggle event has not arrived yet, or never will.
function saveUnsaved() {
  const changed = groups().filter(section => section.open !== expected(section));
  if (changed.length) choose(changed);
}

// On entering another folder, applies each choice, or that folder's default.
// Within one folder, live refreshes and Sections changes leave groups alone.
export function applyFolds() {
  const shown = currentFolder();
  if (applied && shown === folder) return;
  if (applied) saveUnsaved();
  applied = true;
  folder = shown;
  for (const section of groups()) {
    const open = expected(section);
    if (section.open !== open) section.open = open;
  }
}

// A toggle that leaves a group as expected is this script's own, or changes nothing.
document.addEventListener('toggle', event => {
  const section = event.target;
  if (section.matches?.('details[data-group]') && section.open !== expected(section)) choose([section]);
}, true);
// Navigation can discard a native toggle event before it saves the choice.
window.addEventListener('pagehide', saveUnsaved);

applyFolds();
