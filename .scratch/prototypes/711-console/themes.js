// PROTOTYPE (#711): appearance themes for the `console` variant. One layout; a theme is only tokens (colours, fonts,
// shape) and effect switches, applied as CSS custom properties scoped to html[data-variant="console"]. Built-in
// themes, the reader's own themes (made in the editor, kept in this browser), and JSON export and import. A theme
// changes how the page looks and nothing else, so it does nothing the CLI can't.

// ---------- The tokens a theme sets ----------
// Colours, in the editor's groups. Text on a solid ink fill (a tag, a lit cell) is worked out from the fill, black or
// white, whichever reads better, so a theme never has to pair them by hand.
export const COLOR_GROUPS = [
  ["Page", [["bg", "Page"], ["panel", "Panel"], ["panel2", "Panel, raised"], ["selBg", "Selected row"], ["text", "Text"], ["head", "Headings"], ["dim", "Dim text"], ["rule", "Rules"], ["link", "Links"]]],
  ["States", [["urgent", "Urgent ink"], ["urgentText", "Urgent text"], ["look", "Look ink"], ["lookText", "Look text"], ["working", "Working ink"],
    ["workingText", "Working text"], ["landing", "Landing ink"], ["ready", "Ready ink"], ["readyText", "Ready text"], ["done", "Done ink"]]],
  ["Fills", [["proof", "Finished cell"], ["ramp", "Past usual"], ["primary", "Primary button"]]],
  ["Log terminal", [["term", "Background"], ["termText", "Text"], ["termDim", "Dim text"]]],
  ["Sheets", [["sheet", "Paper sheet"], ["sheetText", "Sheet text"]]],
  ["Glows", [["glowWorking", "Working glow"], ["glowUrgent", "Urgent glow"], ["glowLook", "Look glow"]]],
  ["Stage cycle", [["ink1", "Stage ink 1"], ["ink2", "Stage ink 2"], ["ink3", "Stage ink 3"], ["ink4", "Stage ink 4"]]],
];
export const COLOR_KEYS = COLOR_GROUPS.flatMap(([, list]) => list.map(([k]) => k));
export const EFFECTS = [["scanlines", "Scanlines"], ["misreg", "Misregistration"], ["halftone", "Halftone"], ["glow", "Glows"],
  ["colorbar", "Colour bar"], ["crops", "Crop marks"], ["tilt", "Tilted paper"],
  ["pixel", "Pixel borders"], ["dither", "Dither (in place of halftone)"], ["hazard", "Hazard stripes on urgent"], ["beacon", "Warning beacon on Silent and Died"], ["conveyor", "Conveyor belt under the title"], ["bed", "Garden bed under the title"], ["stateedge", "State edge down each row"], ["focus", "Focus on what needs you"],
  ["bevel", "Bevelled metal"], ["rivets", "Rivets on panel corners"], ["belt", "Conveyor belt on what's running"]];
export const FONTS = ["Big Shoulders Display", "JetBrains Mono", "Inter", "IBM Plex Sans", "IBM Plex Mono", "Newsreader", "Fraunces", "Caprasimo", "Nunito", "Silkscreen", "Pixelify Sans", "VT323", "Barlow Condensed", "Titillium Web", "Atkinson Hyperlegible", "system-ui", "Georgia", "ui-monospace"];

