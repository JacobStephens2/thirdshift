// Stages owns its browser view choice and horizontal reading position.
export function initStages(main, refresh) {
  const selector = main?.querySelector('#stages-view');
  if (!selector) return null;
  const key = 'thirdshift:console:stages-view';
  let view = 'all';
  try { if (localStorage.getItem(key) === 'repository') view = 'repository'; } catch { /* optional storage */ }

  main.addEventListener('change', event => {
    if (event.target.id !== 'stages-view') return;
    view = event.target.value;
    try { localStorage.setItem(key, view); } catch { /* optional storage */ }
    refresh().catch(console.error);
  });
  main.addEventListener('click', event => {
    const button = event.target.closest('[data-newest-runs]');
    if (button) button.closest('.stage-section').querySelector('.stage-scroll').scrollLeft = 0;
  });
  if (selector.value !== view) queueMicrotask(() => refresh().catch(console.error));
  const findRun = command => [...main.querySelectorAll('.command')].find(node => node.dataset.command === command);
  return {
    url() {
      const url = new URL(location.href);
      url.searchParams.set('view', view);
      return url;
    },
    accepts(replacement) {
      // A slower response to a previous selection must not switch the view back.
      return replacement.querySelector('#stages-view')?.value === view;
    },
    capture() {
      const active = document.activeElement;
      const focusedRun = active?.closest('.command');
      const focus = main.contains(active) ? {
        id: active.id,
        command: focusedRun?.dataset.command,
        href: active.getAttribute('href'),
        grid: active.closest('.stage-section')?.dataset.grid,
        newest: active.matches('[data-newest-runs]')
      } : null;
      const grids = [...main.querySelectorAll('.stage-section')].map(section => {
        const scroll = section.querySelector('.stage-scroll');
        const bounds = scroll.getBoundingClientRect();
        const labelWidth = scroll.querySelector('th').getBoundingClientRect().width;
        const visibleWidth = run => {
          const box = run.getBoundingClientRect();
          return Math.max(0, Math.min(box.right, bounds.right) - Math.max(box.left, bounds.left + labelWidth));
        };
        const visible = [...scroll.querySelectorAll('.command')].filter(run => visibleWidth(run) > 1);
        // Prefer a visible focused/selected run; otherwise keep the column being
        // read, rather than a sliver of its neighbour under the sticky labels.
        const anchor = (
          visible.find(run => run.contains(active)) ?? visible.find(run => run.getAttribute('aria-current') === 'true') ??
          visible.reduce((best, run) => !best || visibleWidth(run) > visibleWidth(best) ? run : best, null)
        );
        return {
          key: section.dataset.grid,
          left: scroll.scrollLeft,
          command: anchor?.dataset.command,
          offset: anchor ? anchor.getBoundingClientRect().left - bounds.left : 0
        };
      });
      const open = new Map([...main.querySelectorAll('details.stage-section')].map(section => [section.dataset.grid, section.open]));
      return { focus, grids, open };
    },
    restore({ focus, grids, open }) {
      // Repository folds stay as the reader left them.
      for (const section of main.querySelectorAll('details.stage-section')) {
        if (open.has(section.dataset.grid)) section.open = open.get(section.dataset.grid);
      }
      for (const saved of grids) {
        const section = [...main.querySelectorAll('.stage-section')].find(node => node.dataset.grid === saved.key);
        const scroll = section?.querySelector('.stage-scroll');
        if (!scroll) continue;
        const anchor = findRun(saved.command);
        if (anchor && scroll.contains(anchor)) {
          scroll.scrollLeft += anchor.getBoundingClientRect().left - scroll.getBoundingClientRect().left - saved.offset;
        } else scroll.scrollLeft = saved.left;
      }
      if (!focus) return;
      let target;
      if (focus.id) target = document.getElementById(focus.id);
      else if (focus.command) {
        const run = findRun(focus.command);
        target = focus.href ? [...(run?.querySelectorAll('a') ?? [])].find(link => link.getAttribute('href') === focus.href) : run;
      } else {
        const section = [...main.querySelectorAll('.stage-section')].find(node => node.dataset.grid === focus.grid);
        target = section?.querySelector(focus.newest ? '[data-newest-runs]' : '.stage-scroll');
      }
      target?.focus({ preventScroll: true });
    }
  };
}
