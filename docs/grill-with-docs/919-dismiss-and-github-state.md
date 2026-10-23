# Grilling: #919, dismissing Inbox items, and reading GitHub's state

Issue: https://github.com/JacobStephens2/thirdshift/issues/919
Research: #1083 (Ready for review never empties), #711's Q37 in [711-live-runs.md](711-live-runs.md), [ADR-0016](../adr/0016-the-console-is-a-long-running-reader.md) and its #698 Q6.9 amendment, [focus-theme-attention.md](../research/focus-theme-attention.md), #739 (GitHub quota), #1142 (one Console for every machine)

## Round 1 (answered)

**Q1.1 - #1083 first.** #1083, labelled `ready-for-agent`, gives Ready for review the two exits Needs you already has: a ready Command leaves once a later Command on its issue starts on the machine, or once it ended more than `inbox.days` ago. It leaves GitHub out on purpose. Today Ready for review never empties (see Research results). The options:
- (a) **#1083 stays its own issue and lands first.** #919 adds its exits on top. If Q1.3 decides to read GitHub, #919 may revise #1083's age limit, so that a pull request GitHub still shows as open stays in Ready for review however old it is.
- (b) **Fold #1083 into #919's Spec,** so one Spec sets every way out of Ready for review at once.

*Recommended:* (a). #1083 is small, ready, and right whatever #919 decides. Folding it in would hold back the fix for a group that never empties until #919 is grilled, specced and built.

*Answer:* agree

**Q1.2 - What the person does.** The issue says "dismiss / acknowledge". Alarm management (ISA-18.2) uses the two words for different things: acknowledging an alarm that is still active keeps it on screen but stops it demanding attention, while acknowledging one whose condition has passed clears it. In the Inbox, Silent and Slow are live problems. Everything else in Needs you, and every Ready for review row, has ended. The options:
- (a) **One action, Dismiss, on ended items only.** A dismissed item leaves Needs you or Ready for review. Silent and Slow can't be dismissed: each leaves by itself once its Run logs again or its stage ends, and Stop is there for a stuck one.
- (b) **(a), plus Acknowledge for live problems.** An acknowledged Silent or Slow Run stays in Needs you but no longer counts in the tab title, the urgent icon or browser notifications, until its health changes.
- (c) **Acknowledge only.** Nothing leaves. An acknowledged item stops counting and sinks to the bottom of its group.

*Recommended:* (a). Every example in the issue is an ended item that asks nothing more, so it should go, not linger greyed out. "New since you last looked" already marks what you've seen, per browser. (b) doubles the commands and rules to quiet something that ends by itself. If one long Spec run keeps showing Slow, that's a question about its Usual range, not something to acknowledge away.

*Answer:* agree, except dismiss can be used for silent and slow - everything that can go into ready for review or needs you can be dismissed - giving more manual control to the console operator.

*Checked:* A Silent or Slow row is a live Command whose own Run, or one of its dispatched or child Runs, has the problem (`src/logs/reading/inbox.rs:262-277`). Once dismissed it sits in Working or Landing, and its row there still says "silent 12 min" or "slower than usual (…), still working" (`src/logs/reading/health.rs:118-134`, shown by `src/logs/status/inbox.rs:335`). What brings it back is Q2.1.

**Q1.3 - Reading GitHub.** #711's Q37 kept the Inbox local: nothing is read from GitHub, and "the Console reading GitHub on every refresh" was turned down. Since then, the usual stale row has turned out to be one GitHub already knows is done. #1083 lists three pull requests merged by hand still in Ready for review the next day. The issue's own example, #88, failed because PR #789 had been merged, the issue closed two seconds after that merge, and the row still sat in Needs you 43 hours later. The options:
- (a) **Yes: the Inbox learns from GitHub that the Day shift acted.** A ready Command leaves Ready for review once its pull request is merged or closed. An ended problem leaves Needs you once its issue is closed. Dismissal covers what GitHub can't see, such as a Command that died on an issue that stays open.
- (b) **No: the Inbox stays local.** Dismissal and #1083's exits are the only new ways out. You'd merge on GitHub, then dismiss in the Console.
- (c) **GitHub informs, you decide.** The row shows "merged on GitHub" but stays until you dismiss it. This still reads GitHub, but never lets GitHub move a row.

