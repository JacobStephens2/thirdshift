// PROTOTYPE variant "presslayout": the pixel variant's layout on a press floor, drawn in thirdshift.app's press look
// with no pixel art. Live Commands are press lines; Runs and Tickets are press units or sheets; choosing one opens a
// paper job sheet with a menu; the open log is the job log terminal; history is the delivery pile of proof cards.
// Every icon is a small vector drawing in the site's inks, and every state carries its plain word.
import { host, sessions, RUN_STAGES } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, status, isLive, liveCommands, attentionCommands, endedCommands, runsOf,
  stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog, mountLog, commandLogText,
  command, changed, canStop, stopEffectsHTML, usualText, heartbeat, health, needsYou, usageText, sessionsOf, summaryText,
} from "../shared.js";

// No window open on load, like the sections; choosing a unit or a delivery-pile card opens one.
let sel = {};
let confirming = null;
let scrollTarget = null;
let popNext = false; // the job sheet slides in only when a unit or card is newly chosen

// ---------- Icons ----------
// Small vector drawings in the site's press-line style (a unit with an ink band, rollers and a drum; sheets and
// stacks), 64×64, filled with the page's ink tokens. The width and height attributes hold the size even without the
// stylesheet.
const roller = (x, stroke) => `<g transform="translate(${x} 31)"><g class="roll"><circle r="4.5" fill="none" stroke="${stroke}" stroke-width="1.5"/><path d="M0 -4.5v9M-4.5 0h9" stroke="${stroke}" stroke-width="1"/></g></g>`;
function unit({ band, bandStroke = "none", dash = "", ink = "var(--dim)", body = "var(--k2)", edge = "var(--dim)", extra = "" }) {
  return `<rect x="10" y="7" width="44" height="50" rx="3" fill="${body}" stroke="${edge}" stroke-width="2"/>
    <rect x="15" y="12" width="34" height="10" fill="${band}" stroke="${bandStroke}" stroke-width="2"${dash ? ` stroke-dasharray="${dash}"` : ""}/>
    <g class="rollers">${roller(22, "var(--dim)")}${roller(32, "var(--dim)")}${roller(42, "var(--dim)")}</g>
    <g transform="translate(32 46)"><g class="drum"><circle r="7.5" fill="var(--k3)" stroke="${ink}" stroke-width="2.2"/><path d="M0 -7.5v15M-7.5 0h15" stroke="var(--dim)" stroke-width="1"/></g></g>${extra}`;
}
const sheet = (x, y, edge, extra = "") => `<g transform="translate(${x} ${y})"><rect width="34" height="42" rx="1.5" fill="var(--paper)" stroke="#000" stroke-opacity=".35"/><rect width="34" height="4" fill="${edge}"/>
  <path d="M6 13h22M6 19h22M6 25h16" stroke="#000" stroke-opacity=".28" stroke-width="2"/>${extra}</g>`;
