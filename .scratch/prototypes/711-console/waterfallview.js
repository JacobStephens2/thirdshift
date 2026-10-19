// PROTOTYPE (#711): the waterfall view, shared by the inbox variants. The "trace" variant's span waterfall for ONE
// Command (Jaeger, Grafana Tempo, Langfuse; docs/research/console-run-monitoring-ui.md §4 and §8.1): rows are spans,
// the Command, its Runs, their sessions and phases, and a session's tool-call groups, on a shared time axis. The
// critical path is darker, waiting is hatched, silence is dotted, and the current stage carries a faint "usually"
// band. A minimap pans and zooms. Classes are all wf-*, so a variant styles them under its own rules and nothing
// leaks into its rows. Span rows carry data-wf-span, data-wf-kind, data-wf-run, data-wf-cmd and, for a session,
// data-wf-sid, so the variant can link them to its own selection. On a phone the bars go and the spans read as an
// indented list with durations, as Buildkite drops its waterfall on mobile.
import { sessions, pastRuns } from "./data.js";
import {
  esc, dur, age, issueLink, status, isLive, runsOf, health, usual, lastEventAt, foldLog, callLine,
} from "./shared.js";

const ORDER = ["implement", "review", "security-review", "Delivery", "CI watch", "Repair", "Resume", "Self-merge", "cleanup"];
const COVERS = { implement: ["implement", "review"], Repair: ["Repair"], Resume: ["Resume"], "security-review": ["security-review"] };
const WAITS = new Set(["CI watch"]); // phases spent waiting on something else, drawn hatched
const MIN = 60_000;
const views = new Map(); // Command id → { va, vb } in minutes from the Command's start, kept across re-renders

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
// a Run Command, its sessions and phases); a Run row opens its sessions, phases and tool-call groups. A tool-call
// group's own calls unfold inline under it.
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
      row.marks.push({ cls: "wf-hatch", a: from, b: now, title: `waiting on ${r.blockedBy.join(", ")}` });
    }
    const cur = segs.find((s) => s.cur);
    if (cur) { const u = usual(cur.stage); if (u) row.marks.push({ cls: "wf-ghost", a: cur.a + u.lo * MIN, b: cur.a + u.hi * MIN, title: `usually ${u.lo}–${u.hi} min` }); }
    const last = lastEventAt(r);
    if (last && st === "running" && now - last > 60_000) row.marks.push({ cls: "wf-silence", a: last, b: now, title: `no output for ${dur(now - last)}` });
    if (st === "died") row.marks.push({ cls: "wf-silence wf-gone", a: c.diedAt, b: now, title: `process gone ${dur(now - c.diedAt)} ago` }, { cls: "wf-cross", a: c.diedAt, b: c.diedAt, title: "died" });
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
      rows.push({ ...k, depth, foldIn, run: k.run ?? r, marks: k.kind === "wait" ? [{ cls: "wf-hatch", a: k.a, b: k.b, title: "waiting on CI" }] : [] });
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

  // Fold keys carry the Command's id, so each Command keeps its own open and closed spans.
  const key = (id) => `wf-${c.id}-${id}`;
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
        // A dispatched Spec run folds its Tickets, closed on load like the Command above it (Jacob's call): open
        // the Pickup run to see the Spec run, the Spec run to see its Tickets, a Ticket to see its sessions.
        runRow(ch, 1, [key(c.id)], key(ch.id));
        for (const tk of ch.tickets) { const tr = runRow(tk, 2, [key(c.id), key(ch.id)], tk.sessions ? key(tk.id) : null); runKids(tk, tr.segs, 3, [key(c.id), key(ch.id), key(tk.id)]); }
      } else { const cr = runRow(ch, 1, [key(c.id)], key(ch.id)); runKids(ch, cr.segs, 2, [key(c.id), key(ch.id)]); }
    }
  }
  return { rows, key };
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
      return `${r.stage ? `${esc(r.stage)} · ${age(r.stageSince ?? r.started)}` : "running"}${h.key && h.key !== "ok" ? ` · <b class="wf-h-${h.key}">${esc(h.label)}</b>` : ""}`;
    }
    if (row.st === "died") return `died at ${esc(r.stage)} · <b class="wf-h-silent">no output for ${dur(now - r.diedAt)}</b>`;
    if (row.st === "waiting") return `cannot start yet: waiting on ${r.blockedBy.map(esc).join(", ")}`;
    if (row.st === "skipped") return `skipped: ${esc(r.cause)}`;
    return esc(WORDS[row.st] ?? row.st);
  }
  return esc(WORDS[row.st] ?? row.st);
}

