// PROTOTYPE (#711): the lanes view, shared by the inbox variants. The "lanes" variant's time axis over the last 3.5
// hours, one lane per Run (Pickup run → Spec run → its Tickets; a died Run's ✕ and silence; a skipped Pass as a tick):
// bars cut into stages and sized by time, so how long each step took reads at a glance. Silence turns a lane red,
// slower than usual amber; a faint band is the usual range. Classes are all lv-*, so a variant styles them under its
// own rules and nothing leaks into its rows. Every lane carries data-lv-run and data-lv-cmd for the variant to link
// to its own selection.
import { commands, pastRuns, RUN_STAGES, SPEC_STAGES } from "./data.js";
import { esc, age, dur, clock, issueLink, runsOf, health, usual, usualText, lastEventAt, heartbeat, SILENT_MIN, isWorking } from "./shared.js";

let SPAN = 210; // minutes on the axis, ending at now; lanesViewHTML({ span }) sets it per render
const GUTTER = 62; // minutes of room right of "now", for each lane's stage · age flag

const kindOf = (r) => r.kind ?? "Ticket";
const slug = (s) => String(s).toLowerCase().replace(/[^a-z]+/g, "-");
const cmdOf = (run) => commands.find((c) => runsOf(c).includes(run));

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
// A Ticket in words, for the chips on its Spec run's lane: words beside every colour.
function ticketWords(t) {
  if (t.state === "running") { const h = health(t); return `${t.stage} · ${dur(Date.now() - t.stageSince)}${h.key === "slow" ? " · slower than usual" : h.key === "silent" ? ` · ${h.label}` : ""}`; }
  if (t.state === "waiting") return `cannot start yet: waiting on ${t.blockedBy.map(shortRef).join(", ")}`;
  return t.state;
}

// What a lane says about where it is now: a flag past "now" on the track, and the label's state line on a phone.
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

