// PROTOTYPE variant "trace": one Command at a time, in depth, as a span waterfall on a shared time axis (Jaeger,
// Grafana Tempo, Langfuse; docs/research/console-run-monitoring-ui.md §4 and §8.1). Rows are spans: the Command,
// its Runs, their sessions and phases, and a session's tool-call groups. The critical path is darker, waiting is
// hatched, silence is dotted, and the current stage carries a faint "usually" band. A minimap pans and zooms.
// On a phone the bars go and the spans read as an indented list with durations, as Buildkite drops its waterfall.
import { host, sessions, pastRuns } from "../data.js";
import {
  state, esc, rhm, age, dur, clock, issueLink, status, isLive, liveCommands, attentionCommands, endedCommands,
  runsOf, ticketLine, stopCommand, stopHint, requeueHint, signIn, signOut, mountLog, commandLogText, command,
  changed, canStop, stopEffectsHTML, health, heartbeat, usual, usualText, needsYou, usageText, sessionsOf,
  summaryText, lastEventAt, foldLog, callLine,
} from "../shared.js";

let pick = "c1"; // the Command the waterfall shows
let sel = "t239"; // the selected span
let panelMode = "span"; // span | cmdlog
let confirming = null;
const views = new Map(); // Command id → { va, vb } in minutes from the Command's start, kept across re-renders

const ORDER = ["implement", "review", "security-review", "Delivery", "CI watch", "Repair", "Resume", "Self-merge", "cleanup"];
const COVERS = { implement: ["implement", "review"], Repair: ["Repair"], Resume: ["Resume"], "security-review": ["security-review"] };
const WAITS = new Set(["CI watch"]); // phases spent waiting on something else, drawn hatched
const MIN = 60_000;

// ---------- Spans ----------
// Stage segments for a Run. Ended Runs with a past record use its stage minutes, scaled to the Run's span; live and
// other Runs get their stages in order, weighted by the usual time here. Synthesized: the prototype keeps no
// per-stage timestamps.
function fill(entries, a, b) {
  const total = entries.reduce((n, [, w]) => n + w, 0) || 1;
  let t = a;
  return entries.map(([stage, w]) => { const s = { stage, a: t, b: t + ((b - a) * w) / total }; t = s.b; return s; });
}
const mid = (stage) => { const u = usual(stage); return u ? (u.lo + u.hi) / 2 : 6; };
function stagesOf(r, c, now) {
  const a0 = r.started ?? c.started;
  if (!a0) return [];
  if (r.tickets) return [{ stage: "Tickets", a: a0, b: r.ended ?? (c.ended ?? now), cur: !r.ended && isLive(c) }];
  const past = pastRuns.find((p) => p.issue === r.issue);
  if (past && r.ended) return fill(Object.entries(past.mins), a0, r.ended);
  if (r === c && !c.outcome && !c.alive) return [{ stage: c.stage, a: a0, b: c.diedAt, died: true }];
  if (r.stage && r.stageSince && (r.state === "running" || (r === c && isLive(c)))) {
    const before = ORDER.slice(0, ORDER.indexOf(r.stage)).filter((s) => s !== "Repair" && s !== "Resume"
      && (s !== "security-review" || r.sessions?.some((id) => sessions[id]?.kind === "security-review")));
    return [...fill(before.map((s) => [s, mid(s)]), a0, r.stageSince), { stage: r.stage, a: r.stageSince, b: now, cur: true }];
  }
  if (r.ended) return fill(["implement", "review", "Delivery", "CI watch"].map((s) => [s, mid(s)]), a0, r.ended);
  return [];
}

function stOf(r, c) {
  if (r.state) return r.state; // a Ticket: merged, running, waiting, failed, not started, interrupted
  if (r === c) { const s = status(c); return s.key === "live" ? "running" : s.key; }
  if (r.outcome) return r.outcome;
  return c.stopping ? "stopping" : isLive(c) ? "running" : c.outcome ?? "died";
}
const WORDS = { running: "running", merged: "merged", ready: "ready", failed: "failed", interrupted: "interrupted",
  skipped: "skipped", died: "died", waiting: "waiting", "not started": "not started", stopping: "stopping…", done: "done" };

