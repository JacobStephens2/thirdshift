// PROTOTYPE variant "pixel": the garden as a cozy 16-bit farming game. Live Commands are fenced plots on a tilemap,
// Runs and Tickets are pixel plants, choosing one opens an RPG dialogue with a menu, the open log is a journal
// window, and history is an inventory of harvested slots. Every sprite is a string map drawn as crisp SVG rects.
import { host, sessions, RUN_STAGES } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, status, isLive, liveCommands, attentionCommands, endedCommands, runsOf,
  stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog, mountLog, commandLogText,
  command, changed, canStop, stopEffectsHTML, usualText, heartbeat, health, needsYou, usageText, sessionsOf, summaryText,
} from "../shared.js";

// No window open on load, like the sections; choosing a plant or a ledger slot opens one.
let sel = {};
let confirming = null;
let scrollTarget = null;
let popNext = false; // the dialogue pops in only when a plant or slot is newly chosen

// ---------- Sprites ----------
// A Sweetie-16 palette plus two browns for wood and soil. "." is transparent; R is a per-sprite override.
const PAL = { k: "#1a1c2c", p: "#5d275d", r: "#b13e53", o: "#ef7d57", y: "#ffcd75", l: "#a7f070", g: "#38b764",
  t: "#257179", n: "#29366f", b: "#3b5dc9", c: "#41a6f6", s: "#73eff7", w: "#f4f4f4", e: "#94b0c2", d: "#566c86",
  m: "#333c57", u: "#a0663a", v: "#5c3a21", x: "#7d4b2a", R: "#b13e53" };

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

const PACKET = `
................
................
................
................
.....kkkkkk.....
....kwwwwwwk....
....kyyyyyyk....
....kygyygyk....
....kyglgyyk....
....kyyggyyk....
....kyyygyyk....
....kyyyyyyk....
....kyoyoyok....
....kyyyyyyk....
....kkkkkkkk....
.....vvvvvv.....`;
const WAITING = `
................
................
...........u....
...........u....
.......kkkku....
......kwwwwk....
......kwddwk....
......kwwwwk....
.......kkkku....
...........u....
...........u....
...........u....
...........u....
...........u....
.....kk....u....
....kyyk...u....`;
const SPROUT = `
................
................
................
................
................
................
................
................
................
................
...ll.....ll....
..lggl...lggl...
...lggl.lggl....
.....lgggl......
.......gg.......
.......gg.......`;
const BUD = `
................
................
................
................
.......kk.......
......krrk......
......rorr......
......krrk......
.......gg.......
.......gg.lll...
..lll..gglggl...
.lgggl.ggggl....
..lggggggg......
.......gg.......
.......gg.......
.......gg.......`;
const TALL = `
.......kk.......
......krok......
.....krrorr.....
......krrk......
.......gg.......
...lll.gg.......
..lgggggg..ll...
...lll.gg.lggl..
.......ggggll...
..ll...gg.......
.lggl..gg.......
..lgggggg.......
.......gg..ll...
.......gglggl...
.......gggll....
.......gg.......`;
const BLOOM = `
................
.....rr..rr.....
....rrrrrrrr....
....rrryyrrr....
...rrryooyrrr...
....rrryyrrr....
....rrrrrrrr....
.....rr.grr.....
.......gg.......
.......gg.......
..ll...gg..ll...
.lggl..gg.lggl..
..lgggggggggl...
.......gg.......
.......gg.......
.......gg.......`;
const CARROT = `
................
.......l..l.....
......lgllg.....
.......lggl.....
......lgggl.....
.......ggl......
......oooo......
.....ooyooo.....
.....oooooo.....
......oyoo......
......oooo......
.......oo.......
.......oo.......
........o.......
................
................`;
const WILTED = `
................
................
................
................
....dd..........
...doxd.........
...ddxd.........
....dd.d........
........d.......
.........d......
..dd.....d......
.dxxd...dd......
..dxxd.dd.......
....dxdd........
.......d........
.......d........`;
const CUT = `
................
................
................
................
................
................
................
................
................
.......ll.......
.......gg.......
..ll...gg..e....
.lggl..gg.e.e...
..lgggggg..e....
.......gg.......
.......gg.......`;
const DEAD = `
...u............
...uu...........
....u.....d.....
....u....d.d....
....u.....d.....
....u..d..d.....
....u.d.d.d.....
....u....dd.....
....u.....d..d..
....u.....d.d...
....u.....dd....
....u.....d.....
....u.....d.....
....u.....d.....
....u.....d.....
...uuu....d.....`;
const FALLOW = `
................
................
................
................
................
................
................
................
................
................
................
.............l..
............lg..
..l.............
.vxv....vxv.....
vxxxv..vxxxv....`;
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
const SUN = `
.....y.....
..y..y..y..
...yyyyy...
..yyoyoyy..
yyyyyyyyyyy
..yyoooyy..
...yyyyy...
..y..y..y..
.....y.....`;
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
const PADLOCK = `
..kkkk..
.k....k.
.k....k.
kkkkkkkk
kyyyyyyk
kyykkyyk
kyyykyyk
kyyyyyyk
kkkkkkkk`;
// The gate: two posts, pickets and two rails. Drawn by rule rather than by hand.
function gateMap() {
  const W = 26, H = 20, rows = [];
  for (let y = 0; y < H; y++) {
    let r = "";
    for (let x = 0; x < W; x++) {
      if (x < 4 || x >= W - 4) { r += x === 0 || x === 3 || x === W - 4 || x === W - 1 || y === 0 ? "k" : "u"; continue; }
      if (y === 5 || y === 6 || y === 14 || y === 15) r += "v";
      else if ((x - 5) % 4 < 2 && y >= 2) r += "u";
      else r += ".";
    }
    rows.push(r);
  }
  return rows.join("\n");
}