*Recommended:* (a). Saying again in the Console what you just did on GitHub is the chore #919 is about. The cost is real but small. GitHub has to be read on its own schedule and cached, since the Console re-renders the Inbox every second, and #739 shows the GraphQL quota can run out, so the research below is measuring what one batched lookup costs. It also amends ADR 0016, which says the Console reads only what each Command writes about itself. Which GitHub facts count, how often GitHub is read, and what the Inbox does when GitHub can't be reached are for Round 2, once that research reports.

*Updated 2026-10-23:* The research is in, and its numbers back (a). The two machines' Inboxes hold 58 rows in Needs you and Ready for review today. GitHub already shows 36 of them done, from what their records hold: 12 by a merged pull request, and 24 more by a closed issue. The issue is the stronger signal, because a failed Run's record often leaves its pull request out. Of the other 22 rows, 8 are Architect runs with no issue recorded, which name a merged pull request or a closed issue only in their outcome text. The last 14 have nothing GitHub can clear:
- 5 harness billing failures (`402 Payment Required`, or a spend limit);
- 1 Security run that failed for another reason;
- 3 Security runs whose findings really are still waiting;
- 4 failures whose pull request and issue are both still open;
- 1 live Slow Run.

The billing failures and the other failed Security run are what dismissal is for. GitHub can't say they're settled, so they stay until someone says so or they age out. One GraphQL query that looked up 46 pull requests and issues across 10 repositories cost 1 point of the 5,000 an hour, so reading a whole Inbox once a minute would cost about 60 an hour.

*Answer:* agree

*Checked:* The issue's example is common. 13 rows in Needs you today, 7 on `thirdshift` and 6 on `stephens`, are Runs that failed with "PR … is merged, not open": the pull request was merged while the Run was still going. Delivery ends such a Run as merged when its worktree holds nothing the merge left out (`src/delivery.rs:174-184`), so each of these had work the merge didn't take. All 13 issues are closed, so Q1.3's rule takes every one of them out.

*Checked:* The 14 rows above counted only each Command's own issue and pull request. An Architect run or Security run has neither, but the Run it dispatched does: its plan, or its fix. Counting those, two of the five billing failures, clave's and bountywell's, left a plan that is still open and still `in-progress`, so they wait on you for more than a billing problem. The failed Security run set aside for dismissal dispatched a fix whose issue is closed, so GitHub would take it out. That leaves three billing failures with nothing on GitHub: chart35's 48 Architect runs and horarium's, which failed before planning anything, and the thirdshift Architect run with no record. The three Security runs whose findings wait are Q2.8's. Details are in Research results.

**Q1.4 - Where a dismissal is kept.** Whatever Q1.2 calls it, the judgment has to be written where `thirdshift status` and the Console both read it. In #711 you weighed a folder of Console state in `~/.thirdshift`, and were wary of adding a new kind of state to the machine. The options:
- (a) **Beside the Command record,** as a Stop request already is: a small file next to the record, saying who dismissed it, when, and whether from the CLI or the Console. It stays with the Command's other files on that machine, and the Inbox reads it the way it reads a Stop request today.
- (b) **One store per machine,** such as a file or folder under `~/.thirdshift/console/`, listing every dismissed Command.
- (c) **On GitHub,** as a label or comment, so every machine would see it.

*Recommended:* (a). It isn't a new kind of state: the Inbox already reads a file beside each record to find Triage items. A dismissal belongs to one Command, so it should live and go with that Command's files, on the machine whose Console shows it. (c) would write to GitHub, and notify the issue's watchers, about a judgment that only concerns one machine's Inbox.

