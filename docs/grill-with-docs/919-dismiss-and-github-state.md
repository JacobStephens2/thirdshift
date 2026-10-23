# Grilling: #919, dismissing Inbox items, and reading GitHub's state

Issue: https://github.com/JacobStephens2/thirdshift/issues/919
Research: #1083 (Ready for review never empties), #711's Q37 in [711-live-runs.md](711-live-runs.md), [ADR-0016](../adr/0016-the-console-is-a-long-running-reader.md) and its #698 Q6.9 amendment, [focus-theme-attention.md](../research/focus-theme-attention.md), #739 (GitHub quota), #1142 (one Console for every machine)

## Round 1 (open)

**Q1.1 - #1083 first.** #1083, labelled `ready-for-agent`, gives Ready for review the two exits Needs you already has: a ready Command leaves once a later Command on its issue starts on the machine, or once it ended more than `inbox.days` ago. It leaves GitHub out on purpose. Today Ready for review never empties (see Research results). The options:
- (a) **#1083 stays its own issue and lands first.** #919 adds its exits on top. If Q1.3 decides to read GitHub, #919 may revise #1083's age limit, so that a pull request GitHub still shows as open stays in Ready for review however old it is.
- (b) **Fold #1083 into #919's Spec,** so one Spec sets every way out of Ready for review at once.

*Recommended:* (a). #1083 is small, ready, and right whatever #919 decides. Folding it in would hold back the fix for a group that never empties until #919 is grilled, specced and built.

*Answer:* 

**Q1.2 - What the person does.** The issue says "dismiss / acknowledge". Alarm management (ISA-18.2) uses the two words for different things: acknowledging an alarm that is still active keeps it on screen but stops it demanding attention, while acknowledging one whose condition has passed clears it. In the Inbox, Silent and Slow are live problems. Everything else in Needs you, and every Ready for review row, has ended. The options:
- (a) **One action, Dismiss, on ended items only.** A dismissed item leaves Needs you or Ready for review. Silent and Slow can't be dismissed: each leaves by itself once its Run logs again or its stage ends, and Stop is there for a stuck one.
- (b) **(a), plus Acknowledge for live problems.** An acknowledged Silent or Slow Run stays in Needs you but no longer counts in the tab title, the urgent icon or browser notifications, until its health changes.
- (c) **Acknowledge only.** Nothing leaves. An acknowledged item stops counting and sinks to the bottom of its group.

*Recommended:* (a). Every example in the issue is an ended item that asks nothing more, so it should go, not linger greyed out. "New since you last looked" already marks what you've seen, per browser. (b) doubles the commands and rules to quiet something that ends by itself. If one long Spec run keeps showing Slow, that's a question about its Usual range, not something to acknowledge away.

*Answer:* 

**Q1.3 - Reading GitHub.** #711's Q37 kept the Inbox local: nothing is read from GitHub, and "the Console reading GitHub on every refresh" was turned down. Since then, the usual stale row has turned out to be one GitHub already knows is done. #1083 lists three pull requests merged by hand still in Ready for review the next day. The issue's own example, #88, failed because PR #789 had been merged, the issue closed two seconds after that merge, and the row still sat in Needs you 43 hours later. The options:
- (a) **Yes: the Inbox learns from GitHub that the Day shift acted.** A ready Command leaves Ready for review once its pull request is merged or closed. An ended problem leaves Needs you once its issue is closed. Dismissal covers what GitHub can't see, such as a Command that died on an issue that stays open.
- (b) **No: the Inbox stays local.** Dismissal and #1083's exits are the only new ways out. You'd merge on GitHub, then dismiss in the Console.
- (c) **GitHub informs, you decide.** The row shows "merged on GitHub" but stays until you dismiss it. This still reads GitHub, but never lets GitHub move a row.

*Recommended:* (a). Saying again in the Console what you just did on GitHub is the chore #919 is about. The cost is real but small. GitHub has to be read on its own schedule and cached, since the Console re-renders the Inbox every second, and #739 shows the GraphQL quota can run out, so the research below is measuring what one batched lookup costs. It also amends ADR 0016, which says the Console reads only what each Command writes about itself. Which GitHub facts count, how often GitHub is read, and what the Inbox does when GitHub can't be reached are for Round 2, once that research reports.

*Answer:* 

**Q1.4 - Where a dismissal is kept.** Whatever Q1.2 calls it, the judgment has to be written where `thirdshift status` and the Console both read it. In #711 you weighed a folder of Console state in `~/.thirdshift`, and were wary of adding a new kind of state to the machine. The options:
- (a) **Beside the Command record,** as a Stop request already is: a small file next to the record, saying who dismissed it, when, and whether from the CLI or the Console. It stays with the Command's other files on that machine, and the Inbox reads it the way it reads a Stop request today.
- (b) **One store per machine,** such as a file or folder under `~/.thirdshift/console/`, listing every dismissed Command.
- (c) **On GitHub,** as a label or comment, so every machine would see it.

