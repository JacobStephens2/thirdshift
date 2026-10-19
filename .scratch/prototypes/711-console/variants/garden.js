// PROTOTYPE variant "garden": fully themed. Live Commands are raised beds, a Spec run's Tickets are plants whose
// growth shows their state, the open Session log is a field notebook beside the beds, and history is a harvest log.
import { host, sessions, RUN_STAGES, NOW0 } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, liveCommands, attentionCommands, endedCommands,
  ticketLine, stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog,
  mountLog, commandLogText, command, changed, canStop, stopEffectsHTML, usualText, heartbeat, health, needsYou,
  usageText, sessionsOf, summaryText,
} from "../shared.js";

let confirming = null;

// ---------- Drawings ----------
// Every plant stands on the same soil line, so a row of them reads as one bed.
const soil = `<path class="p-soil" d="M2 60 Q30 56 58 60 L58 68 L2 68 Z"/>`;
const leaf = (x, y, flip, cls = "p-leaf") => `<path class="${cls}" d="M${x} ${y} q${flip ? -12 : 12} -2 ${flip ? -14 : 14} -10 q${flip ? 8 : -8} -1 ${flip ? 14 : -14} 10 z"/>`;
function plant(st, stage) {
  let body = "";
  switch (st) {
    case "not started":
      body = `<rect class="p-packet" x="17" y="16" width="26" height="38" rx="2"/><path class="p-packet-fold" d="M17 22 h26"/>
        <circle class="p-seed" cx="25" cy="34" r="2"/><circle class="p-seed" cx="33" cy="38" r="2"/><circle class="p-seed" cx="28" cy="44" r="2"/>`;
      break;
    case "waiting":
      body = `<ellipse class="p-seed" cx="24" cy="59" rx="4" ry="3"/><path class="p-stake" d="M44 62 V26"/>
        <path class="p-twine" d="M44 30 Q36 34 33 40"/><rect class="p-tag" x="26" y="38" width="14" height="10" rx="1" transform="rotate(-12 33 43)"/>`;
      break;
    case "running": {
      const i = Math.max(0, RUN_STAGES.indexOf(stage));
      const top = 54 - (12 + i * 6);
      body = `<g class="p-sway"><path class="p-stem" d="M30 60 V${top}"/>${leaf(30, 54, false)}${i >= 1 ? leaf(30, 46, true) : ""}${i >= 2 ? leaf(30, 40, false) : ""}${i >= 3 ? leaf(30, 34, true) : ""}
        ${i >= 1 ? `<ellipse class="p-bud" cx="30" cy="${top - 3}" rx="3.5" ry="5"/>` : `<path class="p-leaf" d="M30 ${top} q-6 -6 -2 -10 q4 2 2 10z"/>`}</g>`;
      break;
    }
    case "merged":
      body = `<path class="p-stem" d="M30 60 V22"/>${leaf(30, 54, false)}${leaf(30, 46, true)}${leaf(30, 38, false)}
        <g class="p-flower">${[0, 72, 144, 216, 288].map((a) => `<ellipse class="p-petal" cx="30" cy="12" rx="5" ry="8" transform="rotate(${a} 30 20)"/>`).join("")}<circle class="p-heart" cx="30" cy="20" r="4"/></g>`;
      break;
    case "failed":
      body = `<g class="p-droop"><path class="p-stem p-dead" d="M30 60 Q31 36 40 30 Q46 30 44 40"/>${leaf(30, 52, false, "p-leaf p-dead")}${leaf(30, 46, true, "p-leaf p-dead")}<circle class="p-spot" cx="38" cy="46" r="1.5"/><circle class="p-spot" cx="22" cy="40" r="1.5"/>
        <ellipse class="p-bud p-dead" cx="44" cy="42" rx="3" ry="4.5"/></g>`;
      break;
    case "interrupted":
      body = `<path class="p-stem" d="M30 60 V36"/>${leaf(30, 54, false)}${leaf(30, 46, true)}<path class="p-cut" d="M26 37 L34 35"/>`;
      break;
  }
  return `<svg class="plant" viewBox="0 0 60 70" aria-hidden="true">${soil}${body}</svg>`;
}
const GLYPH = {
  merged: `<circle class="g-bloom" cx="12" cy="9" r="5"/><path class="g-stem" d="M12 14 V22"/><path class="g-basket" d="M3 16 h18 l-3 7 h-12 z"/>`,
  ready: `<circle class="g-bloom" cx="12" cy="9" r="5"/><path class="g-stem" d="M12 14 V22"/><path class="g-basket" d="M3 16 h18 l-3 7 h-12 z"/>`,
  failed: `<path class="g-blight" d="M4 20 C4 8 14 3 21 3 C21 12 16 20 4 20 Z"/><circle class="g-spot" cx="12" cy="12" r="1.6"/><circle class="g-spot" cx="16" cy="8" r="1.2"/><circle class="g-spot" cx="9" cy="16" r="1.2"/>`,
  interrupted: `<path class="g-frost" d="M12 2 V22 M3.3 7 L20.7 17 M3.3 17 L20.7 7 M12 2 l-2 2 M12 2 l2 2 M12 22 l-2 -2 M12 22 l2 -2"/>`,
  skipped: `<path class="g-fallow" d="M2 10 Q12 6 22 10 M2 15 Q12 11 22 15 M2 20 Q12 16 22 20"/>`,
  died: `<path class="g-dead" d="M12 22 V10 Q12 4 18 6 M12 14 Q6 12 5 16"/>`,
};
const glyph = (k) => `<svg class="glyph g-${k}" viewBox="0 0 24 24" aria-hidden="true">${GLYPH[k] ?? GLYPH.skipped}</svg>`;
const OUTCOME = { merged: "Harvested · merged", ready: "Harvested · ready for review", failed: "Blight · failed", interrupted: "Frost · interrupted", skipped: "Fallow · skipped" };

