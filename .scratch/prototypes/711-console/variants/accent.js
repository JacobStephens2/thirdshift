// PROTOTYPE variant "accent": plain and neutral, with a light garden accent. Master-detail: a rail of every
// Command on the left, the selected one on the right with its stage stepper, its Runs and tabs for its logs.
// On a phone the rail comes first and a Command opens over it, with a way back. A "Needs you" strip sits above
// both; choosing an item selects its Command and unfolds the Ticket it names.
import { host, sessions, RUN_STAGES, SPEC_STAGES } from "../data.js";
import {
  state, esc, rhm, age, clock, issueLink, status, liveCommands, attentionCommands, endedCommands, runsOf,
  activeRun, ticketCounts, stopCommand, stopHint, requeueHint, signIn, signOut, openCommandLog, mountLog,
  commandLogText, command, changed, canStop, stopEffectsHTML, usual, usualText, isWorking, health, heartbeat,
  needsYou, usageText, sessionsOf, summaryText, isLive,
} from "../shared.js";
import { revealTarget } from "./plain.js";

let selected = "c1";
let showDetail = false; // phones only: the detail replaces the rail
let confirming = null;

// ---------- Icons: small strokes, one per state ----------
const PATHS = {
  sprout: '<path d="M12 21v-9"/><path d="M12 12C12 8 9 6 5 6c0 4 3 6 7 6z"/><path d="M12 10c0-3 2-5 6-5 0 3-2 5-6 5z"/>',
  leaf: '<path class="fill" d="M5 19C5 11 11 5 19 5c0 8-6 14-14 14z"/><path d="M5 19l8-8"/>',
  bud: '<path d="M12 21v-7"/><path d="M12 14c-3 0-4-3-4-5s2-5 4-6c2 1 4 4 4 6s-1 5-4 5z"/>',
  seed: '<ellipse cx="12" cy="13" rx="4.5" ry="6.5" transform="rotate(25 12 13)"/><path d="M10 9.5c1 1.5 2 3 2.5 5.5"/>',
  wilted: '<path d="M8 21v-8a4 4 0 0 1 8 0"/><path d="M16 13c0 3-1.2 5-3.5 6.2 0-3 1.2-5 3.5-6.2z"/>',
  frost: '<path d="M12 3v18M4.2 7.5l15.6 9M4.2 16.5l15.6-9"/><path d="M10 4.5l2 1.5 2-1.5M10 19.5l2-1.5 2 1.5"/>',
  skip: '<circle cx="12" cy="12" r="8"/><path d="M8.5 12h7"/>',
  clock: '<circle cx="12" cy="12" r="8"/><path d="M12 8v4l2.5 2"/>',
};
const ICON_FOR = {
  live: "sprout", running: "sprout", merged: "leaf", ready: "bud", failed: "wilted", died: "wilted",
  interrupted: "frost", skipped: "skip", waiting: "clock", "not started": "seed",
};
const icon = (key, cls = "") => `<svg class="ic ic-${(key ?? "").replace(" ", "-")} ${cls}" viewBox="0 0 24 24" aria-hidden="true">${PATHS[ICON_FOR[key] ?? "seed"]}</svg>`;

// ---------- Small pieces ----------
const ref = (c) => c.issue ?? c.children?.[0]?.issue ?? c.repo;
const titleOf = (c) => c.title ?? c.children?.[0]?.title ?? c.note ?? c.cause ?? "";
const sessionIds = (c) => runsOf(c).flatMap((r) => r.sessions ?? []);

function chip(c) {
  const s = status(c);
  if (s.key === "live") {
    const r = activeRun(c);
    const h = r ? health(r) : { key: "" };
    const word = h.key === "silent" ? " · silent" : h.key === "slow" ? " · slower than usual" : "";
    return `<span class="chip chip-live${h.key && h.key !== "ok" ? ` chip-${h.key}` : ""}">${r ? `${esc(r.stage)} · ${age(r.stageSince)}${word}` : `live · ${age(c.started)}`}</span>`;
  }
  if (s.key === "stopping") return `<span class="chip chip-stopping">stopping…</span>`;
  if (s.key === "died") return `<span class="chip chip-died">died at ${esc(c.stage)}</span>`;
  return `<span class="chip chip-${s.key}">${esc(s.label)}</span>`;
}