*Updated 2026-10-23:* (a) can't dismiss a row that has no Command record. Those are History rows, rebuilt from the Activity log for Commands that predate records: 11 of the 19 rows on `thirdshift` today and 16 of the 39 on `stephens`. Each is at least 5.7 days old, since every newer Command has a record. The ones in Needs you age out within days, and the ones in Ready for review will once #1083 lands. Only (b) could dismiss them, by the row's id. The recommendation stands.

*Answer:* agree

*Checked:* A Ready for review row can be a dispatched Run, which has no record of its own: it lives inside its Pickup run's or Architect run's record, and the Inbox gives it that record's path (`src/logs/reading.rs:263-276`). So a dismissal kept beside a record has to say which of the record's rows it covers.

## Round 2 (open)

**Q2.1 - What brings a dismissed row back.** Q1.2 lets you dismiss any row in Needs you or Ready for review, live ones included, and a live Command can go on to do something new. A failed Security run also hides its waiting findings behind its failure (see Research results). The options:
- (a) **A kind of row it wasn't when dismissed.** A Run dismissed as Slow comes back if it goes Silent, and any dismissed live Command comes back if it then dies, fails or ends ready for review. A failed Security run whose findings still wait comes back as a Finding. Each kind counts once: a Run dismissed while Silent stays dismissed through later silences, though it comes back if it turns Slow.
- (b) **Nothing.** A dismissed Command never returns to Needs you or Ready for review, whatever it does next.
- (c) **Each new occurrence,** as with an acknowledged alarm: also a Run that logs again and later falls silent, or one that is Slow again in a later stage.
- (d) **A time limit,** as with a shelved alarm: a dismissed live problem comes back after an hour if it's still there.

*Recommended:* (a). You judged what you saw, and something new needs a new look. A Run that dies after you dismissed it as Slow is the row Needs you most needs to show, and (b) would hide it. (c) would bring a long Spec run back at every stage that runs past its Usual range. (d) brings rows back on a clock, whatever the Run did. A second silence isn't hidden under (a) either: the Run's row in Working says "silent 12 min".

*Answer:* 

**Q2.2 - A row that stands for many Commands.** The Inbox folds Failed rows with the same cause into one. On `stephens`, one row stands for 48 chart35 Architect runs and another for 7. On `thirdshift`, #972's stands for 10. The options:
- (a) **Dismissing the row dismisses every Command it stands for.** A later Command that fails the same way starts the row again.
- (b) **Dismissing the row dismisses only its newest Command,** and the next one takes its place.
- (c) **Dismissing the row dismisses its cause,** so later failures with that cause stay out of Needs you too.

*Recommended:* (a). The row is what you see and judge, so it's what you dismiss. (b) would take 48 dismissals to clear one row. (c) is a mute: a billing failure that comes back after you dismissed it is news that the factory is still failing for that reason, and worth hearing until it's fixed.

*Answer:* 

**Q2.3 - Which GitHub facts move a row.** Q1.3 settled two: a ready Command leaves Ready for review once its pull request is merged or closed, and an ended problem leaves Needs you once its issue is closed. Three cases sit outside those words:
- An Architect run or Security run records no issue of its own. The Run it dispatched does: its plan, or its fix.
- A Triage row asks for a triage decision, after which the issue may stay open, as `ready-for-human` for instance.
- A failed Run whose pull request you close without merging still holds its issue's Claim until you relabel it.

The options:
- (a) **Finished work, in either group, and triage.** A row leaves once GitHub shows its issue closed or its pull request merged, counting an Architect run's or Security run's dispatched Run as its own. A ready row also leaves once its pull request is closed, as Q1.3 says. A Triage row also leaves once its issue no longer has `needs-triage`. GitHub's latest state counts, so a reopened issue brings its row back. GitHub is read only for rows with a Command record.
- (b) **Q1.3's two facts only,** for the Command's own issue and pull request.

