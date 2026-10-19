// PROTOTYPE variant "plain": one column, no theme. A "Needs you" strip, then live, died, the open Session log and
// history by day. Each Command folds to one line; unfolding it shows its details, its child Runs and its Tickets
// as chips, and a chip unfolds that Ticket's details: two levels at most. Stop confirms inline under its row.
import { host, sessions } from "../data.js";
import {
  state, esc, rhm, age, clock, issueLink, status, liveCommands, attentionCommands, endedCommands,
  ticketCounts, stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog,
  mountLog, commandLogText, command, changed, canStop, stopEffectsHTML, usualText, isWorking, health, heartbeat,
  needsYou, usageText, sessionsOf, summaryText,
} from "../shared.js";

let confirming = null;

const badge = (key, text) => `<span class="b b-${key}">${esc(text)}</span>`;
const sessionLinks = (ids = []) => ids.length ? `<span class="links">${ids.map((id) => {
  const s = sessions[id];
  return `<a href="#log" data-session="${id}"${state.openSession === id ? ' aria-current="true"' : ""}>${esc(s.kind)}${s.live ? " ● live" : ""}</a>`;
}).join(" ")}</span>` : "";
const foldIn = (anc) => anc.length ? ` data-fold-in="${anc.join(" ")}"` : "";

// Health in words: silent is the stall (nothing logged for a while), slow only means past its usual range.
function healthBadge(r) {
  const h = health(r);
  if (h.key === "silent") return ` ${badge("silent", `silent · ${h.label}`)}`;
  if (h.key === "slow") return ` ${badge("slow", "slower than usual")}`;
  return "";
}
const healthCls = (r) => { const h = health(r); return h.key && h.key !== "ok" ? ` h-${h.key}` : ""; };
// Where a working Run is: stage · age, then its usual range here.
function stageCell(r) {
  if (!isWorking(r)) return "";
  const u = usualText(r.stage);
  return `${badge("live", r.stage)} ${age(r.stageSince)}${healthBadge(r)}${u ? `<div class="usual">${esc(u)}</div>` : ""}`;
}
// How long since it last logged anything: the heartbeat.
const pace = (r) => isWorking(r) ? `<div class="r-pace">${heartbeat(r)}${usualText(r.stage) ? ` · ${esc(r.stage)} ${esc(usualText(r.stage))} here` : ""}</div>` : "";
const tokens = (ids) => { const t = usageText(ids ?? []); return t ? `<div class="r-tokens">${esc(t)}</div>` : ""; };

// A Ticket in a few words, for its chip. Blocked Tickets say they cannot start yet (GOV.UK task list wording).
function ticketWord(t) {
  switch (t.state) {
    case "running": { const h = health(t); return `${esc(t.stage)} · ${age(t.stageSince)}${h.key === "slow" ? " · slower than usual" : h.key === "silent" ? " · silent" : ""}`; }
    case "merged": return "merged into the Spec branch";
    case "failed": return "failed";
    case "waiting": return `cannot start yet: waiting on ${esc(t.blockedBy.join(", "))}`;
    case "interrupted": return `interrupted at ${esc(t.stage)}`;
    default: return "not started";
  }
}
function ticketChips(tickets, anc) {
  const chips = tickets.map((t) => `<span class="tchip t-${t.state.replace(" ", "-")}${healthCls(t)}" data-fold="${t.id}"${t.state === "failed" ? ' data-fold-default="open"' : ""}>
    <b>${esc(t.issue)}</b> <span class="tw">${ticketWord(t)}</span></span>`).join("");
  const details = tickets.map((t) => `<div class="row-more tdetail" style="--d:2"${foldIn([...anc, t.id])}>
    <div class="r-name"><span class="kind">Ticket</span> ${issueLink(t.issue)} <span class="title">${esc(t.title)}</span></div>
    ${t.state === "failed" ? `<div class="r-detail cause">failed: ${esc(t.cause)}</div>` : ""}
    ${t.state === "waiting" ? `<div class="r-detail">Cannot start yet: it waits on ${t.blockedBy.map((b) => issueLink(b)).join(", ")}.</div>` : ""}
    ${t.harness ? `<div class="r-hme">${esc(rhm(t))}</div>` : ""}
    ${pace(t)}
    ${t.latest && t.state === "running" ? `<div class="r-latest"><code data-latest="${t.id}">${esc(t.latest)}</code></div>` : ""}
    ${t.pr || t.sessions?.length ? `<div class="r-latest">${t.pr ? `PR ${issueLink(t.pr)} ` : ""}${sessionLinks(t.sessions)}</div>` : ""}
    ${tokens(t.sessions)}
  </div>`).join("");
  return `<div class="chips" style="--d:2"${foldIn(anc)}><span class="muted chips-h">Tickets</span>${chips}</div>${details}`;
}