// A tool-call group's calls, in the shared Session log markup, shown inline when its row unfolds.
function callsHTML(group) {
  return group.calls.map((x) => {
    const l = callLine(x);
    const body = x.name === "Edit"
      ? `<pre class="sl-diff">${(x.input.old_string ? x.input.old_string.split("\n").map((t) => `<span class="sl-del">- ${esc(t)}</span>`) : []).concat(x.input.new_string.split("\n").map((t) => `<span class="sl-add">+ ${esc(t)}</span>`)).join("\n")}</pre>`
      : x.result != null ? `<pre class="sl-out">${esc(x.result)}</pre>` : `<pre class="sl-out">…</pre>`;
    const st = x.running ? `<span class="sl-st sl-run">running${x.elapsed ? ` · ${x.elapsed} s` : ""}</span>` : x.isError ? `<span class="sl-st sl-err">failed</span>` : `<span class="sl-st sl-ok">ok</span>`;
    return `<li class="sl-call${x.isError ? " sl-failed" : ""}"><details${x.isError ? " open" : ""}><summary><span class="sl-tool">${esc(l.tool)}</span> <code class="sl-arg">${esc(l.arg)}</code> ${st}</summary>${body}</details></li>`;
  }).join("");
}

function rowHTML(row, t0, now, crit, key, c, selectedRunId) {
  const isCrit = crit.ids.has(row.run?.id) || crit.ids.has(row.id);
  const durText = row.b > row.a ? `${dur(row.b - row.a)}${row.st === "running" ? " so far" : ""}` : "—";
  const name = row.kind === "cmd"
    ? `<span class="wf-kind">${esc(row.run.kind)}</span> ${row.run.issue ? issueLink(row.run.issue, "wf-ref") : `<b class="wf-ref">${esc(row.run.repo)}</b>`}`
    : row.kind === "run"
      ? `<span class="wf-kind">${esc(row.run.kind ?? "Ticket")}</span> ${issueLink(row.run.issue, "wf-ref")}`
      : `<span class="wf-kind">${esc(row.kind === "tools" ? "tools" : row.kind === "session" ? "session" : row.kind === "wait" ? "waiting" : "phase")}</span> ${esc(row.name)}`;
  const failed = row.st === "failed" || row.st === "died";
  // A tool-call group's row is itself the toggle for its calls; a Command or Run row has a caret. Everything loads
  // folded (the Pickup run and its Spec run included) except a failed or died span, which opens to show where.
  const gkey = row.kind === "tools" ? key(row.id) : null;
  const toggle = row.toggle
    ? `<span class="wf-tw" data-fold="${row.toggle}"${failed ? ' data-fold-default="open"' : ""} aria-label="Show what this span holds"></span>`
    : gkey ? "" : `<span class="wf-tw-none"></span>`;
  const bars = [];
  if (row.segs?.length) {
    for (const s of row.segs) bars.push(`<i class="wf-bar wf-seg${s.cur ? " wf-cur wf-live" : ""}${s.died ? " wf-died" : ""}" style="${pctVars(s.a, s.b, t0)}" title="${esc(s.stage)} · ${dur(s.b - s.a)}"><span>${esc(s.stage)}</span></i>`);
  } else if (row.st !== "waiting" && row.st !== "not started") {
    const live = row.st === "running";
    bars.push(`<i class="wf-bar${live ? " wf-live" : ""}${row.kind === "tools" ? " wf-thin" : ""}${row.st === "failed" ? " wf-failed" : ""}${row.st === "skipped" ? " wf-tick" : ""}" style="${pctVars(row.a, Math.max(row.b, row.a), t0)}" title="${esc(row.name ?? row.run?.kind ?? "")} · ${durText}"></i>`);
  }
  for (const m of row.marks) bars.push(`<i class="${m.cls}" style="${pctVars(m.a, m.b, t0)}" title="${esc(m.title)}">${m.cls === "wf-cross" ? "✕" : ""}</i>`);
  const latest = (row.kind === "run" || row.kind === "cmd") && row.run.latest && (row.st === "running" || row.st === "died")
    ? `<code class="wf-latest" data-latest="${row.run.id}">${esc(row.run.latest)}</code>` : "";
  const sel = (row.kind === "cmd" || row.kind === "run") && row.run.id === selectedRunId;
  // A running Run's health colours its current stage: slow (still working) or silent (maybe stuck).
  const hk = (row.kind === "cmd" || row.kind === "run") && row.st === "running" && row.run.stage && row.run.stage !== "Tickets" ? health(row.run).key : "";
  const attrs = `data-wf-span="${esc(row.id)}" data-wf-kind="${row.kind}" data-wf-run="${esc(row.run?.id ?? c.id)}" data-wf-cmd="${esc(c.id)}"${row.sid ? ` data-wf-sid="${esc(row.sid)}"` : ""}`;
  const nm = `<div class="wf-nm"${gkey ? ` data-fold="${gkey}"` : ""}>${toggle}<span class="wf-nm-text">${name}</span>${row.kind === "run" || row.kind === "cmd" ? `<span class="wf-nm-sub">${esc(row.run.title ?? row.run.note ?? "")}</span>` : ""}${latest}</div>`;
  const li = `<li class="wf-row wf-k-${row.kind} wf-st-${row.st.replace(" ", "-")}${hk ? ` wf-hl-${hk}` : ""}${isCrit ? " wf-crit" : ""}${sel ? " wf-sel" : ""}" ${attrs} style="--d:${row.depth}"${row.foldIn.length ? ` data-fold-in="${row.foldIn.join(" ")}"` : ""}>
    ${nm}
    <div class="wf-du"><span class="wf-du-t">${durText}</span><span class="wf-du-w">${rowWords(row, now)}</span></div>
    <div class="wf-trk">${bars.join("")}</div>
  </li>`;
  // The calls sit in a row of their own, folded under the group's row.
  if (!gkey) return li;
  return `${li}<li class="wf-calls-row" style="--d:${row.depth}" data-fold-in="${[...row.foldIn, gkey].join(" ")}">
    <p class="wf-calls-note">In the ${esc(sessions[row.sid].kind)} session · about ${dur(row.b - row.a)} (spread evenly: the prototype keeps no per-call times)</p>
    <ol class="sl-calls">${callsHTML(row.group)}</ol></li>`;
}

