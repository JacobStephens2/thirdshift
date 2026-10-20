// One lifecycle for Command notices, coalesced refreshes and polling recovery.
export function watchCommands(update) {
  let active = true, refreshing = false, pending = false, polling, stream;
  function startPolling() {
    if (active) polling ??= setInterval(refresh, 5000);
  }
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
  if (typeof EventSource === 'function') {
    stream = new EventSource('/events');
    stream.addEventListener('commands', refresh);
    stream.addEventListener('error', startPolling);
    stream.addEventListener('open', () => {
      clearInterval(polling);
      polling = undefined;
      refresh();
    });
  } else { startPolling(); }
  return () => {
    active = false;
    pending = false;
    stream?.close();
    clearInterval(polling);
  };
}
