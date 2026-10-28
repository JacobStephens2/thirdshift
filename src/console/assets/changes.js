// One stream per tab, shared by the page, peek and notification consumers.
import { refreshAssetStyles } from './look.js';

const subscribers = new Set();
let polling, stream;
let assets;
// Pages without live Command content still need the shared stream in development.
document.addEventListener('console-development-assets', () => watchCommands(() => {}), { once: true });
function refreshAssets(event) {
  const current = JSON.parse(event.data);
  if (assets && current.scripts !== assets.scripts) {
    location.reload();
  } else if (current.styles !== (assets?.styles ?? 0)) {
    refreshAssetStyles();
  }
  assets = current;
}
function refreshAll() {
  for (const refresh of subscribers) refresh();
}
function startPolling() {
  if (subscribers.size) polling ??= setInterval(refreshAll, 5000);
}
function startUpdates() {
  if (typeof EventSource === 'function') {
    stream = new EventSource('/events');
    stream.addEventListener('commands', refreshAll);
    stream.addEventListener('assets', refreshAssets);
    stream.addEventListener('error', startPolling);
    stream.addEventListener('open', () => {
      clearInterval(polling);
      polling = undefined;
      refreshAll();
    });
  } else { startPolling(); }
}

export function watchCommands(update) {
  // Each subscriber coalesces its own work; a slow one never gates the others.
  let active = true, refreshing = false, pending = false;
  async function refresh() {
    if (!active) return;
    pending = true;
    if (refreshing) return;
    refreshing = true;
    try {
      while (active && pending) {
        pending = false;
        await update();
      }
    } catch { startPolling(); }
    finally { refreshing = false; }
  }
  subscribers.add(refresh);
  if (subscribers.size === 1) startUpdates();
  else refresh();
  return () => {
    active = false;
    pending = false;
    subscribers.delete(refresh);
    if (!subscribers.size) {
      stream?.close();
      stream = undefined;
      clearInterval(polling);
      polling = undefined;
    }
  };
}
