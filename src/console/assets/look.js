// The machine owns its look; only Pause motion belongs to this browser.
const root = document.documentElement;
const themes = document.querySelector('#themes');
const sections = document.querySelector('#sections-chooser');
const message = document.querySelector('#look-status');
const pause = document.querySelector('#pause-motion');
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
let csrf;
let revision = 0;
let saving = false;
let paused = false;
let themeInventory = '';
let themeRevision = '';
let styleRequest = 0;
async function refreshStylesheet(previous, link, isCurrent, preserveUnchanged = false) {
  const id = link.id;
  const media = link.media;
  link.removeAttribute('id');
  // Load without applying: a stale response must never replace current fonts.
  link.media = 'not all';
  const url = new URL(link.href);
  url.searchParams.set('revision', Date.now());
  link.href = url.href;
  const loaded = new Promise((resolve, reject) => { link.onload = resolve; link.onerror = reject; });
  document.head.append(link);
  try {
    await loaded;
    if (!isCurrent() || (previous && !previous.isConnected)) {
      link.remove();
      return;
    }
    const rules = sheet => [...sheet.cssRules].map(rule => rule.cssText).join('\n');
    // Selecting a Theme changes /look's revision without changing its CSS.
    // Preserve the loaded font faces when the stylesheet is already current.
    if (preserveUnchanged && previous?.sheet && rules(previous.sheet) === rules(link.sheet)) {
      link.remove();
      return;
    }
    link.media = media;
    link.id = id;
    previous?.remove();
  }
  catch (error) { link.remove(); throw error; }
}

async function refreshThemeStyles() {
  const requestedStyle = ++styleRequest;
  const requestedRevision = revision;
  const link = Object.assign(document.createElement('link'), {
    rel: 'stylesheet', href: '/assets/user-themes.css', id: 'user-theme-styles',
  });
  await refreshStylesheet(document.querySelector('#user-theme-styles'), link,
    () => requestedStyle === styleRequest && requestedRevision === revision, true);
}

let assetStyleRequest = 0;
export async function refreshAssetStyles() {
  const requested = ++assetStyleRequest;
  const links = [...document.querySelectorAll('link[rel="stylesheet"][href]')]
    .filter(link => new URL(link.href).pathname.startsWith('/assets/') && link.id !== 'user-theme-styles' && link.media !== 'not all');
  // Reload the containing sheet too: console.css imports themes.css.
  await Promise.all(links.map(link => refreshStylesheet(link, link.cloneNode(), () => requested === assetStyleRequest)
    .catch(() => { /* Keep the loaded stylesheet if an edit cannot be loaded. */ })));
}
try { paused = localStorage.getItem('thirdshift:console:pause-motion') === 'true'; } catch { /* optional storage */ }

export function motionAllowed() { return !paused && !reduced.matches; }
function motion() {
  root.dataset.paused = String(paused);
  pause?.setAttribute('aria-pressed', String(paused));
  if (!motionAllowed()) for (const animation of document.getAnimations()) animation.cancel();
}
pause?.addEventListener('click', () => {
  paused = !paused;
  try { localStorage.setItem('thirdshift:console:pause-motion', String(paused)); } catch { /* optional storage */ }
  motion();
});
reduced.addEventListener('change', motion);
motion();

export function activity() {
  root.dataset.live = String(!!document.querySelector('.command[data-live="true"], #log-content[data-live="true"], #span-tree[data-live="true"]'));
}

function apply(settings) {
  root.dataset.theme = settings.theme;
  const hide = group => settings.hidden_sections.includes(group.replaceAll(' ', '-'));
  for (const link of document.querySelectorAll('#inbox .folders a[data-group]')) link.hidden = hide(link.dataset.group);
  for (const section of document.querySelectorAll('#inbox > .group[data-group]')) if (hide(section.dataset.group)) section.hidden = true;
  // Hiding the open folder opens the first one left with rows, as the server would.
  if (document.querySelector('#inbox .folders') && !document.querySelector('#inbox > .group:not([hidden])')) {
    (document.querySelector('#inbox .folders a:not([hidden]):not([data-empty])') ?? document.querySelector('#inbox .folders a:not([hidden])'))?.click();
  }
  if (!sections.open) for (const checkbox of document.querySelectorAll('#section-options input')) checkbox.checked = !settings.hidden_sections.includes(checkbox.value);
  if (!themes.open) document.querySelector('#theme-name').value = settings.theme;
  activity();
}