function railItem(c) {
  const s = status(c);
  return `<li><button type="button" class="item${c.id === selected ? " sel" : ""}" data-select="${c.id}"${c.id === selected ? ' aria-current="true"' : ""}>
    ${icon(s.key)}
    <span class="it-line"><span class="it-kind">${esc(c.kind)}</span> <span class="it-ref">${esc(ref(c))}</span></span>
    ${chip(c)}
    <span class="it-title">${esc(titleOf(c))}</span>
    ${c.ended ? `<span class="it-when">${esc(clock(c.ended))}</span>` : ""}
  </button></li>`;
}

function rail() {
  const live = liveCommands(), died = attentionCommands();
  const days = new Map();
  for (const c of endedCommands().sort((a, b) => b.ended - a.ended)) {
    const k = clock(c.ended).includes(" ") ? clock(c.ended).split(" ")[0] : "Today";
    if (!days.has(k)) days.set(k, []);
    days.get(k).push(c);
  }
  return `<nav class="rail" aria-label="Commands">
    <h2 class="rail-h">Live <span class="n">${live.length}</span></h2>
    ${live.length ? `<ul>${live.map(railItem).join("")}</ul>` : `<div class="empty">${icon("not started", "big")}<p><b>Nothing growing.</b> No Command is running on ${esc(host.name)}.</p></div>`}
    ${died.length ? `<h2 class="rail-h">Died <span class="n">${died.length}</span></h2><ul>${died.map(railItem).join("")}</ul>` : ""}
    <h2 class="rail-h">History</h2>
    ${[...days].map(([d, cs]) => `<h3 class="rail-d">${esc(d)}</h3><ul>${cs.map(railItem).join("")}</ul>`).join("")}
    <p class="rail-end">Back to the oldest logs on this machine.</p>
  </nav>`;
}

// A Run's stages in a row: done, the one it is at (with its age), and the rest.
function stepper(label, stages, r, c) {
  const s = status(c);
  const mode = s.key === "live" ? "now" : s.key;
  const at = stages.indexOf(r.stage);
  if (at < 0 && mode !== "merged" && mode !== "ready") return "";
  const items = stages.map((st, i) => {
    let cls, when = "";
    if (mode === "merged") cls = "done";
    else if (mode === "ready") cls = st === "Self-merge" ? "skip" : "done";
    else if (i < at) cls = "done";
    else if (i > at) cls = "todo";
    else {
      const h = mode === "now" ? health(r) : { key: "" };
      cls = `at at-${mode}${h.key && h.key !== "ok" ? ` h-${h.key}` : ""}`;
      when = ` <span class="st-age">${mode === "now" ? age(r.stageSince) : esc(mode)}</span>`
        + (mode === "now" && isWorking(r) ? `<span class="st-pace">${h.key === "silent" ? `silent: ${esc(h.label)}` : h.key === "slow" ? "slower than usual" : "on pace"}</span>${heartbeat(r)}` : "");
    }
    const u = usual(st);
    const typical = u && cls !== "done" && cls !== "skip" ? `<span class="st-usual">usually ${u.lo}–${u.hi} min</span>` : "";
    return `<li class="${cls}"><span class="dot"></span><span class="st-name">${esc(st)}</span>${when}${typical}</li>`;
  }).join("");
  return `<div class="stepper"><p class="st-label">${label}</p><ol>${items}</ol></div>`;
}

