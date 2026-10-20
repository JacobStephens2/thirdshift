// Browser preferences are conveniences; malformed or unavailable storage hides nothing.
function stored(key, fallback) {
  try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; }
}
function remember(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage is optional */ }
}
const visitKey = "thirdshift:console:last-look";
const visit = stored(visitKey, Date.now());
const lastLook = typeof visit === "number" && Number.isFinite(visit) && visit >= 0 && visit <= Date.now() ? visit : Date.now();
function markNew() {
  for (const row of document.querySelectorAll(".command")) {
    row.querySelector(":scope > .row-head > .new").hidden = Number(row.dataset.startedMs) <= lastLook;
  }
}
markNew();
remember(visitKey, Date.now());
window.addEventListener("pagehide", () => remember(visitKey, Date.now()));
const foldKey = "thirdshift:console:folded";
let folded = stored(foldKey, {});
let selectedRowId;
function selectRow(row) {
  document.querySelector('.command[aria-current="true"]')?.removeAttribute("aria-current");
  selectedRowId = row?.id;
  row?.setAttribute("aria-current", "true");
}
if (typeof folded !== "object" || Array.isArray(folded)) folded = {};
for (const section of document.querySelectorAll("details[data-group]")) {
  if (typeof folded[section.dataset.group] === "boolean") section.open = folded[section.dataset.group];
}
document.addEventListener("toggle", event => {
  const section = event.target;
  if (!section.matches("details[data-group]")) return;
  folded[section.dataset.group] = section.open;
  remember(foldKey, folded);
  const selected = document.getElementById(selectedRowId);
  if (section.open && selected && section.contains(selected)) selected.focus({ preventScroll: true });
}, true);
document.addEventListener("focusin", event => {
  const row = event.target.closest(".command");
  if (row) selectRow(row);
});
document.addEventListener("click", event => {
  event.target.closest(".command")?.focus({ preventScroll: true });
});

function logLengths(row) {
  return row.dataset.logGrowth.split(":").map(Number);
}
function growth() {
  return new Map([...document.querySelectorAll(".command")].map(row => [row.id, logLengths(row)]));
}

function morph(html) {
  const next = new DOMParser().parseFromString(html, "text/html");
  const current = document.getElementById("inbox");
  const replacement = next.getElementById("inbox");
  if (!replacement) throw new Error("Missing Inbox");
  const active = document.activeElement;
  const focused = active?.id || active?.closest("details")?.id;
  const wasSummary = active?.matches("summary");
  const scroll = [...current.querySelectorAll("details")].map(section => [section.id, section.querySelector(".commands").scrollTop]);
  const position = [scrollX, scrollY];
  const selection = selectedRowId;
  const previousGrowth = growth();
  Idiomorph.morph(current, replacement, {
    callbacks: {
      beforeAttributeUpdated(name, node) {
        if (name === "open" && node.matches("details[data-group]")) return false;
      }
    }
  });
  const selected = document.getElementById(focused);
  (wasSummary ? selected?.querySelector("summary") : selected)?.focus({ preventScroll: true });
  // Focus cannot enter a folded section. Keep the logical row selected so
  // it remains selected, and regains focus, when that section is reopened.
  selectRow(document.getElementById(selection));
  for (const [id, top] of scroll) document.getElementById(id).querySelector(".commands").scrollTop = top;
  window.scrollTo(...position);
  document.title = next.title;
  document.querySelector('link[rel="icon"]').href = next.querySelector('link[rel="icon"]').getAttribute("href");
  markNew();
  for (const row of document.querySelectorAll('.command[data-logging="true"]')) {
    const previous = previousGrowth.get(row.id) ?? [0, 0];
    const increased = logLengths(row).some((size, index) => size > previous[index]);
    if (increased && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
      row.querySelector(":scope > .row-head > .heartbeat")?.animate([
        { transform: "scale(1)", opacity: 0.4 },
        { transform: "scale(1.8)", opacity: 1, offset: 0.3 },
        { transform: "scale(1)", opacity: 0.4 }
      ], { duration: 900 });
    }
  }
}

// Coalesce notices, but fetch again if a change arrives during the request.
let refreshing = false, pending = false;
async function refresh() {
  pending = true;
  if (refreshing) return;
  refreshing = true;
  try {
    while (pending) {
      pending = false;
      const response = await fetch("/", { cache: "no-store" });
      if (!response.ok) throw new Error("Inbox unavailable");
      morph(await response.text());
    }
  } catch {
    startPolling();
  } finally { refreshing = false; }
}
let polling;
function startPolling() {
  polling ??= setInterval(refresh, 5000);
}
if (typeof EventSource === "function") {
  const stream = new EventSource("/events");
  stream.addEventListener("commands", refresh);
  stream.addEventListener("error", startPolling);
  stream.addEventListener("open", () => {
    clearInterval(polling);
    polling = undefined;
    refresh();
  });
} else { startPolling(); }
