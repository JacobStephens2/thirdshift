// Group disclosures: a heading this browser folded or opened keeps that
// choice; otherwise the server's default for the current folder applies.
const foldKey = 'thirdshift:console:folds';
// Older Consoles stored every group's state here, and rendered every group
// open, so only a closed group there records a choice.
const legacyKey = 'thirdshift:console:folded';

// Browser preferences are conveniences; malformed or unavailable storage hides nothing.
function storedObject(key) {
  try {
    const value = JSON.parse(localStorage.getItem(key));
    return value && typeof value === 'object' && !Array.isArray(value) ? value : null;
  } catch { return null; }
}
function saveChoices(value) {
  try { localStorage.setItem(foldKey, JSON.stringify(value)); } catch { /* storage is optional */ }
}
function choicesIn(value) {
  return Object.fromEntries(Object.entries(value ?? {}).filter(([, open]) => typeof open === 'boolean'));
}
function load() {
  const current = storedObject(foldKey);
  if (current) return choicesIn(current);
  const legacy = Object.fromEntries(Object.entries(choicesIn(storedObject(legacyKey))).filter(([, open]) => !open));
  if (Object.keys(legacy).length) saveChoices(legacy);
  return legacy;
}
const choices = load();

const groups = () => [...document.querySelectorAll('details[data-group]')];
const currentFolder = () => document.querySelector('#inbox .folders a[aria-current="page"]')?.dataset.group;
// The folder whose defaults the groups follow, once this script has applied them.
let applied = false;
let appliedFolder;
// Overview folds Done and Resolved; their own folders, and every other group, open.
function foldedByDefault(section) {
  return appliedFolder === 'overview' && section.hasAttribute('data-overview-folded');
}
// Opening Done in Overview to glance at it is not a choice for its own folder,
// so Overview keeps its choices for those groups apart.
function choiceKey(section) {
  return foldedByDefault(section) ? `${section.dataset.group} in overview` : section.dataset.group;
}
function expectedOpen(section) {
  return choices[choiceKey(section)] ?? !foldedByDefault(section);
}

// Saves only these groups, so another tab's newer choices survive.
function choose(sections) {
  const chosen = Object.fromEntries(sections.map(section => [choiceKey(section), section.open]));
  Object.assign(choices, chosen);
  saveChoices({ ...storedObject(foldKey), ...chosen });
}
// A group left unlike its expected state was changed by hand, even when its
// native toggle event has not arrived yet, or never will.
function saveUnsaved() {
  const changed = groups().filter(section => section.open !== expectedOpen(section));
  if (changed.length) choose(changed);
}

// On entering another folder, applies each choice, or that folder's default.
// Within one folder, live refreshes and Sections changes leave groups alone.
export function applyFolds() {
  const shown = currentFolder();
  if (applied && shown === appliedFolder) return;
  if (applied) saveUnsaved();
  applied = true;
  appliedFolder = shown;
  for (const section of groups()) {
    const open = expectedOpen(section);
    if (section.open !== open) section.open = open;
  }
}

// A toggle that leaves a group as expected is this script's own, or changes nothing.
document.addEventListener('toggle', event => {
  const section = event.target;
  if (section.matches?.('details[data-group]') && section.open !== expectedOpen(section)) choose([section]);
}, true);
// Navigation can discard a native toggle event before it saves the choice.
window.addEventListener('pagehide', saveUnsaved);

applyFolds();
