# Newcomer concepts for a "How thirdshift works" explainer

Research date: 2026-10-19.

**Question.** Which thirdshift concepts do newcomers misunderstand, or need defined before anything else, according to the repo's own record? The output informs a 2-minute narrated explainer video: which terms it must pre-teach, and which misconceptions it must correct.

**Method.** Only this repository's own record counts as evidence: `CONTEXT.md` (the glossary, whose `_Avoid_` lines are the project's own list of wrong readings), the ADRs, the shipped explainer at `site/how-it-works/index.html`, GitHub issues and PRs via `gh` (searched for README, confus\*, clarify, unclear, terminology, rename, glossary, how-it-works, diagram, newcomer, onboarding, explain, misleading, "reads as"), `git log --follow` on `site/how-it-works/`, `README.md`, `docs/guide.md` and `CONTEXT.md`, and the architecture-diagram prototype in the sibling worktree `../diagram`. Code was checked where a claim about who does what was load-bearing.

**Out of scope.** Video script, pacing, visuals and voice. Whether the design is good — only how to explain it. Site aesthetics (see `site-design-inspiration.md`) and diagram craft (see the prototype's `clear-architecture-diagrams.md`).

## TL;DR

- The repo already decided terms must be pre-taught before any causal flow, and recorded why (`../diagram/docs/research/clear-architecture-diagrams.md:310`, :341).
- Variant D acted on it with an eight-term "Key terms" section: Day shift, Issue, Run, Agent session, Worktree, Claim, Repair, Failed run. Its lede states the central misconception in one line: "The agent never marks its own work ready." That line is inaccurate: the opening session is told to open the pull request "marked ready for review" (`prompts/fresh.md:12`). What the agent never decides is whether the pull request *stays* ready, or whether it merges (see row 1).
- All six candidate misconceptions are **confirmed**; none is rejected. Three are already pre-empted in the shipped explainer's own prose — itself evidence the project believes readers get them wrong.
- The most-recorded wrong reading is **sandbox**: `site/how-it-works/index.html:91` carries a dedicated callout, "A checkout is not a sandbox."
- Five term inconsistencies are live, one an open issue ([#812](https://github.com/JacobStephens2/thirdshift/issues/812), CONTEXT.md vs GLOSSARY.md).

## 1. Why pre-teaching is the recorded requirement

The prototype's design note is the only place in the repo that studies newcomer comprehension directly. Its rule 11, "Name the parts before explaining the flow," prescribes "a short glossary of the nouns (Run, Claim, Worktree, Harness, Base branch, Issue branch, Repair, Self-merge) *before* the first causal diagram", and concludes: "thirdshift's CONTEXT.md is already this glossary; the page needs an 8–12-term subset of it, up front" (`clear-architecture-diagrams.md:310`). Its cold-reader critique of the earlier variants is the clearest recorded statement of the problem: variant A "fires 'Origin match', 'Claim', 'Worktree', 'Session prompt', 'Factory skills', 'Harness', 'Repair', 'Resume', 'Self-merge', 'Inherited' and 'Base branch' at a cold reader inside one figure" (line 341). It also warns that scaffolding built for a newcomer harms an expert (line 120) — the reason the shipped page folds sections closed ([#804](https://github.com/JacobStephens2/thirdshift/issues/804), commit `2cbb2ad`).

Variant D's eight terms (`../diagram/site/prototype-architecture/index.html:950-958`) each carry an owner tag — `Person · thirdshift · AI agent · GitHub` (line 960) — so the list pre-teaches not only the words but **who owns each one**, the mechanism behind misconception 1.

*Inference, flagged:* the note argues from cognitive-science sources, not from observed thirdshift readers. No user-feedback thread exists in the issues. The nearest recorded reader perspective is [#61](https://github.com/JacobStephens2/thirdshift/issues/61)'s user stories, written as "so that I'm not misled into thinking…".

## 2. Misconceptions, with evidence

| # | Misconception | Verdict | Evidence |
|---|---|---|---|
| 1 | The agent decides whether its work ships: keeps the PR ready, or merges it | **Confirmed, narrowed** | The agent does open the pull request marked ready (`prompts/fresh.md:12`); what it never decides is the outcome. ADR 0004 rejects "The agent merges from inside a session" because "whether the work ships would be the agent's decision about its own work, not a deterministic check". `docs/guide.md:29` step 5: thirdshift "Takes over deterministically… and marks it ready for review (`gh pr ready`) if the agent left it as a draft." In code: `src/delivery.rs:448` → `src/github.rs:729-730`. |
| 2 | thirdshift sandboxes the agent | **Confirmed** | `site/how-it-works/index.html:91`: "**A checkout is not a sandbox.** Sessions run unattended as your user… the separate Worktree keeps edits apart, but does not confine the process." ADR 0013, "Every Harness runs unattended and fully trusted": approvals off, "its sandbox off, and the worktree trusted… no user setting changes them." ADR 0012: "The worktree is the only boundary." Also `docs/guide.md:251`. |
| 3 | It works in, or touches, your checkout | **Confirmed** | `CONTEXT.md:16`: "A Run works in its own worktree, never in the Launch directory." `docs/guide.md:37`: "so your own checkout is never touched." `README.md:33`. The shipped explainer draws it as "Example: two checkouts, one Git repository". |
| 4 | A failed Run loses work | **Confirmed** | `CONTEXT.md:195`: "any unfinished merge is aborted and its work is still pushed so nothing is lost." `docs/guide.md:730`. [#61](https://github.com/JacobStephens2/thirdshift/issues/61) story 16 asks for a Failed run shown "so that I know a failure doesn't lose work". |
| 5 | It merges by default | **Confirmed, with a caveat** | `CONTEXT.md:154-156`: a **Merge run** is "A Run *asked* to end with its pull request merged", `_Avoid_: auto-merge (GitHub's own feature, which thirdshift does not use)`. `README.md:33`. **Caveat:** the User config can make every Run on a machine a Merge run (`CONTEXT.md:163`) — why the shipped explainer passes `--no-merge` explicitly, "even if your User config normally requests merging". |
| 6 | A closed Ticket means the Spec shipped | **Confirmed** | ADR 0006: "A Ticket closes when it lands on the Spec branch, before its work reaches the Base branch. Rejecting the Spec PR means reopening its Tickets by hand." The shipped explainer states the correction verbatim. |

Two further misconceptions are recorded and worth the video's time:

- **Review is a separate session.** [#61](https://github.com/JacobStephens2/thirdshift/issues/61) user story 14: "I want a single 'one agent session' bracket over M and Y, so that I'm not misled into thinking review is a separate session." Confirmed by `docs/guide.md:29` step 4 (implement *and* review in the opening session) and `CONTEXT.md:203` (a **Repair** is the separate follow-up session).
- **An open pull request means it's done.** `site/how-it-works/index.html:~118`: "A pull request existing is not the finish line." The same section adds that the agent reviews "do not guarantee correctness or replace your judgment."

## 3. Terms the docs use inconsistently

1. **`CONTEXT.md` vs "glossary" vs "GLOSSARY.md".** `docs/agents/domain.md:41-45` and `docs/code-navigation.md:3` call it "the glossary"; [#64](https://github.com/JacobStephens2/thirdshift/issues/64) says "Use Glossary terms exactly"; open issue [#812](https://github.com/JacobStephens2/thirdshift/issues/812) proposes renaming the file. The video should say "glossary" and name no file.
2. **"Run" is overloaded** — the glossary noun (`CONTEXT.md:65`), the verb, and the thing a Spec run starts per Ticket. Open issue [#805](https://github.com/JacobStephens2/thirdshift/issues/805), "consider renaming Run and Merge run to Ticket run", acknowledges this; it is unresolved, so use **Run** as defined and don't pre-empt the rename.
3. **"ticket" vs "Ticket".** [#64](https://github.com/JacobStephens2/thirdshift/issues/64): "Don't use the word 'ticket' on the site." `CONTEXT.md:13` lists "ticket" as a wrong reading of **Issue URL**; `:63` lists "task, sub-task" for **Ticket**. Say "sub-issue", never "ticket" loosely.
4. **"ready" means three things**: the `ready-for-agent` label (and the **Ready issue** rule, `CONTEXT.md:111`), a pull request "ready for review", and a Run outcome. `docs/agents/triage-labels.md:19` records a real consequence: "a `ready-for-agent` Spec with no Tickets yet looks exactly like a standalone Ticket, and a Pickup run would start a plain Run on it."
5. **"Spec" means two things**: the Spec issue (`CONTEXT.md:57`) and the **Spec axis** of code review (`:47`). Likewise **Security review** vs **Security audit**, which the glossary has each `_Avoid_` the other (`:129`, `:149`), and **Harness** vs "agent" (`:35` avoids "agent", while variant D writes "Agent session… Its CLI is the Harness").

## What the explainer must do

**Pre-teach these terms** (checked against `CONTEXT.md`; the first seven are the intersection of variant D's list and the design note's rule-11 list):

- [ ] **Day shift** — you and the other humans: you write the issues and review the pull requests. (`CONTEXT.md:7`)
- [ ] **Run** — one invocation of thirdshift on one issue, from launch to cleanup, ending in one pull request. (`CONTEXT.md:65`)
- [ ] **Base branch** — the branch the work branches off and the pull request targets; normally the one you have checked out. (`CONTEXT.md:22`)
- [ ] **Worktree** — a separate, factory-owned checkout made for one Run, so your own checkout is never touched. (`CONTEXT.md:68`, `docs/guide.md:37`)
- [ ] **Claim** — the `in-progress` label thirdshift puts on an issue it has taken; not a lock. (`CONTEXT.md:71-73`)
- [ ] **Harness** — the headless agent CLI that runs each session: Claude Code by default, or Codex, agy, Grok Build, Muse Code, OpenCode. (`CONTEXT.md:33`)
- [ ] **Repair** — an extra agent session started after the pull request exists, to fix a merge conflict or failing CI. (`CONTEXT.md:203`)
- [ ] **Merge run** — a Run *asked* to end with its pull request merged, rather than left for review. (`CONTEXT.md:154`)
- [ ] Only if the video covers Spec runs, add **Spec** and **Ticket** as one pair: an issue made of GitHub sub-issues, worked in dependency order. (`CONTEXT.md:57`, `:61`)

**Correct these misconceptions:**

- [ ] The agent opens the pull request, but never decides whether it ships: thirdshift checks it, turns it back into a draft if Delivery fails, and does the Self-merge itself. The agent never merges. (ADR 0004; `docs/guide.md:29`; `prompts/fresh.md:12`; `src/github.rs:729`)
- [ ] The Worktree is isolation of *files*, not a sandbox — sessions run as your user with your credentials. ("A checkout is not a sandbox", `site/how-it-works/index.html:91`; ADR 0013)
- [ ] Your checkout is never touched; the Run works in a sibling Worktree. (`CONTEXT.md:16`; `README.md:33`)
- [ ] A failed Run does not lose work — it commits, pushes and drafts the pull request. (`CONTEXT.md:195`; [#61](https://github.com/JacobStephens2/thirdshift/issues/61) story 16)
- [ ] It does not merge by default; merging is asked for per command, or set per machine in the User config. (`CONTEXT.md:154`, `:163`)
- [ ] A closed Ticket means its work reached the **Spec branch**, not `main`. (ADR 0006)
- [ ] Implement and review are *one* session; a Repair is the separate one. ([#61](https://github.com/JacobStephens2/thirdshift/issues/61) story 14; `docs/guide.md:29` step 4)
- [ ] An open pull request is not the finish line, and an agent review is not a correctness guarantee. (`site/how-it-works/index.html`, Delivery section)

**Note on sources.** The prototype page and its design note live in the sibling worktree `../diagram` and are **uncommitted**, so their line numbers are as of 2026-10-19 and may move. Everything else cited is committed on `main`.
