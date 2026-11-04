# Grilling: #706 and #1323, `thirdshift review` and `thirdshift triage`

Issues: https://github.com/JacobStephens2/thirdshift/issues/706, https://github.com/JacobStephens2/thirdshift/issues/1323, under the umbrella https://github.com/JacobStephens2/thirdshift/issues/1691
Research: see "Research results"

## Background
****
Two hand loops Jacob repeats at work, in a private repository:

- **Review loop.** A colleague (a Trusted account on the repository, not an engineer by role) emails that one or more pull requests are ready, sometimes stacked (the second built on the first's branch), and asks for test suites they couldn't run. Jacob makes a worktree, starts a session and runs `/code-review <PR URL>, address the findings`. He accepts almost every finding, and always wants every Standards finding addressed.
- **Triage loop.** A report arrives (a staff member's bug email, a Slack message, a phone transcript, or a machine alert such as a blocked-deploy canary). Jacob makes a worktree, starts a session, runs `/triage <pasted report>` and nearly always accepts the recommended labels.

This file keeps the work examples generic on purpose: the thirdshift repository is public.

## Round 1 (answered)

**Q1.1 - What this grilling settles, and #1691's checklist.** #1691 asks for every core flow skill to run headless. `ask-matt` maps that flow (see "Research results"), so its map can be #1691's checklist: thirdshift already runs `/to-spec`, `/to-tickets`, `/implement`, `/tdd`, `/code-review` and `/pr` inside Runs (with a Spec run doing `/implement-spec`'s job), and `/improve-codebase-architecture` as an Architect run. The gaps are `/code-review` on its own (#706), `/triage` (#1323), `/diagnosing-bugs` on its own, `/retro`, `/to-questionnaire`, and the interactive ones (`/grill-with-docs`, `/wayfinder`, `/prototype`). The options:
- (a) Settle `review` (#706) and `triage` (#1323) here, each becoming its own Spec or Ticket. Rewrite #1691's body as the `ask-matt` checklist, with one linked issue per gap. Q1.18 to Q1.21 settle only how `/to-questionnaire`, `/diagnosing-bugs`, `/retro`, `/grill-with-docs` and `/prototype` touch these two.
- (b) Design a general "run any skill headless" Command under #1691 now, with review and triage as its first two users.
- (c) Make review a kind of triage: `/triage` already treats a PR as "an issue with attached code", checking it out and running tests to verify it, then writing a brief. One `thirdshift triage` Command would take a Report or a PR.

*Recommended:* (a). The two loops have different inputs (a pull request vs. a Report), different outputs (commits on someone else's branch vs. new issues) and different trust problems, so a general Command would be designed around two examples that don't share much. (c) is closer than it looks, but triage on a PR verifies and briefs; it doesn't fix findings, which is the work you want gone. Its verification step (running the tests the author couldn't) still belongs in a review, and Q1.9 covers it.

*Answer:* Let's skip wayfinder for now, as I'm not using that lately. I agree with the recommendation. I think I will want the ability to have flags for the triaging which maybe lets the caller control whether or not it sort of just goes ahead with With filing the issues as recommended or whether it just returns the recommendation. I think that could be useful. Yeah. 'Cause that for example in the get in an email about a canary flat finding or something, it it would be useful, there to just have a more shaped initial notification about that issue. But in but there's a chance that I wouldn't want the issues to be filed as such. or maybe I would. and so I guess that's where the flag could be useful.

Maybe the flag controls whether the skills run Headless or not. And or like a more headless or just the regular version of the skill is run. Or or m or more regular, maybe. Yeah. 

*Checked:* thirdshift has no attended mode today: every session it starts is unattended, and the glossary's opening line defines the factory that way. Q2.1 takes up the modes.

**Q1.2 - A separate issue for reading reports from Slack and email.** You want the "watch an inbox and triage what arrives" idea on file but out of scope. File a new `needs-triage` issue for it now, linked from #1323, that says only the idea: thirdshift (or something outside it) reads reports from Slack or email and feeds them to `thirdshift triage`.

*Recommended:* yes, file it now, linked both ways, with no design in it.

*Answer:* agreed

*Checked:* filed as #1693 (`enhancement`, `needs-triage`), linked from #1323 and naming #1691.

**Q1.3 - What a review delivers.** Your hand loop ends with fixes made in a worktree. Where do they go? The options:
- (a) Commits pushed onto the author's PR branch, plus one PR comment listing what was fixed and every **Unaddressed finding** with what refutes it.
- (b) A new PR based on the author's branch, so the author merges the fixes into their own PR.
- (c) A PR comment or GitHub review only, no code changes.

*Recommended:* (a). It is what "address the findings" does by hand, the author's PR stays the one thing you review, and a same-repository branch accepts the push. (b) doubles the PRs per review, and the stacked case would need PRs on PRs on PRs. (c) leaves the fixing to the author, which is the work you want gone.

*Answer:* agreed, probably with inspiration from or extension or modularization of how ticket run implement already works.

**Q1.4 - Which findings get fixed.** A Ticket run addresses Standards and Spec findings, listing the rest as Unaddressed findings. A colleague's PR often departs from its issue on purpose and says so in its body (a helper dropped because a rule changed). The options:
- (a) Same as a Ticket run: fix every finding it can't refute, and count the PR body's stated reasons as a refutation of a Spec finding.
- (b) Fix Standards findings only; report Spec findings without touching them.
- (c) Fix nothing on its own judgement beyond Standards findings, but file a `needs-triage` issue for each Spec finding.

*Recommended:* (a). It matches what you accept by hand, and a deliberate change the author explained is exactly the "plan decision" an Unaddressed finding may cite.

*Answer:* agreed. Hopefully I can get Holly in a better habit of writing a spec for things first going forward and have those connected to the pull requests.

**Q1.5 - What the Spec axis reviews against.** A Ticket run's Spec axis reads the issue it implements. A colleague's PR may close an issue, link several, or link none. The options:
- (a) Every issue the PR links as closing, plus the PR body; with no linked issue, the PR body alone.
- (b) Only linked closing issues; with none, skip the Spec axis and say so in the comment.

*Recommended:* (a). The PR body is where your colleague writes what changed and why, so it is the best spec available when no issue is linked.

*Answer:* agreed - when an issue is linked though maybe the PR body can be more ignored in favor of the issue description - in more standard /triage -> shaped ticket issue -> /implement fashion

**Q1.6 - How a review is started.** The options:
- (a) Both: `thirdshift review <PR URL>` by hand, and a Pass form, `thirdshift review --author <login>`, for cron, which reviews one waiting PR per invocation, as a Pickup run takes one issue.
- (b) The PR URL form only; cron is a script you write around `gh pr list`.
- (c) The Pass form only.

*Recommended:* (a). The hand form is your loop today and the easiest to test; the Pass form is what speeds up delivery, and one PR per invocation keeps it under the **Command limit** like every other Pass.

*Answer:* agreed

**Q1.7 - Which PRs a review will touch.** Reviewing runs an agent over someone else's code and discussion, and Q1.3(a) pushes to their branch. The options:
- (a) Only open, non-draft PRs whose head branch is in the same repository and whose author is a **Trusted account**; refuse forks, drafts, closed and merged PRs, with a message saying why.
- (b) As (a) but allow forks, delivering by Q1.3(b) for them.
- (c) Any PR the token can read.

*Recommended:* (a). It covers your colleague's PRs and keeps the same trust rule a Spec run already uses for Tickets. Forks can be a later issue if one ever matters.

*Answer:* agreed

**Q1.8 - When a PR is reviewed again.** Under the Pass form, a PR keeps showing up in the author's open list after it is reviewed. The options:
- (a) Review a PR once per head commit: a new push from the author makes it eligible again, while thirdshift's own fix commits don't.
- (b) Review a PR once, ever, unless asked by hand with its URL.
- (c) Mark reviewed PRs with a label (such as `thirdshift-reviewed`) and skip labelled ones; the author or you remove the label to ask again.

*Recommended:* (a). A colleague who pushes fixes after your feedback gets a fresh review without anyone touching labels. The open question it leaves is how thirdshift tells its own commits apart, which a later round can settle once Q1.3 is answered.

*Answer:* agreed

**Q1.9 - Tests the author couldn't run.** Your colleague asked for Integration and Functional suites they couldn't run from a worktree. The options:
- (a) The review session runs what the repository's agent instructions say to run for final validation, no more, and the comment says what ran, what passed and what it could not run.
- (b) A per-repository list of extra commands in the **User config** that every review runs (such as the slower suites), reported in the comment.
- (c) Running tests stays with you; the review only reviews and fixes.

*Recommended:* (b), with (a) as what happens when the list is empty. Suites that need a database or credentials are exactly the ones a worktree session skips unless told, and the machine running the review is where you'd set them up once. One condition either way: some repositories' suites load code by an absolute path to the main checkout, so run from a worktree they pass without testing the branch. A suite the repository's instructions say isn't valid from a worktree must be reported "not run against the branch", never green. A per-run bind mount of the worktree at that path (the host already has `bwrap`) could make them valid without touching the real checkout, but that is the repository's problem to solve, not thirdshift's.

*Answer:* agree

**Q1.10 - How you hear about a finished review.** The options:
- (a) A **Run notification** like any other Command, and an **Inbox** row in Ready for review until the PR is merged or closed.
- (b) The PR comment only; GitHub's own email tells you.

*Recommended:* (a). It is where every other Command already reports, and the Inbox row keeps a reviewed-but-unmerged PR in front of you.

*Answer:* agree

**Q1.11 - How a report reaches `thirdshift triage`.** The options:
- (a) Standard input, or a file named with a flag, run in the repository's **Launch directory** like other Commands; e.g. `pbpaste | thirdshift triage` or `thirdshift triage --file report.txt`.
- (b) A positional argument holding the text.
- (c) An editor opened for you to paste into.

*Recommended:* (a). Standard input suits both you pasting and a canary script piping its alert in, and a file suits long reports; a positional argument breaks on quotes and length.

*Answer:* agree

**Q1.12 - Which labels triage may apply unattended.** You nearly always accept the recommended labels. But a `ready-for-agent` label lets the next **Pickup run** build and possibly **Self-merge** the issue, so an unattended triage of a canary email could end in code merged with no one reading anything. The options:
- (a) Apply every recommended label, `ready-for-agent` included.
- (b) Apply every recommended label except `ready-for-agent`, which needs a flag or a **User config** setting; without it, the issue gets `needs-triage` and the comment says `ready-for-agent` was recommended.
- (c) Never apply `ready-for-agent`; triage only files.

*Recommended:* (b). It matches your preference for opt-in autonomy: the default saves the shaping work, and turning on the full chain is one deliberate setting. It also follows the repository's triage publication procedure, which treats readiness as a decision. (Since Q1.1's answer, this question and Q1.16 and Q1.17 apply to the mode that files issues; Q2.1 asks about a mode that only recommends.)

*Answer:* agree

**Q1.13 - New issues only, or existing ones too.** The `/triage` skill also works through existing `needs-triage` issues. #1323 says "unattended triage" without saying which. The options:
- (a) This grilling covers reports in, issues out. Triaging existing `needs-triage` issues is a later issue.
- (b) Both now.

*Recommended:* (a). Report-in is the loop you described; re-triaging existing issues raises its own questions (who wrote them, whether a human already decided).

*Answer:* agreed, file that separate issue to capture it to consider at some other date or session

*Checked:* filed as #1695 (`enhancement`, `needs-triage`), naming #1691.

**Q1.14 - How far triage goes on a machine alert.** For a canary like a blocked deploy, you'd like the email to arrive with an analysis or even a PR ready. The options:
- (a) Triage investigates the repository (read-only, no credentials, no production access) and puts its diagnosis in the issue; a PR comes only by the ordinary route, a **Ticket run** on the issue once it is ready.
- (b) Triage itself may start a Ticket run when it labels an issue `ready-for-agent`.
- (c) Triage only shapes the report; no investigation.

*Recommended:* (a). It gets you "analysis ready" from the alert, keeps credentials and production decisions with you, and with Q1.12's opt-in plus Pickup, a PR can still follow without a new path. `/triage`'s own "verify the claim" step already reproduces a bug from the reporter's steps, so (a) is that step with read-only limits; when it can't reproduce without credentials, the issue says so and gets `ready-for-human`.

*Answer:* agreed. I think I even want a way to for the agent to be able to explore possi Possibly the whole system. even outside of the work trigger it's launched within so this might be different flags on the unattended harness calls that can let them do this. but that I would want to be an opt-in flag turn on type feature, but I could see it being useful for a broader kind of having an agent do a broader investigation that's even outside the confines of just one directory. though I guess maybe I could kind of do that with Just like launching the agent at a more root level directory or something and then it it's kind of workspace would be the whole thing but I can't even imagine as well a case where I'd want the agent to be able to SSH into other servers in the network you know as part of its triage. that's that could be really useful and it's kind of what I do in the interactive sessions already.

*Checked:* "read-only, worktree only" would be a prompt rule, not a wall. Claude sessions run in `--permission-mode auto`, and Codex sessions with `--dangerously-bypass-approvals-and-sandbox`; ADR 0012 says the worktree is the only boundary. Nothing stops a session reading outside it today, or running `ssh` with your keys. Q3.1 takes this up.

**Q1.15 - Names.** The glossary names Commands "X run". Proposed: **Review run**, a Command that reviews another author's pull request and addresses its findings; **Triage run**, a Command that turns a report into triaged issues; and **Report**, the text a Triage run is given. To avoid: "PR review" (it reads as the GitHub feature), "code review run", and "intake".

*Recommended:* yes to all three names.

*Answer:* agree

**Q1.16 - A Report becomes an issue first.** `/triage` works on issues that already exist; it has no "raw text in" mode. The options:
- (a) The Triage run files the Report as a `needs-triage` issue first (verbatim, with the AI disclaimer and who reported it and how), then triages that issue as `/triage` would. A Report holding several separate problems becomes one issue each, cross-linked.
- (b) The Triage run triages the Report in its session and files only the shaped issues, with the Report quoted in each.

*Recommended:* (a). The raw Report stays on record where the triage can be checked against it, and each step from then on is the `/triage` you already use, which keeps the new skill small.

*Answer:* Agree - just clarifying though that my typical process is > worktree > agent session > prompt: `/triage <Report>` and then use the triage skill to have the agent create the issue(s) accordingly.

*Checked:* that process never files the raw Report on its own: the agent files the shaped issue(s) directly, which is closer to option (b). Q3.2 asks which you mean.

**Q1.17 - What replaces triage's pauses.** `/triage` stops twice for you: after its recommendation ("Wait for direction") and before grilling a request that needs fleshing out. Unattended, nobody answers. The options:
- (a) It applies its own recommendation, within Q1.12's label limits. Where it would grill, it doesn't: the issue gets `needs-info` (the reporter must answer) or `ready-for-human` (you must decide), with the questions written as Q1.18 says.
- (b) It always stops at `needs-triage` with its recommendation in a comment, and you apply it.

*Recommended:* (a). (b) leaves you the same click per issue you want gone; (a) only stops where a person really has to answer something, and says who.

*Answer:* agree

**Q1.18 - `/to-questionnaire` for whoever must answer.** A Triage run's `needs-info` comment on a private GitHub issue never reaches a reporter who isn't on GitHub, such as a staff member who emailed. More generally, any Process can reach something only a person can answer: `ready-for-human`, a **Paused Command**, an **Unaddressed finding** waiting on the **Day shift**. `/to-questionnaire` writes the questions aimed at the person who has the answer. The options:
- (a) Now, for Triage runs only: on `needs-info` or `ready-for-human`, write a questionnaire, post it on the issue, and put it in the **Run notification** ready for you to forward (thirdshift never emails a reporter itself). File an issue under #1691 for a questionnaire Step any Process can use when it stops on a human.
- (b) A catalogue questionnaire Step for every Process now.
- (c) Leave it to the later #1691 issue.

*Recommended:* (a). Triage is where the non-GitHub reporter shows up today, and forwarding through you keeps thirdshift from writing to people outside the repository.

*Answer:* agree

**Q1.19 - `/diagnosing-bugs` on its own.** For a confirmed bug or a canary, the step after triage is often a diagnosis: one command that fails on the bug, then a fix with a regression test. thirdshift embeds the skill but no Command starts it on its own. The options:
- (a) A separate issue under #1691 for a Diagnosis Command on an issue, grilled on its own. The Triage run here stops at Q1.14's read-only investigation, and a PR comes through a Ticket run.
- (b) A Triage run starts a diagnosis itself on a bug it confirms, opening a PR when it finds the fix.

*Recommended:* (a). A diagnosis that changes code needs a worktree, a branch, delivery and a trust decision, which is a Ticket run's machinery, not triage's. Whether a Ticket run on a bug should just drive `/diagnosing-bugs` instead of `/implement` is a question for that issue.

*Answer:* agree

**Q1.20 - `/retro` after reviews.** If Review runs keep fixing the same Standards findings on one author's PRs, the lasting fix is a coding standard or an automated check that catches them first, which is `/retro`'s job. The options:
- (a) A separate issue under #1691 for `thirdshift retro`, which reads the findings of the repository's recent Review runs (and later other Commands) and files a `needs-triage` issue proposing standards or checks. It never edits the standards itself.
- (b) A Retro Step at the end of every Review run.
- (c) No retro for now.

*Recommended:* (a). One review shows a mistake; a pattern needs several, and changing the standards is a one-way door for every later review, so it stays a proposal you decide on.

*Answer:* agree

**Q1.21 - `/grill-with-docs` and `/prototype` headless.** An Architect run already does a kind of headless grilling: its prompt tells the session to "settle its design yourself" before `/to-spec` and `/to-tickets`, and allows a throwaway prototype in its worktree. But it asks nobody anything, runs no grilling skill, may not touch `GLOSSARY.md` or ADRs, and keeps no prototype (see "Research results"). There are two ways a grilling could run headless: a **self-grilling**, where the session answers its own questions with its recommendations (what an Architect run does now), and an **unattended grilling**, where thirdshift writes a round to a grilling file or issue, stops, and resumes once you mark it answered, as this file works. The options:
- (a) A separate issue under #1691 for an unattended grilling: a Command that runs `/grill-with-docs` on an issue one round at a time, ending each round with a **Run notification** and resuming on "answered". It may detour through `/prototype` when a question needs runnable code, keeping the prototype on a `prototype/<name>` branch as `/prototype` says. A Triage run's `ready-for-human` and an Architect run's not-Strong idea become things it can pick up.
- (b) As (a), but also make the Architect run's self-grilling an explicit `/grilling` Step that writes its questions and chosen answers to the plan, so you can see what it settled for you.
- (c) Leave grilling and prototyping with you.

*Recommended:* (a), with (b) as a question for that issue. An unattended grilling is where the interactive skills fit: the round format already works without you in the session, and it's the natural next step after Q1.17 and Q1.18 decide that a Triage run stops when a person must answer.

*Answer:* I don't see the need for the unattended grilling as that kind of asynchronous back and forth sounds a little bit slow and I just don't feel the need for that right now and I think I'd rather just do an interactive agent session that's more synchronous, but making the self-grilling like kind of like how Architect Run does it a little bit more composable or as something in the catalog which can be used potentially in a custom process. that sounds useful.

But again that can probably go into a separate issue and outside of the scope of this particular spectr or specs that are more focused on primarily on review and triage. But unless it's important here. 

## Round 2 (answered)

**Q2.1 - Triage modes.** Q1.1's answer asks for a flag choosing between filing issues as recommended and only returning the recommendation, and wonders about running "the regular version of the skill". The options:
- (a) Two modes. By default a Triage run files: it acts as Q1.12, Q1.16 and Q1.17 settle. With `--recommend-only`, it writes nothing to GitHub and sends its recommendation (category, state, the issue or issues it would file, and its investigation) in the **Run notification** and on standard output. A canary script passes `--recommend-only` until you trust it to file.
- (b) As (a), but recommend-only is the default and `--file` makes it act.
- (c) As (a), plus an attended mode, `--attended`, where thirdshift makes the worktree and opens an ordinary interactive session running `/triage` on the Report, to save the setup steps when you want to drive.

*Recommended:* (a). Filing an issue is cheap to undo (close it), and Q1.12 already holds back the one label that starts work unattended, so filing by default saves the click you always make. (c) is useful but is a new kind of thing for thirdshift, which only runs unattended sessions today; it deserves its own issue under #1691 rather than riding in here.

*Answer:* agreed

**Q2.2 - The same switch for reviews.** A Review run's equivalent of recommend-only is Q1.3(c): post the findings, push nothing. The options:
- (a) Yes: `thirdshift review --recommend-only` posts the review as a PR comment and pushes nothing; the default pushes fixes as Q1.3 settled.
- (b) No: a Review run always pushes.

*Recommended:* (a). Same flag name and meaning across both Commands, and it is the safe first setting for the cron Pass while you build trust in it.

*Answer:* agree

**Q2.3 - How a Review run is built.** Q1.3's answer asks for reuse of the Ticket run's machinery. A Ticket run reviews, addresses findings and delivers its own PR; a Review run reviews and addresses findings the same way, but delivers by pushing to someone else's branch and commenting. A **Custom process** can't start on a PR today (see "Research results"). The options:
- (a) A **Built-in process** made of catalogue **Steps**: the Ticket run's review-and-address work becomes a Step of its own, and a new delivery Step pushes to the PR's branch and comments. Processes learn to start on a PR, so a Custom process can too.
- (b) A Command of its own that calls the shared review-and-address code directly, with no Process file.

*Recommended:* (a). It is what Built-in processes are for, it makes the review-and-address Step reusable (a Spec run's Spec review is close to it already), and it opens PR-started Custom processes for the rest of #1691. It costs more up front than (b).

*Answer:* agree

**Q2.4 - Telling its own commits apart.** Q1.8 settled one review per head commit, not counting thirdshift's own fix commits. Several machines may run the Pass. The options:
- (a) The Review run's PR comment carries a hidden marker naming the head it reviewed and the head after its own push; the Pass skips a PR whose current head is either. Any machine can read it.
- (b) The **Command record** on the machine stores the heads; another machine doesn't see them.

*Recommended:* (a). GitHub is the one place every machine reads, which is how Claims already work across machines.

*Answer:* agree

**Q2.5 - Stacked PRs.** Your colleague's second PR was built on the first's branch. A Review run fixing the lower PR leaves the upper one without those fixes. The options:
- (a) Review each PR against its own base (the upper against the lower's branch), lowest in the stack first. Never merge or rebase the upper; its comment says the lower PR got fixes and the upper needs updating before merge.
- (b) After fixing the lower PR, merge its branch into the upper's and then review the upper.

*Recommended:* (a). Rewriting the shape of someone's stack is a bigger intrusion than adding commits to one branch, and the author or you merge in order anyway.

*Answer:* agree

**Q2.6 - A linked issue against the PR body.** Q1.5's answer puts a linked issue first and the PR body second. Q1.4 settled that the PR body's reasons can refute a Spec finding. With an issue linked, a PR that departs from it on purpose and says why only in its body is the case where the two collide. The options:
- (a) The issue is the spec. A departure the PR body explains is not fixed (reverting a colleague's deliberate change would be wrong) but is listed as an **Unaddressed finding** that cites the body's reason and says the issue and the PR disagree, for you to decide in your review. No `needs-triage` issue is filed for it.
- (b) The issue is the spec and the body counts for nothing: the departure is fixed back to the issue.
- (c) The body's reason refutes the finding outright, as with no linked issue.

*Recommended:* (a). It keeps the issue authoritative, as you asked, without undoing work the author meant, and the disagreement lands in the review you're about to do anyway.

*Answer:* agree - this follows the way the /to-spec -> /to-tickets -> /implement flow works, which is similar to the /triage -> issue -> /implement flow

**Q2.7 - Security review on a Review run.** Your colleague's PRs sometimes include security fixes. A Ticket run runs a **Security review** when a flag or the **User config** asks. The options:
- (a) The same flag and User config setting ask for a Security review on a Review run, after it addresses its findings, run on `security.harness` like any other.
- (b) A Review run never runs one.

*Recommended:* (a). Same switch, same meaning, and a colleague's change is no less in need of it than an agent's.

*Answer:* agree

## Round 3 (answered)

Q2.2 above is still open: it had no answer when this round was written.

**Q3.1 - How far an investigation may reach.** Q1.14's answer wants an opt-in wider investigation: the whole machine, and possibly SSH into other servers, as you do interactively. Today the worktree is the only boundary, and it is a prompt rule rather than a sandbox (see the note under Q1.14). The other half of the trade is the input: a Triage run reads a Report that anyone who can email a staff member could have shaped, so a session that can SSH with your keys is exactly what a prompt injection would want. The options:
- (a) By default the prompt keeps the session to the worktree. An opt-in flag (or a per-repository **User config** setting) lets it read the whole machine, enforced by a sandbox that mounts the filesystem read-only and only the worktree writable (`bwrap`, as the Security sandbox already uses). SSH to other servers is its own issue under #1691.
- (b) As (a), plus SSH now, to hosts named in the User config, on the same opt-in flag.
- (c) The opt-in flag only changes the prompt: it may read anywhere and SSH anywhere your account can, with nothing enforcing read-only.

*Recommended:* (a). Read-only access to the whole machine gets most of the "explore the system" value, and a sandbox makes "read-only" true rather than hoped for. SSH from an unattended session that reads untrusted text needs its own decision about which hosts, which keys and which commands, so a separate issue suits it better than a flag here.

*Answer:* agreed

**Q3.2 - Raw Report on file, or shaped issues only.** You agreed to Q1.16(a), filing the Report as an issue first, but your process files the shaped issue(s) directly and never the raw Report. The options:
- (a) Shaped issues only: each issue the Triage run files is the shaped one, with the Report quoted verbatim in a collapsed section at the bottom, and where it came from. A Report that holds several problems gives several issues, cross-linked, each quoting it.
- (b) As Q1.16(a) said: the raw Report becomes an issue first, then the shaped issues are filed from it, or it is edited into the first one.

*Recommended:* (a). It is what you do by hand, it leaves no extra issue to close, and the quoted Report still lets anyone check the triage against what was said.

*Answer:* agree

**Q3.3 - Reserving a PR across machines.** A **Claim** reserves an issue with the `in-progress` label so two machines don't work on it at once. A Review run's Pass on two machines could pick the same PR. The options:
- (a) The same `in-progress` label, put on the PR when the Review run starts and taken off when it ends; the Pass skips a PR that has it, and a **Pickup run**'s **Sweep** clears it from closed PRs as it does from closed issues.
- (b) A marker comment posted at the start, which the Pass reads.

*Recommended:* (a). It is the reservation thirdshift already understands, shows on GitHub at a glance, and the Sweep already knows how to clean it up.

*Answer:* agree

**Q3.4 - The author pushes during a review.** Q1.3 settled pushing to the author's branch without force. If the author pushes while the Review run works, its push is rejected. The options:
- (a) Don't push. Post the review as `--recommend-only` would (findings, with the fixes it made described but not applied) and say the branch moved. The new head is then eligible for a fresh review under Q1.8.
- (b) Rebase its fix commits onto the new head, re-run the tests, and push once; fall back to (a) if that fails.

*Recommended:* (a). The author's new commits weren't reviewed, so fixes rebased onto them were checked against code nobody looked at; the fresh review that follows covers both.

*Answer:* agree

**Q3.5 - How #706 and #1323 are published.** The options:
- (a) Each becomes a **Spec** with **Tickets**, published by `to-spec` and `to-tickets` at the end of this grilling, with #706 first because the cron Pass is where the time goes. The follow-up issues (questionnaire Step, Diagnosis Command, `thirdshift retro`, self-grilling Step, attended mode, SSH from Q3.1 if (a)) are filed at the same time, and #1691's body is rewritten as the checklist that links them all.
- (b) As (a), but publish #1323 first: a Triage run needs no PR-started Processes, so it is the smaller change.

*Recommended:* (a). Both are multi-session, and the Review run's PR-started Built-in process (Q2.3) is the larger change, which is the better reason to start it first: the triage work can run alongside it without sharing much.

*Answer:* agree

## Round 4 (answered)

Q2.2 above is still open: whether `--recommend-only` applies to a Review run too.

**Q4.1 - A Report about something already filed.** A canary fires again for the same failure, or a second staff member reports the same problem. Filing a new issue each time buries the first. `/triage` checks whether the behaviour is already built, but not whether an open issue already covers the Report. The options:
- (a) Before filing, the Triage run searches the repository's open issues. When one already covers the Report, it comments there instead, quoting the Report and adding anything new it found, and changes no labels. The Run notification links that issue. Under `--recommend-only`, it says which issue it would have commented on.
- (b) Always file a new issue, linking any open issue that looks like the same thing.

*Recommended:* (a). A repeating alert is the common case for the canary hook, and one issue gathering every occurrence is what you'd make by hand. Leaving the labels alone keeps a repeat from undoing a decision you already made on that issue.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- `--recommend-only` works on a Review run too: it posts the review as a PR comment and pushes nothing. The default pushes fixes (Q2.2).
- Before filing, a Triage run looks for an open issue that already covers the Report. If it finds one, it comments there, quoting the Report and anything new, and changes no labels (Q4.1).

- A Triage run may read the whole machine only when a flag or per-repository **User config** setting allows it, and then inside a sandbox that mounts the filesystem read-only with only the worktree writable. Without it, the prompt keeps it to the worktree (Q3.1).
- SSH from a session to other servers gets its own issue under #1691 (Q3.1).
- A Triage run files the shaped issues only, each quoting the Report verbatim in a collapsed section with where it came from; Q1.16's raw-Report issue is dropped (Q3.2).
- A Review run reserves its PR with the `in-progress` label, and the **Sweep** clears it from closed PRs (Q3.3).
- When the PR's branch moves during a Review run, it pushes nothing, posts its findings with the fixes described, and the new head gets its own review (Q3.4).
- #706 and #1323 are each published as a **Spec** with **Tickets**, #706 first, at the end of this grilling. The follow-up issues are filed then, and #1691's body becomes the checklist linking them (Q3.5).
- A Triage run is started with a Report, so it is never a **Pass** and never skipped for the **Command limit**, like any Command a person types; the email flag lets a canary script choose who gets its Run notification (Q1.11, Q1.18).

- A Report reaches a Triage run on standard input or through `--file`, run in the repository's **Launch directory** (Q1.11).
- A Triage run applies `ready-for-agent` only when a flag or the **User config** allows it; otherwise the issue gets `needs-triage` and says `ready-for-agent` was recommended (Q1.12).
- A Triage run takes Reports only; triaging existing issues is #1695 (Q1.13).
- A Triage run investigates and puts its diagnosis in the issue. A PR comes only through a **Ticket run**. A bug it can't reproduce without credentials gets `ready-for-human`, and the issue says why (Q1.14, Q3.1 asks how far it may reach).
- **Review run**, **Triage run** and **Report** are the glossary's terms (Q1.15).
- Unattended, a Triage run applies its own recommendation; where it would grill, it labels the issue `needs-info` or `ready-for-human` instead (Q1.17).
- On `needs-info` or `ready-for-human`, a Triage run writes a questionnaire with `/to-questionnaire`, posts it on the issue, and puts it in the **Run notification** for you to forward. thirdshift never writes to a reporter itself (Q1.18).
- A questionnaire Step any Process can use, a Diagnosis Command and `thirdshift retro` each get an issue under #1691 (Q1.18, Q1.19, Q1.20).
- No unattended grilling. Instead, the Architect run's self-grilling becomes a catalogue **Step** a **Custom process** can use, with its own issue under #1691; the Architect run would use that Step too (Q1.21).
- A `ready-for-human` issue from a Triage run waits for you to take it into an interactive session (Q1.21).
- The Review run is a **Built-in process**. Processes learn to start on a PR, so a Custom process can too (Q2.3).
- The Review run's PR comment carries a hidden marker naming the head it reviewed and the head after its push; the Pass skips a PR whose head is either (Q2.4).
- A Review run reviews each PR against its own base, lowest in a stack first, and never merges or rebases another PR; the upper PR's comment says it needs updating (Q2.5).
- The Pass skips a PR whose base is another open PR until that PR has been reviewed at its current head (Q2.5).
- With an issue linked, the issue is the spec. A departure the PR body explains is listed as an **Unaddressed finding** for you to decide, neither fixed nor filed as `needs-triage` (Q2.6).
- The Security review flag and **User config** setting apply to a Review run, after it addresses its findings (Q2.7).

- `/wayfinder` is left off #1691's checklist for now (Q1.1).
- #1691's body becomes the `ask-matt` checklist, with one linked issue per gap other than `/wayfinder`; #706 and #1323 are designed here (Q1.1).
- A Review run reuses the Ticket run's review-and-address work; it doesn't get a second implementation of it (Q1.3, Q2.3 asks how).
- A Review run pushes only to the PR's own head branch, in the same repository, and never force-pushes (Q1.3, Q1.7).
- When a PR links no issue, its Review run comment says the Spec axis reviewed against the PR body alone, which also reminds the author that a linked issue makes a better spec (Q1.4, Q1.5).
- A Review run reads the PR body, its linked issues, and comments from **Trusted accounts** only; anything else on the PR is not read (Q1.7).
- The Pass form never reviews a PR by anyone other than the `--author` it was given; `--author` is required for it (Q1.6).
- A Review run holds a place under the **Command limit** like any other Command (Q1.6).
- A suite the repository says isn't valid from a worktree is reported "not run against the branch" (Q1.9).
- A Review run shows in the **Inbox** under Ready for review until the PR is merged or closed (Q1.10).
- A Triage run files issues by default; `--recommend-only` writes nothing to GitHub and puts the recommendation in the Run notification and on standard output (Q2.1).
- An attended mode, where thirdshift makes the worktree and opens an interactive `/triage` session, gets its own issue under #1691 (Q2.1).

## Research results (2026-11-03)

- #706's triage notes ask for: the result's form, the review axes, which head is reviewed, who authorizes writes, forks, changing heads, closed or merged PRs, credentials and untrusted instructions, and keeping it apart from #429's Second opinion. Q1.3 to Q1.8 cover these. Confirmed by reading the issue.
- #1691 and #1323 have no body beyond their titles. Confirmed by reading the issues.
- A Ticket run reviews with the embedded `thirdshift-code-review` skill against the Base branch, then addresses findings: a behaviour finding's test runs first, and a passing run may decline it. Unaddressed findings go in the PR body, and the session itself files a `needs-triage` issue for one a Spec or the Day shift must decide. Every review prompt takes an Issue URL and a thirdshift branch; nothing points one at another author's PR branch. Confirmed by source (`src/prompt.rs:207-254`).
- PR handling rejects fork PRs and any PR whose head isn't thirdshift's expected branch, and there is no author filter on PRs. Trusted account checks cover issues only, not PRs. Confirmed by source (`src/pull_request.rs:603-608`, `src/github.rs:692-718`, `src/trust.rs:85-136`).
- A **Custom process** starts from a name and an optional issue only. A Step has no free-text input: only `THIRDSHIFT_ISSUE_URL` and `THIRDSHIFT_RESULTS_DIR` reach it. So neither a PR URL (review) nor a pasted Report (triage) can be passed to a Custom process today. Confirmed by source (`src/args.rs:29-44`, `src/processes.rs:77-79`, `src/processes.rs:3437-3467`).
- No triage skill is embedded. The embedded skills are code-review, codebase-design, diagnosing-bugs, implement, improve-codebase-architecture, pr, resolving-merge-conflicts, security-audit, tdd, to-spec and to-tickets, so a Triage run needs a `thirdshift-triage` skill added. Code already files issues with labels for Security fix publication, Base CI failures and Architect ideas. Confirmed by source (`src/skills.rs:27`, `src/github.rs:597-621`).
- The installed `/triage` skill covers issues and, where a repository allows it, external PRs ("a PR is an issue with attached code"). It recommends and then waits for direction, verifies the claim (reproducing a bug, or checking out a PR and running its tests), grills when needed, then applies one category and one state role with a brief or triage notes. It has no mode for raw text. Confirmed by reading `~/.agents/skills/triage/SKILL.md`.
- `ask-matt` maps the flow: the main flow (`/grill-with-docs`, `/prototype` with `/handoff`, `/to-spec`, `/to-tickets`, `/implement` or `/implement-spec`, `/tdd`, `/code-review`, `/pr`, `/retro`), on-ramps (`/triage`, `/diagnosing-bugs`, `/wayfinder`), upkeep (`/improve-codebase-architecture`), and standalone skills including `/to-questionnaire`. Confirmed by reading `~/.agents/skills/ask-matt/SKILL.md`.
- An Architect run's Architecture review tells its session to "settle its design yourself" for a Strong recommendation, then publish with `/to-spec` and `/to-tickets`. It runs no grilling skill, may not edit `GLOSSARY.md` or ADRs, and may build a throwaway prototype in a worktree that is thrown away when the session ends. Confirmed by `prompts/architecture-review.md` and `skills/thirdshift-improve-codebase-architecture/SKILL.md:13`.
- A Run notification can be sent for a Command with no issue. Its recipient is the flag's address, then the per-repository address for the Launch directory's origin, then `email.to`. Confirmed by source (`src/notification.rs:26-60`).

## Waiting on research

- Nothing.
s