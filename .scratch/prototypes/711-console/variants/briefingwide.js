// PROTOTYPE variant "briefingwide": the Day shift briefing laid out across a wide screen. Same sentences and folds as
// "briefing"; only the page uses the width: lede beside its tally, cards two or three to a row, the log in a column of its own.
// anything need me?", so the page answers in written sentences built from the data, then gives the evidence.
// The lede sits above every heading, so it reads even with every section closed.
import { host, sessions, NOW0 } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, liveCommands, attentionCommands, endedCommands, runsOf,
  stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog, mountLog, commandLogText,
  command, changed, canStop, stopEffectsHTML, usualText, health, lastEventAt, needsYou, usageText, sessionsOf,
  summaryText,
} from "../shared.js";

// Where the Day shift last looked: 09:00 yesterday.
const lastLooked = (() => { const d = new Date(NOW0); d.setDate(d.getDate() - 1); d.setHours(9, 0, 0, 0); return d.getTime(); })();

let confirming = null; // { id, where }
let firstRender = true; // the "Needs you" count pulses once, on the first render only

const WORDS = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
const num = (n) => WORDS[n] ?? String(n);
const cap = (s) => s.charAt(0).toUpperCase() + s.slice(1);
const plural = (n, one, many) => (n === 1 ? one : many);
const passPhrase = (c) => `${/^[AEIOU]/.test(c.kind) ? "an" : "a"} ${c.kind} on ${esc(c.repo)}`;
const title = (r) => r.title ? ` <span class="ttl">${esc(r.title)}</span>` : "";

// ---------- Drill-down folds (app.js) ----------
// An item folds to its one sentence; the sentence is the toggle. Unfolding shows the evidence, which may hold
// further folds ("What you can do", each Ticket). `keys` is the item's own key plus its ancestors'.
const head = (key, html, open = false) => `<p class="what" data-fold="${key}"${open ? ' data-fold-default="open"' : ""}>${html}</p>`;
const more = (keys, html) => `<div class="more" data-fold-in="${keys}">${html}</div>`;
// A Stop confirm stays outside the fold, so it shows whatever the reader folds.
const stopOutside = (c, where) => (confirming?.id === c.id && confirming.where === where ? stopBlock(c, where) : "");
const stopInside = (c, where) => (confirming?.id === c.id && confirming.where === where ? "" : stopBlock(c, where));

// ---------- What the data says ----------
// Live Runs by health: silent (nothing logged lately, may be stuck) or slow (past its usual range, still moving).
const byHealth = (key) => needsYou().filter((it) => it.cmd.alive && health(it.run).key === key);
const moved = (r) => { const t = lastEventAt(r); return t ? `its log moved ${age(t)} ago` : "its log is quiet"; };
const usualParen = (stage) => { const u = usualText(stage); return u ? u.replace(/^usually /, "the usual ") : ""; };
const failed = () => endedCommands().filter((c) => c.outcome === "failed");
const triage = () => endedCommands().filter((c) => c.outcome === "interrupted" && /needs-triage/.test(c.cause ?? ""));
const waitingFindings = () => endedCommands().flatMap((c) => (c.findings ?? []).filter((f) => f.waits).map((f) => ({ c, f })));
const readyPRs = () => endedCommands().flatMap((c) => runsOf(c)).filter((r) => r.outcome === "ready" && r.pr);