// Builds every row of one Command's waterfall. Folds stay two levels deep: the Command row opens its Runs (or, for
// a Run Command, its sessions and phases); a Run row opens its sessions, phases and tool-call groups.
function build(c, now) {
  const rows = [];
  const end = (r) => r.ended ?? (r === c && !c.alive && !c.outcome ? c.diedAt : r.state === "running" || isLive(c) ? now : c.ended ?? now);

  const runRow = (r, depth, foldIn, toggle) => {
    const st = stOf(r, c);
    const segs = st === "waiting" || st === "not started" ? [] : stagesOf(r, c, now);
    const a = segs[0]?.a ?? r.started ?? c.started, b = segs.at(-1)?.b ?? end(r);
    const row = { id: r.id, kind: r === c ? "cmd" : "run", depth, foldIn, toggle, run: r, st, segs, a, b, marks: [] };
    if (st === "not started") { row.a = row.b = c.started; }
    if (st === "waiting") {
      const from = c.children?.find((x) => x.tickets?.includes(r))?.started ?? c.started;
      row.a = from; row.b = now;
      row.marks.push({ cls: "hatch", a: from, b: now, title: `waiting on ${r.blockedBy.join(", ")}` });
    }
    const cur = segs.find((s) => s.cur);
    if (cur) { const u = usual(cur.stage); if (u) row.marks.push({ cls: "ghost", a: cur.a + u.lo * MIN, b: cur.a + u.hi * MIN, title: `usually ${u.lo}–${u.hi} min` }); }
    const last = lastEventAt(r);
    if (last && (st === "running") && now - last > 60_000) row.marks.push({ cls: "silence", a: last, b: now, title: `no output for ${dur(now - last)}` });
    if (st === "died") row.marks.push({ cls: "silence gone", a: c.diedAt, b: now, title: `process gone ${dur(now - c.diedAt)} ago` }, { cls: "cross", a: c.diedAt, b: c.diedAt, title: "died" });
    rows.push(row);
    return row;
  };

  // A Run's sessions and phases, in time order, with each session's tool-call groups under it.
  const runKids = (r, segs, depth, foldIn) => {
    const kids = [];
    const used = new Set();
    for (const sid of r.sessions ?? []) {
      const s = sessions[sid];
      const mine = segs.filter((g) => (COVERS[s.kind] ?? [s.kind]).includes(g.stage));
      mine.forEach((g) => used.add(g));
      let a, b;
      if (mine.length) { a = Math.min(...mine.map((g) => g.a)); b = Math.max(...mine.map((g) => g.b)); }
      else { b = s.live ? now : r.ended ?? end(r); a = b - (s.minutes ?? 5) * MIN; }
      kids.push({ id: `s:${sid}`, kind: "session", sid, a, b, st: s.live ? "running" : "done", name: `${s.kind} session` });
    }
    for (const g of segs) if (!used.has(g) && g.stage !== "Tickets")
      kids.push({ id: `p:${r.id}:${g.stage}`, kind: WAITS.has(g.stage) ? "wait" : "phase", a: g.a, b: g.b, st: g.cur ? "running" : g.died ? "died" : "done", name: g.stage, stage: g.stage, cur: g.cur, run: r });
    kids.sort((x, y) => x.a - y.a);
    for (const k of kids) {
      rows.push({ ...k, depth, foldIn, run: k.run ?? r, marks: k.kind === "wait" ? [{ cls: "hatch", a: k.a, b: k.b, title: `waiting on CI` }] : [] });
      if (k.kind === "session") groupRows(k, r, depth + 1, foldIn);
    }
  };

  // Synthesized: the prototype's Session logs carry no per-event timestamps, so each item takes a slice of the
  // session's span in proportion to its weight (a tool call counts for more than a line of text).
  const groupRows = (k, r, depth, foldIn) => {
    const s = sessions[k.sid];
    if (!s.events) return;
    const items = foldLog(s);
    const w = items.map((it) => (it.k === "tools" ? 1 + it.calls.length * 2 : 1));
    const total = w.reduce((n, x) => n + x, 0);
    let t = k.a;
    items.forEach((it, n) => {
      const span = ((k.b - k.a) * w[n]) / total;
      if (it.k === "tools") {
        const names = [...it.calls.reduce((m, x) => m.set(x.name, (m.get(x.name) ?? 0) + 1), new Map())].map(([x, v]) => (v > 1 ? `${x} ×${v}` : x)).join(" · ");
        const st = it.calls.some((x) => x.running) ? "running" : it.calls.some((x) => x.isError) ? "failed" : "done";
        rows.push({ id: `g:${k.sid}:${n}`, kind: "tools", sid: k.sid, group: it, depth, foldIn, run: r, a: t, b: t + span, st,
          name: `${it.calls.length} tool call${it.calls.length === 1 ? "" : "s"}: ${names}`, marks: [] });
      }
      t += span;
    });
  };

  const key = (id) => `tr-${id}`;
  if (c.kind === "Run" || c.kind === "Spec run") {
    const top = runRow(c, 0, [], key(c.id));
    if (c.tickets) {
      for (const t of c.tickets) { const tr = runRow(t, 1, [key(c.id)], t.sessions ? key(t.id) : null); runKids(t, tr.segs, 2, [key(c.id), key(t.id)]); }
    } else runKids(c, top.segs, 1, [key(c.id)]);
    for (const ch of c.children ?? []) { const cr = runRow(ch, 1, [key(c.id)], key(ch.id)); runKids(ch, cr.segs, 2, [key(c.id), key(ch.id)]); }
  } else {
    // A Pass: the Command row is a plain bar; its own sessions run in order, then what it dispatched.
    const st = stOf(c, c);
    const top = { id: c.id, kind: "cmd", depth: 0, foldIn: [], toggle: st === "skipped" ? null : key(c.id), run: c, st, segs: [],
      a: c.started, b: st === "skipped" ? c.started : (c.ended ?? now), marks: [] };
    rows.push(top);
    let t = c.started;
    for (const sid of c.sessions ?? []) {
      const s = sessions[sid];
      const k = { id: `s:${sid}`, kind: "session", sid, a: t, b: t + (s.minutes ?? 5) * MIN, st: "done", name: `${s.kind} session` };
      t = k.b;
      rows.push({ ...k, depth: 1, foldIn: [key(c.id)], run: c, marks: [] });
    }
    for (const ch of c.children ?? []) {
      if (ch.tickets) {
        // A dispatched Spec run is a heading row; its Tickets show with the Command, so a Ticket's sessions are
        // still only two unfolds away.
        runRow(ch, 1, [key(c.id)], null);
        for (const tk of ch.tickets) { const tr = runRow(tk, 2, [key(c.id)], tk.sessions ? key(tk.id) : null); runKids(tk, tr.segs, 3, [key(c.id), key(tk.id)]); }
      } else { const cr = runRow(ch, 1, [key(c.id)], key(ch.id)); runKids(ch, cr.segs, 2, [key(c.id), key(ch.id)]); }
    }
  }
  return rows;
}