const press = {
  id: "press", name: "Press", builtIn: true,
  colors: {
    bg: "#0A0A0A", panel: "#141414", panel2: "#1E1E1E", selBg: "#1E1E1E", text: "#E8E6E1", dim: "#9A978F", rule: "#2A2A2A", link: "#00AEEF",
    urgent: "#EC008C", urgentText: "#FF5CB8", look: "#FFF200", lookText: "#FFF200", working: "#00AEEF", workingText: "#00AEEF",
    landing: "#00AEEF", ready: "#E8E6E1", readyText: "#E8E6E1", done: "#9A978F",
    proof: "#E8E6E1", ramp: "#FFF200", primary: "#FFF200",
    term: "#000000", termText: "#E8E6E1", termDim: "#9A978F", sheet: "#E8E6E1", sheetText: "#111111",
    glowWorking: "#00FFFF", glowUrgent: "#FF00FF", glowLook: "#FFFF00",
    ink1: "#00AEEF", ink2: "#EC008C", ink3: "#FFF200", ink4: "#E8E6E1",
  },
  fonts: { head: "Big Shoulders Display", body: "JetBrains Mono", mono: "JetBrains Mono" },
  type: { headStyle: "display", headCase: "none", labelCase: "uppercase", bodySize: 13 },
  shape: { radius: 0, density: "compact" },
  tags: "filled",
  effects: { scanlines: true, misreg: true, halftone: true, glow: true, colorbar: true, crops: true, tilt: true },
  motion: "full",
};
// Press paper: the same press, printed on paper. Inks darkened where they are text, so every pair stays AA.
const pressPaper = {
  ...press, id: "press-paper", name: "Press paper",
  colors: {
    bg: "#E8E6E1", panel: "#F4F2EE", panel2: "#DCD9D2", selBg: "#DCD9D2", text: "#111111", dim: "#55514A", rule: "#C6C1B6", link: "#006A94",
    urgent: "#EC008C", urgentText: "#A80064", look: "#FFF200", lookText: "#615500", working: "#00AEEF", workingText: "#005A80",
    landing: "#00AEEF", ready: "#111111", readyText: "#111111", done: "#68645C",
    proof: "#FFFFFF", ramp: "#E8D200", primary: "#FFF200",
    term: "#141414", termText: "#E8E6E1", termDim: "#A6A299", sheet: "#FBFAF7", sheetText: "#111111",
    glowWorking: "#00AEEF", glowUrgent: "#EC008C", glowLook: "#E8D200",
    ink1: "#00AEEF", ink2: "#EC008C", ink3: "#E8D200", ink4: "#111111",
  },
  effects: { scanlines: false, misreg: true, halftone: true, glow: false, colorbar: true, crops: true, tilt: true },
};
const plain = {
  id: "plain", name: "Plain", builtIn: true,
  colors: {
    bg: "#FCFCFB", panel: "#FFFFFF", panel2: "#F3F4F6", selBg: "#EEF3FD", text: "#1C1D1F", dim: "#62676E", rule: "#E4E6E9", link: "#1C1D1F",
    urgent: "#C4262E", urgentText: "#C4262E", look: "#9A5B00", lookText: "#9A5B00", working: "#1F6FEB", workingText: "#1A5FCC",
    landing: "#7B4FD4", ready: "#1A7F37", readyText: "#1A7F37", done: "#62676E",
    proof: "#EEF4FD", ramp: "#1F6FEB", primary: "#1C1D1F",
    term: "#F7F8FA", termText: "#1C1D1F", termDim: "#62676E", sheet: "#FFFFFF", sheetText: "#1C1D1F",
    glowWorking: "#1F6FEB", glowUrgent: "#C4262E", glowLook: "#9A5B00",
    ink1: "#AEB4BC", ink2: "#C3C8CE", ink3: "#AEB4BC", ink4: "#C3C8CE",
  },
  fonts: { head: "Inter", body: "Inter", mono: "JetBrains Mono" },
  type: { headStyle: "label", headCase: "uppercase", labelCase: "none", bodySize: 13.5 },
  shape: { radius: 6, density: "compact" },
  tags: "text",
  effects: { scanlines: false, misreg: false, halftone: false, glow: false, colorbar: false, crops: false, tilt: false },
  motion: "full",
};
// Plain dark: inbox's own dark scheme (inbox.css, prefers-color-scheme: dark).
const plainDark = {
  ...plain, id: "plain-dark", name: "Plain dark",
  colors: {
    bg: "#111214", panel: "#15171A", panel2: "#1C1F23", selBg: "#17223A", text: "#E6E7E9", dim: "#9096A0", rule: "#26292E", link: "#E6E7E9",
    urgent: "#FF6B6B", urgentText: "#FF6B6B", look: "#F0B45A", lookText: "#F0B45A", working: "#6AA8FF", workingText: "#6AA8FF",
    landing: "#B39CFF", ready: "#57C97A", readyText: "#57C97A", done: "#9096A0",
    proof: "#1A2230", ramp: "#6AA8FF", primary: "#6AA8FF",
    term: "#0E0F11", termText: "#E6E7E9", termDim: "#9096A0", sheet: "#1B1D21", sheetText: "#E6E7E9",
    glowWorking: "#6AA8FF", glowUrgent: "#FF6B6B", glowLook: "#F0B45A",
    ink1: "#3A4049", ink2: "#4A515B", ink3: "#3A4049", ink4: "#4A515B",
  },
};
// Garden: the garden variant's palette (soil, leaf, cream paper; Fraunces over JetBrains Mono), as plain words like
// every theme. Growing is leaf green, blooming-ready is frost blue, waiting is heart gold, urgent is the stop red.
const gardenLight = {
  id: "garden-light", name: "Garden light", builtIn: true,
  colors: {
    bg: "#F6F0E1", panel: "#FBF7EC", panel2: "#EFE6CF", selBg: "#E6EEDA", text: "#2E2A1F", dim: "#6B6250", rule: "#D9CCAA", link: "#2F6B2A",
    urgent: "#A23B2A", urgentText: "#A23B2A", look: "#F2C14E", lookText: "#7A5200", working: "#4F8A3C", workingText: "#2F6B2A",
    landing: "#A0773F", ready: "#5B8FB9", readyText: "#2F5F86", done: "#6B6250",
    proof: "#FFFAF0", ramp: "#4F8A3C", primary: "#4F8A3C",
    term: "#FFFAF0", termText: "#2E2A1F", termDim: "#6B6250", sheet: "#FFFAF0", sheetText: "#2E2A1F",
    glowWorking: "#6FAE58", glowUrgent: "#E07A5F", glowLook: "#F2C14E",
    ink1: "#4F8A3C", ink2: "#E07A5F", ink3: "#F2C14E", ink4: "#A0773F",
  },
  // Caprasimo (a chunky retro nursery-sign face, one weight) over Nunito (soft and rounded), so the garden doesn't
  // read like a warm serif UI.
  fonts: { head: "Caprasimo", body: "Nunito", mono: "JetBrains Mono" },
  type: { headStyle: "display", headCase: "none", labelCase: "none", bodySize: 14, headWeight: 400 },
  shape: { radius: 8, density: "cozy" },
  tags: "filled",
  effects: { scanlines: false, misreg: false, halftone: false, glow: false, colorbar: false, crops: false, tilt: true, bed: true },
  motion: "full",
};
// Garden dark: the garden variant's night garden (garden.css, prefers-color-scheme: dark).
const gardenDark = {
  ...gardenLight, id: "garden-dark", name: "Garden dark",
  colors: {
    bg: "#161A14", panel: "#1B2018", panel2: "#1E241B", selBg: "#233020", text: "#E8E2CF", dim: "#A59D84", rule: "#36402F", link: "#9BD480",
    urgent: "#EC7A66", urgentText: "#EC7A66", look: "#F2C14E", lookText: "#F2C14E", working: "#6FAE58", workingText: "#8BC970",
    landing: "#C39A5A", ready: "#8EC1E6", readyText: "#8EC1E6", done: "#A59D84",
    proof: "#1E241B", ramp: "#6FAE58", primary: "#8BC970",
    term: "#10130E", termText: "#E8E2CF", termDim: "#A59D84", sheet: "#1E241B", sheetText: "#E8E2CF",
    glowWorking: "#8BC970", glowUrgent: "#EC7A66", glowLook: "#F2C14E",
    ink1: "#6FAE58", ink2: "#F0937A", ink3: "#F2C14E", ink4: "#8A6A3A",
  },
  effects: { ...gardenLight.effects, glow: true },
};
// Factory: an industrial plant in the manner of Factorio. Third shift (the night shift; it's in the name) is the
// game's GUI on gunmetal: cool neutral steel panels, its cream headings, Factorio orange for links and selection. Day
// shift is the same machinery on cool concrete, never warm, so neither reads as Garden. The states are the entity
// status lights: red urgent, yellow look, green working (only on tags and lit stages, never a large fill), orange
// landing, express-belt blue ready. The stage cycle is the belt tiers: yellow, red, blue, then iron.
// Fonts: Barlow Condensed for headings, the condensed caps of machine plates and stencilled signage; Titillium Web for
// body text, the face Factorio itself uses; JetBrains Mono for times and logs. Machined, not pixel: square-ish corners,
// bevelled steel, rivets, belt chevrons and hazard stripes.
const factoryDay = {
  id: "factory-day", name: "Factory day shift", builtIn: true,
  colors: {
    bg: "#E3E6E8", panel: "#EFF1F3", panel2: "#E6E9EB", selBg: "#F6F7F8", text: "#1B1D1F", head: "#1B1D1F", dim: "#3E4246", rule: "#8E9397", link: "#6A3A00",
    urgent: "#A8261C", urgentText: "#7E1912", look: "#E8B33A", lookText: "#4F3B00", working: "#2E7D32", workingText: "#164A1A",
    landing: "#E08A00", ready: "#1F6FA8", readyText: "#0E4A74", done: "#3C4044",
    proof: "#F6F7F8", ramp: "#E8B33A", primary: "#FFA200",
    term: "#1C1D1F", termText: "#E3E3E3", termDim: "#9A9DA2", sheet: "#EFF1F3", sheetText: "#1B1D1F",
    glowWorking: "#5EB663", glowUrgent: "#E03434", glowLook: "#E8B33A",
    ink1: "#D19A1E", ink2: "#B8352A", ink3: "#2F86C4", ink4: "#7E8388",
  },
  fonts: { head: "Barlow Condensed", body: "Titillium Web", mono: "JetBrains Mono" },
  type: { headStyle: "display", headCase: "uppercase", labelCase: "uppercase", bodySize: 14, headWeight: 700, monoScale: 1 },
  shape: { radius: 2, density: "compact" },
  tags: "filled",
  effects: { scanlines: false, misreg: false, halftone: false, glow: false, colorbar: false, crops: false, tilt: false,
    pixel: false, dither: false, hazard: true, beacon: false, bevel: true, rivets: true, belt: true, conveyor: true },
  motion: "full",
};
const factoryNight = {
  ...factoryDay, id: "factory-night", name: "Factory third shift",
  colors: {
    bg: "#1C1D1F", panel: "#2F3033", panel2: "#3D3F42", selBg: "#3B3E43", text: "#E3E3E3", head: "#FFE6C0", dim: "#A3A6AB", rule: "#55575B", link: "#FFA200",
    urgent: "#E03434", urgentText: "#FF6B5E", look: "#E8B33A", lookText: "#F2C14E", working: "#5EB663", workingText: "#7FD07F",
    landing: "#FFA200", ready: "#4AA8E5", readyText: "#5FB4EA", done: "#8F9195",
    proof: "#34363A", ramp: "#E8B33A", primary: "#FFA200",
    term: "#141516", termText: "#E3E3E3", termDim: "#9A9DA2", sheet: "#3D3F42", sheetText: "#E3E3E3",
    glowWorking: "#5EB663", glowUrgent: "#FF3B30", glowLook: "#FFB000",
    ink1: "#E8B33A", ink2: "#D64538", ink3: "#4AA8E5", ink4: "#A0A4A8",
  },
  effects: { ...factoryDay.effects, glow: true, beacon: true },
};
// Signal: built for understanding what thirdshift is doing at a glance. Colour is kept for state alone: panels,
// headings, rules, links and the stage cycle are neutral greys, so any colour on the page says something about a
// Run. The state inks are the Okabe–Ito palette, distinguishable with the common colour-vision deficiencies:
// vermilion urgent, amber look, blue working, reddish purple landing, bluish green ready, grey done; never red
// against green. Every state is said three ways: a filled tag with its word, a coloured edge down the row, the
// colour. Atkinson Hyperlegible (from the Braille Institute) keeps l, I and 1, O and 0 apart in issue numbers and
// ages; headings in sentence case; no effects; motion only for activity.
const signalLight = {
  id: "signal-light", name: "Signal light", builtIn: true,
  colors: {
    bg: "#FAFAFA", panel: "#FFFFFF", panel2: "#F1F2F4", selBg: "#ECEDEF", text: "#111316", head: "#111316", dim: "#4F555C", rule: "#DDE0E3", link: "#111316",
    urgent: "#D55E00", urgentText: "#B3470A", look: "#E69F00", lookText: "#7A4E00", working: "#0072B2", workingText: "#00609A",
    landing: "#9B3F76", ready: "#009E73", readyText: "#00694A", done: "#5A6067",
    proof: "#F4F5F6", ramp: "#0072B2", primary: "#111316",
    term: "#F6F7F8", termText: "#111316", termDim: "#4F555C", sheet: "#FFFFFF", sheetText: "#111316",
    glowWorking: "#0072B2", glowUrgent: "#D55E00", glowLook: "#E69F00",
    ink1: "#9AA0A6", ink2: "#B8BDC2", ink3: "#9AA0A6", ink4: "#B8BDC2",
  },
  fonts: { head: "Atkinson Hyperlegible", body: "Atkinson Hyperlegible", mono: "IBM Plex Mono" },
  type: { headStyle: "display", headCase: "none", labelCase: "none", bodySize: 14.5, headWeight: 700, monoScale: 1 },
  shape: { radius: 4, density: "compact" },
  tags: "filled",
  effects: { scanlines: false, misreg: false, halftone: false, glow: false, colorbar: false, crops: false, tilt: false, stateedge: true },
  motion: "full",
};
const signalDark = {
  ...signalLight, id: "signal-dark", name: "Signal dark",
  colors: {
    bg: "#111315", panel: "#181B1E", panel2: "#20242A", selBg: "#262B31", text: "#ECEEF0", head: "#ECEEF0", dim: "#A4AAB1", rule: "#2C3137", link: "#ECEEF0",
    urgent: "#FF7A33", urgentText: "#FF8A4C", look: "#F0B429", lookText: "#F2B53A", working: "#3FA2E0", workingText: "#62B8F0",
    landing: "#D97DB6", ready: "#2CC79A", readyText: "#3CCB9E", done: "#9AA0A6",
    proof: "#1C2024", ramp: "#3FA2E0", primary: "#ECEEF0",
    term: "#0C0E10", termText: "#ECEEF0", termDim: "#A4AAB1", sheet: "#20242A", sheetText: "#ECEEF0",
    glowWorking: "#3FA2E0", glowUrgent: "#FF7A33", glowLook: "#F0B429",
    ink1: "#5B6168", ink2: "#737A82", ink3: "#5B6168", ink4: "#737A82",
  },
};
// Focus: visual weight follows what needs the Day shift. A calm, near-monochrome page where only what needs a
// decision is loud: urgent work gets the one strong colour (crimson), bold type, a thick edge and a tinted band; look
// items get amber at medium weight; healthy work (Working, Landing, Ready, Done, Resolved) is set in low-chroma slate
// and sage and recedes to the dim text colour until hovered or selected. Receding is done with colour, not opacity, so
// every pair still passes AA. In the grid, lanes and waterfall the healthy bars desaturate, so a silent or slow stage
// stands alone. IBM Plex Sans, sentence-case headings, no effects; the only motion is the alarm on silent work.
const focusLight = {
  id: "focus-light", name: "Focus light", builtIn: true,
  colors: {
    // docs/research/focus-theme-attention.md: urgent text darkened (#9A1424) and dim lightened (#6B6B70) to widen
    // the gap between them; the amber edge darkened to #B06800 so it passes WCAG 1.4.11 (3:1); healthy states
    // (working, landing, ready, done) on a grey ramp, so hue is left to what needs you.
    bg: "#F7F7F5", panel: "#FFFFFF", panel2: "#F0F0EE", selBg: "#EEEEEB", text: "#18181A", head: "#18181A", dim: "#6B6B70", rule: "#E3E3E0", link: "#18181A",
    urgent: "#D7263D", urgentText: "#9A1424", look: "#B06800", lookText: "#7E4C00", working: "#5E5E63", workingText: "#4A4A4F",
    landing: "#6B6B70", ready: "#3F3F44", readyText: "#3F3F44", done: "#6B6B70",
    proof: "#F2F2EF", ramp: "#8A8A90", primary: "#18181A",
    term: "#F4F4F2", termText: "#18181A", termDim: "#5E5E63", sheet: "#FFFFFF", sheetText: "#18181A",
    glowWorking: "#8A8A90", glowUrgent: "#D7263D", glowLook: "#B06800",
    ink1: "#A3A3A8", ink2: "#BDBDC2", ink3: "#A3A3A8", ink4: "#BDBDC2",
  },
  fonts: { head: "IBM Plex Sans", body: "IBM Plex Sans", mono: "IBM Plex Mono" },
  type: { headStyle: "display", headCase: "none", labelCase: "none", bodySize: 14, headWeight: 600, monoScale: 1 },
  shape: { radius: 4, density: "compact" },
  tags: "filled",
  effects: { scanlines: false, misreg: false, halftone: false, glow: false, colorbar: false, crops: false, tilt: false, focus: true },
  motion: "full",
};
const focusDark = {
  ...focusLight, id: "focus-dark", name: "Focus dark",
  colors: {
    bg: "#121214", panel: "#19191C", panel2: "#212125", selBg: "#2A2A2F", text: "#E8E8EA", head: "#E8E8EA", dim: "#A0A0A8", rule: "#2C2C31", link: "#E8E8EA",
    // The research note: urgent text a light tint (#FFB0B5) since saturated red read weaker than dim (APCA) and
    // vibrates on dark; the edge stays saturated (#FF6B78); healthy states on a grey ramp.
    urgent: "#FF6B78", urgentText: "#FFB0B5", look: "#FFB020", lookText: "#FFB52E", working: "#A8A8B0", workingText: "#B4B4BC",
    landing: "#9A9AA2", ready: "#C4C4CA", readyText: "#C4C4CA", done: "#9A9AA2",
    proof: "#1E1E22", ramp: "#6E6E76", primary: "#E8E8EA",
    term: "#0E0E10", termText: "#E8E8EA", termDim: "#A0A0A8", sheet: "#212125", sheetText: "#E8E8EA",
    glowWorking: "#8A8A92", glowUrgent: "#FF6B78", glowLook: "#FFB020",
    ink1: "#55555C", ink2: "#6A6A72", ink3: "#55555C", ink4: "#6A6A72",
  },
};
// The auto themes follow the system's light or dark setting: [light, dark].
const pressAuto = { id: "press-auto", name: "Press auto", builtIn: true, follows: ["press-paper", "press"] };
const plainAuto = { id: "plain-auto", name: "Plain auto", builtIn: true, follows: ["plain", "plain-dark"] };
const gardenAuto = { id: "garden-auto", name: "Garden auto", builtIn: true, follows: ["garden-light", "garden-dark"] };
const focusAuto = { id: "focus-auto", name: "Focus auto", builtIn: true, follows: ["focus-light", "focus-dark"] };
const signalAuto = { id: "signal-auto", name: "Signal auto", builtIn: true, follows: ["signal-light", "signal-dark"] };
const factoryAuto = { id: "factory-auto", name: "Factory auto", builtIn: true, follows: ["factory-day", "factory-night"] };