export async function loadLook() {
  if (!themes || saving) return;
  const requestedRevision = revision;
  try {
    const response = await fetch('/look', { cache: 'no-store' });
    if (!response.ok || response.redirected) throw new Error('Look unavailable');
    if (response.headers.get('x-thirdshift-console-assets') === 'development') {
      document.dispatchEvent(new Event('console-development-assets'));
    }
    const settings = await response.json();
    if (requestedRevision !== revision || saving) return;
    csrf = settings.csrf;
    const picker = document.querySelector('#theme-name');
    const inventory = JSON.stringify(settings.themes);
    if (inventory !== themeInventory || root.dataset.theme !== settings.theme || themeRevision !== settings.theme_revision) {
      await refreshThemeStyles();
      if (requestedRevision !== revision || saving) return;
      themeInventory = inventory;
      themeRevision = settings.theme_revision;
      const selected = picker.value;
      picker.replaceChildren(...settings.themes.map(name => new Option(name.replace('-', ' · '), name)));
      picker.value = settings.themes.includes(selected) ? selected : settings.theme;
    }
    if (!document.querySelector('#section-options input')) {
      for (const name of settings.sections) {
        const label = document.createElement('label');
        const checkbox = document.createElement('input');
        checkbox.type = 'checkbox'; checkbox.value = name;
        label.append(checkbox, ` Show ${name.replaceAll('-', ' ').replace(/^./, c => c.toUpperCase())}`);
        document.querySelector('#section-options').append(label);
      }
    }
    apply(settings);
  } catch { message.textContent = 'Could not load the Console look. Reload to try again.'; }
}

async function save(path, values) {
  revision++;
  saving = true;
  message.textContent = '';
  try {
    const response = await fetch(path, { method: 'POST', headers: { 'x-csrf-token': csrf }, body: new URLSearchParams(values) });
    if (!response.ok || response.redirected) throw new Error('Save failed');
    if (path === '/theme') await refreshThemeStyles();
    apply(await response.json());
    message.textContent = 'Console look saved.';
  } catch { message.textContent = 'Could not save the Console look. Reload to try again.'; }
  finally { saving = false; }
}

if (themes) {
  const editor = document.querySelector('#theme-document');
  const editorStatus = document.querySelector('#theme-editor-status');
  const exportDocument = async copy => {
    editorStatus.textContent = '';
    try {
      const name = document.querySelector('#theme-name').value;
      const response = await fetch(`/themes/${encodeURIComponent(name)}`, { cache: 'no-store' });
      if (!response.ok || response.redirected) throw new Error(await response.text());
      const theme = await response.json();
      if (copy) theme.name += '-copy';
      editor.value = JSON.stringify(theme, null, 2);
      editor.focus();
      editorStatus.textContent = copy ? 'Choose a name and edit the tokens, then save.' : 'Exported JSON. Copy it to a file or paste it into another Console.';
    } catch (error) { editorStatus.textContent = `Could not export Theme: ${error.message}`; }
  };
  document.querySelector('#edit-theme').addEventListener('click', () => exportDocument(true));
  document.querySelector('#export-theme').addEventListener('click', () => exportDocument(false));
  document.querySelector('#theme-editor').addEventListener('submit', async event => {
    event.preventDefault();
    if (saving) return;
    revision++;
    saving = true;
    editorStatus.textContent = '';
    try {
      const response = await fetch('/themes', { method: 'POST', headers: { 'content-type': 'application/json', 'x-csrf-token': csrf }, body: editor.value });
      if (!response.ok || response.redirected) throw new Error(await response.text());
      editor.value = JSON.stringify(await response.json(), null, 2);
      await refreshThemeStyles();
      editorStatus.textContent = 'Theme saved. Choose it above to apply.';
    } catch (error) { editorStatus.textContent = `Could not save Theme: ${error.message}`; }
    finally { saving = false; }
    await loadLook();
  });
  document.querySelector('[data-open-themes]').addEventListener('click', async () => { await loadLook(); themes.showModal(); });
  document.querySelector('[data-close-themes]').addEventListener('click', () => themes.close());
  document.querySelector('[data-open-sections]').addEventListener('click', async () => { await loadLook(); sections.showModal(); });
  document.querySelector('[data-close-sections]').addEventListener('click', () => sections.close());
  document.querySelector('#theme-form').addEventListener('submit', event => {
    event.preventDefault(); save('/theme', { theme: document.querySelector('#theme-name').value });
  });
  document.querySelector('#sections-form').addEventListener('submit', event => {
    event.preventDefault();
    const options = [...document.querySelectorAll('#section-options input')];
    save('/sections', { hide: options.filter(c => !c.checked).map(c => c.value).join(','), show: options.filter(c => c.checked).map(c => c.value).join(',') });
  });
  loadLook();
}
activity();