// The critical path: what decides when the Command can finish. In a Spec run, the chain of blocking Tickets (the
// one waiting, back through what blocks it) and then the Spec review; for a single Run, the Run itself.
function critical(c) {
  const spec = c.tickets ? c : c.children?.find((x) => x.tickets);
  if (spec) {
    const ts = spec.tickets;
    let last = ts.find((t) => t.state === "waiting")
      ?? ts.filter((t) => t.state === "running" || t.ended).sort((x, y) => (y.ended ?? Infinity) - (x.ended ?? Infinity))[0];
    const chain = [];
    while (last && !chain.includes(last)) { chain.unshift(last); last = last.blockedBy ? ts.find((t) => t.issue === last.blockedBy[0]) : null; }
    const ids = new Set([c.id, spec.id, ...chain.map((t) => t.id)]);
    const word = (t) => (t.state === "running" ? `${t.stage}, running` : t.state);
    return { ids, label: `${chain.map((t) => `${t.issue} (${word(t)})`).join(" → ")}${!spec.ended && isLive(c) ? " → spec-review" : ""}` };
  }
  const ids = new Set(runsOf(c).map((r) => r.id));
  return { ids, label: c.kind === "Run" ? `one Run, one stage after another${c.children?.length ? `, with its ${c.children[0].kind} in the middle` : ""}` : "each session in turn, then what it dispatched" };
}