*Recommended:* (a). A merged pull request is finished work whether or not its issue closed with it. The dispatched Run is the only way GitHub can tell anything about most Architect and Security rows: on `stephens` it shows two of them finished, and four plans or fixes still open and claimed (see Research results). A pull request closed unmerged leaves a failed Run's issue claimed, so that row should stay until you relabel or close the issue. Once `needs-triage` is off, a Triage row's question is answered. Rows with no record are at least six days old and age out within days, so reading GitHub for them would buy little. Findings are Q2.8.

*Answer:* 

**Q2.4 - Who reads GitHub, and how often.** Q1.3 settled that GitHub is read on its own schedule and kept, never on the Console's one-second render. Both machines' Consoles run as systemd services that can find `gh`, though nothing in the Console calls it today. Pickup runs, whose Sweep already reads GitHub, start every 2 minutes on `thirdshift` and every 10 on each of `stephens`'s 18 repositories, but none sweeps while a Run holds its repository. The options:
- (a) **The Console, once a minute.** A background task looks up every row's issue and pull request in one batched query and keeps the answer in a file on the machine. `thirdshift status` reads the same file, and asks GitHub itself first when no Console has refreshed it in the last few minutes, so the two always show the same groups. About 60 GraphQL points an hour per machine.
- (b) **Each Pickup run's Sweep.** It looks up its repository's rows and writes what GitHub says beside each record, and the Console goes on reading only local files. A row moves within one Pickup interval, but not while a Run holds its repository, and never on a repository with no Pickup line.
- (c) **Whatever shows the Inbox,** `thirdshift status` or the Console, asks GitHub when the kept answer is more than a minute old, with no background task.
- (d) **GitHub webhooks.** GitHub tells the Console the moment a pull request merges or an issue closes. Each repository needs a webhook with a shared secret, the Console needs a route open to anyone on the internet, and deliveries missed while it restarts still need a poll to catch up.

*Recommended:* (a). The issue's complaint is a pull request you merged by hand still sitting in Ready for review. Under (b) that's likeliest while the factory is busy on the same repository, which is exactly when its Pickup runs skip. (c) makes `status` wait on GitHub whenever it runs more than a minute after the last read. (d) is the most live, but needs setting up on 19 repositories, opens a new door into the Console, and still needs (a)'s poll. (a) changes what ADR 0016 says the Console reads; see Q2.9.

*Answer:* 

**Q2.5 - Whether age still moves a row GitHub says is waiting.** Today an ended problem leaves Needs you after `inbox.days`, 7 by default, and #1083 gives Ready for review the same limit. Age stands in for knowing whether the Day shift has acted. With GitHub read, it can become the fallback. The options:
- (a) **No, in either group.** A row whose issue or pull request GitHub shows still open stays until GitHub shows it finished, a later Command starts on its issue, or you dismiss it. Age moves only the rows GitHub can't speak for: those with no issue or pull request at all, such as an Architect run whose review failed before it planned anything; rows with no record; and any row GitHub hasn't been read for.
- (b) **Only in Ready for review.** An open pull request keeps its row there however old, and Needs you keeps its age limit.
- (c) **Yes, as today and in #1083.**

*Recommended:* (a). An open pull request, or a failure whose issue is still open and claimed, still waits on you however old it is, and an Inbox that hides it after a week can't be trusted to be complete. It would have kept no extra row so far: the 13 failures that aged out of Needs you with an issue recorded all have that issue closed now. clave's and bountywell's open, claimed plans are the kind it would keep. Dismissal handles the ones you mean to leave. This narrows #1083's age limit, as Q1.1 allowed.

*Answer:* 

**Q2.6 - The command, and taking a dismissal back.** ADR 0016 wants a CLI command with `--json` for every Console action. `stop` and `show` name a Command as `repo#number`, an Issue URL, a Pass's `kind-stamp`, or a record's path. The options:
- (a) **`thirdshift dismiss <command> [--json]`, with `--undo` to take it back.** It names Commands as `stop` does, and dismisses the whole row its Command is in (Q2.2). It asks for no reason. It refuses, saying why, a row outside Needs you and Ready for review, and one with no record. The Console offers both, and the Activity log records both.
- (b) **The same, with no undo.** A dismissal is final, and the row stays visible in Done or Resolved, saying who dismissed it.

