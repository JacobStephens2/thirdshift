// PROTOTYPE variant "press": the Console as the press floor of thirdshift.app. Each live Run is a press line,
// one unit per stage, with the current unit lit and its age in large type; a Spec run's Tickets are sheets in
// the feeder, on the press or on the delivery pile. Died Commands are misprints, history is the delivery pile,
// and the Session log plays in the press terminal.
import { host, sessions, RUN_STAGES, SPEC_STAGES } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, liveCommands, attentionCommands, endedCommands,
  ticketLine, stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog,
  mountLog, commandLogText, command, changed, canStop, stopEffectsHTML, usualText, health, heartbeat,
  needsYou, usageText, sessionsOf, summaryText,
} from "../shared.js";

let confirming = null;

const INKS = ["c", "m", "y", "k"];
const regmark = `<svg class="reg" viewBox="0 0 28 28" aria-hidden="true"><circle cx="14" cy="14" r="8" fill="none" stroke="currentColor" stroke-width="1.2"/><circle cx="14" cy="14" r="3.5" fill="currentColor"/><path d="M14 0v28M0 14h28" stroke="currentColor" stroke-width="1.2"/></svg>`;
const colorbar = `<div class="colorbar" aria-hidden="true"><i class="cb-c"></i><i class="cb-m"></i><i class="cb-y"></i><i class="cb-k"></i><i class="cb-c50"></i><i class="cb-m50"></i><i class="cb-y50"></i></div>`;

const sessionLinks = (ids = []) => ids.map((id) => {
  const s = sessions[id];
  return `<a href="#log" data-session="${id}"${state.openSession === id ? ' aria-current="true"' : ""}>${esc(s.kind)}${s.live ? ' <b class="on-air">●</b>' : ""}</a>`;
}).join("");
const links = (r, cmdlogId) => {
  const parts = [];
  if (cmdlogId) parts.push(`<a href="#log" data-cmdlog="${cmdlogId}"${state.openCommandLog === cmdlogId ? ' aria-current="true"' : ""}>Command log</a>`);
  if (r.sessions?.length) parts.push(sessionLinks(r.sessions));
  return parts.length ? `<nav class="reels" aria-label="Logs">${parts.join("")}</nav>` : "";
};

// A press line: one unit per stage. jam marks the unit where a dead process stopped.
function pressLine(r, stages, { mini = false, jam = false } = {}) {
  const cur = stages.indexOf(r.stage);
  const lit = cur >= 0;
  // A stall is silence, not stage age: health() says silent (nothing logged lately) or slow (past its usual range).
  const h = lit && !jam ? health(r) : { key: "" };
  const stalled = h.key === "silent";
  const units = stages.map((st, i) => {
    const ink = INKS[i % 4];
    const cls = i < cur ? "done" : i === cur ? (jam ? "jam" : "lit") : "next";
    const big = i === cur
      ? `<span class="u-age">${jam ? "jammed" : age(r.stageSince)}</span>${jam ? "" : healthTag(h)}${!jam && usualText(st) ? `<span class="u-usual">${esc(usualText(st))}</span>` : ""}${!jam ? heartbeat(r) : ""}`
      : `<span class="u-state">${i < cur ? "✓ done" : "—"}</span>`;
    const rollers = cls === "lit" ? `<span class="u-rollers" aria-hidden="true"><i></i><i></i><i></i>${mini ? "" : "<b></b>"}</span>` : "";
    return `<li class="unit ink-${ink} ${cls}"${i === cur ? ' aria-current="step"' : ""}>
      ${mini ? "" : `<span class="u-no">unit ${i + 1}</span>`}
      <span class="u-plate${i === cur ? " ht" : ""}"><span class="u-letter misreg">${ink.toUpperCase()}</span></span>
      <span class="u-name">${esc(st)}</span>${big}${rollers}</li>`;
  }).join("");
  const side = mini ? "" : `<li class="unit side"><span class="u-no">off the line</span><span class="u-name">Repair · Resume</span><span class="u-state">idle</span></li>`;
  // The belt runs under the stage units only, so the sheet stops at the middle of the lit one.
  const belt = lit && !jam ? `<div class="belt${mini ? " mini" : ""}" aria-hidden="true" style="--to:${((cur + 0.5) / stages.length).toFixed(3)};--cols:${mini ? 6 : 6.8}"><i class="belt-sheet"></i></div>` : "";
  return `<ol class="pline${mini ? " mini" : ""}${stalled ? " is-stalled" : ""}" aria-label="Stages">${units}${side}</ol>${belt}`;
}