// ---------- Markup ----------
const pctVars = (a, b, t0) => `--a:${((a - t0) / MIN).toFixed(3)};--b:${((b - t0) / MIN).toFixed(3)}`;

function rowWords(row, now) {
  const r = row.run;
  if (row.kind === "cmd" || row.kind === "run") {
    if (row.st === "running") {
      const h = r.stage && r.stage !== "Tickets" ? health(r) : { key: "" };
      return `${r.stage ? `${esc(r.stage)} · ${age(r.stageSince ?? r.started)}` : "running"}${h.key && h.key !== "ok" ? ` · <b class="h-${h.key}">${esc(h.label)}</b>` : ""}`;
    }
    if (row.st === "died") return `died at ${esc(r.stage)} · <b class="h-silent">no output for ${dur(now - r.diedAt)}</b>`;
    if (row.st === "waiting") return `cannot start yet: waiting on ${r.blockedBy.map(esc).join(", ")}`;
    if (row.st === "skipped") return `skipped: ${esc(r.cause)}`;
    return esc(WORDS[row.st] ?? row.st);
  }
  if (row.st === "running") return "running";
  return esc(WORDS[row.st] ?? row.st);
}

function rowHTML(row, t0, now, crit) {
  const isCrit = crit.ids.has(row.run?.id) || crit.ids.has(row.id);
  const durText = row.b > row.a ? `${dur(row.b - row.a)}${row.st === "running" ? " so far" : ""}` : "—";
  const name = row.kind === "cmd"
    ? `<span class="kind">${esc(row.run.kind)}</span> ${row.run.issue ? issueLink(row.run.issue) : `<b>${esc(row.run.repo)}</b>`}`
    : row.kind === "run"
      ? `<span class="kind">${esc(row.run.kind ?? "Ticket")}</span> ${issueLink(row.run.issue)}`
      : `<span class="kind">${esc(row.kind === "tools" ? "tools" : row.kind === "session" ? "session" : row.kind === "wait" ? "waiting" : "phase")}</span> ${esc(row.name)}`;
  const failed = row.st === "failed" || row.st === "died";
  const toggle = row.toggle
    ? `<span class="tw" data-fold="${row.toggle}"${failed || row.kind === "cmd" ? ' data-fold-default="open"' : ""} aria-label="Show what this span holds"></span>`
    : `<span class="tw-none"></span>`;
  const bars = [];
  if (row.segs?.length) {
    for (const s of row.segs) bars.push(`<i class="bar seg${s.cur ? " cur live" : ""}${s.died ? " died" : ""}" style="${pctVars(s.a, s.b, t0)}" title="${esc(s.stage)} · ${dur(s.b - s.a)}"><span>${esc(s.stage)}</span></i>`);
  } else if (row.st !== "waiting" && row.st !== "not started") {
    const live = row.st === "running";
    bars.push(`<i class="bar${live ? " live" : ""}${row.kind === "tools" ? " thin" : ""}${row.st === "failed" ? " failed" : ""}${row.st === "skipped" ? " tick" : ""}" style="${pctVars(row.a, Math.max(row.b, row.a), t0)}" title="${esc(row.name ?? row.run?.kind ?? "")} · ${durText}"></i>`);
  }
  for (const m of row.marks) bars.push(`<i class="${m.cls}" style="${pctVars(m.a, m.b, t0)}" title="${esc(m.title)}">${m.cls === "cross" ? "✕" : ""}</i>`);
  const latest = (row.kind === "run" || row.kind === "cmd") && row.run.latest && (row.st === "running" || row.st === "died")
    ? `<code class="latest" data-latest="${row.run.id}">${esc(row.run.latest)}</code>` : "";
  return `<li class="row k-${row.kind} st-${row.st.replace(" ", "-")}${isCrit ? " crit" : ""}${sel === row.id ? " sel" : ""}" data-span="${esc(row.id)}" style="--d:${row.depth}"${row.foldIn.length ? ` data-fold-in="${row.foldIn.join(" ")}"` : ""}>
    <div class="nm">${toggle}<span class="nm-text">${name}</span>${row.kind === "run" || row.kind === "cmd" ? `<span class="nm-sub">${esc(row.run.title ?? row.run.note ?? "")}</span>` : ""}${latest}</div>
    <div class="du"><span class="du-t">${durText}</span><span class="du-w">${rowWords(row, now)}</span></div>
    <div class="trk">${bars.join("")}</div>
  </li>`;
}