// ---------- Pieces ----------
// How a growing Run is doing, in words: silence is the stall (it may be stuck); a stage past its usual range is only
// slow while its log still moves.
function vigour(r) {
  const h = health(r);
  if (h.key === "silent") return `<p class="vigour v-silent"><span class="v-mark" aria-hidden="true">!</span> ${esc(h.label)}; it may be stuck</p>`;
  if (h.key === "slow") return `<p class="vigour v-slow"><span class="v-mark" aria-hidden="true">~</span> slower than usual, still growing</p>`;
  return h.key ? `<p class="vigour v-ok">on pace</p>` : "";
}
// Stage · age, then what is usual for that stage here and when the log last moved.
const pace = (r) => `<p class="plot-state"><b>${esc(r.stage)}</b> · ${age(r.stageSince)}${usualText(r.stage) ? ` <span class="usual">(${esc(usualText(r.stage))})</span>` : ""}</p>
  ${vigour(r)}<p class="hb-line">${heartbeat(r)}</p>`;
const spent = (ids) => { const t = usageText(ids); return t ? `<p class="spent"><span>Spent:</span> ${esc(t)}</p>` : ""; };
const logLinks = (ids = [], cmdLog) => {
  const links = ids.map((id) => {
    const s = sessions[id];
    return `<a href="#log" data-session="${id}"${state.openSession === id ? ' aria-current="true"' : ""}>${esc(s.kind)}${s.live ? ' <span class="growing-dot" title="live"></span>' : ""}</a>`;
  });
  if (cmdLog) links.unshift(`<a href="#log" data-cmdlog="${cmdLog}"${state.openCommandLog === cmdLog ? ' aria-current="true"' : ""}>Command log</a>`);
  return links.length ? `<p class="notes-links"><span>Notes:</span> ${links.join(" ")}</p>` : "";
};
function growth(stage) {
  const i = RUN_STAGES.indexOf(stage);
  return `<ol class="growth" aria-label="Stages">${RUN_STAGES.map((s, n) => `<li class="${n < i ? "done" : n === i ? "now" : ""}">${esc(s)}</li>`).join("")}</ol>`;
}
const WORD = { merged: "bloomed", running: "growing", waiting: "waiting", failed: "wilted", "not started": "not sown", interrupted: "cut" };
function ticketPlant(t) {
  const running = t.state === "running";
  const h = health(t).key;
  return `<li class="plot t-${t.state.replace(" ", "-")}${h ? ` h-${h}` : ""}">
    ${plant(t.state, t.stage)}
    <div class="plot-card">
      <p class="ref">${issueLink(t.issue)} <span class="word">${esc(WORD[t.state] ?? t.state)}</span></p>
      <p class="plot-title">${esc(t.title)}</p>
      ${running ? pace(t) : `<p class="plot-state">${t.state === "waiting" ? `<span class="tag">waiting on ${t.blockedBy.map((b) => issueLink(b)).join(", ")}</span>` : esc(ticketLine(t))}</p>`}
      ${running ? `<p class="hme">${esc(rhm(t))}</p><p class="latest"><span aria-hidden="true">↳</span> <code data-latest="${t.id}">${esc(t.latest)}</code></p>` : ""}
      ${t.pr ? `<p class="hme">PR ${issueLink(t.pr)}</p>` : ""}
      ${spent(t.sessions ?? [])}
      ${t.sessions?.length ? logLinks(t.sessions) : ""}
    </div>
  </li>`;
}
// What a folded bed says on its sign: each Ticket as a chip with words, or the Run's stage, age and vigour, so a
// stall shows without opening it.
function chip(t) {
  const h = health(t);
  const what = t.state === "running" ? `${esc(t.stage)} · ${age(t.stageSince)}${h.key === "slow" ? " · slower than usual" : h.key === "silent" ? ` · ${esc(h.label)}` : ""}`
    : t.state === "waiting" ? `waiting on ${t.blockedBy.map(esc).join(", ")}` : esc(ticketLine(t));
  return `<li class="chip c-${t.state.replace(" ", "-")}${h.key ? ` h-${h.key}` : ""}"><span class="ref">${esc(t.issue)}</span> <b>${esc(WORD[t.state] ?? t.state)}</b> <span>${what}</span></li>`;
}
function bedSummary(c) {
  const parts = [];
  for (const ch of c.children ?? []) if (ch.tickets) parts.push(`<p class="bed-summary">Spec run ${esc(ch.issue)} · ${ch.tickets.length} Tickets</p><ul class="chips">${ch.tickets.map(chip).join("")}</ul>`);
  if (c.stage) {
    const h = health(c);
    parts.push(`<p class="bed-summary"><b>${esc(c.stage)}</b> · ${age(c.stageSince)}${h.key === "silent" ? ` · <span class="v-silent">! ${esc(h.label)}</span>` : h.key === "slow" ? " · slower than usual" : ""}</p>`);
  }
  return parts.join("");
}
const stopButton = (c) => c.stopping ? `<span class="stopping">stopping…</span>` : canStop(c) && confirming !== c.id ? `<button type="button" class="pull" data-stop="${c.id}">Stop</button>` : "";
function stopConfirm(c) {
  if (confirming !== c.id || !canStop(c)) return "";
  return `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
    <p><b>Pull up this ${esc(c.kind)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code>, not put back in line, so the Day shift looks at it first.</p>
    <p>What carries on:</p>${stopEffectsHTML(c)}
    <div class="confirm-actions">
      <button type="button" class="pull" data-do-stop="${c.id}">Stop, label needs-triage</button>
      <button type="button" data-do-requeue="${c.id}">Stop and requeue instead</button>
      <button type="button" data-cancel>Cancel</button>
    </div>
    <p class="cli">Same as <code>$ ${esc(stopHint(c))}</code><br>Requeue: <code>$ ${esc(requeueHint(c))}</code></p>
  </div>`;
}