// Words with every colour: silent is the alarm, slow is a note, on pace says so.
const healthTag = (h) => h.key === "silent" ? `<span class="u-stall silent">silent · ${esc(h.label)}</span>`
  : h.key === "slow" ? `<span class="u-stall slow">slow · still printing</span>`
  : h.key === "ok" ? `<span class="u-ok">on pace</span>` : "";
const stderr = (r, label = "latest") => r.latest
  ? `<ol class="stderr-lines"><li><span class="prefix">${label}:</span> <span data-latest="${r.id}">${esc(r.latest)}</span></li></ol>` : "";

const stopButton = (c) => c.stopping ? `<span class="stopping">Stopping… salvage finishing</span>`
  : canStop(c) && confirming !== c.id ? `<button type="button" class="btn-stop" data-stop="${c.id}">Stop</button>` : "";
function stopControls(c) {
  if (!canStop(c) || confirming !== c.id) return "";
  return `<div class="stop-sheet" role="alertdialog" aria-label="Confirm Stop">
    <p class="eyebrow">Stop the press?</p>
    <p>Stopping this ${esc(c.kind)} labels <b>${esc(c.stopTarget)}</b> <code>needs-triage</code>. It is <b>not</b> put back in line: the Day shift looks at it first.</p>
    <p class="eyebrow">What still runs to the end</p>${stopEffectsHTML(c)}
    <div class="stop-actions">
      <button type="button" class="ink-btn" data-do-stop="${c.id}">Stop · needs-triage</button>
      <button type="button" class="ghost-btn" data-do-requeue="${c.id}">Requeue instead</button>
      <button type="button" class="ghost-btn" data-cancel>Cancel</button>
    </div>
    <div class="cli"><code>${esc(stopHint(c))}</code><code>${esc(requeueHint(c))}</code></div>
  </div>`;
}

// A Ticket sheet shows everything once its press line is open: at most two levels (NN/g), the line and its sheets.
function sheet(t) {
  const head = (stateHTML) => `<div class="sh-head"><span class="sh-issue">${issueLink(t.issue)}</span> <span class="sh-state">${stateHTML}</span></div>`;
  const more = (inner) => `<div class="sh-more"><p class="sh-title">${esc(t.title)}</p>${inner}</div>`;
  const tokens = usageText(t.sessions ?? []);
  if (t.state === "running") return `<li class="sheet running">${head(`<b>${esc(t.stage)}</b> · ${age(t.stageSince)}`)}
    ${more(`<p class="hme">${esc(rhm(t))}</p>${pressLine(t, RUN_STAGES, { mini: true })}${stderr(t)}${tokens ? `<p class="tokens">${esc(tokens)}</p>` : ""}${links(t)}`)}</li>`;
  return `<li class="sheet st-${t.state.replace(" ", "-")}">${head(esc(ticketLine(t)))}${more(`${t.pr ? `<p class="sh-pr">PR ${issueLink(t.pr)}</p>` : ""}${tokens ? `<p class="tokens">${esc(tokens)}</p>` : ""}${links(t)}`)}</li>`;
}

function specRun(sp) {
  const feeder = sp.tickets.filter((t) => t.state === "waiting" || t.state === "not started");
  const onPress = sp.tickets.filter((t) => t.state === "running");
  const pile = sp.tickets.filter((t) => !["waiting", "not started", "running"].includes(t.state));
  const bin = (title, note, list) => `<div class="bin"><p class="eyebrow">${title}</p><p class="bin-note">${note}</p>
    <ul class="sheets">${list.length ? list.map(sheet).join("") : '<li class="sheet empty">empty</li>'}</ul></div>`;
  return `<div class="spec">
    <p class="line-head"><span class="kind">Spec run</span> ${issueLink(sp.issue)} <span class="title">${esc(sp.title)}</span> <span class="hme">Spec branch <code>${esc(sp.specBranch)}</code></span></p>
    ${pressLine(sp, SPEC_STAGES)}
    <div class="bins">
      ${bin("Feeder", "Tickets waiting or not started", feeder)}
      ${bin("On the press", "Ticket Runs in flight", onPress)}
      ${bin("Delivery pile", `merged into ${esc(sp.specBranch)}`, pile)}
    </div></div>`;
}

