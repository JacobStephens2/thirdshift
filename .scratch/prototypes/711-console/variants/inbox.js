// PROTOTYPE variant "inbox": the agent-inbox console from docs/research/software-factory-console-trends.md §5.
// One keyboard-first list grouped by what the Day shift must do (Needs you, Ready for review, Landing, Working,
// Done, Resolved), each row a state word, the issue, a one-line now-line and an age. A peek pane shows the selected
// row lightly (health, usual range, heartbeat, tokens, the last 8 log entries, Stop); Enter opens the full log, and
// `t` tiles live Session logs side by side on a wide screen. Needs you folds like the other groups but loads open
// (its h2 carries data-section-default="open"); the default selection still comes from its first item.
import { host, sessions, commands } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, isLive, runsOf, ticketLine, canStop, stopCommand, stopHint,
  requeueHint, stopEffectsHTML, signIn, signOut, mountLog, commandLogText, changed, needsYou, health, isWorking,
  usualText, heartbeat, lastEventAt, usageText, sessionsOf, summaryText, endedCommands, liveCommands,
} from "../shared.js";
import { stageGridHTML, stageStripHTML, pastNote, bindStageGrid } from "../stagegrid.js";
import { lanesViewHTML, bindLanesView } from "../lanesview.js";
import { waterfallHTML, bindWaterfall } from "../waterfallview.js";

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
let wasPeekOpen = false;   // so the peek slides in only when it opens, not on every re-render
const rows = new Map();    // key → { cmd, run, group }
let sgNote = null;         // a past Run chosen in the stage grid that has no row of its own

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
const endWord = (o) => ({ merged: ["✓", "Merged", "done"], ready: ["◆", "Ready", "ready"], interrupted: ["■", "Stopped", "done"],
  skipped: ["–", "Skipped", "done"], failed: ["!", "Failed", "urgent"] }[o] ?? ["·", o, "done"]);

// ---------- Rows ----------
function row(key, cmd, run, group, [glyph, word, tone], now, ts, extra = "") {
  rows.set(key, { cmd, run, group });
  const stopping = cmd.stopping;
  const g = stopping ? ["…", "Stopping", "working"] : [glyph, word, tone];
  return `<div class="row t-${g[2]}${sel === key ? " is-sel" : ""}" data-row="${esc(key)}" tabindex="-1">
    <span class="st"><span class="gl${g[2] === "working" && !stopping ? " spin" : ""}" aria-hidden="true">${g[0]}</span> <span class="wd">${g[1]}</span></span>
    <span class="sj"><span class="kd">${esc(kindOf(run, cmd))}</span> ${subj(run, cmd)}</span>
    <span class="now">${now}</span>
    <span class="ag" title="${ts ? esc(clock(ts)) : ""}">${ts ? age(ts) : ""}</span>
    <span class="ac">${extra}</span>
  </div>`;
}
const stopBtn = (c) => (canStop(c) ? `<button type="button" class="stop" data-act="stop" data-id="${c.id}">Stop</button>` : "");