const TILES = {
  "--t-soil": tileURL(`
vvxvvvvv
vvvvvxvv
xvvvvvvv
vvvvxvvv
vvvvvvvx
vxvvvvvv
vvvvvvxv
vvvxvvvv`),
  "--t-grass": tileURL(`
gggggggg
gglggggg
gggggtgg
gggggggg
ggtggggl
gggggggg
glgggtgg
gggggggg`),
  "--t-fence": tileURL(`
.kk.....
kuuk....
kuukkkkk
kuuuuuuu
kuukkkkk
kuuk....
kuukkkkk
kuuuuuuu`),
  "--i-bash": tileURL(`
kkkkkkk
kmmmmmk
kmlmmmk
kmmlmmk
kmlmllk
kmmmmmk
kkkkkkk`),
  "--i-edit": tileURL(`
.....rr
....yrr
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
kwswk..
kswwk..
kwwsk..
.kkkk..
....kk.
.....kk`),
};
const TILE_VARS = Object.entries(TILES).map(([k, v]) => `${k}:${v}`).join(";");

// ---------- What a plant is ----------
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
    case "running": { const i = RUN_STAGES.indexOf(r.stage); return i <= 0 ? SPROUT : i === 1 ? BUD : TALL; }
    case "merged": return where === "ledger" ? CARROT : BLOOM;
    case "ready": return BLOOM;
    case "failed": return WILTED;
    case "interrupted": return CUT;
    case "waiting": return WAITING;
    case "not started": return PACKET;
    case "skipped": return FALLOW;
    case "died": return DEAD;
    default: return SPROUT;
  }
}
// Silence is the stall (no output for a while: it may be stuck); a stage past its usual range is only slow.
const vigour = (r, c) => (lifeOf(r, c) === "running" ? health(r).key : "");
const VIGOUR = { silent: "! may be stuck", slow: "~ slower than usual, still growing", ok: "on pace" };
const bubble = (k) => k === "silent" || k === "slow"
  ? `<span class="bubble b-${k}" title="${k === "silent" ? "No output for a while: it may be stuck" : "Slower than usual, still growing"}">${spr(BUBBLE.replaceAll("r", "R"), "sm", { R: k === "silent" ? PAL.r : PAL.o })}</span>` : "";
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