const ICONS = {
  // On the press: the band inked cyan, rollers and drum turn (CSS).
  running: () => unit({ band: "var(--c)", ink: "var(--c)", edge: "var(--paper)" }),
  // Queued, not started: an empty dashed band, the unit dimmed.
  queued: () => unit({ band: "none", bandStroke: "var(--dim)", dash: "4 3", body: "var(--k)" }),
  // Waiting on a blocker: a yellow dashed band and pause bars on the drum.
  waiting: () => unit({ band: "none", bandStroke: "var(--y)", dash: "4 3", ink: "var(--y)", body: "var(--k)",
    extra: '<path d="M29 42v8M35 42v8" stroke="var(--y)" stroke-width="2.6" stroke-linecap="round"/>' }),
  // Delivered: a stack of printed sheets with a cyan edge and a check.
  delivered: () => `${sheet(19, 15, "var(--c)")}${sheet(15, 11, "var(--c)")}${sheet(11, 7, "var(--c)",
    '<path d="M10 31l6 6 11-13" fill="none" stroke="var(--c)" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"/>')}`,
  // Misprint, failed: the plates out of register and a magenta smear.
  misprint: () => sheet(15, 10, "var(--m)", `<rect x="7" y="21" width="16" height="12" fill="var(--c)" opacity=".75"/><rect x="11" y="24" width="16" height="12" fill="var(--m)" opacity=".8"/>
    <path d="M4 39c6-4 12 2 18-2s8-6 10-3" fill="none" stroke="var(--m)" stroke-width="2.4" stroke-linecap="round"/>`),
  // Pulled, interrupted: a sheet with a yellow cut across it and a folded corner.
  pulled: () => sheet(15, 10, "var(--y)", `<path d="M-2 33L36 21" stroke="var(--y)" stroke-width="2.4" stroke-dasharray="5 3"/><path d="M26 42l8-8v8z" fill="var(--k3)"/>`),
  // Jammed, died: a magenta band, a crumpled sheet caught in the rollers, a cross on the drum.
  jammed: () => unit({ band: "var(--m)", ink: "var(--m)", edge: "var(--m)",
    extra: `<path d="M17 26l7-4 6 5 7-5 8 4-3 6-7-2-6 4-6-3-5 2z" fill="var(--paper)" stroke="#000" stroke-opacity=".4"/>
      <path d="M26 40l12 12M38 40L26 52" stroke="var(--m)" stroke-width="3" stroke-linecap="round"/>` }),
  // Idle, skipped: the unit outlined in grey, nothing inked.
  idle: () => unit({ band: "none", bandStroke: "var(--rule)", body: "none", edge: "var(--rule)", ink: "var(--rule)" }),
  // A Security finding: a small beetle, magenta when high.
  bug: ({ high }) => `<g fill="${high ? "var(--m)" : "var(--dim)"}" stroke="${high ? "var(--m)" : "var(--dim)"}" stroke-width="2.4" stroke-linecap="round">
    <ellipse cx="32" cy="36" rx="12" ry="15"/><circle cx="32" cy="18" r="6"/><path d="M20 28l-9-5M20 37h-10M21 46l-8 6M44 28l9-5M44 37h10M43 46l8 6M29 13l-4-7M35 13l4-7" fill="none"/></g>
    <path d="M32 22v29" stroke="var(--k)" stroke-width="2"/>`,
  // The registration mark, as on the site; it turns slowly while anything runs.
  reg: () => `<g class="regmark" fill="none" stroke="var(--paper)" stroke-width="2.4"><circle cx="32" cy="32" r="16"/><path d="M32 4v56M4 32h56"/></g><circle cx="32" cy="32" r="7" fill="var(--paper)"/>`,
  // A flag over a unit: "!" when silent (magenta), "~" when slower than usual (yellow).
  alert: ({ ink, glyph }) => `<circle cx="32" cy="32" r="27" fill="${ink}"/><text x="32" y="44" text-anchor="middle" font-family="JetBrains Mono, monospace" font-weight="700" font-size="34" fill="#000">${glyph}</text>`,
};
function spr(kind, cls = "", opts = {}) {
  const k = /\blg\b/.test(cls) ? 72 : /\bsm\b/.test(cls) ? 26 : 56;
  return `<svg class="ico ico-${kind} ${cls}" width="${k}" height="${k}" viewBox="0 0 64 64" aria-hidden="true">${ICONS[kind](opts)}</svg>`;
}

