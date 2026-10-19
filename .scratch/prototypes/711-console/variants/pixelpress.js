// PROTOTYPE variant "pixelpress": the pixel variant's layout, moved from a garden to a press floor and inked with
// thirdshift.app's process colours. Live Commands are press lines on a checker-plate floor, Runs and Tickets are
// press units and sheets, choosing one opens an RPG dialogue with a menu, the open log is a job log window, and
// history is an inventory of delivered sheets. Every sprite is a string map drawn as crisp SVG rects; every state
// still carries its plain word beside the drawing.
import { host, sessions, RUN_STAGES } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, status, isLive, liveCommands, attentionCommands, endedCommands, runsOf,
  stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog, mountLog, commandLogText,
  command, changed, canStop, stopEffectsHTML, usualText, heartbeat, health, needsYou, usageText, sessionsOf, summaryText,
} from "../shared.js";

// No window open on load, like the sections; choosing a unit or a delivered slot opens one.
let sel = {};
let confirming = null;
let scrollTarget = null;
let popNext = false; // the dialogue pops in only when a unit or slot is newly chosen

// ---------- Sprites ----------
// Press-floor inks from site/styles.css, with a shade or two of each: blacks and steel, paper, cyan, magenta,
// yellow. "." is transparent; R is a per-sprite override (a running unit's ink fountain, a defect's shell).
const PAL = { k: "#0A0A0A", h: "#141414", m: "#1E1E1E", s: "#2A2A2A", d: "#55524C", e: "#9A978F", w: "#E8E6E1",
  q: "#B9B6AE", c: "#00AEEF", b: "#00719C", l: "#7FD8F7", p: "#EC008C", r: "#FF5CB8", x: "#8A0052", y: "#FFF200",
  o: "#B8AE00", R: "#00AEEF" };

function rects(map, pal) {
  const rows = map.trim().split("\n").map((r) => r.trim());
  const w = Math.max(...rows.map((r) => r.length)), h = rows.length;
  let body = "";
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length;) {
      const ch = row[x];
      let x2 = x + 1;
      while (x2 < row.length && row[x2] === ch) x2++;
      if (ch !== "." && pal[ch]) body += `<rect x="${x}" y="${y}" width="${x2 - x}" height="1" fill="${pal[ch]}"/>`;
      x = x2;
    }
  });
  return { w, h, body };
}
// Sized by whole pixels: 3× normal, 2× small, 4× large. The width and height attributes hold that size even
// without the stylesheet; CSS (--px, --px-sm, --px-lg) only steps it down a whole pixel on a phone.
function spr(map, cls = "", over = {}) {
  const { w, h, body } = rects(map, { ...PAL, ...over });
  const k = /\blg\b/.test(cls) ? 4 : /\bsm\b/.test(cls) ? 2 : 3;
  return `<svg class="spr ${cls}" style="--sw:${w};--sh:${h}" width="${w * k}" height="${h * k}" viewBox="0 0 ${w} ${h}" shape-rendering="crispEdges" aria-hidden="true">${body}</svg>`;
}
function tileURL(map) {
  const { w, h, body } = rects(map, PAL);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" shape-rendering="crispEdges">${body}</svg>`;
  return `url('data:image/svg+xml,${encodeURIComponent(svg)}')`;
}

