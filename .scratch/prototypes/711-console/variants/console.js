// PROTOTYPE variant "console": one layout, themes as tokens. Jacob chose inbox's structure for the Console and wants
// both looks, inbox and inboxpress, so this is that structure once, with its appearance moved into themes (themes.js):
// colours, fonts, shape and effects are CSS custom properties, and a theme only sets them. Built in: Press
// rows of auto, light and dark: Focus (the default, weight follows need), Plain (inbox's look), Press (inboxpress's
// look; Press paper in light), Signal (state colours only), Garden and Factory; the reader
// can make, edit, export and import their own. Theme ▾ in the header (or Shift+T) switches. Everything else is inbox:
// one keyboard-first list grouped by what the Day shift must do (Needs you, Ready for review, Landing, Working, Done,
// Resolved), a peek, tiles, then the Stage grid, Lanes and Waterfall. Needs you folds but loads open.
import { host, sessions, commands } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, isLive, runsOf, ticketLine, canStop, stopCommand, stopHint,
  requeueHint, stopEffectsHTML, signIn, signOut, mountLog, commandLogText, changed, needsYou, health, isWorking,
  usualText, heartbeat, lastEventAt, usageText, sessionsOf, summaryText, endedCommands, liveCommands, usual, SILENT_MIN,
} from "../shared.js";
import { stageGridHTML, stageStripHTML, pastNote, bindStageGrid } from "../stagegrid.js";
import { lanesViewHTML, bindLanesView } from "../lanesview.js";
import { waterfallHTML, bindWaterfall } from "../waterfallview.js";
import { themeButtonHTML, themeHintHTML, themePanelHTML, bindThemes, applyTheme, toggleThemePanel, escapeThemes, panelOpen } from "../themes.js";

const DAY = 86_400_000;
let sel = null;            // the selected row's key
let picked = false;        // the reader has chosen a row (until then the waterfall shows a sample Spec run)
let peekOpen = false;      // the peek starts closed; choosing a row (click, space, enter) opens it
let full = false;          // the peek shows the full log instead of the last 8 entries
let peekLog = null;        // { kind: "session", id } | { kind: "cmd" } chosen in the peek
let confirming = null;     // Command id whose Stop is being confirmed
let tiles = [];            // Session log ids pinned as tiles, at most 4
let scrollSel = false;
let lastBar = null;
let wasPeekOpen = false;
let peekSel = null;        // the row the peek last showed, so a new selection crossfades
let firstPaint = true;     // the Needs you rows cascade in once, on the first paint only
const rows = new Map();    // key → { cmd, run, group }
let groupCounts = {};      // the healthy sections' sizes, for the phone's one-line summary

// docs/research/focus-theme-attention.md. When the Day shift last looked, per browser: read-only local state that
// marks what's new since then and never hides anything. Updated on leaving, or after 30 s on the page.
const LAST_KEY = "proto711:console-last-visit";
const lastVisit = (() => { try { const v = Number(localStorage.getItem(LAST_KEY)); return v > 0 ? v : null; } catch { return null; } })();
const markVisit = () => { try { localStorage.setItem(LAST_KEY, String(Date.now())); } catch { /* storage blocked */ } };
addEventListener("pagehide", markVisit);
setTimeout(markVisit, 30_000);
// Pause motion (WCAG 2.2.2): one switch, in words, that stops all activity motion; kept per browser.
const PAUSE_KEY = "proto711:console-motion-paused";
let motionPaused = (() => { try { return localStorage.getItem(PAUSE_KEY) === "1"; } catch { return false; } })();
// When this page first saw each Run silent: its alarm pulses three times from then, then holds, and a re-render
// doesn't restart it.
const silentSeen = new Map();
let sgNote = null;         // a past Run chosen in the stage grid that has no row of its own

// ---------- Pages: one level above sections ----------
// Inbox is the home (today's console); the others give one view the whole width. The page lives in the URL as
// ?page=, next to ?variant=, so a reload keeps it; 1–7 switch pages.
const PAGES = [
  { key: "inbox", label: "Inbox" }, { key: "lanes", label: "Lanes" }, { key: "stages", label: "Stages" },
  { key: "waterfall", label: "Waterfall" }, { key: "history", label: "History" }, { key: "logs", label: "Logs" },
  { key: "security", label: "Security" },
];
let page = (() => { const p = new URLSearchParams(location.search).get("page"); return PAGES.some((x) => x.key === p) ? p : "inbox"; })();
let wfPick = null;         // the Command the Waterfall page shows (null: the sample Spec run, as in Inbox)
let histOutcome = null;    // History filters: an outcome and a repository, or null for all
let histRepo = null;
let logPick = { kind: "session", id: "s239" }; // the log the Logs page reads
function setPage(key) {
  if (!PAGES.some((x) => x.key === key) || key === page) return;
  page = key;
  cursorHead = null;
  const u = new URL(location.href);
  u.searchParams.set("page", key);
  history.replaceState(null, "", u);
  scrollTo(0, 0);
  changed();
}

const wide = () => matchMedia("(min-width: 1000px)").matches;
const kindOf = (r, c) => r.kind ?? (c.tickets?.includes(r) || (c.children ?? []).some((ch) => ch.tickets?.includes(r)) ? "Ticket" : "Run");
const subj = (r, c) => r.issue
  ? `${issueLink(r.issue, "ref")} <span class="ttl">${esc(r.title ?? "")}</span>`
  : `<span class="ref">${esc(c.repo)}</span> <span class="ttl">${esc(c.note ?? c.cause ?? "")}</span>`;

// ---------- State words: a glyph and a word for every row, never colour alone ----------
function needWord(it) {
  const r = it.run, c = it.cmd;
  if (!c.outcome && !c.alive && r === c) return ["!", "Died", "urgent"];
  if (/no output/.test(it.what)) return ["!", "Silent", "urgent"];
  if (c.outcome === "failed") return ["!", "Failed", "urgent"];
  if (/slower|·/.test(it.what) && isWorking(r)) return ["?", "Slow", "look"];
  if (/needs-triage/.test(it.what)) return ["?", "Triage", "look"];
  if (/finding/.test(it.what)) return ["?", "Finding", "look"];
  return it.level === "urgent" ? ["!", "Urgent", "urgent"] : ["?", "Look", "look"];
}
function needAge(it) {
  const r = it.run, c = it.cmd;
  if (!c.outcome && !c.alive) return c.diedAt;
  if (isWorking(r)) return /no output/.test(it.what) ? lastEventAt(r) : r.stageSince;
  return c.ended ?? c.started;
}
// When the problem began, for "new since you last looked".
function needBegan(it) {
  const r = it.run, c = it.cmd, w = needWord(it)[1];
  if (w === "Died") return c.diedAt;
  if (w === "Silent") return lastEventAt(r) + SILENT_MIN * 60_000;
  if (w === "Slow") return r.stageSince + (usual(r.stage)?.hi ?? 0) * 60_000;
  return c.ended ?? c.started;
}
// A persisting problem rises in two static steps, in words: silent past 30 min; slow past twice its usual.
const SILENT_STEP2 = 30;
function escalation(it) {
  const r = it.run, w = needWord(it)[1];
  if (w === "Silent") {
    const ms = Date.now() - lastEventAt(r), step = ms >= SILENT_STEP2 * 60_000 ? 2 : 1;
    return { step, kind: "silent", what: `${r.stage} · silent ${dur(ms)}${step === 2 ? ` · past ${SILENT_STEP2}` : ""}` };
  }
  if (w === "Slow") {
    const u = usual(r.stage), ms = Date.now() - r.stageSince, ratio = u ? ms / 60_000 / u.hi : 1, step = ratio >= 2 ? 2 : 1;
    return { step, kind: "slow", what: `${r.stage} · ${dur(ms)} · ${step === 2 ? `${Math.floor(ratio)}× usual` : `past usual ${u?.lo}–${u?.hi} min`}` };
  }
  return { step: 1, kind: w.toLowerCase(), what: it.what };
}
// One problem, one item: Commands failing with the same cause become one item with a count.
function groupSameCause(items) {
  const out = [], byCause = new Map();
  for (const it of items) {
    const key = needWord(it)[1] === "Failed" && it.why ? it.why : null;
    if (key && byCause.has(key)) { byCause.get(key).also.push(it); continue; }
    const copy = { ...it, also: [] };
    if (key) byCause.set(key, copy);
    out.push(copy);
  }
  return out;
}
// Worst first: urgent before look; within each, the higher escalation step, then the longest-standing.
function sortNeeds(items) {
  const rank = (it) => [it.level === "urgent" ? 0 : 1, -escalation(it).step, needAge(it) ?? 0];
  return [...items].sort((a, b) => { const x = rank(a), y = rank(b); return x[0] - y[0] || x[1] - y[1] || x[2] - y[2]; });
}
const endWord = (o) => ({ merged: ["✓", "Merged", "done"], ready: ["◆", "Ready", "ready"], interrupted: ["■", "Stopped", "done"],
  skipped: ["–", "Skipped", "done"], failed: ["!", "Failed", "urgent"] }[o] ?? ["·", o, "done"]);