// The picker shows one row per family, each in the order auto, light, dark.
export const BUILT_IN = [focusAuto, focusLight, focusDark, plainAuto, plain, plainDark, pressAuto, pressPaper, press,
  signalAuto, signalLight, signalDark, gardenAuto, gardenLight, gardenDark, factoryAuto, factoryDay, factoryNight].map((t) => normalize(t));
export const DEFAULT_THEME = "focus-auto"; // Jacob's call: Focus, following the system's light or dark setting

// A theme made before a setting existed gets that setting's neutral value: a new effect off, mono at its own size.
// Built-ins, stored themes and imports all pass through here, so older exports keep importing.
function normalize(t) {
  if (!t || typeof t !== "object" || t.follows) return t;
  t.effects = { ...Object.fromEntries(EFFECTS.map(([k]) => [k, false])), ...(t.effects ?? {}) };
  // Headings took the text colour (display) or the dim colour (label) before they had a colour of their own.
  if (t.colors && !t.colors.head) t.colors = { ...t.colors, head: t.type?.headStyle === "label" ? t.colors.dim : t.colors.text };
  if (t.type && !Number.isFinite(t.type.monoScale)) t.type = { ...t.type, monoScale: 1 };
  return t;
}

// ---------- Storage (a per-viewer convenience: the page works without it) ----------
const KEY_CURRENT = "proto711:console-theme";
const KEY_USER = "proto711:console-themes";
const KEY_HINT = "proto711:console-theme-hint";
const read = (k, dflt) => { try { const v = localStorage.getItem(k); return v == null ? dflt : JSON.parse(v); } catch { return dflt; } };
const write = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* storage blocked: lasts until reload */ } };