// A press unit: the ink fountain (R) on top, two rollers, the sheet in the body, a belt underneath. The sheet row
// changes with the stage, more ink on it the further the Run has got.
const unit = (sheet) => `
................
.kkkkkkkkkkkkkk.
.kRRRRRRRRRRRRk.
.kkkkkkkkkkkkkk.
.ks.kkk..kkk.sk.
.ks.kek..kek.sk.
.ks.kkk..kkk.sk.
.kssssssssssssk.
.ksdkkkkkkkkdsk.
${sheet}
.ksdkkkkkkkkdsk.
.kssssssssssssk.
kkkkkkkkkkkkkkkk
kdkdkdkdkdkdkdkd
kkkkkkkkkkkkkkkk
.kk..........kk.`;
const UNIT_BLANK = unit(".ksdkwwwwwwkdsk.");
const UNIT_HALF = unit(".ksdkRwRwRwkdsk.");
const UNIT_FULL = unit(".ksdkRRRRRRkdsk.");
const UNIT_IDLE = unit(".ksdkhhhhhhkdsk.");
// Blank sheets stacked in the feeder: not started.
const FEEDER = `
................
................
................
................
................
................
................
...kkkkkkkkkk...
...kwwwwwwwwk...
...kkkkkkkkkk...
...kwwwwwwwwk...
...kkkkkkkkkk...
...kwwwwwwwwk...
..kkkkkkkkkkkk..
..kssssssssssk..
..kkkkkkkkkkkk..`;
// One sheet in the feeder with a tag on a string: waiting on the Tickets that block it.
const WAITING = `
...........d....
...........d....
.........kkkk...
.........kyyk...
.........kyok...
.........kkkk...
...........d....
...kkkkkkkkkk...
...kwwwwwwwwk...
...kwwwwwwwwk...
...kwwwwwwwwk...
...kkkkkkkkkk...
................
..kkkkkkkkkkkk..
..kssssssssssk..
..kkkkkkkkkkkk..`;
// A finished sheet with a colour bar and lines of type: merged or ready.
const PRINTED = `
................
...kkkkkkkkkk...
...kwwwwwwwwk...
...kwccppyywk...
...kwwwwwwwwk...
...kwkkkkkkwk...
...kwwwwwwwwk...
...kwkkkkwwwk...
...kwwwwwwwwk...
...kwkkkkkkwk...
...kwwwwwwwwk...
...kwkkkwwwwk...
...kwwwwwwwwk...
...kkkkkkkkkk...
....qqqqqqqqqq..
................`;
// The delivery pile: printed sheets stacked, for the inventory.
const STACK = `
................
...kkkkkkkkkk...
...kwccppyywk...
...kwwwwwwwwk...
...kwkkkkkkwk...
...kwwwwwwwwk...
...kwkkkkwwwk...
...kwwwwwwwwk...
...kkkkkkkkkk...
..kqqqqqqqqqqk..
..kkkkkkkkkkkk..
.kqqqqqqqqqqqqk.
.kkkkkkkkkkkkkk.
kqqqqqqqqqqqqqqk
kkkkkkkkkkkkkkkk
................`;
// A torn, smudged misprint: failed.
const MISPRINT = `
................
...kkkkkkkk.....
...kwwwwwwwk....
...kwpwwwwwwk...
...kwpppwwwwk...
...kwwppprwwk...
...kwwwrppwwk...
...kwkkkpppwk...
...kwwwwwrpwk...
...kwkkkkwwwk...
...kwwwwwwwk....
...kwwwwwwk.....
...kwwwk.kk.....
...kkkk.........
................
................`;
// A sheet cut off mid-press, the rest left blank: interrupted.
const CUTOFF = `
................
...kkkkkkkkkk...
...kwccppyywk...
...kwwwwwwwwk...
...kwkkkkkkwk...
...kwwwwwwwwk...
...kwkkkkwwwk...
...kkkkkkkkkk...
................
..e.e.e.e.e.e...
................
...kkkkkkkkkk...
...kqqqqqqqqk...
...kqqqqqqqqk...
...kkkkkkkkkk...
................`;
// A jammed unit: smoke over it, a broken roller, a torn sheet. The process died here.
const JAMMED = `
..e....ee...e...
.eee..eeee.eee..
..e....ee...e...
.kkkkkkkkkkkkkk.
.kppppppppppppk.
.kkkkkkkkkkkkkk.
.ks.kkk..k.k.sk.
.ks.kpk...k..sk.
.ks.kkk..k.k.sk.
.kssssssssssssk.
.ksdkkkkkkkkdsk.
.ksdkwpwwpwkdsk.
.kssssssssssssk.
kkkkkkkkkkkkkkkk
kdkdkdkdkdkdkdkd
kkkkkkkkkkkkkkkk`;
// A defect: an ink speck with legs. R is its shell, magenta for high severity.
const BUG = `
..k....k..
...k..k...
..kkkkkk..
.kRRkkRRk.
kRRRkkRRRk
kRwRkkRwRk
kRRRkkRRRk
.kRRkkRRk.
..kk..kk..`;
const MOON = `
...wwww....
.wwwwq.....
wwwwq......
wwwq.......
wwwq.......
wwwq.......
wwwwq......
.wwwwq.....
...wwww....`;
// The on-air lamp on a running line's sign.
const LAMP = `
.kkk.
kcclk
kccck
kbcck
.kkk.`;
const BUBBLE = `
.kkkkkkk.
kwwwwwwwk
kwwwrwwwk
kwwwrwwwk
kwwwrwwwk
kwwwwwwwk
kwwwrwwwk
kwwwwwwwk
.kkkkkkk.
...kk....
...k.....`;
const KEYPAD = `
kkkkkkk
kbbbbbk
kkkkkkk
kwkwkwk
kkkkkkk
kwkwkwk
kkkkkkk
kwkwkwk
kkkkkkk`;
// The press-room door: steel leaves, a seam, two windows, handles, a hazard stripe along the bottom. Drawn by rule.
function doorMap() {
  const W = 26, H = 20, rows = [];
  for (let y = 0; y < H; y++) {
    let r = "";
    for (let x = 0; x < W; x++) {
      if (x === 0 || x === W - 1 || y === 0 || y === H - 1) { r += "k"; continue; }
      if (x === 12 || x === 13) { r += "k"; continue; }
      if (y >= 16) { r += (x + y) % 4 < 2 ? "y" : "k"; continue; }
      if (y >= 3 && y <= 7 && ((x >= 3 && x <= 9) || (x >= 16 && x <= 22))) { r += y === 3 || x === 3 || x === 16 ? "l" : "b"; continue; }
      if ((x === 10 || x === 15) && (y === 10 || y === 11)) { r += "e"; continue; }
      r += y % 5 === 0 ? "d" : "s";
    }
    rows.push(r);
  }
  return rows.join("\n");
}

