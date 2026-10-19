// PROTOTYPE (#711, Q27): what every Console variant shares. The data model, in-memory state, ages, the stub
// Stop and sign-in, and the Session log folding and markup. Variants own their layout and styling; the Session
// log markup here uses neutral `sl-*` classes that each variant styles for itself.

import { commands, sessions, pastRuns, NOW0 } from "./data.js";

export const state = { signedIn: true, openSession: "s239", openCommandLog: null };

const listeners = new Set();
export const onChange = (fn) => listeners.add(fn);
export const changed = () => listeners.forEach((fn) => fn());

// ---------- Text helpers ----------
export const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
export const rhm = (r) => `${r.harness} · ${r.model} · ${r.effort}`;

export function dur(ms) {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
  const d = Math.floor(h / 24);
  return h % 24 ? `${d} d ${h % 24} h` : `${d} d`;
}
// A live age that the ticker below keeps current.
export const age = (ts) => `<time data-since="${ts}" datetime="${new Date(ts).toISOString()}">${dur(Date.now() - ts)}</time>`;
export function clock(ts) {
  const d = new Date(ts), today = new Date(NOW0);
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  if (d.toDateString() === today.toDateString()) return hm;
  const y = new Date(NOW0 - 86_400_000);
  if (d.toDateString() === y.toDateString()) return `yesterday ${hm}`;
  return `${d.toLocaleDateString([], { weekday: "short" })} ${hm}`;
}
export const issueLink = (ref, cls = "") => ref ? `<a class="${cls}" href="#${esc(ref.replace("#", "-"))}" title="Opens the issue on GitHub">${esc(ref)}</a>` : "";
export const stopHint = (cmd) => `thirdshift stop ${cmd.stopTarget}`;
export const requeueHint = (cmd) => `thirdshift stop --requeue ${cmd.stopTarget}`;

setInterval(() => {
  for (const t of document.querySelectorAll("time[data-since]")) t.textContent = dur(Date.now() - Number(t.dataset.since));
}, 5000);

// ---------- Commands ----------
// live: process alive, no outcome. died: process gone, no outcome. Otherwise the outcome.
export function status(c) {
  if (c.stopping) return { key: "stopping", label: "stopping…" };
  if (!c.outcome && c.alive) return { key: "live", label: "live" };
  if (!c.outcome) return { key: "died", label: `died at ${c.stage}` };
  return { key: c.outcome, label: c.outcome };
}
export const isLive = (c) => !c.outcome && c.alive;
export const liveCommands = () => commands.filter(isLive);
// Died Commands sit with the live ones: they need a look, and they have no end time.
export const attentionCommands = () => commands.filter((c) => !c.outcome && !c.alive);
export const endedCommands = () => commands.filter((c) => c.outcome);
export const allCommands = () => commands;
// The Command's subject: its issue, or what the Pass did.
export const subject = (c) => c.issue ? `${c.issue}` : c.children?.[0]?.issue ?? c.repo;
// Every Run a Command holds, for the "where is it now" line: the Command itself, its children, a Spec run's Tickets.
export function runsOf(c) {
  const out = [];
  const walk = (r) => { out.push(r); (r.children ?? []).forEach(walk); (r.tickets ?? []).forEach((t) => out.push(t)); };
  walk(c);
  return out;
}
export const activeRun = (c) => runsOf(c).find((r) => r.state === "running") ?? (c.stage ? c : null);
export const ticketCounts = (spec) => spec.tickets.reduce((n, t) => ((n[t.state] = (n[t.state] ?? 0) + 1), n), {});
export function ticketLine(t) {
  switch (t.state) {
    case "running": return `${t.stage} · ${dur(Date.now() - t.stageSince)}`;
    case "merged": return `merged into the Spec branch`;
    case "failed": return `failed: ${t.cause}`;
    case "waiting": return `waiting on ${t.blockedBy.join(", ")}`;
    case "interrupted": return `interrupted at ${t.stage}`;
    default: return "not started";
  }
}

