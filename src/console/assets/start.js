// Both page controls submit to the same scoped, CSRF-protected start API.
export function initStarts(refresh) {
  const area = document.querySelector('[data-start-area]');
  const dialog = document.getElementById('start-confirm');
  if (!area || !dialog) return;
  const result = area.querySelector('[data-start-result]');
  const accept = dialog.querySelector('[data-start-accept]');
  let pending;
  let sending = false;
  const buttons = area.querySelectorAll('[data-start-enable]');
  for (const button of buttons) button.disabled = false;

  function review(request, harness, machine = '') {
    if (sending) return;
    pending = { request, machine };
    dialog.querySelector('[data-start-description]').textContent = `${request.command ?? 'ticket'} on ${request.repository}${request.issue ? ` issue ${request.issue}` : ''}, with Harness ${harness}${machine ? ` on machine ${machine}` : ''}.`;
    dialog.querySelector('[data-start-summary]').textContent = JSON.stringify(request, null, 2);
    dialog.showModal();
  }
  area.querySelector('[data-start-form]')?.addEventListener('submit', event => {
    event.preventDefault();
    const values = new FormData(event.target);
    const request = {};
    for (const field of ['repository', 'issue', 'harness', 'model', 'effort']) {
      const value = values.get(field).trim();
      if (value) request[field] = value;
    }
    for (const field of ['merge', 'base_fix', 'security_review', 'security_fix', 'security_close']) {
      if (values.get(field)) request[field] = values.get(field) === 'true';
    }
    if (values.get('parallel')) request.parallel = Number(values.get('parallel'));
    const mode = values.get('email');
    const address = values.get('email_address').trim();
    if (address && (!mode || mode === 'never')) {
      result.textContent = 'Choose Always or Unless merged to use an email address.';
      return;
    }
    if (mode) request.email = { mode, ...(address ? { address } : {}) };
    review(request, request.harness, event.target.elements.repository.selectedOptions[0].dataset.machine);
  });
  area.addEventListener('click', event => {
    const button = event.target.closest('[data-start-request]');
    if (button) review(JSON.parse(button.dataset.startRequest), button.dataset.startHarness, area.dataset.machine);
  });
  dialog.querySelector('[data-start-cancel]').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => { pending = undefined; });
  accept.addEventListener('click', async () => {
    if (!pending || sending) return;
    const { request, machine } = pending;
    sending = true;
    for (const button of buttons) button.disabled = true;
    accept.disabled = true;
    dialog.close();
    result.textContent = 'Starting…';
    try {
      const response = await fetch(machine ? `/fleet/runs?machine=${encodeURIComponent(machine)}` : '/api/v1/runs', {
        method: 'POST', headers: { 'content-type': 'application/json', 'x-csrf-token': area.dataset.csrf },
        body: JSON.stringify(request)
      });
      const value = await response.json();
      if (!response.ok) throw new Error(value.error ?? `Start failed (${response.status}).`);
      result.replaceChildren(document.createTextNode(value.name ? `Started Command ${value.name}.` : `Pass skipped: ${value.reason}`));
      if (value.name) {
        const link = document.createElement('a');
        link.href = `/?command=${encodeURIComponent(value.name)}${machine ? `&machine=${encodeURIComponent(machine)}` : ''}`;
        link.textContent = 'Open Inbox';
        result.append(' ', link);
      }
      try { await refresh(); } catch { result.append(' Reload Inbox to see the latest rows.'); }
    } catch (error) {
      // A lost response can follow a successful spawn: never retry automatically.
      result.textContent = `${error.message} Check Inbox and logs before retrying.`;
    } finally {
      sending = false;
      for (const button of buttons) button.disabled = false;
      accept.disabled = false;
    }
  });
}