// The Command's own place: its own stepper if it is a Run, or its Spec run's. Ticket and child Run steppers
// sit inside their rows, one unfold down.
function steppers(c) {
  const out = [];
  if (c.kind === "Run" && (c.stage || c.outcome === "merged" || c.outcome === "ready")) out.push(stepper(`Run ${esc(c.issue)}`, RUN_STAGES, c, c));
  for (const r of [c, ...(c.children ?? [])]) if (r.kind === "Spec run" && r.stage) out.push(stepper(`Spec run ${esc(r.issue)}`, SPEC_STAGES, r, c));
  return out.length ? `<section class="d-sec"><h3>Where it is</h3>${out.join("")}</section>` : "";
}

// A Ticket's or child Run's stepper, from its own state rather than its Command's.
function ownStepper(r, k) {
  const like = k === "running" || k === "live" ? { alive: true, outcome: null }
    : { alive: false, outcome: k === "not started" || k === "waiting" ? "skipped" : k };
  return stepper("Stages", RUN_STAGES, r, like);
}

const logLinks = (r) => (r.sessions ?? []).length ? `<div class="tk-logs">${r.sessions.map((id) => {
  const s = sessions[id];
  return `<button type="button" class="linkish" data-tab-session="${id}">${esc(s.kind)} Session log${s.live ? ' <span class="live-dot" title="live"></span>' : ""}</button>`;
}).join("")}</div>` : "";

// One row of the checklist: folded, a line naming it and its state; unfolded, its stepper, harness, latest
// and logs. anc lists the folds above it (app.js shows it only once all of them are open).
function foldRow(r, k, line, anc, kind = "") {
  const h = health(r);
  const meta = [
    r.harness ? `<span class="hme">${esc(rhm(r))}</span>` : "",
    r.latest && k === "running" ? `<code class="latest" data-latest="${r.id}">${esc(r.latest)}</code>` : "",
    r.pr ? `<span>PR ${issueLink(r.pr)}</span>` : "",
  ].filter(Boolean).join(" ");
  const tokens = usageText(r.sessions ?? []);
  return `<li class="tk tk-${k.replace(" ", "-")}${h.key && h.key !== "ok" ? ` h-${h.key}` : ""}"${anc.length ? ` data-fold-in="${anc.join(" ")}"` : ""}>
    <div class="tk-head" data-fold="${r.id}"${k === "failed" ? ' data-fold-default="open"' : ""}>${icon(k)}
      <div class="tk-main"><div>${kind ? `<span class="tk-kind">${esc(kind)}</span> ` : ""}${issueLink(r.issue, "ref")} <span class="tk-title">${esc(r.title)}</span></div>
        <div class="tk-line">${line}</div></div></div>
    <div class="tk-more" data-fold-in="${[...anc, r.id].join(" ")}">
      ${ownStepper(r, k)}
      ${meta ? `<div class="tk-meta">${meta}</div>` : ""}
      ${isWorking(r) ? `<div class="tk-pace">${heartbeat(r)} · ${esc(r.stage)} ${esc(usualText(r.stage) || "has no usual range yet")}</div>` : ""}
      ${tokens ? `<div class="tk-tokens">${esc(tokens)}</div>` : ""}
      ${summaryText(r) ? `<div class="tk-pace">${esc(summaryText(r))}</div>` : ""}
      ${logLinks(r)}
    </div></li>`;
}

function ticketRow(t, anc) {
  let line;
  switch (t.state) {
    case "running": { const h = health(t); line = `${esc(t.stage)} · ${age(t.stageSince)}${h.key === "slow" ? ' · <span class="pace-word">slower than usual</span>' : h.key === "silent" ? ` · <span class="pace-word">silent: ${esc(h.label)}</span>` : ""}`; break; }
    case "merged": line = `merged into the Spec branch${t.pr ? ` · ${issueLink(t.pr)}` : ""}`; break;
    case "failed": line = `failed: ${esc(t.cause)}`; break;
    case "waiting": line = `cannot start yet: waiting on ${t.blockedBy.map((b) => issueLink(b)).join(", ")}`; break;
    case "interrupted": line = `interrupted at ${esc(t.stage)}`; break;
    default: line = "not started";
  }
  return foldRow(t, t.state, line, anc);
}