function stepFor(span) { return [1, 2, 5, 10, 15, 30, 60, 120, 240, 480].find((s) => span / s <= 7) ?? 960; }
function axisHTML(va, vb, t0) {
  const step = stepFor(vb - va), out = [];
  for (let m = Math.ceil(va / step) * step; m <= vb; m += step) {
    out.push(`<span class="tick" style="--t:${m}">${new Date(t0 + m * MIN).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}</span>`);
  }
  return out.join("");
}

function needsStrip() {
  const items = needsYou();
  if (!items.length) return `<p class="needs none">Nothing needs you on ${esc(host.name)}.</p>`;
  return `<div class="needs" role="region" aria-label="Needs you"><span class="needs-h">Needs you · ${items.length}</span><ul>${items.map((it) =>
    `<li><button type="button" class="nd nd-${it.level}" data-pick="${it.cmd.id}" data-sel="${esc(it.run.id)}"><b>${it.level === "urgent" ? "Urgent" : "Look"}</b> ${esc(it.ref ?? "")} · ${esc(it.what)}</button></li>`).join("")}</ul></div>`;
}

function picker() {
  const chip = (c) => {
    const s = status(c);
    return `<li><button type="button" class="chip s-${s.key}" data-pick="${c.id}"${pick === c.id ? ' aria-current="true"' : ""}><span class="chip-k">${esc(c.kind)}</span> ${esc(c.issue ?? c.repo)} <span class="chip-s">${esc(s.label)}</span></button></li>`;
  };
  const groups = [["Live", liveCommands()], ["Died", attentionCommands()], ["Ended", endedCommands().sort((a, b) => b.ended - a.ended)]];
  return `<nav class="picker" aria-label="Choose a Command">${groups.filter(([, cs]) => cs.length).map(([h, cs]) => `<span class="pk-h">${h}</span><ul>${cs.map(chip).join("")}</ul>`).join("")}</nav>`;
}

function headHTML(c, crit) {
  const s = status(c);
  const total = usageText(sessionsOf(c));
  const confirm = confirming === c.id && canStop(c) ? `<div class="confirm" role="alertdialog" aria-label="Confirm Stop">
      <p><b>Stop this ${esc(c.kind)}?</b> ${esc(c.stopTarget)} will be labelled <code>needs-triage</code>, not put back in line, so the Day shift sees it first.</p>
      ${stopEffectsHTML(c)}
      <div class="confirm-actions"><button type="button" class="danger" data-do-stop="${c.id}">Stop · label needs-triage</button>
        <button type="button" data-do-requeue="${c.id}">Requeue instead</button><button type="button" data-cancel>Cancel</button></div>
      <p class="hint"><code>$ ${esc(stopHint(c))}</code> · <code>$ ${esc(requeueHint(c))}</code></p></div>` : "";
  return `<div class="wf-head">
    <div class="wf-title"><span class="kind">${esc(c.kind)} · ${esc(c.repo)}</span>
      <span class="wf-name">${c.issue ? `${issueLink(c.issue)} ${esc(c.title)}` : esc(c.note ?? c.cause ?? c.repo)}</span>
      <span class="wf-meta"><span class="pill s-${s.key}">${esc(s.label)}</span> started ${clock(c.started)}${c.ended ? ` · ended ${clock(c.ended)} after ${dur(c.ended - c.started)}` : ""} · ${esc(rhm(c))}${total ? ` · ${esc(total)}` : ""}</span></div>
    <div class="wf-acts">${c.commandLog ? `<button type="button" class="ghostbtn" data-cmdlog="${c.id}">Command log</button>` : ""}${canStop(c) ? `<button type="button" class="danger" data-stop="${c.id}">Stop</button>` : ""}</div>
    ${confirm}
    <p class="crit-line"><span class="crit-key" aria-hidden="true"></span><b>Critical path</b> ${esc(crit.label)}</p>
  </div>`;
}

