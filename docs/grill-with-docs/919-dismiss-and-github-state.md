# Grilling: #919, dismissing Inbox items, and reading GitHub's state

Issue: https://github.com/JacobStephens2/thirdshift/issues/919
Research: #1083 (Ready for review never empties; in review as PR #1180), #711's Q37 in [711-live-runs.md](711-live-runs.md), [ADR-0016](../adr/0016-the-console-is-a-long-running-reader.md) and its #698 Q6.9 amendment, [ADR-0023](../adr/0023-the-inbox-reads-github-to-learn-the-day-shift-acted.md) (written from Round 2), [focus-theme-attention.md](../research/focus-theme-attention.md), #739 (GitHub quota), #1142 (one Console for every machine), [triage-labels.md](../agents/triage-labels.md) (publishing a Spec)

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

## Round 2 (answered)

**Q2.1 - What brings a dismissed row back.** Q1.2 lets you dismiss any row in Needs you or Ready for review, live ones included, and a live Command can go on to do something new. A failed Security run also hides its waiting findings behind its failure (see Research results). The options:
- (a) **A kind of row it wasn't when dismissed.** A Run dismissed as Slow comes back if it goes Silent, and any dismissed live Command comes back if it then dies, fails or ends ready for review. A failed Security run whose findings still wait comes back as a Finding. Each kind counts once: a Run dismissed while Silent stays dismissed through later silences, though it comes back if it turns Slow.
- (b) **Nothing.** A dismissed Command never returns to Needs you or Ready for review, whatever it does next.
- (c) **Each new occurrence,** as with an acknowledged alarm: also a Run that logs again and later falls silent, or one that is Slow again in a later stage.
- (d) **A time limit,** as with a shelved alarm: a dismissed live problem comes back after an hour if it's still there.

*Recommended:* (a). You judged what you saw, and something new needs a new look. A Run that dies after you dismissed it as Slow is the row Needs you most needs to show, and (b) would hide it. (c) would bring a long Spec run back at every stage that runs past its Usual range. (d) brings rows back on a clock, whatever the Run did. A second silence isn't hidden under (a) either: the Run's row in Working says "silent 12 min".

*Answer:* agree 

**Q2.2 - A row that stands for many Commands.** The Inbox folds Failed rows with the same cause into one. On `stephens`, one row stands for 48 chart35 Architect runs and another for 7. On `thirdshift`, #972's stands for 10. The options:
- (a) **Dismissing the row dismisses every Command it stands for.** A later Command that fails the same way starts the row again.
- (b) **Dismissing the row dismisses only its newest Command,** and the next one takes its place.
- (c) **Dismissing the row dismisses its cause,** so later failures with that cause stay out of Needs you too.

*Recommended:* (a). The row is what you see and judge, so it's what you dismiss. (b) would take 48 dismissals to clear one row. (c) is a mute: a billing failure that comes back after you dismissed it is news that the factory is still failing for that reason, and worth hearing until it's fixed.

*Answer:* agree

**Q2.3 - Which GitHub facts move a row.** Q1.3 settled two: a ready Command leaves Ready for review once its pull request is merged or closed, and an ended problem leaves Needs you once its issue is closed. Three cases sit outside those words:
- An Architect run or Security run records no issue of its own. The Run it dispatched does: its plan, or its fix.
- A Triage row asks for a triage decision, after which the issue may stay open, as `ready-for-human` for instance.
- A failed Run whose pull request you close without merging still holds its issue's Claim until you relabel it.

The options:
- (a) **Finished work, in either group, and triage.** A row leaves once GitHub shows its issue closed or its pull request merged, counting an Architect run's or Security run's dispatched Run as its own. A ready row also leaves once its pull request is closed, as Q1.3 says. A Triage row also leaves once its issue no longer has `needs-triage`. GitHub's latest state counts, so a reopened issue brings its row back. GitHub is read only for rows with a Command record.
- (b) **Q1.3's two facts only,** for the Command's own issue and pull request.

*Recommended:* (a). A merged pull request is finished work whether or not its issue closed with it. The dispatched Run is the only way GitHub can tell anything about most Architect and Security rows: on `stephens` it shows two of them finished, and four plans or fixes still open and claimed (see Research results). A pull request closed unmerged leaves a failed Run's issue claimed, so that row should stay until you relabel or close the issue. Once `needs-triage` is off, a Triage row's question is answered. Rows with no record are at least six days old and age out within days, so reading GitHub for them would buy little. Findings are Q2.8.

*Answer:* agree

*Checked:* An Architect run or Security run dispatches at most one Run. Each dispatch adds one to its record (`src/logs/recording.rs:348-362`), and no Command on either machine has two. So "the Run it dispatched" is always one Run.

**Q2.4 - Who reads GitHub, and how often.** Q1.3 settled that GitHub is read on its own schedule and kept, never on the Console's one-second render. Both machines' Consoles run as systemd services that can find `gh`, though nothing in the Console calls it today. Pickup runs, whose Sweep already reads GitHub, start every 2 minutes on `thirdshift` and every 10 on each of `stephens`'s 18 repositories, but none sweeps while a Run holds its repository. The options:
- (a) **The Console, once a minute.** A background task looks up every row's issue and pull request in one batched query and keeps the answer in a file on the machine. `thirdshift status` reads the same file, and asks GitHub itself first when no Console has refreshed it in the last few minutes, so the two always show the same groups. About 60 GraphQL points an hour per machine.
- (b) **Each Pickup run's Sweep.** It looks up its repository's rows and writes what GitHub says beside each record, and the Console goes on reading only local files. A row moves within one Pickup interval, but not while a Run holds its repository, and never on a repository with no Pickup line.
- (c) **Whatever shows the Inbox,** `thirdshift status` or the Console, asks GitHub when the kept answer is more than a minute old, with no background task.
- (d) **GitHub webhooks.** GitHub tells the Console the moment a pull request merges or an issue closes. Each repository needs a webhook with a shared secret, the Console needs a route open to anyone on the internet, and deliveries missed while it restarts still need a poll to catch up.