// ---------- Health: slow is not stuck ----------
// A stall is silence: nothing in the Session log or Command log for SILENT_MIN. A stage past its usual range is
// only slow, and says so in words. "Usually" comes from earlier Runs on this machine (data.js pastRuns).
export const SILENT_MIN = 10;
export function lastEventAt(r) {
  for (const id of r.sessions ?? []) if (sessions[id]?.live) return sessions[id].lastEventAt;
  return r.lastEvent ?? null;
}
export function usual(stage) {
  const xs = pastRuns.map((p) => p.mins[stage]).filter((m) => m != null).sort((a, b) => a - b);
  if (xs.length < 3) return null;
  const q = (f) => xs[Math.min(xs.length - 1, Math.floor(f * (xs.length - 1) + 0.5))];
  return { lo: q(0.25), hi: q(0.75), n: xs.length };
}
export const usualText = (stage) => { const u = usual(stage); return u ? `usually ${u.lo}–${u.hi} min` : ""; };
// A Run that is working now: a running Ticket, or a live Command or child with a stage of its own.
export const isWorking = (r) => r.state === "running" || (!r.state && !r.outcome && r.stage && r.stage !== "Tickets" && r.alive !== false);
export function health(r) {
  if (!isWorking(r)) return { key: "", label: "" };
  const last = lastEventAt(r);
  if (last && Date.now() - last > SILENT_MIN * 60_000) return { key: "silent", label: `no output for ${dur(Date.now() - last)}` };
  const u = usual(r.stage);
  if (u && r.stageSince && (Date.now() - r.stageSince) / 60_000 > u.hi) return { key: "slow", label: `slower than usual (${u.lo}–${u.hi} min), still working` };
  return { key: "ok", label: "on pace" };
}
// "last output 4 s ago", kept current by the ticker.
export const heartbeat = (r) => { const t = lastEventAt(r); return t ? `<span class="hb">last output ${age(t)} ago</span>` : ""; };

// What needs the Day shift, one item per problem, most urgent first. urgent: nothing will fix it unattended.
export function needsYou() {
  const out = [];
  for (const c of commands) {
    if (!c.outcome && !c.alive) out.push({ id: c.id, level: "urgent", cmd: c, run: c, ref: c.issue ?? c.repo,
      what: `died at ${c.stage}`, why: `Its process is gone with no outcome, ${dur(Date.now() - c.diedAt)} ago; nothing will finish it.` });
    if (isLive(c)) for (const r of runsOf(c)) {
      const h = health(r);
      if (h.key === "silent") out.push({ id: r.id, level: "urgent", cmd: c, run: r, ref: r.issue, what: `${r.stage}, ${h.label}`, why: "Nothing has been logged for a while, so it may be stuck rather than slow." });
      if (h.key === "slow") out.push({ id: r.id, level: "look", cmd: c, run: r, ref: r.issue, what: `${r.stage} · ${dur(Date.now() - r.stageSince)}`, why: `Slower than usual (${usualText(r.stage)}), but its log is still moving.` });
    }
    if (c.outcome === "failed") out.push({ id: c.id, level: "urgent", cmd: c, run: c, ref: c.issue ?? c.repo, what: "failed", why: c.cause });
    if (c.outcome === "interrupted" && /needs-triage/.test(c.cause ?? "")) out.push({ id: c.id, level: "look", cmd: c, run: c, ref: c.issue, what: "labelled needs-triage", why: c.cause });
    for (const f of c.findings ?? []) if (f.waits) out.push({ id: `${c.id}:${f.title}`, level: "look", cmd: c, run: c, ref: c.repo, what: `${f.sev} Security finding waits`, why: f.title });
  }
  return out.sort((a, b) => (a.level === b.level ? 0 : a.level === "urgent" ? -1 : 1));
}

