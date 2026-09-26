// Copy buttons: put the command next to the button on the clipboard.
for (const button of document.querySelectorAll(".install button.copy")) {
  const code = button.closest(".install").querySelector("code");
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(code.textContent);
      button.textContent = "Copied";
    } catch {
      // No clipboard access (e.g. an insecure origin): select the command so it can be copied by hand.
      getSelection().selectAllChildren(code);
      button.textContent = "Selected";
    }
    setTimeout(() => { button.textContent = "Copy"; }, 1600);
  });
}

// How a Run works: step the static press line, unless the reader prefers reduced motion. The markup is the
// source: this only sets state on it, and the terminal replays copies of the lines already in the columns.
const press = document.querySelector(".press");
if (press) stepPressLine(press);

function stepPressLine(section) {
  const REPLAY_MS = 380;
  const units = [...section.querySelectorAll(".press-svg .unit")];
  const delivery = section.querySelector(".press-svg .delivery");
  const cols = [...section.querySelectorAll(".unit-col")];
  const linesByUnit = cols.map(col => [...col.querySelectorAll(".stderr-lines li")]);
  const markers = [...section.querySelectorAll(".press-steps i")];
  const stepButtons = [...section.querySelectorAll(".press-controls [data-step]")];
  const term = section.querySelector(".press-term");
  const log = term.querySelector("ol");
  const pause = term.querySelector(".pause");
  const steppedOnly = [section.querySelector(".press-steps"), section.querySelector(".press-controls"), term];

  let step = 0, paused = false, litLines = [], shown = 0, timer = null;

  // Sets the whole state for step n (1 to 4), never a change from the previous step, so a fast scroll that
  // skips a step can't leave the press and the terminal out of step.
  function render(n) {
    step = n;
    for (const unit of units) unit.classList.toggle("lit", Number(unit.dataset.unit) === n);
    delivery.classList.toggle("lit", n === 4);
    cols.forEach((col, i) => col.classList.toggle("lit", i + 1 === n));
    for (const b of stepButtons) b.setAttribute("aria-current", Number(b.dataset.step) === n ? "step" : "false");
    // Earlier units' lines stay, dimmed, as they would in a real terminal; the lit unit's lines replay.
    log.replaceChildren(...linesByUnit.slice(0, n - 1).flat().map(li => copyLine(li, "old")));
    litLines = linesByUnit[n - 1];
    shown = 0;
    printLine();
    play();
  }

  function copyLine(li, extraClass) {
    const copy = li.cloneNode(true);
    if (extraClass) copy.classList.add(extraClass);
    return copy;
  }

  function printLine() {
    if (shown < litLines.length) log.append(copyLine(litLines[shown++]));
    log.scrollTop = log.scrollHeight;
  }

  // Continues the lit unit's replay. A step rendered while paused shows all its lines at once, so pausing never
  // hides any of them; pausing mid-replay only stops the timer, so Play carries on from the same line.
  function play() {
    clearInterval(timer);
    if (paused) {
      while (shown < litLines.length) printLine();
      return;
    }
    timer = setInterval(() => {
      printLine();
      if (shown >= litLines.length) clearInterval(timer);
    }, REPLAY_MS);
  }

  // For the first render, before the observer reports: the step whose marker holds the viewport's centre line,
  // 1 above the section and 4 below it.
  function stepAtCentre() {
    const centre = innerHeight / 2;
    return Math.max(1, markers.filter(m => m.getBoundingClientRect().top <= centre).length);
  }

  // Follows the observer's own record of which markers cross the centre band, not the live scroll position: the
  // observer only notifies on a change from what it last saw, so reading live positions here could leave a stale
  // step if the page moves back before the next observation. The prototype found a -50% root margin never fires
  // in Chrome; -45% leaves a thin band around the centre.
  const inBand = new Set();
  const observer = new IntersectionObserver(entries => {
    for (const e of entries) {
      const n = markers.indexOf(e.target) + 1;
      if (e.isIntersecting) inBand.add(n);
      else inBand.delete(n);
    }
    // Two markers share the band only at their boundary, where either step is right.
    const n = Math.max(...inBand);
    if (inBand.size && n !== step) render(n);
  }, { rootMargin: "-45% 0px -45% 0px" });

  // A step button scrolls, instantly, to the middle of that step's marker, so the buttons and scrolling agree.
  for (const b of stepButtons) {
    b.addEventListener("click", () => {
      const n = Number(b.dataset.step);
      const r = markers[n - 1].getBoundingClientRect();
      scrollTo({ top: scrollY + r.top + r.height / 2 - innerHeight / 2, behavior: "instant" });
      render(n);
    });
  }

  pause.addEventListener("click", () => {
    paused = !paused;
    pause.textContent = paused ? "Play" : "Pause";
    section.classList.toggle("is-paused", paused);
    if (paused) clearInterval(timer);
    else play();
  });

  function setStepped(on) {
    section.classList.toggle("is-stepped", on);
    for (const el of steppedOnly) el.hidden = !on;
    if (on) {
      for (const m of markers) observer.observe(m);
      render(stepAtCentre());
    } else {
      observer.disconnect();
      inBand.clear();
      clearInterval(timer);
      step = 0;
    }
  }

  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
  setStepped(!reducedMotion.matches);
  reducedMotion.addEventListener("change", () => setStepped(!reducedMotion.matches));
}