*Recommended:* (a). The issue's complaint is a pull request you merged by hand still sitting in Ready for review. Under (b) that's likeliest while the factory is busy on the same repository, which is exactly when its Pickup runs skip. (c) makes `status` wait on GitHub whenever it runs more than a minute after the last read. (d) is the most live, but needs setting up on 19 repositories, opens a new door into the Console, and still needs (a)'s poll. (a) changes what ADR 0016 says the Console reads; see Q2.9.

*Answer:* agree

*Checked:* `gh` on `thirdshift`, on `stephens` and on this laptop is signed in as the same account, JacobStephens2. So the factory on both machines and both Consoles share one GraphQL quota and one REST quota, and the two Consoles together would spend about 120 GraphQL points an hour. At 21:00 UTC on 2026-10-23, 3,883 of the hour's 5,000 GraphQL points and 4,694 of its 5,000 REST requests were left. How the Consoles give way to the factory is Q3.2.

**Q2.5 - Whether age still moves a row GitHub says is waiting.** Today an ended problem leaves Needs you after `inbox.days`, 7 by default, and #1083 gives Ready for review the same limit. Age stands in for knowing whether the Day shift has acted. With GitHub read, it can become the fallback. The options:
- (a) **No, in either group.** A row whose issue or pull request GitHub shows still open stays until GitHub shows it finished, a later Command starts on its issue, or you dismiss it. Age moves only the rows GitHub can't speak for: those with no issue or pull request at all, such as an Architect run whose review failed before it planned anything; rows with no record; and any row GitHub hasn't been read for.
- (b) **Only in Ready for review.** An open pull request keeps its row there however old, and Needs you keeps its age limit.
- (c) **Yes, as today and in #1083.**

*Recommended:* (a). An open pull request, or a failure whose issue is still open and claimed, still waits on you however old it is, and an Inbox that hides it after a week can't be trusted to be complete. It would have kept no extra row so far: the 13 failures that aged out of Needs you with an issue recorded all have that issue closed now. clave's and bountywell's open, claimed plans are the kind it would keep. Dismissal handles the ones you mean to leave. This narrows #1083's age limit, as Q1.1 allowed.

*Answer:* agree, so this inbox then starts to function more like a mailbox inbox

*Checked:* Yes. Needs you and Ready for review now keep a row until someone deals with it, as mail stays in an inbox until it's answered or archived: GitHub shows it finished, a later Command takes up its issue, or you dismiss it. Age clears only what GitHub can't speak for. The glossary's Inbox and Needs you entries now say so. Three questions follow, in Round 3: how long GitHub has to be read for a row (Q3.3), where a row goes once it leaves after waiting past `inbox.days` (Q3.4), and whether you'll want to dismiss many rows at once (Q3.9).

**Q2.6 - The command, and taking a dismissal back.** ADR 0016 wants a CLI command with `--json` for every Console action. `stop` and `show` name a Command as `repo#number`, an Issue URL, a Pass's `kind-stamp`, or a record's path. The options:
- (a) **`thirdshift dismiss <command> [--json]`, with `--undo` to take it back.** It names Commands as `stop` does, and dismisses the whole row its Command is in (Q2.2). It asks for no reason. It refuses, saying why, a row outside Needs you and Ready for review, and one with no record. The Console offers both, and the Activity log records both.
- (b) **The same, with no undo.** A dismissal is final, and the row stays visible in Done or Resolved, saying who dismissed it.

*Recommended:* (a). A misclick in the Console shouldn't hide a problem for good, and taking a dismissal back is cheap: the dismissal beside the record goes, and the Activity log says who took it back.

*Answer:* agree

**Q2.7 - Where a row goes once it leaves.** Whether you dismissed it or GitHub moved it. The options:
- (a) **Where it would be without the problem, saying why it left.** A live Command goes to Working or Landing. An ended one goes to Done if it ended in the last day, then Resolved, then History, by the usual rules. Its row says `"dismissed by <who>"`, or what GitHub showed, such as "pull request merged".
- (b) **A new group, Dismissed,** after Resolved.
- (c) **Straight to History.**

*Recommended:* (a). The groups already sort by how recent, and Resolved already means "older, with nothing waiting", which is what a dismissal says. The reason on the row keeps a dismissed failure in Done from reading as a success. (b) adds an eighth group. (c) hides what you did a minute ago.

*Answer:* agree

*Checked:* The usual rules go by when the Command ended. Done holds the last day, Resolved runs until `inbox.days`, and anything older is only on the History page, which the Inbox leaves out (`src/logs/reading/inbox.rs:209-224`, `src/logs/status/inbox.rs:202`). Under Q2.5 a row can wait longer than `inbox.days`, so dealing with it would send it straight to the History page. That's Q3.4.

