// Poll the server's escaped view: Trial and Grader artifacts change independently
// of Command records. Rechecks can change an ended Experiment as well.
const main = document.getElementById('experiments');
const peek = document.getElementById('peek');
const content = document.getElementById('peek-content');
let currentPeek, pendingOnly = false, allRechecks = false, refreshing = false;

function controls() {
  const filter = main.querySelector('[data-pending-only]');
  if (!filter) pendingOnly = false;
  for (const row of main.querySelectorAll('.trial-row[data-task]')) row.hidden = pendingOnly && row.dataset.pending !== 'true';
  for (const row of main.querySelectorAll('[data-recheck-extra]')) row.hidden = !allRechecks;
  if (filter) {
    filter.setAttribute('aria-pressed', String(pendingOnly));
    filter.textContent = pendingOnly ? 'Show every Eval task below' : 'Show only their Eval tasks below';
  }
  const all = main.querySelector('[data-all-rechecks]');
  if (all) {
    all.setAttribute('aria-expanded', String(allRechecks));
    all.textContent = allRechecks ? 'Show fewer' : `Show all ${all.dataset.allRechecks}`;
  }
}

function openPeek(id, source = document) {
  const template = source.getElementById(id);
  if (!template) return;
  const position = content.scrollTop;
  content.replaceChildren(template.content.cloneNode(true));
  currentPeek = id;
  peek.hidden = false;
  document.body.classList.add('peek-open');
  content.scrollTop = position;
}

// A list row opens its Experiment page, as its headline link does.
document.addEventListener('click', event => {
  const row = event.target.closest('.experiment-row');
  if (!row || event.target.closest('a, button, input, select, textarea')) return;
  row.querySelector('.row-head .subject a')?.click();
});

document.addEventListener('click', async event => {
  const target = event.target.closest('[data-experiment-peek], [data-pending-only], [data-all-rechecks], [data-copy], [data-close-peek], .peek-link');
  if (!target) return;
  if (target.matches('[data-experiment-peek]')) openPeek(target.dataset.experimentPeek);
  else if (target.matches('[data-pending-only]')) {
    pendingOnly = !pendingOnly;
    controls();
    // Unfold the suites that hold the Eval tasks it now shows.
    if (pendingOnly) for (const row of main.querySelectorAll('.trial-row[data-pending="true"]')) row.closest('details.trial-suite')?.setAttribute('open', '');
  }
  else if (target.matches('[data-all-rechecks]')) { allRechecks = !allRechecks; controls(); }
  else if (target.matches('[data-close-peek], .peek-link')) currentPeek = undefined;
  else if (target.matches('[data-copy]')) {
    const status = target.closest('.experiment-callout')?.querySelector('[data-copy-status]');
    try { await navigator.clipboard.writeText(target.dataset.copy); if (status) status.textContent = 'Command copied'; }
    catch { if (status) status.textContent = target.dataset.copy; }
  }
});
document.addEventListener('keydown', event => { if (event.key === 'Escape') currentPeek = undefined; });

async function refresh() {
  if (refreshing) return;
  refreshing = true;
  try {
    const response = await fetch(location.href, { cache: 'no-store' });
    if (!response.ok) throw new Error('Experiment view unavailable');
    const next = new DOMParser().parseFromString(await response.text(), 'text/html');
    const replacement = next.getElementById('experiments');
    if (!replacement) throw new Error('Experiment view unavailable');
    // A rename from the CLI changes the heading and the tab title together.
    document.title = next.title;
    const position = [scrollX, scrollY];
    Idiomorph.morph(main, replacement, { callbacks: { beforeAttributeUpdated(name, node) {
      if (name === 'open' && node.matches('details')) return false;
    } } });
    controls();
    if (currentPeek && !peek.hidden) openPeek(currentPeek, next);
    window.scrollTo(...position);
    document.getElementById('experiment-live-status').textContent = '';
  } catch {
    document.getElementById('experiment-live-status').textContent = 'Updates unavailable. Retrying…';
  } finally { refreshing = false; }
}
const timer = setInterval(refresh, 2000);
window.addEventListener('pagehide', () => clearInterval(timer), { once: true });
