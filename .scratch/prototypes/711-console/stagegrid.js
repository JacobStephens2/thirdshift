// PROTOTYPE (#711): the stage grid, shared by the inbox variants. Airflow's Grid applied to thirdshift, as in the
// "grid" variant: rows are stages, columns are Runs (live ones pinned first, then the Runs that finished here, newest
// first), every cell a state with its time as text beside the row's usual range. It answers "is this normal?". Also a
// one-Run "stage strip" for a peek: the same cells, in a line. Classes are all sg-*, so a variant styles them under
// its own wrapper and nothing leaks into its rows.
import { commands, pastRuns, RUN_STAGES } from "./data.js";
import { esc, age, clock, isLive, attentionCommands, runsOf, health, usual, usualText, lastEventAt, isWorking } from "./shared.js";

export const STAGES = ["implement", "review", "Delivery", "CI watch", "Repair", "Resume", "Self-merge", "cleanup"];

// The pinned columns: Runs working when first asked, and the died Command. Kept, so a Run stopped from the page
// stays pinned and shows how it ended.
let pinnedCache = null;
const pinned = () => (pinnedCache ??= [
  ...commands.filter(isLive).flatMap((c) => runsOf(c).filter(isWorking).map((run) => ({ run, cmd: c }))),
  ...attentionCommands().map((c) => ({ run: c, cmd: c })),
]);

// Stage minutes for a pinned Run. Only the current stage's start is known; the time before it is shared out over the
// earlier stages in proportion to their usual lengths. Made up for the prototype, as data.js has no per-stage history
// for live Runs.
function synth(r, endAt) {
  const i = RUN_STAGES.indexOf(r.stage);
  const mins = { [r.stage]: Math.max(0, (endAt - r.stageSince) / 60_000) };
  const before = RUN_STAGES.slice(0, Math.max(0, i));
  const span = (r.stageSince - r.started) / 60_000;
  if (before.length && span > 0) {
    const w = before.map((s) => { const u = usual(s); return u ? (u.lo + u.hi) / 2 : 1; });
    const tot = w.reduce((a, b) => a + b, 0);
    before.forEach((s, k) => { mins[s] = (span * w[k]) / tot; });
  }
  return mins;
}

// The Run and Command behind an issue, when the prototype has them.
export function findRun(issue) {
  for (const c of commands) for (const r of runsOf(c)) if (r.issue === issue) return { cmd: c, run: r };
  return { cmd: null, run: null };
}

export function columns() {
  const now = Date.now();
  const live = pinned().map(({ run: r, cmd: c }) => {
    const working = isWorking(r) && isLive(c);
    const died = !c.outcome && !c.alive;
    const endAt = died ? c.diedAt : working ? now : (c.ended ?? now);
    const outcome = c.stopping ? "stopping" : working ? "live" : died ? "died" : r.state === "interrupted" ? "interrupted" : (c.outcome ?? "live");
    return { id: r.id, runId: r.id, issue: r.issue, title: r.title, run: r, cmd: c, pinned: true, working, died, outcome,
      ended: working ? null : endAt, mins: synth(r, endAt), cur: r.stage, repairs: 0, resumes: 0 };
  });
  const past = pastRuns.map((p) => {
    const { cmd, run } = findRun(p.issue);
    return { id: `p:${p.issue}`, runId: run?.id ?? null, issue: p.issue, title: run?.title, run, cmd, pinned: false, outcome: p.outcome,
      ended: p.ended, mins: p.mins, cur: Object.keys(p.mins).at(-1), repairs: p.repairs ?? 0, resumes: p.resumes ?? 0 };
  });
  return [...live, ...past];
}

const fmt = (m) => (m < 1 ? "<1m" : m < 60 ? `${Math.round(m)}m` : `${Math.floor(m / 60)}h ${Math.round(m % 60)}m`);
const chip = (outcome) => `<span class="sg-chip o-${esc(outcome)}">${esc(outcome)}</span>`;