// ---------- Rows ----------
function row(key, cmd, run, group, [glyph, word, tone], now, ts, extra = "") {
  rows.set(key, { cmd, run, group });
  const stopping = cmd.stopping;
  const g = stopping ? ["…", "Stopping", "working"] : [glyph, word, tone];
  // A working Run carries a heartbeat dot that beats when its log moves; a silent one shows a flat line instead.
  const hb = isWorking(run) && !stopping && !cmd.outcome
    ? `<i class="hb-dot${health(run).key === "silent" ? " flat" : ""}" data-hb="${esc(run.id)}" aria-hidden="true"></i>` : "";
  const full = now.replace(/<[^>]+>/g, "").replace(/"/g, "&quot;");
  return `<div class="row t-${g[2]} w-${g[1].toLowerCase()}${sel === key ? " is-sel" : ""}" data-row="${esc(key)}" tabindex="-1">
    <span class="st"><span class="gl${g[2] === "working" && !stopping ? " spin" : ""}" aria-hidden="true">${g[0]}</span> <span class="wd">${g[1]}</span></span>
    <span class="sj"><span class="kd">${esc(kindOf(run, cmd))}</span> ${subj(run, cmd)}</span>
    <span class="now" title="${full}">${now}</span>
    <span class="ag" title="${ts ? esc(clock(ts)) : ""}">${hb}${ts ? age(ts) : ""}</span>
    <span class="ac">${extra}</span>
  </div>`;
}
const stopBtn = (c) => (canStop(c) ? `<button type="button" class="stop" data-act="stop" data-id="${c.id}">Stop</button>` : "");

const FLOOD = 7; // past this many items, the rest fold behind "N more" under a summary line
function needsRows(items) {
  const one = (it) => {
    const w = needWord(it), e = escalation(it);
    const isNew = lastVisit != null && needBegan(it) > lastVisit;
    const seen = silentSeen.get(it.run.id);
    const now = `<b>${esc(e.what)}</b>${it.also.length ? ` <span class="also">×${it.also.length + 1}: ${[it, ...it.also].map((x) => esc(x.ref)).join(", ")}</span>` : ""} <span class="mu">· ${esc(it.why ?? "")}</span>${isNew ? ` <span class="new-w"><i class="new-dot" aria-hidden="true"></i>new</span>` : ""}`;
    return `<div class="entry esc-${e.step} esc-${e.kind}"${seen ? ` style="--alarm-delay:-${Date.now() - seen}ms"` : ""}>${row(`n:${it.id}`, it.cmd, it.run, "needs", w, now, needAge(it), isLive(it.cmd) ? stopBtn(it.cmd) : "")}</div>`;
  };
  if (items.length <= FLOOD) return items.map(one).join("");
  const kinds = new Map();
  for (const it of items) { const k = needWord(it)[1].toLowerCase(); kinds.set(k, (kinds.get(k) ?? 0) + 1 + it.also.length); }
  const summary = [...kinds].sort((a, b) => b[1] - a[1]).map(([k, n]) => `${n} ${k}`).join(" · ");
  return `<p class="needs-sum">${esc(summary)}</p>${items.slice(0, FLOOD).map(one).join("")}
    <div class="needs-more" data-fold="needs-more">${items.length - FLOOD} more</div>
    <div class="needs-rest" data-fold-in="needs-more">${items.slice(FLOOD).map(one).join("")}</div>`;
}

// A Spec run's Tickets as a plan checklist under its row: ✓ merged, ▸ running, ⏸ waiting, ○ not started.
const PLAN = { merged: ["✓", "merged"], running: ["▸", "running"], waiting: ["⏸", "waiting"], "not started": ["○", "not started"],
  failed: ["✗", "failed"], interrupted: ["■", "stopped"] };
function plan(spec, cmd) {
  const n = spec.tickets.filter((t) => t.state === "merged").length;
  const items = spec.tickets.map((t) => {
    const [g, w] = PLAN[t.state] ?? ["·", t.state];
    const key = `r:${t.id}`;
    rows.set(key, { cmd, run: t, group: "plan" });
    const detail = t.state === "running" ? `${esc(t.stage)} · ${age(t.stageSince)} <span class="mu">${esc(usualText(t.stage))}</span>`
      : t.state === "waiting" ? `cannot start yet: waiting on ${t.blockedBy.map(esc).join(", ")}` : esc(ticketLine(t));
    return `<li class="pl p-${t.state.replace(" ", "-")}${sel === key ? " is-sel" : ""}" data-row="${esc(key)}"><span class="gl" aria-hidden="true">${g}</span> <span class="wd">${w}</span> ${issueLink(t.issue, "ref")} <span class="ttl">${esc(t.title)}</span> <span class="mu">— ${detail}</span></li>`;
  }).join("");
  const k = `plan-${spec.id}`;
  return `<div class="plan-h" data-fold="${k}" data-fold-default="open">Plan · Spec run ${issueLink(spec.issue, "ref")} · ${n} of ${spec.tickets.length} merged · Spec branch <code>${esc(spec.specBranch)}</code></div>
    <ol class="plan" data-fold-in="${k}">${items}</ol>`;
}

function liveNow(c) {
  const r = runsOf(c).find((x) => x.state === "running") ?? c;
  const bits = [];
  if (r.latest) bits.push(`<code data-latest="${r.id}">${esc(r.latest)}</code>`);
  if (r !== c && r.issue) bits.push(`<span class="mu">${esc(r.issue)}</span>`);
  if (r.stage) bits.push(`<span class="mu">${esc(r.stage)} · ${age(r.stageSince)}${usualText(r.stage) ? ` (${esc(usualText(r.stage))})` : ""}</span>`);
  if (lastEventAt(r)) bits.push(`<span class="mu">${heartbeat(r)}</span>`);
  return bits.join(" ");
}

function groups(items) {
  const needKeys = new Set(items.map((it) => it.cmd.id));
  const needRunIds = new Set(items.map((it) => it.run.id));
  // Ready for review: ended ready Commands, or the ready Run a Pass dispatched (the Security run's fix).
  const ready = [];
  for (const c of endedCommands()) {
    if (c.outcome !== "ready") continue;
    const fix = (c.children ?? []).find((ch) => ch.outcome === "ready");
    ready.push([c, fix ?? c]);
  }
  ready.sort((a, b) => (b[1].ended ?? b[0].ended) - (a[1].ended ?? a[0].ended));
  const readyHTML = ready.map(([c, r]) => `<div class="entry">${row(`r:${r.id}`, c, r, "ready", endWord("ready"),
    `PR ${issueLink(r.pr, "ref")}${summaryText(r) ? ` <span class="mu">· ${esc(summaryText(r))}</span>` : ""}${r !== c ? ` <span class="mu">· fix from the ${esc(c.kind)}</span>` : ""}`,
    r.ended ?? c.ended)}</div>`).join("");

  // Landing: live Merge runs at CI watch or Self-merge. One already in Needs you points there instead.
  const landing = liveCommands().filter((c) => c.merge && ["CI watch", "Self-merge"].includes(c.stage));
  const landingHTML = landing.map((c) => {
    if (needRunIds.has(c.id)) {
      const it = items.find((i) => i.run.id === c.id);
      // Its own key (l:), so j/k stops here too rather than treating it as the Needs you row it points to.
      return `<div class="entry">${row(`l:${it.id}`, c, c, "landing", ["↧", "Landing", "landing"],
        `<span class="mu">see Needs you:</span> ${esc(it.what)}`, c.stageSince, stopBtn(c))}</div>`;
    }
    return `<div class="entry">${row(`c:${c.id}`, c, c, "landing", ["↧", "Landing", "landing"], liveNow(c), c.stageSince, stopBtn(c))}</div>`;
  }).join("");

  // Working: every other live Command, with a Spec run's plan under it.
  const working = liveCommands().filter((c) => !landing.includes(c));
  const workingHTML = working.map((c) => {
    const spec = (c.children ?? []).find((ch) => ch.tickets) ?? (c.tickets ? c : null);
    return `<div class="entry">${row(`c:${c.id}`, c, c, "working", ["◐", "Working", "working"], liveNow(c), c.started, stopBtn(c))}${spec ? plan(spec, c) : ""}</div>`;
  }).join("");

  // Done in the last day; older ones fold into Resolved. Failed ones and anything needing triage live in Needs you.
  const ended = endedCommands().filter((c) => c.outcome !== "ready" && c.outcome !== "failed" && !needKeys.has(c.id))
    .sort((a, b) => b.ended - a.ended);
  const endRow = (c, group) => `<div class="entry">${row(`c:${c.id}`, c, c, group, endWord(c.outcome),
    c.outcome === "skipped" ? `<span class="mu">skipped: ${esc(c.cause)}</span>`
      : `${c.pr ? `PR ${issueLink(c.pr, "ref")} ` : ""}<span class="mu">${esc(summaryText(c) || c.cause || c.note || "")}${(c.children ?? []).map((ch) => ` · ${esc(ch.kind)} ${esc(ch.issue)} ${esc(ch.outcome)}`).join("")}</span>`,
    c.ended)}</div>`;
  const done = ended.filter((c) => Date.now() - c.ended < DAY);
  const resolved = ended.filter((c) => Date.now() - c.ended >= DAY);
  const empty = (t) => `<p class="empty">${t}</p>`;
  groupCounts = { ready: ready.length, landing: landing.length, working: working.length, done: done.length, resolved: resolved.length };
  return `
    <section class="grp ready"><h2 class="grp-h">Ready for review <span class="count">${ready.length}</span></h2>${readyHTML || empty("Nothing waits for review.")}</section>
    <section class="grp landing"><h2 class="grp-h">Landing <span class="count">${landing.length}</span></h2>${landingHTML || empty("No Merge run is landing.")}</section>
    <section class="grp working"><h2 class="grp-h">Working <span class="count">${working.length}</span></h2>${workingHTML || empty("Nothing else is running.")}</section>
    <section class="grp done"><h2 class="grp-h">Done <span class="count">${done.length}</span> <span class="sub">last 24 h</span></h2>${done.map((c) => endRow(c, "done")).join("") || empty("Nothing else ended today.")}</section>
    <section class="grp resolved"><h2 class="grp-h">Resolved <span class="count">${resolved.length}</span> <span class="sub">older, nothing waiting</span></h2>${resolved.map((c) => endRow(c, "resolved")).join("") || empty("Nothing older.")}</section>`;
}

// ---------- Peek ----------
function sessionIds(r, c) { return r === c ? sessionsOf(c) : r.sessions ?? []; }
function pickSession(r, c) {
  const ids = sessionIds(r, c);
  if (peekLog?.kind === "session" && ids.includes(peekLog.id)) return peekLog.id;
  return ids.find((id) => sessions[id]?.live) ?? ids.at(-1) ?? null;
}
function peek() {
  const it = rows.get(sel);
  if (!it) return `<aside class="peek is-empty" aria-label="Peek"><p class="mu">Choose a row, or press <kbd>space</kbd>, to peek at it.</p></aside>`;
  const { cmd: c, run: r } = it;
  const st = status(c);
  const h = health(r);
  const facts = [];
  const fact = (k, v) => v && facts.push(`<dt>${k}</dt><dd>${v}</dd>`);
  fact("State", r.state ? esc(r === c ? st.label : ticketLine(r)) : c.stopping ? "stopping…" : esc(st.label));
  if (isWorking(r)) fact("Health", `<span class="h-${h.key}">${esc(h.label)}</span>`);
  if (r.stage && (isWorking(r) || (!c.outcome && !c.alive))) fact("Stage", `${esc(r.stage)} · ${age(r.stageSince)}${usualText(r.stage) ? ` <span class="mu">${esc(usualText(r.stage))}</span>` : ""}`);
  if (lastEventAt(r) && (isWorking(r) || (!c.outcome && !c.alive))) fact("Heartbeat", heartbeat(r));
  fact("Harness", esc(rhm(r.harness ? r : c)));
  if (r.latest && (isWorking(r) || (!c.outcome && !c.alive))) fact("Latest", `<code data-latest="${r.id}">${esc(r.latest)}</code>`);
  fact("Tokens", esc(usageText(sessionIds(r, c))));
  if (r.pr) fact("PR", `${issueLink(r.pr, "ref")}${summaryText(r) ? ` <span class="mu">${esc(summaryText(r))}</span>` : ""}`);
  if (r.cause || (r === c && c.cause)) fact("Cause", esc(r.cause ?? c.cause));
  if (r.blockedBy) fact("Waiting on", r.blockedBy.map(esc).join(", "));
  if (c.findings && r === c) fact("Findings", c.findings.map((f) => `<span class="sev sev-${f.sev}">${esc(f.sev)}</span> ${esc(f.title)} <span class="mu">— ${f.reproduced ? `reproduced, fixed by ${esc(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced; waits for the Day shift"}</span>`).join("<br>"));
  if (r !== c) fact("Part of", `${esc(c.kind)} ${c.issue ? esc(c.issue) : `on ${esc(c.repo)}`}`);

  const sid = pickSession(r, c);
  const showCmd = peekLog?.kind === "cmd" && c.commandLog;
  const tabs = [
    ...sessionIds(r, c).map((id) => `<button type="button" class="tab" data-act="log-session" data-id="${id}" aria-pressed="${!showCmd && id === sid}">${esc(sessions[id].owner)} · ${esc(sessions[id].kind)}${sessions[id].live ? " ●" : ""}</button>`),
    c.commandLog ? `<button type="button" class="tab" data-act="log-cmd" aria-pressed="${!!showCmd}">Command log</button>` : "",
  ].join("");
  const log = showCmd ? `<pre class="cmdlog${full ? "" : " compact"}">${esc(full ? commandLogText(c) : commandLogText(c).split("\n").slice(-8).join("\n"))}</pre>`
    : sid ? `<div class="logwrap${full ? "" : " compact"}"><div class="log-box"></div></div>` : `<p class="mu">No Session log.</p>`;

  const confirm = confirming === c.id && canStop(c) ? `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
      <p><b>Stop this ${esc(c.kind)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code> rather than put back in line, so the Day shift sees it first.</p>
      ${stopEffectsHTML(c)}
      <div class="confirm-actions">
        <button type="button" class="stop solid" data-act="do-stop" data-id="${c.id}">Stop · label needs-triage</button>
        <button type="button" data-act="requeue" data-id="${c.id}">Requeue instead</button>
        <button type="button" data-act="cancel">Cancel</button>
      </div>
      <p class="hint"><code>$ ${esc(stopHint(c))}</code> · <code>$ ${esc(requeueHint(c))}</code></p>
    </div>` : "";

  const entering = !wasPeekOpen;
  const switching = wasPeekOpen && peekSel !== sel;
  peekSel = sel;
  return `<aside class="peek${full ? " is-full" : ""}${entering ? " is-entering" : ""}${switching ? " is-switching" : ""}" aria-label="Peek">
    <span class="crop tl" aria-hidden="true"></span><span class="crop tr" aria-hidden="true"></span><span class="crop bl" aria-hidden="true"></span><span class="crop br" aria-hidden="true"></span>
    <div class="pk-head">
      <div class="pk-title"><span class="kd">${esc(kindOf(r, c))}</span> ${subj(r, c)}</div>
      <div class="pk-acts">
        ${canStop(c) && !confirming ? `<button type="button" class="stop" data-act="stop" data-id="${c.id}">Stop</button>` : ""}
        ${sid && isWorking(r) ? `<button type="button" data-act="tile" title="Pin this Session log as a tile (t)">Tile</button>` : ""}
        <button type="button" class="x" data-act="close-peek" aria-label="Close the peek (esc)">✕</button>
      </div>
    </div>
    ${confirm}
    <dl class="facts">${facts.join("")}</dl>
    ${(() => { const strip = stageStripHTML(r === c && !r.stage ? null : r); return strip ? `<div class="pk-stages"><p class="pk-lbl">Stages</p>${strip}</div>` : ""; })()}
    <div class="pk-log-head"><span class="tabs">${tabs}</span>
      <button type="button" class="lnk" data-act="${full ? "compact" : "full"}">${full ? "Last 8 entries" : "Full log ↵"}</button></div>
    <div class="press-term${!showCmd && sid && sessions[sid]?.live ? " is-live" : ""}">
      <div class="term-bar"><span>${showCmd ? `Command log · ${esc(c.commandLog)}` : sid ? `Session log · ${esc(sessions[sid].owner)} · ${esc(sessions[sid].kind)} · ${esc(sessions[sid].harness)}` : "No log"}</span><span>${!showCmd && sid && sessions[sid]?.live ? '<span class="on-air">● live</span>' : full ? "full" : "last 8 entries"}</span></div>
      ${log}
    </div>
  </aside>`;
}

function tilesHTML() {
  if (!tiles.length) return "";
  return `<section class="tiles" aria-label="Tiled Session logs"><h3 class="grp-h">Tiles <span class="count">${tiles.length}</span> <span class="sub">t pins the selected Run · up to 4</span></h3>
    <div class="tile-grid">${tiles.map((id) => {
      const s = sessions[id];
      const r = commands.flatMap(runsOf).find((x) => x.sessions?.includes(id));
      return `<div class="tile"><div class="tile-bar"><b>${esc(s.owner)}</b> · ${esc(s.kind)} · ${esc(s.harness)}${s.live ? ' <span class="live">● live</span>' : ""}
        <span class="mu">${r ? heartbeat(r) : ""}</span><button type="button" class="x" data-act="untile" data-id="${id}" aria-label="Unpin">✕</button></div>
        <div class="tile-body" data-tile="${id}"></div></div>`;
    }).join("")}</div></section>`;
}

// ---------- Pages ----------
// The menu: a tab per page with its number and, where it helps, a count; one "☰ Inbox ▾" button on a phone.
function pagesNav(needCount) {
  const waiting = commands.flatMap((c) => c.findings ?? []).filter((f) => f.waits).length;
  const counts = { inbox: needCount, history: endedCommands().length, security: waiting };
  const tab = (p, i) => `<button type="button" class="pg-tab" data-act="page" data-id="${p.key}"${p.key === page ? ' aria-current="page"' : ""}>
      <span class="pg-n">${i + 1}</span> ${p.label}${counts[p.key] ? ` <span class="pg-c">${counts[p.key]}</span>` : ""}</button>`;
  const cur = PAGES.find((x) => x.key === page);
  return `<nav class="pages" aria-label="Pages">${PAGES.map(tab).join("")}</nav>
    <details class="pages-compact"><summary>☰ ${cur.label} ▾</summary><div class="pc-list">${PAGES.map(tab).join("")}</div></details>`;
}
const pageHead = (title, sub) => `<div class="pg-head"><p class="pg-title">${title}</p>${sub ? `<p class="pg-sub">${sub}</p>` : ""}</div>`;

// The Lanes page's time window, in minutes: 3.5 h, 12 h or 24 h. Older Commands are on History.
let laneSpan = 210;
const SPANS = [[210, "3.5 h"], [720, "12 h"], [1440, "24 h"]];
function lanesPage() {
  const picker = `<span class="pg-spans" role="group" aria-label="Time window">${SPANS.map(([m, label]) =>
    `<button type="button" class="pg-span" data-act="span" data-id="${m}" aria-pressed="${laneSpan === m}">${label}</button>`).join("")}</span>`;
  return `${pageHead("Lanes", `how long each step took · ${picker} · older Commands are on <button type="button" class="lnk" data-act="page" data-id="history">History</button>`)}
    <div class="pg-body">${lanesViewHTML({ dots: true, span: laneSpan })}</div>`;
}
function stagesPage() {
  return `${pageHead("Stages", "is this normal? each stage's time against its usual range · choose a column to peek at it")}
    <div class="pg-body">${stageGridHTML(rows.get(sel)?.run.id)}${sgNote ? pastNote(sgNote) : ""}</div>`;
}
function waterfallPage(sampleSpec) {
  const cmd = commands.find((c) => c.id === wfPick) ?? sampleSpec ?? commands[0];
  const live = commands.filter(isLive), died = commands.filter((c) => !c.outcome && !c.alive);
  const ended = endedCommands().slice().sort((a, b) => b.ended - a.ended);
  const chip = (c) => `<button type="button" class="chip" data-act="wf-pick" data-id="${c.id}" aria-pressed="${c === cmd}">${esc(c.kind)} ${esc(c.issue ?? c.repo)}${c.outcome ? ` <span class="mu">${esc(c.outcome)}</span>` : ""}</button>`;
  const group = (label, list) => (list.length ? `<span class="chips-l">${label}</span>${list.map(chip).join("")}` : "");
  return `${pageHead("Waterfall", "one Command in depth, from Run down to tool call")}
    <div class="chips" role="group" aria-label="Command">${group("Live", live)}${group("Died", died)}${group("Ended", ended)}</div>
    <div class="pg-body">${waterfallHTML(cmd, { selectedRunId: rows.get(sel)?.cmd === cmd ? rows.get(sel)?.run.id : null })}</div>`;
}
function historyPage() {
  const all = endedCommands().slice().sort((a, b) => b.ended - a.ended);
  const outcomes = ["merged", "ready", "failed", "interrupted", "skipped"];
  const repos = [...new Set(commands.map((c) => c.repo))];
  const list = all.filter((c) => (!histOutcome || c.outcome === histOutcome) && (!histRepo || c.repo === histRepo));
  const chip = (act, id, on, n) => `<button type="button" class="chip" data-act="${act}" data-id="${id}" aria-pressed="${on}">${id}${n != null ? ` <span class="mu">${n}</span>` : ""}</button>`;
  const rowsHTML = list.map((c) => `<div class="entry">${row(`c:${c.id}`, c, c, "history", endWord(c.outcome),
    c.outcome === "skipped" ? `<span class="mu">skipped: ${esc(c.cause)}</span>`
      : `${c.pr ? `PR ${issueLink(c.pr, "ref")} ` : ""}<span class="mu">${esc(summaryText(c) || c.cause || c.note || "")}${(c.children ?? []).map((ch) => ` · ${esc(ch.kind)} ${esc(ch.issue)} ${esc(ch.outcome)}`).join("")}</span>`,
    c.ended)}</div>`).join("");
  return `${pageHead("History", "every ended Command, newest first, back to the oldest logs on this machine")}
    <div class="chips" role="group" aria-label="Filter by outcome"><span class="chips-l">Outcome</span>${outcomes.map((o) => chip("hf-outcome", o, histOutcome === o, all.filter((c) => c.outcome === o).length)).join("")}</div>
    <div class="chips" role="group" aria-label="Filter by repository"><span class="chips-l">Repository</span>${repos.map((r) => chip("hf-repo", r, histRepo === r, all.filter((c) => c.repo === r).length)).join("")}</div>
    <div class="pg-rows">${rowsHTML || `<p class="empty">No ended Command matches.</p>`}</div>`;
}
function logsPage() {
  const ss = Object.values(sessions);
  const cmdLogs = commands.filter((c) => c.commandLog);
  const item = (kind, id, title, meta, live) => `<li><button type="button" class="lg-item" data-act="read-log" data-kind="${kind}" data-id="${id}" aria-pressed="${logPick.kind === kind && logPick.id === id}">
      <span class="lg-t">${title}${live ? ' <span class="lg-live">● live</span>' : ""}</span><span class="lg-m">${meta}</span></button></li>`;
  const sessionItems = ss.map((x) => item("session", x.id, `${esc(x.owner)} · ${esc(x.kind)}`,
    `${esc(x.harness)} · ${x.live ? "live" : "ended"} · ${x.events?.length ?? 0} events${usageText([x.id]) ? ` · ${esc(usageText([x.id]).split(" · ")[0])}` : ""}`, x.live)).join("");
  const cmdItems = cmdLogs.map((c) => item("cmd", c.id, `${esc(c.kind)} ${esc(c.issue ?? c.repo)}`,
    `plain text · ${esc(c.outcome ?? (c.alive ? "live" : "died"))}`, isLive(c))).join("");
  let reader;
  if (logPick.kind === "cmd") {
    const c = commands.find((x) => x.id === logPick.id) ?? cmdLogs[0];
    reader = `<div class="press-term"><div class="term-bar"><span><b>Command log</b> · ${esc(c.kind)} ${esc(c.issue ?? c.repo)} · ${esc(c.commandLog)}</span><span>plain text</span></div>
      <pre class="cmdlog lr-cmd">${esc(commandLogText(c))}</pre></div>`;
  } else {
    const x = sessions[logPick.id] ?? ss[0];
    reader = `<div class="press-term${x.live ? " is-live" : ""}"><div class="term-bar"><span><b>Session log</b> · ${esc(x.owner)} · ${esc(x.kind)} · ${esc(x.harness)} · ${esc(x.file ?? "")}</span><span>${x.live ? '<span class="on-air">● live</span>' : "ended"}</span></div>
      <div class="log-box lr-box" data-sid="${esc(x.id)}"></div></div>`;
  }
  return `${pageHead("Logs", "every Session log and Command log on this machine · choose one to read it")}
    <div class="logs">
      <div class="lg-lists"><p class="pk-lbl">Session logs <span class="mu">${ss.length}</span></p><ul class="lg-list">${sessionItems}</ul>
        <p class="pk-lbl">Command logs <span class="mu">${cmdLogs.length}</span></p><ul class="lg-list">${cmdItems}</ul></div>
      <div class="lg-reader">${reader}</div>
    </div>`;
}
function securityPage() {
  const runs = commands.filter((c) => c.kind === "Security run");
  const body = runs.map((c) => {
    const rowHTML = `<div class="entry">${row(`c:${c.id}`, c, c, "security", c.outcome ? endWord(c.outcome) : ["◐", "Working", "working"],
      `<span class="mu">${esc(summaryText(c) || c.note || c.cause || "")}</span>`, c.ended ?? c.started)}</div>`;
    const findings = (c.findings ?? []).map((f) => {
      const fix = f.fix ? (c.children ?? []).find((ch) => ch.issue === f.fix) : null;
      return `<li class="sf"><span class="sev sev-${f.sev}">${esc(f.sev)}</span> <b>${esc(f.title)}</b>
        <span class="sf-m">${f.reproduced ? "reproduced" : "not reproduced"}${f.fix ? ` · fixed by ${issueLink(f.fix, "ref")} (${esc(f.fixOutcome ?? fix?.outcome ?? "")}${fix?.pr ? `, PR ${issueLink(fix.pr, "ref")}` : ""})` : ""}${f.waits ? " · <b>waits for the Day shift</b>, never fixed unattended" : ""}</span></li>`;
    }).join("");
    const sess = [...(c.sessions ?? []), ...(c.children ?? []).flatMap((ch) => ch.sessions ?? [])].map((id) => sessions[id]).filter(Boolean)
      .map((x) => `<button type="button" class="chip" data-act="read-log" data-kind="session" data-id="${x.id}">${esc(x.owner)} · ${esc(x.kind)}</button>`).join("");
    return `${rowHTML}
      <p class="pk-lbl">Findings <span class="mu">${(c.findings ?? []).length}</span></p><ul class="sf-list">${findings || `<li class="empty">No findings.</li>`}</ul>
      <p class="pk-lbl">Sessions <span class="mu">open in the Logs page</span></p><div class="chips">${sess}</div>`;
  }).join("");
  return `${pageHead("Security", "Security runs, their findings and their sessions")}
    <div class="pg-rows">${body || `<p class="empty">No Security run on this machine yet.</p>`}</div>`;
}

// ---------- Page ----------
function signInPage(root) {
  applyTheme();
  root.innerHTML = `<div class="ibx signin"><form><p class="slug">thirdshift · Console</p><p class="si-host misreg">${esc(host.name)}</p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Sign in</button><p class="mu">One machine's Console. No user name.</p></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  // Whether any Command is live (and not stopping): the title strip (Factory's conveyor, Garden's bed) moves only then,
  // since motion is kept for what thirdshift is doing.
  document.documentElement.dataset.cLive = liveCommands().some((c) => !c.stopping) ? "on" : "off";
  if (!state.signedIn) return signInPage(root);
  bindKeys();
  rows.clear();
  const items = sortNeeds(groupSameCause(needsYou()));
  // Silence first seen by this page, so each alarm pulses three times from then and holds.
  const now0 = Date.now();
  for (const c of liveCommands()) for (const r of runsOf(c)) if (health(r).key === "silent" && !silentSeen.has(r.id)) silentSeen.set(r.id, now0);
  document.documentElement.dataset.cPaused = motionPaused ? "on" : "off";
  // Inbox's rows are built on every page, so the peek, the grid, the lanes and the waterfall can select into them.
  const needsHTML = needsRows(items);
  const groupsHTML = groups(items);
  const urgent = items.filter((i) => i.level === "urgent").length;
  // The Waterfall page opens on a sample Spec run until the reader picks a Command: the live one (Pickup run → Spec
  // run chart35#230 → its Tickets), since that is the shape a waterfall explains best.
  const sampleSpec = commands.find((c) => !c.outcome && c.alive && (c.tickets || c.children?.some((ch) => ch.tickets)))
    ?? commands.find((c) => c.tickets || c.children?.some((ch) => ch.tickets));
  const inboxHTML = `<section class="grp needs"><h2 class="grp-h" data-section-default="open">Needs you <span class="count${urgent ? " hot" : ""}">${items.length}</span> <span class="sub">${urgent} urgent · urgent first</span></h2>
          ${needsHTML || `<p class="empty">Nothing needs you. Every Run is working, landing or done.</p>`}</section>
        <p class="healthy-line" aria-label="Healthy sections">${[["ready", "Ready"], ["landing", "Landing"], ["working", "Working"], ["done", "Done"], ["resolved", "Resolved"]]
          .map(([k, label]) => `<button type="button" class="hl-b" data-act="open-sec" data-id="${k}">${label} ${groupCounts[k] ?? 0}</button>`).join(" · ")}</p>
        ${groupsHTML}
        ${tilesHTML()}`;
  const mainHTML = page === "inbox" ? inboxHTML
    : page === "lanes" ? lanesPage()
    : page === "stages" ? stagesPage()
    : page === "waterfall" ? waterfallPage(sampleSpec)
    : page === "history" ? historyPage()
    : page === "logs" ? logsPage()
    : securityPage();
  // The selection is checked after the page's own rows are built (History's, say), so it isn't reset under them.
  if (!sel || !rows.has(sel)) sel = items.length ? `n:${items[0].id}` : rows.keys().next().value ?? null;
  const peekHTML = peekOpen ? peek() : "";
  wasPeekOpen = peekOpen;
  root.innerHTML = `<div class="ibx${peekOpen ? " peek-open" : ""}">
    <header class="top">
      <p class="slug">thirdshift · Console</p>
      <div class="host misreg">${esc(host.name)}</div>
      <p class="keys" aria-label="Keys"><kbd>j</kbd><kbd>k</kbd> move · <kbd>space</kbd> peek · <kbd>↵</kbd> open · <kbd>t</kbd> tile · <span class="jk" tabindex="0" aria-describedby="jk-tip"><kbd>g</kbd>+letter jump<span class="jk-tip" id="jk-tip" role="tooltip"><b>g, then:</b><span><kbd>n</kbd> Needs you</span><span><kbd>r</kbd> Ready for review</span><span><kbd>l</kbd> Landing</span><span><kbd>w</kbd> Working</span><span><kbd>d</kbd> Done</span><span><kbd>o</kbd> Resolved</span><span><kbd>s</kbd> Stages page</span><span><kbd>t</kbd> Lanes page</span><span><kbd>f</kbd> Waterfall page</span></span></span> · <kbd>1</kbd>–<kbd>7</kbd> pages · <kbd>⇧T</kbd> theme · <kbd>z</kbd> fold section · <kbd>esc</kbd> close · <kbd>?</kbd> all keys</p>
      <nav><span class="th-slot">${themeButtonHTML()}${themePanelHTML()}</span><button type="button" class="lnk motion-btn" data-act="motion" aria-pressed="${motionPaused}">${motionPaused ? "Resume motion" : "Pause motion"}</button>${page === "inbox" ? "<span data-sections-menu></span>" : ""}<a href="${host.other.url}">${esc(host.other.name)} Console →</a> <button type="button" class="lnk" data-act="signout">Sign out</button></nav>
      <div class="colorbar" aria-hidden="true"><i class="cb-c"></i><i class="cb-m"></i><i class="cb-y"></i><i class="cb-k"></i><i class="cb-c50"></i><i class="cb-m50"></i><i class="cb-y50"></i></div>
    </header>
    ${pagesNav(items.length)}
    ${themeHintHTML()}
    <div class="cols">
      <div class="list pg-${page}">
        <h1 class="vh">${esc(PAGES.find((x) => x.key === page).label)}</h1>
        <span class="selbar" aria-hidden="true"></span>
        ${mainHTML}
      </div>
      ${peekHTML}
    </div>
  </div>`;

  // The peek's log: the last 8 entries (CSS hides the rest), or the whole log with its search bar.
  const box = root.querySelector(".peek .log-box");
  if (box) { const it = rows.get(sel); mountLog(box, pickSession(it.run, it.cmd)); }
  for (const t of root.querySelectorAll("[data-tile]")) mountLog(t, t.dataset.tile);
  // The Logs page's reader: a Session log with its search, raw view and download bar (the live one keeps appending).
  const reader = root.querySelector(".lr-box");
  if (reader) mountLog(reader, reader.dataset.sid);

  // A new now-line flashes as the live session moves on.
  new MutationObserver((ms) => ms.forEach((m) => {
    const el = m.target.nodeType === 1 ? m.target : m.target.parentElement;
    if (!el?.matches?.("[data-latest]")) return;
    el.classList.remove("flash"); void el.offsetWidth; el.classList.add("flash");
    // The same Run's heartbeat dots beat once.
    for (const dot of root.querySelectorAll(`[data-hb="${CSS.escape(el.dataset.latest)}"]:not(.flat)`)) {
      dot.classList.remove("beat"); void dot.offsetWidth; dot.classList.add("beat");
    }
  })).observe(root, { subtree: true, childList: true, characterData: true });
  // The cascade runs on the first paint only. app.js builds each render on a copy of the root, which keeps its
  // classes, so the class is taken off again rather than left to repeat on every j or k.
  root.classList.remove("is-first");
  if (firstPaint) { root.classList.add("is-first"); firstPaint = false; setTimeout(() => root.classList.remove("is-first"), 800); }
  // Looping pulses keep their phase across re-renders, so moving the selection doesn't restart them
  // (36 s is a whole number of every loop: 1.8, 2.4 and 4 s).
  root.style.setProperty("--phase", `-${Date.now() % 36000}ms`);
  // Alarms outside the rows (grid cell, lane, waterfall span) count their three pulses from the first silence seen.
  const firstSilent = Math.min(...silentSeen.values());
  if (Number.isFinite(firstSilent)) root.style.setProperty("--alarm-delay", `-${Date.now() - firstSilent}ms`);

  root.addEventListener("click", (e) => onClick(e, root));
  bindStageGrid(root);
  bindLanesView(root);
  bindWaterfall(root);
  bindThemes(root, changed);
  showMark(root);
  requestAnimationFrame(() => { placeBar(root); if (cursorHead) cursorAt(cursorHead); });
  // Clicking a heading puts the j/k cursor there, so j carries on from the section just opened or closed.
  root.addEventListener("click", (e) => {
    const h = e.target.closest(".proto-toggle");
    if (h && !e.target.closest("a, button")) { markHidden = false; showMark(root); cursorAt(h.dataset.sectionKey); }
  });
  // Folding a section moves or hides the selected row without a re-render; the bar follows (and hides with it).
  root.addEventListener("proto:layout", () => placeBar(root));
}

// The selection bar slides from the last selected row to the new one.
function placeBar(root) {
  const list = root.querySelector(".list"), bar = root.querySelector(".selbar");
  if (cursorHead && bar) { bar.style.opacity = 0; return; }
  const el = [...root.querySelectorAll(`[data-row="${CSS.escape(sel ?? "")}"]`)].find((x) => x.offsetParent);
  if (!list || !bar) return;
  if (!el) { bar.style.opacity = 0; return; }
  const lr = list.getBoundingClientRect(), er = el.getBoundingClientRect();
  const next = { top: er.top - lr.top, h: er.height };
  if (lastBar) { bar.style.transition = "none"; bar.style.transform = `translateY(${lastBar.top}px)`; bar.style.height = `${lastBar.h}px`; void bar.offsetWidth; bar.style.transition = ""; }
  bar.style.opacity = 1;
  bar.style.transform = `translateY(${next.top}px)`;
  bar.style.height = `${next.h}px`;
  lastBar = next;
  if (scrollSel) { el.scrollIntoView({ block: "nearest" }); scrollSel = false; }
}

function select(key, open = true) {
  cursorHead = null;
  markHidden = false; // choosing a row (a click, say) shows the selection mark again
  picked = true;
  if (key !== sel) { peekLog = null; full = false; confirming = null; }
  sel = key;
  if (open) peekOpen = true;
  scrollSel = true;
  changed();
}

function onClick(e, root) {
  const act = e.target.closest("[data-act]");
  if (act) {
    const { act: a, id } = act.dataset;
    e.preventDefault();
    if (a === "stop") {
      const it = [...rows].find(([, v]) => v.cmd.id === id && v.run === v.cmd) ?? [...rows].find(([, v]) => v.cmd.id === id);
      if (it && it[0] !== sel) sel = it[0];
      peekOpen = true; confirming = id; changed();
    } else if (a === "span") { laneSpan = Number(id); changed(); }
    else if (a === "motion") {
      motionPaused = !motionPaused;
      try { localStorage.setItem(PAUSE_KEY, motionPaused ? "1" : "0"); } catch { /* storage blocked: lasts until reload */ }
      changed();
    }
    else if (a === "open-sec") {
      const h = document.querySelector(`#app .grp.${CSS.escape(id)} > h2`);
      if (h && h.getAttribute("aria-expanded") === "false") h.click();
      h?.scrollIntoView({ block: "start" });
    }
    else if (a === "do-stop") { confirming = null; stopCommand(id, false); }
    else if (a === "requeue") { confirming = null; stopCommand(id, true); }
    else if (a === "cancel") { confirming = null; changed(); }
    else if (a === "close-peek") { peekOpen = false; full = false; confirming = null; changed(); }
    else if (a === "full") { full = true; changed(); }
    else if (a === "compact") { full = false; changed(); }
    else if (a === "log-session") { peekLog = { kind: "session", id }; changed(); }
    else if (a === "log-cmd") { peekLog = { kind: "cmd" }; changed(); }
    else if (a === "tile") tileSelected();
    else if (a === "untile") { tiles = tiles.filter((t) => t !== id); changed(); }
    else if (a === "signout") signOut();
    else if (a === "page") setPage(id);
    else if (a === "wf-pick") { wfPick = id; changed(); }
    else if (a === "hf-outcome") { histOutcome = histOutcome === id ? null : id; changed(); }
    else if (a === "hf-repo") { histRepo = histRepo === id ? null : id; changed(); }
    else if (a === "read-log") { logPick = { kind: act.dataset.kind, id }; if (page !== "logs") setPage("logs"); else changed(); }
    return;
  }
  if (e.target.closest("a")) return;
  const g = e.target.closest("[data-sg-col]");
  if (g && root.contains(g)) { pickFromGrid(g.dataset.sgCol, g.dataset.sgRun); return; }
  const l = e.target.closest("[data-lv-run]");
  if (l && root.contains(l)) { pickFromLanes(l.dataset.lvRun, l.dataset.lvCmd); return; }
  // A waterfall span selects its Run's row; a session span also shows that session in the peek. A tool-call group
  // unfolds its calls inline instead (waterfallview.js handles that).
  const w = e.target.closest("[data-wf-span]");
  if (w && root.contains(w)) { if (w.dataset.wfKind !== "tools") pickFromWaterfall(w.dataset.wfRun, w.dataset.wfCmd, w.dataset.wfSid); return; }
  const r = e.target.closest("[data-row]");
  if (r && root.contains(r)) select(r.dataset.row);
}

// A grid column selects its Run's row (or, failing that, its Command's), as clicking the row would. A past Run with
// no row of its own gets a one-line note under the grid instead.
function pickFromGrid(colId, runId) {
  const cmd = runId && commands.find((c) => runsOf(c).some((x) => x.id === runId));
  const key = (runId && [...rows].find(([, v]) => v.run.id === runId)?.[0])
    ?? (cmd && ([...rows].find(([, v]) => v.cmd.id === cmd.id && v.run === v.cmd) ?? [...rows].find(([, v]) => v.cmd.id === cmd.id))?.[0]);
  if (key) { sgNote = null; select(key); return; }
  sgNote = colId;
  changed();
}

// A lane selects its Run's row (a plan row for a Ticket), or failing that its Command's, as clicking the row would.
function pickFromLanes(runId, cmdId) {
  const key = [...rows].find(([, v]) => v.run.id === runId)?.[0]
    ?? ([...rows].find(([, v]) => v.cmd.id === cmdId && v.run === v.cmd) ?? [...rows].find(([, v]) => v.cmd.id === cmdId))?.[0];
  if (key) select(key);
}

// A waterfall span selects its Run's row (or its Command's) and opens the peek, without scrolling the list away from
// the waterfall the reader is looking at. A session span also switches the peek's log to that session.
function pickFromWaterfall(runId, cmdId, sid) {
  const key = [...rows].find(([, v]) => v.run.id === runId)?.[0]
    ?? ([...rows].find(([, v]) => v.cmd.id === cmdId && v.run === v.cmd) ?? [...rows].find(([, v]) => v.cmd.id === cmdId))?.[0];
  if (!key) return;
  if (key !== sel) { peekLog = null; full = false; confirming = null; }
  sel = key;
  peekOpen = true;
  if (sid) peekLog = { kind: "session", id: sid };
  changed();
}

function tileSelected() {
  const it = rows.get(sel);
  if (!it) return;
  const id = pickSession(it.run, it.cmd);
  if (!id) return;
  tiles = tiles.includes(id) ? tiles.filter((t) => t !== id) : [...tiles, id].slice(-4);
  changed();
}

// ---------- Keyboard ----------
// j/k move, space peeks, enter opens the full log, t tiles, Shift+T themes, esc closes, g then a letter jumps to a section (see JUMPS). ← and → stay with the
// prototype bar, and no key fires while typing or inside the Stop confirm.
let keysBound = false;
let gAt = 0;
function bindKeys() {
  if (keysBound) return;
  keysBound = true;
  addEventListener("keydown", onKey);
}
// j/k walks section headings as well as rows, in page order, so a closed section is one stop and space folds it
// open and closed (as space opens and closes the peek on a row). cursorHead is the heading j/k is on, if any.
let cursorHead = null;
// esc with nothing left to close hides the selection mark (a heading's ring, a row's highlight and bar); the next j
// or k shows it again and carries on from the same place.
let markHidden = false;
const showMark = (root = document.getElementById("app")) => root?.classList.toggle("mark-hidden", markHidden);
function stops() {
  const seen = new Set();
  return [...document.querySelectorAll("#app .proto-toggle, #app [data-row]")]
    .filter((el) => el.offsetParent && !el.closest(".peek") && !el.closest(".proto-hidden"))
    .map((el) => (el.matches(".proto-toggle") ? { head: el.dataset.sectionKey } : { row: el.dataset.row }))
    .filter((x) => { const id = x.head ? `h:${x.head}` : `r:${x.row}`; return !seen.has(id) && seen.add(id); });
}
const headEl = (key) => document.querySelector(`#app .proto-toggle[data-section-key="${CSS.escape(key ?? "")}"]`);
// Put the cursor on a heading: mark it, hide the row selection bar, and bring it into view.
function cursorAt(key) {
  cursorHead = key;
  for (const h of document.querySelectorAll("#app .proto-toggle.is-cursor")) h.classList.remove("is-cursor");
  const h = headEl(key);
  if (!h) { cursorHead = null; return; }
  h.classList.add("is-cursor");
  for (const r of document.querySelectorAll("#app [data-row].is-sel")) r.classList.remove("is-sel");
  const bar = document.querySelector("#app .selbar");
  if (bar) bar.style.opacity = 0;
  h.scrollIntoView({ block: "nearest" });
}
function visibleRows() {
  const seen = new Set();
  return [...document.querySelectorAll("#app [data-row]")].filter((el) => el.offsetParent && !el.closest(".peek"))
    .map((el) => el.dataset.row).filter((k) => !seen.has(k) && seen.add(k));
}
function jump(cls) {
  // Sections live on the Inbox page: from any other page, go there first.
  if (page !== "inbox") setPage("inbox");
  const sec = document.querySelector(`#app .grp.${cls}`);
  // A section the reader has hidden (the Sections menu) stays out of the way: its key does nothing.
  if (!sec || sec.classList.contains("proto-hidden")) return;
  const h2 = sec.querySelector("h2[aria-expanded='false']");
  if (h2) h2.click();
  const first = sec.querySelector("[data-row]");
  if (first) { cursorHead = null; select(first.dataset.row, false); }
  else { const hk = (sec.querySelector(".proto-toggle") ?? sec.querySelector("h2"))?.dataset.sectionKey; if (hk) cursorAt(hk); else sec.scrollIntoView({ block: "start" }); }
}
function onKey(e) {
  if (document.documentElement.dataset.variant !== "console" || !state.signedIn) return;
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.target.closest?.("input, textarea, select, [contenteditable], .confirm")) return;
  const k = e.key;
  // Shift+T opens and closes the themes; plain t tiles. Esc closes the theme editor, then the panel, before the peek.
  if ((k === "T" || k === "t") && e.shiftKey) { e.preventDefault(); toggleThemePanel(); return; }
  if (k === "Escape" && escapeThemes()) { e.preventDefault(); return; }
  if (panelOpen() && e.target.closest?.(".th-panel")) return;
  // g then a letter jumps to an Inbox section, in page order: n Needs you, r Ready, l Landing, w Working, d Done,
  // o Resolved (the older ones); or opens a view's page: s Stages, t Lanes (the timeline), f Waterfall.
  const JUMPS = { n: "needs", r: "ready", l: "landing", w: "working", d: "done", o: "resolved" };
  const PAGE_JUMPS = { s: "stages", t: "lanes", f: "waterfall" };
  if (Date.now() - gAt < 1500 && (k in JUMPS || k in PAGE_JUMPS)) {
    gAt = 0; e.preventDefault();
    if (k in PAGE_JUMPS) setPage(PAGE_JUMPS[k]); else jump(JUMPS[k]);
    return;
  }
  if (k === "g") { gAt = Date.now(); return; }
  // 1–7 switch pages (never inside the g window, which belongs to section jumps, nor with the theme panel open).
  if (/^[1-7]$/.test(k) && !e.shiftKey && Date.now() - gAt >= 1500 && !panelOpen()) {
    e.preventDefault(); setPage(PAGES[Number(k) - 1].key); return;
  }
  // A focused heading (clicked, say) handles its own space and enter; don't also toggle the peek.
  if ((k === " " || k === "Enter") && e.target.closest?.(".proto-toggle")) return;
  if (k === "j" || k === "k") {
    if (markHidden) { markHidden = false; showMark(); }
    const list = stops();
    if (!list.length) return;
    e.preventDefault();
    const i = cursorHead ? list.findIndex((x) => x.head === cursorHead) : list.findIndex((x) => x.row === sel);
    // Wraps: j past the last stop (Waterfall's heading, say) goes back to the first (Needs you), and k the other way.
    const n = list.length;
    const next = list[i < 0 ? 0 : (i + (k === "j" ? 1 : -1) + n) % n];
    if (next.head) cursorAt(next.head);
    else { cursorHead = null; select(next.row, peekOpen); }
  } else if ((k === " " || k === "Enter") && cursorHead && headEl(cursorHead)) {
    e.preventDefault();
    const h = headEl(cursorHead);
    (h.matches("h2") ? h : h.querySelector("h2")).click();
  } else if (k === " ") {
    if (e.target.closest?.("a, button")) return;
    e.preventDefault();
    peekOpen = !peekOpen; if (!peekOpen) { full = false; confirming = null; }
    changed();
  } else if (k === "Enter") {
    if (e.target.closest?.("a, button")) return;
    e.preventDefault(); peekOpen = true; full = true; changed();
  } else if (k === "t") {
    tileSelected();
  } else if (k === "Escape") {
    if (confirming) confirming = null;
    else if (full) full = false;
    else if (peekOpen) peekOpen = false;
    else if (!markHidden) { e.preventDefault(); markHidden = true; showMark(); return; }
    else return;
    changed();
  }
}

// Every shortcut in words, for the ? list app.js shows (the legend in the header is the short form).
export const KEYS = [
  ["j  k", "Move down / up through rows and section headings"],
  ["space", "On a row: open or close its details (the peek). On a section heading: fold or unfold the section"],
  ["enter", "On a row: open its full Session log. On a section heading: fold or unfold the section"],
  ["z", "Fold the section you're in (the one you opened last)"],
  ["g  then  n", "Jump to Needs you"],
  ["g  then  r", "Jump to Ready for review"],
  ["g  then  l", "Jump to Landing"],
  ["g  then  w", "Jump to Working"],
  ["g  then  d", "Jump to Done"],
  ["g  then  o", "Jump to Resolved (the older ones)"],
  ["g  then  s", "Open the Stages page (the stage grid)"],
  ["g  then  t", "Open the Lanes page (the timeline)"],
  ["g  then  f", "Open the Waterfall page"],
  ["t", "Tile the selected Run's live log beside the others (wide screens)"],
  ["1–7", "Switch page: 1 Inbox, 2 Lanes, 3 Stages, 4 Waterfall, 5 History, 6 Logs, 7 Security"],
  ["Shift T", "Open or close the theme picker"],
  ["esc", "Close the theme picker, the Stop confirm, the full log or the details, in that order"],
  ["?", "Show or hide this list"],
];