// A child Run under its Command's fold, shown whole: no fold of its own, so the Ticket chips are the only second level.
function childRows(r, anc) {
  const outcome = r.outcome ? badge(r.outcome, r.outcome) : "";
  const spec = r.tickets ? `<div class="r-detail">${r.tickets.length} Tickets · ${Object.entries(ticketCounts(r)).map(([k, v]) => `${v} ${k}`).join(" · ")} · Spec branch <code>${esc(r.specBranch)}</code></div>` : "";
  const out = [`<div class="row child" style="--d:1"${foldIn(anc)}>
    <div class="r-name"><span class="kind">${esc(r.kind)}</span> ${issueLink(r.issue)} <span class="title">${esc(r.title ?? "")}</span>
      ${spec}${r.cause ? `<div class="r-detail cause">${esc(r.cause)}</div>` : ""}
      ${r.harness ? `<div class="r-hme">${esc(rhm(r))}</div>` : ""}
      ${r.pr || r.sessions?.length ? `<div class="r-latest">${r.pr ? `PR ${issueLink(r.pr)} ` : ""}${sessionLinks(r.sessions)}</div>` : ""}
      ${tokens(r.sessions)}</div>
    <div class="r-stage">${outcome || (r.stage === "Tickets" ? `${badge("live", "Tickets")} ${age(r.stageSince)}` : stageCell(r))}</div>
  </div>`];
  if (r.tickets) out.push(ticketChips(r.tickets, anc));
  return out;
}

