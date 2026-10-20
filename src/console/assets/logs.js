import { activity } from './look.js';
const content = document.querySelector("#log-content");
const follow = document.querySelector("#follow-log");
const status = document.querySelector("#log-status");
const scroll = () => { if (follow.checked) window.scrollTo(0, document.documentElement.scrollHeight); };
follow.addEventListener("change", scroll);
const stream = new EventSource(content.dataset.stream);
stream.addEventListener("lines", event => {
  const batch = JSON.parse(event.data);
  if (batch.offset > Number(content.dataset.offset)) {
    const fragment = document.createElement("template");
    fragment.innerHTML = batch.html;
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
  if (batch.done) stream.close();
});
stream.addEventListener("log-error", event => { status.textContent = event.data; stream.close(); });
stream.onerror = () => { status.textContent = "Connection lost; reconnecting…"; };
window.addEventListener("pagehide", () => stream.close());
scroll();
