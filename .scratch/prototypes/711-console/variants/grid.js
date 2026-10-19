// PROTOTYPE variant "grid": Airflow's Grid view applied to thirdshift (docs/research/console-run-monitoring-ui.md,
// concept 2). Its question is "is this normal?": rows are stages, columns are Runs (live ones pinned first, then the
// Runs that finished here, newest first), and each cell is a state with its duration as text, beside the row's usual
// range. A table, not a chart. Choosing a column opens that Run below the grid.
import { host, commands, sessions, pastRuns, RUN_STAGES } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, isLive, attentionCommands, endedCommands, runsOf,
  stopCommand, canStop, stopEffectsHTML, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog,
  mountLog, commandLogText, changed, health, heartbeat, usual, usualText, needsYou, usageText, sessionsOf,
  summaryText, lastEventAt, isWorking,
} from "../shared.js";

const ROWS = ["implement", "review", "Delivery", "CI watch", "Repair", "Resume", "Self-merge", "cleanup"];

let selected = "t239";
let confirming = null;
let reveal = false; // open and scroll to the Run section after the next render

// The pinned columns: Runs working when the page loaded, and the died Command. Kept by id, so a Run stopped from
// this page stays pinned and shows how it ended.
const pinned = [
  ...commands.filter(isLive).flatMap((c) => runsOf(c).filter(isWorking).map((run) => ({ run, cmd: c }))),
  ...attentionCommands().map((c) => ({ run: c, cmd: c })),
];

// Stage minutes for a pinned Run. Only the current stage's start is known; the time before it is shared out over the
// earlier stages in proportion to their usual lengths. Synthesized for the prototype, as data.js has no per-stage
// history for live Runs.
function synth(r, endAt) {
  const order = RUN_STAGES;
  const i = order.indexOf(r.stage);
  const mins = { [r.stage]: Math.max(0, (endAt - r.stageSince) / 60_000) };
  const before = order.slice(0, Math.max(0, i));
  const span = (r.stageSince - r.started) / 60_000;
  if (before.length && span > 0) {
    const w = before.map((s) => { const u = usual(s); return u ? (u.lo + u.hi) / 2 : 1; });
    const tot = w.reduce((a, b) => a + b, 0);
    before.forEach((s, k) => { mins[s] = (span * w[k]) / tot; });
  }
  return mins;
}

// Finds the Run and Command behind a past Run's issue, when the prototype has them.
function findRun(issue) {
  for (const c of commands) for (const r of runsOf(c)) if (r.issue === issue) return { cmd: c, run: r };
  return { cmd: null, run: null };
}

function columns() {
  const now = Date.now();
  const live = pinned.map(({ run: r, cmd: c }) => {
    const working = isWorking(r) && isLive(c);
    const died = !c.outcome && !c.alive;
    const endAt = died ? c.diedAt : working ? now : (c.ended ?? now);
    const outcome = c.stopping ? "stopping" : working ? "live" : died ? "died" : r.state === "interrupted" ? "interrupted" : (c.outcome ?? "live");
    return { id: r.id, issue: r.issue, title: r.title, run: r, cmd: c, pinned: true, working, died, outcome,
      ended: working ? null : endAt, mins: synth(r, endAt), cur: r.stage, repairs: 0, resumes: 0 };
  });
  const past = pastRuns.map((p) => {
    const { cmd, run } = findRun(p.issue);
    const last = Object.keys(p.mins).at(-1);
    return { id: `p:${p.issue}`, issue: p.issue, title: run?.title, run, cmd, pinned: false, outcome: p.outcome,
      ended: p.ended, mins: p.mins, cur: last, repairs: p.repairs ?? 0, resumes: p.resumes ?? 0 };
  });
  return [...live, ...past];
}

const fmt = (m) => (m < 1 ? "<1m" : m < 60 ? `${Math.round(m)}m` : `${Math.floor(m / 60)}h ${Math.round(m % 60)}m`);
const chip = (outcome) => `<span class="chip o-${esc(outcome)}">${esc(outcome)}</span>`;

