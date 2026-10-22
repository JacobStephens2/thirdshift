# Grilling: #941, plain words for metaphorical terms, with #807, #805 and #713

Issues: https://github.com/JacobStephens2/thirdshift/issues/941 (glossary terms), https://github.com/JacobStephens2/thirdshift/issues/807 (the site's voice), https://github.com/JacobStephens2/thirdshift/issues/805 (Run and Merge run), https://github.com/JacobStephens2/thirdshift/issues/713 (your own agent subscriptions)
Research: see "Research results", with each source's link

## Round 1 (answered)

**Q1.1 - The homepage's print theme (#807).** The homepage is built on a print shop: "Your issues go to press overnight", a press line whose units are named C, M, Y and K, "Misprint: a Failed run", "Delivery pile", "Proof sheets", "Pulled from the press" and "the press hall". How it works and Architecture mostly use plain words already. The options:
- (a) Keep the name, the colophon's story and the look (type, colours and the press drawing). Make every heading, label and sentence say plainly what it means. Print words stay only where the colophon explains them.
- (b) As (a), and also retire the press drawing and the print look, for a plain design.
- (c) Keep the homepage as it is, and close #807.

*Recommended:* (a). The theme makes the site memorable and the colophon gives it meaning, but a reader shouldn't have to decode "Misprint" to learn that a Run failed. Round 2 would list each phrase with a proposed plain replacement, the headline included.

*Answer:* agree