// One cell: its state class and words. A live cell's time ticks; finished ones are fixed text.
export function cellInfo(col, stage) {
  const m = col.mins[stage];
  if (m == null) return null;
  const u = usual(stage);
  const ratio = u ? m / u.hi : 0;
  const level = !u ? 2 : ratio < 0.5 ? 1 : ratio < 1 ? 2 : ratio < 1.5 ? 3 : ratio < 2 ? 4 : 5;
  const isCur = stage === col.cur;
  let cls = `done l${level}`, time = fmt(m), word = "";
  if (isCur && col.died) { cls = "died"; word = "died"; }
  else if (isCur && col.working) {
    const h = health(col.run);
    time = age(col.run.stageSince);
    if (h.key === "silent") { cls = "silent"; word = `silent ${fmt((Date.now() - lastEventAt(col.run)) / 60_000)}`; }
    else if (h.key === "slow") { cls = "live slow"; word = "slow, moving"; }
    else { cls = "live"; word = "running"; }
  } else if (isCur && col.outcome === "stopping") { cls = "live"; word = "stopping…"; }
  else if (isCur && col.outcome === "failed") { cls = `failed l${level}`; word = "failed"; }
  else if (isCur && col.outcome === "interrupted") { cls = `interrupted l${level}`; word = "interrupted"; }
  else if (isCur && ["merged", "ready"].includes(col.outcome)) word = col.outcome;
  const times = u && !col.died ? Math.floor(m / u.hi) : 0;
  const flag = times >= 2 ? `${times}× usual` : "";
  const count = stage === "Repair" ? col.repairs : stage === "Resume" ? col.resumes : 0;
  return { cls, time, word, flag, count, cur: isCur };
}

const cellInner = (info) => `<span class="sg-time">${info.time}</span>${info.word ? `<span class="sg-word">${esc(info.word)}</span>` : ""}`
  + `${info.flag ? `<span class="sg-flag">${info.flag}</span>` : ""}${info.count ? `<span class="sg-dot" title="${info.count} in this Run">● ${info.count}</span>` : ""}`;

const runAttr = (col) => `data-sg-col="${esc(col.id)}"${col.runId ? ` data-sg-run="${esc(col.runId)}"` : ""}`;

// The grid: a table on wide screens, one card per Run on a phone. selectedRunId outlines that Run's column.
export function stageGridHTML(selectedRunId = null) {
  const cols = columns();
  const isSel = (col) => !!selectedRunId && col.runId === selectedRunId;
  const head = cols.map((col, ci) => `<th scope="col" class="sg-h${col.pinned ? " pin" : ""}${isSel(col) ? " sel" : ""}" data-c="${ci}" style="--c:${ci}">
      <button type="button" class="sg-hbtn" ${runAttr(col)} aria-pressed="${isSel(col)}">
        ${col.pinned ? `<span class="sg-eyebrow">${col.died ? "died" : "live"}</span>` : ""}<span class="sg-issue">${esc(col.issue)}</span>${chip(col.outcome)}
        <span class="sg-when">${col.ended ? clock(col.ended) : "now"}</span></button></th>`).join("");
  const rows = STAGES.map((stage, ri) => {
    const cells = cols.map((col, ci) => {
      const info = cellInfo(col, stage);
      const extra = `${col.pinned ? " pin" : ""}${isSel(col) ? " sel" : ""}`;
      if (!info) return `<td class="sg-c empty${extra}" data-r="${ri}" data-c="${ci}" style="--c:${ci}"><span aria-label="not reached">·</span></td>`;
      return `<td class="sg-c ${info.cls}${info.cur ? " cur" : ""}${extra}" data-r="${ri}" data-c="${ci}" ${runAttr(col)} tabindex="0" style="--c:${ci}"
        aria-label="${esc(`${col.issue} ${stage}: ${info.word || ""} ${info.flag}`)}">${cellInner(info)}</td>`;
    }).join("");
    return `<tr><th scope="row" class="sg-rh" data-r="${ri}" style="--i:${ri}"><span class="sg-tag" aria-hidden="true"></span><span class="sg-stage">${esc(stage)}</span><span class="sg-usual">${esc(usualText(stage) || "too few Runs for a usual")}</span></th>${cells}</tr>`;
  }).join("");
  const table = `<div class="sg-scroll"><table class="sg-table">
    <caption>Each cell is the time a Run spent in a stage, shaded deeper as it runs past the row's usual range. Live Runs are pinned on the left; choose one to select it.</caption>
    <thead><tr><th scope="col" class="sg-corner">Stage · usual</th>${head}</tr></thead><tbody>${rows}</tbody></table></div>`;
  const cards = `<div class="sg-cards">${cols.map((col) => {
    const items = STAGES.map((stage) => {
      const info = cellInfo(col, stage);
      return info ? `<li class="sg-c ${info.cls}"><span class="sg-stage">${esc(stage)}</span>${cellInner(info)}<span class="sg-usual">${esc(usualText(stage))}</span></li>` : "";
    }).join("");
    return `<article class="sg-card${col.pinned ? " pin" : ""}${isSel(col) ? " sel" : ""}">
      <button type="button" class="sg-cardhead" ${runAttr(col)}><span class="sg-issue">${esc(col.issue)}</span>${chip(col.outcome)}<span class="sg-when">${col.ended ? clock(col.ended) : "now"}</span></button>
      ${col.title ? `<p class="sg-cardtitle">${esc(col.title)}</p>` : ""}<ul>${items}</ul></article>`;
  }).join("")}</div>`;
  return `<div class="sg">${table}${cards}</div>`;
}