// ---------- Small pieces ----------
function logLinks(r, c) {
  const links = [];
  if (c?.commandLog && r === c) links.push(`<a href="#log" data-cmdlog="${c.id}"${state.openCommandLog === c.id ? ' aria-current="true"' : ""}>Command log</a>`);
  for (const id of r.sessions ?? []) {
    const s = sessions[id];
    links.push(`<a href="#log" data-session="${id}"${state.openSession === id ? ' aria-current="true"' : ""}>${esc(s.kind)} Session log${s.live ? ' <span class="live-dot" aria-label="live"></span>' : ""}</a>`);
  }
  return links.length ? `<p class="links">${links.join("")}</p>` : "";
}
function facts(r, tokenIds = r.sessions ?? []) {
  const working = r.state === "running" || (!r.outcome && r.stage && r.alive !== false);
  const stage = working ? ` · <b>${esc(r.stage)}</b> · ${age(r.stageSince)}${usualText(r.stage) ? ` <span class="usual">(${esc(usualText(r.stage))})</span>` : ""}` : "";
  const beat = working && lastEventAt(r) ? `<span class="beat">last output ${age(lastEventAt(r))} ago</span>` : "";
  const latest = r.latest && working
    ? `<span class="latest">latest: <code${r.id ? ` data-latest="${r.id}"` : ""}>${esc(r.latest)}</code></span>` : "";
  const tokens = usageText(tokenIds);
  return `<p class="facts">${issueLink(r.issue)}${title(r)}${stage}<br><span class="hme">${esc(rhm(r))}</span>${beat}${latest}${tokens ? `<span class="tokens">${esc(tokens)}</span>` : ""}</p>`;
}
function stopBlock(c, where) {
  if (c.stopping) return `<p class="stop-row stopping">Stopping… pushed work is kept while salvage and the Claim finish.</p>`;
  if (!canStop(c)) return "";
  if (confirming?.id === c.id && confirming.where === where) return `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
    <p><b>Stop the ${esc(c.kind)} for ${esc(c.stopTarget)}?</b> It stops everything this Command started. ${esc(c.stopTarget)} will be labelled <code>needs-triage</code>, not put back in line, so it waits for you. What still finishes:</p>
    ${stopEffectsHTML(c)}
    <p class="confirm-actions"><button type="button" class="danger" data-do-stop="${c.id}">Stop, label needs-triage</button>
      <button type="button" data-do-requeue="${c.id}">Stop and requeue instead</button>
      <button type="button" data-cancel>Cancel</button></p>
    <p class="cli">Same as <code>${esc(stopHint(c))}</code>, or <code>${esc(requeueHint(c))}</code> to requeue.</p></div>`;
  return `<p class="stop-row"><button type="button" class="danger-quiet" data-stop="${c.id}" data-where="${where}">Stop…</button> <span class="cli">or <code>${esc(stopHint(c))}</code></span></p>`;
}
const canDo = (keys, items) => {
  if (!items.length) return "";
  const own = `${keys.split(" ")[0]}-do`;
  return `<div class="do"><p class="do-label" data-fold="${own}">What you can do</p><ul data-fold-in="${keys} ${own}">${items.map((i) => `<li>${i}</li>`).join("")}</ul></div>`;
};

function ticketTicks(spec, keys) {
  const mark = { merged: "✓", running: "●", waiting: "◌", failed: "✕", "not started": "○", interrupted: "■" };
  return `<ul class="ticks">${spec.tickets.map((t) => {
    let line;
    switch (t.state) {
      case "merged": line = `merged into the Spec branch${t.pr ? ` as PR ${issueLink(t.pr)}` : ""}`; break;
      case "running": line = `${esc(t.stage)} · ${age(t.stageSince)}`; break;
      case "waiting": line = `waiting on ${t.blockedBy.map((b) => issueLink(b)).join(", ")}`; break;
      case "failed": line = `failed: ${esc(t.cause)}`; break;
      case "interrupted": line = "interrupted"; break;
      default: line = "not started";
    }
    const own = `tk-${t.id}`;
    const latest = t.state === "running" && t.latest ? `<br><span class="latest">latest: <code data-latest="${t.id}">${esc(t.latest)}</code></span>` : "";
    const detail = `${t.title ? `<span class="ttl">${esc(t.title)}</span>` : ""}${t.harness ? `<br><span class="hme">${esc(rhm(t))}</span>` : ""}${latest}${t.sessions ? logLinks(t) : ""}`;
    return `<li class="tk-${t.state.replace(" ", "-")}"><span class="tk-head" data-fold="${own}"><span class="mk" aria-hidden="true">${mark[t.state] ?? "·"}</span> ${issueLink(t.issue)} <span class="tk-line">— ${line}</span></span>
      <div class="tk-more" data-fold-in="${keys} ${own}">${detail}</div></li>`;
  }).join("")}</ul>`;
}

