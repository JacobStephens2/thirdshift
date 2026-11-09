// Resource changes need their own clock: Command events can stay idle for hours.
const status = document.getElementById('host-resources');
if (status) {
  const link = status.querySelector('a');
  async function refresh() {
    try {
      if (document.hidden) return;
      const response = await fetch('/resources/status', { signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error('Host status request failed');
      const next = new DOMParser().parseFromString(await response.text(), 'text/html').getElementById('host-resources');
      if (!next?.querySelector('a')) throw new Error('Missing host status');
      status.dataset.state = next.dataset.state;
      link.textContent = next.querySelector('a').textContent;
      link.title = next.querySelector('a').title;
    } catch {
      status.dataset.state = 'unknown';
      link.textContent = 'Host resources unavailable';
      link.title = 'Could not refresh host resources. Open Resources or reload to try again.';
    } finally {
      setTimeout(refresh, 10000);
    }
  }
  setTimeout(refresh, 10000);
}