// The details of the selected span: facts, logs, or the calls in a tool-call group.
function panelHTML(c, rows, now) {
  if (panelMode === "cmdlog") return `<h3>Command log</h3><p class="pn-sub">${esc(c.commandLog)} · plain text</p><pre class="cmdlog">${esc(commandLogText(c))}</pre>`;
  const row = rows.find((r) => r.id === sel) ?? rows[0];
  if (!row) return "";
  const r = row.run;
  const facts = [];
  const add = (k, v) => v && facts.push(`<dt>${k}</dt><dd>${v}</dd>`);
  if (row.kind === "cmd" || row.kind === "run") {
    add("Issue", r.issue ? `${issueLink(r.issue)} ${esc(r.title ?? "")}` : esc(r.repo ?? c.repo));
    add("Harness · Model · Effort", r.harness ? esc(rhm(r)) : "");
    add("Now", rowWords(row, now));
    if (row.st === "running" && r.stage && r.stage !== "Tickets") { add("Heartbeat", heartbeat(r)); add("Typical here", esc(usualText(r.stage))); add("Health", esc(health(r).label)); }
    add("Latest", r.latest && (row.st === "running" || row.st === "died") ? `<code data-latest="${r.id}">${esc(r.latest)}</code>` : "");
    add("Spent", esc(usageText(row.kind === "cmd" ? sessionsOf(c) : r.sessions ?? [])));
    add("What came of it", esc(summaryText(r)));
    add("Pull request", r.pr ? issueLink(r.pr) : "");
    add("Cause", r.cause && row.st !== "skipped" ? esc(r.cause) : r.state && !["running", "merged"].includes(r.state) ? esc(ticketLine(r)) : "");
    add("Spec branch", r.specBranch ? `<code>${esc(r.specBranch)}</code>` : "");
    const links = (r.sessions ?? []).map((sid) => `<button type="button" class="ghostbtn" data-sel="s:${sid}">${esc(sessions[sid].kind)} session${sessions[sid].live ? " ●" : ""}</button>`).join("");
    const findings = r.findings ? `<h4>Security findings</h4><ul class="findings">${r.findings.map((f) => `<li><span class="sev sev-${f.sev}">${esc(f.sev)}</span> ${esc(f.title)} <em>${f.reproduced ? `reproduced · fixed by ${issueLink(f.fix)} (${esc(f.fixOutcome)})` : "not reproduced · waits for the Day shift"}</em></li>`).join("")}</ul>` : "";
    return `<h3>${esc(r.kind ?? "Ticket")} ${r.issue ? esc(r.issue) : ""}</h3><dl class="facts">${facts.join("")}</dl>${links ? `<div class="pn-links">${links}</div>` : ""}${findings}`;
  }
  if (row.kind === "session") {
    const s = sessions[row.sid];
    add("Owner", esc(s.owner));
    add("Harness", esc(s.harness));
    add("Length", `${dur(row.b - row.a)}${s.live ? " so far" : ""}`);
    add("Spent", esc(usageText([row.sid])));
    add("Heartbeat", s.live ? heartbeat(r) : "");
    add("File", s.file ? `<code>${esc(s.file)}</code>` : "");
    return `<h3>${esc(s.kind)} session${s.live ? ' <span class="pill s-live">live</span>' : ""}</h3><dl class="facts">${facts.join("")}</dl><div class="pn-log"><div class="pn-log-body"></div></div>`;
  }
  if (row.kind === "tools") {
    const calls = row.group.calls.map((x) => {
      const l = callLine(x);
      const body = x.name === "Edit"
        ? `<pre class="sl-diff">${(x.input.old_string ? x.input.old_string.split("\n").map((t) => `<span class="sl-del">- ${esc(t)}</span>`) : []).concat(x.input.new_string.split("\n").map((t) => `<span class="sl-add">+ ${esc(t)}</span>`)).join("\n")}</pre>`
        : x.result != null ? `<pre class="sl-out">${esc(x.result)}</pre>` : `<pre class="sl-out">…</pre>`;
      const st = x.running ? `<span class="sl-st sl-run">running${x.elapsed ? ` · ${x.elapsed} s` : ""}</span>` : x.isError ? `<span class="sl-st sl-err">failed</span>` : `<span class="sl-st sl-ok">ok</span>`;
      return `<li class="sl-call${x.isError ? " sl-failed" : ""}"><details${x.isError || x.name === "Edit" ? " open" : ""}><summary><span class="sl-tool">${esc(l.tool)}</span> <code class="sl-arg">${esc(l.arg)}</code> ${st}</summary>${body}</details></li>`;
    }).join("");
    return `<h3>${esc(row.name)}</h3><p class="pn-sub">In the ${esc(sessions[row.sid].kind)} session · about ${dur(row.b - row.a)} (spread evenly: the prototype keeps no per-call times)</p><ol class="sl-calls">${calls}</ol>`;
  }
  add("Stage", esc(row.name));
  add("Length", `${dur(row.b - row.a)}${row.cur ? " so far" : ""}`);
  add("Typical here", esc(usualText(row.stage)));
  add("Waiting", row.kind === "wait" ? "thirdshift is waiting on CI, not working: no session runs here" : "");
  if (row.cur) { add("Heartbeat", heartbeat(r)); add("Health", esc(health(r).label)); }
  return `<h3>${esc(row.name)}</h3><dl class="facts">${facts.join("")}</dl>`;
}