function needsRows(items) {
  return items.map((it) => {
    const w = needWord(it);
    const now = `<b>${esc(it.what)}</b> <span class="mu">· ${esc(it.why ?? "")}</span>`;
    return `<div class="entry">${row(`n:${it.id}`, it.cmd, it.run, "needs", w, now, needAge(it), isLive(it.cmd) ? stopBtn(it.cmd) : "")}</div>`;
  }).join("");
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
  return `
    <section class="grp ready"><h2 class="grp-h">Ready for review <span class="count">${ready.length}</span></h2>${readyHTML || empty("Nothing waits for review.")}</section>
    <section class="grp landing"><h2 class="grp-h">Landing <span class="count">${landing.length}</span></h2>${landingHTML || empty("No Merge run is landing.")}</section>
    <section class="grp working"><h2 class="grp-h">Working <span class="count">${working.length}</span></h2>${workingHTML || empty("Nothing else is running.")}</section>
    <section class="grp done"><h2 class="grp-h">Done <span class="count">${done.length}</span> <span class="sub">last 24 h</span></h2>${done.map((c) => endRow(c, "done")).join("") || empty("Nothing else ended today.")}</section>
    <section class="grp resolved"><h2 class="grp-h">Resolved <span class="count">${resolved.length}</span> <span class="sub">older, nothing waiting</span></h2>${resolved.map((c) => endRow(c, "resolved")).join("") || empty("Nothing older.")}</section>
    <section class="grp stages"><h2 class="grp-h">Stage grid <span class="sub">is this normal?</span></h2>${stageGridHTML(rows.get(sel)?.run.id)}${sgNote ? pastNote(sgNote) : ""}</section>
    <section class="grp lanes"><h2 class="grp-h">Lanes <span class="sub">how long each step took</span></h2>${lanesViewHTML()}</section>`;
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
  return `<aside class="peek${full ? " is-full" : ""}${entering ? " is-entering" : ""}" aria-label="Peek">
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
    ${log}
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

// ---------- Page ----------
function signInPage(root) {
  root.innerHTML = `<div class="ibx signin"><form><p class="si-host"><b>${esc(host.name)}</b> <span class="mu">Console</span></p>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label>
    <button type="submit">Sign in</button><p class="mu">One machine's Console. No user name.</p></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  bindKeys();
  rows.clear();
  const items = needsYou();
  const needsHTML = needsRows(items);
  const groupsHTML = groups(items);
  if (!sel || !rows.has(sel)) sel = items.length ? `n:${items[0].id}` : rows.keys().next().value ?? null;
  const urgent = items.filter((i) => i.level === "urgent").length;
  // The waterfall shows the selected row's Command in depth. Until the reader picks a row it shows a sample Spec run,
  // the live one (Pickup run → Spec run chart35#230 → its Tickets), since that is the shape a waterfall explains best.
  const sampleSpec = commands.find((c) => !c.outcome && c.alive && (c.tickets || c.children?.some((ch) => ch.tickets)))
    ?? commands.find((c) => c.tickets || c.children?.some((ch) => ch.tickets));
  const wfCmd = (picked ? rows.get(sel)?.cmd : null) ?? sampleSpec ?? rows.get(sel)?.cmd ?? commands[0];
  const waterfallSec = `<section class="grp waterfall"><h2 class="grp-h">Waterfall <span class="sub">one Command in depth</span></h2>${waterfallHTML(wfCmd, { selectedRunId: picked ? rows.get(sel)?.run.id : null })}</section>`;
  const peekHTML = peekOpen ? peek() : "";
  wasPeekOpen = peekOpen;
  root.innerHTML = `<div class="ibx${peekOpen ? " peek-open" : ""}">
    <header class="top">
      <div class="host"><b>${esc(host.name)}</b> <span class="mu">Console · inbox</span></div>
      <p class="keys" aria-label="Keys"><kbd>j</kbd><kbd>k</kbd> move · <kbd>space</kbd> peek · <kbd>↵</kbd> open · <kbd>t</kbd> tile · <span class="jk" tabindex="0" aria-describedby="jk-tip"><kbd>g</kbd>+letter jump<span class="jk-tip" id="jk-tip" role="tooltip"><b>g, then:</b><span><kbd>n</kbd> Needs you</span><span><kbd>r</kbd> Ready for review</span><span><kbd>l</kbd> Landing</span><span><kbd>w</kbd> Working</span><span><kbd>d</kbd> Done</span><span><kbd>o</kbd> Resolved</span><span><kbd>s</kbd> Stage grid</span><span><kbd>t</kbd> Lanes</span><span><kbd>f</kbd> Waterfall</span></span></span> · <kbd>z</kbd> fold section · <kbd>esc</kbd> close · <kbd>?</kbd> all keys</p>
      <nav><span data-sections-menu></span><a href="${host.other.url}">${esc(host.other.name)} →</a> <button type="button" class="lnk" data-act="signout">Sign out</button></nav>
    </header>
    <div class="cols">
      <div class="list">
        <span class="selbar" aria-hidden="true"></span>
        <section class="grp needs"><h2 class="grp-h" data-section-default="open">Needs you <span class="count${urgent ? " hot" : ""}">${items.length}</span> <span class="sub">${urgent} urgent · urgent first</span></h2>
          ${needsHTML || `<p class="empty">Nothing needs you. Every Run is working, landing or done.</p>`}</section>
        ${groupsHTML}
        ${waterfallSec}
        ${tilesHTML()}
      </div>
      ${peekHTML}
    </div>
  </div>`;

  // The peek's log: the last 8 entries (CSS hides the rest), or the whole log with its search bar.
  const box = root.querySelector(".peek .log-box");
  if (box) { const it = rows.get(sel); mountLog(box, pickSession(it.run, it.cmd)); }
  for (const t of root.querySelectorAll("[data-tile]")) mountLog(t, t.dataset.tile);

  // A new now-line flashes as the live session moves on.
  new MutationObserver((ms) => ms.forEach((m) => {
    const el = m.target.nodeType === 1 ? m.target : m.target.parentElement;
    if (!el?.matches?.("[data-latest]")) return;
    el.classList.remove("flash"); void el.offsetWidth; el.classList.add("flash");
  })).observe(root, { subtree: true, childList: true, characterData: true });

  root.addEventListener("click", (e) => onClick(e, root));
  bindStageGrid(root);
  bindLanesView(root);
  bindWaterfall(root);
  // The stage grid's looping shimmer and pulse keep their phase across re-renders, so j/k doesn't restart them.
  root.style.setProperty("--phase", `-${Date.now() % 36000}ms`);
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
    } else if (a === "do-stop") { confirming = null; stopCommand(id, false); }
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
// j/k move, space peeks, enter opens the full log, t tiles, esc closes, g then a letter jumps to a section (see JUMPS). ← and → stay with the
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
  if (document.documentElement.dataset.variant !== "inbox" || !state.signedIn) return;
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.target.closest?.("input, textarea, select, [contenteditable], .confirm")) return;
  const k = e.key;
  // g then a letter jumps to a section, in page order: n Needs you, r Ready, l Landing, w Working, d Done,
  // o Resolved (the older ones), s Stage grid, t Lanes (the timeline), f Waterfall.
  const JUMPS = { n: "needs", r: "ready", l: "landing", w: "working", d: "done", o: "resolved", s: "stages", t: "lanes", f: "waterfall" };
  if (Date.now() - gAt < 1500 && k in JUMPS) {
    gAt = 0; e.preventDefault();
    jump(JUMPS[k]);
    return;
  }
  if (k === "g") { gAt = Date.now(); return; }
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
  ["g  then  s", "Jump to the Stage grid"],
  ["g  then  t", "Jump to Lanes (the timeline)"],
  ["g  then  f", "Jump to the Waterfall"],
  ["t", "Tile the selected Run's live log beside the others (wide screens)"],
  ["esc", "Close the theme picker, the Stop confirm, the full log or the details, in that order"],
  ["?", "Show or hide this list"],
];