// One cell: its state class and words. Live cells tick; finished ones are fixed text.
function cellInfo(col, stage) {
  const m = col.mins[stage];
  if (m == null) return null;
  const u = usual(stage);
  const ratio = u ? m / u.hi : 0;
  const level = !u ? 2 : ratio < 0.5 ? 1 : ratio < 1 ? 2 : ratio < 1.5 ? 3 : ratio < 2 ? 4 : 5;
  const isCur = stage === col.cur;
  let cls = `done l${level}`, time = fmt(m), word = "";
  if (isCur && col.died) { cls = "died"; word = "died"; }
  else if (isCur && col.working) {
    const h = health(col.run);
    time = age(col.run.stageSince);
    if (h.key === "silent") { cls = "silent"; word = `silent ${fmt((Date.now() - lastEventAt(col.run)) / 60_000)}`; }
    else if (h.key === "slow") { cls = "live slow"; word = "slow, moving"; }
    else { cls = "live"; word = "running"; }
  } else if (isCur && col.outcome === "stopping") { cls = "live"; word = "stopping…"; }
  else if (isCur && col.outcome === "failed") { cls = `failed l${level}`; word = "failed"; }
  else if (isCur && col.outcome === "interrupted") { cls = `interrupted l${level}`; word = "interrupted"; }
  else if (isCur && ["merged", "ready"].includes(col.outcome)) word = col.outcome;
  const times = u && !col.died ? Math.floor(m / u.hi) : 0;
  const flag = times >= 2 ? `${times}× usual` : "";
  const count = stage === "Repair" ? col.repairs : stage === "Resume" ? col.resumes : 0;
  return { cls, time, word, flag, count };
}

function cellInner(info) {
  return `<span class="gc-time">${info.time}</span>${info.word ? `<span class="gc-word">${esc(info.word)}</span>` : ""}`
    + `${info.flag ? `<span class="gc-flag">${info.flag}</span>` : ""}${info.count ? `<span class="gc-dot" title="${info.count} in this Run">● ${info.count}</span>` : ""}`;
}

function table(cols) {
  const head = cols.map((col, ci) => `<th scope="col" class="gh${col.pinned ? " pin" : ""}${col.id === selected ? " sel" : ""}" data-c="${ci}">
      <button type="button" class="gh-btn" data-select="${esc(col.id)}" aria-pressed="${col.id === selected}">
        <span class="gh-issue">${esc(col.issue)}</span>${chip(col.outcome)}
        <span class="gh-when">${col.ended ? clock(col.ended) : "now"}</span></button></th>`).join("");
  const rows = ROWS.map((stage, ri) => {
    const cells = cols.map((col, ci) => {
      const info = cellInfo(col, stage);
      const pin = col.pinned ? " pin" : "";
      const sel = col.id === selected ? " sel" : "";
      if (!info) return `<td class="gc empty${pin}${sel}" data-r="${ri}" data-c="${ci}" style="--c:${ci}"><span class="gc-none" aria-label="not reached">·</span></td>`;
      return `<td class="gc ${info.cls}${pin}${sel}" data-r="${ri}" data-c="${ci}" data-select="${esc(col.id)}" tabindex="0" style="--c:${ci}"
        aria-label="${esc(`${col.issue} ${stage}: ${info.word || ""} ${info.flag}`)}">${cellInner(info)}</td>`;
    }).join("");
    return `<tr><th scope="row" class="rh" data-r="${ri}"><span class="rh-stage">${esc(stage)}</span><span class="rh-usual">${esc(usualText(stage) || "too few Runs for a usual")}</span></th>${cells}</tr>`;
  }).join("");
  return `<div class="grid-scroll"><table class="grid-table">
    <caption>Each cell is the time a Run spent in a stage. Shading deepens as it runs past the row's usual range. Live Runs are pinned on the left.</caption>
    <thead><tr><th scope="col" class="corner">Stage · usual</th>${head}</tr></thead><tbody>${rows}</tbody></table></div>`;
}