function signInPage(root) {
  root.innerHTML = `<div class="tr signin"><form><h1>${esc(host.name)} <span class="muted">Console</span></h1>
    <label>Password <input type="password" autocomplete="current-password" autofocus></label><button type="submit">Sign in</button></form></div>`;
  root.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); signIn(); });
}

export function render(root) {
  if (!state.signedIn) return signInPage(root);
  const c = command(pick) ?? command("c1");
  const now = Date.now();
  const t0 = c.started;
  const rows = build(c, now);
  if (!rows.some((r) => r.id === sel)) sel = rows.find((r) => r.st === "running" && r.kind === "run")?.id ?? rows[0].id;
  const crit = critical(c);
  const end = Math.max(...rows.map((r) => r.b), t0 + MIN);
  const full = { va: -((end - t0) / MIN) * 0.02, vb: ((end - t0) / MIN) * 1.04 };
  const view = views.get(c.id) ?? full;
  const mini = rows.filter((r) => r.depth <= 1 && r.b > r.a).map((r) =>
    `<i class="mb st-${r.st.replace(" ", "-")}" style="--a:${((r.a - t0) / MIN).toFixed(3)};--b:${((r.b - t0) / MIN).toFixed(3)};--r:${r.depth}"></i>`).join("");

  root.innerHTML = `<div class="tr">
    <header class="top"><div class="host"><span class="dot"></span><b>${esc(host.name)}</b> <span class="muted">Console · trace</span></div>
      <nav><a href="${host.other.url}">${esc(host.other.name)} →</a> <button type="button" class="link" data-signout>Sign out</button></nav></header>
    ${needsStrip()}
    ${picker()}
    <section class="wf-sec"><h2>Waterfall <span class="muted">· ${esc(c.kind)} ${esc(c.issue ?? c.repo)}</span></h2>
      ${headHTML(c, crit)}
      <div class="wf-layout">
        <div class="wf" style="--va:${view.va};--vb:${view.vb};--fa:${full.va};--fb:${full.vb}">
          <div class="wf-tools"><span class="legend"><i class="lg lg-crit"></i>critical path <i class="lg lg-hatch"></i>waiting <i class="lg lg-sil"></i>silent <i class="lg lg-ghost"></i>usual time <i class="lg lg-live"></i>running</span>
            <span class="zoom"><button type="button" data-zoom="out" aria-label="Zoom out">−</button><button type="button" data-zoom="in" aria-label="Zoom in">+</button><button type="button" data-zoom="fit">Fit</button></span></div>
          <div class="mini" aria-label="Minimap: drag the window to pan">${mini}<span class="win"></span></div>
          <ol class="rows">
            <li class="row axis-row" aria-hidden="true"><div class="nm">Span</div><div class="du">Time</div><div class="trk axis">${axisHTML(view.va, view.vb, t0)}<span class="now-l" style="--t:${((now - t0) / MIN).toFixed(3)}"></span></div></li>
            ${rows.map((r) => rowHTML(r, t0, now, crit)).join("")}
          </ol>
        </div>
        <aside class="panel" aria-label="Selected span">${panelHTML(c, rows, now)}</aside>
      </div>
    </section>
  </div>`;

  const logEl = root.querySelector(".pn-log-body");
  if (logEl) { const row = rows.find((r) => r.id === sel); mountLog(logEl, row.sid, logEl); }

  // Pan and zoom by setting the view variables; the bars are positioned in CSS from them, so nothing re-renders.
  const wf = root.querySelector(".wf"), win = root.querySelector(".win"), mini2 = root.querySelector(".mini"), axis = root.querySelector(".axis");
  const setView = (va, vb) => {
    const w = Math.max(2, Math.min(full.vb - full.va, vb - va));
    va = Math.max(full.va, Math.min(full.vb - w, va));
    const v = { va, vb: va + w };
    views.set(c.id, v);
    wf.style.setProperty("--va", v.va); wf.style.setProperty("--vb", v.vb);
    win.style.left = `${((v.va - full.va) / (full.vb - full.va)) * 100}%`;
    win.style.width = `${((v.vb - v.va) / (full.vb - full.va)) * 100}%`;
    axis.querySelectorAll(".tick").forEach((x) => x.remove());
    axis.insertAdjacentHTML("afterbegin", axisHTML(v.va, v.vb, t0));
  };
  setView(view.va, view.vb);
  let drag = null;
  mini2.addEventListener("pointerdown", (e) => {
    const box = mini2.getBoundingClientRect(), v = views.get(c.id);
    const at = full.va + ((e.clientX - box.left) / box.width) * (full.vb - full.va);
    if (e.target !== win) setView(at - (v.vb - v.va) / 2, at + (v.vb - v.va) / 2);
    drag = { x: e.clientX, v: views.get(c.id), w: box.width };
    mini2.setPointerCapture(e.pointerId);
  });
  mini2.addEventListener("pointermove", (e) => {
    if (!drag) return;
    const d = ((e.clientX - drag.x) / drag.w) * (full.vb - full.va);
    setView(drag.v.va + d, drag.v.vb + d);
  });
  mini2.addEventListener("pointerup", () => { drag = null; });

  root.addEventListener("click", (e) => {
    const t = e.target.closest("button, a, [data-span]");
    if (!t) return;
    const d = t.dataset;
    if (d.zoom) {
      const v = views.get(c.id), m = (v.va + v.vb) / 2, w = v.vb - v.va;
      if (d.zoom === "fit") setView(full.va, full.vb);
      else { const nw = d.zoom === "in" ? w / 1.6 : w * 1.6; setView(m - nw / 2, m + nw / 2); }
      return;
    }
    if (d.pick) { e.preventDefault(); pick = d.pick; sel = d.sel ?? null; panelMode = "span"; confirming = null; changed(); openWaterfall(); return; }
    if (d.sel) { sel = d.sel; panelMode = "span"; changed(); return; }
    if (d.stop) { confirming = d.stop; changed(); return; }
    if (d.doStop) { confirming = null; stopCommand(d.doStop, false); return; }
    if (d.doRequeue) { confirming = null; stopCommand(d.doRequeue, true); return; }
    if ("cancel" in d) { confirming = null; changed(); return; }
    if (d.cmdlog) { panelMode = "cmdlog"; changed(); return; }
    if ("signout" in d) { signOut(); return; }
    if (d.span && !e.target.closest("a, button, .tw")) { sel = d.span; panelMode = "span"; changed(); }
  });
}

// Choosing something to look at opens the Waterfall section if the reader had it closed.
function openWaterfall() {
  const h2 = document.querySelector(".wf-sec h2");
  if (h2 && h2.getAttribute("aria-expanded") !== "true") h2.click();
}
