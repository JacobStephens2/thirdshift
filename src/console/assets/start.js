// Page controls submit to the same scoped, CSRF-protected start API.
export function initStarts(refresh) {
  let area, dialog, result, accept;
  let pending;
  let sending = false;
  const buttons = () => document.querySelectorAll('[data-start-enable]');

  function update(next) {
    const current = document.getElementById('inbox-start');
    const replacement = next?.getElementById('inbox-start');
    // A scope change can introduce or remove the confirmation controls.
    // Otherwise keep the person's form edits and the submission result.
    const permission = node => {
      const controls = node?.querySelector('[data-start-area]');
      return controls ? `${controls.dataset.startRun}:${controls.dataset.startRelease}` : '';
    };
    if (current && replacement && !sending && permission(current) !== permission(replacement)) {
      current.replaceChildren(...replacement.childNodes);
      pending = undefined;
    }
    area = document.querySelector('[data-start-area]');
    dialog = document.getElementById('start-confirm');
    result = area?.querySelector('[data-start-result]');
    accept = dialog?.querySelector('[data-start-accept]');
    for (const button of buttons()) button.disabled = sending || !dialog;
  }
  update();

  function review(request, harness, machine = '', machineName = machine) {
    if (sending) return;
    update();
    if (!dialog) return;
    pending = { request, machine };
    const releaseMode = request.command === 'release' && !request.version ? (request.bump ? ` --${request.bump}` : ' automatic minor or patch Pass') : '';
    dialog.querySelector('[data-start-description]').textContent = `${request.command ?? 'ticket'}${request.version ? ` ${request.version}` : releaseMode} on ${request.repository}${request.issue ? ` issue ${request.issue}` : ''}${request.pull_request ? ` pull request ${request.pull_request}` : ''}, ${harness ? `with Harness ${harness}` : 'using User config and CLI defaults'}${machineName ? ` on machine ${machineName}` : ''}.`;
    dialog.querySelector('[data-start-summary]').textContent = JSON.stringify(request, null, 2);
    dialog.showModal();
  }
  document.addEventListener('submit', event => {
    if (!event.target.matches('[data-start-form]')) return;
    event.preventDefault();
    const values = new FormData(event.target);
    const request = {};
    for (const field of ['command', 'repository', 'version', 'bump', 'issue', 'harness', 'model', 'effort']) {
      const value = values.get(field)?.trim();
      if (value) request[field] = value;
    }
    if (request.version && request.bump) {
      result.textContent = 'Version and Bump are mutually exclusive.';
      return;
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
  // Delegation includes rows introduced or replaced by the Inbox's live morph.
  document.addEventListener('click', event => {
    const button = event.target.closest('[data-start-request]');
    if (button) review(JSON.parse(button.dataset.startRequest), button.dataset.startHarness,
      button.dataset.startMachine ?? area?.dataset.machine, button.dataset.startMachineName);
    if (event.target.closest('[data-start-cancel]')) dialog.close();
  });
  document.addEventListener('close', event => {
    if (event.target.id === 'start-confirm') pending = undefined;
  }, true);
  document.addEventListener('click', async event => {
    if (!event.target.closest('[data-start-accept]')) return;
    if (!pending || sending) return;
    const { request, machine } = pending;
    sending = true;
    for (const button of buttons()) button.disabled = true;
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
      for (const button of buttons()) button.disabled = false;
      accept.disabled = false;
    }
  });
  return update;
}