*Recommended:* (a). It isn't a new kind of state: the Inbox already reads a file beside each record to find Triage items. A dismissal belongs to one Command, so it should live and go with that Command's files, on the machine whose Console shows it. (c) would write to GitHub, and notify the issue's watchers, about a judgment that only concerns one machine's Inbox.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- **A Console dismissal is already allowed.** ADR 0016's amendment of 2026-10-21 (#698 Q6.9) lets the Console record judgments only a person makes, and names "dismissing an item in Needs you (#919)". Whatever Q1.2 decides is also a `thirdshift` command with `--json`, a test pairs the two, and it never starts work.
- **The Activity log records each dismissal,** naming who asked and whether from the CLI or the Console, as it does for a Stop. The Console log stays for sign-ins and other account events.
- **Each Console dismisses only its own machine's Commands,** since each machine serves its own Console. One Console for every machine is #1142.
- **`thirdshift status`, `status --json` and the Console Inbox keep agreeing on every row's group,** as #711's Q39 and #1083 require. Anything that moves a row, whether a dismissal or a fact from GitHub, moves it in all three.
- **A row that leaves Needs you also leaves the tab title's count, the urgent icon and browser notifications,** since all three count the rendered Needs you rows.
- **The glossary needs rewriting.** Its Needs you entry still says "Nothing else clears it, since the Console does nothing the CLI can't", which ADR 0016's amendment has overtaken. It's rewritten once Q1.2 names the action. "Alerts", the issue's word, is one the glossary avoids for Needs you, so this file says "items" or "rows".
- **#917 decides where the button sits.** Its parked layout work (one-line rows and the peek) owns how the Inbox looks. This grill decides what the action does.

## Research results (2026-10-23)

- **Ready for review never empties.** `src/logs/reading/inbox.rs:209-212` puts every ready Command there with no age limit and no later-Command exit, so its rows outlive `inbox.days`. #1083 is the fix, and it leaves GitHub out of scope. Confirmed by source and #1083.
- **An ended problem leaves Needs you only two ways:** a later Command on the same issue starts on the machine, or it's older than `inbox.days`, 7 by default (`src/logs/reading/inbox.rs:199-207`). Confirmed by source.
- **A Finding never notices triage.** Its Needs you row comes from the record's `security_findings_waiting`, set once when the Command ends (`src/run_ending.rs:85`, `src/logs/reading/inbox.rs:188-190`). The glossary says a finding stops waiting once the Day shift triages it on GitHub, so a triaged finding stays in Needs you for up to 7 days. Confirmed by source and `GLOSSARY.md`.
- **The issue's example.** #88 closed as completed at 03:20:29 UTC on 2026-10-19, two seconds after PR #789 merged. The Pickup run that failed on it, "PR … is merged, not open", had started at 02:20 UTC. Confirmed by `gh` and the issue.
- **The Console re-renders the Inbox every second** to find changes to push to open pages (`src/console/live.rs:17-19`). Reading GitHub at render time would mean 3,600 reads an hour, so GitHub needs its own pace and a cache. Confirmed by source.
- **Stop keeps its request beside the Command record,** as `<record>.stop.json` (`src/logs/stop.rs:43-49`), and the Inbox reads that file to find Triage items (`src/logs/reading/inbox.rs:180`). Confirmed by source.
- **The tab title, the urgent icon and browser notifications all count rendered Needs you rows** (`src/console/inbox.html:8-9`, `src/console/assets/notifications.js:41`). Confirmed by source.
- **Why acknowledging was ruled out before.** #711's research (`docs/research/focus-theme-attention.md:39`, `:81`, `:129`) ruled out acknowledging and shelving only because the CLI had no such command. It notes that alarm systems shelve a nuisance alarm with a time-out and an audit trail. Confirmed by docs.
- **GitHub quota.** #739 records the GraphQL quota running out on 2026-10-17, with 15 jobs a minute across seven repositories. Today `gh api rate_limit` showed 4,501 of 5,000 GraphQL points left. Confirmed by #739 and `gh`.

## Waiting on research

- An agent is finding out how thirdshift calls GitHub (one `gh` call per item, or batched GraphQL), what one batched lookup of several pull requests and issues costs in quota, what the Command record keeps about its pull request, issue and findings, how a Security run decides a finding still waits, how the Console's Stop action is authorized and recorded, and what each live machine's Needs you and Ready for review hold today compared with GitHub. For Round 2: which GitHub facts count, how often GitHub is read, Findings, and the dismissal's command.
