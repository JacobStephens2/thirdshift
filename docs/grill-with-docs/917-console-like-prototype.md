# Grilling: #917, make the Console more like the #711 prototype

Issue: https://github.com/JacobStephens2/thirdshift/issues/917
Research: the prototype at commit `05b1b0a` (branch `prototype/711-console`), `.scratch/prototypes/711-console/`, and its `FEEDBACK.md`; [ADR-0018](../adr/0018-the-console-has-one-layout-and-themes.md)

## Round 1 (open)

**Q1 - How close a match.** What "more like the prototype" means decides how every later gap is judged. The options:
- (a) **The prototype is the reference.** Its `console` variant defines the layout, and the Focus Theme's colours and fonts should match it. The live Console departs from it only where a written decision says so.
- (b) **Pick features.** Bring back a named list of things you miss and leave the rest as it is.
- (c) **Same feel, not the same pixels.** Get the density and hierarchy back, but don't hold the live Console to the prototype's details.

*Recommended:* (a). You chose this variant in #711, and ADR 0018 already says there's one layout and every look is a Theme. Making the prototype the reference turns each later gap into a yes-or-no question instead of a matter of taste.

*Answer:* 

**Q2 - Where an Inbox row's extra detail goes.** A live Inbox row shows a lot the prototype keeps out of the row: the harness and model (`codex · gpt-6.1-sol · high`), the PR URL, the cause, a list of Session log file paths, and nested child Runs. The prototype row is one line and leaves detail to the peek (Space). Outside Needs you, live rows also show only the raw `live`, `ended` or `died`. The prototype gives every row a symbol and a state word: ◐ Working, ↧ Landing, ◆ Ready, ✓ Merged, ■ Stopped, – Skipped. The options:
- (a) **One line, detail in the peek.** A row is: symbol and state word, kind (RUN / SPEC RUN / TICKET), issue and title, stage · reason, age, and Stop when the Command is live. Everything else goes in the peek, or on the log page it links to.
- (b) **Keep the cards** and only restyle them.

*Recommended:* (a). Nothing is lost; it's one keypress away. It also brings back a state word for every row, which makes the live CSS styles for Landing and Ready work again. Those two, and Stopping, are filed as a bug of their own, #1106, so they don't wait for this.

*Answer:* 

**Q3 - Width.** The prototype uses the whole window, up to 1600px. The live Console uses a 1120px centred column, which is why its Stages table runs out of room. `FEEDBACK.md` records that you found the `briefing` variant too narrow. The options:
- (a) **Full width** with a 16px gutter on every page that shows Commands. Mostly-prose pages such as Account and sign-in keep a narrow column.
- (b) **Keep the column.**

*Recommended:* (a).

*Answer:* 

**Q4 - Header controls.** The prototype puts the Theme picker (with colour swatches), Pause motion, Sections, the other-Consoles link and Sign out in the header, with a strip of keyboard hints under them. The live Console has Themes, Sections, Pause motion and Keyboard help as plain buttons in a footer, no hint strip, and Sign out only on the Account page. Its tabs add Console log and Account, which the prototype doesn't have. The options:
- (a) **The prototype's header.** Controls and Sign out at the top, the hint strip under them, Console log and Account kept as tabs to the right of 1–7. On a phone the hint strip hides and the controls collapse into one menu.
- (b) **Keep the footer.**

*Recommended:* (a).

*Answer:* 

**Q5 - Stages page.** The prototype's Stages page is a grid with one column per Run, live Runs pinned on the left, stages in pipeline order, a usual-range label on each stage, and cells shaded more deeply the further a stage runs past its **Usual range**. Stages a Run never reached are blank. The live page is a plain table that says "Not reached · —" in most cells, sorts stages alphabetically, and starts a new table every 3 Runs. The options:
- (a) **The prototype's grid, one per repository.** Stages in pipeline order, no split every 3 Runs, blanks for stages not reached.
- (b) **The prototype's grid with every repository mixed**, as its made-up data shows.
- (c) **Keep the table** and only fix the stage order.