// What a folded line still says: each lit unit's stage and age, the Spec run's running Tickets included.
function litSummary(c) {
  const lit = [];
  const add = (r, stages) => { const i = stages.indexOf(r.stage); if (i >= 0) lit.push({ r, ink: INKS[i % 4] }); };
  if (c.stage) add(c, RUN_STAGES);
  for (const ch of c.children ?? []) {
    add(ch, ch.tickets ? SPEC_STAGES : RUN_STAGES);
    for (const t of ch.tickets ?? []) if (t.state === "running") add(t, RUN_STAGES);
  }
  const chips = lit.map(({ r, ink }) => {
    const h = health(r);
    return `<span class="lit-chip ink-${ink}${h.key === "silent" ? " is-stalled" : ""}"><i class="ht"></i>${r.issue ? `${esc(r.issue)} · ` : ""}<b>${esc(r.stage)}</b> · ${age(r.stageSince)}${healthTag(h)}</span>`;
  });
  // Every other Ticket as a chip too, so the folded line already says where each one is.
  for (const ch of c.children ?? []) for (const t of ch.tickets ?? []) if (t.state !== "running")
    chips.push(`<span class="ticket-chip st-${t.state.replace(" ", "-")}">${esc(t.issue)} · ${esc(t.state === "waiting" ? `waiting on ${t.blockedBy.join(", ")}` : t.state)}</span>`);
  return chips.length ? `<p class="lit-summary">${chips.join("")}</p>` : "";
}
function liveJob(c) {
  const key = `job-${c.id}`;
  const head = `<header class="job-head" data-fold="${key}">
    <div><p class="eyebrow">${esc(c.kind)} · ${esc(c.repo)} · started ${clock(c.started)} · ${age(c.started)} on the press</p>
      <h3>${c.issue ? `${issueLink(c.issue)} ${esc(c.title)}` : esc(c.note ?? c.repo)}</h3>
      ${litSummary(c)}</div>
    <div class="job-stop">${stopButton(c)}</div></header>${stopControls(c)}`;
  const tokens = usageText(sessionsOf(c));
  let body = `<p class="hme">${esc(rhm(c))}${c.merge ? " · Merge run" : ""}${c.pr ? ` · PR ${issueLink(c.pr)}` : ""}</p>${tokens ? `<p class="tokens">${esc(tokens)} so far</p>` : ""}${links(c, c.commandLog ? c.id : null)}`;
  if (c.stage) body += pressLine(c, RUN_STAGES) + stderr(c);
  for (const ch of c.children ?? []) body += ch.tickets ? specRun(ch) : pressLine(ch, RUN_STAGES) + stderr(ch);
  return `<article class="job" id="press-${c.id}">${head}<div class="job-body" data-fold-in="${key}">${body}</div></article>`;
}

function misprint(c) {
  const key = `misprint-${c.id}`;
  return `<article class="misprint" id="press-${c.id}">
    <div class="mp-head" data-fold="${key}" data-fold-default="open"><h3 class="misreg">Died at ${esc(c.stage)}</h3>
      <p><span class="kind">${esc(c.kind)}</span> ${issueLink(c.issue)} ${esc(c.title)} · ${age(c.diedAt)} ago · ${heartbeat(c)}</p></div>
    <div class="mp-more" data-fold-in="${key}">
    <p class="eyebrow">Misprint · the process is gone, with no outcome</p>
    <p class="hme">${esc(rhm(c))}</p>
    ${pressLine(c, RUN_STAGES, { mini: true, jam: true })}
    <ol class="stderr-lines"><li class="failed"><span class="prefix">last tool call, ${age(c.diedAt)} ago:</span> ${esc(c.latest)} <span class="sl-st sl-err">no result</span></li><li class="failed">(the log ends here: nothing has printed since)</li></ol>
    ${usageText(sessionsOf(c)) ? `<p class="tokens">${esc(usageText(sessionsOf(c)))}</p>` : ""}
    ${links(c, c.commandLog ? c.id : null)}</div></article>`;
}