// One lane. Flat: no per-Command fold. Only a Spec run's lane folds its Ticket lanes, open by default.
function lane(r, c, depth, now, x, { crit = null, spec = null, dots = false } = {}) {
  const st = runState(r, c), kind = kindOf(r);
  const marks = [];
  if (st === "waiting") {
    const from = Math.max(0, x(c.children?.find((s) => s.tickets?.includes(r))?.started ?? c.started));
    marks.push(`<span class="lv-hatch" style="--x:${from.toFixed(2)};--w:${(x(endOf(r, c, now)) - from).toFixed(2)}" title="waiting, not working"></span>`);
  } else if (st === "skipped") {
    marks.push(`<span class="lv-tickmark" style="--x:${x(c.ended).toFixed(2)}"></span>`);
  } else if (st !== "not started") {
    const segs = segments(r, c, now);
    let n = 0;
    for (const s of segs) {
      const a = Math.max(0, x(s.a)), b = x(s.b);
      if (b <= 0) continue;
      const w = Math.max(0.6, b - a);
      // The usual range for this stage, as a faint band behind the current one: where it "should" have ended.
      const u = s.cur && st === "running" && r.stageSince ? usual(s.stage) : null;
      if (u) {
        const ua = x(r.stageSince + u.lo * 60_000), ub = Math.min(SPAN + GUTTER, x(r.stageSince + u.hi * 60_000));
        if (ub > 0) marks.push(`<span class="lv-ghost" style="--x:${Math.max(0, ua).toFixed(2)};--w:${(ub - Math.max(0, ua)).toFixed(2)}" title="${esc(usualText(s.stage))}, from earlier Runs here"></span>`);
      }
      const h = s.cur && st === "running" ? heat(r) : "";
      // The ink index cycles C/M/Y/K by stage order, for a variant that inks its segments.
      marks.push(`<span class="lv-seg lv-s-${slug(s.stage)} lv-i${n++ % 4}${s.cur ? " lv-cur" : ""}${s.thin ? " lv-thin" : ""}${h ? ` lv-${h}` : ""}${x(s.a) < 0 ? " lv-clipped" : ""}" style="--x:${a.toFixed(2)};--w:${w.toFixed(2)}" title="${esc(s.stage)} · ${dur(s.b - s.a)}">${w >= 16 && !s.thin ? esc(s.stage) : ""}</span>`);
    }
    // Silence: from the last thing this Run logged to now, dotted, over the bar.
    if (health(r).key === "silent") {
      const a = Math.max(0, x(lastEventAt(r)));
      marks.push(`<span class="lv-silence" style="--x:${a.toFixed(2)};--w:${(SPAN - a).toFixed(2)}" title="nothing logged since ${clock(lastEventAt(r))}"><i>silent</i></span>`);
    }
    // Repairs on the CI watch stretch, a Resume at the end of implement.
    const { repairs, resumes } = attempts(r);
    const ci = segs.find((s) => s.stage === "CI watch") ?? segs.at(-1), impl = segs.find((s) => s.stage === "implement");
    for (let k = 1; k <= repairs && ci; k++) {
      const at = x(ci.a + ((ci.b - ci.a) * k) / (repairs + 1));
      if (at > 0) marks.push(`<span class="lv-attempt lv-repair" style="--x:${at.toFixed(2)}" title="Repair ${k}">R${k}</span>`);
    }
    if (resumes && impl && x(impl.b) > 0) marks.push(`<span class="lv-attempt lv-resume" style="--x:${x(impl.b).toFixed(2)}" title="Resume">↻</span>`);
    if (st === "died") {
      const d = x(c.diedAt);
      marks.push(`<span class="lv-gap" style="--x:${d.toFixed(2)};--w:${(SPAN - d).toFixed(2)}"></span><span class="lv-cross" style="--x:${d.toFixed(2)}" aria-label="process gone">✕</span>`);
    }
  }
  const f = flagFor(r, c, now, x);
  marks.push(`<span class="lv-flag ${f.cls ? `lv-${f.cls}` : ""}" style="--x:${f.x.toFixed(2)}">${f.html}</span>`);

  const name = r.issue ? issueLink(r.issue, "ref") : `<b>${esc(c.repo)}</b>`;
  const title = r.title ?? (st === "skipped" ? `skipped: ${c.cause}` : c.note ?? "");
  const working = st === "running" || st === "died";
  const silent = health(r).key === "silent";
  const hb = working ? `<span class="lv-hb">${dots && st === "running" && isWorking(r) ? `<i class="hb-dot${silent ? " flat" : ""}" data-hb="${esc(r.id)}" aria-hidden="true"></i>` : ""}${heartbeat(r)}</span>` : "";
  const chips = r.tickets ? `<div class="lv-chips">${(crit ?? r.tickets).map((t) =>
    `<span class="lv-chip lv-t-${slug(t.state)} ${heat(t) ? `lv-${heat(t)}` : ""}">${esc(t.issue)} <span>${esc(ticketWords(t))}</span></span>`).join("")}</div>
    ${crit ? `<div class="lv-crit-note">critical path: ${crit.map((t) => esc(shortRef(t.issue))).join(" → ")} → Spec PR</div>` : ""}` : "";
  const fold = r.tickets ? ` data-fold="lv-${esc(r.id)}" data-fold-default="open"` : "";
  const onCrit = !crit && r.state && !c.outcome && c.alive && (c.children ?? []).some((ch) => ch.tickets?.includes(r));
  return `<div class="lv-lane lv-d${depth} lv-st-${slug(st)}${onCrit ? " lv-on-crit" : ""}" data-lv-run="${esc(r.id)}" data-lv-cmd="${esc(c.id)}"${spec ? ` data-fold-in="lv-${esc(spec)}"` : ""}>
    <div class="lv-label" style="--d:${depth}">
      <div class="lv-top"${fold}><span class="lv-kind">${esc(kind)}</span> ${name}${onCrit ? ' <span class="lv-crit-tag">critical path</span>' : ""}</div>
      ${title ? `<div class="lv-title">${esc(title)}</div>` : ""}
      <div class="lv-state lv-flag-text ${f.cls ? `lv-${f.cls}` : ""}">${f.html}</div>
      ${hb}
      ${chips}
    </div>
    <div class="lv-track" title="Choose to open this Run">${marks.join("")}</div>
  </div>`;
}

function lanesFor(c, now, x, dots) {
  const out = [lane(c, c, 0, now, x, { dots })];
  for (const ch of c.children ?? []) {
    out.push(lane(ch, c, 1, now, x, { crit: ch.tickets && !c.outcome && c.alive ? criticalPath(ch) : null, dots }));
    for (const t of ch.tickets ?? []) out.push(lane(t, c, 2, now, x, { spec: ch.id, dots }));
  }
  for (const t of c.tickets ?? []) out.push(lane(t, c, 1, now, x, { dots }));
  return out.join("");
}