*Recommended:* (a). A misclick in the Console shouldn't hide a problem for good, and taking a dismissal back is cheap: the dismissal beside the record goes, and the Activity log says who took it back.

*Answer:* 

**Q2.7 - Where a row goes once it leaves.** Whether you dismissed it or GitHub moved it. The options:
- (a) **Where it would be without the problem, saying why it left.** A live Command goes to Working or Landing. An ended one goes to Done if it ended in the last day, then Resolved, then History, by the usual rules. Its row says "dismissed by <who>", or what GitHub showed, such as "pull request merged".
- (b) **A new group, Dismissed,** after Resolved.
- (c) **Straight to History.**

*Recommended:* (a). The groups already sort by how recent, and Resolved already means "older, with nothing waiting", which is what a dismissal says. The reason on the row keeps a dismissed failure in Done from reading as a success. (b) adds an eighth group. (c) hides what you did a minute ago.

*Answer:* 

**Q2.8 - Findings and GitHub.** A Finding row has no issue or pull request of its own. The glossary says a finding stops waiting once the Day shift triages it on GitHub, but the Inbox never looks, so a triaged finding stays in Needs you until it ages out or is dismissed. The check a Security run makes before it starts, whether any finding still waits, could tell from one list of the repository's findings (`src/github/advisories.rs:500-515`). The options:
- (a) **A Finding leaves once none of its findings still waits,** by the Security run's own check, on the same schedule as the rest: one list per repository with a Finding row, from the REST quota rather than GraphQL.
- (b) **Dismissal and age only.**

*Recommended:* (a). It makes the Inbox agree with the glossary, and a Finding has nothing else GitHub can tell it by. It reads only what the Security run already reads, and the row shows what it shows today.

*Answer:* 

**Q2.9 - Writing down the reversal.** Reading GitHub reverses #711's Q37 and changes what ADR 0016 says the Console reads. It would be hard to undo once the Inbox depends on it, a reader of ADR 0016 wouldn't expect it, and it was a real trade-off, so it meets the bar for an ADR. The options:
- (a) **A new ADR, 0022,** written once Q2.4 is answered, with a line in ADR 0016 pointing to it.
- (b) **An amendment to ADR 0016,** as #711's Q46 and #698's Q6.9 were.
- (c) **No ADR.** This file and the Spec are enough.