function childRun(r, anc) {
  const k = r.outcome ?? "live";
  return foldRow(r, k, `${esc(k)}${r.pr ? ` · PR ${issueLink(r.pr)}` : ""}${r.cause ? ` · ${esc(r.cause)}` : ""}`, anc, r.kind);
}

function runsSection(c) {
  const parts = [];
  const specs = [c, ...(c.children ?? [])].filter((r) => r.tickets);
  for (const sp of specs) {
    const n = ticketCounts(sp);
    const sum = `<span class="spec-sum">Spec branch <code>${esc(sp.specBranch)}</code> · ${Object.entries(n).map(([k, v]) => `${v} ${k}`).join(" · ")}</span>`;
    // Selecting the Command is the first level, so its Tickets show straight away and each Ticket is the one
    // fold under it (NN/g: no more than two disclosure levels).
    parts.push(`<div class="spec">${sp === c ? `<p class="spec-h">${sum}</p>` : `<p class="spec-h"><span class="tk-kind">Spec run</span> ${issueLink(sp.issue, "ref")} <span class="tk-title">${esc(sp.title)}</span> ${sum}</p>`}
      <ul class="tickets">${sp.tickets.map((t) => ticketRow(t, [])).join("")}</ul></div>`);
  }
  const kids = (c.children ?? []).filter((r) => !r.tickets);
  if (kids.length) parts.push(`<ul class="tickets">${kids.map((r) => childRun(r, [])).join("")}</ul>`);
  if (!parts.length) return "";
  return `<section class="d-sec"><h3>${specs.length ? "Tickets" : "Runs it started"}</h3>${parts.join("")}</section>`;
}

function findingsSection(c) {
  if (!c.findings) return "";
  return `<section class="d-sec"><h3>Security findings</h3><div class="tbl-wrap"><table class="findings">
    <thead><tr><th>Severity</th><th>Finding</th><th>Reproduced</th><th>Fix</th></tr></thead>
    <tbody>${c.findings.map((f) => `<tr><td><span class="sev sev-${f.sev}">${esc(f.sev)}</span></td><td>${esc(f.title)}</td>
      <td>${f.reproduced ? "yes" : "no"}</td><td>${f.fix ? `${issueLink(f.fix)} · ${esc(f.fixOutcome)}` : "waits for the Day shift"}</td></tr>`).join("")}</tbody>
  </table></div></section>`;
}

// Keeps the log tab on something this Command owns: its live session first, else its first, else its Command log.
function ensureTab(c) {
  const ids = sessionIds(c);
  if (state.openCommandLog === c.id || ids.includes(state.openSession)) return;
  const pick = ids.find((id) => sessions[id].live) ?? ids[0];
  if (pick) { state.openSession = pick; state.openCommandLog = null; }
  else if (c.commandLog) { state.openCommandLog = c.id; state.openSession = null; }
  else { state.openSession = null; state.openCommandLog = null; }
}

