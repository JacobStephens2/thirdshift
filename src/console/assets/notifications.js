import { watchCommands } from './changes.js';

const button = document.getElementById('notifications-toggle');
const status = document.getElementById('notifications-status');
const preference = 'thirdshift:console:notifications';
const snapshot = 'thirdshift:console:notification-count';
let stopWatching, generation = 0, failure = '';

function unavailable() {
  if (!window.isSecureContext) return 'Notifications require HTTPS or localhost.';
  if (typeof window.Notification !== 'function') return 'This browser has no Notification API.';
  if (Notification.permission === 'denied') return 'Notifications are blocked. Allow them in browser site settings.';
  if (!navigator.locks) return 'This browser cannot coordinate notifications across tabs.';
  try {
    localStorage.setItem(`${preference}:check`, '1');
    localStorage.removeItem(`${preference}:check`);
  } catch { return 'Notifications require browser storage.'; }
  return failure;
}

function enabled() {
  return !unavailable() && Notification.permission === 'granted' && localStorage.getItem(preference) === 'on';
}

function showFailure(message) {
  failure = message;
  sync();
}

async function observe(previous, currentGeneration) {
  // Fetch inside the lock: a delayed tab must never compare an old response
  // with a newer count, or a fall followed by a rise could notify twice.
  return navigator.locks.request(snapshot, async () => {
    if (currentGeneration !== generation || !enabled()) return;
    const response = await fetch('/', { cache: 'no-store' });
    if (!response.ok) throw new Error('Inbox unavailable');
    const next = new DOMParser().parseFromString(await response.text(), 'text/html');
    const inbox = next.getElementById('inbox');
    if (!inbox) throw new Error('Missing Inbox');
    if (currentGeneration !== generation || !enabled()) return;
    const rows = [...inbox.querySelectorAll('[data-group="needs you"] > .commands > .command')];
    let shared;
    try { shared = JSON.parse(localStorage.getItem(snapshot)); } catch { /* reset malformed history */ }
    const current = { count: rows.length, ids: rows.map(row => row.id) };
    const handled = shared?.count > current.count || (shared?.count === current.count && shared?.handled === true);
    const notify = previous && current.count > previous.count && !handled;
    // Loading a tab establishes its own baseline without consuming a rise
    // another open tab has yet to see. A fall is consumed too, so a delayed tab
    // cannot mistake it for a rise from its older baseline.
    localStorage.setItem(snapshot, JSON.stringify({ count: current.count, handled: handled || !!notify }));
    if (!notify) return current;
    const added = rows.filter(row => !previous.ids.includes(row.id));
    const newest = (added.length ? added : rows).sort((a, b) => Number(b.dataset.occurredMs) - Number(a.dataset.occurredMs))[0];
    const text = selector => newest?.querySelector(`:scope > .row-head > ${selector}`)?.textContent.trim() ?? '';
    const host = next.querySelector('header h1').textContent;
    let notification;
    try {
      notification = new Notification(`${rows.length} need you · ${host}`, {
        body: `${text('.subject')} · ${text('.issue-title')} · ${text('.state')}`,
        icon: '/assets/favicon-urgent.svg'
      });
    } catch {
      showFailure('This browser cannot show notifications from an open tab.');
      return current;
    }
    notification.onclick = event => {
      event.preventDefault();
      window.focus();
      notification.close();
      location.assign('/?page=inbox');
    };
    return current;
  });
}

function sync() {
  const reason = unavailable();
  const on = enabled();
  button.disabled = !!reason;
  button.setAttribute('aria-pressed', String(on));
  button.textContent = on ? 'Turn notifications off' : 'Turn notifications on';
  status.textContent = reason || '';
  if (!on) {
    generation++;
    stopWatching?.();
    stopWatching = undefined;
  } else if (!stopWatching) {
    let previous;
    const currentGeneration = ++generation;
    stopWatching = watchCommands(async () => {
      previous = await observe(previous, currentGeneration);
    });
  }
}

button.addEventListener('click', async () => {
  try {
    if (enabled()) localStorage.setItem(preference, 'off');
    else {
      const permission = await Notification.requestPermission();
      if (permission === 'granted') localStorage.setItem(preference, 'on');
      else if (permission === 'default') status.textContent = 'Notification permission was not granted.';
    }
    sync();
  } catch { showFailure('Notifications unavailable. Reload to try again.'); }
});
window.addEventListener('storage', event => { if (event.key === preference || event.key === null) sync(); });
window.addEventListener('focus', sync);
window.addEventListener('pagehide', () => { generation++; stopWatching?.(); stopWatching = undefined; });
window.addEventListener('pageshow', sync);
sync();