*Recommended:* (a). It decides where the Inbox's facts come from, with costs of its own (quota, staleness, what happens when GitHub can't be reached), not how the Console runs. Its own ADR has room for the options weighed here, where ADR 0016's amendments are one-line permissions.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- **A Console dismissal is already allowed.** ADR 0016's amendment of 2026-10-21 (#698 Q6.9) lets the Console record judgments only a person makes, and named "dismissing an item in Needs you (#919)". It now says Needs you or Ready for review (Q1.2). Dismissal is also a `thirdshift` command with `--json`, a test pairs the two, and it never starts work.
- **The Activity log records each dismissal,** naming who asked and whether from the CLI or the Console, as it does for a Stop. The Console log stays for sign-ins and other account events.
- **Any signed-in Console account may dismiss, as any may Stop,** behind the same session and CSRF checks. The account's email is the name the Activity log gives.
- **Each Console dismisses only its own machine's Commands,** since each machine serves its own Console. One Console for every machine is #1142.
- **`thirdshift status`, `status --json` and the Console Inbox keep agreeing on every row's group,** as #711's Q39 and #1083 require. Anything that moves a row, whether a dismissal or a fact from GitHub, moves it in all three.
- **A row that leaves Needs you also leaves the tab title's count, the urgent icon and browser notifications,** since all three count the rendered Needs you rows. A move GitHub causes reaches open pages like any other change, through the one-second observer.
- **#1083 lands first, as its own issue** (Q1.1). It stays `ready-for-agent` and unchanged. #919 builds on its two ways out of Ready for review, and Q2.5 decides whether GitHub narrows its age limit.
- **Dismissal is the only new action** (Q1.2). There is no Acknowledge: nothing stays in Needs you but stops counting.
- **Every row in Needs you or Ready for review can be dismissed, live ones included** (Q1.2). Rows in Landing, Working, Done, Resolved and History can't. A Landing row's "see Needs you" line points to the row to dismiss.
- **A dismissal changes only the Inbox.** It never stops, starts or changes a Command, its Runs, its issue, its labels, its Claim or its pull request, and it never writes to GitHub (Q1.4). A dismissed Silent Run keeps running, and Stop still ends it.
- **A dismissal is kept beside its Command's record** (Q1.4), saying who, when, from the CLI or the Console, and which of the record's rows it covers, since a dispatched Run shares its Pickup run's record.
- **A row with no record can't be dismissed** (Q1.4). These are Commands from before records began on 2026-10-19, rebuilt from the Activity log. Round 1 called them History rows, but they're not the History group, so from Round 2 this file calls them rows with no record. They leave by age: those in Needs you within a day, those in Ready for review once #1083 lands.
- **GitHub is read in batches, on its own schedule, and kept,** never at the Console's one-second render (Q1.3). Q2.4 decides who reads it. What the Inbox does when GitHub can't be read, and how it gives way to the factory's quota (#739), wait on that answer.
- **The issue's own kind of row leaves by itself.** All 13 rows that failed with "PR … is merged, not open" have closed issues (Q1.3's first check). Whether such a Run should count as failed at all is outside #919.
- **The glossary is rewritten.** It now has **Dismissal**, and its Console, Inbox, Needs you and Activity log entries mention it. The Needs you entry no longer says "Nothing else clears it". "Alerts", the issue's word, is one the glossary avoids for Needs you, so this file says "items" or "rows".
- **#917 decides where the button sits.** Its parked layout work (one-line rows and the peek) owns how the Inbox looks. This grill decides what the action does.

## Research results (2026-10-23)