// Tokens and session length, added up over Session logs.
const short = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : String(n));
export function usageOf(ids) {
  return ids.map((id) => sessions[id]).filter((x) => x?.input != null)
    .reduce((a, x) => ({ input: a.input + x.input, cached: a.cached + x.cached, output: a.output + x.output, minutes: a.minutes + x.minutes, n: a.n + 1 }),
      { input: 0, cached: 0, output: 0, minutes: 0, n: 0 });
}
export const sessionsOf = (c) => runsOf(c).flatMap((r) => r.sessions ?? []);
export function usageText(ids) {
  const u = usageOf(ids);
  return u.n ? `${short(u.input)} tokens in (${short(u.cached)} cached) · ${short(u.output)} out · ${dur(u.minutes * 60_000)} in ${u.n} session${u.n === 1 ? "" : "s"}` : "";
}
// One line saying what came of an ended Run.
export function summaryText(r) {
  const x = r.summary;
  if (!x) return "";
  return [x.diffstat, x.repairs ? `${x.repairs} Repair${x.repairs === 1 ? "" : "s"}${x.repairFor ? ` (${x.repairFor})` : ""}` : "no Repairs",
    x.resumes ? `${x.resumes} Resume` : "", x.unaddressed ? `${x.unaddressed} Unaddressed finding${x.unaddressed === 1 ? "" : "s"}` : ""].filter(Boolean).join(" · ");
}

// ---------- Stop (stub) and sign-in ----------
export function stopCommand(id, requeue) {
  const c = commands.find((x) => x.id === id);
  if (!c || !isLive(c) || c.stopping) return;
  // Stop is gentle: salvage, Claim ending and Spec accounting finish first, so the Command shows "stopping…" a while.
  c.stopping = true;
  changed();
  setTimeout(() => {
    c.stopping = false; c.alive = false; c.outcome = "interrupted"; c.ended = Date.now();
    c.cause = requeue ? `stopped from the Console; ${c.stopTarget} put back as ready-for-agent`
      : `stopped from the Console; ${c.stopTarget} labelled needs-triage`;
    for (const r of runsOf(c)) if (r.state === "running") { r.state = "interrupted"; }
    for (const s of Object.values(sessions)) if (runsOf(c).some((r) => r.sessions?.includes(s.id))) s.live = false;
    changed();
  }, 3000);
}
// Stop shows only while the process is alive and not already stopping.
export const canStop = (c) => isLive(c) && !c.stopping;
// What the Stop confirm says carries on, so pressing it holds no surprises.
export function stopEffects(c) {
  const kids = runsOf(c).filter((r) => r !== c && r.state === "running");
  return [
    "Commits already pushed stay on origin.",
    "Unfinished work is pushed first, or its worktree is kept if the push fails.",
    "The Claim ends and Spec accounting finishes before the process exits, so this can take a minute.",
    kids.length ? `It also stops ${kids.map((k) => `${k.issue} (${k.stage})`).join(", ")}.` : null,
  ].filter(Boolean);
}
export const stopEffectsHTML = (c) => `<ul class="stop-effects">${stopEffects(c).map((t) => `<li>${esc(t)}</li>`).join("")}</ul>`;

export function signIn() { state.signedIn = true; changed(); }
export function signOut() { state.signedIn = false; changed(); }
export function openSession(id) { state.openSession = id; state.openCommandLog = null; changed(); }
export function openCommandLog(id) { state.openCommandLog = id; state.openSession = null; changed(); }

