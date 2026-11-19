# Time pages

Lanes and Stages display Command timelines and per-stage durations against each repository's Usual range, with window filtering and repository grouping.

## Sub-features

- `lanes-window` switches the Lanes time window (`3.5`, `12`, `24` hours) and opens a run's Peek.
- `stages-views` switches Stages between `All runs` and `By repository`, expands folded repository grids, and opens a column's Peek.

## How to get to it (user POV)

- Choose `2 Lanes` (`2` or `g` then `b`) or `3 Stages` (`3` or `g` then `t`) in `nav[aria-label="Pages"]`.

## Driving it with Playwright

Preconditions:

- Console started with `--no-sign-in` against `seed-home.cjs` (records started within the last 35 minutes so they appear inside the default `3.5 hours` window) and `Doctor` passed.

- **Inspect Lanes and change window.** Navigate to `${origin}/?page=lanes`. `.lane .time-run` renders seeded runs (`acme/widgets#7`, `#8`, `#9` and `acme/gadgets#10`) with `.timeline` bars. Select `12` in `[name="hours"]` and click `Show`; URL contains `hours=12` and runs remain visible. Click the `.peek-link` for `acme/widgets#8` to verify Peek opens beside Lanes.
- **Inspect Stages and toggle By repository.** Navigate to `${origin}/?page=stages`. Select `repository` in `#stages-view` and wait for `details.stage-section` count to reach `2` (the view switch morphs asynchronously). Separate `details.stage-section` disclosures appear for `acme/gadgets` and `acme/widgets`. Click each `details.stage-section > summary` to expand its `.stage-grid` table showing lowercase stage row headers (`implement`, `review`, `CI watch`, `cleanup`) and `Usual range:`.
- **Proof.** Capture `1440×900` `.png` of Stages with `By repository` grids expanded (or Lanes) and record a Phase 2 `.mp4` walkthrough switching Lanes -> Stages `By repository` -> opening a column Peek.

## Gotchas

- When multiple repositories exist (`acme/widgets` and `acme/gadgets`), `By repository` renders each `details.stage-section` folded closed on load; expand `summary` before reading `.stage-grid`.
- `#stages-view` persists the selected mode in `localStorage` (`thirdshift:console:stages-view`); reset it to `All runs` (`all`) if a later check expects the default view.