- **Ready for review never empties.** `src/logs/reading/inbox.rs:209-212` puts every ready Command there with no age limit and no later-Command exit, so its rows outlive `inbox.days`. #1083 is the fix, and it leaves GitHub out of scope. Confirmed by source and #1083.
- **An ended problem leaves Needs you only two ways:** a later Command on the same issue starts on the machine, or it's older than `inbox.days`, 7 by default (`src/logs/reading/inbox.rs:199-207`). Confirmed by source.
- **A Finding never notices triage.** Its Needs you row comes from the record's `security_findings_waiting`, set once when the Command ends (`src/run_ending.rs:85`, `src/logs/reading/inbox.rs:188-190`). The glossary says a finding stops waiting once the Day shift triages it on GitHub, so a triaged finding stays in Needs you for up to 7 days. Confirmed by source and `GLOSSARY.md`.
- **The issue's example.** #88 closed as completed at 03:20:29 UTC on 2026-10-19, two seconds after PR #789 merged. The Pickup run that failed on it, "PR … is merged, not open", had started at 02:20 UTC. Confirmed by `gh` and the issue.
- **The issue's example, 13 times.** 13 rows in Needs you, 7 on `thirdshift` and 6 on `stephens`, failed with "PR … is merged, not open". Delivery ends a Run whose pull request was merged under it as merged when its worktree holds nothing the merge left out (`src/delivery.rs:174-184`), so each of these had work the merge didn't take. All 13 issues are closed. Confirmed by source, `status --json` and one GraphQL query.
- **The Console re-renders the Inbox every second** to find changes to push to open pages (`src/console/live.rs:17-19`). Reading GitHub at render time would mean 3,600 reads an hour, so GitHub needs its own pace and a cache. Confirmed by source.
- **Stop keeps its request beside the Command record,** as `<record>.stop.json` (`src/logs/stop.rs:43-49`), and the Inbox reads that file to find Triage items (`src/logs/reading/inbox.rs:180`). Confirmed by source.
- **A dispatched Run has no record of its own.** It lives inside its Pickup run's or Architect run's record, and the Inbox gives it that record's path (`src/logs/reading.rs:263-276`). Confirmed by source.
- **The tab title, the urgent icon and browser notifications all count rendered Needs you rows** (`src/console/inbox.html:8-9`, `src/console/assets/notifications.js:41`). Confirmed by source.
- **A Silent or Slow Run's row says so in any group.** Its line reads "silent 12 min" or "slower than usual (…), still working" (`src/logs/reading/health.rs:118-134`), wherever the row sits (`src/logs/status/inbox.rs:335`). Confirmed by source.
- **Why acknowledging was ruled out before.** #711's research (`docs/research/focus-theme-attention.md:39`, `:81`, `:129`) ruled out acknowledging and shelving only because the CLI had no such command. It notes that alarm systems shelve a nuisance alarm with a time-out and an audit trail. Confirmed by docs.
- **GitHub quota.** #739 records the GraphQL quota running out on 2026-10-17, with 15 jobs a minute across seven repositories. Today `gh api rate_limit` showed 4,501 of 5,000 GraphQL points left. Confirmed by #739 and `gh`.
- **What the two Inboxes hold today.** `thirdshift` has 17 rows in Needs you and 2 in Ready for review. `stephens` has 28 and 11. Per machine:
  - `thirdshift`: 16 of 19 rows are done on GitHub, 4 by the recorded pull request and 12 by the issue. #981's live Slow Run and the failed Spec runs on #977 and #978, whose pull requests are open, rightly remain.
  - `stephens`: 20 of 39 are done on GitHub by recorded fields, 8 by pull request and 12 by issue. 8 more Architect runs name a merged pull request or closed issue only in their text. 11 remain: 5 harness billing failures, 1 Security run that failed for another reason, 3 Security runs with findings still waiting (two of which also failed on billing), and 2 failures whose work is still open (a fix on thereish and a Base fix on drome).
  - Ready for review on `stephens` still holds six Architect runs 18 days old and chart35#230 at 15 days. All their pull requests are merged.

  Confirmed by `thirdshift status --json` on each machine and one GraphQL query per machine.