function liveBed(c) {
  const sign = `<header class="bed-sign" data-fold="bed-${c.id}">
    <div><p class="bed-kind">${esc(c.kind)} · ${esc(c.repo)}</p>
      <h3>${c.issue ? `${issueLink(c.issue)} ${esc(c.title)}` : esc(c.note ?? c.repo)}</h3>
      <p class="bed-meta">planted ${age(c.started)} ago · ${esc(rhm(c))}</p>
      ${bedSummary(c)}</div>
    ${stopButton(c)}
  </header>${stopConfirm(c)}`;
  let body = "";
  for (const ch of c.children ?? []) {
    if (!ch.tickets) continue;
    body += `<div class="spec">
      <p class="spec-head"><span class="bed-kind">Spec run</span> ${issueLink(ch.issue)} <b>${esc(ch.title)}</b></p>
      <p class="bed-meta">Spec branch <code>${esc(ch.specBranch)}</code> · ${esc(ch.stage)} · ${age(ch.stageSince)}</p>
      <ul class="row-of-plants">${ch.tickets.map(ticketPlant).join("")}</ul>
    </div>`;
  }
  if (c.stage) {
    const h = health(c).key;
    body += `<div class="single${h ? ` h-${h}` : ""}">${plant("running", c.stage)}
      <div>${pace(c)}<p class="bed-meta">${c.pr ? `PR ${issueLink(c.pr)}` : ""}${c.merge ? " · Merge run" : ""}</p>
      ${growth(c.stage)}
      <p class="latest"><span aria-hidden="true">↳</span> <code>${esc(c.latest)}</code></p></div></div>`;
  }
  return `<article class="bed live" id="g-${c.id}">${sign}<div class="bed-body" data-fold-in="bed-${c.id}">${body}${spent(sessionsOf(c))}${logLinks(c.sessions, c.commandLog)}</div><div class="soil-strip"></div></article>`;
}
function deadBed(c) {
  return `<article class="bed withered" id="g-${c.id}">
    <header class="bed-sign" data-fold="bed-${c.id}" data-fold-default="open"><div><p class="bed-kind">${esc(c.kind)} · ${esc(c.repo)}</p>
      <h3>${issueLink(c.issue)} ${esc(c.title)}</h3>
      <p class="bed-meta">${esc(rhm(c))}</p></div>
      <div class="stake"><span>died at ${esc(c.stage)}</span><small>${age(c.diedAt)} ago</small></div></header>
    <div class="bed-body" data-fold-in="bed-${c.id}"><div class="single">${plant("failed")}
      <div><p><span class="word">Withered</span> The process is gone and left no outcome. Last seen:</p><p class="latest"><span aria-hidden="true">↳</span> <code>${esc(c.latest)}</code></p>
      <p class="hb-line">${heartbeat(c)}</p></div></div>
      ${spent(sessionsOf(c))}${logLinks(c.sessions, c.commandLog)}</div><div class="soil-strip"></div></article>`;
}

