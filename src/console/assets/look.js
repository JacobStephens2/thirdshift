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
  root.dataset.live = String(!!document.querySelector('.command[data-live="true"], #log-content[data-live="true"]'));
}

function apply(settings) {
  root.dataset.theme = settings.theme;
  for (const section of document.querySelectorAll('#inbox > details[data-group]')) {
    section.hidden = settings.hidden_sections.includes(section.dataset.group.replaceAll(' ', '-'));
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
    const settings = await response.json();
    if (requestedRevision !== revision || saving) return;
    csrf = settings.csrf;
    const picker = document.querySelector('#theme-name');
    if (!picker.options.length) {
      for (const name of settings.themes) picker.add(new Option(name.replace('-', ' · '), name));
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
    apply(await response.json());
    message.textContent = 'Console look saved.';
  } catch { message.textContent = 'Could not save the Console look. Reload to try again.'; }
  finally { saving = false; }
}

if (themes) {
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