- **Architect and Security rows can be judged by the Run they dispatched.** Such a row's record holds no issue, but its dispatched Run's does, and the Inbox already reads dispatched Runs' issues to find later Commands (`src/logs/reading/inbox.rs:293-323`). On `stephens`, six of these rows dispatched a Run. chart35's row of 7 (plan closed, pull request merged) and the Security fix voice-agent-scheduling#37 (closed) are finished. The plans clave#45, bountywell#12 and drome#7, and the fix thereish#269, are open and still `in-progress`. The rows that dispatched nothing are the billing failures from chart35's 48 Architect runs and from horarium, and three Security runs. Confirmed by `status --json` and one GraphQL query.
- **No Command record is old enough to age out yet.** Records began on 2026-10-19, at 02:20 UTC on `thirdshift` and 03:21 UTC on `stephens`, and the first reaches seven days on 2026-10-26. Every row that has aged out so far is one with no record. Confirmed by `status --all --json`.
- **What age has taken out of Needs you so far.** On the two machines, 13 failures with an issue recorded aged out with no later Command on that issue, and every one of those issues is closed now. Six more on `thirdshift`, five Architect runs and a Security run, have no issue. Confirmed by `status --all --json` and one GraphQL query.
- **One GraphQL query covers a whole Inbox for 1 point.** Queries that looked up 5, 20, 34 and 46 pull requests and issues by alias each cost 1 point; the last spanned 10 repositories. Confirmed by `rateLimit { cost }` in each query.
- **thirdshift reads GitHub one item at a time today.** Every call is a `gh` subprocess from `src/github.rs`. GraphQL is used only for one issue's sub-issues, blockers and timeline (`src/github.rs:220-234`). Nothing batches several pull requests or issues, nothing caches, and rate-limit errors are recognised but not waited out (`src/github/check_read.rs:37-43`; that's #739). Confirmed by source.
- **Both Consoles can call `gh`, and neither does today.** Each runs as a systemd service with only `HOME` set, and `gh` is at `/usr/bin/gh`, on systemd's default `PATH`. Nothing under `src/console` calls GitHub. Confirmed by `systemctl show` and `command -v` on each machine, and by source.
- **The two machines' schedules.** `thirdshift` starts a Security run and an Architect run every minute and a Pickup run every 2 minutes, all on thirdshift. `stephens` starts a Pickup run every 10 minutes on each of 18 repositories and every 2 on thirdshift, and a Security run once a night on 12 of them. It has no Architect line. Confirmed by `thirdshift cronjobs --json` on each machine.
- **The Command record keeps the pull request's URL and the issue's number, URL and title, never their state** (`src/logs/command_record.rs:30-70`). Six records whose Run failed with "PR … is merged, not open" have no pull request recorded at all (#88 on `thirdshift`, and #701, #765, #786, #793 and #803 on `stephens`). A rule keyed on the recorded pull request misses them; one keyed on the issue catches them. Confirmed by source and `status --json`.
- **A failed Security run hides its waiting findings.** In the Inbox, Failed outranks Finding (`src/logs/reading/inbox.rs:178-192`), so no row on either machine shows as a Finding. The three Security runs on `stephens` whose findings GitHub confirms are still waiting all show as Failed. Dismissing one of those failures would hide its findings too. Confirmed by source, `status --json`, and the findings' state on GitHub. Only counts were taken; no finding is named here.
- **The Security run's own check could tell the Inbox whether a finding still waits.** `waiting_for_day_shift` (`src/github/advisories.rs:115-145`) is a pure check over the finding records. Loading them takes one paginated REST list per repository: its advisories in every state or, on a private repository without them, its `security-finding` issues (`src/github/advisories.rs:500-515`). It also takes one read per fix Ticket. The record's own comment says "status never refreshes findings from GitHub" (`src/logs/command_record.rs:59`). Confirmed by source.
- **One collapsed row can stand for many Commands.** On `stephens`, one chart35 Architect row stands for 48 failures with the same cause and another for 7. On `thirdshift`, #972's row stands for 10. Confirmed by `status --json`.
- **The Console runs on one thread** (`src/console.rs:304-310`) and renders pages off it (`src/console.rs:482-488`). Its only background tasks are the one-second Inbox observer and the restart and account watcher (`src/console.rs:322-347`). Confirmed by source.
- **How a Console action is paired with the CLI.** Every route sits in one table beside the commands it matches (`src/console.rs:93`). `every_console_route_has_a_cli_equivalent` (`src/console.rs:729-731`) fails if a route names none, or one that doesn't parse. Stop's page and its POST each pair four `thirdshift stop` forms. Confirmed by source.
- **How a Command is named.** `stop` and `show` take `repo#number`, with just the repository's name when that's unambiguous on the machine, an Issue URL, a Pass's `kind-stamp` such as `architect-…`, or a record's absolute path (`src/logs/reading/selection.rs:21-55`). Confirmed by source.
- **How Stop records who asked.** The Console checks the session and a CSRF token on every POST (`src/console/auth.rs:82-146`) and writes the account's email into `<record>.stop.json`. The Activity log then gets "<subject> stopped from the Console by <email>" (`src/logs/recording.rs:278-289`). From the CLI, the name is the Unix user's. The Console log has no Stop event. Confirmed by source.
- **The Sweep reads one list per Pickup run:** closed issues still labelled `in-progress` (`src/pickup.rs:262-291`). Only a Pickup run not skipped for the lock makes one, so a Run holding a repository stops its Sweep. Confirmed by source and `GLOSSARY.md`.

## Waiting on research

- Nothing is running.