function logsSection(c) {
  const ids = sessionIds(c);
  if (!ids.length && !c.commandLog) return `<section class="d-sec"><h3>Logs</h3><p class="muted">Skipped before doing any work, so it keeps no Command log.</p></section>`;
  const tabs = [
    c.commandLog ? `<button type="button" role="tab" data-tab-cmdlog="${c.id}" aria-selected="${state.openCommandLog === c.id}">Command log</button>` : "",
    ...ids.map((id) => { const s = sessions[id]; return `<button type="button" role="tab" data-tab-session="${id}" aria-selected="${state.openSession === id}">${esc(s.owner.includes("#") ? `#${s.owner.split("#")[1]}` : s.owner)} · ${esc(s.kind)}${s.live ? ' <span class="live-dot" title="live"></span>' : ""}</button>`; }),
  ].join("");
  let panel = "";
  if (state.openCommandLog === c.id) panel = `<p class="file">${esc(c.commandLog)} · plain text</p><pre class="logbox cmdlog">${esc(commandLogText(c))}</pre>`;
  else if (state.openSession) {
    const s = sessions[state.openSession];
    panel = `<p class="file">${esc(s.file ?? `${s.kind} Session log`)} · ${esc(s.harness)}${s.live ? " · new lines appear at the bottom" : ""}</p><div class="logbox sesslog"></div>`;
  }
  return `<section class="d-sec"><h3>Logs</h3><div class="tabs" role="tablist">${tabs}</div>${panel}</section>`;
}

function detail(c) {
  if (!c) return `<div class="detail"><div class="empty">${icon("not started", "big")}<p>Pick a Command to see it.</p></div></div>`;
  const s = status(c);
  ensureTab(c);
  const facts = [
    ["Harness · Model · Effort", esc(rhm(c))],
    ["Started", `${esc(clock(c.started))} <span class="muted">· ${age(c.started)} ago</span>`],
    s.key === "live" ? null : s.key === "stopping" ? ["Status", `<span class="chip chip-stopping">stopping…</span> <span class="muted">salvage, Claim ending and Spec accounting finish first</span>`] : s.key === "died" ? ["Status", `<span class="chip chip-died">died at ${esc(c.stage)}</span> <span class="muted">process gone ${age(c.diedAt)} ago, with no outcome</span>`]
      : ["Ended", `<span class="chip chip-${s.key}">${esc(s.label)}</span> ${esc(clock(c.ended))}${c.cause ? ` <span class="muted">· ${esc(c.cause)}</span>` : ""}`],
    c.pr ? ["Pull request", issueLink(c.pr)] : null,
    c.note ? ["Note", esc(c.note)] : null,
    c.latest && s.key !== "live" ? ["Last event", `<code>${esc(c.latest)}</code>`] : null,
    c.latest && s.key === "live" ? ["Latest", `<code class="latest" data-latest="${c.id}">${esc(c.latest)}</code>`] : null,
    pace(c),
    usageText(sessionsOf(c)) ? ["Tokens and time", esc(usageText(sessionsOf(c)))] : null,
    c.summary ? ["What came of it", esc(summaryText(c))] : null,
  ].filter(Boolean);
  const confirm = confirming === c.id && canStop(c) ? `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
    <p><b>Stop this ${esc(c.kind)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code> rather than put back in line, so the Day shift looks at it first.</p>
    ${stopEffectsHTML(c)}
    <div class="confirm-actions">
      <button type="button" class="btn-danger" data-do-stop="${c.id}">Stop, label needs-triage</button>
      <button type="button" data-do-requeue="${c.id}">Requeue instead</button>
      <button type="button" data-cancel>Cancel</button>
    </div>
    <p class="hint"><code>$ ${esc(stopHint(c))}</code><br><code>$ ${esc(requeueHint(c))}</code></p>
  </div>` : "";
  return `<div class="detail">
    <button type="button" class="back" data-back>← All Commands</button>
    <header class="d-head">
      <p class="d-kind">${icon(s.key)} ${esc(c.kind)} · ${esc(c.repo)}</p>
      <div class="d-title"><h2>${c.issue ? `${issueLink(c.issue, "ref")} ${esc(c.title)}` : esc(titleOf(c) || c.repo)}</h2>
        ${canStop(c) && confirming !== c.id ? `<div class="stop"><button type="button" class="btn-stop" data-stop="${c.id}">Stop</button><code class="hint-inline">${esc(stopHint(c))}</code></div>` : ""}</div>
      ${confirm}
      <dl class="facts">${facts.map(([k, v]) => `<div><dt>${k}</dt><dd>${v}</dd></div>`).join("")}</dl>
    </header>
    ${steppers(c)}${runsSection(c)}${findingsSection(c)}${logsSection(c)}
  </div>`;
}

// Pace of the Run working now: how long since it last logged, and whether that is slow or silent, in words.
function pace(c) {
  if (!isLive(c)) return null;
  const r = activeRun(c);
  if (!r || !isWorking(r)) return null;
  const h = health(r);
  return ["Pace", `<span class="pace-word p-${h.key}">${h.key === "silent" ? `silent: ${esc(h.label)}` : h.key === "slow" ? "slower than usual, still working" : "on pace"}</span> · ${heartbeat(r)} · ${esc(r.issue ?? "")} ${esc(r.stage)} ${esc(usualText(r.stage))}`];
}

// One item per problem, urgent first (SRE: page on symptoms that need a human). No h2, so it never folds away.
function needsStrip() {
  const items = needsYou();
  if (!items.length) return `<div class="needs none" role="region" aria-label="Needs you">${icon("merged")}<p><b>Nothing needs you.</b> Every Command is working or done.</p></div>`;
  return `<div class="needs" role="region" aria-label="Needs you">
    <p class="needs-h"><b>Needs you</b> <span class="n">${items.length}</span></p>
    <ul>${items.map((it) => `<li><button type="button" class="need need-${it.level}" data-need-cmd="${it.cmd.id}" data-need-run="${it.run.id}">
      <span class="lvl">${it.level === "urgent" ? "Urgent" : "Look"}</span>
      <span class="what"><b>${esc(it.ref)}</b> ${esc(it.what)}</span>
      <span class="why">${esc(it.why)}</span></button></li>`).join("")}</ul>
  </div>`;
}

// The latest event line flashes once each time the live session changes it.
function flashOnChange(root) {
  new MutationObserver((ms) => {
    for (const m of ms) {
      const el = m.target.closest?.("[data-latest]");
      if (!el) continue;
      el.classList.remove("flash");
      void el.offsetWidth;
      el.classList.add("flash");
    }
  }).observe(root, { subtree: true, childList: true, characterData: true });
}

function signInPage(root) {
  root.innerHTML = `<div class="acc signin"><form>
    <svg class="logo" viewBox="0 0 24 24" aria-hidden="true">${PATHS.sprout}</svg>
    <h1>${esc(host.name)} <span class="muted">Console</span></h1>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Sign in</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  const c = command(selected);
  root.innerHTML = `<div class="acc${showDetail ? " show-detail" : ""}">
    <header class="top">
      <h1><svg class="logo" viewBox="0 0 24 24" aria-hidden="true">${PATHS.sprout}</svg>${esc(host.name)} <span class="muted">Console</span></h1>
      <nav><a href="${host.other.url}">${esc(host.other.name)} →</a><button type="button" class="link" data-signout>Sign out</button></nav>
    </header>
    ${needsStrip()}
    <div class="split">${rail()}${detail(c)}</div>
  </div>`;
  const box = root.querySelector(".sesslog");
  if (box) mountLog(box, state.openSession);
  flashOnChange(root);
  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a");
    if (!t) return;
    const d = t.dataset;
    if (d.needCmd) {
      // Select its Command, then unfold the Ticket or child Run it names, if any.
      if (d.needCmd !== selected) { selected = d.needCmd; confirming = null; ensureTab(command(selected)); }
      showDetail = true; changed();
      const row = d.needRun !== d.needCmd && document.querySelector(`#app .tk-head[data-fold="${d.needRun}"]`);
      revealTarget(row ? `.tk-head[data-fold="${d.needRun}"]` : ".d-head", row ? [d.needRun] : []);
    }
    else if (d.select) {
      if (d.select !== selected) { selected = d.select; confirming = null; ensureTab(command(selected)); }
      showDetail = true; changed();
      if (matchMedia("(max-width: 760px)").matches) scrollTo(0, 0);
    }
    else if ("back" in d) { showDetail = false; changed(); }
    else if (d.stop) { confirming = d.stop; changed(); }
    else if (d.doStop) { confirming = null; stopCommand(d.doStop, false); }
    else if (d.doRequeue) { confirming = null; stopCommand(d.doRequeue, true); }
    else if ("cancel" in d) { confirming = null; changed(); }
    else if ("signout" in d) signOut();
    else if (d.tabSession) { state.openSession = d.tabSession; state.openCommandLog = null; changed(); }
    else if (d.tabCmdlog) openCommandLog(d.tabCmdlog);
  });
}