*Recommended:* (a). A Usual range belongs to one repository, so mixing repositories in one grid would put different ranges side by side. The alphabetical order is a bug whatever you choose, filed as #1105.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- #917 is an `enhancement`, now labelled `needs-triage` instead of `ready-for-human`. Once grilled, it becomes a Spec with Tickets, and it stays `needs-triage` until they're published, per `docs/agents/triage-labels.md`.
- ADR 0018 still holds: there is one layout, and a Theme may change only colours, fonts, shape, effects and motion. Bringing back the prototype's rows, header and grids is layout work for every Theme. Only the look, such as the uppercase mono labels, goes in Theme tokens.
- ADR 0016 still holds: every view and action has a CLI equivalent. The live Console's line naming the matching command stays, even though the prototype has none.
- Themes stay machine-wide and are edited as JSON, not per browser with a visual editor as in the prototype. That was decided for #711 (ADR 0016's amendment for Q46) and isn't reopened here.
- The live Console's contrast fixes to the Press and Factory Themes stay.

## Research results (2026-10-21)

- **The prototype's saved tab is the git version.** Nothing was committed to `prototype/711-console` after `05b1b0a`. `~/Downloads/7 need you · thirdshift Console · PROTOTYPE #711.html` is a 26 KB snapshot of the drawn Inbox with no scripts, so only the git copy works. Confirmed by `git log` and by comparing the saved files.
- **The prototype's server at :8711 serves 404s:** its folder, the deleted `prototypes` worktree, is gone. A copy from git is served at http://localhost:8712/?variant=console&page=inbox. Confirmed by `lsof` and the browser.
- **Stage order is alphabetical** (filed as #1105) because `src/logs/status/time.rs:230` gathers stage names in a `BTreeSet`; `:218` splits tables at 3 Runs. Confirmed by source and the live Stages page.
- **The live Console's Landing and Ready styles never apply** (filed as #1106, with Stopping): `src/logs/status/inbox.rs:315` sets the row state to `live`, `ended` or `died`, so `data-state="Landing"` and `"Ready"` in `src/console/assets/console.css:65-66` never match. Confirmed by source.
- **The Focus Theme's colours, fonts, 14px body and 600 weight match the prototype** (`src/console/assets/themes.css:19-67` against the prototype's `themes.js:208-226`). Live lacks the prototype's label-case and tracking, size, density, terminal and stage-colour tokens and its effect switches. Live loads IBM Plex Mono at 400 only, so 700-weight mono labels would be faked by the browser. Confirmed by source.
- **Further gaps, for later rounds.** Confirmed by reading both sources:
  - **Inbox:** no notes on section headings ("4 urgent · urgent first", "last 24 h"), no escalation step, no inline Stop. "New since you last looked" compares start times instead of when the problem began. Spec run Tickets show "☐" whatever their state.
  - **Peek:** a generic list of every `show --json` field instead of chosen facts, a Stages strip and a log tail. Stop opens a separate page.
  - **Lanes:** a bar per Run, with no usual-range band, silence or Slow marks, Repair and Resume markers, or Ticket lanes.
  - **Waterfall:** every tool call expanded, instead of grouped and unfolding.
  - **History:** select boxes, instead of toggle chips with counts.
  - **Logs:** separate list and reader pages, with no search, raw view, download or new-entries pill.
  - **Security:** cards, instead of findings with severity, "reproduced" and "fixed by".
  - **Keyboard:** no `t` to tile live logs. Enter opens the Command log, not the Session log, and several `g` letters differ.
  - **Tab title:** "N need you" and the urgent favicon show only on the Inbox, and the title reads "0 need you" when nothing waits.
  - **Phone:** no compact page menu and no Stage cards.

## Waiting on research

- Nothing is running.
- **Parked (2026-10-22)** until the Specs that add Console pages land: #700, #977, #978 and #981, linked as #917's blockers. Re-check the gaps above against the Console then. This file's state is also posted as a triage comment on #917.