// The phone layout: one card per Run, each a list of its stages.
function cards(cols) {
  return `<div class="gcards">${cols.map((col) => {
    const items = ROWS.map((stage) => {
      const info = cellInfo(col, stage);
      if (!info) return "";
      return `<li class="gc ${info.cls}"><span class="gcard-stage">${esc(stage)}</span>${cellInner(info)}<span class="gcard-usual">${esc(usualText(stage))}</span></li>`;
    }).join("");
    return `<article class="gcard${col.pinned ? " pin" : ""}${col.id === selected ? " sel" : ""}">
      <button type="button" class="gcard-head" data-select="${esc(col.id)}"><span class="gh-issue">${esc(col.issue)}</span>${chip(col.outcome)}<span class="gh-when">${col.ended ? clock(col.ended) : "now"}</span></button>
      ${col.title ? `<p class="gcard-title">${esc(col.title)}</p>` : ""}<ul>${items}</ul></article>`;
  }).join("")}</div>`;
}

function needsStrip(cols) {
  const items = needsYou();
  if (!items.length) return `<div class="needs none">Nothing needs you on ${esc(host.name)}.</div>`;
  return `<div class="needs" aria-label="Needs you"><span class="needs-label">Needs you</span><ul>${items.map((it) => {
    const col = cols.find((x) => x.id === it.run.id || (it.ref && x.issue === it.ref));
    return `<li class="n-${it.level}"><button type="button" ${col ? `data-select="${esc(col.id)}"` : 'data-history=""'} title="${esc(it.why ?? "")}">
      <b>${esc(it.ref ?? "")}</b> ${esc(it.what)}</button></li>`;
  }).join("")}</ul></div>`;
}

function logLinks(cmd, run) {
  const links = [];
  if (cmd?.commandLog) links.push(`<button type="button" class="lk" data-cmdlog="${cmd.id}" aria-pressed="${state.openCommandLog === cmd.id}">Command log</button>`);
  for (const id of run?.sessions ?? []) {
    const s = sessions[id];
    links.push(`<button type="button" class="lk" data-session="${id}" aria-pressed="${state.openSession === id}">${esc(s.kind)} Session log${s.live ? " · live" : ""}</button>`);
  }
  return links.join("");
}

function detail(cols) {
  const col = cols.find((x) => x.id === selected);
  if (!col) return `<p class="muted">Choose a column or a cell in the grid to see that Run.</p>`;
  const { cmd: c, run: r } = col;
  if (!r) return `<div class="run-head"><h3>${esc(col.issue)}</h3>${chip(col.outcome)}</div>
    <p class="muted">Its logs are older than anything the prototype keeps; the grid still has its stage times.</p>`;
  const facts = [];
  const h = health(r);
  if (col.working) {
    facts.push(["Now", `${esc(r.stage)} · ${age(r.stageSince)} <span class="muted">(${esc(usualText(r.stage))})</span>`]);
    facts.push(["Health", `<span class="h-${h.key}">${esc(h.label)}</span> · ${heartbeat(r)}`]);
    if (r.latest) facts.push(["Latest", `<code data-latest="${r.id}">${esc(r.latest)}</code>`]);
  } else if (col.died) {
    facts.push(["Now", `died at ${esc(c.stage)}, ${age(c.diedAt)} ago; its process is gone and nothing will finish it`]);
    if (r.latest) facts.push(["Last seen", `<code>${esc(r.latest)}</code>`]);
  } else {
    facts.push(["Outcome", `${chip(col.outcome)}${col.ended ? ` <span class="muted">${clock(col.ended)}</span>` : ""}`]);
  }
  if (r.harness) facts.push(["Harness · Model · Effort", esc(rhm(r))]);
  if (c && c !== r) facts.push(["Part of", `${esc(c.kind)} on ${esc(c.repo)}${c.issue ? ` (${esc(c.issue)})` : ""}`]);
  const use = usageText(r.sessions ?? []);
  if (use) facts.push(["This Run", esc(use)]);
  if (c && c !== r) { const all = usageText(sessionsOf(c)); if (all) facts.push(["Whole Command", esc(all)]); }
  const sum = summaryText(r) || (c ? summaryText(c) : "");
  if (sum) facts.push(["What came of it", esc(sum)]);
  const cause = r.cause ?? (c?.outcome ? c.cause : null);
  if (cause) facts.push(["Cause", esc(cause)]);
  if (r.pr) facts.push(["Pull request", issueLink(r.pr)]);
  const st = c ? status(c) : null;
  const stop = c && canStop(c) ? `<button type="button" class="btn-stop" data-stop="${c.id}">Stop</button>`
    : st?.key === "stopping" ? `<span class="chip o-stopping">stopping… salvage finishing</span>` : "";
  const confirm = c && confirming === c.id && canStop(c) ? `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
      <p><b>Stop this ${esc(c.kind)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code> rather than put back in line, so the Day shift looks at it before the factory takes it again.</p>
      ${stopEffectsHTML(c)}
      <div class="confirm-actions">
        <button type="button" class="btn-stop solid" data-do-stop="${c.id}">Stop · label needs-triage</button>
        <button type="button" data-do-requeue="${c.id}">Stop and requeue instead</button>
        <button type="button" data-cancel>Cancel</button></div>
      <p class="hint"><code>$ ${esc(stopHint(c))}</code> · requeue: <code>$ ${esc(requeueHint(c))}</code></p></div>` : "";
  let log = "";
  if (c && state.openCommandLog === c.id) log = `<p class="log-file">${esc(c.commandLog)} · plain text</p><pre class="cmdlog">${esc(commandLogText(c))}</pre>`;
  else if ((r.sessions ?? []).includes(state.openSession)) {
    const s = sessions[state.openSession];
    log = `<p class="log-file">${esc(s.file ?? `${s.kind} Session log`)}${s.live ? ' · <span class="chip o-live">live</span>' : ""}</p><div class="log-body"></div>`;
  } else if (logLinks(c, r)) log = `<p class="muted">Choose a log above.</p>`;
  return `<div class="run-head">
      <div><p class="kind">${esc(r.kind ?? "Ticket")}</p><h3>${issueLink(r.issue)} ${esc(r.title ?? "")}</h3></div>${stop}</div>
    ${confirm}
    <dl class="facts">${facts.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join("")}</dl>
    <div class="links">${logLinks(c, r)}</div>
    <div class="log-box">${log}</div>`;
}

