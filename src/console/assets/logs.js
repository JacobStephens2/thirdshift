import { activity } from './look.js';
const content = document.querySelector("#log-content");
const follow = document.querySelector("#follow-log");
const status = document.querySelector("#log-status");
const scroll = () => { if (follow.checked) window.scrollTo(0, document.documentElement.scrollHeight); };
follow.addEventListener("change", scroll);
let stream, polling, pollingRequest;
let stopped = false;
function stop() {
  stopped = true;
  stream?.close();
  clearInterval(polling);
  pollingRequest?.abort();
}
function append(batch) {
  if (stopped) return;
  if (batch.offset > Number(content.dataset.offset)) {
    const fragment = document.createElement("template");
    fragment.innerHTML = batch.html;
    // Retained Codex evidence arrives after the streamed file_change item.
    // Refine that edit in place so an open reader loses its placeholder too.
    for (const edit of fragment.content.querySelectorAll('[data-codex-edit]')) {
      const previous = [...content.querySelectorAll('[data-codex-edit]')]
        .find(node => node.dataset.codexEdit === edit.dataset.codexEdit);
      if (previous) {
        previous.replaceChildren(...edit.childNodes);
        edit.remove();
      }
    }
    for (const group of fragment.content.querySelectorAll('.tool-group')) {
      if (!group.querySelector('.tools').children.length) group.remove();
    }
    for (const message of fragment.content.querySelectorAll('.message[data-message-id]')) {
      const current = [...content.querySelectorAll('.message[data-message-id]')]
        .find(node => node.dataset.messageId === message.dataset.messageId);
      if (current) {
        // Normalized messages with stable IDs are cumulative snapshots. Keep
        // the existing article while replacing its escaped text and role.
        current.replaceChildren(...message.childNodes);
        message.remove();
      }
    }
    const last = content.lastElementChild;
    const first = fragment.content.firstElementChild;
    // A tool sequence crossing a stream batch remains one folded group. Existing
    // nodes (and the reader's open/closed choices) are never replaced.
    if (last?.classList.contains("tool-group") && first?.classList.contains("tool-group")) {
      last.querySelector(".tools").append(...first.querySelector(".tools").childNodes);
      first.remove();
    }
    content.append(fragment.content);
    content.dataset.offset = batch.offset;
    scroll();
  }
  status.textContent = batch.done ? "Complete" : "Live";
  content.dataset.live = String(!batch.done);
  activity();
  if (batch.done) stop();
}
async function poll() {
  if (stopped || pollingRequest) return;
  pollingRequest = new AbortController();
  try {
    const offset = content.dataset.offset;
    const path = content.dataset.stream.replace(/\d+$/, offset);
    const response = await fetch(path, { headers: { accept: 'application/json' }, cache: 'no-store', signal: pollingRequest.signal });
    if (!response.ok || response.redirected) throw new Error('Log unavailable');
    const batch = await response.json();
    // A recovered stream may already have appended part of this batch.
    if (offset === content.dataset.offset) append(batch);
  } catch (error) {
    if (!stopped && error.name !== 'AbortError') status.textContent = 'Connection lost; retrying…';
  } finally { pollingRequest = undefined; }
}
function startPolling() {
  if (stopped) return;
  status.textContent = 'Connection lost; checking for new lines…';
  polling ??= setInterval(poll, 5000);
  poll();
}
function connect() {
  const offset = content.dataset.offset;
  const source = new EventSource(content.dataset.stream.replace(/\d+$/, offset));
  stream = source;
  source.addEventListener('lines', event => {
    if (source === stream) append(JSON.parse(event.data));
  });
  source.addEventListener('open', () => {
    if (stopped || source !== stream) return;
    clearInterval(polling);
    polling = undefined;
    pollingRequest?.abort();
    // Polling may have advanced beyond this connection's starting offset.
    // Resume with a fresh stream so replayed batches cannot overlap the DOM.
    if (offset !== content.dataset.offset) {
      source.close();
      connect();
    }
  });
  source.addEventListener('log-error', event => {
    if (source === stream) { status.textContent = event.data; stop(); }
  });
  source.onerror = () => { if (source === stream) startPolling(); };
}
if (typeof EventSource === 'function') connect();
else startPolling();
window.addEventListener('pagehide', stop);
scroll();
