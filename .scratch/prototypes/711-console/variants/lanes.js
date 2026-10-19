// PROTOTYPE variant "lanes" (the surprise): a time axis over the last 3.5 hours, one lane per Run, nested under
// its Command. The question: does a time axis make a stall visible at a glance? After the research note
// (docs/research/console-run-monitoring-ui.md): a stall is silence, not stage length, so a lane turns red when its
// logs have said nothing for SILENT_MIN and amber when a stage runs past its usual range; a faint band shows that
// usual range. A "Needs you" strip leads; drill-down stops at two levels; History below is an Activity log style
// list; the Session log opens in a drawer above the prototype bar.
import { host, commands, sessions, pastRuns, RUN_STAGES, SPEC_STAGES } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, endedCommands, runsOf, ticketLine,
  stopCommand, stopHint, requeueHint, signIn, signOut, openSession, openCommandLog, mountLog, commandLogText, changed,
  canStop, stopEffectsHTML, SILENT_MIN, lastEventAt, usual, usualText, health, heartbeat, needsYou, usageText,
  sessionsOf, summaryText,
} from "../shared.js";

const SPAN = 210; // minutes on the axis, ending at now
const GUTTER = 62; // minutes of room right of "now", for each lane's stage · age flag

let confirming = null;
let drawerOpen = false;
let drawerRun = "t239";
let view = null; // "list" or "timeline"; unset means list on a phone, as Buildkite drops its waterfall there

const hms = (ts) => new Date(ts).toLocaleTimeString([], { hour12: false });
setInterval(() => { for (const el of document.querySelectorAll(".lanes-clock")) el.textContent = hms(Date.now()); }, 1000);

const kindOf = (r) => r.kind ?? "Ticket";
const slug = (s) => String(s).toLowerCase().replace(/[^a-z]+/g, "-");
const cmdOf = (run) => commands.find((c) => runsOf(c).includes(run));
function runById(id) { for (const c of commands) for (const r of runsOf(c)) if (r.id === id) return r; }
function runOfSession(sid) { for (const c of commands) for (const r of runsOf(c)) if (r.sessions?.includes(sid)) return r; }

// A lane's state: a Ticket's own, a Run's outcome, or else its Command's.
function runState(r, c) {
  if (r.state) return r.state;
  if (r.outcome) return r.outcome;
  if (c.outcome) return c.outcome;
  return c.alive ? "running" : "died";
}
const endOf = (r, c, now) => r.ended ?? c.ended ?? (c.alive ? now : c.diedAt);