// The floor is checker plate; a line's bed is a belt; hazard stripes take the fence's place.
const TILES = {
  "--t-soil": tileURL(`
mmmmmmmk
mdmmmmmk
mmmmmmmk
mmmmmdmk
mmmmmmmk
mmdmmmmk
mmmmmmmk
mmmmmmmk`),
  "--t-grass": tileURL(`
hhshhhhh
hsshhhhh
hhhhhhsh
hhhhhssh
hhhhhhhh
hhhshhhh
hhsshhhh
hhhhhhhh`),
  "--t-fence": tileURL(`
yyyykkkk
yyykkkky
yykkkkyy
ykkkkyyy
kkkkyyyy
kkkyyyyk
kkyyyykk
kyyyykkk`),
  "--i-bash": tileURL(`
kkkkkkk
kmmmmmk
kmcmmmk
kmmcmmk
kmcmcck
kmmmmmk
kkkkkkk`),
  "--i-edit": tileURL(`
.....pp
....ypp
...yyy.
..yyy..
.yyy...
kwy....
kk.....`),
  "--i-read": tileURL(`
.......
kkk.kkk
kwwkwwk
kwwkwwk
kwwkwwk
kkkkkkk
.......`),
  "--i-grep": tileURL(`
.kkk...
kwcwk..
kcwwk..
kwwck..
.kkkk..
....kk.
.....kk`),
};
const TILE_VARS = Object.entries(TILES).map(([k, v]) => `${k}:${v}`).join(";");

// ---------- What a unit or sheet is ----------
// A Ticket has a state; a Command or child Run has an outcome, or none while live or dead. A child of a Command
// that ended takes the Command's outcome.
function lifeOf(r, c) {
  if (r.state) return r.state;
  if (r.outcome) return r.outcome;
  if (r !== c && c?.outcome) return c.outcome;
  if (r.alive === false || (r !== c && c && !c.alive)) return "died";
  return "running";
}
function spriteFor(r, c, where = "field") {
  switch (lifeOf(r, c)) {
    case "running": { const i = RUN_STAGES.indexOf(r.stage); return i <= 0 ? UNIT_BLANK : i === 1 ? UNIT_HALF : UNIT_FULL; }
    case "merged": return where === "ledger" ? STACK : PRINTED;
    case "ready": return PRINTED;
    case "failed": return MISPRINT;
    case "interrupted": return CUTOFF;
    case "waiting": return WAITING;
    case "not started": return FEEDER;
    case "skipped": return UNIT_IDLE;
    case "died": return JAMMED;
    default: return UNIT_BLANK;
  }
}
// Silence is the stall (no output for a while: it may be stuck); a stage past its usual range is only slow.
const vigour = (r, c) => (lifeOf(r, c) === "running" ? health(r).key : "");
const VIGOUR = { silent: "! may be stuck", slow: "~ slower than usual, still running", ok: "on pace" };
// A running unit's ink fountain shows its pace: cyan on pace, yellow slow, magenta silent.
const INK = { silent: PAL.p, slow: PAL.y };
const bubble = (k) => k === "silent" || k === "slow"
  ? `<span class="bubble b-${k}" title="${k === "silent" ? "No output for a while: it may be stuck" : "Slower than usual, still running"}">${spr(BUBBLE.replaceAll("r", "R"), "sm", { R: k === "silent" ? PAL.p : PAL.o })}</span>` : "";