let userThemes = read(KEY_USER, []).map(normalize).filter(isValid);
// Themes renamed since a reader may have chosen them: Pixel became Factory.
const RENAMED = { "pixel-auto": "factory-auto", "pixel-day": "factory-day", "pixel-night": "factory-night" };
let currentId = RENAMED[read(KEY_CURRENT, DEFAULT_THEME)] ?? read(KEY_CURRENT, DEFAULT_THEME);
let hintDismissed = read(KEY_HINT, false);
// UI state: the panel, the editor's draft, the import box, and a one-line message.
const ui = { open: false, draft: null, importing: false, message: "" };

export const allThemes = () => [...BUILT_IN, ...userThemes];
const byId = (id) => allThemes().find((t) => t.id === id);
const dark = () => matchMedia("(prefers-color-scheme: dark)").matches;
// The theme in force: the editor's draft while editing, else the chosen one (an auto theme resolved against the system).
export function resolved(t = ui.draft ?? byId(currentId) ?? byId(DEFAULT_THEME)) {
  return t.follows ? byId(t.follows[dark() ? 1 : 0]) : t;
}
export const currentTheme = () => byId(currentId) ?? byId(DEFAULT_THEME);

// ---------- Colour maths: WCAG contrast, and black or white on a fill ----------
function lum(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; })
    .reduce((s, c, i) => s + c * [0.2126, 0.7152, 0.0722][i], 0);
}
export function ratio(a, b) { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); }
const onFill = (hex) => (ratio(hex, "#000000") >= ratio(hex, "#FFFFFF") ? "#000000" : "#FFFFFF");
const HEX = /^#[0-9a-fA-F]{6}$/;

