// PROTOTYPE (#711, Q27): the variant switcher. ?variant= picks the look; ← and → (or the bar) cycle through them.
import { state, onChange, signIn, signOut, needsYou } from "./shared.js";

const VARIANTS = [
  // Ranked, best first: the themed Console, the Console home in both looks, the stage lanes in both looks, the grid page, then the rest.
  { key: "console", name: "Console, with themes", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;500;700&family=Inter:wght@400;500;600;700&family=IBM+Plex+Sans:wght@400;600&family=IBM+Plex+Mono:wght@400;600&family=Newsreader:opsz,wght@6..72,400;6..72,600&family=Fraunces:opsz,wght@9..144,400;9..144,700&family=Caprasimo&family=Nunito:wght@400;600;700;800&family=Barlow+Condensed:wght@600;700&family=Titillium+Web:wght@400;600;700&family=Atkinson+Hyperlegible:ital,wght@0,400;0,700;1,400&family=Silkscreen:wght@400;700&family=Pixelify+Sans:wght@400;500;600;700&family=VT323" },
  { key: "inboxpress", name: "Inbox, pressplain look", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;700" },
  { key: "inbox", name: "Research: agent inbox", fonts: "family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;600" },
  { key: "lanes", name: "Surprise: stage lanes", fonts: "family=IBM+Plex+Sans+Condensed:wght@400;600&family=JetBrains+Mono:wght@400;700" },
  { key: "lanespress", name: "Stage lanes, press floor look", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;700" },
  { key: "gridpress", name: "Stage grid, thirdshift look", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;700" },
  { key: "pressplain", name: "Press look, plain words", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;700" },
  { key: "briefingwide", name: "Briefing, wide", fonts: "family=Newsreader:ital,opsz,wght@0,6..72,400;0,6..72,600;1,6..72,400&family=JetBrains+Mono:wght@400;700" },
  { key: "briefing", name: "Idea: the Day shift briefing", fonts: "family=Newsreader:ital,opsz,wght@0,6..72,400;0,6..72,600;1,6..72,400&family=JetBrains+Mono:wght@400;700" },
  { key: "trace", name: "Research: trace waterfall", fonts: "family=Inter:wght@400;600;700&family=JetBrains+Mono:wght@400;700" },
  { key: "grid", name: "Research: stage grid", fonts: "family=IBM+Plex+Sans:wght@400;600&family=IBM+Plex+Mono:wght@400;600" },
  { key: "accent", name: "Plain, garden accent", fonts: "family=Inter:wght@400;600;700&family=JetBrains+Mono:wght@400;700" },
  { key: "press", name: "Press floor", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;700" },
  { key: "presslayout", name: "Press floor, pixel layout, no pixel art", fonts: "family=Big+Shoulders+Display:wght@800&family=JetBrains+Mono:wght@400;700" },
  { key: "plain", name: "Plain and informational", fonts: "family=Inter:wght@400;600;700&family=JetBrains+Mono:wght@400;700" },
  { key: "garden", name: "Garden, fully themed", fonts: "family=Fraunces:opsz,wght@9..144,400;9..144,700&family=JetBrains+Mono:wght@400;700" },
  { key: "pixelpress", name: "Press floor, pixel art", fonts: "family=Pixelify+Sans:wght@400;700&family=VT323" },
  { key: "pixel", name: "Garden, pixel art", fonts: "family=Pixelify+Sans:wght@400;700&family=VT323" },
];
// A variant that fails to load shows its error instead of taking the others down with it.
const modules = Object.fromEntries(await Promise.all(VARIANTS.map(async (v) => [v.key, await import(`./variants/${v.key}.js`)
  .catch((err) => ({ render: (root) => {
    // A variant still being built has no file yet; say so plainly rather than showing the fetch error.
    const building = /Failed to fetch|404/.test(String(err));
    root.innerHTML = `<p style="padding:24px 16px;font:15px/1.5 system-ui,sans-serif">${building
      ? `<b>${v.name}</b> is still being built. Reload in a minute or two.`
      : `<b>${v.key}</b> failed to load: ${String(err).replace(/</g, "&lt;")}`}</p>`;
  } }))])));

let root = document.getElementById("app");
const label = document.querySelector(".proto-label");
const auth = document.querySelector(".proto-auth");
let current = VARIANTS.findIndex((v) => v.key === new URLSearchParams(location.search).get("variant"));
if (current < 0) current = 0;

function render() {
  const v = VARIANTS[current];
  document.documentElement.dataset.variant = v.key;
  document.getElementById("variant-css").href = `variants/${v.key}.css`;
  document.getElementById("variant-fonts").href = `https://fonts.googleapis.com/css2?${v.fonts}&display=swap`;
  label.textContent = `${current + 1}/${VARIANTS.length} · ${v.name}`;
  auth.textContent = state.signedIn ? "Show sign-in" : "Skip sign-in";
  const y = scrollY;
  // A fresh element each render, so a variant's listeners never stack.
  const fresh = root.cloneNode(false);
  root.replaceWith(fresh);
  root = fresh;
  modules[v.key].render(root);
  if (state.signedIn) { openLogOnChoice(v.key); collapsible(root, v.key); folds(root, v.key); sectionsMenu(root, v.key); }
  attentionOutside();
  scrollTo(0, y);
}

// Every section starts closed; its heading opens and closes it. Variants mark sections up differently, so a
// section is its h2 (or the head block holding it) plus the siblings that follow, up to the next heading.
// Open sections stay open across re-renders and variant switches.
const opened = new Set();
let lastOpened = null; // the section the reader opened most recently, which z folds first
const sectionSeen = new Set(); // sections the page has met, so a default-open one the reader closed stays closed
// Choosing a Session log or Command log opens the section that shows it, wherever the variant put it.
let shownLog = `${state.openSession}|${state.openCommandLog}`;
let openLogSection = null;
function openLogOnChoice(variant) {
  const now = `${state.openSession}|${state.openCommandLog}`;
  openLogSection = now !== shownLog ? variant : null;
  shownLog = now;
}
function collapsible(root, variant) {
  for (const h2 of root.querySelectorAll("h2")) {
    let head = h2;
    while (head.parentElement !== root && (head.parentElement.tagName === "HEADER" || /head|title/i.test(head.parentElement.className))
      && head.parentElement.querySelectorAll("h2").length === 1) head = head.parentElement;
    const body = [];
    for (let n = head.nextElementSibling; n && !(n.matches("h2") || n.querySelector(":scope > h2, :scope > [class*=head] > h2")); n = n.nextElementSibling) body.push(n);
    if (!body.length) continue;
    const key = `${variant}:${h2.textContent.replace(/\d+/g, "").trim()}`;
    // data-section-default="open": the section starts open the first time the page meets it (Needs you, say),
    // and after that keeps whatever the reader chose.
    if (!sectionSeen.has(key) && h2.dataset.sectionDefault === "open") opened.add(key);
    sectionSeen.add(key);
    head.classList.add("proto-toggle");
    h2.setAttribute("role", "button");
    h2.tabIndex = 0;
    const set = (open) => {
      head.classList.toggle("is-open", open);
      h2.setAttribute("aria-expanded", open);
      body.forEach((n) => n.classList.toggle("proto-shut", !open));
      // A log opened from closed starts at its newest line, and a timeline at "now".
      if (open) for (const sc of head.parentElement.querySelectorAll("[data-scroll-end]")) sc.scrollLeft = sc.scrollWidth;
      if (open) for (const sl of head.parentElement.querySelectorAll(".sl")) {
        for (let sc = sl.parentElement; sc && sc !== root; sc = sc.parentElement)
          if (sc.scrollHeight > sc.clientHeight && getComputedStyle(sc).overflowY !== "visible") { sc.scrollTop = sc.scrollHeight; break; }
      }
    };
    if (openLogSection === variant && body.some((n) => n.matches(".sl, .sl-bar, .sl-empty, .stderr-lines") || n.querySelector(".sl, .sl-bar, .sl-empty, .stderr-lines"))) opened.add(key);
    set(opened.has(key));
    head.dataset.sectionKey = key;
    const toggle = (e) => {
      if (e.target.closest("a, button, input")) return;
      opened.has(key) ? opened.delete(key) : opened.add(key);
      set(opened.has(key));
      if (opened.has(key)) { reveal(body); lastOpened = key; }
      layoutChanged(root);
    };
    head.addEventListener("click", toggle);
    h2.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggle(e); } });
  }
}
function go(step) {
  current = (current + step + VARIANTS.length) % VARIANTS.length;
  const u = new URL(location.href);
  u.searchParams.set("variant", VARIANTS[current].key);
  history.replaceState(null, "", u);
  render();
}
document.querySelectorAll(".proto-bar [data-step]").forEach((b) => b.addEventListener("click", () => go(Number(b.dataset.step))));
auth.addEventListener("click", () => (state.signedIn ? signOut() : signIn()));
addEventListener("keydown", (e) => {
  if (e.target.closest?.("input, textarea, [contenteditable]") || e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.key === "ArrowLeft") go(-1);
  if (e.key === "ArrowRight") go(1);
});

// Drill-down folds inside a section (at most two levels deep, after NN/g). A variant marks a toggle with data-fold="<key>", and anything that shows
// only once that toggle is open with data-fold-in="<key> <ancestor keys>…". An element shows when every key it
// lists is open, so closing a Command hides its grandchildren too, whether the markup nests them or lays them
// out as siblings (as the lanes do). Every fold starts closed and stays as the reader left it across re-renders.
const unfolded = new Set();
const foldSeen = new Set(); // keys the page has met, so a default-open fold the reader closed stays closed
function folds(root, variant) {
  const k = (key) => `${variant}:${key}`;
  // data-fold-default="open": a failure, say, opens the first time it appears (GitHub opens failed steps).
  for (const t of root.querySelectorAll("[data-fold]")) {
    const key = k(t.dataset.fold);
    if (!foldSeen.has(key) && t.dataset.foldDefault === "open") unfolded.add(key);
    foldSeen.add(key);
  }
  const apply = () => {
    for (const el of root.querySelectorAll("[data-fold-in]"))
      el.classList.toggle("proto-folded", !el.dataset.foldIn.split(" ").filter(Boolean).every((key) => unfolded.has(k(key))));
    for (const t of root.querySelectorAll("[data-fold]")) {
      const open = unfolded.has(k(t.dataset.fold));
      t.classList.toggle("is-unfolded", open);
      t.setAttribute("aria-expanded", open);
    }
  };
  for (const t of root.querySelectorAll("[data-fold]")) {
    if (!t.querySelector(":scope .fold-caret")) t.insertAdjacentHTML("afterbegin", '<span class="fold-caret" aria-hidden="true"></span>');
    if (!t.matches("button, a")) { t.tabIndex = 0; t.setAttribute("role", "button"); }
    const toggle = (e) => {
      // Links and buttons inside a toggle (an issue link, Stop) keep their own job.
      const hit = e.target.closest("a, button, input, summary");
      if (hit && hit !== t && t.contains(hit)) return;
      e.stopPropagation();
      const key = k(t.dataset.fold);
      const before = new Set(root.querySelectorAll("[data-fold-in]:not(.proto-folded)"));
      unfolded.has(key) ? unfolded.delete(key) : unfolded.add(key);
      apply();
      reveal([...root.querySelectorAll("[data-fold-in]:not(.proto-folded)")].filter((el) => !before.has(el)));
      layoutChanged(root);
    };
    t.addEventListener("click", toggle);
    t.addEventListener("keydown", (e) => { if ((e.key === "Enter" || e.key === " ") && e.target === t) { e.preventDefault(); toggle(e); } });
  }
  apply();
}

// Folding or opening something moves what's below it without a re-render; anything drawn over the page by position
// (a selection bar, say) listens for this and redraws.
function layoutChanged(root) { root.dispatchEvent(new CustomEvent("proto:layout", { bubbles: true })); }

// What the reader just opened slides in, one after another; a re-render shows things still.
function reveal(els) {
  els.forEach((el, i) => {
    el.style.setProperty("--reveal-delay", `${Math.min(i, 8) * 35}ms`);
    el.classList.remove("proto-reveal");
    void el.offsetWidth;
    el.classList.add("proto-reveal");
    el.addEventListener("animationend", () => el.classList.remove("proto-reveal"), { once: true });
  });
}

// Attention leaves the page (research: software-factory-console-trends.md, T6): the tab title counts what needs
// the Day shift, and the tab icon gets a dot while anything urgent waits. Read-only, the same in every variant.
function attentionOutside() {
  const items = state.signedIn ? needsYou() : [];
  const urgent = items.some((i) => i.level === "urgent");
  document.title = `${items.length ? `${items.length} need you · ` : ""}thirdshift Console · PROTOTYPE #711`;
  const dot = urgent ? '<circle cx="25" cy="7" r="6" fill="%23e5484d" stroke="white" stroke-width="2"/>' : "";
  let link = document.querySelector('link[rel="icon"]');
  if (!link) { link = document.createElement("link"); link.rel = "icon"; document.head.append(link); }
  link.href = `data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="3" y="5" width="22" height="22" rx="5" fill="%23222"/><path d="M9 12h10M9 17h10M9 22h6" stroke="white" stroke-width="2.4" stroke-linecap="round"/>${dot}</svg>`;
}
setInterval(attentionOutside, 5000);

// Which sections show. A variant opts in by rendering an empty element with data-sections-menu where the control
// belongs (its header, say); this fills it with a checkbox per section and hides the unticked ones. The choice is
// a per-viewer convenience, so it lives in this browser's storage, and the page works the same without it.
const hiddenKey = (variant) => `proto711:hidden-sections:${variant}`;
function loadHidden(variant) {
  try { return new Set(JSON.parse(localStorage.getItem(hiddenKey(variant)) ?? "[]")); } catch { return new Set(); }
}
function saveHidden(variant, set) {
  try { localStorage.setItem(hiddenKey(variant), JSON.stringify([...set])); } catch { /* storage blocked: the choice lasts until reload */ }
}
const hiddenSections = {};
let sectionsMenuOpen = false;
function sectionsMenu(root, variant) {
  const slot = root.querySelector("[data-sections-menu]");
  if (!slot) return;
  const hidden = (hiddenSections[variant] ??= loadHidden(variant));
  // A section's name is its heading's own first words, without counts or the eyebrow beside them.
  const sections = [...root.querySelectorAll("h2")].map((h2) => {
    const name = h2.dataset.sectionName ?? [...h2.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim())?.textContent.trim() ?? h2.textContent.trim();
    const box = h2.closest("section");
    const whole = box && box.querySelectorAll("h2").length === 1 ? [box] : null;
    if (whole) return { name, els: whole };
    // Otherwise the section is its head and the siblings after it, up to the next heading (as collapsible() reads it).
    const head = h2.closest(".proto-toggle") ?? h2, els = [head];
    for (let n = head.nextElementSibling; n && !(n.matches("h2") || n.querySelector("h2")); n = n.nextElementSibling) els.push(n);
    return { name, els };
  });
  for (const sec of sections) for (const el of sec.els) el.classList.toggle("proto-hidden", hidden.has(sec.name));
  const shown = sections.filter((x) => !hidden.has(x.name)).length;
  slot.innerHTML = `<details class="sections-menu"${sectionsMenuOpen ? " open" : ""}>
    <summary>Sections <span class="sm-count">${shown}/${sections.length}</span></summary>
    <div class="sm-panel" role="group" aria-label="Sections to show">
      ${sections.map((x) => `<label><input type="checkbox" data-section="${x.name.replace(/"/g, "&quot;")}"${hidden.has(x.name) ? "" : " checked"}> ${x.name}</label>`).join("")}
      <button type="button" class="sm-all"${hidden.size ? "" : " disabled"}>Show all</button>
    </div></details>`;
  const menu = slot.firstElementChild;
  menu.addEventListener("toggle", () => { sectionsMenuOpen = menu.open; });
  menu.addEventListener("change", (e) => {
    const name = e.target.dataset.section;
    if (name == null) return;
    e.target.checked ? hidden.delete(name) : hidden.add(name);
    saveHidden(variant, hidden);
    render();
  });
  menu.querySelector(".sm-all").addEventListener("click", () => { hidden.clear(); saveHidden(variant, hidden); render(); });
}

// z folds the section you're in: the one opened most recently, while it's still open; otherwise the open section at
// the top of the screen. Its heading is then brought into view, so the reader keeps their place.
addEventListener("keydown", (e) => {
  if (e.key !== "z" || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || !state.signedIn) return;
  if (e.target.closest?.("input, textarea, select, [contenteditable]")) return;
  const heads = [...root.querySelectorAll(".proto-toggle.is-open")].filter((h) => h.offsetParent);
  if (!heads.length) return;
  const recent = heads.find((h) => h.dataset.sectionKey === lastOpened);
  const atTop = heads.filter((h) => h.getBoundingClientRect().top <= innerHeight * 0.4).at(-1) ?? heads[0];
  const head = recent ?? atTop;
  e.preventDefault();
  (head.matches("h2") ? head : head.querySelector("h2")).click();
  lastOpened = null;
  if (head.getBoundingClientRect().top < 0) head.scrollIntoView({ block: "start" });
});

// ? lists every shortcut in words, for variants that export KEYS. It lives outside the variant's root, so a
// re-render leaves it be; esc or ? closes it.
const variantKeys = () => modules[VARIANTS[current].key].KEYS;
function keysHelp(open) {
  document.querySelector(".proto-keys")?.remove();
  if (!open || !variantKeys()) return;
  const rows = variantKeys().map(([k, what]) => `<tr><th scope="row">${k.split(/\s{2,}/).map((p) => (p === "then" ? `<span class="pk-then">then</span>` : p.split(" ").map((x) => `<kbd>${x}</kbd>`).join(" + "))).join(" ")}</th><td>${what}</td></tr>`).join("");
  document.body.insertAdjacentHTML("beforeend", `<div class="proto-keys" role="dialog" aria-label="Keyboard shortcuts">
    <div class="pk-box"><div class="pk-head"><b>Keyboard shortcuts</b><button type="button" class="pk-x" aria-label="Close">×</button></div>
    <table>${rows}</table><p class="pk-note">← and → switch prototype variants (the red bar), not part of the Console.</p></div></div>`);
  const box = document.querySelector(".proto-keys");
  box.addEventListener("click", (e) => { if (e.target === box || e.target.closest(".pk-x")) keysHelp(false); });
}
addEventListener("keydown", (e) => {
  if (e.target.closest?.("input, textarea, select, [contenteditable]") || e.metaKey || e.ctrlKey || e.altKey) return;
  const open = !!document.querySelector(".proto-keys");
  if (e.key === "?") { e.preventDefault(); keysHelp(!open); }
  else if (e.key === "Escape" && open) { e.preventDefault(); e.stopImmediatePropagation(); keysHelp(false); }
}, true);

// Last, once every helper above is defined.
onChange(render);
render();