function stepFor(span) { return [1, 2, 5, 10, 15, 30, 60, 120, 240, 480].find((s) => span / s <= 7) ?? 960; }
function axisHTML(va, vb, t0) {
  const step = stepFor(vb - va), out = [];
  for (let m = Math.ceil(va / step) * step; m <= vb; m += step) {
    out.push(`<span class="wf-tick" style="--t:${m}">${new Date(t0 + m * MIN).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}</span>`);
  }
  return out.join("");
}

// The section body for one Command: which Command it shows, its critical path, then the waterfall. Two siblings,
// so app.js folds both under the section's h2.
export function waterfallHTML(c, { selectedRunId = null } = {}) {
  if (!c) return `<p class="wf-showing">Nothing to show yet.</p>`;
  const now = Date.now();
  const t0 = c.started;
  const { rows, key } = build(c, now);
  const crit = critical(c);
  const end = Math.max(...rows.map((r) => r.b), t0 + MIN);
  const full = { va: -((end - t0) / MIN) * 0.02, vb: ((end - t0) / MIN) * 1.04 };
  const view = views.get(c.id) ?? full;
  const mini = rows.filter((r) => r.depth <= 1 && r.b > r.a).map((r) =>
    `<i class="wf-mb wf-st-${r.st.replace(" ", "-")}" style="--a:${((r.a - t0) / MIN).toFixed(3)};--b:${((r.b - t0) / MIN).toFixed(3)};--r:${r.depth}"></i>`).join("");
  const what = `${esc(c.kind)} · ${c.issue ? esc(c.issue) : esc(c.repo)}`;
  return `<p class="wf-showing">Showing <b>${what}</b>. Select another row to change it.
      <span class="wf-crit-line"><span class="wf-crit-key" aria-hidden="true"></span><b>Critical path</b> ${esc(crit.label)}</span></p>
    <div class="wf-box" data-wf-cmd-box="${esc(c.id)}" data-wf-t0="${t0}" data-wf-fa="${full.va}" data-wf-fb="${full.vb}" style="--va:${view.va};--vb:${view.vb};--fa:${full.va};--fb:${full.vb}">
      <div class="wf-tools"><span class="wf-legend"><i class="wf-lg wf-lg-crit"></i>critical path <i class="wf-lg wf-lg-hatch"></i>waiting <i class="wf-lg wf-lg-sil"></i>silent <i class="wf-lg wf-lg-ghost"></i>usual time <i class="wf-lg wf-lg-live"></i>running</span>
        <span class="wf-zoom"><button type="button" data-wf-zoom="out" aria-label="Zoom out">−</button><button type="button" data-wf-zoom="in" aria-label="Zoom in">+</button><button type="button" data-wf-zoom="fit">Fit</button></span></div>
      <div class="wf-mini" aria-label="Minimap: drag the window to pan">${mini}<span class="wf-win"></span></div>
      <ol class="wf-rows">
        <li class="wf-row wf-axis-row" aria-hidden="true"><div class="wf-nm">Span</div><div class="wf-du">Time</div><div class="wf-trk wf-axis">${axisHTML(view.va, view.vb, t0)}<span class="wf-now" style="--t:${((now - t0) / MIN).toFixed(3)}"></span></div></li>
        ${rows.map((r) => rowHTML(r, t0, now, crit, key, c, selectedRunId)).join("")}
      </ol>
    </div>`;
}