// SYNTHESIZED: the data keeps only when a Run started, entered its current stage and ended, not each stage's
// start. Earlier stages share the time before the current one by a rough weight, so the bars look plausible.
const WEIGHT = { implement: 3, review: 1.2, Delivery: 0.4, "CI watch": 1.6, "Self-merge": 0.2, cleanup: 0.15, Tickets: 4, "spec-review": 1 };
function spread(stages, a, b) {
  const total = stages.reduce((n, s) => n + (WEIGHT[s] ?? 1), 0);
  let t = a;
  return stages.map((s) => { const w = ((WEIGHT[s] ?? 1) / total) * (b - a); const seg = { stage: s, a: t, b: t + w }; t += w; return seg; });
}
function segments(r, c, now) {
  const st = runState(r, c), kind = kindOf(r), start = r.started, end = endOf(r, c, now);
  if (kind === "Pickup run") {
    const child = c.children?.[0], t = child?.started ?? end;
    return [{ stage: "pickup", a: start, b: t }, ...(child ? [{ stage: `dispatched ${child.issue}`, a: t, b: end, thin: true }] : [])];
  }
  if (kind === "Architect run") return [{ stage: "architecture-review", a: start, b: end }];
  if (kind === "Security run") return spread(["security-audit", "security-reproduction", "security-fix-publishing"], start, end);
  let list = kind === "Spec run" ? SPEC_STAGES : RUN_STAGES;
  if (st === "ready") list = list.filter((s) => s !== "Self-merge");
  if (st === "merged" || st === "ready") return spread(list, start, end);
  const i = Math.max(0, list.indexOf(r.stage ?? c.stage ?? list[0]));
  if (r.stageSince && r.stageSince < end) return [...spread(list.slice(0, i), start, r.stageSince), { stage: list[i], a: r.stageSince, b: end, cur: true }];
  return spread(list.slice(0, i + 1), start, end).map((s, k, all) => (k === all.length - 1 ? { ...s, cur: true } : s));
}
// Red for silence (maybe stuck), amber for slower than usual (still working).
const heat = (r) => ({ silent: "hot", slow: "warm" })[health(r).key] ?? "";
// Repairs and Resumes a Run went through, from its summary or the earlier Runs list.
function attempts(r) {
  const p = pastRuns.find((x) => x.issue === r.issue);
  return { repairs: r.summary?.repairs ?? p?.repairs ?? 0, resumes: r.summary?.resumes ?? p?.resumes ?? 0 };
}
// A live Spec run's Tickets in the order they gate its Spec PR: merged, then running, then those waiting.
function criticalPath(spec) {
  const rank = { merged: 0, failed: 1, running: 2, waiting: 3, "not started": 4 };
  return [...spec.tickets].sort((a, b) => (rank[a.state] - rank[b.state]) || ((a.ended ?? a.started ?? 0) - (b.ended ?? b.started ?? 0)));
}
const shortRef = (ref) => `#${ref.split("#")[1]}`;
// A Ticket in words, for the chips on its Spec run's lane: words beside every colour (GOV.UK tags).
function ticketWords(t) {
  if (t.state === "running") { const h = health(t); return `${t.stage} · ${dur(Date.now() - t.stageSince)}${h.key === "slow" ? " · slower than usual" : h.key === "silent" ? ` · ${h.label}` : ""}`; }
  if (t.state === "waiting") return `cannot start yet: waiting on ${t.blockedBy.map(shortRef).join(", ")}`;
  if (t.state === "failed") return "failed";
  return t.state;
}

// What a lane says about where it is now: drawn as a flag on the axis, and as the label's second line on a phone.
function flagFor(r, c, now, x) {
  const st = runState(r, c), kind = kindOf(r);
  if (r === c && c.stopping) return { x: SPAN, html: "stopping… salvage finishing", cls: "stopping" };
  if (st === "running") {
    if (kind === "Pickup run") return { x: SPAN, html: `running ${esc(c.children?.[0]?.issue ?? "")}` };
    if (kind === "Spec run") { const m = r.tickets.filter((t) => t.state === "merged").length; return { x: SPAN, html: `Tickets · ${m} of ${r.tickets.length} merged` }; }
    const h = health(r);
    const note = h.key === "slow" ? " · slower than usual" : h.key === "silent" ? ` · ${esc(h.label)}` : "";
    return { x: SPAN, html: `${esc(r.stage)} · ${age(r.stageSince)}${note}`, cls: heat(r) };
  }
  if (st === "waiting") return { x: SPAN, html: `waiting on ${r.blockedBy.map((b) => esc(b)).join(", ")}`, cls: "wait" };
  if (st === "not started") return { x: SPAN, html: "not started", cls: "dim" };
  if (st === "died") return { x: SPAN, html: `died at ${esc(c.stage)} · ${age(c.diedAt)} ago`, cls: "died" };
  if (st === "skipped") return { x: x(c.ended), html: "skipped", cls: "dim" };
  const end = endOf(r, c, now);
  return { x: x(end), html: `${esc(st)}${r.pr ? ` · PR ${esc(r.pr)}` : ""}`, cls: `o-${st}` };
}

const sessionLinks = (r) => (r.sessions ?? []).map((id) => {
  const s = sessions[id];
  return `<a href="#" data-session="${id}" data-run="${r.id}"${state.openSession === id ? ' aria-current="true"' : ""}>${esc(s.kind)}${s.live ? " ●" : ""}${s.secure ? " 🔒" : ""}</a>`;
}).join("");