function outcomeLine(r) {
  const k = r.outcome ?? r.state;
  return `<span class="o o-${k}">${esc(OUTCOME[k] ?? k)}</span>${r.cause ? ` <span class="cause">${esc(r.cause)}</span>` : ""}`;
}
function entry(c) {
  const kids = (c.children ?? []).map((r) => `<li>${glyph(r.outcome)} <span class="bed-kind">${esc(r.kind)}</span> ${issueLink(r.issue)} ${esc(r.title)} · ${outcomeLine(r)}${r.pr ? ` · PR ${issueLink(r.pr)}` : ""}${summaryText(r) ? ` · ${esc(summaryText(r))}` : ""}${logLinks(r.sessions)}</li>`);
  const tickets = (c.tickets ?? []).map((t) => `<li class="mini">${plant(t.state, t.stage)}<span><span class="bed-kind">Ticket</span> ${issueLink(t.issue)} ${esc(t.title)} · <span class="o o-${t.state.replace(" ", "-")}">${esc(ticketLine(t))}</span>${t.pr ? ` · PR ${issueLink(t.pr)}` : ""}</span></li>`);
  const pests = c.findings ? `<div class="pests"><p class="bed-kind">Pests found</p><ul>${c.findings.map((f) =>
    `<li><span class="sev sev-${f.sev}">${esc(f.sev)}</span> ${esc(f.title)} <span class="cause">· ${f.reproduced ? `reproduced · fix ${issueLink(f.fix)} ${esc(f.fixOutcome)}` : "not reproduced · waits for the Day shift"}</span></li>`).join("")}</ul></div>` : "";
  const nested = [...kids, ...tickets];
  // The card: what came of it, readable folded. Unfolding shows what grew under it and its notes.
  const card = [c.pr ? `PR ${issueLink(c.pr)}` : "", summaryText(c) ? esc(summaryText(c)) : "", c.note ? esc(c.note) : ""].filter(Boolean).join(" · ");
  return `<li class="entry e-${c.outcome}" id="g-${c.id}">
    ${glyph(c.outcome)}
    <div class="entry-body">
      <p class="entry-head" data-fold="h-${c.id}"${c.outcome === "failed" ? ' data-fold-default="open"' : ""}><span><time>${esc(clock(c.ended))}</time> <span class="bed-kind">${esc(c.kind)} · ${esc(c.repo)}</span>${c.issue ? ` ${issueLink(c.issue)} ${esc(c.title)}` : ""} · <span class="o o-${c.outcome}">${esc(OUTCOME[c.outcome] ?? c.outcome)}</span></span></p>
      <div class="entry-card">${c.cause ? `<p class="cause">${esc(c.cause)}</p>` : ""}${card ? `<p>${card}</p>` : ""}</div>
      <div class="entry-more" data-fold-in="h-${c.id}">
        <p class="bed-meta">${esc(rhm(c))}${c.ended !== c.started ? ` · grew for ${esc(dur(c.ended - c.started))}` : ""}</p>
        ${spent(sessionsOf(c))}
        ${nested.length ? `<ul class="vine">${nested.join("")}</ul>` : ""}
        ${pests}
        ${logLinks(c.sessions, c.commandLog)}
      </div>
    </div></li>`;
}