// ---------- The lede ----------
function lede() {
  const live = liveCommands(), died = attentionCommands(), silent = byHealth("silent"), slow = byHealth("slow");
  const s = [];
  s.push(live.length ? `${cap(num(live.length))} ${plural(live.length, "Command is", "Commands are")} running on ${esc(host.name)}.`
    : `Nothing is running on ${esc(host.name)} right now.`);
  const now = silent.length + died.length;
  if (now) {
    const bits = [...silent.map((it) => `${issueLink(it.run.issue)} has printed nothing for ${age(lastEventAt(it.run))} at ${esc(it.run.stage)}`),
      ...died.map((c) => `${issueLink(c.issue)} died at ${esc(c.stage)} ${age(c.diedAt)} ago and nothing is working on it`)];
    s.push(`${cap(num(now))} ${plural(now, "needs", "need")} you now: ${bits.length > 1 ? `${bits.slice(0, -1).join(", ")}, and ${bits.at(-1)}` : bits[0]}.`);
  }
  for (const it of slow) s.push(`${issueLink(it.run.issue)} is slow in ${esc(it.run.stage)}, ${age(it.run.stageSince)} against ${esc(usualParen(it.run.stage))}, but ${moved(it.run)}.`);
  if (!now && !slow.length) s.push("Nothing running needs you.");
  const rest = [[failed().length, "failed work"], [waitingFindings().length, "a Security finding"], [triage().length, "a stopped issue to triage"],
    [readyPRs().length, "pull requests to review"]].filter(([k]) => k).map(([, w]) => w);
  if (rest.length) s.push(`Below that wait ${rest.length > 1 ? `${rest.slice(0, -1).join(", ")} and ${rest.at(-1)}` : rest[0]}.`);

  const since = endedCommands().filter((c) => c.ended > lastLooked);
  const n = (o) => since.filter((c) => c.outcome === o).length;
  const parts = [["merged", n("merged")], ["ready for review", n("ready")], ["failed", n("failed")], ["interrupted", n("interrupted")], ["skipped", n("skipped")]]
    .filter(([, k]) => k).map(([w, k]) => `<b>${k}</b> ${w}`);
  return `<p class="lede">${s.join(" ")}</p>
    <p class="tally">Since you last looked, ${esc(clock(lastLooked))}: ${parts.length ? parts.join(" · ") : "nothing ended"}.</p>`;
}

