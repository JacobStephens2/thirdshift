# Handoff: Q31 answered, back to the site design session (#29)

You are resuming the `/grill-with-docs` session that is designing thirdshift.app (https://github.com/JacobStephens2/thirdshift/issues/29). That session handed off Q31 ("what should it look like?") to a `/prototype` session. Q31 is now answered. Your next steps are `/to-spec` → `/to-tickets` against #29.

## Where the answer lives

- **Branch `prototype/site`**, pushed to origin (throwaway: never merge, and no PR, since a PR invites merging). Fetch it with `git fetch origin prototype/site`.
- **Commit `5b559d0`**: its message holds the four verdicts and the reason for each. Read it with `git show 5b559d0 --stat`.
- **`site/prototype/index.html`** on that branch is the prototype, and its header comment repeats the verdict. Serve it with `python3 -m http.server -d site 8029`, then open `/prototype/?h=D&p=D&t=A&mode=motion&fx=subtle`.
- **The Tokens button** in the floating bar prints the full token set for the current choice: colours with their roles, fonts, and effect parameters. The CSS custom properties at the top of the file (the `INKS.A` and `FX.subtle` objects) are the source of truth for the spec's design tokens. Don't copy them from anywhere else.

## Verdicts, in brief (full text in the commit)

1. **Hero: D, "Proof, compact".**
   - A paper-white proof sheet with crop marks, registration marks and a colour bar.
   - The blackletter wordmark heads the sheet, with the slug "Proof · pulled at 03:12 · unit 4 of 4".
   - The headline is `clamp(44px, 7.2vw, 100px)`.
   - The framing text sits on the left and the install block, printed on the sheet, on the right.
   - Why: the owner wants the first impression to be "a proof pulled from the press", which matches what he saw at the print shop, whereas masthead A felt like a newspaper. A also left too much empty space in its right middle. The compact size keeps the install command on the first screen, which B (the full-size sheet) did not.
2. **Press line: D**, in the motion state.
   - The side elevation with rollers, from A.
   - A's thin "one agent session" bracket over M and Y. The owner tried the dashed box from floor plan B and preferred the bracket.
   - Big unit labels (24px, e.g. "2 · M · IMPLEMENT"), because A's small labels were hard to read.
3. **Ink and type: A, "Process".** The handoff's starting palette and fonts, unchanged.
4. **Effects: Subtle.** Halftone 35%, misregistration 1px, scanlines 12%.

## Owner's thinking to carry into the spec (not yet decided)

- **Floor plan (press B) for section 5, "The factory grows":** the owner found B more flexible as the process gets more complex ("maybe the printing press factory … can be eventually used"). The suggestion was the elevation for one Run in section 3, and a top-down floor with several lines (#26, #27, #28) in section 5. He said "maybe". Grill it.
- **Is CMYK an artificial constraint on the stages?** The owner raised it. The prototype session's answer was:
  - The inks are identity, not meaning, since every unit also has a number and a name.
  - Real sheet-fed presses have fifth and sixth units (spot colour, coater), so a new stage can become a unit after K.
  - The real risk is the way the stages are split, which follows the code.
  
  The owner has not confirmed this. Grill it.
- **Static (reduced-motion) state of press D:** the owner did not review it. In the prototype, every unit is lit, and below the diagram are four columns, each with that unit's real stderr lines, plus a Failed-run note.

## Facts checked in `src/` that the spec can rely on

The terminal replay's lines in the prototype (`UNITS[].lines`, `JAM.lines`) follow the real `progress::step` formats:
- `worktree.rs`: `creating worktree … on issue-7 from origin/main`, `pushing`, `merging origin/<base> into`, `cleaning up the worktree and local branch`.
- `run.rs`: `checking the PR`, `<cause>; starting Repair n of 3`, `checking the PR is open, ready and mergeable`, `repairs exhausted: <cause>`.
- `ci.rs`: `waiting up to 60s for CI on <sha7>`, `CI on …: n of m checks still running`, `CI passed/failed on …`.
- `session.rs`: `<kind>: session ended after 18m 42s: 96 turns, $5.87 at API prices`.
- `main.rs`: the final line `PR <url> is ready for review`; on failure, the error and then `session log: <path>`.
- The CI grace period defaults to 60s.
- The first visible line of a Run is the worktree line, because pre-flight prints nothing.

The example Run uses acme/widgets#7. The prototype's order of lines on a Failed run (cleanup before the error line) is inferred from the code, not observed.

## Loose ends

- **Research docs on another machine:** the prototype session ran on the owner's Mac, which does not have `docs/research/site-design-inspiration.md`, `site-stack-for-communication.md`, `Tursack_Printing_History.md`, or the uncommitted Day shift entry in `CONTEXT.md`. They are presumably still uncommitted on the other machine. Check `git status` there and don't lose them.
- **Pointer on #29:** the prototype skill wants a comment on #29 pointing to branch `prototype/site` and commit `5b559d0`. It has **not** been posted. Ask the owner before posting, or fold the pointer into the spec issue.
- **Out of scope for the prototype:** sections 5–7 (the factory grows, proof sheets, requirements) were not prototyped, and neither were the deploy, `/install.sh`, favicon, OG cards or analytics (#54).
- **Prototype shortcuts not to copy:**
  - The press line is built by JS, but the real site must ship the static diagram in the HTML for no-JS visitors.
  - Every font is loaded in full, but the real site must subset the wordmark font or use inline SVG.
- **Implementation notes from the prototype:**
  - An IntersectionObserver root margin of `-50% 0px -50% 0px` never fired in Chrome; `-45% 0px -45% 0px` works.
  - Without a `[hidden] { display: none !important; }` reset, `display: grid` overrides the `hidden` attribute.

## Suggested skills

- `grilling` (or the owner's `/grill-with-docs`): settle the open points above.
- `domain-modeling`: if the spec needs new glossary terms, or an ADR for the site's stack and look.
- `/to-spec`, then `/to-tickets` against #29. These are the owner's commands; run them once the grilling is done.
- `writing-for-agents`: when the tickets are written for thirdshift's own agents to implement.