function stateLine(r, c) {
  switch (lifeOf(r, c)) {
    case "running": return r.stageSince ? `${esc(r.stage)} · ${age(r.stageSince)}` : `live · ${age(r.started)}`;
    case "waiting": return `waiting on ${r.blockedBy.map(esc).join(", ")}`;
    case "not started": return "not started";
    case "merged": return r.state ? "merged into the Spec branch" : "merged";
    case "ready": return "ready for review";
    case "failed": return "failed";
    case "interrupted": return `interrupted${r.stage ? ` at ${esc(r.stage)}` : ""}`;
    case "skipped": return "skipped";
    case "died": return `died at ${esc(r.stage)} · ${age(r.diedAt)} ago`;
  }
  return "";
}
const isSel = (c, r) => sel.cmd === c.id && sel.run === r.id;

// ---------- The press floor ----------
// The sprite opens the dialogue; the caption under it shows the issue and state, then the rest.
function crop(r, c) {
  const life = lifeOf(r, c);
  const v = vigour(r, c);
  const usual = life === "running" ? usualText(r.stage) : "";
  const spent = usageText(r.sessions ?? (r === c ? sessionsOf(c) : []));
  return `<li class="crop-cell${isSel(c, r) ? " is-sel" : ""}${v ? ` v-${v}` : ""}"><button type="button" class="crop l-${life.replace(" ", "-")}${isSel(c, r) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${r.id}" aria-label="Look at ${esc(r.issue)}: ${esc(LIFE_WORD[life] ?? life)}">
    ${bubble(v)}
    <span class="tile">${spr(spriteFor(r, c), "", INK[v] ? { R: INK[v] } : {})}</span>
  </button>
  <div class="cap">
    <div class="cap-head"><span class="cap-ref">${esc(r.issue)}</span> <span class="cap-word">${esc(LIFE_WORD[life] ?? life)}</span> <span class="cap-st st-${life.replace(" ", "-")}">${stateLine(r, c)}</span>
      ${usual ? `<span class="cap-usual">(${esc(usual)})</span>` : ""}
      ${v && v !== "ok" ? `<span class="cap-v v-${v}">${esc(v === "silent" ? `! ${health(r).label}, may be stuck` : VIGOUR.slow)}</span>` : ""}</div>
    <div class="cap-more">
      <span class="cap-title">${esc(r.title ?? "")}</span>
      ${r.harness ? `<span class="cap-hme">${esc(rhm(r))}</span>` : ""}
      ${r.latest && (life === "running" || life === "died") ? `<span class="cap-ev">› <span data-latest="${r.id}">${esc(r.latest)}</span></span>` : ""}
      ${life === "running" || life === "died" ? `<span class="cap-hb">${heartbeat(r)}</span>` : ""}
      ${spent ? `<span class="cap-spent">${esc(spent)}</span>` : ""}
    </div>
  </div></li>`;
}
// Words for every sprite, so a state never rests on colour or drawing alone. The press word sits beside the plain
// state line ("misprint" beside "failed"), never in its place.
const LIFE_WORD = { running: "on the press", merged: "printed", ready: "printed", waiting: "in the feeder", "not started": "blank sheet", failed: "misprint", interrupted: "cut off", skipped: "idle", died: "jammed" };
// The one line a folded press line shows: what its units are doing, and a flag if one is slow or silent.
function tally(c) {
  const spec = c.children?.find((ch) => ch.tickets);
  const runs = spec ? spec.tickets : [c];
  const words = { running: "running", merged: "merged", ready: "ready", waiting: "waiting", "not started": "not started", failed: "failed", interrupted: "interrupted", died: "died" };
  const n = {};
  for (const r of runs) { const w = words[lifeOf(r, c)] ?? lifeOf(r, c); n[w] = (n[w] ?? 0) + 1; }
  const line = spec ? Object.entries(n).map(([w, k]) => `<span class="tl-${w.replace(" ", "-")}">${k} ${w}</span>`).join(" · ") : stateLine(c, c);
  const flags = runs.map((r) => [r, vigour(r, c)]).filter(([, v]) => v === "silent" || v === "slow")
    .map(([r, v]) => `<span class="tl-stall tl-${v}">${v === "silent" ? "!" : "~"} ${esc(r.issue)} ${esc(r.stage)} · ${age(r.stageSince)} · ${v === "silent" ? `${esc(health(r).label)}, may be stuck` : "slower than usual"}</span>`);
  return `<p class="plot-tally">${spec ? `${esc(spec.issue)} · ` : c.issue ? `${esc(c.issue)} · ` : ""}${line}${flags.length ? ` ${flags.join(" ")}` : ""}</p>`;
}
function meter(r) {
  const i = RUN_STAGES.indexOf(r.stage);
  return `<ol class="meter" aria-label="Stages">${RUN_STAGES.map((st, n) =>
    `<li class="${n < i ? "done" : n === i ? "cur" : ""}"><span>${esc(st)}</span></li>`).join("")}</ol>`;
}
function plot(c) {
  const s = status(c);
  const spec = c.children?.find((ch) => ch.tickets);
  const crops = spec ? spec.tickets.map((t) => crop(t, c)).join("") : crop(c, c);
  const key = `plot-${c.id}`;
  return `<article class="plot is-${s.key}">
    <div class="plot-sign${isSel(c, c) ? " is-sel" : ""}" data-fold="${key}"${s.key === "died" ? ' data-fold-default="open"' : ""}>
      ${s.key === "live" ? spr(LAMP, "sm sun") : ""}${s.key === "stopping" ? '<span class="ps-stopping">stopping…</span>' : ""}
      <span class="ps-kind">${esc(c.kind)} · ${esc(c.repo)}</span>
      <span class="ps-line">${s.key === "live" ? `running ${age(c.started)}` : `started ${clock(c.started)}`}${c.note ? ` · ${esc(c.note)}` : ""}</span>
      <button type="button" class="px-btn sm sign-look" data-sel-cmd="${c.id}" data-sel-run="${c.id}">▶ Look</button>
    </div>
    ${tally(c)}
    <div class="fence" aria-hidden="true" data-fold-in="${key}"></div>
    <div class="bed" data-fold-in="${key}">
      ${spec ? `<button type="button" class="spec-label${isSel(c, spec) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${spec.id}">
        Spec run <b>${esc(spec.issue)}</b> ${esc(spec.title)} <span>· Spec branch ${esc(spec.specBranch)} · Tickets ${age(spec.stageSince)}</span></button>` : ""}
      <ul class="crops${spec ? "" : " one"}">${crops}</ul>
      ${!spec && s.key === "live" ? meter(c) : ""}
    </div>
  </article>`;
}