// Lanes drill down two levels at most (NN/g): a Command's one fold shows its details and every lane under it, each
// already in full; a Spec run's lane also carries its Tickets as chips, so their state reads without opening more.
function lane(r, c, depth, now, x, crit = null, spec = null) {
  const st = runState(r, c), kind = kindOf(r);
  const marks = [];
  if (st === "waiting") {
    const from = Math.max(0, x(cmdOf(r)?.children?.find((s) => s.tickets?.includes(r))?.started ?? c.started));
    marks.push(`<span class="hatch" style="--x:${from.toFixed(2)};--w:${(x(endOf(r, c, now)) - from).toFixed(2)}" title="waiting, not working"></span>`);
  } else if (st === "skipped") {
    marks.push(`<span class="tickmark" style="--x:${x(c.ended).toFixed(2)}"></span>`);
  } else if (st !== "not started") {
    const segs = segments(r, c, now);
    for (const s of segs) {
      const a = Math.max(0, x(s.a)), b = x(s.b);
      if (b <= 0) continue;
      const w = Math.max(0.6, b - a);
      // The usual range for this stage, as a faint band behind the current one: where it "should" have ended.
      const u = s.cur && st === "running" && r.stageSince ? usual(s.stage) : null;
      if (u) {
        const ua = x(r.stageSince + u.lo * 60_000), ub = Math.min(SPAN + GUTTER, x(r.stageSince + u.hi * 60_000));
        if (ub > 0) marks.push(`<span class="ghost" style="--x:${Math.max(0, ua).toFixed(2)};--w:${(ub - Math.max(0, ua)).toFixed(2)}" title="${esc(usualText(s.stage))}, from earlier Runs here"></span>`);
      }
      const h = s.cur && st === "running" ? heat(r) : "";
      marks.push(`<span class="seg s-${slug(s.stage)}${s.cur ? " cur" : ""}${s.thin ? " thin" : ""}${h ? ` ${h}` : ""}${x(s.a) < 0 ? " clipped" : ""}" style="--x:${a.toFixed(2)};--w:${w.toFixed(2)}" title="${esc(s.stage)} · ${dur(s.b - s.a)}">${w >= 16 && !s.thin ? esc(s.stage) : ""}</span>`);
    }
    // Silence: from the last thing this Run logged to now, dotted, over the bar.
    if (health(r).key === "silent") {
      const a = Math.max(0, x(lastEventAt(r)));
      marks.push(`<span class="silence" style="--x:${a.toFixed(2)};--w:${(SPAN - a).toFixed(2)}" title="nothing logged since ${clock(lastEventAt(r))}"><i>silent</i></span>`);
    }
    // Repairs on the CI watch stretch, a Resume at the end of implement.
    const { repairs, resumes } = attempts(r);
    const ci = segs.find((s) => s.stage === "CI watch") ?? segs.at(-1), impl = segs.find((s) => s.stage === "implement");
    for (let k = 1; k <= repairs && ci; k++) {
      const at = x(ci.a + ((ci.b - ci.a) * k) / (repairs + 1));
      if (at > 0) marks.push(`<span class="attempt repair" style="--x:${at.toFixed(2)}" title="Repair ${k}">R${k}</span>`);
    }
    if (resumes && impl && x(impl.b) > 0) marks.push(`<span class="attempt resume" style="--x:${x(impl.b).toFixed(2)}" title="Resume">↻</span>`);
    if (st === "died") {
      const d = x(c.diedAt);
      marks.push(`<span class="gap" style="--x:${d.toFixed(2)};--w:${(SPAN - d).toFixed(2)}"></span><span class="cross" style="--x:${d.toFixed(2)}" aria-label="process gone">✕</span>`);
    }
  }
  const f = flagFor(r, c, now, x);
  marks.push(`<span class="flag ${f.cls ?? ""}" style="--x:${f.x.toFixed(2)}">${f.html}</span>`);

  const name = r.issue ? issueLink(r.issue) : `<b>${esc(c.repo)}</b>`;
  const title = r.title ?? (st === "skipped" ? `skipped: ${c.cause}` : c.note ?? "");
  const latest = r.latest && (st === "running" || st === "died") ? `<code data-latest="${r.id}">${esc(r.latest)}</code>` : "";
  const links = [
    depth === 0 && c.commandLog ? `<a href="#" data-cmdlog="${c.id}" data-run="${r.id}"${state.openCommandLog === c.id ? ' aria-current="true"' : ""}>Command log</a>` : "",
    sessionLinks(r),
  ].join("");
  const used = depth === 0 ? usageText(sessionsOf(c)) : usageText(r.sessions ?? []);
  const chips = r.tickets ? `<div class="tchips">${(crit ?? r.tickets).map((t) =>
    `<span class="tchip t-${slug(t.state)} ${heat(t)}">${esc(t.issue)} <span>${esc(ticketWords(t))}</span></span>`).join("")}</div>
    ${crit ? `<div class="crit-note">critical path: ${crit.map((t) => esc(shortRef(t.issue))).join(" → ")} → Spec PR</div>` : ""}` : "";
  const extra = `${title ? `<div class="ll-title">${esc(title)}</div>` : ""}
    ${chips}
    ${r.harness || depth === 0 ? `<div class="ll-hme">${esc(rhm(r.harness ? r : c))}</div>` : ""}
    ${latest ? `<div class="ll-latest">${latest}</div>` : ""}
    ${st === "running" || st === "died" ? `<div class="ll-hb">${heartbeat(r)}</div>` : ""}
    ${used ? `<div class="ll-use">${esc(used)}</div>` : ""}
    ${links ? `<div class="ll-links">${links}</div>` : ""}`;
  const stop = depth === 0 && canStop(c) ? `<button type="button" class="stop" data-stop="${c.id}">Stop</button>` : "";
  // A Command's lane folds its children; a Spec run's lane folds its Tickets (open by default, so a Command's one
  // unfold still reaches them). A died or failed Command opens the first time it shows (GitHub opens failed steps).
  const top = depth === 0
    ? `<div class="ll-top" data-fold="${c.id}"${st === "died" || st === "failed" ? ' data-fold-default="open"' : ""}>`
    : r.tickets ? `<div class="ll-top" data-fold="${r.id}" data-fold-default="open">` : `<div class="ll-top">`;
  const inner = depth === 0 ? ` data-fold-in="${c.id}"` : "";
  const onCrit = crit === null && r.state && cmdOf(r) && isCritTicket(r);
  return `<div class="lane d${depth} st-${slug(st)}${onCrit ? " crit" : ""}" data-lane="${r.id}"${depth ? ` data-fold-in="${c.id}${spec ? ` ${spec}` : ""}"` : ""}>
    <div class="ll" style="--d:${depth}">
      ${top}<span class="kind">${esc(kind)}</span> ${name}${stop}</div>
      <div class="ll-stage flag-text ${f.cls ?? ""}">${f.html}</div>
      <div class="ll-x"${inner}>${extra}</div>
    </div>
    <div class="lt" data-open-run="${r.id}" title="Open this Run's logs">${marks.join("")}</div>
    <div class="lm"${inner}>${extra}</div>
  </div>`;
}
// A Ticket on a live Spec run's critical path: every Ticket gates the Spec PR, so all of a live one's are.
function isCritTicket(t) {
  const c = cmdOf(t);
  return !c.outcome && c.alive && (c.children ?? []).some((ch) => ch.tickets?.includes(t));
}

