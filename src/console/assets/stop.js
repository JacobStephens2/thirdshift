// The Stop page shares the Command push stream, with polling after a disconnect.
let refreshing = false, pending = false, polling;
async function refresh() {
  pending = true;
  if (refreshing) return;
  refreshing = true;
  try {
    while (pending) {
      pending = false;
      const response = await fetch(location.href, { cache: 'no-store' });
      if (!response.ok || response.redirected) throw new Error('Stop unavailable');
      const next = new DOMParser().parseFromString(await response.text(), 'text/html');
      const replacement = next.getElementById('stop-state');
      if (!replacement) throw new Error('Stop unavailable');
      const current = document.getElementById('stop-state');
      if (current.innerHTML !== replacement.innerHTML) current.replaceWith(replacement);
    }
  } catch { startPolling(); }
  finally { refreshing = false; }
}
function startPolling() { polling ??= setInterval(refresh, 5000); }
if (typeof EventSource === 'function') {
  const stream = new EventSource('/events');
  stream.addEventListener('commands', refresh);
  stream.addEventListener('error', startPolling);
  stream.addEventListener('open', () => {
    clearInterval(polling);
    polling = undefined;
    refresh();
  });
} else { startPolling(); }