function terminal() {
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    const lines = commandLogText(c).split("\n").map((l, i, all) => {
      const m = l.match(/^(thirdshift: \S+)(.*)$/);
      const cls = i === all.length - 1 ? (c.outcome === "merged" || c.outcome === "ready" ? "ready" : c.outcome ? "failed" : "") : "";
      return `<li class="${cls}">${m ? `<span class="prefix">${esc(m[1])}</span>${esc(m[2])}` : esc(l)}</li>`;
    }).join("");
    return `<section id="log" class="press-term"><div class="term-bar term-head"><span class="term-titleline"><h2 class="term-title">Command log</h2> · stderr · ${esc(c.commandLog)}</span><span>plain text</span></div>
      <ol class="stderr-lines term-body">${lines}</ol></section>`;
  }
  const s = sessions[state.openSession];
  if (!s) return "";
  return `<section id="log" class="press-term${s.live ? " is-live" : ""}"><div class="term-bar term-head"><span class="term-titleline"><h2 class="term-title">Session log</h2> · ${esc(s.owner)} · ${esc(s.kind)} · ${esc(s.harness)}${s.file ? ` · <span class="file">${esc(s.file)}</span>` : ""}</span>${s.live ? '<span class="on-air">● live</span>' : "<span>ended</span>"}</div>
    <div class="term-body"></div></section>`;
}

function childLines(c) {
  const rows = [];
  const add = (r, depth) => rows.push(`<li style="--d:${depth}"><span class="kind">${esc(r.kind ?? "Ticket")}</span> ${issueLink(r.issue)} ${esc(r.title ?? "")} — <b class="o-${(r.state ?? r.outcome ?? "").replace(" ", "-")}">${esc(r.state ? ticketLine(r) : r.outcome)}</b>${r.pr ? ` · PR ${issueLink(r.pr)}` : ""}${r.cause && !r.state ? ` <span class="cause">${esc(r.cause)}</span>` : ""} ${links(r)}</li>`);
  for (const ch of c.children ?? []) { add(ch, 0); for (const t of ch.tickets ?? []) add(t, 1); }
  for (const t of c.tickets ?? []) add(t, 0);
  return rows.length ? `<ul class="kids">${rows.join("")}</ul>` : "";
}

function proofCard(c) {
  const took = c.ended && c.started ? dur(c.ended - c.started) : "";
  const findings = c.findings ? `<ul class="findings">${c.findings.map((f) => `<li><b class="sev sev-${f.sev}">${esc(f.sev)}</b> ${esc(f.title)} <span class="cause">${f.reproduced ? `reproduced · fixed by ${issueLink(f.fix)}, ${esc(f.fixOutcome)}` : "not reproduced · waits for the Day shift"}</span></li>`).join("")}</ul>` : "";
  const key = `proof-${c.id}`;
  const summary = summaryText(c);
  const tokens = usageText(sessionsOf(c));
  return `<li class="proof-card out-${c.outcome}" id="press-${c.id}">
    <div class="pc-head" data-fold="${key}"${c.outcome === "failed" ? ' data-fold-default="open"' : ""}><p class="stamp">${esc(c.outcome)}</p>
      <h3>${c.issue ? `${issueLink(c.issue)} ${esc(c.title)}` : `${esc(c.kind)} · ${esc(c.repo)}`}</h3>
      ${summary ? `<p class="run-summary">${esc(summary)}</p>` : ""}</div>
    <div class="pc-more" data-fold-in="${key}">
    <p class="slug">${esc(c.kind)} · ${esc(c.repo)} · ${clock(c.ended)}${took ? ` · ran ${took}` : ""}${c.pr ? ` · PR ${issueLink(c.pr)}` : ""}</p>
    ${c.cause ? `<p class="cause">${esc(c.cause)}</p>` : ""}${c.note ? `<p class="cause">${esc(c.note)}</p>` : ""}
    <p class="hme">${esc(rhm(c))}</p>${tokens ? `<p class="tokens">${esc(tokens)}</p>` : ""}${childLines(c)}${findings}${links(c, c.commandLog ? c.id : null)}</div></li>`;
}