function lanesFor(c, now, x) {
  const out = [lane(c, c, 0, now, x)];
  for (const ch of c.children ?? []) {
    out.push(lane(ch, c, 1, now, x, ch.tickets && !c.outcome && c.alive ? criticalPath(ch) : null));
    for (const t of ch.tickets ?? []) out.push(lane(t, c, 2, now, x, undefined, ch.id));
  }
  for (const t of c.tickets ?? []) out.push(lane(t, c, 1, now, x));
  return out.join("");
}

function axis(now, x, start) {
  const ticks = [];
  const first = new Date(start); first.setSeconds(0, 0); first.setMinutes(first.getMinutes() < 30 ? 30 : 60);
  for (let t = first.getTime(); t <= now; t += 30 * 60_000) {
    const d = new Date(t);
    ticks.push(`<span class="tick${d.getMinutes() ? " half" : ""}" style="--x:${x(t).toFixed(2)}">${d.getMinutes() ? "" : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}</span>`);
  }
  ticks.push(`<span class="tick now" style="--x:${SPAN}">now</span>`);
  return `<div class="lane axis"><div class="ll"><span class="kind">last 3.5 h</span></div><div class="lt">${ticks.join("")}</div></div>`;
}

function confirmPanel() {
  const c = commands.find((k) => k.id === confirming);
  if (!c || !canStop(c)) return "";
  return `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
    <p><b>Stop the ${esc(c.kind)} on ${esc(c.stopTarget)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code> rather than put back in line, so the Day shift sees it before the factory takes it again.</p>
    <p class="keeps">What carries on:</p>${stopEffectsHTML(c)}
    <div class="confirm-actions">
      <button type="button" class="stop solid" data-do-stop="${c.id}">Stop · label needs-triage</button>
      <button type="button" data-do-requeue="${c.id}">Requeue instead</button>
      <button type="button" data-cancel>Cancel</button>
    </div>
    <p class="hint"><code>$ ${esc(stopHint(c))}</code> <span>·</span> <code>$ ${esc(requeueHint(c))}</code></p>
  </div>`;
}