**Q1.2 - Weeding and Fencing (#941).** Weeding is Architect runs started every few minutes, so that each starts as soon as the last one's plan merges. Fencing is Security runs started on a schedule with fixing allowed. Neither word appears in a command, setting or output, only in documentation and the site. Other tools call recurring agent work "automations" (GitHub, Devin, Cursor, Factory) or "scheduled tasks" (Codex, Jules, Claude Code), and none has a single word for either setup. The options:
- (a) Retire both words and describe the setups plainly, such as "Architect runs every few minutes" and "scheduled Security runs with fixing on". The manual's two sections get plain headings.
- (b) Replace each with a new single term. No tool offers one, so it would be coined.
- (c) Keep both.

*Recommended:* (a). Each is a schedule for a run type that already has a name, so the plain phrase is short, needs no definition, and reads like the other tools' "scheduled tasks". Retiring them means edits to about a dozen documentation and site files.

*Answer:* agree

*Checked:* the manual links to its own `#weeding` and `#fencing` sections, and the README's old-bookmark anchors of those names point there, so the links move with the new headings.

**Q1.3 - The other figurative terms.** Besides Weeding, Fencing, Day shift and Run (Q1.2, Q1.6 and Q1.7), a few glossary terms have a figurative edge: Factory and Factory skills, Sweep, Pass, Claim and Harness. The options:
- (a) Keep them all. "Software factory" and "harness" are the industry's own words, and Sweep, Pass and Claim are ordinary English used in their ordinary sense.
- (b) Name the ones to review in Round 2.

*Recommended:* (a).

*Answer:* agree

**Q1.4 - Your own agent subscriptions (#713).** thirdshift runs the agent CLIs already installed and logged in as you, so each Run is billed however that CLI is logged in. The manual documents plan billing only for Claude Code. The triage note on #713 warned that "subscription-based factory" could read as thirdshift itself being a paid subscription. The options:
- (a) Say it in the homepage's opening paragraph and the README's first lines, in words like: "thirdshift drives the coding agents you already use, logged in as you, so Runs use the plan or account each one is logged in with. thirdshift itself is free and open source." Mention plan billing by name only where the manual documents it (Claude Code today).
- (b) Say it only under Requirements.
- (c) Leave the positioning as it is, and close #713.

*Recommended:* (a). Cost is an early question for anyone running agents overnight, and the honest answer is a selling point. Limiting the claim to what the manual documents avoids promising plan billing for a Harness that doesn't offer it.

*Answer:* agree

**Q1.5 - #1095 runs before this is settled.** #1095, posted today as ready-for-agent, fixes the site's facts in its current voice: the Console, the sandbox status, a Failed run's lines and the manual links. A Pickup run may take it before this grilling ends. The options:
- (a) Let it run. Its facts survive a change of voice, and the rewrite builds on them.
- (b) Hold it, back at needs-triage, until the voice is decided, so the pages change once.

*Recommended:* (a). The pages are wrong today, and a second pass over them costs less than leaving them wrong meanwhile.

*Answer:* (a). You started a Run on #1095 on 22 October (said in chat).

**Q1.6 - Day shift (#941).** The Day shift is the humans' half of the workflow: shaping work into issues and reviewing the pull requests the factory delivers. It pairs with the product's name and appears 149 times, in the Session prompts and Factory skills too. No tool has a word for it. The nearest are AI Hero's "human-in-the-loop", which means humans pairing with the agent during a session, and its "AFK", which names the agent's side: left to run unattended. The options:
- (a) Keep "Day shift". The homepage explains it in its first paragraph, and nothing standard replaces it.
- (b) Replace it with a plain word, such as "maintainers".
- (c) Use AI Hero's "human-in-the-loop".

*Recommended:* (a). With no standard word, the #698 rule keeps thirdshift's own, and this one carries the name. Human-in-the-loop would say the wrong thing: nobody is in a thirdshift session's loop. "Maintainers" is plain but narrower than the Day shift, which includes whoever writes and reviews issues.

*Answer:* agree

**Q1.7 - Run and Merge run (#805).** A Run is one invocation on a single issue that isn't a Spec, ending in one pull request. A Merge run is a Run that Self-merges its pull request. On its own, "Run" is easy to misread: Spec runs, Pickup runs, Architect runs and Security runs are runs too, so "a Pickup run starts a Run" needs its capital R to make sense. The other tools offer no usable standard. GitHub, Jules, Devin and Factory call one issue-to-pull-request attempt a "session", and GitHub, Codex and Jules call it a "task". thirdshift already uses Session for one agent session, avoids "task", and has Eval task. "Ticket" matches AI Hero's Ticket and Factory's "Ticket to PR" starter, and the glossary's Ticket already covers a standalone issue. The options:
- (a) Rename Run to Ticket run, and fold Merge run into it: a Ticket run either leaves its pull request for review or Self-merges it, as the `merge` and `no-merge` options already say. Ticket run and Spec run then each name what they run on.
- (b) Rename Run to Ticket run, and keep Merge run for one that Self-merges.
- (c) Keep Run and Merge run.

*Recommended:* (a). Ticket run and Spec run read at once, and Self-merge already names what a Merge run adds. It's the largest rename here: "Run" appears about 2,300 times in 225 files, counting code. Round 2 would settle which uses change (the words people and agents read: glossary, manual, site, prompts and CLI output) and which can keep "run" (code identifiers, log paths and record fields).

*Answer:* Agree

*Checked:* Command records, the Activity log and `status --json` all store the kind as "Run", and the Console and `status` print it from there. Log folders are named `issue`, not `run`. Q2.2 decides which of these change.

## Round 2 (answered)

**Q2.1 - The homepage's phrases (#807).** Following Q1.1, each print phrase gets a plain one. The press drawing and its colours stay, with plain labels. The headline also appears on the share card, which gets re-rendered. Proposed, as current → new:
- Hero tag "Proof · pulled at 03:12 · unit 4 of 4" → removed.
- Headline "Your issues go to press overnight." → "Your issues become pull requests overnight."
- "You plan by day. The press runs at night." → "You plan by day. Agents work at night."
- "How a Run works" → "How a Ticket run works". The 12 October replay stays as recorded.
- In the press drawing, "FEEDER" and "DELIVERY PILE" are removed, leaving "ISSUE #663" and "PR #665 · MERGED". "UNIT 1 · 1 · C · PREPARE" → "STEP 1 · PREPARE", and likewise for Implement, Review and Finish, each keeping its colour.
- "Misprint: a Failed run" → "When a Ticket run fails".
- "Delivery pile · What comes off the press" → "Results · What you get back".
- "Plan view · the press hall · The factory grows" → "Workflows · Five workflows". On the floor plan, "PRESS HALL · PLAN" → "WORKFLOWS" and "LINE 1 · ONE RUN · INSTALLED" → "1 · TICKET RUN". "INSTALLED" goes, since every line is, and the Merge run's line goes, since Q1.7 folded it into the Ticket run.
- "Proof sheets · Pulled from the press" → "Examples · Real pull requests".
- "Colophon" → "About the name". Its story is unchanged.

Kept, as already plain or factory words: "Two shifts", "One issue in, one pull request out.", "Engineer the factory while the factory engineers the software." and "Before you run it · Requirements and platforms". The options:
- (a) Use the list as proposed.
- (b) Use it with changes: say which rows.

*Recommended:* (a).

*Answer:* agree

**Q2.2 - Which uses of "Run" change (#805).** After Q1.7, people and agents should read "Ticket run" wherever they meet the kind. Some uses are stored values that old records and scripts depend on: Command records, the Activity log and `status --json` all hold the kind as "Run". The options:
- (a) Change every word people and agents read: glossary, manual, README, site, Session prompts, Factory skills, CLI help and output, Run notifications and the Console's labels. Keep stored and machine-readable values as they are: the kind "Run" in Command records, the Activity log and JSON, the log folders, and code identifiers. Displays show the stored "Run" as "Ticket run".
- (b) As (a), and also store "Ticket run" in new records, the Activity log and JSON, with readers still accepting the old "Run".
- (c) As (b), and also rename code identifiers: types, modules and functions.

*Recommended:* (a). What people and agents read uses one name, old records and scripts keep working, and nothing needs migrating. Code identifiers can follow when Architect runs next touch that code.

*Answer:* agree

**Q2.3 - Terms built on "Run".** Q1.7's reason applies to compound terms too. "Failed run" means a failed Run specifically, and its Spec counterpart is already "Failed spec run". "Run notification" covers Ticket, Spec, Architect and Security runs. "Run metrics" names the queued #981 Spec, and is measured on Runs. The options:
- (a) "Failed run" becomes "Failed ticket run", pairing with "Failed spec run". "Run notification" keeps its name, since it covers every kind of run. "Run metrics" keeps its name, so #981 needs no rename, and its definition says it is measured on Ticket runs. Continuation and Recovery keep their names, with "Ticket run" in their definitions.
- (b) Keep "Failed run" for a failed Ticket run.
- (c) Make "Failed run" cover a failed Ticket run or Spec run, and retire "Failed spec run".

*Recommended:* (a). It keeps the pairs regular. A Failed run that means only a failed Ticket run would need its capital F to be told apart from a failed Pickup run, the same problem Q1.7 fixed.

*Answer:* agree

**Q2.4 - Old records and old names.** ADRs, research notes, release notes and finished grillings record what was true when they were written, and they say "Run", "Merge run", "Weeding" and "Fencing". The options:
- (a) Leave them as written. Each retired word goes into the _Avoid_ list of the term that replaces it, marked as its former name, so a reader of an old record can map it: Ticket run (Run, Merge run), Architect run (Weeding), Security run (Fencing), and likewise for any other renamed term.
- (b) Update them to the new names too.

*Recommended:* (a). A dated record should read as it was decided, and the glossary keeps the mapping in one place.

*Answer:* agree

**Q2.5 - When the glossary changes.** The usual practice (the domain-modeling skill) is to update GLOSSARY.md as soon as a term is settled. Here, every factory session reads the glossary, and Standards reviews check changes against it. A glossary that says to avoid "Run" while the Session prompts and manual still use it would get "Run" flagged in unrelated pull requests until the rename lands. The options:
- (a) Change the glossary in the same pull request as the Session prompts, Factory skills and manual, so agents never read two names. Until then, this file records the decisions.
- (b) Change the glossary now, on the `website-update` branch, ahead of the rename.

*Recommended:* (a).

*Answer:* agree

## Round 3 (answered)

**Q3.1 - How the work is split.** Q2.2 and Q2.5 sort the rename by who reads the words. Proposed: one Spec with three Tickets, run in order:
- **Ticket A, the words agents read:** GLOSSARY.md, the Session prompts, the Factory skills, the manual, the README and the repository's agent docs (docs/agents, code navigation and testing). That's about 570 uses, 390 of them in the manual. It's one pull request, as Q2.5 requires, and it also writes the ADR if Q3.3 says so.
- **Ticket B, what thirdshift prints:** CLI help and output, Run notifications, the comments Setup writes into the User config, and the Console's labels. Displays show the stored "Run" as "Ticket run", and output says "Failed ticket run". The manual's quoted output lines follow. That's about 170 string literals in the source, with their tests. Blocked by A.
- **Ticket C, the site:** Q2.1's homepage phrases, Q1.4's sentence on the homepage and in the README's first lines, the new names on How it works and Architecture (five workflows, and Figure 3), and a re-rendered share card. That's about 95 uses. Blocked by A, by B (so the homepage quotes B's output), and by #1095.

When the Spec is published, #805, #807, #941 and #713 close, each with a comment linking the Spec and this file. The options:
- (a) As proposed.
- (b) One Ticket per term, as #941's brief suggested.
- (c) Something else: say how.

*Recommended:* (a). Tickets per term would each edit the same glossary, prompts and manual, so they would run one after another anyway, and each reader's words would change in pieces. Splitting by reader changes each audience's words in one pull request.

*Answer:* agree

**Q3.2 - When the Spec runs, and whether it Self-merges.** Pickup takes the lowest-numbered Ready issue, so a new Spec would wait behind the queue: #976, the three eval Specs (#977, #978 and #981) and about a dozen standalone Tickets, which add more uses of "Run" as they land. On this host `merge.always` is on, so a Spec PR Self-merges once green. Ticket A rewrites the instructions every later session reads, and Ticket C rewrites the public homepage. The options:
- (a) Publish it ready-for-agent. Once #1095 merges, start it yourself with `thirdshift no-merge <Spec URL>`, so the Spec PR waits for your review. Its Tickets block each other, so it builds one Ticket run at a time beside Pickup's work, and Conflict Repairs handle overlaps in the manual and glossary.
- (b) As (a), but let the Spec PR Self-merge.
- (c) Let Pickup reach it in queue order. The Spec PR then Self-merges, as the User config says.
- (d) Hold it until the eval Specs finish.

*Recommended:* (a). Waiting only adds more "Run" to rename and keeps the old homepage up longer. One review of the whole Spec PR is a small cost for a change that rewrites every session's instructions and the public site at once.

*Answer:* agree, except okay to self-merge

**Q3.3 - An ADR for the stored "Run".** Q2.2 keeps "Run" in stored records while every display says "Ticket run". A later reader, or an Architect run, could take that for an oversight and migrate the records. The options:
- (a) Ticket A writes an ADR recording the rename (Q1.7) and why stored values keep "Run" (Q2.2), in the same pull request as the glossary.
- (b) No ADR. The glossary's former-name note is enough.

*Recommended:* (a). It would surprise a reader without the context, and it came from a real trade-off between migrating records and translating them for display. That is what an ADR is for.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- From the #103 grilling (1 October): the factory vocabulary stays. You chose not to rename it then, and this grilling doesn't reopen "factory".
- From the #698 grilling (21 October): a self-descriptive or industry-standard word beats a metaphor, unless the metaphor lowers cognitive load. Where there's no standard, thirdshift keeps its own word. Every term here is judged by that test.
- The rename work that follows is agent work across the glossary, manual, README, site, prompts and skills: one Ticket per term, or one Spec (#941's brief). Round 3 decides how it's packaged and when it runs.
- From Q1.1: thirdshift keeps its name, the colophon's story and the homepage's look (type, colours and the press drawing). Every heading, label and sentence says plainly what it means, starting with the phrases in Q2.1.
- From Q1.2: Weeding and Fencing are retired, and their glossary entries go. The manual's two sections become "Architect runs every few minutes" and "Security runs on a schedule, with fixing allowed", and the comments in its crontab example follow. Links to the old anchors move to the new ones. The README's old-bookmark anchors keep their names and point at the new sections.
- From Q1.3: Factory, Factory skills, Sweep, Pass, Claim and Harness stay. With Weeding, Fencing and Day shift decided, #941 is fully answered.
- From Q1.4: the homepage's opening paragraph and the README's first lines say that thirdshift drives the coding agents you already use, logged in as you, so runs use each one's plan or account, and that thirdshift itself is free and open source. Plan billing is named only for Claude Code until the manual documents it for another Harness. Q3.1 covers when #713 closes.
- From Q1.5: #1095 is running, and the homepage rewrite builds on its result, so the rewrite starts after #1095 merges.
- From Q1.6: Day shift stays everywhere, the Session prompts and Factory skills included.
- From Q1.7: Run becomes Ticket run, and Merge run folds into it: a Ticket run leaves its pull request for review or Self-merges it. Spec run, Self-merge and the `merge` and `no-merge` options keep their names. There are now five workflows: Ticket run, Spec run, Architect run, Pickup run and Security run. How it works and Architecture take the new names, and Architecture's Figure 3 becomes a Ticket run that Self-merges. "Run" as a verb, as in "Run the factory on the issue", and lowercase "run" for runs in general stay as they are.
- From Q2.1: the homepage's print phrases take the plain replacements listed there, and the share card is re-rendered from the new headline and press drawing.
- From Q2.2: every word people and agents read says "Ticket run": glossary, manual, README, site, Session prompts, Factory skills, CLI help and output, Run notifications and the Console's labels. Command records, the Activity log and `status --json` keep storing "Run", and displays show it as "Ticket run". Log folders and code identifiers keep their names.
- From Q2.3: "Failed run" becomes "Failed ticket run". "Run notification", "Run metrics", "Continuation" and "Recovery" keep their names, and their definitions say "Ticket run" where they meant a Run.
- From Q2.4: ADRs, research notes, release notes and finished grillings keep the words they were written with. The glossary's _Avoid_ lists give each retired word as the former name of its replacement: Ticket run (Run, Merge run), Failed ticket run (Failed run), Architect run (Weeding) and Security run (Fencing).
- From Q2.5: GLOSSARY.md changes in the same pull request as the Session prompts, Factory skills and manual. Until then, this file is the record of the decisions.
- From Q3.1: one Spec with three Tickets, each blocking the next. A changes the words agents read and writes the ADR. B changes what thirdshift prints. C changes the site, and is also blocked by #1095. When the Spec is published, #805, #807, #941 and #713 close, each with a comment linking the Spec and this file.
- From Q3.2, (b): the Spec is published ready-for-agent. Once #1095 merges, you start it with `thirdshift <Spec URL>`, and its Spec PR Self-merges once green, as `merge.always` says.
- From Q3.3: Ticket A writes ADR 0022, recording the rename and why stored values keep "Run".
- Ticket B covers everything thirdshift writes for people, not only the terminal: its comments and issue bodies on GitHub, and its emails.
- Ticket A leaves the hand-written site pages to Ticket C, even though the new `AGENTS.md` rule would otherwise have it correct them in the same pull request.
- /videos/ keeps its descriptions, since they describe the videos as recorded, and open issues keep their wording. As Q2.4 does for dated records, agents map "Run" in them through the glossary's former-name note.
- GitHub's repository description and topics use neither "Run" nor a metaphor, so they stay.

## Research results (22 October 2026)

- Weeding and Fencing appear in GLOSSARY.md, the manual (13 times), the README (3), the three site pages (5), research notes and one code comment. No command, setting or output uses them. Confirmed by `rg`.
- "Day shift" appears 149 times in 40 files, including Session prompts and Factory skills. "Merge run" appears 150 times in 46 files. "Run" or "Runs" appears about 2,300 times in 225 files, counting code and sentence starts. Confirmed by `rg`, excluding the generated pages.
- The glossary already defines a Ticket as "an issue scoping one session of work, standing alone or as a sub-issue of a Spec", so every Run is a Run on a Ticket. Confirmed in GLOSSARY.md.
- The manual documents plan billing only for Claude Code: "When `claude` is logged in with a claude.ai subscription, Runs use the plan's usage allowance, not API billing." Confirmed in docs/guide.md, under Usage and cost.
- One issue-to-pull-request attempt is a "session" at GitHub (Copilot, now "Copilot cloud agent"), in Jules's API and at Devin; a "task" at GitHub, in Codex Cloud and in Jules's app; a "cloud chat" at Codex; a "Cloud Agent" at Cursor; and a "Delegation", with a "Ticket to PR" starter, at Factory. AI Hero defines a Ticket as "a handoff artifact scoping one session of work" and a Session as "one bounded run of interaction with an agent". Confirmed in each vendor's docs, read 22 October 2026: [GitHub](https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/overview), [Codex](https://learn.chatgpt.com/docs/third-party/linear), [Jules](https://jules.google/docs/api/reference/sessions/), [Devin](https://docs.devin.ai/integrations/linear), [Cursor](https://cursor.com/docs/integrations/linear), [Factory](https://docs.factory.com/software-factory/automations), [AI Hero](https://www.aihero.dev/ai-coding-dictionary).
- No tool runs a parent issue's sub-issues into one pull request. GitHub's docs say sub-issues are "tackled in separate pull requests"; Devin's nearest is a stack of pull requests, and Factory's Missions aren't issue-based. AI Hero's Spec is the document only, "made of tickets". Confirmed in [GitHub](https://docs.github.com/en/copilot/tutorials/cloud-agent/improve-a-project), [Devin](https://docs.devin.ai/work-with-devin/stacked-prs) and [Factory](https://docs.factory.com/missions/overview)'s docs.
- No tool names the humans' part of the work. AI Hero's "human-in-the-loop" is humans pairing with the agent during a session, and its "AFK" leaves the agent to run unattended. Confirmed in [AI Hero](https://www.aihero.dev/ai-coding-dictionary).
- Recurring agent work is an "automation" at GitHub, Devin (formerly "Scheduled Sessions"), Cursor and Factory, and a "scheduled task" at Codex, Jules and Claude Code. Scheduled security fixing goes by product names: Dependabot security updates, Snyk's Automatic Fix PRs and Backlog PRs, Codex's Continuous Scanning, and templates at Devin and Jules. Confirmed in [GitHub](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-automations), [Codex](https://learn.chatgpt.com/docs/automations), [Jules](https://jules.google/docs/scheduled-tasks/), [Devin](https://docs.devin.ai/product-guides/scheduled-sessions), [Cursor](https://cursor.com/docs/cloud-agent/automations), [Claude Code](https://code.claude.com/docs/en/routines), [Dependabot](https://docs.github.com/en/code-security/concepts/supply-chain-security/dependabot-security-updates) and [Snyk](https://docs.snyk.io/scan-fix-and-prevent/fix/snyk-pull-or-merge-requests/enable-automatic-backlog-prs-for-previously-known-vulnerabilities)'s docs.
- Merging on green CI is "auto-merge" at GitHub and Devin, and "automerge" at Renovate. GitHub's agent "cannot approve or merge a pull request". thirdshift avoids "auto-merge" because it doesn't use GitHub's feature (ADR-0004), so Self-merge stays. Confirmed in [GitHub](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/automatically-merging-a-pull-request) and [Renovate](https://docs.renovatebot.com/key-concepts/automerge/)'s docs.
- Command records and `status --json` give the kind as one of "Run", "Spec run", "Pickup run", "Architect run" and "Security run", and the Activity log writes the same words. History reading matches those stored words, so existing records say "Run". Log folders are `issue`, `pickup`, `architect` and `secure`. Confirmed with `thirdshift status --json` on the `thirdshift` host and in the source (`src/logs/activity.rs`, `src/logs/reading/history.rs`).
- The glossary pairs "Failed run" (a Run that ends without a ready, mergeable, green pull request) with "Failed spec run", and defines "Run notification" across Runs, Spec runs, Architect runs and Security runs. Confirmed in GLOSSARY.md.
- Capitalised "Run", "Runs", "Merge run" and "Failed run" appear about 570 times in the words agents read: 390 in the manual, 133 in the glossary, 13 each in the README and the Factory skills, and the rest in the Session prompts and agent docs. The three hand-written site pages have about 95, and about 170 string literals in the source contain "Run". Confirmed by `rg`.
- On the `thirdshift` host, the User config has `merge.always = true`, `spec.parallel = 3` and `pickup.limit = 10`. Confirmed in `~/.thirdshift/config.toml`.