// ---------- Needs you ----------
// One card per problem, in the order and with the rule every variant shares (shared.js needsYou): died and silent
// first, then failures, then what is slow but moving, stopped issues and findings. A failed Spec run is one card.
function needsSection() {
  const cards = [];
  needsYou().forEach((it, i) => {
    const c = it.cmd, r = it.run;
    const k = `need-${it.id.replace(/[^\w-]/g, "_")}`;
    const urgent = it.level === "urgent" ? " urgent" : "";
    if (!c.outcome && !c.alive) cards.push(`<article class="card${urgent}">
      ${head(k, `${issueLink(c.issue)} died at ${esc(c.stage)} ${age(c.diedAt)} ago. Its process is gone and left no outcome, so nothing will finish it, and its Claim keeps it out of every Pickup run.`, true)}
      ${more(k, `${facts({ ...c, stage: null }, sessionsOf(c))}<p class="facts">Last seen: <code>${esc(c.latest)}</code>, ${age(c.diedAt)} ago; nothing has printed since.</p>
      ${canDo(k, ["Read its Command log to see where it stopped.", `Start it again with <code>thirdshift &lt;${esc(c.issue)}'s Issue URL&gt;</code>; a Continuation picks up its Issue branch if one was pushed.`])}
      ${logLinks(c, c)}`)}</article>`);
    else if (c.alive && health(r).key === "silent") cards.push(`<article class="card${urgent}">
      ${head(k, `${issueLink(r.issue)} has printed nothing for ${age(lastEventAt(r))} at ${esc(r.stage)}. ${esc(it.why)}`, true)}
      ${more(k, `${facts(r)}
      ${canDo(k, ["Read its Command log for the last thing it printed.", `If it is stuck, Stop it. Pushed work is kept, and ${esc(c.stopTarget)} waits for you as <code>needs-triage</code>.`])}
      ${logLinks(c, c)}${stopInside(c, `need-${it.id}`)}`)}${stopOutside(c, `need-${it.id}`)}</article>`);
    else if (c.alive && health(r).key === "slow") cards.push(`<article class="card">
      ${head(k, `${issueLink(r.issue)} has been in ${esc(r.stage)} for <span class="stall-age">${age(r.stageSince)}</span>, slower than ${esc(usualParen(r.stage))}, but ${moved(r)}. Slow, not stuck.`)}
      ${more(k, `${facts(r)}
      ${canDo(k, [`Watch its Session log below before deciding.`, `If it is stuck after all, Stop the ${esc(c.kind)}. That also stops the rest of its Spec run.`])}
      ${logLinks(r)}${stopInside(c, `need-${it.id}`)}`)}${stopOutside(c, `need-${it.id}`)}</article>`);
    else if (c.outcome === "failed") cards.push(`<article class="card${urgent}">
      ${head(k, `${c.kind === "Spec run" ? "The Spec run on " : ""}${issueLink(c.issue)} failed ${esc(clock(c.ended))}: ${esc(c.cause)}. Its Claim stays, so nothing retries it until you act.`, true)}
      ${more(k, `${facts(c, sessionsOf(c))}${summaryText(c) ? `<p class="facts summary">${esc(summaryText(c))}.</p>` : ""}${c.tickets ? ticketTicks(c, k) : ""}${(c.children ?? []).map((ch) => `<p class="facts">${esc(ch.kind)} ${issueLink(ch.issue)}${title(ch)}: ${esc(ch.outcome)}${ch.cause ? `, ${esc(ch.cause)}` : ""}.</p>`).join("")}
      ${canDo(k, ["Read its Command log for the step that failed.", `Fix the cause, then start it again with <code>thirdshift &lt;${esc(c.issue)}'s Issue URL&gt;</code>.`])}
      ${logLinks(c, c)}`)}</article>`);
    else if (c.findings && it.id.includes(":")) cards.push(`<article class="card">
      ${head(k, `A Security run on ${esc(c.repo)} found <i>${esc(it.why)}</i> (${esc(it.what.split(" ")[0])}) and could not reproduce it. A finding nothing reproduces is never fixed unattended, so it waits for you to triage.`)}
      ${more(k, `${canDo(k, ["Read the reproduction Session logs, then file an issue or dismiss it."])}
      ${logLinks(c, c)}`)}</article>`);
    else if (c.outcome === "interrupted") cards.push(`<article class="card">
      ${head(k, `${issueLink(c.issue)} was stopped during ${esc(c.stage)} and labelled <code>needs-triage</code>, so it waits for you rather than going back in line.`)}
      ${more(k, `${facts(c, sessionsOf(c))}${summaryText(c) ? `<p class="facts summary">${esc(summaryText(c))}.</p>` : ""}
      ${canDo(k, ["Relabel it <code>ready-for-agent</code> on GitHub to put it back in line, or close it."])}
      ${logLinks(c, c)}`)}</article>`);
  });
  const prs = readyPRs();
  if (prs.length) cards.push(`<article class="card quiet">
    <p class="what">${cap(num(prs.length))} pull ${plural(prs.length, "request is", "requests are")} ready for your review: ${prs.map((r) => `PR ${issueLink(r.pr)} for ${issueLink(r.issue)}${title(r)}`).join("; ")}.</p></article>`);
  return cards.length ? cards.join("") : `<p class="empty">Nothing needs you. The factory is either working or resting.</p>`;
}

// ---------- Running fine ----------
function running() {
  const live = liveCommands();
  if (!live.length) return `<p class="empty">Nothing is running. The next Pickup run starts when the scheduler says so.</p>`;
  return live.map((c) => {
    const spec = c.children?.find((ch) => ch.tickets);
    const h = runsOf(c).map(health).map((x) => x.key);
    const note = h.includes("silent") ? " It has gone quiet; see Needs you." : h.includes("slow") ? " One Ticket is slower than usual but still moving; see Needs you." : "";
    let what;
    if (c.kind === "Pickup run" && spec) {
      const merged = spec.tickets.filter((t) => t.state === "merged").length;
      what = `A Pickup run on ${esc(c.repo)} took ${issueLink(spec.issue)}${title(spec)} ${age(c.started)} ago and is working through it as a Spec run: ${num(merged)} of ${num(spec.tickets.length)} Tickets merged into <code>${esc(spec.specBranch)}</code>.${note}`;
    } else if (c.kind === "Run") {
      what = `${issueLink(c.issue)}${title(c)} has been at ${esc(c.stage)} for ${age(c.stageSince)}${c.merge ? ". It's a Merge run" : ""}${c.pr ? `${c.merge ? ", and" : ". Its"} pull request is ${issueLink(c.pr)}` : ""}.${note}`;
    } else what = `${cap(passPhrase(c))} has been running for ${age(c.started)}.`;
    const k = `run-${c.id}`;
    return `<article class="item">
      ${head(k, what)}
      ${more(k, `${c.kind === "Run" ? facts(c, sessionsOf(c)) : `<p class="facts"><span class="hme">${esc(rhm(c))}</span>${usageText(sessionsOf(c)) ? `<span class="tokens">${esc(usageText(sessionsOf(c)))} so far</span>` : ""}</p>`}
      ${spec ? ticketTicks(spec, k) : ""}
      ${logLinks(c, c)}${stopInside(c, "running")}`)}${stopOutside(c, "running")}</article>`;
  }).join("");
}