// ---------- What a unit is ----------
// A Ticket has a state; a Command or child Run has an outcome, or none while live or dead. A child of a Command
// that ended takes the Command's outcome.
function lifeOf(r, c) {
  if (r.state) return r.state;
  if (r.outcome) return r.outcome;
  if (r !== c && c?.outcome) return c.outcome;
  if (r.alive === false || (r !== c && c && !c.alive)) return "died";
  return "running";
}
function spriteFor(r, c) {
  switch (lifeOf(r, c)) {
    case "merged": case "ready": return "delivered";
    case "failed": return "misprint";
    case "interrupted": return "pulled";
    case "waiting": return "waiting";
    case "not started": return "queued";
    case "skipped": return "idle";
    case "died": return "jammed";
    default: return "running";
  }
}
// Silence is the stall (no output for a while: it may be stuck); a stage past its usual range is only slow.
const vigour = (r, c) => (lifeOf(r, c) === "running" ? health(r).key : "");
const VIGOUR = { silent: "! may be stuck", slow: "~ slower than usual, still working", ok: "on pace" };
const bubble = (k) => k === "silent" || k === "slow"
  ? `<span class="bubble b-${k}" title="${k === "silent" ? "No output for a while: it may be stuck" : "Slower than usual, still working"}">${spr("alert", "sm", k === "silent" ? { ink: "var(--m)", glyph: "!" } : { ink: "var(--y)", glyph: "~" })}</span>` : "";
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
// The unit opens the job sheet; the caption under it says issue and state, then the rest.
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
// Words for every drawing, so a state never rests on colour or shape alone. The press word comes first; the caption
// beside it always says the plain state too.
const LIFE_WORD = { running: "on the press", merged: "delivered", ready: "delivered", waiting: "waiting", "not started": "queued", failed: "misprint", interrupted: "pulled", skipped: "idle", died: "jammed" };
// The one line a folded press line shows: what its units are doing, with a flag for a silent or slow one.
function tally(c) {
  const spec = c.children?.find((ch) => ch.tickets);
  const runs = spec ? spec.tickets : [c];
  const words = { running: "running", merged: "merged", ready: "ready", waiting: "waiting", "not started": "not started", failed: "failed", interrupted: "interrupted", died: "died" };
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
      ${s.key === "live" ? spr("reg", "sm reg-live") : ""}${s.key === "stopping" ? '<span class="ps-stopping">stopping…</span>' : ""}
      <span class="ps-kind">${esc(c.kind)} · ${esc(c.repo)}</span>
      <span class="ps-line">${s.key === "live" ? `started ${age(c.started)} ago` : `started ${clock(c.started)}`}${c.note ? ` · ${esc(c.note)}` : ""}</span>
      <button type="button" class="px-btn sm sign-look" data-sel-cmd="${c.id}" data-sel-run="${c.id}">Job sheet →</button>
    </div>
    ${tally(c)}
    <div class="fence" aria-hidden="true" data-fold-in="${key}"><span class="belt-sheet"></span></div>
    <div class="bed" data-fold-in="${key}">
      ${spec ? `<button type="button" class="spec-label${isSel(c, spec) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${spec.id}">
        Spec run <b>${esc(spec.issue)}</b> ${esc(spec.title)} <span>· Spec branch ${esc(spec.specBranch)} · Tickets ${age(spec.stageSince)}</span></button>` : ""}
      <ul class="crops${spec ? "" : " one"}">${crops}</ul>
      ${!spec && s.key === "live" ? meter(c) : ""}
    </div>
  </article>`;
}

// ---------- The job sheet ----------
function miniRow(r, c) {
  const life = lifeOf(r, c);
  return `<li>${spr(spriteFor(r, c), "sm")}<span><b>${esc(r.kind ?? "Ticket")}</b> ${esc(r.issue)} ${esc(r.title ?? "")}
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
  if (canStop(c)) items.push(`<li><button type="button" class="danger" data-act="stop" data-id="${c.id}">Stop this ${esc(c.kind)}</button></li>`);
  else if (c.stopping) items.push(`<li><span class="menu-off">Stopping… salvage and Claim ending finish first</span></li>`);
  if (r.issue) items.push(`<li><a href="#${esc(r.issue.replace("#", "-"))}">Open ${esc(r.issue)} on GitHub</a></li>`);
  return items.length ? `<ul class="menu">${items.join("")}</ul>` : `<p class="dlg-note">Nothing to read: a skipped Pass keeps no Command log.</p>`;
}
function onEscape(e) {
  if (e.key !== "Escape" || !sel.cmd || document.documentElement.dataset.variant !== "presslayout" || e.target.closest?.("input, textarea")) return;
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
        <p class="dlg-line">Stop this ${esc(c.kind)}? ${esc(c.stopTarget)} will be labelled <b>needs-triage</b>, not put back in line, so the Day shift looks at it first.</p>
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
  if (summaryText(r)) facts.push(["What came of it", esc(summaryText(r))]);
  const spent = usageText(isCmd ? sessionsOf(c) : (r.sessions ?? []));
  if (spent) facts.push(["Tokens and time", esc(spent)]);
  let extra = "";
  if (isCmd) {
    const kids = [];
    for (const ch of c.children ?? []) { kids.push(miniRow(ch, c)); if (ch.tickets) kids.push(...ch.tickets.map((t) => miniRow(t, c))); }
    for (const t of c.tickets ?? []) kids.push(miniRow(t, c));
    if (kids.length) extra += `<h4>On this press line</h4><ul class="minis">${kids.join("")}</ul>`;
    if (c.findings) extra += `<h4>Security findings</h4><ul class="minis">${c.findings.map((f) =>
      `<li>${spr("bug", f.sev === "high" ? "sm bug-high" : "sm", { high: f.sev === "high" })}<span><b class="sev-${f.sev}">${esc(f.sev)}</b> ${esc(f.title)}
        <em>${f.reproduced ? `reproduced · fixed by ${esc(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced · waits for the Day shift"}</em></span></li>`).join("")}</ul>`;
  }
  const running = lifeOf(r, c) === "running" && RUN_STAGES.includes(r.stage);
  return `<section class="dialogue pxbox${popNext ? " pop" : ""}" id="dlg" aria-live="polite">
      <button type="button" class="dlg-close" data-act="close" aria-label="Close this window">✕</button>
    <div class="dlg-portrait">${spr(spriteFor(r, c), "lg")}</div>
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
// The Job log is its own section with a plain "Job log" h2, so app.js can fold it like the floor and the pile.
function journal() {
  const open = (meta, inner) => `<section class="journal pxbox" id="journal" aria-labelledby="px-journal"><h2 class="px-h" id="px-journal">Job log</h2>${meta}${inner}</section>`;
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    return open(`<div class="jr-bar">Command log · ${esc(c.kind)} · ${esc(c.issue ?? c.repo)} · plain text</div><p class="jr-file">${esc(c.commandLog)}</p>`,
      `<pre class="jr-body jr-cmdlog">${esc(commandLogText(c))}</pre>`);
  }
  const s = sessions[state.openSession];
  if (!s) return open("", `<p class="empty">Choose a unit and pick “Read the Session log”.</p>`);
  return open(`<div class="jr-bar">Session log · ${esc(s.owner)} · ${esc(s.kind)} · ${esc(s.harness)}${s.live ? ' <span class="jr-live">● live</span>' : ""}</div>${s.file ? `<p class="jr-file">${esc(s.file)}</p>` : ""}`,
    `<div class="jr-body"></div>`);
}
function slot(c) {
  const s = status(c);
  return `<li><button type="button" class="slot o-${s.key}${isSel(c, c) ? " is-sel" : ""}" data-sel-cmd="${c.id}" data-sel-run="${c.id}">
    <span class="slot-art">${spr(spriteFor(c, c))}${c.findings ? `<span class="slot-bug${c.findings.some((f) => f.sev === "high") ? " bug-high" : ""}">${spr("bug", "sm", { high: c.findings.some((f) => f.sev === "high") })}</span>` : ""}</span>
    <span class="slot-ref">${esc(c.issue ?? c.repo)}</span><span class="slot-kind">${esc(c.kind)}</span><span class="slot-out">${esc(LIFE_WORD[s.key] ?? s.label)} · ${esc(s.label)}</span>
    ${c.pr ? `<span class="slot-pr">PR ${esc(c.pr)}</span>` : ""}
    ${c.cause ? `<span class="slot-cause">${esc(c.cause)}</span>` : summaryText(c) ? `<span class="slot-sum">${esc(summaryText(c))}</span>` : ""}
    ${c.findings ? `<span class="slot-pest">${c.findings.length} Security finding${c.findings.length === 1 ? "" : "s"}</span>` : ""}
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
    <p class="q-title">Job board · needs you <span class="count">${items.length}</span></p>
    ${items.length ? `<ul class="q-list">${items.map((n) => `<li><button type="button" class="quest q-${n.level}" data-sel-cmd="${n.cmd.id}" data-sel-run="${n.run.id}">
      <span class="q-mark" aria-hidden="true">${n.level === "urgent" ? "!" : "?"}</span>
      <span class="q-level">${n.level === "urgent" ? "Urgent" : "Look"}</span>
      <span class="q-what"><b>${esc(n.ref)}</b> ${esc(n.what)}</span>
      <span class="q-why">${esc(n.why)}</span></button></li>`).join("")}</ul>` : `<p class="empty">Nothing on the job board. The floor runs itself today.</p>`}
  </div>`;
}