// ---------- Session logs ----------
// Folds a Harness's events into display items: text, thinking, sys, usage, and tools groups. Consecutive tool
// calls, with only their results between them, fold into one group; anything else ends the group.
export function foldLog(s) {
  const items = [];
  const calls = new Map();
  let group = null;
  const push = (it) => { group = null; items.push(it); };
  const addCall = (call) => {
    if (!group) { group = { k: "tools", calls: [] }; items.push(group); }
    group.calls.push(call); calls.set(call.id, call);
  };
  let thinkTokens = null;
  for (const e of s.events ?? []) {
    if (s.harness === "claude") {
      if (e.type === "system" && e.subtype === "thinking_tokens") { thinkTokens = e.tokens; continue; }
      if (e.type === "system" && e.subtype === "init") { push({ k: "sys", text: `session started · ${e.model}` }); continue; }
      if (e.type === "system") {
        const label = { task_started: "subagent started", task_progress: "subagent", task_notification: `subagent ${e.status ?? ""}` }[e.subtype] ?? e.subtype;
        push({ k: "sys", text: `${label}: ${e.description}` }); continue;
      }
      if (e.type === "tool_progress") { const c = calls.get(e.tool_use_id); if (c) c.elapsed = e.elapsed_time_seconds; continue; }
      for (const b of e.message?.content ?? []) {
        if (b.type === "text") push({ k: "text", text: b.text });
        else if (b.type === "thinking") { push({ k: "thinking", text: b.thinking, tokens: thinkTokens }); thinkTokens = null; }
        else if (b.type === "tool_use") addCall({ id: b.id, name: b.name, input: b.input, running: true });
        else if (b.type === "tool_result") { const c = calls.get(b.tool_use_id); if (c) Object.assign(c, { result: b.content, isError: !!b.is_error, running: false }); }
      }
    } else {
      if (e.type === "thread.started") push({ k: "sys", text: `thread started · ${e.thread_id}` });
      else if (e.type === "turn.started") push({ k: "sys", text: "turn started" });
      else if (e.type === "turn.completed") push({ k: "usage", text: `turn completed · ${e.usage.input_tokens.toLocaleString()} input (${e.usage.cached_input_tokens.toLocaleString()} cached) · ${e.usage.output_tokens.toLocaleString()} output` });
      else if (e.item?.type === "agent_message") push({ k: "text", text: e.item.text });
      else if (e.item?.type === "command_execution") {
        if (e.type === "item.started") addCall({ id: e.item.id, name: "Bash", input: { command: e.item.command }, running: true });
        else { const c = calls.get(e.item.id); if (c) Object.assign(c, { result: e.item.aggregated_output, isError: e.item.exit_code !== 0, exit: e.item.exit_code, running: false }); }
      }
    }
  }
  return items;
}

// One line naming a tool call, used in group summaries and the "latest event" line.
export function callLine(c) {
  const i = c.input ?? {};
  switch (c.name) {
    case "Bash": return { tool: "Bash", arg: i.command };
    case "Edit": return { tool: "Edit", arg: i.file_path };
    case "Read": return { tool: "Read", arg: i.file_path };
    case "Grep": return { tool: "Grep", arg: i.pattern };
    default: return { tool: c.name, arg: JSON.stringify(i) };
  }
}
export const latestText = (c) => { const l = callLine(c); return `${l.tool}: ${l.arg}`; };

function callStatus(c) {
  if (c.running) return `<span class="sl-st sl-run">running${c.elapsed ? ` · ${c.elapsed} s` : ""}</span>`;
  if (c.isError) return `<span class="sl-st sl-err">${c.exit != null ? `exit ${c.exit}` : "error"}</span>`;
  return `<span class="sl-st sl-ok">ok</span>`;
}
function diff(oldS, newS) {
  const del = oldS ? oldS.split("\n").map((l) => `<span class="sl-del">- ${esc(l)}</span>`) : [];
  const add = newS.split("\n").map((l) => `<span class="sl-add">+ ${esc(l)}</span>`);
  return { html: [...del, ...add].join("\n"), plus: add.length, minus: del.length };
}
function callBody(c) {
  const i = c.input ?? {};
  switch (c.name) {
    case "Bash": return c.result != null ? `<pre class="sl-out">${esc(c.result)}</pre>` : `<pre class="sl-out sl-pending">…</pre>`;
    case "Edit": return `<pre class="sl-diff">${diff(i.old_string, i.new_string).html}</pre>`;
    case "Read": return `<pre class="sl-out">${esc(c.result ?? "")}</pre>`;
    case "Grep": return `<pre class="sl-out">${esc(c.result ?? "")}</pre>`;
    default: return `<pre class="sl-out">${esc(JSON.stringify(i, null, 2))}\n\n${esc(c.result ?? "")}</pre>`;
  }
}
function callSummary(c) {
  const i = c.input ?? {};
  switch (c.name) {
    case "Bash": return `<span class="sl-tool">Bash</span> <code class="sl-arg">$ ${esc(i.command)}</code>`;
    case "Edit": { const d = diff(i.old_string, i.new_string); return `<span class="sl-tool">Edit</span> <code class="sl-arg">${esc(i.file_path)}</code> <span class="sl-meta"><span class="sl-plus">+${d.plus}</span> <span class="sl-minus">−${d.minus}</span></span>`; }
    case "Read": return `<span class="sl-tool">Read</span> <code class="sl-arg">${esc(i.file_path)}</code>${i.limit ? ` <span class="sl-meta">lines ${i.offset}–${i.offset + i.limit - 1}</span>` : ""}`;
    case "Grep": { const n = (c.result ?? "").split("\n").filter(Boolean).length; return `<span class="sl-tool">Grep</span> <code class="sl-arg">${esc(i.pattern)}</code> <span class="sl-meta">in ${esc(i.path ?? ".")}${c.result != null ? ` · ${n} match${n === 1 ? "" : "es"}` : ""}</span>`; }
    default: return `<span class="sl-tool">${esc(c.name)}</span>`;
  }
}

