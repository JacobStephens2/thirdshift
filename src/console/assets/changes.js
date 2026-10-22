// One stream per tab, shared by the page, peek and notification consumers.
const subscribers = new Set();
let watcher;
export function watchCommands(update) {
  const subscriber = () => update();
  subscribers.add(subscriber);
  if (watcher) watcher.refresh();
  else watcher = watchUpdates(async () => {
    const results = await Promise.allSettled([...subscribers].map(update => Promise.resolve().then(update)));
    const failed = results.find(result => result.status === 'rejected');
    if (failed) throw failed.reason;
  });
  return () => {
    subscribers.delete(subscriber);
    if (!subscribers.size) {
      watcher?.stop();
      watcher = undefined;
    }
  };
}

// Coalesce changes while all subscribers finish, and recover by polling.
function watchUpdates(update) {
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
  return {
    refresh,
    stop() {
      active = false;
      pending = false;
      stream?.close();
      clearInterval(polling);
    }
  };
}