**Q2.8 - Findings and GitHub.** A Finding row has no issue or pull request of its own. The glossary says a finding stops waiting once the Day shift triages it on GitHub, but the Inbox never looks, so a triaged finding stays in Needs you until it ages out or is dismissed. The check a Security run makes before it starts, whether any finding still waits, could tell from one list of the repository's findings (`src/github/advisories.rs:500-515`). The options:
- (a) **A Finding leaves once none of its findings still waits,** by the Security run's own check, on the same schedule as the rest: one list per repository with a Finding row, from the REST quota rather than GraphQL.
- (b) **Dismissal and age only.**

*Recommended:* (a). It makes the Inbox agree with the glossary, and a Finding has nothing else GitHub can tell it by. It reads only what the Security run already reads, and the row shows what it shows today.

*Answer:* agree

**Q2.9 - Writing down the reversal.** Reading GitHub reverses #711's Q37 and changes what ADR 0016 says the Console reads. It would be hard to undo once the Inbox depends on it, a reader of ADR 0016 wouldn't expect it, and it was a real trade-off, so it meets the bar for an ADR. The options:
- (a) **A new ADR, 0022,** written once Q2.4 is answered, with a line in ADR 0016 pointing to it.
- (b) **An amendment to ADR 0016,** as #711's Q46 and #698's Q6.9 were.
- (c) **No ADR.** This file and the Spec are enough.

*Recommended:* (a). It decides where the Inbox's facts come from, with costs of its own (quota, staleness, what happens when GitHub can't be reached), not how the Console runs. Its own ADR has room for the options weighed here, where ADR 0016's amendments are one-line permissions.

*Answer:* agree

*Checked:* ADR 0022 has since been taken on `main`, by #1101's Ticket runs and plain schedules, so this one is [ADR 0023](../adr/0023-the-inbox-reads-github-to-learn-the-day-shift-acted.md), "The Inbox reads GitHub to learn the Day shift has acted". It's written, and ADR 0016 now points to it. Round 3's answers on failures, the quota and how long GitHub is read for a row (Q3.1 to Q3.3) will go in its Consequences.

## Round 3 (answered)

**Q3.1 - When GitHub can't be read.** Q2.4 has the Console read GitHub once a minute, and Q2.5 lets a row wait in Needs you or Ready for review for as long as GitHub shows its work open. A read can fail: the network is down, `gh` is signed out, the quota is spent (#739), or a repository has been renamed, deleted or hidden from the account. Nothing warns about GitHub today. The Inbox has one warning line, shown by `status` and the Console, which today only says when one setting contradicts another (see Research results). The options:
- (a) **Keep the last answer, and say how old it is.** A row keeps the group its last-read facts give it, however old they are. A row GitHub hasn't been read for yet goes by age, as Q2.5 says, and so does one whose issue or pull request GitHub says doesn't exist or can't be seen. Once the last good read is more than 5 minutes old, the warning line says when GitHub last answered and why the latest read failed. `status` asks GitHub itself only when the kept answer is more than 5 minutes old, and if that fails too, it shows the kept answer with the same warning.
- (b) **(a), until a day has passed without a good read.** After that, every row goes by age, as if GitHub had never been read.
- (c) **Forget the answer when a read fails,** so every row goes by age until GitHub answers again.

*Recommended:* (a). The last answer stays right for nearly every row, since a merged pull request stays merged, and the warning says when it might not be. Under (b) or (c), an outage would put every row GitHub had finished back by age: a week of finished failures would reappear in Needs you at once, then leave again when GitHub came back. Five minutes is five missed reads, long enough to ride out a blip and short enough to notice.

*Answer:* agree