// ---------- Apply: tokens into one <style>, effects and modes onto <html> ----------
function fontStack(f) {
  const generic = /Mono|monospace|VT323/.test(f) ? "ui-monospace, monospace" : /Newsreader|Fraunces|Georgia/.test(f) ? "Georgia, serif" : "system-ui, sans-serif";
  return /^(system-ui|Georgia|ui-monospace)$/.test(f) ? `${f}, ${generic}` : `"${f}", ${generic}`;
}
// A mono font scaled with type.monoScale: an @font-face alias of the same file with size-adjust, since font-size
// can't follow the font. Only fonts whose file is known here can be scaled; any other mono shows at its own size.
const MONO_FILES = { VT323: "https://fonts.gstatic.com/s/vt323/v18/pxiKyp0ihIEF2isfFJXUdVNF.woff2" };
const monoScaled = (t) => (t.type.monoScale ?? 1) !== 1 && MONO_FILES[t.fonts.mono];
function monoFace(t) {
  if (!monoScaled(t)) return "";
  return `@font-face { font-family: "${t.fonts.mono} Scaled"; src: url(${MONO_FILES[t.fonts.mono]}) format("woff2"); size-adjust: ${Math.round(t.type.monoScale * 100)}%; font-display: swap; }`;
}
export function tokensOf(t) {
  const c = t.colors, fx = t.effects, display = t.type.headStyle === "display";
  const glow = (hex) => (fx.glow ? hex : "transparent");
  const v = {
    "--bg": c.bg, "--panel": c.panel, "--panel-2": c.panel2, "--sel-bg": c.selBg, "--text": c.text, "--dim": c.dim, "--rule": c.rule, "--link": c.link,
    "--urgent": c.urgent, "--urgent-text": c.urgentText, "--look": c.look, "--look-text": c.lookText, "--working": c.working, "--working-text": c.workingText,
    "--landing": c.landing, "--ready": c.ready, "--ready-text": c.readyText, "--done": c.done,
    "--proof": c.proof, "--ramp": c.ramp, "--primary": c.primary,
    "--term": c.term, "--term-text": c.termText, "--term-dim": c.termDim, "--sheet": c.sheet, "--sheet-text": c.sheetText,
    "--glow-working": glow(c.glowWorking), "--glow-urgent": glow(c.glowUrgent), "--glow-look": glow(c.glowLook),
    "--ink-1": c.ink1, "--ink-2": c.ink2, "--ink-3": c.ink3, "--ink-4": c.ink4,
    // Text on a solid fill: black or white, whichever reads better on that fill.
    "--on-urgent": onFill(c.urgent), "--on-look": onFill(c.look), "--on-working": onFill(c.working), "--on-landing": onFill(c.landing),
    "--on-ready": onFill(c.ready), "--on-done": onFill(c.done), "--on-proof": onFill(c.proof), "--on-primary": onFill(c.primary),
    "--on-ink-1": onFill(c.ink1), "--on-ink-2": onFill(c.ink2), "--on-ink-3": onFill(c.ink3), "--on-ink-4": onFill(c.ink4),
    "--font-head": fontStack(t.fonts.head), "--font-body": fontStack(t.fonts.body), "--font-mono": monoScaled(t) ? `"${t.fonts.mono} Scaled", ${fontStack(t.fonts.mono)}` : fontStack(t.fonts.mono),
    "--body-size": `${t.type.bodySize}px`,
    "--head-case": t.type.headCase, "--label-case": t.type.labelCase, "--label-tracking": t.type.labelCase === "uppercase" ? ".12em" : "0",
    "--head-size": display ? "18px" : "12px", "--head-needs-size": display ? "24px" : "12px", "--head-weight": String(t.type.headWeight ?? (display ? 800 : 600)),
    "--head-tracking": display ? ".01em" : ".06em", "--head-color": c.head, "--head-needs-color": display ? c.urgentText : c.text,
    "--host-size": display ? "clamp(30px, 3.2vw, 44px)" : "18px", "--host-line": display ? ".9" : "1.2", "--grp-border": "0px", // no coloured bars between sections in any theme (Jacob: they clutter; Plain has none),
    // Compact enough that every section heading fits above the fold on a 14" laptop, as in Plain.
    "--grp-gap": display ? "8px" : "18px", "--grp-after": display ? "0px" : "6px", "--bar-h": display ? "7px" : "0px",
    "--radius": `${t.shape.radius}px`, "--radius-sm": `${Math.min(t.shape.radius, 4)}px`,
    "--row-pad": t.shape.density === "cozy" ? "6px" : "4px", "--row-min": t.shape.density === "cozy" ? "40px" : "36px",
    // A tag holds its glyph on a solid fill, or the glyph alone in the state's colour.
    "--tag-fill": t.tags === "filled" ? "100%" : "0%", "--cell-fill": t.tags === "filled" ? "100%" : "16%",
    "--sl": fx.scanlines ? ".1" : "0", "--mr": fx.misreg ? "1px" : "0px", "--ht": fx.halftone || fx.dither ? ".35" : "0",
    // Dither: a 2×2 checker (2 px cells) where the halftone dots would go.
    ...(fx.dither ? { "--halftone": "conic-gradient(rgba(0,0,0,.6) 25%, transparent 0 50%, rgba(0,0,0,.6) 0 75%, transparent 0) 0 0 / 4px 4px" } : {}),
    "--fx-colorbar": fx.colorbar ? "flex" : "none", "--fx-crops": fx.crops ? "block" : "none", "--crop-color": fx.crops ? c.dim : "transparent",
    "--tilt": fx.tilt ? "1" : "0",
    "--scheme": lum(c.bg) < 0.2 ? "dark" : "light",
  };
  return v;
}
let mql = null;
export function applyTheme(t = resolved()) {
  t = t.follows ? resolved(t) : t;
  const v = tokensOf(t);
  let el = document.getElementById("console-theme");
  if (!el) { el = document.createElement("style"); el.id = "console-theme"; document.head.append(el); }
  el.textContent = `${monoFace(t)} html[data-variant="console"] { color-scheme: ${v["--scheme"]}; ${Object.entries(v).map(([k, x]) => `${k}: ${x};`).join(" ")} }`;
  const h = document.documentElement.dataset;
  h.cTags = t.tags; h.cHead = t.type.headStyle; h.cMotion = t.motion; h.cTilt = t.effects.tilt ? "on" : "off";
  h.cPixel = t.effects.pixel ? "on" : "off"; h.cHazard = t.effects.hazard ? "on" : "off"; h.cBeacon = t.effects.beacon ? "on" : "off";
  h.cBevel = t.effects.bevel ? "on" : "off"; h.cRivets = t.effects.rivets ? "on" : "off"; h.cBelt = t.effects.belt ? "on" : "off";
  h.cConveyor = t.effects.conveyor ? "on" : "off"; h.cBed = t.effects.bed ? "on" : "off"; h.cStateedge = t.effects.stateedge ? "on" : "off"; h.cFocus = t.effects.focus ? "on" : "off";
  // An auto theme re-applies when the system switches between light and dark.
  if (!mql) { mql = matchMedia("(prefers-color-scheme: dark)"); mql.addEventListener("change", () => { if (currentTheme().follows && !ui.draft) applyTheme(); }); }
}