function commandBlock(c) {
  const s = status(c);
  const key = c.id;
  const urgent = c.outcome === "failed" || s.key === "died";
  let stage;
  if (s.key === "live") stage = c.stage ? stageCell(c) : `${badge("live", "live")} ${age(c.started)}`;
  else if (s.key === "died") stage = `${badge("died", s.label)} <span class="muted">${age(c.diedAt)} ago</span>`;
  else stage = badge(s.key, s.label);
  const act = canStop(c) ? `<button type="button" class="btn-stop" data-stop="${c.id}">Stop</button>`
    : s.key === "stopping" ? `<span class="muted">finishing salvage…</span>`
    : `<span class="muted">${c.ended ? clock(c.ended) : `started ${clock(c.started)}`}</span>`;
  // Folded, an ended Command still says what came of it: its cause, or its Run summary.
  const oneLine = c.outcome && (c.cause || c.summary) ? `<div class="r-detail${c.outcome === "failed" ? " cause" : ""}">${esc(c.cause ?? summaryText(c))}</div>` : "";
  const head = `<div class="row cmd${healthCls(c)}" data-fold="${key}" style="--d:0"${urgent ? ' data-fold-default="open"' : ""}>
    <div class="r-name"><span class="kind">${esc(c.kind)}</span> ${c.issue ? `${issueLink(c.issue)} <span class="title">${esc(c.title)}</span>` : `<span class="title">${esc(c.repo)}</span>`}${oneLine}</div>
    <div class="r-stage">${stage}</div>
    <div class="r-act">${act}</div>
  </div>`;
  const more = `<div class="row-more cmd-more" style="--d:0" data-fold-in="${key}">
    ${c.note ? `<div class="r-detail">${esc(c.note)}</div>` : ""}
    ${c.outcome && c.cause && c.summary ? `<div class="r-detail">${esc(summaryText(c))}</div>` : ""}
    <div class="r-hme">${esc(rhm(c))}</div>
    ${pace(c)}
    <div class="r-latest">${c.latest ? `<code${s.key === "live" ? ` data-latest="${c.id}"` : ""}>${esc(c.latest)}</code>` : ""}${c.pr ? ` PR ${issueLink(c.pr)}` : ""}
      ${c.commandLog ? `<span class="links"><a href="#log" data-cmdlog="${c.id}"${state.openCommandLog === c.id ? ' aria-current="true"' : ""}>Command log</a></span>` : ""} ${sessionLinks(c.sessions)}</div>
    ${tokens(sessionsOf(c))}
  </div>`;
  // The confirm sits outside every fold, so it shows even with the Command folded.
  const confirm = confirming === c.id && canStop(c) ? `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
    <p><b>Stop this ${esc(c.kind)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code>, not put back in line, so the Day shift looks at it first.</p>
    ${stopEffectsHTML(c)}
    <div class="confirm-actions">
      <button type="button" class="btn-stop" data-do-stop="${c.id}">Stop and label needs-triage</button>
      <button type="button" data-do-requeue="${c.id}">Stop and requeue instead</button>
      <button type="button" data-cancel>Cancel</button>
    </div>
    <p class="hint">Same as <code>${esc(stopHint(c))}</code> · requeue: <code>${esc(requeueHint(c))}</code></p>
  </div>` : "";
  const kids = [];
  for (const ch of c.children ?? []) kids.push(...childRows(ch, [key]));
  if (c.tickets) kids.push(ticketChips(c.tickets, [key]));
  const findings = c.findings ? `<div class="findings" style="--d:1" data-fold-in="${key}"><h4>Security findings</h4><ul>${c.findings.map((f) =>
    `<li>${badge(`sev-${f.sev}`, `${f.sev} severity`)} ${esc(f.title)} <span class="muted">· ${f.reproduced ? `reproduced · fixed by ${issueLink(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced · waits for the Day shift"}</span></li>`).join("")}</ul></div>` : "";
  return `<article class="cmd-block s-${s.key}" id="cmd-${c.id}">${head}${confirm}${more}${kids.join("")}${findings}</article>`;
}

// One item per problem, urgent first (SRE: page on symptoms that need a human). No h2, so it never folds away.
function needsStrip() {
  const items = needsYou();
  if (!items.length) return `<div class="needs none" role="region" aria-label="Needs you"><p class="needs-h"><b>Nothing needs you.</b> <span class="muted">Every Command is working or done.</span></p></div>`;
  return `<div class="needs" role="region" aria-label="Needs you">
    <p class="needs-h"><b>Needs you</b> <span class="count">${items.length}</span> <span class="muted">urgent first</span></p>
    <ul>${items.map((it) => `<li><button type="button" class="need need-${it.level}" data-need-cmd="${it.cmd.id}" data-need-run="${it.run.id}">
      <span class="lvl">${it.level === "urgent" ? "Urgent" : "Look"}</span>
      <span class="what"><b>${esc(it.ref)}</b> ${esc(it.what)}</span>
      <span class="why">${esc(it.why)}</span></button></li>`).join("")}</ul>
  </div>`;
}

// Opens whatever a "Needs you" item names: its section, its Command's fold, its Ticket's chip; then shows it.
// Sections and folds belong to app.js, so this clicks their toggles rather than reaching into their state.
export function revealTarget(sel, foldKeys) {
  const app = document.getElementById("app");
  let el = app.querySelector(sel);
  if (!el) return;
  for (let shut = el.closest(".proto-shut"); shut; shut = el.closest(".proto-shut")) {
    let head = shut.previousElementSibling;
    while (head && !head.classList.contains("proto-toggle")) head = head.previousElementSibling;
    if (!head) break;
    (head.matches("h2") ? head : head.querySelector("h2")).click();
  }
  for (const k of foldKeys) {
    const t = app.querySelector(`[data-fold="${k}"]`);
    if (t && !t.classList.contains("is-unfolded")) t.click();
  }
  el = app.querySelector(sel);
  el.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  el.classList.remove("is-target"); void el.offsetWidth; el.classList.add("is-target");
}

function logSection() {
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    return `<section id="log" class="log"><div class="log-head"><h2>Command log</h2><span class="muted">${esc(c.commandLog)} · plain text</span></div><pre class="cmdlog">${esc(commandLogText(c))}</pre></section>`;
  }
  const s = sessions[state.openSession];
  if (!s) return "";
  return `<section id="log" class="log"><div class="log-head"><h2>Session log</h2><span>${esc(s.owner)} · ${esc(s.kind)} · ${esc(s.harness)}</span>${s.live ? badge("live", "live") : ""}<span class="muted file">${esc(s.file ?? "")}</span></div><div class="log-body"></div></section>`;
}

// The latest event line flashes once each time the live session changes it.
export function flashOnChange(root) {
  new MutationObserver((ms) => {
    for (const m of ms) {
      const el = m.target.closest?.("[data-latest]") ?? m.target.parentElement?.closest?.("[data-latest]");
      if (!el) continue;
      el.classList.remove("flash");
      void el.offsetWidth;
      el.classList.add("flash");
    }
  }).observe(root, { subtree: true, childList: true, characterData: true });
}

function signInPage(root) {
  root.innerHTML = `<div class="plain signin"><form>
    <h1>${esc(host.name)} <span class="muted">Console</span></h1>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Sign in</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  const days = new Map();
  for (const c of endedCommands().sort((a, b) => b.ended - a.ended)) {
    const k = clock(c.ended).includes(" ") ? clock(c.ended).split(" ")[0] : "today";
    if (!days.has(k)) days.set(k, []);
    days.get(k).push(c);
  }
  root.innerHTML = `<div class="plain">
    <header class="top"><h1>${esc(host.name)} <span class="muted">Console</span></h1>
      <nav><a href="${host.other.url}">${esc(host.other.name)} →</a> <button type="button" class="link" data-signout>Sign out</button></nav></header>
    ${needsStrip()}
    <section><h2>Live <span class="count">${liveCommands().length}</span></h2>${liveCommands().map(commandBlock).join("")}</section>
    ${attentionCommands().length ? `<section><h2>Died <span class="count">${attentionCommands().length}</span></h2><p class="muted sec-note">The process is gone with no outcome.</p>${attentionCommands().map(commandBlock).join("")}</section>` : ""}
    ${logSection()}
    <section><h2>History</h2>${[...days].map(([d, cs]) => `<h3>${esc(d)}</h3>${cs.map(commandBlock).join("")}`).join("")}
      <p class="muted end">Back to the oldest logs on this machine.</p></section>
  </div>`;
  const body = root.querySelector(".log-body");
  if (body) mountLog(body, state.openSession);
  flashOnChange(root);
  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a");
    if (!t) return;
    const d = t.dataset;
    if (d.needCmd) {
      // A Ticket's item opens its chip too; anything else opens its Command.
      const chip = d.needRun !== d.needCmd && document.querySelector(`#app .tchip[data-fold="${d.needRun}"]`);
      revealTarget(chip ? `.tchip[data-fold="${d.needRun}"]` : `#cmd-${d.needCmd}`, chip ? [d.needCmd, d.needRun] : [d.needCmd]);
    }
    else if (d.stop) { confirming = d.stop; changed(); }
    else if (d.doStop) { confirming = null; stopCommand(d.doStop, false); }
    else if (d.doRequeue) { confirming = null; stopCommand(d.doRequeue, true); }
    else if ("cancel" in d) { confirming = null; changed(); }
    else if ("signout" in d) signOut();
    else if (d.session) { e.preventDefault(); openSession(d.session); document.getElementById("log")?.scrollIntoView(); }
    else if (d.cmdlog) { e.preventDefault(); openCommandLog(d.cmdlog); document.getElementById("log")?.scrollIntoView(); }
  });
}
