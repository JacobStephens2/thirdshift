# Improving the Focus theme: shaping attention to where it is most needed

Research date: 2026-10-19, for issue #711 (the Console) and the themed `console` prototype in `.scratch/prototypes/711-console/` (kept on the branch `prototype/711-console`, commit `05b1b0a`). It builds on [console-run-monitoring-ui.md](console-run-monitoring-ui.md) (what earns attention; silence, not stage age, as the stall rule) and [software-factory-console-trends.md](software-factory-console-trends.md) (attention-state grouping), and doesn't repeat them.

**Question.** The Focus theme (`themes.js`: `focusLight`, `focusDark`, `focusAuto`; `variants/console.css`: the `focus` effect block) sets visual weight by how much an item needs the Day shift:

- urgent rows get a crimson 6px edge, a 9% tinted band and bold text;
- look rows get a 3px amber edge;
- healthy groups recede to the dim text colour;
- healthy timeline bars desaturate.

How could it better steer attention to what needs a decision, in light and dark, while staying calm and accessible, and within the Console's rules? Those rules are: nothing the CLI can't do; motion only for thirdshift's activity; phone first; words beside colour.

**Method.**

- **Primary sources, cited inline:**
  - Christopher Healey's pre-attentive processing survey (his own pages);
  - the W3C's WCAG 2.2 Understanding documents;
  - the APCA project's own documentation;
  - Google's SRE book and Material dark-theme codelab;
  - Nielsen Norman Group;
  - PagerDuty, Grafana and Datadog docs.
- **Paywalled standards.** ISA-101 (high-performance HMI) and ISA-18.2 / EEMUA 191 (alarm management) are paywalled. I used the publisher's own overview, plus vendor summaries that name the standard: Emerson/Movicon for ISA-101, ABB for EEMUA 191. Figures found only in secondary write-ups are marked **[unverified]**.
- **Measured contrast.** Every current Focus colour pair was measured with both the WCAG 2 ratio and APCA Lc, using the APCA-W3 0.0.98G constants in a small script. The numbers are in §1.
- **Out of scope.**
  - The page layout itself (the inbox structure is settled).
  - Other themes, except where the `focus` effect is reused on them.
  - Anything that writes to thirdshift: there is no acknowledge or shelve command in the CLI.

## TL;DR