// ---------- The field ----------
// The sprite opens the dialogue; the caption under it folds to issue and state, and unfolds to the rest.
function crop(r, c) {
  const life = lifeOf(r, c);
  const v = vigour(r, c);
  const usual = life === "running" ? usualText(r.stage) : "";
  const spent = usageText(r.sessions ?? (r === c ? sessionsOf(c) : []));
  return `<li class="crop-cell${isSel(c, r) ? " is-sel" : ""}${v ? ` v-${v}` : ""}"><button type="button" class="crop l-${life.replace(" ", "-")}${isSel(c, r) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${r.id}" aria-label="Look at ${esc(r.issue)}: ${esc(LIFE_WORD[life] ?? life)}">
    ${bubble(v)}
    <span class="tile">${spr(spriteFor(r, c))}</span>
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
// Words for every sprite, so a state never rests on colour or drawing alone.
const LIFE_WORD = { running: "growing", merged: "harvested", ready: "harvested", waiting: "waiting", "not started": "seed packet", failed: "wilted", interrupted: "cut", skipped: "fallow", died: "dead" };
// The one line a folded plot shows: what its plants are doing, and a "!" if one has been in a stage too long.
function tally(c) {
  const spec = c.children?.find((ch) => ch.tickets);
  const runs = spec ? spec.tickets : [c];
  const words = { running: "growing", merged: "bloomed", ready: "bloomed", waiting: "waiting", "not started": "seeds", failed: "wilted", interrupted: "cut", died: "dead" };
  const n = {};
  for (const r of runs) { const w = words[lifeOf(r, c)] ?? lifeOf(r, c); n[w] = (n[w] ?? 0) + 1; }
  const line = spec ? Object.entries(n).map(([w, k]) => `<span class="tl-${w}">${k} ${w}</span>`).join(" · ") : stateLine(c, c);
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
      ${s.key === "live" ? spr(SUN, "sm sun") : ""}${s.key === "stopping" ? '<span class="ps-stopping">stopping…</span>' : ""}
      <span class="ps-kind">${esc(c.kind)} · ${esc(c.repo)}</span>
      <span class="ps-line">${s.key === "live" ? `planted ${age(c.started)} ago` : `planted ${clock(c.started)}`}${c.note ? ` · ${esc(c.note)}` : ""}</span>
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
  if (canStop(c)) items.push(`<li><button type="button" class="danger" data-act="stop" data-id="${c.id}">Pull it up (Stop the ${esc(c.kind)})</button></li>`);
  else if (c.stopping) items.push(`<li><span class="menu-off">Stopping… salvage and Claim ending finish first</span></li>`);
  if (r.issue) items.push(`<li><a href="#${esc(r.issue.replace("#", "-"))}">Open ${esc(r.issue)} on GitHub</a></li>`);
  return items.length ? `<ul class="menu">${items.join("")}</ul>` : `<p class="dlg-note">Nothing to read: a skipped Pass keeps no Command log.</p>`;
}
function onEscape(e) {
  if (e.key !== "Escape" || !sel.cmd || document.documentElement.dataset.variant !== "pixel" || e.target.closest?.("input, textarea")) return;
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
        <p class="dlg-line">Pull up this ${esc(c.kind)}? ${esc(c.stopTarget)} will be labelled <b>needs-triage</b>, not put back in line, so the Day shift looks at it first.</p>
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
  if (v) facts.push(["Vigour", `<span class="stall v-${v}">${esc(v === "silent" ? `! ${health(r).label}, may be stuck` : VIGOUR[v])}</span>`]);
  if (v && usualText(r.stage)) facts.push(["Usually", esc(usualText(r.stage).replace(/^usually /, ""))]);
  if (v || lifeOf(r, c) === "died") facts.push(["Heartbeat", heartbeat(r) || "none recorded"]);
  if (r.harness) facts.push(["Harness · Model · Effort", esc(rhm(r))]);
  if (r.latest && ["running", "died"].includes(lifeOf(r, c))) facts.push(["Latest", `<code data-latest="${r.id}">${esc(r.latest)}</code>`]);
  if (r.cause) facts.push(["Cause", esc(r.cause)]);
  if (r.blockedBy) facts.push(["Blocked by", r.blockedBy.map(esc).join(", ")]);
  if (r.pr) facts.push(["Pull request", esc(r.pr)]);
  if (r.specBranch) facts.push(["Spec branch", esc(r.specBranch)]);
  if (isCmd && c.note) facts.push(["Note", esc(c.note)]);
  if (r.started) facts.push(["Planted", `${clock(r.started)}${r.ended ? ` · ended ${clock(r.ended)} after ${dur(r.ended - r.started)}` : ""}`]);
  if (!isCmd) facts.push(["Part of", `${esc(c.kind)} on ${esc(c.repo)}${c.issue ? ` (${esc(c.issue)})` : ""}`]);
  if (summaryText(r)) facts.push(["Harvest", esc(summaryText(r))]);
  const spent = usageText(isCmd ? sessionsOf(c) : (r.sessions ?? []));
  if (spent) facts.push(["Spent", esc(spent)]);
  let extra = "";
  if (isCmd) {
    const kids = [];
    for (const ch of c.children ?? []) { kids.push(miniRow(ch, c)); if (ch.tickets) kids.push(...ch.tickets.map((t) => miniRow(t, c))); }
    for (const t of c.tickets ?? []) kids.push(miniRow(t, c));
    if (kids.length) extra += `<h4>In this plot</h4><ul class="minis">${kids.join("")}</ul>`;
    if (c.findings) extra += `<h4>Pests found (Security findings)</h4><ul class="minis">${c.findings.map((f) =>
      `<li>${spr(BUG, f.sev === "high" ? "sm bug-high" : "sm", { R: f.sev === "high" ? PAL.r : PAL.d })}<span><b class="sev-${f.sev}">${esc(f.sev)}</b> ${esc(f.title)}
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

// ---------- Journal and ledger ----------
// The Journal is its own section with a plain "Journal" h2, so app.js can fold it like the field and the ledger.
function journal() {
  const open = (meta, inner) => `<section class="journal pxbox" id="journal" aria-labelledby="px-journal"><h2 class="px-h" id="px-journal">Journal</h2>${meta}${inner}</section>`;
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    return open(`<div class="jr-bar">Command log · ${esc(c.kind)} · ${esc(c.issue ?? c.repo)} · plain text</div><p class="jr-file">${esc(c.commandLog)}</p>`,
      `<pre class="jr-body jr-cmdlog">${esc(commandLogText(c))}</pre>`);
  }
  const s = sessions[state.openSession];
  if (!s) return open("", `<p class="empty">Pick a plant and choose “Read the Session log”.</p>`);
  return open(`<div class="jr-bar">Session log · ${esc(s.owner)} · ${esc(s.kind)} · ${esc(s.harness)}${s.live ? ' <span class="jr-live">● growing</span>' : ""}</div>${s.file ? `<p class="jr-file">${esc(s.file)}</p>` : ""}`,
    `<div class="jr-body"></div>`);
}
function slot(c) {
  const s = status(c);
  return `<li><button type="button" class="slot o-${s.key}${isSel(c, c) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${c.id}">
    <span class="slot-art">${spr(spriteFor(c, c, "ledger"))}${c.findings ? `<span class="slot-bug${c.findings.some((f) => f.sev === "high") ? " bug-high" : ""}">${spr(BUG, "sm", { R: c.findings.some((f) => f.sev === "high") ? PAL.r : PAL.d })}</span>` : ""}</span>
    <span class="slot-ref">${esc(c.issue ?? c.repo)}</span><span class="slot-kind">${esc(c.kind)}</span><span class="slot-out">${esc(LIFE_WORD[s.key] ?? s.label)} · ${esc(s.label)}</span>
    ${c.pr ? `<span class="slot-pr">PR ${esc(c.pr)}</span>` : ""}
    ${c.cause ? `<span class="slot-cause">${esc(c.cause)}</span>` : summaryText(c) ? `<span class="slot-sum">${esc(summaryText(c))}</span>` : ""}
    ${c.findings ? `<span class="slot-pest">${c.findings.length} pest${c.findings.length === 1 ? "" : "s"} (Security findings)</span>` : ""}
  </button></li>`;
}
function ledger() {
  const days = new Map();
  for (const c of endedCommands().sort((a, b) => b.ended - a.ended)) {
    const k = clock(c.ended).includes(" ") ? clock(c.ended).split(" ")[0] : "today";
    if (!days.has(k)) days.set(k, []);
    days.get(k).push(c);
  }
  if (!days.size) return `<p class="empty">Nothing harvested yet.</p>`;
  return [...days].map(([d, cs]) => `<h3 class="day">${esc(d)}</h3><ul class="slots">${cs.map(slot).join("")}</ul>`).join("");
}

// ---------- Quest log ----------
function quests() {
  const items = needsYou();
  return `<div class="quests pxbox" role="region" aria-label="Needs you">
    <p class="q-title">${spr(BUBBLE.replaceAll("r", "R"), "sm", { R: PAL.r })} Quest log · needs you <span class="count">${items.length}</span></p>
    ${items.length ? `<ul class="q-list">${items.map((n) => `<li><button type="button" class="quest q-${n.level}" data-sel-cmd="${n.cmd.id}" data-sel-run="${n.run.id}">
      <span class="q-mark" aria-hidden="true">${n.level === "urgent" ? "!" : "?"}</span>
      <span class="q-level">${n.level === "urgent" ? "Urgent" : "Look"}</span>
      <span class="q-what"><b>${esc(n.ref)}</b> ${esc(n.what)}</span>
      <span class="q-why">${esc(n.why)}</span></button></li>`).join("")}</ul>` : `<p class="empty">No quests. The field tends itself today.</p>`}
  </div>`;
}

// ---------- Page ----------
function hudClock() {
  const d = new Date();
  const season = ["Winter", "Winter", "Spring", "Spring", "Spring", "Summer", "Summer", "Summer", "Autumn", "Autumn", "Autumn", "Winter"][d.getMonth()];
  return `${season} · Day ${d.getDate()} · ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}`;
}
setInterval(() => { for (const el of document.querySelectorAll(".px-clock")) el.textContent = hudClock(); }, 5000);

function gate(root) {
  root.innerHTML = `<div class="px px-gate" style="${TILE_VARS}"><form class="gate pxbox">
    <div class="sign"><span class="sign-host">${esc(host.name)}</span><span class="sign-sub">Console</span></div>
    <div class="gate-art">${spr(gateMap(), "lg")}<span class="lock">${spr(PADLOCK)}</span></div>
    <p>The garden gate is locked.</p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit" class="px-btn">▶ Unlock the gate</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return gate(root);
  const field = [...liveCommands(), ...attentionCommands()];
  root.innerHTML = `<div class="px" style="${TILE_VARS}">
    <header class="hud pxbox">
      <div class="sign"><span class="sign-host">${esc(host.name)}</span><span class="sign-sub">Console</span></div>
      <div class="hud-day">${spr(SUN, "sm")}<span class="px-clock">${hudClock()}</span></div>
      <nav class="hud-nav"><a href="${host.other.url}">▶ ${esc(host.other.name)}’s field</a><button type="button" class="px-btn sm" data-signout>Sign out</button></nav>
    </header>
    ${quests()}
    <div class="px-main">
      <div class="px-left">
        <section class="field pxbox" aria-labelledby="px-field"><h2 class="px-h" id="px-field">The field <span class="count">${liveCommands().length} growing</span></h2>
          ${field.length ? field.map(plot).join("") : `<p class="empty">Nothing planted right now. The field rests.</p>`}
        </section>
        ${dialogue()}
      </div>
      ${journal()}
    </div>
    <section class="ledger pxbox" aria-labelledby="px-ledger"><h2 class="px-h" id="px-ledger">Harvest ledger</h2>${ledger()}
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
    // Closing the window clears the selection; choosing a plant or a ledger slot opens it again.
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