// Needs tending: one line per problem, urgent first. Choosing one opens the bed or entry it names.
function tending() {
  const items = needsYou();
  const key = (n) => (n.cmd.outcome ? `h-${n.cmd.id}` : `bed-${n.cmd.id}`);
  return `<section class="tending" aria-label="Needs tending">
    <p class="tending-title">Needs tending <span class="count">${items.length}</span></p>
    ${items.length ? `<ul>${items.map((n) => `<li><button type="button" class="tend t-${n.level}" data-reveal="${key(n)}">
      <span class="t-level">${n.level === "urgent" ? "Tend now" : "Have a look"}</span>
      <span class="t-what"><span class="ref">${esc(n.ref)}</span> ${esc(n.what)}</span>
      <span class="t-why">${esc(n.why)}</span></button></li>`).join("")}</ul>` : `<p class="empty">Nothing needs tending. Everything is growing on its own.</p>`}
  </section>`;
}
// Opens whatever a tending item names: its section, then its fold, then brings it into view.
function revealFold(root, key) {
  const t = root.querySelector(`[data-fold="${key}"]`);
  if (!t) return;
  const shut = t.closest(".proto-shut");
  if (shut) {
    let n = shut;
    while (n && !(n.matches("h2") || n.querySelector(":scope > h2"))) n = n.previousElementSibling;
    (n?.matches("h2") ? n : n?.querySelector(":scope > h2"))?.click();
  }
  if (!t.classList.contains("is-unfolded")) t.click();
  t.scrollIntoView({ block: "center", behavior: "smooth" });
}

function notebook() {
  let head = "", body = "";
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    head = `<p class="nb-title">Command log · ${esc(c.kind)} ${esc(c.issue ?? c.repo)}</p><p class="nb-file">${esc(c.commandLog)} · plain text</p>`;
    body = `<pre class="nb-cmdlog">${esc(commandLogText(c))}</pre>`;
  } else if (sessions[state.openSession]) {
    const s = sessions[state.openSession];
    head = `<p class="nb-title">Session log · ${esc(s.owner)} · ${esc(s.kind)}${s.live ? ' <span class="growing">growing</span>' : ""}</p><p class="nb-file">${esc(s.harness)} · ${esc(s.file ?? "")}</p>`;
    body = `<div class="nb-log"></div>`;
  } else body = `<p class="empty">Nothing open. Pick a Session log or a Command log from a bed.</p>`;
  return `<aside id="log" class="notebook"><h2>Field notebook</h2>${head}<div class="nb-scroll">${body}</div></aside>`;
}