function historyRow(c) {
  const s = status(c);
  const subj = c.issue ? `${issueLink(c.issue)} <span class="h-title">${esc(c.title)}</span>` : `<b>${esc(c.repo)}</b>${c.note ? ` <span class="h-title">${esc(c.note)}</span>` : ""}`;
  const runItem = (r, nested = false) => {
    const st = r.state ?? r.outcome ?? c.outcome;
    return `<li${nested ? ' class="nested"' : ""}><span class="kind">${esc(kindOf(r))}</span> ${issueLink(r.issue)} ${esc(r.title ?? "")}
      <span class="chip o-${slug(st)}">${esc(st)}</span>
      ${r.started && r.ended ? `<span class="h-dur">${dur(r.ended - r.started)}</span>` : ""}
      ${r.state && !["merged", "not started"].includes(r.state) ? `<span class="h-cause">${esc(ticketLine(r))}</span>` : r.cause ? `<span class="h-cause">${esc(r.cause)}</span>` : ""}
      ${r.pr ? `<span>PR ${issueLink(r.pr)}</span>` : ""}
      ${summaryText(r) ? `<span class="h-sum">${esc(summaryText(r))}</span>` : ""}
      ${usageText(r.sessions ?? []) ? `<span class="h-use">${esc(usageText(r.sessions ?? []))}</span>` : ""}
      <span class="ll-links">${sessionLinks(r)}</span></li>`;
  };
  const kids = [...(c.children ?? []).flatMap((ch) => [runItem(ch), ...(ch.tickets ?? []).map((t) => runItem(t, true))]), ...(c.tickets ?? []).map((t) => runItem(t))];
  const findings = (c.findings ?? []).map((f) => `<li><span class="chip sev-${f.sev}">${esc(f.sev)}</span> ${esc(f.title)}
    <span class="h-cause">${f.reproduced ? `reproduced · fixed by ${issueLink(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced · waits for the Day shift"}</span></li>`);
  const links = [c.commandLog ? `<a href="#" data-cmdlog="${c.id}" data-run="${c.id}">Command log</a>` : "", sessionLinks(c)].join("");
  return `<li class="h-item" id="h-${c.id}"><details${s.key === "failed" ? " open" : ""}><summary class="act">
      <time class="h-time">${esc(clock(c.ended))}</time>
      <span class="kind">${esc(c.kind)}</span>
      <span class="h-subj">${subj}</span>
      <span class="chip o-${s.key}">${esc(s.label)}</span>
      <span class="h-dur">${dur(c.ended - c.started)}</span>
      ${c.cause ? `<span class="h-cause">${esc(c.cause)}</span>` : ""}
    </summary>
    <div class="h-body">
      ${links ? `<div class="ll-links">${links}</div>` : `<p class="h-cause">Skipped before any work, so it keeps no Command log.</p>`}
      ${summaryText(c) ? `<div class="h-sum">${esc(summaryText(c))}</div>` : ""}
      <div class="ll-hme">${esc(rhm(c))}</div>
      ${usageText(sessionsOf(c)) ? `<div class="h-use">${esc(usageText(sessionsOf(c)))}</div>` : ""}
      ${kids.length ? `<ul class="h-kids">${kids.join("")}</ul>` : ""}
      ${findings.length ? `<h4>Security findings</h4><ul class="h-kids">${findings.join("")}</ul>` : ""}
    </div></details></li>`;
}

function drawer() {
  // The drawer follows the open log, whichever variant opened it; drawerRun picks the tabs when it agrees.
  const picked = runById(drawerRun);
  const r = state.openCommandLog
    ? (picked && cmdOf(picked)?.id === state.openCommandLog ? picked : commands.find((k) => k.id === state.openCommandLog))
    : (picked?.sessions?.includes(state.openSession) ? picked : runOfSession(state.openSession));
  if (!r) return "";
  const c = cmdOf(r);
  const s = sessions[state.openSession];
  const tabs = [
    c.commandLog ? `<button type="button" role="tab" data-cmdlog="${c.id}" data-run="${r.id}" aria-selected="${state.openCommandLog === c.id}">Command log</button>` : "",
    ...(r.sessions ?? []).map((id) => `<button type="button" role="tab" data-session="${id}" data-run="${r.id}" aria-selected="${state.openSession === id}">${esc(sessions[id].kind)}${sessions[id].live ? ' <i class="dot"></i>' : ""}</button>`),
  ].join("");
  const file = state.openCommandLog ? c.commandLog : s?.file ?? "";
  const body = state.openCommandLog ? `<pre class="cmdlog">${esc(commandLogText(c))}</pre>` : s ? `<div class="dr-log"></div>` : "";
  return `<aside class="drawer${drawerOpen ? " open" : ""}" aria-label="Logs">
    <div class="dr-head">
      <button type="button" class="dr-toggle" data-drawer-toggle aria-expanded="${drawerOpen}">${drawerOpen ? "▾" : "▴"}</button>
      <div class="dr-what"><b>${esc(r.issue ?? c.repo)}</b> <span>${esc(r.title ?? c.kind)}</span></div>
      <nav class="dr-tabs" role="tablist">${tabs}</nav>
      ${s?.live && !state.openCommandLog ? `<span class="live-tag">live</span>` : ""}
    </div>
    <div class="dr-body">${file ? `<div class="dr-file">${esc(file)}</div>` : ""}${body}</div>
  </aside>`;
}

function signInPage(root) {
  root.innerHTML = `<div class="lanes-page signin"><form>
    <div class="si-host"><span class="pulse"></span>${esc(host.name)}</div>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Sign in</button>
    <p>One machine's Console. No user name.</p></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

function openRun(id) {
  const r = runById(id);
  if (!r) return;
  const c = cmdOf(r);
  if (r.sessions?.length) { drawerRun = id; drawerOpen = true; openSession(r.sessions[0]); }
  else if (c.commandLog) { drawerRun = id; drawerOpen = true; openCommandLog(c.id); }
}

const listMode = () => (view ? view === "list" : matchMedia("(max-width: 600px)").matches);

// "Needs you" leads (SRE: page only on what is actionable, one item per problem). It sits outside every h2
// section, so it shows with them all closed.
function needsStrip(items) {
  if (!items.length) return `<div class="needs calm"><p class="needs-h">Nothing needs you. Everything live is moving.</p></div>`;
  return `<div class="needs"><p class="needs-h">Needs you <span class="n">${items.length}</span></p><ul>${items.map((n) =>
    `<li><button type="button" class="need ${n.level}" data-need="${esc(n.cmd.id)}" data-need-run="${esc(n.run.id)}">
      <span class="lvl">${n.level === "urgent" ? "act" : "look"}</span><b>${esc(n.ref)}</b> ${esc(n.what)}<span class="why">${esc(n.why ?? "")}</span></button></li>`).join("")}</ul></div>`;
}
// Opens the section and the Command's fold that hold an item, then brings its lane or history row into view.
function goTo(root, cmdId, runId) {
  const lane = root.querySelector(`[data-lane="${CSS.escape(runId)}"]`) ?? root.querySelector(`[data-lane="${CSS.escape(cmdId)}"]`);
  const target = lane ?? root.querySelector(`#h-${CSS.escape(cmdId)}`);
  if (!target) return;
  const h2 = target.closest("section")?.querySelector("h2[role=button]");
  if (h2?.getAttribute("aria-expanded") === "false") h2.click();
  const fold = root.querySelector(`[data-fold="${CSS.escape(cmdId)}"]`);
  if (lane && fold && !fold.classList.contains("is-unfolded")) fold.click();
  const det = target.querySelector("details");
  if (!lane && det) det.open = true;
  requestAnimationFrame(() => {
    target.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    target.classList.remove("is-pointed"); void target.offsetWidth; target.classList.add("is-pointed");
  });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  const now = Date.now(), start = now - SPAN * 60_000;
  const x = (ts) => (ts - start) / 60_000;
  const first = new Date(start); first.setSeconds(0, 0); first.setMinutes(first.getMinutes() < 30 ? 30 : 60);
  const onAxis = [
    ...commands.filter((c) => !c.outcome),
    ...commands.filter((c) => c.outcome && c.ended > start).sort((a, b) => b.ended - a.ended),
  ];
  const needs = needsYou();
  root.innerHTML = `<div class="lanes-page${drawerOpen ? " drawer-open" : ""}${listMode() ? " is-list" : ""}">
    <header class="top">
      <div class="host"><span class="pulse"></span><b>${esc(host.name)}</b> <span class="muted">Console</span></div>
      <a class="other" href="${host.other.url}">${esc(host.other.name)} Console →</a>
      <div class="now">now <b class="lanes-clock">${hms(now)}</b></div>
      <button type="button" class="link" data-signout>Sign out</button>
    </header>
    ${needsStrip(needs)}
    <section class="timeline">
    <h2>Lanes <span class="muted">· the last 3.5 h</span></h2>
    <p class="legend"><span class="sw seg-sw"></span>stage <span class="sw cur-sw"></span>current stage
      <span class="sw ghost-sw"></span>its usual range here <span class="sw warm-sw"></span>slower than usual, still working
      <span class="sw hot-sw"></span>silent ${SILENT_MIN}+ min: maybe stuck <span class="sw wait-sw"></span>waiting, not working
      <span class="sw crit-sw"></span>critical path <span class="sw att-sw">R1</span>Repair <span class="sw att-sw">↻</span>Resume
      <span class="sw died-sw">✕</span>process gone</p>
    <div class="view-toggle" role="group" aria-label="View"><button type="button" data-view="list" aria-pressed="${listMode()}">List</button><button type="button" data-view="timeline" aria-pressed="${!listMode()}">Timeline</button></div>
    ${confirmPanel()}
    <div class="lanes-scroll" data-scroll-end>
      <div class="lanes" style="--span:${SPAN};--gut:${GUTTER};--g0:${x(first.getTime()).toFixed(2)}">
        ${axis(now, x, start)}
        ${onAxis.map((c) => lanesFor(c, now, x)).join("")}
      </div>
    </div>
    </section>
    <section class="history">
      <h2>History</h2>
      <p class="muted">Every ended Command, newest first, back to the oldest logs on this machine.</p>
      <ol class="h-list">${endedCommands().sort((a, b) => b.ended - a.ended).map(historyRow).join("")}</ol>
    </section>
    ${drawer()}
  </div>`;

  // The latest event flashes when the live session moves on.
  new MutationObserver((ms) => ms.forEach((m) => {
    const el = m.target.nodeType === 1 ? m.target : m.target.parentElement;
    if (!el?.matches("[data-latest]")) return;
    el.classList.remove("is-new"); void el.offsetWidth; el.classList.add("is-new");
  })).observe(root, { subtree: true, childList: true, characterData: true });
  const scroller = root.querySelector(".lanes-scroll");
  if (!listMode()) requestAnimationFrame(() => { scroller.scrollLeft = scroller.scrollWidth; });
  const logEl = root.querySelector(".dr-log");
  if (logEl && drawerOpen) mountLog(logEl, state.openSession, root.querySelector(".dr-body"));

  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a, .lt[data-open-run]");
    if (!t) return;
    const d = t.dataset;
    if (d.need) { goTo(root, d.need, d.needRun); return; }
    if (d.view) { view = d.view; changed(); return; }
    if (d.stop) { confirming = d.stop; changed(); }
    else if (d.doStop) { confirming = null; stopCommand(d.doStop, false); }
    else if (d.doRequeue) { confirming = null; stopCommand(d.doRequeue, true); }
    else if ("cancel" in d) { confirming = null; changed(); }
    else if ("signout" in d) signOut();
    else if ("drawerToggle" in d) { drawerOpen = !drawerOpen; changed(); }
    else if (d.session) { e.preventDefault(); drawerRun = d.run; drawerOpen = true; openSession(d.session); }
    else if (d.cmdlog) { e.preventDefault(); drawerRun = d.run; drawerOpen = true; openCommandLog(d.cmdlog); }
    else if (d.openRun) openRun(d.openRun);
  });
}