// ---------- Done ----------
function childSentence(ch) {
  const done = ch.outcome === "merged" ? `merged as PR ${issueLink(ch.pr)}` : ch.outcome === "ready" ? `is ready for review as PR ${issueLink(ch.pr)}`
    : `${esc(ch.outcome)}${ch.stage ? ` at ${esc(ch.stage)}` : ""}${ch.cause ? `: ${esc(ch.cause)}` : ""}`;
  return ch.kind === "Base fix" ? `Its Base fix ${issueLink(ch.issue)}${title(ch)} ${done}.` : `It started ${issueLink(ch.issue)}${title(ch)}, which ${done}.`;
}
function doneSentence(c) {
  const took = dur(c.ended - c.started);
  const who = c.issue ? `${c.kind === "Spec run" ? "The Spec run on " : ""}${issueLink(c.issue)}${title(c)}` : cap(passPhrase(c));
  switch (c.outcome) {
    case "merged": return c.issue ? `${who} merged as PR ${issueLink(c.pr)} after ${took}.` : `${who} finished after ${took}${c.note ? `; ${esc(c.note)}` : ""}.`;
    case "ready": return c.issue ? `${who} is ready for review as PR ${issueLink(c.pr)}, after ${took}.`
      : `${who} finished after ${took} with ${num(c.findings?.length ?? 0)} ${plural(c.findings?.length ?? 0, "finding", "findings")}.`;
    case "failed": return `${who} failed after ${took}: ${esc(c.cause)}.`;
    case "interrupted": return `${who} was interrupted${c.stage ? ` at ${esc(c.stage)}` : ""} after ${took}: ${esc(c.cause)}.`;
    case "skipped": return `${who} was skipped: ${esc(c.cause)}. Skipped isn't a failure, and it sent no Run notification.`;
    default: return `${who}: ${esc(c.outcome)}.`;
  }
}
function done() {
  const list = endedCommands().slice().sort((a, b) => b.ended - a.ended);
  if (!list.length) return `<p class="empty">Nothing has ended on this machine yet.</p>`;
  let html = "", day = null, markerShown = false;
  for (const c of list) {
    if (!markerShown && c.ended <= lastLooked) { html += `<p class="marker"><span>you last looked here, ${esc(clock(lastLooked))}</span></p>`; markerShown = true; day = null; }
    const d = new Date(c.ended).toDateString() === new Date(NOW0).toDateString() ? "Today" : clock(c.ended).split(" ")[0];
    if (d !== day) { html += `<h3>${esc(cap(d))}</h3>`; day = d; }
    const findings = c.findings ? `<ul class="sub">${c.findings.map((f) => `<li><span class="sev sev-${f.sev}">${esc(f.sev)}</span> ${esc(f.title)}: ${f.reproduced ? `reproduced, fixed by ${issueLink(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced; waits for you"}.</li>`).join("")}</ul>` : "";
    const kids = (c.children ?? []).filter((ch) => !ch.tickets).map((ch) => `<li>${childSentence(ch)}${logLinks(ch)}</li>`).join("");
    const k = `done-${c.id}`;
    html += `<article class="done o-${c.outcome}">
      ${head(k, `<time class="when">${esc(clock(c.ended).split(" ").at(-1))}</time> ${doneSentence(c)}`, c.outcome === "failed")}
      ${more(k, `${summaryText(c) ? `<p class="facts summary">${esc(summaryText(c))}.</p>` : ""}${kids ? `<ul class="sub">${kids}</ul>` : ""}${c.tickets ? ticketTicks(c, k) : ""}${findings}
      <p class="facts"><span class="hme">${esc(c.kind)} · ${esc(rhm(c))}</span>${usageText(sessionsOf(c)) ? `<span class="tokens">${esc(usageText(sessionsOf(c)))}</span>` : ""}</p>${logLinks(c, c)}`)}</article>`;
  }
  return html + `<p class="end">That is everything back to the oldest logs on this machine.</p>`;
}

// ---------- Session log ----------
function logSection() {
  if (state.openCommandLog) {
    const c = command(state.openCommandLog);
    return `<p class="log-what">The Command log of ${c.issue ? issueLink(c.issue) : passPhrase(c)}, as plain text. <span class="file">${esc(c.commandLog)}</span></p>
      <pre class="cmdlog">${esc(commandLogText(c))}</pre>`;
  }
  const s = sessions[state.openSession];
  if (!s) return `<p class="empty">Choose a Session log from any item above.</p>`;
  return `<p class="log-what">${issueLink(s.owner.includes("#") ? s.owner : "")}${s.owner.includes("#") ? "" : esc(s.owner)} · ${esc(s.kind)} session on ${esc(s.harness)}${s.live ? " · <b class=\"live\">live, new lines appear at the bottom</b>" : ""} <span class="file">${esc(s.file ?? "")}</span></p>
    <div class="log-box"></div>`;
}

function signInPage(root) {
  root.innerHTML = `<div class="briefing signin"><form>
    <p class="kicker">Day shift briefing</p>
    <h1>${esc(host.name)}</h1>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Read the briefing</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  const nNeeds = needsYou().length + (readyPRs().length ? 1 : 0);
  const today = new Date(NOW0).toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
  root.innerHTML = `<div class="briefing">
    <header class="mast">
      <p class="kicker">Day shift briefing · ${esc(today)}</p>
      <h1>${esc(host.name)}</h1>
      <nav><a href="${host.other.url}">${esc(host.other.name)}'s briefing →</a><button type="button" class="link" data-signout>Sign out</button></nav>
    </header>
    <div class="top">${lede()}</div>
    <div class="cols">
      <div class="main">
        <section class="sec needs"><h2>Needs you <span class="count${firstRender ? " pulse" : ""}">${nNeeds}</span></h2>${needsSection()}</section>
        <section class="sec running"><h2>Running fine <span class="count">${liveCommands().length}</span></h2>${running()}</section>
        <section class="sec done-sec"><h2>Done since you last looked</h2>${done()}</section>
      </div>
      <aside class="side">
        <section class="sec" id="log"><h2>Session log</h2>${logSection()}</section>
      </aside>
    </div>
  </div>`;
  // The latest-event line flashes briefly when the live session moves it on.
  const watch = new MutationObserver((recs) => {
    for (const r of recs) {
      const el = r.target.nodeType === 1 ? r.target : r.target.parentElement;
      if (!el?.matches?.("[data-latest]")) continue;
      el.classList.remove("flash"); void el.offsetWidth; el.classList.add("flash");
    }
  });
  for (const el of root.querySelectorAll("[data-latest]")) watch.observe(el, { childList: true, characterData: true, subtree: true });
  firstRender = false;
  const box = root.querySelector(".log-box");
  if (box) mountLog(box, state.openSession);
  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a");
    if (!t) return;
    const d = t.dataset;
    if (d.stop) { confirming = { id: d.stop, where: d.where }; changed(); }
    else if (d.doStop) { confirming = null; stopCommand(d.doStop, false); }
    else if (d.doRequeue) { confirming = null; stopCommand(d.doRequeue, true); }
    else if ("cancel" in d) { confirming = null; changed(); }
    else if ("signout" in d) signOut();
    else if (d.session) { e.preventDefault(); openSession(d.session); revealLog(); }
    else if (d.cmdlog) { e.preventDefault(); openCommandLog(d.cmdlog); revealLog(); }
  });
}

// The log section may be closed; open it and bring it into view.
function revealLog() {
  const sec = document.getElementById("log");
  const h2 = sec?.querySelector("h2");
  if (h2 && h2.getAttribute("aria-expanded") === "false") h2.click();
  sec?.scrollIntoView({ block: "start" });
}