// Which folds are open: the newest group, and Edit calls in it, unless the reader closed them.
const toggled = new Map(); // key → open
function isOpen(key, dflt) { return toggled.has(key) ? toggled.get(key) : dflt; }

// Failures open by default (the failing call and its group); reads and greps are quieter than edits, commands
// and commits. q filters to the items that mention it.
const QUIET = new Set(["Read", "Grep"]);
export function logHTML(id, q = "") {
  const s = sessions[id];
  if (!s.events) return `<p class="sl-empty">This Session log is kept on the machine; the prototype has no content for it.</p>`;
  const items = foldLog(s);
  const lastGroup = items.findLastIndex((it) => it.k === "tools");
  const needle = q.trim().toLowerCase();
  const hit = (it) => !needle || JSON.stringify(it).toLowerCase().includes(needle);
  let shown = 0;
  const li = items.map((it, n) => {
    if (!hit(it)) return "";
    shown++;
    const key = `${id}:${n}`;
    switch (it.k) {
      case "text": return `<li class="sl-text">${esc(it.text)}</li>`;
      case "sys": return `<li class="sl-sys">${esc(it.text)}</li>`;
      case "usage": return `<li class="sl-usage">${esc(it.text)}</li>`;
      case "thinking": return `<li class="sl-think"><details data-key="${key}"${isOpen(key, !!needle) ? " open" : ""}><summary>Thinking${it.tokens ? ` · ${it.tokens.toLocaleString()} tokens` : ""}</summary><p>${esc(it.text)}</p></details></li>`;
      case "tools": {
        const counts = it.calls.reduce((m, c) => m.set(c.name, (m.get(c.name) ?? 0) + 1), new Map());
        const names = [...counts].map(([k, v]) => (v > 1 ? `${k} ×${v}` : k)).join(" · ");
        const failed = it.calls.some((c) => c.isError);
        const calls = it.calls.map((c) => {
          const ck = `${id}:${c.id}`;
          const quiet = QUIET.has(c.name) && !c.isError;
          const dflt = c.isError || (n === lastGroup && c.name === "Edit") || (!!needle && JSON.stringify(c).toLowerCase().includes(needle));
          return `<li class="sl-call sl-${c.name.toLowerCase()}${quiet ? " sl-quiet" : ""}${c.isError ? " sl-failed" : ""}"><details data-key="${ck}"${isOpen(ck, dflt) ? " open" : ""}><summary>${callSummary(c)} ${callStatus(c)}</summary>${callBody(c)}</details></li>`;
        }).join("");
        const running = it.calls.some((c) => c.running);
        const quiet = it.calls.every((c) => QUIET.has(c.name) && !c.isError);
        return `<li class="sl-group${running ? " is-running" : ""}${quiet ? " sl-quiet" : ""}${failed ? " sl-failed" : ""}"><details data-key="${key}"${isOpen(key, n === lastGroup || failed || !!needle) ? " open" : ""}><summary><span class="sl-count">${it.calls.length} tool call${it.calls.length === 1 ? "" : "s"}</span> <span class="sl-names">${esc(names)}</span>${failed ? ' <span class="sl-st sl-err">a call failed</span>' : ""}</summary><ol class="sl-calls">${calls}</ol></details></li>`;
      }
    }
  }).join("");
  return `${needle ? `<p class="sl-matches">${shown} of ${items.length} entries mention “${esc(q.trim())}”</p>` : ""}<ol class="sl">${li}</ol>`;
}