// ---------- Validation and import ----------
function isValid(t) {
  return t && typeof t === "object" && typeof t.name === "string" && t.colors && COLOR_KEYS.every((k) => HEX.test(t.colors[k] ?? ""))
    && t.fonts && ["head", "body", "mono"].every((k) => typeof t.fonts[k] === "string")
    && t.type && ["display", "label"].includes(t.type.headStyle) && ["none", "uppercase"].includes(t.type.headCase) && ["none", "uppercase"].includes(t.type.labelCase)
    && Number.isFinite(t.type.bodySize) && Number.isFinite(t.type.monoScale) && t.type.monoScale >= 0.8 && t.type.monoScale <= 1.6 && t.shape && Number.isFinite(t.shape.radius) && ["compact", "cozy"].includes(t.shape.density)
    && ["filled", "text"].includes(t.tags) && t.effects && EFFECTS.every(([k]) => typeof t.effects[k] === "boolean")
    && ["full", "calm", "none"].includes(t.motion);
}
function exportable(t) {
  const { id, builtIn, ...rest } = resolved(t);
  return { thirdshiftConsoleTheme: 1, ...rest, name: t.name };
}
function newId() { return `user-${Date.now().toString(36)}`; }

// ---------- The header button, hint and panel ----------
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch]));
const swatches = (t) => {
  const c = resolved(t).colors;
  return `<span class="th-sw" aria-hidden="true">${[c.urgent, c.look, c.working, c.ready, c.done].map((x) => `<i style="background:${x}"></i>`).join("")}</span>`;
};
export function themeButtonHTML() {
  const t = currentTheme();
  return `<button type="button" class="th-btn" data-th="toggle" aria-expanded="${ui.open}" aria-haspopup="dialog" title="Change how the Console looks (Shift+T)">
    <span class="th-lbl">Theme:</span> <b>${esc(t.name)}</b> ${swatches(t)} <span aria-hidden="true">▾</span></button>`;
}
export function themeHintHTML() {
  if (hintDismissed) return "";
  return `<p class="th-hint" role="note"><span>${esc(currentTheme().name)} theme · switch looks with <b>Theme ▾</b> (or <kbd>Shift</kbd>+<kbd>T</kbd>)</span>
    <button type="button" class="lnk" data-th="hint-x">Got it</button></p>`;
}
function card(t) {
  const r = resolved(t), c = r.colors, inUse = t.id === currentId && !ui.draft;
  // The card's words in the card's own fonts, inline, so the page's current theme can't leak in (an auto theme shares
  // its family's fonts, so the light one's are its).
  const f = t.follows ? byId(t.follows[0]) : r;
  const nameStyle = `font-family:${esc(fontStack(f.fonts.head))};font-weight:${f.type.headWeight ?? (f.type.headStyle === "display" ? 800 : 600)};text-transform:${f.type.headCase};font-size:${f.type.headStyle === "display" ? 15 : 13}px;letter-spacing:${f.type.headStyle === "display" ? ".01em" : ".06em"}`;
  const subStyle = `font-family:${esc(fontStack(f.fonts.body))};font-weight:400;text-transform:none;letter-spacing:0`;
  const prev = `<span class="th-prev" style="background:${c.bg};color:${c.text};border-color:${c.rule}">
    <span class="th-prev-panel" style="background:${c.panel};border-color:${c.rule}">
      <span style="font-family:${esc(fontStack(r.fonts.head))};font-weight:${r.type.headWeight ?? (r.type.headStyle === "display" ? 800 : 600)}">Aa</span>
      <span style="color:${c.dim}">dim</span></span>
    <span class="th-prev-inks">${[["urgent", c.urgent], ["look", c.look], ["working", c.working], ["ready", c.ready], ["done", c.done]].map(([w, x]) => `<i title="${w}" style="background:${x}"></i>`).join("")}</span></span>`;
  return `<li><button type="button" class="th-card${inUse ? " in-use" : ""}" data-th="use" data-id="${esc(t.id)}" aria-pressed="${inUse}">
    ${prev}<span class="th-card-name" style="${nameStyle}">${esc(t.name)}${inUse ? ` <span class="th-inuse" style="${subStyle}">✓ in use</span>` : ""}</span>
    <span class="th-card-sub" style="${subStyle}">${t.follows ? `follows your system: ${byId(t.follows[0]).name} or ${byId(t.follows[1]).name}` : t.builtIn ? "built in" : "yours"}</span></button></li>`;
}
function contrastHTML(t) {
  const c = resolved(t).colors;
  const pairs = [["Text on page", c.text, c.bg], ["Dim on page", c.dim, c.bg], ["Links on page", c.link, c.bg], ["Text on panel", c.text, c.panel], ["Headings on page", c.head, c.bg],
    ["Urgent text", c.urgentText, c.bg], ["Look text", c.lookText, c.bg], ["Working text", c.workingText, c.bg], ["Ready text", c.readyText, c.bg],
    ["Done", c.done, c.bg], ["Log text", c.termText, c.term], ["Sheet text", c.sheetText, c.sheet]];
  return `<ul class="th-contrast" aria-label="Contrast checks">${pairs.map(([n, a, b]) => {
    const r = ratio(a, b), ok = r >= 4.5;
    return `<li class="${ok ? "ok" : "bad"}"><i style="background:${b};color:${a}">Aa</i> ${n} <b>${r.toFixed(1)}:1</b> <span>${ok ? "AA ✓" : "AA ✗"}</span></li>`;
  }).join("")}</ul>`;
}
function editorHTML() {
  const t = ui.draft;
  const color = (k, label) => `<label class="th-color"><span>${label}</span><input type="color" data-tk="${k}" value="${t.colors[k].toLowerCase()}">
    <input type="text" class="th-hex" data-tk="${k}" value="${t.colors[k]}" maxlength="7" spellcheck="false" aria-label="${label} hex"></label>`;
  const select = (path, label, opts) => {
    const val = path.split(".").reduce((o, k) => o[k], t);
    const font = path.startsWith("fonts.");
    const fs = (v) => (font ? ` style="font-family:${esc(fontStack(v))}"` : "");
    return `<label class="th-field"><span>${label}</span><select data-tp="${path}"${fs(val)}>${opts.map(([v, l]) => `<option value="${esc(v)}"${fs(v)}${String(v) === String(val) ? " selected" : ""}>${esc(l)}</option>`).join("")}</select></label>`;
  };
  return `<form class="th-editor" data-th-form onsubmit="return false">
    <div class="th-ed-head"><b>${t.id && userThemes.some((u) => u.id === t.id) ? "Edit theme" : "New theme"}</b>
      <span class="mu">changes show on the page as you edit</span></div>
    <label class="th-field th-name"><span>Name</span><input type="text" data-tp="name" value="${esc(t.name)}" maxlength="40"></label>
    ${COLOR_GROUPS.map(([g, list], i) => `<details class="th-group"${i < 2 ? " open" : ""}><summary>${g}</summary><div class="th-colors">${list.map(([k, l]) => color(k, l)).join("")}</div></details>`).join("")}
    <details class="th-group" open><summary>Type and shape</summary><div class="th-fields">
      ${select("fonts.head", "Heading font", FONTS.map((f) => [f, f]))}
      ${select("fonts.body", "Body font", FONTS.map((f) => [f, f]))}
      ${select("fonts.mono", "Mono font", FONTS.map((f) => [f, f]))}
      ${select("type.headStyle", "Heading style", [["display", "Display (large)"], ["label", "Label (small)"]])}
      ${select("type.headCase", "Heading case", [["none", "As typed"], ["uppercase", "UPPERCASE"]])}
      ${select("type.labelCase", "Label case", [["uppercase", "UPPERCASE"], ["none", "As typed"]])}
      ${select("type.monoScale", "Mono size", [[1, "As drawn"], [1.15, "1.15×"], [1.25, "1.25×"], [1.35, "1.35×"]])}
      ${select("type.bodySize", "Body size", [[12, "12 px"], [13, "13 px"], [13.5, "13.5 px"], [14, "14 px"], [15, "15 px"]])}
      ${select("shape.radius", "Corners", [[0, "Square"], [4, "4 px"], [6, "6 px"], [10, "10 px"]])}
      ${select("shape.density", "Density", [["compact", "Compact"], ["cozy", "Cozy"]])}
      ${select("tags", "State tags", [["filled", "Filled ink"], ["text", "Coloured glyph"]])}
    </div></details>
    <details class="th-group" open><summary>Effects and motion</summary><div class="th-fields">
      ${EFFECTS.map(([k, l]) => `<label class="th-check"><input type="checkbox" data-tfx="${k}"${t.effects[k] ? " checked" : ""}> ${l}</label>`).join("")}
      ${select("motion", "Motion", [["full", "Full"], ["calm", "Calm: only what carries meaning"], ["none", "None"]])}
      ${matchMedia("(prefers-reduced-motion: reduce)").matches ? `<p class="mu th-note">Your system asks for reduced motion, so nothing moves whatever this says.</p>` : ""}
    </div></details>
    <details class="th-group" open><summary>Contrast</summary><div data-th-contrast>${contrastHTML(t)}</div></details>
    <div class="th-actions">
      <button type="button" class="th-primary" data-th="save">Save</button>
      <button type="button" data-th="cancel-edit">Cancel</button>
    </div></form>`;
}
export function themePanelHTML() {
  if (!ui.open) return "";
  const t = currentTheme();
  const mine = userThemes.some((u) => u.id === t.id);
  const body = ui.draft ? editorHTML() : `
    <ul class="th-cards">${allThemes().map(card).join("")}</ul>
    <div class="th-actions">
      <button type="button" class="th-primary" data-th="new">New theme from this one</button>
      ${mine ? `<button type="button" data-th="edit">Edit</button><button type="button" data-th="duplicate">Duplicate</button><button type="button" data-th="delete">Delete</button>` : `<button type="button" data-th="duplicate">Duplicate</button>`}
      <button type="button" data-th="export">Export JSON</button>
      <button type="button" data-th="import">Import JSON</button>
    </div>
    ${ui.importing ? `<div class="th-import"><label class="th-field"><span>Paste a theme's JSON</span><textarea data-th-import rows="5" spellcheck="false"></textarea></label>
      <div class="th-actions"><button type="button" class="th-primary" data-th="do-import">Import</button><button type="button" data-th="cancel-import">Cancel</button></div></div>` : ""}
    <p class="mu th-note">A theme changes how the Console looks, nothing else. Yours are kept in this browser; Export JSON to keep or share one.</p>`;
  return `<div class="th-panel" role="dialog" aria-label="Themes">
    <div class="th-panel-head"><b>${ui.draft ? "Theme editor" : "Themes"}</b>${ui.message ? `<span class="th-msg" role="status">${esc(ui.message)}</span>` : ""}
      <button type="button" class="x" data-th="close" aria-label="Close themes (esc)">✕</button></div>
    ${body}</div>`;
}