// One line about a past Run the page has no row for: issue, outcome, stage minutes, Repairs.
export function pastNote(colId) {
  const col = columns().find((c) => c.id === colId);
  if (!col) return "";
  const stages = STAGES.filter((s) => col.mins[s] != null).map((s) => `${s} ${fmt(col.mins[s])}`).join(" · ");
  return `<p class="sg-note"><b>${esc(col.issue)}</b> ${esc(col.outcome)} ${col.ended ? `at ${esc(clock(col.ended))}` : ""} · ${esc(stages)}`
    + `${col.repairs ? ` · ${col.repairs} Repair${col.repairs === 1 ? "" : "s"}` : " · no Repairs"}${col.resumes ? ` · ${col.resumes} Resume` : ""}`
    + ` <span class="sg-note-why">· older than the Commands this page lists, so it has no row</span></p>`;
}

// The stage strip for one Run: its stages in order, each with its time and usual range, the current one lit.
export function stageStripHTML(run) {
  if (!run) return "";
  const col = columns().find((c) => c.runId === run.id) ?? columns().find((c) => run.issue && c.issue === run.issue);
  if (!col) return "";
  const items = STAGES.map((stage) => {
    const info = cellInfo(col, stage);
    if (!info) return "";
    const u = usual(stage);
    return `<li class="sg-c sg-s ${info.cls}${info.cur ? " cur" : ""}"><span class="sg-stage">${esc(stage)}</span> ${cellInner(info)}`
      + `${u ? `<span class="sg-usual">usually ${u.lo}–${u.hi}</span>` : ""}</li>`;
  }).join("");
  return items ? `<ol class="sg sg-strip" aria-label="Stages">${items}</ol>` : "";
}

// Hovering or focusing a cell lights its row and column.
export function bindStageGrid(root) {
  const t = root.querySelector(".sg-table");
  if (!t) return;
  const clear = () => t.querySelectorAll(".hl").forEach((x) => x.classList.remove("hl"));
  const light = (e) => {
    const cell = e.target.closest("[data-r], [data-c]");
    clear();
    if (!cell) return;
    const { r, c } = cell.dataset;
    if (r != null) t.querySelectorAll(`[data-r="${r}"]`).forEach((x) => x.classList.add("hl"));
    if (c != null) t.querySelectorAll(`[data-c="${c}"]`).forEach((x) => x.classList.add("hl"));
  };
  t.addEventListener("mouseover", light);
  t.addEventListener("focusin", light);
  t.addEventListener("mouseleave", clear);
}