// The alarm board: one job ticket per problem, most urgent first, above the floor. Choosing one opens its line.
function needsBoard() {
  const items = needsYou();
  if (!items.length) return `<aside class="board quiet" aria-label="Needs you"><p class="eyebrow">Needs you</p><p>Nothing. The floor runs itself.</p></aside>`;
  return `<aside class="board" aria-label="Needs you"><p class="eyebrow">Needs you · ${items.length}</p>
    <ol class="tickets">${items.map((it) => `<li class="ticket lv-${it.level}"><button type="button" class="ticket-btn" data-jump="${it.cmd.id}">
      <span class="lv">${it.level === "urgent" ? "Stop the line" : "Have a look"}</span>
      <b class="ref">${esc(it.ref)}</b> <span class="what">${esc(it.what)}</span>
      <span class="why">${esc(it.why)}</span></button></li>`).join("")}</ol></aside>`;
}
// Opens the section and the fold holding a Command, then scrolls to it.
function jumpTo(root, id) {
  const el = root.querySelector(`#press-${CSS.escape(id)}`);
  if (!el) return;
  if (el.closest(".proto-shut")) el.closest("section")?.querySelector("h2[role=button]")?.click();
  const t = el.querySelector("[data-fold]");
  if (t && t.getAttribute("aria-expanded") !== "true") t.click();
  el.scrollIntoView({ block: "start", behavior: "smooth" });
}

function signInPage(root) {
  root.innerHTML = `<div class="press-floor"><div class="sheet-wrap signin">${regmark.replace('class="reg"', 'class="reg l"')}${regmark.replace('class="reg"', 'class="reg r"')}
    <form class="paper-sheet">
      <div class="topline"><span class="slug">thirdshift · press floor · Console</span><span class="slug">${esc(host.name)}</span></div>
      <h1>${esc(host.name)}</h1>
      <label class="eyebrow" for="pw">Password</label>
      <div class="install"><input id="pw" type="password" autocomplete="current-password" autofocus><button type="submit">Sign in</button></div>
      ${colorbar}
    </form></div></div>`;
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
  const live = liveCommands(), died = attentionCommands();
  root.innerHTML = `<div class="press-floor">
    <header class="floor-head">
      <div class="topline"><span class="slug">${regmark} thirdshift · press floor · Console</span>
        <nav><a href="${host.other.url}">${esc(host.other.name)}’s press floor →</a><button type="button" class="link" data-signout>Sign out</button></nav></div>
      <h1 class="misreg">${esc(host.name)}</h1>
      ${colorbar}
    </header>
    ${needsBoard()}
    <section class="sec grid-bg"><div class="sec-head"><p class="eyebrow">${live.length} Command${live.length === 1 ? "" : "s"} running</p><h2 class="misreg">On the press</h2></div>
      ${live.length ? live.map(liveJob).join("") : '<p class="empty-floor">The press is quiet.</p>'}</section>
    ${died.length ? `<section class="sec"><div class="sec-head"><p class="eyebrow">Needs a look</p><h2 class="misreg">Misprints</h2></div>${died.map(misprint).join("")}</section>` : ""}
    ${terminal()}
    <section class="sec"><div class="sec-head"><p class="eyebrow">History, back to the oldest logs</p><h2 class="misreg">Delivery pile</h2></div>
      ${[...days].map(([d, cs]) => `<h4 class="day">${esc(d)}</h4><ul class="proofs">${cs.map(proofCard).join("")}</ul>`).join("")}</section>
  </div>`;
  new MutationObserver((muts) => {
    for (const m of muts) {
      const el = m.target.closest?.("[data-latest]");
      if (!el) continue;
      el.classList.remove("ink-flash");
      void el.offsetWidth;
      el.classList.add("ink-flash");
    }
  }).observe(root, { childList: true, characterData: true, subtree: true });
  const body = root.querySelector(".press-term .term-body:not(ol)");
  if (body) mountLog(body, state.openSession);
  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a");
    if (!t) return;
    if (t.dataset.jump) jumpTo(root, t.dataset.jump);
    else if (t.dataset.stop) { confirming = t.dataset.stop; changed(); }
    else if (t.dataset.doStop) { confirming = null; stopCommand(t.dataset.doStop, false); }
    else if (t.dataset.doRequeue) { confirming = null; stopCommand(t.dataset.doRequeue, true); }
    else if ("cancel" in t.dataset) { confirming = null; changed(); }
    else if ("signout" in t.dataset) signOut();
    else if (t.dataset.session) { e.preventDefault(); openSession(t.dataset.session); document.getElementById("log")?.scrollIntoView(); }
    else if (t.dataset.cmdlog) { e.preventDefault(); openCommandLog(t.dataset.cmdlog); document.getElementById("log")?.scrollIntoView(); }
  });
}