- **Fix dark mode's inverted hierarchy first.** In Focus dark, receded "dim" text has *more* perceived contrast than urgent text: APCA Lc −50.6 against −48.8, because saturated red is dark ([APCA in a Nutshell](https://git.apcacontrast.com/documentation/APCA_in_a_Nutshell)). Saturated reds also "vibrate" on dark grounds ([Google codelab](https://codelabs.developers.google.com/codelabs/design-material-darktheme)). Lighten urgent text to a tint (#FFB0B5, Lc −70.9) and keep dim where it is.
- **Make the amber edge pass 1.4.11.** In Focus light, the look edge is #E08A00, which is 2.5:1 against the background. As a state indicator it needs 3:1 ([WCAG 1.4.11](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)). Use #B06800 (4.06:1).
- **Recede further in light.** Light dim reads at Lc 80.9, nearly level with urgent text (76.8). Darken urgent text to #9A1424 (Lc 82.3) and lighten dim to #6B6B70 (Lc 71.5, WCAG 4.94). Recede by colour plus weight, never by size or opacity ([NN/g](https://www.nngroup.com/articles/visual-hierarchy-ux-definition/)).
- **Take hue off healthy states.** Grey-scale the healthy states (working, landing, ready, done) in Focus. That leaves hue as the one feature urgent and look rows own, so they pop out pre-attentively rather than by search ([Healey](https://www.csc2.ncsu.edu/faculty/healey/PP/)). It's also what high-performance HMIs do: grey baseline, colour "to indicate the abnormal, rather than normal operations" ([Emerson/Movicon on ISA-101](https://resources.ipd.com.au/documents/white-papers/Best-practices-for-high-performance-HMI-design_White%20Paper_ENUS_2023.pdf)).
- **Add time without motion.** Two static escalation steps by silence or age, each with words: "silent 14 min" against "silent 45 min, past 30". This follows Grafana-style thresholds ([Grafana](https://grafana.com/docs/grafana/latest/panels-visualizations/configure-thresholds/)) and PagerDuty's raising of urgency when a condition persists ([PagerDuty](https://support.pagerduty.com/main/docs/configurable-service-settings)).
- **Plan for floods.** Past about seven Needs-you items, show a one-line summary by kind and fold the rest behind "N more". Alarm guidance treats floods as the failure mode, and keeps high priority rare: about 5% high, 15% medium, 80% low ([ABB summary of EEMUA 191](https://library.e.abb.com/public/3fe0e33c700746d0bcf700526ba81c62/3BDD017132_en_3BDD017132_EN_Alarmmanagement_Check.pdf)).
- **Stop pulses after a few cycles.** The silent alarm loops indefinitely, which WCAG 2.2.2 says needs a pause mechanism after 5 seconds ([WCAG 2.2.2](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html)). Run it 3 times when a row turns silent, then hold a strong static state. The activity motion (spinner, belt) also needs a visible pause; the theme editor's motion setting is too buried.
- **No shelving or acknowledging.** These need a write the CLI can't do. A per-viewer "new since you last looked" marker, read-only and kept in the browser, is legitimate. It should add emphasis to what's new, never hide what isn't.

## 1. What Focus does today, measured

Light bg #F7F7F5 and dark bg #121214. WCAG 2 contrast ratio and APCA Lc (APCA-W3 0.0.98G). APCA Lc is signed: negative for light text on a dark ground.

| Pair | Light WCAG | Light Lc | Dark WCAG | Dark Lc |
|---|---|---|---|---|
| text | 16.53 | 99.7 | 15.29 | −92.4 |
| dim (receded rows) | 6.91 | **80.9** | 7.21 | **−50.6** |
| urgentText | 6.37 | 76.8 | 6.79 | **−48.8** |
| lookText | 6.70 | 79.7 | 10.60 | −70.0 |
| urgent edge/fill | 4.62 | 67.6 | 5.77 | −42.7 |
| look edge/fill | **2.51** | 46.9 | 10.23 | −68.1 |
| rule | 1.20 | 9.3 | 1.35 | ~0 |

What the table shows:

- **Every text pair passes WCAG 1.4.3.** But WCAG ratios aren't perceptually uniform: they overstate contrast for dark colours ([APCA in a Nutshell](https://git.apcacontrast.com/documentation/APCA_in_a_Nutshell)). In APCA terms the hierarchy is wrong in both modes:
  - in dark, receded text out-contrasts urgent text (−50.6 against −48.8), and look text (−70) out-shouts urgent;
  - in light, receded text (80.9) is a hair below urgent (76.8) and look (79.7). The difference rests on bold weight and the band alone.
- **The light look edge fails 1.4.11.** WCAG 1.4.11 needs 3:1 for "visual information necessary to indicate state" ([WCAG 1.4.11](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)). The amber look edge at 2.51:1 fails as the row's state cue. The word next to it saves 1.4.1, but not 1.4.11.
- **Rules fall below APCA's floor.** Rules are under Lc 15, which APCA treats as invisible ([APCA](https://git.apcacontrast.com/documentation/APCA_in_a_Nutshell)). That's fine for decoration, but don't rely on them to separate groups in Focus. Use spacing.

## 2. Salience: what pops out without searching

- **One unique feature pops out; combinations don't.** A target with one unique visual property "pop[s] out", while a target defined by a combination of features ("conjunction target") "normally cannot be detected preattentively" and needs serial search ([Healey, Perception in Visualization](https://www.csc2.ncsu.edu/faculty/healey/PP/)). Colour (hue) and intensity are both pre-attentive features (same source).
- **Hue dominates when others use it too.** The visual system "favours colour over shape" for boundary detection, and colour variation interferes with other features ([Healey](https://www.csc2.ncsu.edu/faculty/healey/PP/)).
- **Implication for Focus:**
  - Urgent should be the only thing on screen with a saturated warm hue *and* a filled band.
  - Look should be the only thing with a thin warm edge.
  - Healthy rows should carry no hue at all.
- **Today's healthy colours compete.** Healthy rows still show slate (#6B7A8F) and sage (#5F7F6B) tags. Those are low-chroma hues, but hues nonetheless. They give the eye extra coloured targets, which weakens urgent's single-feature pop-out.
- **Change blindness.** Changes during an interruption or a glance away are missed, "not a failure to see because of limited visual acuity" ([Healey](https://www.csc2.ncsu.edu/faculty/healey/PP/)). When a row newly becomes urgent between visits, its arrival can go unseen. That's an argument for a "new since you last looked" marker (§7) rather than relying on the reader noticing.

## 3. Alarm-management lessons: rare, ranked, floods handled

- **Grey baseline, colour for the abnormal.** High-performance HMI practice, as summarised by a vendor citing ISA-101, says "the background color should be light gray". It says foreground colours are "used sparingly to indicate abnormal situations". It also says "color should be removed from graphics in favor of grayscale and then used to indicate the abnormal, rather than normal operations" ([Emerson/Movicon white paper on ISA-101](https://resources.ipd.com.au/documents/white-papers/Best-practices-for-high-performance-HMI-design_White%20Paper_ENUS_2023.pdf)). The same paper warns that "overuse of red or green in everyday graphics to indicate running/not running can make it hard to see abnormal events". It adds that alarms "should be shown in multiple ways, with different colors, shapes and text". ISA's own page for ISA-101 confirms only its scope ("human-centered design, effective display structures"), not these specifics ([ISA](https://www.isa.org/standards-and-publications/isa-standards/isa-101-standards)).
- **Keep high priority rare.** An ABB summary of EEMUA 191 gives "5% as high, 15% as medium, and 80% as low". It also says fewer than ten alarms should arrive "within the first 10 minutes of a major system failure" ([ABB](https://library.e.abb.com/public/3fe0e33c700746d0bcf700526ba81c62/3BDD017132_en_3BDD017132_EN_Alarmmanagement_Check.pdf)). The ISA-18.2 flood threshold of "more than 10 alarms in 10 minutes per operator" appears only in secondary write-ups, such as [TiPS](https://tipsweb.com/taming-alarm-floods/) **[unverified]**.
- **For thirdshift:**
  - Focus should have three levels, no more: urgent (high), look (medium), and everything else (no alarm).
  - The page should handle the case where Needs you grows long. On a bad night, a broken Base branch can fail every Pickup run.
- **Shelving doesn't fit.** Alarm systems let operators shelve a nuisance alarm, with a time-out and an audit trail. The Console can't, because the CLI has no such command. See §7 for what is legitimate.

## 4. Attention economics

- **Pages must be actionable.** Every page should be actionable, and "when pages occur too frequently, employees second-guess, skim, or even ignore incoming alerts" ([Google SRE, Monitoring Distributed Systems](https://sre.google/sre-book/monitoring-distributed-systems/)). Focus's urgent level should therefore be reserved for things only a human can resolve: died, silent, failed, and Security findings that wait. That's what `needsYou()` already marks urgent. Slow-but-moving stays "look".
- **Limit the levels of emphasis.** NN/g: "If everything is contrasted, then nothing stands out". Use "no more than 3 contrast variations for complex designs" and no more than three sizes. Hierarchy shouldn't rely on colour alone ([NN/g, Visual Hierarchy](https://www.nngroup.com/articles/visual-hierarchy-ux-definition/)). Focus today has four emphasis treatments (band, edge, bold, dim); keep it to three contrast steps.
- **Rank by urgency and fold the rest.**
  - PagerDuty separates high urgency ("requires immediate attention") from low ("can wait"), with different notification behaviour for each. Services can "raise urgency of unacknowledged incidents to high when support hours start" ([PagerDuty: configurable service settings](https://support.pagerduty.com/main/docs/configurable-service-settings); [notification rules](https://support.pagerduty.com/main/docs/notification-rules)).
  - Datadog gives monitors a priority from P1 to P5 that the monitor data carries ([Datadog DDSQL: monitor groups](https://docs.datadoghq.com/ddsql_reference/data_directory/dd/dd.monitor_groups.dataset/)).
  - Grafana colours by discrete thresholds above a base value ([Grafana thresholds](https://grafana.com/docs/grafana/latest/panels-visualizations/configure-thresholds/)).
  - The common pattern is a small, ranked set of levels, with a level that can rise as a condition persists. Nothing flickers.

## 5. Dark mode

- **The inversion.** The hierarchy inverts in dark (§1). Saturated colours "tend to visually 'vibrate' against darker backgrounds, making them harder to read", and lighter tonal variants keep the palette expressive ([Google: Design a dark theme with Material](https://codelabs.developers.google.com/codelabs/design-material-darktheme)).
- **The fix: raise urgent, keep receded where it is.**

  | Token | Now | Proposed | WCAG | Lc |
  |---|---|---|---|---|
  | urgentText | #FF6B78 | #FFB0B5 (a light tint) | 10.81 | −70.9 |
  | urgent edge | — | keep saturated #FF6B78 | 6.79 | — |
  | dim | #A0A0A8 | unchanged | — | −50.6 (≥ Lc 45, so receded rows stay readable) |

  Receding by darkening dim instead (#85858D, Lc −36.9) would push healthy text below APCA's Lc 45 floor for text ([APCA](https://git.apcacontrast.com/documentation/APCA_in_a_Nutshell)), so don't.
- **Band.** The 9% band on #121214 gives #27171B. Urgent text on it is fine (WCAG 6.2). A 12% band reads more clearly as a region in dark without vibrating, since it's a large low-chroma area **[unverified, judgement]**.
- **Surface.** Material uses #121212 as the base dark surface (same codelab), and Focus's #121214 already matches.

## 6. Time without motion

- **Static escalation steps.** Focus currently treats a 14-minute silence and a 3-hour silence the same. Add two static steps, Grafana-threshold style, each with words (WCAG 1.4.1 needs a non-colour cue ([WCAG 1.4.1](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html))):
  - **Silent:** 10–30 min is "silent N min"; over 30 min is "silent N min · past 30". The latter gets a doubled edge (6px to 10px), and its age in bold.
  - **Look, slow:** over usual hi is "slower than usual". Over 2× usual it says "2× usual", and the age is bold.
  - **Died:** it stays at the top level; age only orders it.

  It's the same idea as PagerDuty raising urgency when a condition persists, without a notification.
- **Order by escalation.** Within Needs you, sort urgent by escalation step, then by age, so the longest-silent item is first.
- **The alarm pulse.** WCAG 2.2.2 requires a way to pause, stop or hide motion that starts automatically and "lasts more than five seconds" ([WCAG 2.2.2](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html)). Today's silent pulse is infinite, so:
  - set `animation-iteration-count: 3` when a row enters the silent state (keyed by a class added once, not on every render);
  - then hold the static past-threshold treatment.
- **Activity motion.** The Working spinner, belt and heartbeat also auto-start and last beyond 5s. Add a visible one-click "Pause motion" control in the header, or move the theme editor's motion level onto the Theme button's panel. `prefers-reduced-motion` already covers the OS setting ([WCAG 2.3.3, technique C39](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions.html)).

## 7. Floods, "seen", and what's legitimate

- **Floods.** When Needs you has more than 7 items (the prototype has 7 today):
  - Show a summary line under the heading, e.g. "4 failed · 2 silent · 1 died · 3 look". Each part is a filter chip.
  - List urgent items first, up to 7 rows, then fold the rest behind "N more".
  - Look items fold before urgent ones do.
  - Group repeated causes: several Runs failing on the same Inherited failure collapse to one row, "CI red on clippy, also failing on main: 4 Runs". That's one problem, one item, as the SRE book argues for symptoms ([Google SRE](https://sre.google/sre-book/monitoring-distributed-systems/)).
- **Acknowledging and shelving are ruled out.** They change shared state, and the CLI has no such command, so the page can't do them.
- **A per-viewer "seen" marker is legitimate.** Keep a last-visit timestamp in the browser (like the theme choice) and mark rows that became urgent or look since then with a small "new" word and dot. It reads nothing thirdshift doesn't already show, writes nothing to thirdshift, and counters change blindness (§2). It must never hide or demote an item. A seen problem is still a problem until thirdshift's state says otherwise.

## 8. Phone

- **Needs you leads.** Keep it first and open, and wrap each urgent row's "why" line rather than truncating it. On a phone, the reason is the information.
- **Collapse healthy sections.** Turn them into one line of counts, "Ready 2 · Landing 1 · Working 1 · Done 2", each a tap target to unfold. That keeps the first screen for what needs you.
- **Keep the band and edge, drop the dimming.** On a small screen the band and edge carry well. Make the receded colour switch to normal text when the reader taps into a healthy section, since there's no hover on touch.

## 9. What to remove

- **Hue from healthy states in Focus:** working, landing, ready and done go to the grey ramp. Words and glyphs still distinguish them.
- **The glow tokens:** Focus doesn't use glows; leave them neutral to avoid accidental warm halos.
- **The light theme's amber edge colour (#E08A00):** replace it (§1).
- **Infinite alarm loops:** cap them (§6).
- **Any fourth emphasis level:** keep urgent band, look edge, and normal. Bold belongs to urgent only.

## 10. Recommended changes

| Change | Reason | Source | How to verify in the prototype |
|---|---|---|---|
| Focus dark `urgentText` #FF6B78 → #FFB0B5; keep the edge #FF6B78 | Urgent text currently has less perceived contrast than receded text (Lc −48.8 vs −50.6); saturated red vibrates on dark | [APCA](https://git.apcacontrast.com/documentation/APCA_in_a_Nutshell); [Google codelab](https://codelabs.developers.google.com/codelabs/design-material-darktheme) | Re-run the contrast script: urgentText Lc ≈ −71 < dim −50.6; squint test on Focus dark, where urgent rows read first |
| Focus light `look` (edge and fill) #E08A00 → #B06800 | State indicator must be ≥ 3:1 (now 2.51:1) | [WCAG 1.4.11](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html) | Ratio of `--look` to bg ≥ 3:1; the editor's AA check shows ✓ |
| Focus light `urgentText` → #9A1424, `dim` → #6B6B70 | Widen the gap between urgent (Lc 82) and receded (Lc 71.5) while both pass AA | [APCA](https://git.apcacontrast.com/documentation/APCA_in_a_Nutshell); [WCAG 1.4.3](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html) | Contrast script; receded rows visibly lighter than urgent at a squint |
| Healthy states (working, landing, ready, done) to the grey ramp in Focus only | Hue is the pop-out feature; reserve it for the abnormal | [Healey](https://www.csc2.ncsu.edu/faculty/healey/PP/); [Emerson/Movicon on ISA-101](https://resources.ipd.com.au/documents/white-papers/Best-practices-for-high-performance-HMI-design_White%20Paper_ENUS_2023.pdf) | Ready, Landing and Working rows show no hue; only Needs-you rows carry colour |
| Silent and slow escalate in two static steps with words ("past 30", "2× usual"), with a heavier edge and bold age at step 2 | Time context without motion; a persisting problem should rise | [Grafana thresholds](https://grafana.com/docs/grafana/latest/panels-visualizations/configure-thresholds/); [PagerDuty](https://support.pagerduty.com/main/docs/configurable-service-settings); [WCAG 1.4.1](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html) | Set #702's `lastEvent` to `ago(45)` in data.js: its row says "past 30" and has the heavier edge |
| Sort Needs you: urgent by escalation step, then age; then look | The worst first, as alarm lists rank by priority | [ABB summary of EEMUA 191](https://library.e.abb.com/public/3fe0e33c700746d0bcf700526ba81c62/3BDD017132_en_3BDD017132_EN_Alarmmanagement_Check.pdf); [Datadog priority](https://docs.datadoghq.com/ddsql_reference/data_directory/dd/dd.monitor_groups.dataset/) | With the change above, #702 sorts above the 14-minute items |
| Alarm pulse runs 3 times on entering silent, then holds static | Auto-motion over 5s needs a pause | [WCAG 2.2.2](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html) | DevTools: `animation-iteration-count: 3` on `.w-silent .gl`; it doesn't restart on j/k |
| A visible "Pause motion" control (header or Theme panel) | Activity motion is also continuous | [WCAG 2.2.2](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html); [WCAG 2.3.3 / C39](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions.html) | Toggling it stops the spinner, belt and heartbeat; the choice persists per browser |
| Over 7 Needs-you items: a summary line by kind, then "N more"; group the same cause | Floods bury the important; one problem, one item | [ABB/EEMUA](https://library.e.abb.com/public/3fe0e33c700746d0bcf700526ba81c62/3BDD017132_en_3BDD017132_EN_Alarmmanagement_Check.pdf); [Google SRE](https://sre.google/sre-book/monitoring-distributed-systems/); ISA-18.2 flood threshold **[unverified]** | Add 10 failing Commands to data.js: 7 rows show, then "3 more" and a summary line |
| "New since you last looked" word and dot, per browser, never hiding | Counters change blindness without a write to thirdshift | [Healey on change blindness](https://www.csc2.ncsu.edu/faculty/healey/PP/) | Reload with a stored last-visit before c2's silence: #702 shows "new"; nothing is hidden |
| Phone: healthy sections as one count line; urgent "why" lines wrap | The first screen is for what needs you | [NN/g](https://www.nngroup.com/articles/visual-hierarchy-ux-definition/) | phone.html?only=console in Focus: Needs you is fully readable and healthy sections take one line |
| Keep three emphasis levels only (urgent band, look edge, normal); bold only on urgent | "If everything is contrasted, then nothing stands out" | [NN/g](https://www.nngroup.com/articles/visual-hierarchy-ux-definition/) | Audit the CSS: no bold outside urgent rows in Focus |