**Q3.2 - Leaving the quota to the factory.** Every reader here signs in as JacobStephens2, so the factory on both machines and both Consoles share one GraphQL quota of 5,000 points an hour and one REST quota of 5,000 requests (Q2.4's check). The Consoles would spend about 120 points an hour between them, plus a REST list a minute for each repository with findings to check. The factory ran the GraphQL quota out by itself on 2026-10-17 (#739). Each read can ask how much is left at no extra cost. The options:
- (a) **The Console steps aside when the quota runs low.** If its last read showed fewer than 500 GraphQL points or 500 REST requests left, a tenth of each, it skips its reads until the quota resets, and the warning line says so. `status` does the same.
- (b) **Always read.** 120 points an hour is under 3% of the quota.
- (c) **Read less often as the quota falls,** such as every 5 minutes once less than half is left.

*Recommended:* (a). The factory's reads do work, and the Inbox's only tidy it, so when there isn't enough for both, the factory should have it. Below a tenth, the factory is close to failing anyway (#739), and an Inbox a few minutes behind is the cheaper loss. (b) could take a Run's last points. (c) is a second rule to tune for little gain.

*Answer:* agree

**Q3.3 - How long GitHub is read for a row.** Q2.3 lets GitHub's latest state count, so a reopened issue brings back a row GitHub had moved out. That means reading GitHub for rows it has already moved, and since Q2.5, no age limit ends a row GitHub can speak for, so nothing would stop a row being read for good. Each read would then grow with the machine's history, by thousands of rows within months. The options:
- (a) **Until the row reaches the History page.** GitHub is read for every row in Needs you and Ready for review, and for every row that has left them, while it's in Done or Resolved. On the History page a row keeps what GitHub last showed, and reopening its issue no longer brings it back.
- (b) **Only while the row is in Needs you or Ready for review.** Once GitHub moves a row out, it stays out. This takes back the part of Q2.3 about reopened issues.
- (c) **Every row with a record, for good.**

*Recommended:* (a). An issue is usually reopened soon after it's closed, while its row is still in Done or Resolved; Q3.4 decides how long that is. After that, a reopened issue that needs the factory gets `ready-for-agent`, and the new Command takes over from the old row. (b) loses what Q2.3 settled, and (c) makes every read bigger every day.

*Answer:* agree

**Q3.4 - Where a row goes when it leaves late.** Q2.7 sends a row that leaves to Done, Resolved or History by the usual rules, and those go by when its Command ended: Done for a day, Resolved until `inbox.days`, then only the History page, which the Inbox leaves out (Q2.7's check). Q2.5 lets a row wait past `inbox.days`. When you then dismiss it, or merge its pull request, it goes straight to the History page, and in the Console its Undo goes with it. The options:
- (a) **Resolved counts from when the row stopped waiting.** Done still holds the Commands that ended in the last day. Resolved holds older ones until `inbox.days` after they stopped waiting: when they ended, for most, or when they left Needs you or Ready for review. A pull request merged today on a Run that ended ten days ago shows in Resolved for a week, saying "pull request merged".
- (b) **By when the Command ended, as today.** It goes straight to the History page.
- (c) **A day in Done from when it left,** then the usual rules.

*Recommended:* (a). Resolved already means "older, with nothing waiting", so its clock should start when the waiting ended. It keeps what you just did in view, with its Undo, and changes nothing for a row that never waited. (b) hides a row the moment you deal with it, which is why Q2.7 turned down sending rows straight to History. (c) mixes old work into Done, beside the last day's, then drops it out of the Inbox a day later.

*Answer:* agree

*Checked:* Today a row that ages out of Needs you skips Resolved and goes straight to the History page, because Needs you's age limit and Resolved's reach are both `inbox.days` (`src/logs/reading/inbox.rs:199-224`). Read literally, (a)'s "when they left Needs you" would now keep such a row in Resolved for another `inbox.days`. That's Q4.2.

**Q3.5 - Live rows and GitHub.** Q1.3 and Q2.3 move a row once GitHub shows its work finished. A Silent or Slow row is live: its Run is still going. If you merge a live Run's pull request by hand, as happened with #88, or close its issue, the Run carries on until Delivery finds the pull request merged (`src/delivery.rs:174-184`), or until someone stops it. The options:
- (a) **GitHub moves only ended rows.** A Silent or Slow row stays until its Run logs again, its stage ends, its Command ends, or someone stops or dismisses it, whatever GitHub shows. Once it ends, GitHub's facts apply as for any ended row.
- (b) **GitHub moves live rows too.** A Slow Run whose pull request you merged leaves Needs you while it runs.

*Recommended:* (a). A Run still going on a closed issue or a merged pull request is spending time on finished work, so its silence or slowness is worth seeing while Stop is a click away. Once it ends, GitHub's facts take its row out if the work is done.

*Answer:* agree

**Q3.6 - Where the Console offers Dismiss.** #917 will decide the Inbox's layout, but it's parked, and the Spec needs Dismiss somewhere now. Stop is in a row's Command details: its button opens a page that asks you to confirm, with `thirdshift stop …` beside it (`src/console/peek.html:3`, `src/console/stop.html`). The options:
- (a) **In Command details, beside Stop, with no confirming page.** One click dismisses the row, with `thirdshift dismiss …` beside the button. Once it's dismissed, the same place offers Undo, with `thirdshift dismiss --undo …`. #917 can move both onto the row later.
- (b) **The same, with a confirming page,** as Stop has.
- (c) **A button on every row** in Needs you and Ready for review, without opening Command details.

*Recommended:* (a). Stop asks first because it can't be undone, and a dismissal can. (c) changes the rows themselves, which is #917's work.

*Answer:* agree

**Q3.7 - Settings.** In #711 you wanted `inbox.days` in the User config. Reading GitHub brings three numbers of its own: the minute between reads (Q2.4), the 5 minutes before a warning (Q3.1), and the reserve left to the factory (Q3.2). The options:
- (a) **No new settings.** The numbers are fixed, and every machine reads GitHub.
- (b) **One setting to turn reading GitHub off,** on by default. With it off, the Inbox works as before #919, by age and dismissal.
- (c) **(b), plus a setting for each number.**

*Recommended:* (a). Every machine already needs `gh` signed in to run the factory, so there's no machine where reading can't work, and Q3.2 guards the quota. A switch can come later if a machine needs one. (b) and (c) add modes for `status`, the Console and their tests to cover.

*Answer:* agree

**Q3.8 - What a waiting row shows of GitHub.** The issue asks whether to get live GitHub status updates. So far GitHub's state only moves rows (Q1.3), and a row that leaves says why (Q2.7). A row still waiting shows what it shows today. The options:
- (a) **Nothing more.** The Inbox as a whole says when GitHub was last read, once that's more than 5 minutes ago (Q3.1).
- (b) **Each waiting row also shows its issue's and pull request's state,** such as "pull request open, changes requested" or "pull request closed, issue still claimed", from the same query.
- (c) **(b), plus its checks,** passing or failing.

*Recommended:* (a). A waiting row is one GitHub shows open, so most rows would say the same thing, and its issue and pull request are a click away. (b) and (c) are a view of GitHub in the Inbox, a feature of its own, which could be a new issue if you want one.

*Answer:* agree

**Q3.9 - Dismissing several rows at once.** With the Inbox working like a mail inbox (Q2.5), a row GitHub can speak for stays until it's dealt with. Mail apps let you archive many messages at once. `stop` takes one Command, and Q2.2 already lets one dismissal clear a row that stands for 48. The options:
- (a) **One row at a time,** in the Console and the CLI, as with Stop.
- (b) **`thirdshift dismiss` takes several Commands,** and the Console lets you tick rows and dismiss them together.
- (c) **(b), plus Dismiss all for a group.**

*Recommended:* (a). Few rows will be left for a dismissal. On the two machines today, the rows GitHub can't settle are the billing failures of chart35's and horarium's Architect runs and one row with no record, all of which still leave by age. Every other row that GitHub doesn't take out waits on work that's still open, and each is worth a look. Dismiss all would hide new problems along with old ones. (b) can come later if the Inbox shows a need.

*Answer:* agree

**Q3.10 - The issue's "(title unavailable)".** The issue's example row said "(title unavailable)". Every row with a record that says so, 30 on `thirdshift` and 10 on `stephens` today, is a Pickup run whose own record has its issue's number but not its title, while the Run it dispatched recorded the title. The Inbox reads the Pickup run's (`src/logs/status/inbox.rs:274-284`). The other 45 are rows with no record, whose Activity log line never had a title. The options:
- (a) **A standalone Ticket, labelled `ready-for-agent`:** a Pickup run's row shows the title its dispatched Run recorded. It's filed alongside #919's Spec but outside it, so a Pickup run can take it at once.
- (b) **A Ticket inside #919's Spec.**
- (c) **Fill in missing titles from GitHub's read,** in the same query.

*Recommended:* (a). The title is already on the machine, so the fix needs nothing from GitHub, and it's a bug of its own that needn't wait for the Spec. (c) would leave titles missing whenever GitHub can't be read.

*Answer:* agree

**Q3.11 - Publishing #919.** Once this round is answered, the design is settled, and `docs/agents/triage-labels.md` says to decide first whether Tickets will follow. #711 and #981 were each grilled, then retitled "Spec: …", with their Tickets as sub-issues labelled `ready-for-agent`. A Spec labelled `ready-for-agent` is taken by a Pickup run and built by one Spec run into one pull request (ADR 0006). #981 is `ready-for-human`, so it waits for you to start it. #919 is `ready-for-human` now. The options:
- (a) **#919 becomes the Spec, with Tickets, and the factory builds it.** Retitle it "Spec: …", rewrite its body from this file, and publish its Tickets as sub-issues with their blocking links, keeping it `needs-triage` until all are in place, then labelling it `ready-for-agent`. Roughly, the Tickets are:
  1. `thirdshift dismiss` and `--undo`, kept beside the record, with their Activity log lines and the Inbox rules of Q2.1, Q2.2 and Q2.7;
  2. the Console's Dismiss and Undo (Q3.6), after Ticket 1;
  3. the Console reading GitHub, with `status` sharing its answer (Q3.1, Q3.2, Q3.7);
  4. GitHub moving rows (Q2.3, Q2.5, Q3.3 to Q3.5), after Ticket 3 and #1083;
  5. Findings leaving once triaged (Q2.8), after Ticket 3.

  Each corrects the site pages it changes.
- (b) **The same, but #919 ends `ready-for-human`,** as #981 is, and you start its Spec run when you choose.
- (c) **A new Spec issue,** with #919 closed in its favour.

*Recommended:* (a). Each Ticket is one session, and none needs you once the design is settled. (b) suits a Spec you want to pace yourself. (c) splits #919's history from the work. You approve the Tickets' exact text before they're published.

*Answer:* agree, we'll use the to-spec and then to-tickets skills to make these

*Checked:* Both skills are in `~/.claude/skills/`, and only you can start them (`disable-model-invocation: true`), as `/to-spec` and `/to-tickets`. `to-spec` writes the Spec from the current conversation, so it's best run in this session, and `docs/agents/triage-labels.md` overrides both skills' readiness and parent-modification defaults. This branch is 85 commits behind `main`, whose glossary now says Ticket run where this branch's says Run (#1101). That's Q4.1.

## Round 4 (answered)

**Q4.1 - Getting the glossary and ADR 0023 onto `main` before the Spec.** This branch, `dismiss-grill`, holds the only copy of ADR 0023, the **Dismissal** entry and the other glossary changes, and it's 85 commits behind `main`. `/to-spec` writes from this conversation and the checkout it runs in, and the Spec run's sessions will read `main`'s glossary and ADRs. Meanwhile `main` has renamed the run on one issue from Run to Ticket run (#1101, ADR 0022): its glossary has no **Run** left, and this branch's has 46. A trial merge of `main` into this branch conflicts only in `GLOSSARY.md`. #1083's PR #1180 changes the same Inbox line. Earlier grill files reached `main` through ordinary pull requests (#1100, #1108). The options:
- (a) **Merge `main` in now, take its words, and land the docs first.** I merge `main` into `dismiss-grill`, resolve the glossary, and put ADR 0023 and the new glossary text in `main`'s words, such as "a **Slow** Ticket run that then dies". Then I push the branch and open a pull request with this file, the glossary, ADR 0016's new line and ADR 0023. Once you've merged it, you run `/to-spec` and `/to-tickets` here.
- (b) **Run `/to-spec` and `/to-tickets` from this branch now,** and land the docs afterwards.
- (c) **The Spec's first Ticket carries the glossary and ADR changes.**

*Recommended:* (a). The Spec, and every session that builds it, should read `main`'s words and the decisions recorded here, and a docs-only pull request is quick to review. Whichever of it and #1180 merges second keeps both #1083's later-Command exit and GitHub's in the Inbox line. (b) writes the Spec in this branch's older words. (c) mixes the docs into a code review, and leaves the Tickets that run beside the first without ADR 0023 to read.

*Answer:* agree

*Checked:* Done on 2026-10-23. `main` is merged into `dismiss-grill`, with one conflict, in `GLOSSARY.md`. The new glossary text and ADR 0023 now say Ticket run, and "the Spec run or Ticket run it dispatched". This file keeps the words it was written in, as ADR 0022 lets dated records do, and the glossary lists Run as Ticket run's former name. The docs go to `main` in a pull request of their own.

**Q4.2 - A row that ages out.** Q3.4 starts Resolved's clock when a row stops waiting, which its option put as "when they left Needs you or Ready for review". Today a row that ages out of Needs you skips Resolved and goes straight to the History page (Q3.4's check). Read literally, Q3.4 would now keep it in Resolved for another `inbox.days`. Since Q2.5, only rows GitHub can't speak for age out, such as chart35's 48 billing failures. The options:
- (a) **Ageing out isn't dealing with a row.** A row that ages out goes straight to the History page, as today. Resolved's clock restarts only when GitHub settles a row, a later Command takes up its issue, or someone dismisses it.
- (b) **Ageing out counts as leaving.** The row spends another `inbox.days` in Resolved.

*Recommended:* (a). Q3.4 keeps in view what someone has just dealt with, with its Undo. Nobody dealt with a row that aged out, and its week in Needs you was its chance to be seen. (b) would keep chart35's billing failures in the Inbox for two weeks.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- **A Console dismissal is already allowed.** ADR 0016's amendment of 2026-10-21 (#698 Q6.9) lets the Console record judgments only a person makes, and named "dismissing an item in Needs you (#919)". It now says Needs you or Ready for review (Q1.2). Dismissal is also a `thirdshift` command with `--json`, a test pairs the two, and it never starts work.
- **The Activity log records each dismissal,** naming who asked and whether from the CLI or the Console, as it does for a Stop. The Console log stays for sign-ins and other account events.
- **Any signed-in Console account may dismiss, as any may Stop,** behind the same session and CSRF checks. The account's email is the name the Activity log gives.
- **Each Console dismisses only its own machine's Commands,** since each machine serves its own Console. One Console for every machine is #1142.
- **`thirdshift status`, `status --json` and the Console Inbox keep agreeing on every row's group,** as #711's Q39 and #1083 require. Anything that moves a row, whether a dismissal or a fact from GitHub, moves it in all three.
- **A row that leaves Needs you also leaves the tab title's count, the urgent icon and browser notifications,** since all three count the rendered Needs you rows. A move GitHub causes reaches open pages like any other change, through the one-second observer.
- **#1083 lands first, as its own issue** (Q1.1), and is in review as PR #1180. #919 builds on its two ways out of Ready for review, then narrows its age limit (Q2.5): a ready Command whose pull request GitHub shows open stays in Ready for review however old it is. #1180 also writes into the glossary that the Inbox reads only the machine, and #919's Spec takes that back.
- **Dismissal is the only new action** (Q1.2). There is no Acknowledge: nothing stays in Needs you but stops counting.
- **Every row in Needs you or Ready for review can be dismissed, live ones included** (Q1.2). Rows in Landing, Working, Done, Resolved and History can't. A Landing row's "see Needs you" line points to the row to dismiss.
- **A dismissal changes only the Inbox.** It never stops, starts or changes a Command, its Runs, its issue, its labels, its Claim or its pull request, and it never writes to GitHub (Q1.4). A dismissed Silent Run keeps running, and Stop still ends it.
- **A dismissal is kept beside its Command's record** (Q1.4), saying who, when, from the CLI or the Console, and which of the record's rows it covers, since a dispatched Run shares its Pickup run's record.
- **A row with no record can't be dismissed** (Q1.4). These are Commands from before records began on 2026-10-19, rebuilt from the Activity log. Round 1 called them History rows, but they're not the History group, so from Round 2 this file calls them rows with no record. They leave by age: those in Needs you within a day, those in Ready for review once #1083 lands.
- **Each machine's Console reads GitHub once a minute** (Q2.4), never at its one-second render (Q1.3). One batched GraphQL query covers every row's issue and pull request, including those of an Architect run's or Security run's dispatched Run (Q2.3), and one REST list per repository covers each Security run whose findings were waiting (Q2.8). The answer goes in one file on the machine, as Q2.4 says. It's a copy of GitHub, not something a Command did, so it doesn't sit among a Command's files, and losing it costs one read. `thirdshift status` reads the same file. Failures, the quota (#739) and how long a row is read for are Q3.1 to Q3.3.
- **The issue's own kind of row leaves by itself.** All 13 rows that failed with "PR … is merged, not open" have closed issues (Q1.3's first check). Whether such a Run should count as failed at all is outside #919.
- **The glossary is rewritten.** It now has **Dismissal**, which says what brings a dismissed row back and that a dismissal can be taken back, and its Console, Inbox, Needs you and Activity log entries mention it. The Needs you and Inbox entries now say how GitHub settles a row, and that age clears only rows GitHub can't speak for. "Alerts", the issue's word, is one the glossary avoids for Needs you, so this file says "items" or "rows".
- **#917 decides how the Inbox looks.** Its parked layout work (one-line rows and the peek) owns the rows, and Q3.6 places Dismiss until then. This grill decides what the action does.
- **Needs you and Ready for review work like a mail inbox** (Q2.5). A row stays until GitHub shows it finished, a later Command starts on its issue, or someone dismisses it. Age clears only rows GitHub can't speak for: those with no issue or pull request, theirs or their dispatched Run's; rows with no record; and rows GitHub hasn't been read for yet. `inbox.days` keeps its name and its default of 7, and still sets how long Resolved lasts.
- **A dismissal holds until its row turns into a kind it wasn't, or someone takes it back** (Q2.1, Q2.6). Nothing GitHub does ends it: a reopened issue brings back a row GitHub moved out, not one you dismissed. Each kind counts once for the whole row, so a Spec run dismissed while one Ticket's Run was Silent stays dismissed if another Ticket's Run goes Silent. That Run's row in Working says so.
- **A collapsed row that comes back counts only the failures since** (Q2.2). Dismissing the row dismisses the 48 Commands it stands for, and a 49th failure with the same cause starts a row of one.
- **A row GitHub moves follows GitHub's latest state, and nothing is written.** Nothing goes beside the record, and the Activity log gets no line: it records what people and the factory did, and GitHub's state is read, not decided.
- **`thirdshift dismiss --undo` works wherever the dismissed row now is,** and refuses a row that isn't dismissed (Q2.6). Taken back, the row goes where GitHub's facts and the other rules put it, so a row whose pull request has merged since stays out of Ready for review.
- **A failed Security run's findings outlast its dismissal** (Q2.1, Q2.8). Dismiss the failure, and the row comes back as a Finding while any of its findings still waits. Once they're all triaged, it doesn't come back.
- **A row that comes back to Needs you notifies like a new one.** A browser notifies whenever the count of Needs you rows rises, and keeps no list of rows it has seen, so a row brought back by a new kind, a reopened issue or your own Undo counts as new (see Research results).
- **A row that leaves says why, wherever it goes** (Q2.7): "dismissed by" and who, or what GitHub showed, such as "pull request merged", "issue closed", "`needs-triage` taken off" or "findings triaged". `status`, `status --json`, the Console and the History page all show it. The exact words are the Spec's.
- **Reading GitHub never writes to it** (ADR 0023). Taking `in-progress` off a closed issue stays the Sweep's job (`src/pickup.rs:259-292`).
- **When GitHub can't be read, the Inbox keeps its last answer** (Q3.1). A row keeps the group its last-read facts give it. A row not read yet, or whose issue or pull request GitHub says is gone or hidden, goes by age. After 5 minutes without a good read, the warning line in `status` and the Console says when GitHub last answered and why the latest read failed. `status` asks GitHub itself only when the kept answer is more than 5 minutes old.
- **The Console leaves the last tenth of each quota to the factory** (Q3.2). With fewer than 500 GraphQL points or 500 REST requests left, it skips reads until that quota resets, and the warning line says why. `status` does the same.
- **GitHub is read for a row until it reaches the History page** (Q3.3). There, the row keeps what GitHub last showed, and a reopened issue no longer brings it back. Each read grows with the Inbox, not with the machine's history.
- **Resolved's clock starts when a row stops waiting** (Q3.4), and Done still holds the Commands that ended in the last day. A row stops waiting when whatever ended its wait happened: when GitHub says its pull request merged or its issue closed, when the later Command started, or when it was dismissed, not when the Inbox noticed. So on the day #919 ships, rows GitHub finished weeks ago go straight to the History page instead of filling Resolved at once. An Undo makes a row wait again if it still has a problem, and its clock starts afresh when that wait ends. Ageing out doesn't end a wait (Q4.2): a row that ages out goes straight to the History page, as today.
- **GitHub moves only ended rows** (Q3.5). A Silent or Slow row stays until its Run logs again, its stage ends, its Command ends, or someone stops or dismisses it.
- **Dismiss sits in Command details, beside Stop** (Q3.6). One click dismisses, with no confirming page, and `thirdshift dismiss …` is shown beside the button. A dismissed row's Command details offers Undo, with `thirdshift dismiss --undo …`. #917 may later move both onto the rows.
- **No new settings** (Q3.7). Every machine reads GitHub, and the minute between reads, the 5 minutes before a warning and the reserve are fixed.
- **A waiting row shows nothing new about GitHub** (Q3.8). Showing each waiting row's GitHub state, or its checks, would be an issue of its own, and none has been filed.
- **Rows are dismissed one at a time** (Q3.9), in the Console and the CLI.
- **"(title unavailable)" gets a standalone Ticket** (Q3.10), labelled `ready-for-agent` and filed alongside the Spec but outside it: a Pickup run's row shows the title its dispatched Run recorded.
- **#919 becomes the Spec, written with `/to-spec`, and its Tickets come from `/to-tickets`** (Q3.11). Both are published as `docs/agents/triage-labels.md` says: `needs-triage` while publishing, then `ready-for-agent` once every Ticket and blocking link is in place. The standalone "(title unavailable)" Ticket comes from `/to-tickets` too. Each Ticket corrects the hand-written site pages it changes, and adds what it builds to the domain-to-code index in `docs/code-navigation.md`.
- **Rows with no record will be gone before #919 ships.** The newest started before 03:21 UTC on 2026-10-19, so every one is older than `inbox.days` by 2026-10-26, and #1083 ages them out of Ready for review. Q1.4's refusal stays, but the Spec needn't do more for them, and no collapsed row will mix them with Commands that have records.

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
- **How Stop records who asked.** The Console checks the session and a CSRF token on every POST (`src/console/auth.rs:82-146`) and writes the account's email into `<record>.stop.json`. The Activity log then gets `<subject> stopped from the Console by <email>` (`src/logs/recording.rs:278-289`). From the CLI, the name is the Unix user's. The Console log has no Stop event. Confirmed by source.
- **The Sweep reads one list per Pickup run:** closed issues still labelled `in-progress` (`src/pickup.rs:262-291`). Only a Pickup run not skipped for the lock makes one, so a Run holding a repository stops its Sweep. Confirmed by source and `GLOSSARY.md`.
- **Every reader shares one GitHub account.** `gh` on `thirdshift`, on `stephens` and on this laptop is signed in as JacobStephens2, so the factory on both machines and both Consoles draw on one GraphQL quota and one REST quota. At 21:00 UTC on 2026-10-23, 3,883 of 5,000 GraphQL points and 4,694 of 5,000 REST requests were left in the hour. Confirmed by `gh api user` and `rateLimit` on each machine.
- **An Architect run or Security run dispatches at most one Run.** Each dispatch adds one to its record (`src/logs/recording.rs:348-362`), and no Command on either machine has two. Confirmed by source and `status --json`.
- **Done, Resolved and History go by when the Command ended.** Done is the last day, Resolved lasts until `inbox.days`, and older Commands are only on the History page: the Inbox asks for no History rows (`src/logs/reading/inbox.rs:209-224`, `src/logs/status/inbox.rs:202`). Confirmed by source.
- **Notifications fire when the count rises.** A browser notifies when there are more Needs you rows than at its last look and no other tab has already notified (`src/console/assets/notifications.js:44-46`). It keeps no list of rows it has seen. Confirmed by source.
- **The Inbox has one warning line,** shown by `status` and as "Warning: …" at the top of the Console's Inbox. Today it only says that the waiting-line setting contradicts the silent time (`src/logs/status/inbox.rs:14`, `:269`; `src/console/inbox.html:39`). Nothing warns about GitHub. Confirmed by source.
- **Stop sits in a row's Command details,** a button that opens a page asking you to confirm, with the CLI form beside it (`src/console/peek.html:3`, `src/console/stop.html`). Confirmed by source.
- **"(title unavailable)" comes from a Pickup run's own record.** All 40 rows with a record that show it, 30 on `thirdshift` and 10 on `stephens`, are Pickup runs whose record has the issue's number but not its title, while the Run each dispatched recorded the title (`src/logs/status/inbox.rs:274-284`). The other 45 are rows with no record. Confirmed by `status --json` on each machine.
- **#1083 is in review** as PR #1180, "Age ready Commands out of Ready for review like ended Needs you rows". Its glossary change says a pull request merged or closed by hand stays in Ready for review until a later Command or age, "since the Inbox reads only the machine". Confirmed by `gh`.
- **How grilled issues became Specs.** #711 and #981 were each retitled "Spec: …", and their Tickets are sub-issues labelled `ready-for-agent`. #981 itself is `ready-for-human`. A Ticket inside a Spec is never a Ready issue: its Spec run reaches it (`GLOSSARY.md`, Ready issue). `docs/agents/triage-labels.md` publishes a multi-session Spec as `needs-triage`, and labels it `ready-for-agent` once every Ticket and blocking link is in place. Confirmed by `gh` and docs.
- **ADR 0022 is taken.** `main` has `0022-ticket-runs-and-plain-schedules.md` (#1101), and no branch has an 0023, so #919's ADR is 0023. Confirmed by `git ls-tree` on every branch.
- **Needs you's age limit and Resolved's reach are the same `inbox.days`,** so a row that ages out of Needs you skips Resolved (`src/logs/reading/inbox.rs:199-224`). Confirmed by source.
- **`to-spec` and `to-tickets` are yours to start.** Both are in `~/.claude/skills/`, with the same files in `~/.agents/skills/`, marked `disable-model-invocation: true`, so they run as `/to-spec` and `/to-tickets`. `to-spec` "takes the current conversation context". Confirmed by their `SKILL.md`.
- **This branch is behind `main`.** `dismiss-grill` is 85 commits behind `origin/main` and 4 ahead. `main`'s glossary calls the run on one issue a Ticket run and uses **Run** nowhere (#1101, ADR 0022); this branch's glossary uses it 46 times. A trial merge (`git merge-tree`) conflicts only in `GLOSSARY.md`. Confirmed by `git`.
- **Grill files reach `main` through ordinary pull requests:** 711's and 429's with #1100, 917's with #1108. Confirmed by `git log`.

## Waiting on research

- An agent is re-checking the code facts above against `main` as merged on 2026-10-23, 94 commits newer than the research. No open question waits on it.