// ---------- The dialogue ----------
function miniRow(r, c) {
  const life = lifeOf(r, c);
  return `<li>${spr(spriteFor(r, c, "ledger"), "sm")}<span><b>${esc(r.kind ?? "Ticket")}</b> ${esc(r.issue)} ${esc(r.title ?? "")}
    <em class="st-${life.replace(" ", "-")}">${stateLine(r, c)}</em>${r.pr ? ` · PR ${esc(r.pr)}` : ""}${r.cause ? ` · ${esc(r.cause)}` : ""}</span></li>`;
}
function menu(c, r, isCmd) {
  const ids = isCmd ? runsOf(c).flatMap((x) => x.sessions ?? []) : (r.sessions ?? []);
  const items = ids.map((id) => {
    const s = sessions[id];
    const cur = state.openSession === id;
    return `<li><button type="button" data-act="session" data-id="${id}"${cur ? ' aria-current="true"' : ""}>Read the Session log · ${esc(s.owner)} ${esc(s.kind)}${s.live ? " (live)" : ""}</button></li>`;
  });
  if (c.commandLog) items.push(`<li><button type="button" data-act="cmdlog" data-id="${c.id}"${state.openCommandLog === c.id ? ' aria-current="true"' : ""}>Read the Command log${isCmd ? "" : ` of the ${esc(c.kind)}`}</button></li>`);
  if (canStop(c)) items.push(`<li><button type="button" class="danger" data-act="stop" data-id="${c.id}">Stop the line (Stop the ${esc(c.kind)})</button></li>`);
  else if (c.stopping) items.push(`<li><span class="menu-off">Stopping… salvage and Claim ending finish first</span></li>`);
  if (r.issue) items.push(`<li><a href="#${esc(r.issue.replace("#", "-"))}">Open ${esc(r.issue)} on GitHub</a></li>`);
  return items.length ? `<ul class="menu">${items.join("")}</ul>` : `<p class="dlg-note">Nothing to read: a skipped Pass keeps no Command log.</p>`;
}
function onEscape(e) {
  if (e.key !== "Escape" || !sel.cmd || document.documentElement.dataset.variant !== "pixelpress" || e.target.closest?.("input, textarea")) return;
  removeEventListener("keydown", onEscape);
  sel = {}; confirming = null; changed();
}
function dialogue() {
  const c = command(sel.cmd);
  if (!c) return "";
  const r = runsOf(c).find((x) => x.id === sel.run) ?? c;
  const isCmd = r === c;
  if (confirming === c.id && canStop(c)) {
    return `<section class="dialogue pxbox is-confirm" id="dlg" aria-live="polite">
      <button type="button" class="dlg-close" data-act="close" aria-label="Close this window">✕</button>
      <div class="dlg-portrait">${spr(spriteFor(r, c), "lg")}</div>
      <div class="dlg-text">
        <p class="dlg-who">${esc(c.kind)} · ${esc(c.repo)}</p>
        <p class="dlg-line">Stop the line? This stops the ${esc(c.kind)}. ${esc(c.stopTarget)} will be labelled <b>needs-triage</b>, not put back in line, so the Day shift looks at it first.</p>
        <p class="dlg-note">What carries on:</p>${stopEffectsHTML(c)}
        <ul class="menu">
          <li><button type="button" class="danger" data-act="do-stop" data-id="${c.id}">Stop, label needs-triage</button></li>
          <li><button type="button" data-act="requeue" data-id="${c.id}">Stop and requeue instead</button></li>
          <li><button type="button" data-act="cancel">Cancel</button></li>
        </ul>
        <p class="hint">Same as <code>$ ${esc(stopHint(c))}</code><br>Requeue: <code>$ ${esc(requeueHint(c))}</code></p>
      </div></section>`;
  }
  const facts = [];
  const v = vigour(r, c);
  facts.push(["Now", `<span class="cap-word">${esc(LIFE_WORD[lifeOf(r, c)] ?? lifeOf(r, c))}</span> <span class="st-${lifeOf(r, c).replace(" ", "-")}">${stateLine(r, c)}</span>`]);
  if (v) facts.push(["Pace", `<span class="stall v-${v}">${esc(v === "silent" ? `! ${health(r).label}, may be stuck` : VIGOUR[v])}</span>`]);
  if (v && usualText(r.stage)) facts.push(["Usually", esc(usualText(r.stage).replace(/^usually /, ""))]);
  if (v || lifeOf(r, c) === "died") facts.push(["Heartbeat", heartbeat(r) || "none recorded"]);
  if (r.harness) facts.push(["Harness · Model · Effort", esc(rhm(r))]);
  if (r.latest && ["running", "died"].includes(lifeOf(r, c))) facts.push(["Latest", `<code data-latest="${r.id}">${esc(r.latest)}</code>`]);
  if (r.cause) facts.push(["Cause", esc(r.cause)]);
  if (r.blockedBy) facts.push(["Blocked by", r.blockedBy.map(esc).join(", ")]);
  if (r.pr) facts.push(["Pull request", esc(r.pr)]);
  if (r.specBranch) facts.push(["Spec branch", esc(r.specBranch)]);
  if (isCmd && c.note) facts.push(["Note", esc(c.note)]);
  if (r.started) facts.push(["Started", `${clock(r.started)}${r.ended ? ` · ended ${clock(r.ended)} after ${dur(r.ended - r.started)}` : ""}`]);
  if (!isCmd) facts.push(["Part of", `${esc(c.kind)} on ${esc(c.repo)}${c.issue ? ` (${esc(c.issue)})` : ""}`]);
  if (summaryText(r)) facts.push(["Delivered", esc(summaryText(r))]);
  const spent = usageText(isCmd ? sessionsOf(c) : (r.sessions ?? []));
  if (spent) facts.push(["Spent", esc(spent)]);
  let extra = "";
  if (isCmd) {
    const kids = [];
    for (const ch of c.children ?? []) { kids.push(miniRow(ch, c)); if (ch.tickets) kids.push(...ch.tickets.map((t) => miniRow(t, c))); }
    for (const t of c.tickets ?? []) kids.push(miniRow(t, c));
    if (kids.length) extra += `<h4>On this line</h4><ul class="minis">${kids.join("")}</ul>`;
    if (c.findings) extra += `<h4>Defects found (Security findings)</h4><ul class="minis">${c.findings.map((f) =>
      `<li>${spr(BUG, f.sev === "high" ? "sm bug-high" : "sm", { R: f.sev === "high" ? PAL.p : PAL.d })}<span><b class="sev-${f.sev}">${esc(f.sev)}</b> ${esc(f.title)}
        <em>${f.reproduced ? `reproduced · fixed by ${esc(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced · waits for the Day shift"}</em></span></li>`).join("")}</ul>`;
  }
  const running = lifeOf(r, c) === "running" && RUN_STAGES.includes(r.stage);
  return `<section class="dialogue pxbox${popNext ? " pop" : ""}" id="dlg" aria-live="polite">
      <button type="button" class="dlg-close" data-act="close" aria-label="Close this window">✕</button>
    <div class="dlg-portrait">${spr(spriteFor(r, c, isCmd && c.outcome ? "ledger" : "field"), "lg")}</div>
    <div class="dlg-text">
      <p class="dlg-who">${esc(r.kind ?? "Ticket")}${r.issue ? ` · ${esc(r.issue)}` : ` · ${esc(c.repo)}`}</p>
      ${r.title ? `<p class="dlg-line">“${esc(r.title)}”</p>` : ""}
      <dl class="facts">${facts.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join("")}</dl>
      ${running ? meter(r) : ""}
      ${extra}
      ${menu(c, r, isCmd)}
    </div></section>`;
}

// ---------- Job log and delivery pile ----------
// The job log is its own section with a plain "Job log" h2, so app.js can fold it like the floor and the pile.
function journal() {
  const open = (meta, inner) => `<section class="journal pxbox" id="journal" aria-labelledby="px-journal"><h2 class="px-h" id="px-journal">Job log</h2>${meta}${inner}</section>`;
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    return open(`<div class="jr-bar">Command log · ${esc(c.kind)} · ${esc(c.issue ?? c.repo)} · plain text</div><p class="jr-file">${esc(c.commandLog)}</p>`,
      `<pre class="jr-body jr-cmdlog">${esc(commandLogText(c))}</pre>`);
  }
  const s = sessions[state.openSession];
  if (!s) return open("", `<p class="empty">Pick a unit and choose “Read the Session log”.</p>`);
  return open(`<div class="jr-bar">Session log · ${esc(s.owner)} · ${esc(s.kind)} · ${esc(s.harness)}${s.live ? ' <span class="jr-live">● running</span>' : ""}</div>${s.file ? `<p class="jr-file">${esc(s.file)}</p>` : ""}`,
    `<div class="jr-body"></div>`);
}
function slot(c) {
  const s = status(c);
  return `<li><button type="button" class="slot o-${s.key}${isSel(c, c) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${c.id}">
    <span class="slot-art">${spr(spriteFor(c, c, "ledger"))}${c.findings ? `<span class="slot-bug${c.findings.some((f) => f.sev === "high") ? " bug-high" : ""}">${spr(BUG, "sm", { R: c.findings.some((f) => f.sev === "high") ? PAL.p : PAL.d })}</span>` : ""}</span>
    <span class="slot-ref">${esc(c.issue ?? c.repo)}</span><span class="slot-kind">${esc(c.kind)}</span><span class="slot-out">${esc(LIFE_WORD[s.key] ?? s.label)} · ${esc(s.label)}</span>
    ${c.pr ? `<span class="slot-pr">PR ${esc(c.pr)}</span>` : ""}
    ${c.cause ? `<span class="slot-cause">${esc(c.cause)}</span>` : summaryText(c) ? `<span class="slot-sum">${esc(summaryText(c))}</span>` : ""}
    ${c.findings ? `<span class="slot-pest">${c.findings.length} defect${c.findings.length === 1 ? "" : "s"} (Security findings)</span>` : ""}
  </button></li>`;
}
function ledger() {
  const days = new Map();
  for (const c of endedCommands().sort((a, b) => b.ended - a.ended)) {
    const k = clock(c.ended).includes(" ") ? clock(c.ended).split(" ")[0] : "today";
    if (!days.has(k)) days.set(k, []);
    days.get(k).push(c);
  }
  if (!days.size) return `<p class="empty">Nothing delivered yet.</p>`;
  return [...days].map(([d, cs]) => `<h3 class="day">${esc(d)}</h3><ul class="slots">${cs.map(slot).join("")}</ul>`).join("");
}

// ---------- Job board ----------
function quests() {
  const items = needsYou();
  return `<div class="quests pxbox" role="region" aria-label="Needs you">
    <p class="q-title">${spr(BUBBLE.replaceAll("r", "R"), "sm", { R: PAL.p })} Job board · needs you <span class="count">${items.length}</span></p>
    ${items.length ? `<ul class="q-list">${items.map((n) => `<li><button type="button" class="quest q-${n.level}" data-sel-cmd="${n.cmd.id}" data-sel-run="${n.run.id}">
      <span class="q-mark" aria-hidden="true">${n.level === "urgent" ? "!" : "?"}</span>
      <span class="q-level">${n.level === "urgent" ? "Urgent" : "Look"}</span>
      <span class="q-what"><b>${esc(n.ref)}</b> ${esc(n.what)}</span>
      <span class="q-why">${esc(n.why)}</span></button></li>`).join("")}</ul>` : `<p class="empty">No jobs waiting. The floor runs itself tonight.</p>`}
  </div>`;
}

// ---------- Page ----------
function hudClock() {
  const d = new Date();
  return `Shift clock · third shift · ${d.toLocaleDateString([], { weekday: "short" })} ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}`;
}
setInterval(() => { for (const el of document.querySelectorAll(".px-clock")) el.textContent = hudClock(); }, 5000);

function gate(root) {
  root.innerHTML = `<div class="px px-gate" style="${TILE_VARS}"><form class="gate pxbox">
    <div class="sign"><span class="sign-host">${esc(host.name)}</span><span class="sign-sub">Console</span></div>
    <div class="gate-art">${spr(doorMap(), "lg")}<span class="lock">${spr(KEYPAD)}</span></div>
    <p>The press-room door is locked.</p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit" class="px-btn">▶ Unlock the door</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return gate(root);
  const field = [...liveCommands(), ...attentionCommands()];
  root.innerHTML = `<div class="px" style="${TILE_VARS}">
    <header class="hud pxbox">
      <div class="sign"><span class="sign-host">${esc(host.name)}</span><span class="sign-sub">Console</span></div>
      <div class="hud-day">${spr(MOON, "sm")}<span class="px-clock">${hudClock()}</span></div>
      <nav class="hud-nav"><a href="${host.other.url}">▶ ${esc(host.other.name)}’s floor</a><button type="button" class="px-btn sm" data-signout>Sign out</button></nav>
      <div class="cmyk" aria-hidden="true"><i class="ck-c"></i><i class="ck-m"></i><i class="ck-y"></i><i class="ck-k"></i></div>
    </header>
    ${quests()}
    <div class="px-main">
      <div class="px-left">
        <section class="field pxbox" aria-labelledby="px-field"><h2 class="px-h" id="px-field">Press floor <span class="count">${liveCommands().length} running</span></h2>
          ${field.length ? field.map(plot).join("") : `<p class="empty">Nothing on the press right now. The floor is quiet.</p>`}
        </section>
        ${dialogue()}
      </div>
      ${journal()}
    </div>
    <section class="ledger pxbox" aria-labelledby="px-ledger"><h2 class="px-h" id="px-ledger">Delivery pile</h2>${ledger()}
      <p class="ledger-end">Back to the oldest logs on this machine.</p></section>
  </div>`;
  popNext = false;
  const body = root.querySelector("div.jr-body");
  if (body) mountLog(body, state.openSession);
  root.addEventListener("click", (e) => {
    const t = e.target.closest("[data-sel-cmd], [data-act], [data-signout]");
    if (!t) return;
    if (t.dataset.selCmd) { sel = { cmd: t.dataset.selCmd, run: t.dataset.selRun }; confirming = null; scrollTarget = "#dlg"; popNext = true; changed(); return; }
    const { act, id } = t.dataset;
    if (act === "session") { scrollTarget = "#journal"; openSession(id); }
    else if (act === "cmdlog") { scrollTarget = "#journal"; openCommandLog(id); }
    else if (act === "stop") { confirming = id; scrollTarget = "#dlg"; changed(); }
    else if (act === "do-stop") { confirming = null; stopCommand(id, false); }
    else if (act === "requeue") { confirming = null; stopCommand(id, true); }
    else if (act === "cancel") { confirming = null; changed(); }
    // Closing the window clears the selection; choosing a unit or a delivered slot opens it again.
    else if (act === "close") { sel = {}; confirming = null; changed(); }
    else if ("signout" in t.dataset) signOut();
  });
  // Escape closes the window too, unless the reader is typing.
  addEventListener("keydown", onEscape);
  if (scrollTarget) {
    const el = root.querySelector(scrollTarget);
    scrollTarget = null;
    requestAnimationFrame(() => el?.scrollIntoView({ block: "nearest" }));
  }
}