function season() {
  const d = new Date(NOW0), m = d.getMonth(), h = d.getHours();
  const s = ["Winter", "Winter", "Spring", "Spring", "Spring", "Summer", "Summer", "Summer", "Autumn", "Autumn", "Autumn", "Winter"][m];
  const t = h < 5 ? "night" : h < 12 ? "morning" : h < 17 ? "afternoon" : h < 21 ? "evening" : "night";
  return `${s} · ${t} · ${d.toLocaleDateString([], { day: "numeric", month: "short" })}`;
}

function gate(root) {
  root.innerHTML = `<div class="garden gate-page"><form class="gate">
    <svg class="gate-art" viewBox="0 0 200 110" aria-hidden="true"><path class="g-arch" d="M30 108 V50 Q100 -10 170 50 V108"/>
      ${[50, 70, 90, 110, 130, 150].map((x) => `<path class="g-bar" d="M${x} 108 V${x < 100 ? 52 - (x - 50) * 0.5 : 52 - (150 - x) * 0.5}"/>`).join("")}
      <path class="g-rail" d="M30 70 H170 M30 96 H170"/><circle class="p-petal" cx="36" cy="46" r="5"/><circle class="p-petal" cx="166" cy="48" r="4"/></svg>
    <h1>${esc(host.name)}</h1><p class="bed-meta">The garden gate</p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit" class="pull">Open the gate</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return gate(root);
  const days = new Map();
  for (const c of endedCommands().sort((a, b) => b.ended - a.ended)) {
    const k = clock(c.ended).includes(" ") ? clock(c.ended).split(" ")[0] : "today";
    if (!days.has(k)) days.set(k, []);
    days.get(k).push(c);
  }
  const live = liveCommands(), dead = attentionCommands();
  root.innerHTML = `<div class="garden">
    <header class="sign">
      <div class="drift" aria-hidden="true"><svg class="drift-leaf" viewBox="0 0 12 12"><path d="M1 11 C1 5 5 1 11 1 C11 7 7 11 1 11 Z"/></svg><i class="pollen p1"></i><i class="pollen p2"></i><i class="pollen p3"></i></div>
      <div class="sign-board"><p class="season">${esc(season())}</p><h1>${esc(host.name)}</h1><p class="sign-sub">Console · what grows on this machine</p></div>
      <nav><a href="${host.other.url}">${esc(host.other.name)}’s garden →</a><button type="button" class="link" data-signout>Close the gate</button></nav>
    </header>
    ${tending()}
    <div class="plot-layout">
      <div class="beds">
        <h2>In the ground <span class="count">${live.length}</span></h2>
        ${live.length ? live.map(liveBed).join("") : `<p class="empty">Nothing in the ground. The beds are resting until the next Pickup run.</p>`}
        ${dead.length ? `<h2>Withered <span class="count">${dead.length}</span></h2>${dead.map(deadBed).join("")}` : ""}
      </div>
      ${notebook()}
    </div>
    <section class="harvest"><h2>Harvest log</h2>
      ${days.size ? [...days].map(([d, cs]) => `<h3>${esc(d[0].toUpperCase() + d.slice(1))}</h3><ul class="journal">${cs.map(entry).join("")}</ul>`).join("") : `<p class="empty">No harvest yet.</p>`}
      <p class="empty">Back to the oldest logs on this machine.</p>
    </section>
  </div>`;
  const nb = root.querySelector(".nb-log");
  if (nb) mountLog(nb, state.openSession, root.querySelector(".nb-scroll"));
  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a");
    if (!t) return;
    if (t.dataset.reveal) revealFold(root, t.dataset.reveal);
    else if (t.dataset.stop) { confirming = t.dataset.stop; changed(); }
    else if (t.dataset.doStop) { confirming = null; stopCommand(t.dataset.doStop, false); }
    else if (t.dataset.doRequeue) { confirming = null; stopCommand(t.dataset.doRequeue, true); }
    else if ("cancel" in t.dataset) { confirming = null; changed(); }
    else if ("signout" in t.dataset) signOut();
    else if (t.dataset.session) { e.preventDefault(); openSession(t.dataset.session); document.getElementById("log")?.scrollIntoView({ block: "nearest" }); }
    else if (t.dataset.cmdlog) { e.preventDefault(); openCommandLog(t.dataset.cmdlog); document.getElementById("log")?.scrollIntoView({ block: "nearest" }); }
  });
}