// The raw JSONL, one numbered line per event, each line an anchor to send someone.
function rawHTML(id, q = "") {
  const s = sessions[id];
  const needle = q.trim().toLowerCase();
  const lines = (s.events ?? []).map((e, i) => [i + 1, JSON.stringify(e)]).filter(([, l]) => !needle || l.toLowerCase().includes(needle));
  return `<ol class="sl-raw">${lines.map(([n, l]) => `<li id="${id}-L${n}"><a class="sl-ln" href="#${id}-L${n}">${n}</a><code>${esc(l)}</code></li>`).join("")}</ol>`;
}

// Renders a Session log into el, with a bar above it (search, structured or raw, download), and keeps it current
// while the session is live. The scroller follows new lines only while the reader is at the bottom; otherwise a
// "N new entries ↓" pill offers to jump.
const mounted = new Set();
export function mountLog(el, id, scroller = el) {
  const m = { el, id, scroller, q: "", raw: false, unseen: 0, count: foldLog(sessions[id]).length };
  const s = sessions[id];
  if (s.events) {
    const href = `data:application/x-ndjson;charset=utf-8,${encodeURIComponent(s.events.map((e) => JSON.stringify(e)).join("\n"))}`;
    el.insertAdjacentHTML("beforebegin", `<div class="sl-bar" role="search">
      <input type="search" placeholder="Search this log" aria-label="Search this Session log">
      <span class="sl-modes" role="group" aria-label="View"><button type="button" data-mode="structured" aria-pressed="true">Structured</button><button type="button" data-mode="raw" aria-pressed="false">Raw JSONL</button></span>
      <a class="sl-dl" download="${esc((s.file ?? `${id}.jsonl`).split("/").pop())}" href="${href}">Download</a></div>`);
    const bar = el.previousElementSibling;
    bar.querySelector("input").addEventListener("input", (e) => { m.q = e.target.value; paint(m); });
    bar.addEventListener("click", (e) => {
      const b = e.target.closest("[data-mode]");
      if (!b) return;
      m.raw = b.dataset.mode === "raw";
      bar.querySelectorAll("[data-mode]").forEach((x) => x.setAttribute("aria-pressed", x === b));
      paint(m);
    });
  }
  el.addEventListener("toggle", (e) => { const k = e.target.dataset?.key; if (k) toggled.set(k, e.target.open); }, true);
  el.addEventListener("click", (e) => {
    if (!e.target.closest(".sl-new")) return;
    m.unseen = 0; paint(m); scroller.scrollTop = scroller.scrollHeight;
  });
  scroller.addEventListener("scroll", () => {
    if (m.unseen && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 60) { m.unseen = 0; paint(m); }
  });
  paint(m);
  mounted.add(m);
  requestAnimationFrame(() => { scroller.scrollTop = scroller.scrollHeight; });
}
function paint(m) {
  m.el.innerHTML = (m.raw ? rawHTML(m.id, m.q) : logHTML(m.id, m.q))
    + (m.unseen ? `<button type="button" class="sl-new">${m.unseen} new entr${m.unseen === 1 ? "y" : "ies"} ↓</button>` : "");
}