// ---------- Interaction ----------
let rerender = () => {};
export const panelOpen = () => ui.open;
export function toggleThemePanel(open = !ui.open) {
  ui.open = open; ui.message = "";
  if (!open && ui.draft) { ui.draft = null; applyTheme(); }
  rerender();
}
// Esc closes the editor, then the panel. Returns whether it did anything.
export function escapeThemes() {
  if (!ui.open) return false;
  if (ui.draft) { ui.draft = null; ui.message = ""; applyTheme(); } else ui.open = false;
  rerender();
  return true;
}
function use(id) { currentId = id; write(KEY_CURRENT, id); ui.message = `${byId(id).name} in use`; applyTheme(); rerender(); }
function startDraft(from, keepId = false) {
  const base = structuredClone(resolved(from));
  ui.draft = { ...base, id: keepId ? from.id : null, builtIn: false, name: keepId ? from.name : `${from.name} copy` };
  delete ui.draft.follows;
  ui.message = "";
  applyTheme(ui.draft); rerender();
}
function saveDraft() {
  const d = ui.draft;
  d.name = d.name.trim() || "Untitled theme";
  if (!d.id) d.id = newId();
  const { builtIn, ...clean } = d;
  userThemes = [...userThemes.filter((u) => u.id !== d.id), clean];
  write(KEY_USER, userThemes);
  ui.draft = null;
  use(d.id);
}
function download(t) {
  const blob = new Blob([JSON.stringify(exportable(t), null, 2)], { type: "application/json" });
  const a = Object.assign(document.createElement("a"), { href: URL.createObjectURL(blob), download: `${t.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}.thirdshift-theme.json` });
  document.body.append(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  ui.message = "Exported"; rerender();
}
function importText(text) {
  let t;
  try { t = normalize(JSON.parse(text)); } catch { ui.message = "That isn't JSON"; rerender(); return; }
  if (!isValid(t)) { ui.message = "Not a Console theme: a colour, font or setting is missing or out of range"; rerender(); return; }
  const { thirdshiftConsoleTheme, ...rest } = t;
  const theme = { ...rest, id: newId(), builtIn: false };
  userThemes = [...userThemes, theme]; write(KEY_USER, userThemes);
  ui.importing = false;
  use(theme.id);
}
// Updates the live preview and the contrast list without a re-render, so the input being edited keeps its focus.
function updateDraftLive(root) {
  applyTheme(ui.draft);
  const c = root.querySelector("[data-th-contrast]");
  if (c) c.innerHTML = contrastHTML(ui.draft);
}
function setPath(obj, path, value) {
  const keys = path.split("."), last = keys.pop();
  const o = keys.reduce((x, k) => x[k], obj);
  o[last] = typeof o[last] === "number" ? Number(value) : value;
}
export function bindThemes(root, onChange) {
  rerender = onChange;
  applyTheme();
  root.addEventListener("click", (e) => {
    const b = e.target.closest("[data-th]");
    if (!b || !root.contains(b)) return;
    e.preventDefault();
    const a = b.dataset.th, t = currentTheme();
    if (a === "toggle") toggleThemePanel();
    else if (a === "close") toggleThemePanel(false);
    else if (a === "hint-x") { hintDismissed = true; write(KEY_HINT, true); rerender(); }
    else if (a === "use") use(b.dataset.id);
    else if (a === "new" || a === "duplicate") startDraft(t);
    else if (a === "edit") startDraft(t, true);
    else if (a === "delete") {
      userThemes = userThemes.filter((u) => u.id !== t.id); write(KEY_USER, userThemes);
      currentId = DEFAULT_THEME; write(KEY_CURRENT, currentId); ui.message = `Deleted ${t.name}`; applyTheme(); rerender();
    }
    else if (a === "export") download(t);
    else if (a === "import") { ui.importing = true; rerender(); }
    else if (a === "cancel-import") { ui.importing = false; rerender(); }
    else if (a === "do-import") importText(root.querySelector("[data-th-import]")?.value ?? "");
    else if (a === "save") saveDraft();
    else if (a === "cancel-edit") { ui.draft = null; applyTheme(); rerender(); }
  });
  root.addEventListener("input", (e) => {
    const el = e.target;
    if (!ui.draft || !el.closest?.("[data-th-form]")) return;
    if (el.dataset.tk) {
      const v = el.value.trim();
      if (!HEX.test(v)) return; // a half-typed hex waits until it is whole
      ui.draft.colors[el.dataset.tk] = v.toUpperCase();
      for (const twin of root.querySelectorAll(`[data-tk="${el.dataset.tk}"]`)) if (twin !== el) twin.value = twin.type === "color" ? v.toLowerCase() : v.toUpperCase();
    } else if (el.dataset.tp) {
      setPath(ui.draft, el.dataset.tp, el.value);
      if (el.dataset.tp.startsWith("fonts.")) el.style.fontFamily = fontStack(el.value);
    }
    else if (el.dataset.tfx) ui.draft.effects[el.dataset.tfx] = el.checked;
    else return;
    updateDraftLive(root);
  });
}
