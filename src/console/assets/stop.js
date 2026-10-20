import './look.js';
import { watchCommands } from './changes.js';

watchCommands(async () => {
  const response = await fetch(location.href, { cache: 'no-store' });
  if (!response.ok || response.redirected) throw new Error('Stop unavailable');
  const next = new DOMParser().parseFromString(await response.text(), 'text/html');
  const replacement = next.getElementById('stop-state');
  if (!replacement) throw new Error('Stop unavailable');
  const current = document.getElementById('stop-state');
  if (current.innerHTML !== replacement.innerHTML) current.replaceWith(replacement);
});