function axis(now, x, start) {
  const ticks = [];
  const first = new Date(start); first.setSeconds(0, 0); first.setMinutes(first.getMinutes() < 30 ? 30 : 60);
  // Ticks every half hour with hours labelled; on longer windows every hour (labels every 2 h) or every 2 h (every 4 h).
  const step = SPAN <= 240 ? 30 : SPAN <= 720 ? 60 : 120, every = SPAN <= 240 ? 60 : SPAN <= 720 ? 120 : 240;
  const t0 = Math.ceil(start / (step * 60_000)) * step * 60_000;
  for (let t = t0; t <= now; t += step * 60_000) {
    const d = new Date(t), mins = d.getHours() * 60 + d.getMinutes(), labelled = mins % every === 0;
    ticks.push(`<span class="lv-tick${labelled ? "" : " lv-half"}" style="--x:${x(t).toFixed(2)}">${labelled ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false }) : ""}</span>`);
  }
  ticks.push(`<span class="lv-tick lv-now" style="--x:${SPAN}">now</span>`);
  return `<div class="lv-lane lv-axis"><div class="lv-label"><span class="lv-kind">last ${SPAN % 60 ? (SPAN / 60).toFixed(1) : SPAN / 60} h</span></div><div class="lv-track">${ticks.join("")}</div></div>`;
}

// The section body: a legend, then the timeline in its own sideways scroller. Two siblings, so app.js folds both
// under the section's h2; data-scroll-end has app.js bring "now" into view whenever the section is open.
export function lanesViewHTML({ dots = false, span = 210 } = {}) {
  SPAN = span;
  const now = Date.now(), start = now - SPAN * 60_000;
  const x = (ts) => (ts - start) / 60_000;
  const first = new Date(start); first.setSeconds(0, 0); first.setMinutes(first.getMinutes() < 30 ? 30 : 60);
  const onAxis = [
    ...commands.filter((c) => !c.outcome),
    ...commands.filter((c) => c.outcome && c.ended > start).sort((a, b) => b.ended - a.ended),
  ];
  return `<p class="lv-legend"><span class="lv-sw lv-sw-seg"></span>stage <span class="lv-sw lv-sw-cur"></span>current stage
      <span class="lv-sw lv-sw-ghost"></span>its usual range here <span class="lv-sw lv-sw-warm"></span>slower than usual, still working
      <span class="lv-sw lv-sw-hot"></span>silent ${SILENT_MIN}+ min: maybe stuck <span class="lv-sw lv-sw-wait"></span>waiting, not working
      <span class="lv-sw lv-sw-died">✕</span>process gone</p>
    <div class="lv-scroll" data-scroll-end${SPAN === 210 ? "" : ` style="--ppm:${Math.max(0.5, 630 / SPAN).toFixed(3)}px"`}>
      <div class="lv-lanes" style="--span:${SPAN};--gut:${GUTTER};--g0:${x(first.getTime()).toFixed(2)}">
        ${axis(now, x, start)}
        ${onAxis.map((c) => lanesFor(c, now, x, dots)).join("")}
      </div>
    </div>`;
}

// The bars grow left to right when the reader opens the section (app.js marks what it reveals with .proto-reveal);
// a re-render (j/k, the live log) shows them still. The scroll position survives re-renders once the reader has
// moved it away from "now".
let held = null; // scrollLeft the reader chose, or null to follow "now"
export function bindLanesView(root) {
  const sc = root.querySelector(".lv-scroll");
  if (!sc) return;
  const lanes = sc.querySelector(".lv-lanes");
  new MutationObserver(() => {
    if (!sc.classList.contains("proto-reveal")) return;
    lanes.classList.add("lv-grow");
    setTimeout(() => lanes.classList.remove("lv-grow"), 1400);
  }).observe(sc, { attributes: true, attributeFilter: ["class"] });
  // app.js scrolls an open section to "now" after this render; put the reader's own position back once that is
  // done, ignoring the scroll events the two moves cause.
  const want = held;
  let settling = true;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (want != null && sc.offsetParent) sc.scrollLeft = want;
    requestAnimationFrame(() => { settling = false; });
  }));
  sc.addEventListener("scroll", () => {
    if (settling) return;
    held = sc.scrollWidth - sc.scrollLeft - sc.clientWidth < 8 ? null : sc.scrollLeft;
  }, { passive: true });
}