// ---------- Page ----------
function hudClock() {
  return `Shift clock · third shift · ${new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}`;
}
setInterval(() => { for (const el of document.querySelectorAll(".px-clock")) el.textContent = hudClock(); }, 5000);

function gate(root) {
  root.innerHTML = `<div class="px px-gate"><form class="gate pxbox">
    <span class="crop-mark tl"></span><span class="crop-mark tr"></span><span class="crop-mark bl"></span><span class="crop-mark br"></span>
    <p class="slug">thirdshift · press floor · Console</p>
    <div class="sign"><span class="sign-host misreg">${esc(host.name)}</span></div>
    <p>Sign in to see this machine's press floor.</p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit" class="px-btn">Sign in</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return gate(root);
  const field = [...liveCommands(), ...attentionCommands()];
  root.innerHTML = `<div class="px">
    <header class="hud pxbox">
      <div class="sign"><span class="slug">thirdshift · press floor · Console</span><span class="sign-host misreg">${esc(host.name)}</span></div>
      <div class="hud-day">${spr("reg", "sm reg-live")}<span class="px-clock">${hudClock()}</span></div>
      <nav class="hud-nav"><a href="${host.other.url}">${esc(host.other.name)} Console →</a><button type="button" class="px-btn sm" data-signout>Sign out</button></nav>
      <div class="colorbar" aria-hidden="true"><i class="cb-c"></i><i class="cb-m"></i><i class="cb-y"></i><i class="cb-k"></i><i class="cb-c50"></i><i class="cb-m50"></i><i class="cb-y50"></i></div>
    </header>
    ${quests()}
    <div class="px-main">
      <div class="px-left">
        <section class="field pxbox" aria-labelledby="px-field"><h2 class="px-h" id="px-field">Press floor <span class="count">${liveCommands().length} running</span></h2>
          ${field.length ? field.map(plot).join("") : `<p class="empty">Nothing on the press right now.</p>`}
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
    // Closing the job sheet clears the selection; choosing a unit or a card opens it again.
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