function history() {
  const ended = endedCommands().sort((a, b) => b.ended - a.ended);
  if (!ended.length) return `<p class="muted">No Command has ended on ${esc(host.name)} yet.</p>`;
  return `<ol class="hist">${ended.map((c) => {
    const key = `gh-${c.id}`;
    const subj = c.issue ? `${issueLink(c.issue)} ${esc(c.title)}` : `${esc(c.repo)}${c.note ? ` · ${esc(c.note)}` : ""}`;
    const kids = runsOf(c).slice(1).map((r) => `<li>${esc(r.kind ?? "Ticket")} ${issueLink(r.issue)} ${esc(r.title ?? "")} ${chip(r.state ?? r.outcome ?? c.outcome)}${r.cause ? ` <span class="muted">${esc(r.cause)}</span>` : ""}</li>`).join("");
    const findings = (c.findings ?? []).map((f) => `<li><span class="chip sev-${f.sev}">${esc(f.sev)}</span> ${esc(f.title)} <span class="muted">· ${f.reproduced ? `reproduced · fixed by ${esc(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced · waits for the Day shift"}</span></li>`).join("");
    const sum = summaryText(c), use = usageText(sessionsOf(c));
    return `<li class="hist-item o-${c.outcome}">
      <div class="hist-head" data-fold="${key}"${c.outcome === "failed" ? ' data-fold-default="open"' : ""}>
        <span class="hist-when">${clock(c.ended)}</span><span class="kind">${esc(c.kind)}</span> <span class="hist-subj">${subj}</span> ${chip(c.outcome)}</div>
      <div class="hist-body" data-fold-in="${key}">
        ${c.cause ? `<p>${esc(c.outcome === "skipped" ? `Skipped: ${c.cause}.` : c.cause)}</p>` : ""}
        ${sum ? `<p>${esc(sum)}</p>` : ""}${use ? `<p class="muted">${esc(use)}${c.started ? ` · took ${dur(c.ended - c.started)}` : ""}</p>` : ""}
        ${kids ? `<ul class="hist-kids">${kids}</ul>` : ""}
        ${findings ? `<p class="hist-sub">Security findings</p><ul class="hist-kids">${findings}</ul>` : ""}
        ${c.commandLog ? `<button type="button" class="lk" data-cmdlog="${c.id}" data-col="">Command log</button>` : ""}
      </div></li>`;
  }).join("")}</ol>`;
}

function signInPage(root) {
  root.innerHTML = `<div class="grid-page signin"><form>
    <p class="si-host">${esc(host.name)} <span class="muted">Console</span></p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Sign in</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  const cols = columns();
  root.innerHTML = `<div class="grid-page">
    <header class="top"><p class="host"><b>${esc(host.name)}</b> <span class="muted">Console · stage grid</span></p>
      <nav><a href="${host.other.url}">${esc(host.other.name)} →</a> <button type="button" class="link" data-signout>Sign out</button></nav></header>
    ${needsStrip(cols)}
    <section class="sec-grid"><h2>Stage grid</h2>${table(cols)}${cards(cols)}</section>
    <section class="sec-run" id="grid-run"><h2>Run</h2>${detail(cols)}</section>
    <section class="sec-hist"><h2>History</h2><p class="muted sec-note">Every ended Command, newest first, with the Pass outcomes a grid of stages can't show.</p>${history()}</section>
  </div>`;

  const body = root.querySelector(".log-body");
  if (body) mountLog(body, state.openSession);

  // Hovering or focusing a cell lights its row and column.
  const tableEl = root.querySelector(".grid-table");
  const light = (e) => {
    const t = e.target.closest("[data-r], [data-c]");
    tableEl.querySelectorAll(".hl").forEach((x) => x.classList.remove("hl"));
    if (!t) return;
    const { r, c } = t.dataset;
    if (r != null) tableEl.querySelectorAll(`[data-r="${r}"]`).forEach((x) => x.classList.add("hl"));
    if (c != null) tableEl.querySelectorAll(`[data-c="${c}"]`).forEach((x) => x.classList.add("hl"));
  };
  tableEl.addEventListener("mouseover", light);
  tableEl.addEventListener("focusin", light);
  tableEl.addEventListener("mouseleave", () => tableEl.querySelectorAll(".hl").forEach((x) => x.classList.remove("hl")));
  tableEl.addEventListener("keydown", (e) => {
    const td = e.target.closest("td[data-select]");
    if (td && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); td.click(); }
  });

  root.addEventListener("click", (e) => {
    const t = e.target.closest("[data-select], [data-history], [data-stop], [data-do-stop], [data-do-requeue], [data-cancel], [data-signout], [data-session], [data-cmdlog]");
    if (!t) return;
    const d = t.dataset;
    if (d.select) {
      selected = d.select; confirming = null; reveal = true;
      const col = cols.find((x) => x.id === selected);
      const first = (col?.run?.sessions ?? []).find((id) => sessions[id].live) ?? col?.run?.sessions?.[0];
      if (first) openSession(first); else changed();
    } else if ("history" in d) {
      const h2 = root.querySelector(".sec-hist h2");
      if (h2 && h2.getAttribute("aria-expanded") !== "true") h2.click();
      h2?.scrollIntoView({ block: "start", behavior: "smooth" });
    } else if (d.stop) { confirming = d.stop; changed(); }
    else if (d.doStop) { confirming = null; stopCommand(d.doStop, false); }
    else if (d.doRequeue) { confirming = null; stopCommand(d.doRequeue, true); }
    else if ("cancel" in d) { confirming = null; changed(); }
    else if ("signout" in d) signOut();
    else if (d.session) openSession(d.session);
    else if (d.cmdlog) {
      // A history row's Command log opens in the Run section, under that Command's own Run.
      const c = commands.find((x) => x.id === d.cmdlog);
      if ("col" in d && c) { const col = cols.find((x) => x.cmd === c && (x.run === c || x.issue === c.issue)) ?? cols.find((x) => x.cmd === c); if (col) selected = col.id; reveal = true; }
      openCommandLog(d.cmdlog);
    }
  });

  // After choosing a Run, open its section (app.js folds every section on load) and bring it into view.
  if (reveal) {
    reveal = false;
    requestAnimationFrame(() => {
      const h2 = root.querySelector(".sec-run h2");
      if (h2 && h2.getAttribute("aria-expanded") !== "true") h2.click();
      root.querySelector("#grid-run")?.scrollIntoView({ block: "start", behavior: "smooth" });
    });
  }
}
