// Folded sections are a per-browser convenience. A fresh visit opens Needs you.
const key = "thirdshift:console:folded";
let folded = {};
try { folded = JSON.parse(localStorage.getItem(key) ?? "{}"); } catch { /* storage is optional */ }
for (const section of document.querySelectorAll("details[data-group]")) {
  if (typeof folded?.[section.dataset.group] === "boolean") section.open = folded[section.dataset.group];
  section.addEventListener("toggle", () => {
    folded ??= {};
    folded[section.dataset.group] = section.open;
    try { localStorage.setItem(key, JSON.stringify(folded)); } catch { /* storage is optional */ }
  });
}