// The live session appends an event every few seconds; once its script runs out it keeps running checks.
let extra = 100;
const loop = [
  (id) => [{ type: "assistant", message: { content: [{ type: "tool_use", id, name: "Bash", input: { command: "cargo clippy --all-targets -- -D warnings" } }] } },
           { type: "user", message: { content: [{ type: "tool_result", tool_use_id: id, content: "Finished `dev` profile in 21.4s" }] } }],
  (id) => [{ type: "assistant", message: { content: [{ type: "tool_use", id, name: "Read", input: { file_path: "src/sync.rs", offset: 55, limit: 20 } }] } },
           { type: "user", message: { content: [{ type: "tool_result", tool_use_id: id, content: "fn on_ack(&mut self, id: EntryId) {\n    self.journal.ack_through(id);\n}" }] } }],
  (id) => [{ type: "assistant", message: { content: [{ type: "text", text: "Checking the change once more before Delivery." }] } }],
];
setInterval(() => {
  const s = sessions.s239;
  if (!s.live) return;
  if (s.tail.length) s.events.push(s.tail.shift());
  else { const id = `x${extra++}`; s.events.push(...loop[extra % loop.length](id)); }
  s.lastEventAt = Date.now();
  // The Run's one-line latest event follows the newest tool call.
  const lastCall = foldLog(s).filter((it) => it.k === "tools").at(-1)?.calls.at(-1);
  if (lastCall) {
    const t = runsOf(commandsById.c1).find((r) => r.id === "t239");
    t.latest = latestText(lastCall);
    for (const el of document.querySelectorAll('[data-latest="t239"]')) el.textContent = t.latest;
  }
  for (const m of [...mounted]) {
    if (!m.el.isConnected) { mounted.delete(m); continue; }
    if (m.id !== "s239") continue;
    const atBottom = m.scroller.scrollHeight - m.scroller.scrollTop - m.scroller.clientHeight < 60;
    const count = foldLog(s).length;
    if (!atBottom) m.unseen += count - m.count;
    m.count = count;
    paint(m);
    if (atBottom) m.scroller.scrollTop = m.scroller.scrollHeight;
  }
}, 4000);
sessions.s239.lastEventAt = NOW0 - 4000;
const commandsById = Object.fromEntries(commands.map((c) => [c.id, c]));
export const command = (id) => commandsById[id];

// ---------- Command log (plain text) ----------
export function commandLogText(c) {
  const t0 = c.started;
  const line = (min, msg) => `thirdshift: ${new Date(t0 + min * 60_000).toLocaleTimeString([], { hour12: false })} ${msg}`;
  const out = [line(0, `${c.kind} starting`), line(0, `logging this command to ~/.thirdshift/logs/${c.commandLog}`), line(0.1, `sessions run on ${rhm(c)}`)];
  if (c.note) out.push(line(0.1, c.note));
  for (const r of runsOf(c).slice(1)) {
    if (r.state === "waiting") { out.push(line(3, `${r.issue}: waiting on ${r.blockedBy.join(", ")}`)); continue; }
    if (!r.started) continue;
    const m = (r.started - t0) / 60_000;
    out.push(line(m, `${r.issue}: creating worktree on issue-${r.issue.split("#")[1]}`), line(m + 0.1, `${r.issue}: implement: session started`));
    if (r.state === "merged") out.push(line((r.ended - t0) / 60_000, `${r.issue}: merged into ${runsOf(c)[1]?.specBranch ?? "the Base branch"}`));
    if (r.state === "running") out.push(line((r.stageSince - t0) / 60_000, `${r.issue}: implement: reviewing the branch`), line((Date.now() - t0) / 60_000, `${r.issue}: implement: $ ${r.latest.replace(/^Bash: /, "")}`));
  }
  if (c.stage === "CI watch" && isLive(c)) out.push(line((c.stageSince - t0) / 60_000, `waiting up to 60s for CI on 4f1c2a9`), line((Date.now() - t0) / 60_000, c.latest));
  if (!c.outcome && !c.alive) out.push(line((c.diedAt - t0) / 60_000, `implement: ${c.latest}`), "(the log ends here: the process is gone)");
  if (c.outcome) out.push(line((c.ended - t0) / 60_000, `${c.outcome}${c.cause ? `: ${c.cause}` : ""}`));
  return out.join("\n");
}