// Pan and zoom by setting the view variables; the bars are positioned in CSS from them, so nothing re-renders. A
// tool-call group's row toggles its calls wherever it is clicked. Bars grow only when the reader opens the section
// or unfolds a span (app.js marks what it reveals with .proto-reveal), never on a re-render.
export function bindWaterfall(root) {
  const box = root.querySelector(".wf-box");
  if (!box) return;
  const cid = box.dataset.wfCmdBox, t0 = Number(box.dataset.wfT0);
  const full = { va: Number(box.dataset.wfFa), vb: Number(box.dataset.wfFb) };
  const win = box.querySelector(".wf-win"), mini = box.querySelector(".wf-mini"), axis = box.querySelector(".wf-axis");
  const setView = (va, vb) => {
    const w = Math.max(2, Math.min(full.vb - full.va, vb - va));
    va = Math.max(full.va, Math.min(full.vb - w, va));
    const v = { va, vb: va + w };
    views.set(cid, v);
    box.style.setProperty("--va", v.va); box.style.setProperty("--vb", v.vb);
    win.style.left = `${((v.va - full.va) / (full.vb - full.va)) * 100}%`;
    win.style.width = `${((v.vb - v.va) / (full.vb - full.va)) * 100}%`;
    axis.querySelectorAll(".wf-tick").forEach((x) => x.remove());
    axis.insertAdjacentHTML("afterbegin", axisHTML(v.va, v.vb, t0));
  };
  const v0 = views.get(cid) ?? full;
  setView(v0.va, v0.vb);
  let drag = null;
  mini.addEventListener("pointerdown", (e) => {
    const r = mini.getBoundingClientRect(), v = views.get(cid);
    const at = full.va + ((e.clientX - r.left) / r.width) * (full.vb - full.va);
    if (e.target !== win) setView(at - (v.vb - v.va) / 2, at + (v.vb - v.va) / 2);
    drag = { x: e.clientX, v: views.get(cid), w: r.width };
    mini.setPointerCapture(e.pointerId);
  });
  mini.addEventListener("pointermove", (e) => {
    if (!drag) return;
    const d = ((e.clientX - drag.x) / drag.w) * (full.vb - full.va);
    setView(drag.v.va + d, drag.v.vb + d);
  });
  mini.addEventListener("pointerup", () => { drag = null; });
  box.addEventListener("click", (e) => {
    const z = e.target.closest("[data-wf-zoom]");
    if (z) {
      const v = views.get(cid), m = (v.va + v.vb) / 2, w = v.vb - v.va;
      if (z.dataset.wfZoom === "fit") setView(full.va, full.vb);
      else { const nw = z.dataset.wfZoom === "in" ? w / 1.6 : w * 1.6; setView(m - nw / 2, m + nw / 2); }
      return;
    }
    // A click on a tool-call group's duration or bar toggles it, as a click on its name does.
    const row = e.target.closest('.wf-row[data-wf-kind="tools"]');
    if (row && !e.target.closest("[data-fold], a, button, summary")) row.querySelector(".wf-nm[data-fold]")?.click();
  });
  // Grow the bars the reader just revealed, for long enough for the animation to finish.
  const grow = (el) => { el.classList.add("wf-grow"); setTimeout(() => el.classList.remove("wf-grow"), 1200); };
  new MutationObserver((ms) => {
    for (const m of ms) if (m.target.classList?.contains("proto-reveal") && !m.target.classList.contains("wf-grow")) grow(m.target);
  }).observe(box.parentElement, { subtree: true, attributes: true, attributeFilter: ["class"] });
}
