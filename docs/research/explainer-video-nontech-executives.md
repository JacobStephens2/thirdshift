# What the thirdshift explainer can say to non-technical executives

Research date: 2026-10-20.

The general findings (what non-technical leaders want to know, how analogies decide a viewer's errors, the vocabulary budget, and the claims a business register invites) are in the explainer-video skill's [`research/nontech-executives.md`](https://github.com/JacobStephens2/skills/blob/main/skills/explainer-video/research/nontech-executives.md). This note keeps what is thirdshift's own: its analogy mappings, its term translations, what it can claim, and the non-technical cut's beats with their sources in this repository. It builds on [explainer-video-audiences.md](explainer-video-audiences.md), the technical-executive cut. The video built from it is "thirdshift, in plain terms" (#887).

**Method.** Product claims come only from this repository, read-only: `README.md`, `docs/guide.md`, `GLOSSARY.md`, `docs/adr/`, `site/index.html`, `site/how-it-works/index.html`.

## Safe analogies

Each row maps a relation, not the worker's character, and accepts one blind spot.

| thirdshift step | Safe analogy | Why it holds | Predictable blind spot to accept |
|---|---|---|---|
| Issue → a Run starts | **A work order handed to a contracted shop** | A written request, scoped before work starts; the shop checks the request makes sense first (`docs/guide.md:29` step 1) | Implies a quote and a deadline. Neither exists; say nothing about time or price. |
| Work happens in a Worktree | **A separate bench, not your desk** | "so your own checkout is never touched" (`docs/guide.md:37`) | Suggests a locked room. It is not: "A checkout is not a sandbox" (`site/how-it-works/index.html:91`). Correct it in the same breath. |
| Agent self-review on two axes | **A second pair of eyes before the work leaves the bench** | Two independent reviewer roles inside one session (`site/how-it-works/index.html:109`) | Suggests a guarantee. Correct it: "They do not guarantee correctness or replace your judgment" (`:110`). |
| Pull request left ready | **A draft submitted for sign-off** | `README.md:33`; marked ready only when "the factory stands behind it" (`docs/guide.md:731`) | Suggests nothing exists before the signature. The work does exist on the branch; that is fine to leave implicit. |
| Merge (opt-in) | **You sign it off, or you ask the shop to file it once every check passes** | ADR 0004: merging happens only once thirdshift "has seen it pass"; never GitHub auto-merge | Suggests the shop decides. It never does: ADR 0004 rejects the agent merging "its own work." |
| Failed run | **The work comes back with a note, not in the bin** | "the work is pushed so nothing is lost, and the pull request goes back to a draft" (`docs/guide.md:7`, `:730-731`) | None material. **[inference]** |

**Why the general barred analogies hold here.** "Runs itself": thirdshift "never schedules itself: the operating system's scheduler starts each one" (`GLOSSARY.md:92`; ADR 0009), and an unattended Run "that hits the plan's usage limit stalls or fails partway through" (`docs/guide.md:264`). "A locked sandbox": ADR 0013 runs sessions with approvals off, "its sandbox off, and the worktree trusted"; "The worktree is the only boundary." "Checks its own homework": the self-approval reading ADR 0004 exists to deny.

## Terms

Pre-teach only these three, in this order: (1) *a written request* (the issue); (2) *a draft for sign-off* (the pull request); (3) *the automatic checks* (CI). Everything else is translated inline, and the thirdshift term is never spoken.

| thirdshift / GitHub term | Lay phrasing to speak | Repo source |
|---|---|---|
| GitHub issue, Issue URL | "a written request for a change" | `GLOSSARY.md:11` |
| Pull request | "a proposed change, written up for sign-off" | `README.md:3`, `:33` |
| Ready for review / `gh pr ready` | "marked ready for a person to sign off" | `docs/guide.md:29` step 5 |
| Draft (a Failed run's PR) | "sent back as unfinished, with the work kept" | `docs/guide.md:7`, `:730` |
| Merge / Self-merge / Merge run | "filed into the main copy, only if you ask for that" | `GLOSSARY.md:157`, ADR 0004 |
| CI / checks / green | "the automatic checks the company already runs" | `docs/guide.md:29` step 6 |
| Code review, Standards and Spec axes | "a second pair of eyes: does it follow the house rules, and does it do what was asked" | `site/how-it-works/index.html:109` |
| Worktree, Launch directory | "its own separate copy of the files, not yours" | `GLOSSARY.md:68`, `docs/guide.md:37` |
| Harness, agent session, headless | "the AI coding tool it drives" | `GLOSSARY.md:33` |
| Repair | "a second attempt when a check fails" | `GLOSSARY.md:209` |
| Claim / `in-progress` label | "the request is marked as taken" | `GLOSSARY.md:71` |
| Day shift / third shift | "your people work days; this runs overnight" | `README.md:3` |
| cron / Pickup run / scheduler | "your computer's own timer starts it" | `GLOSSARY.md:92`, ADR 0009 |
| Spec, Ticket, Base branch, Architect run, Security run, Inherited failure, Base fix | **not mentioned at all** | — |

## Claims

- **"Overnight": allowed.** It describes when the documented Run ran: "thirdshift printed on stderr during a real Run, overnight on 12 October 2026" (`site/index.html:105`). `README.md:3` already frames it that way.
- **"While you sleep": literal timing only.** ADR 0004 and `docs/guide.md:29` step 5 are the substantiation that work does not ship unseen.
- **"It runs itself," "fully autonomous," "automates engineering": barred.** See `GLOSSARY.md:92` and `docs/guide.md:264` above; it automates one path from a written request to a draft for sign-off, and the decision stays human (`README.md:33`, ADR 0004).
- **"Enterprise-ready," "governed," "auditable": barred.** No SSO, org policy or audit-trail controls appear in the docs, and there is no real-time monitoring. What it can substantiate: a person decides (ADR 0004), and every prompt is published (`prompts/`, `skills/`).
- **The access limit, in lay terms:** sessions run unattended as your user, with that account's files, credentials and tools (`README.md:17`), and "A checkout is not a sandbox" (`site/how-it-works/index.html:91`).
- **License:** MIT-licensed, open source (`README.md:60`). Don't call it a "personal project" on screen.

## Non-technical executive cut: thirdshift rules and outline

These add to the general rules in the skill's note.

1. The first spoken sentence says a person on your team still decides what ships, then that thirdshift does the work overnight.
2. The three new terms are *written request*, *draft for sign-off* and *automatic checks*, each with one look throughout.
3. Not one of these words is spoken or printed: issue, pull request, PR, repo, repository, branch, merge, commit, CI, pipeline, cron, worktree, harness, sandbox, agent, session, code review, spec, ticket, stderr, `gh`.
4. The risk beat states the access fact (`README.md:17`, `site/how-it-works/index.html:91`).
5. The proof beat names the real Run with its date and times (request #663 taken 00:21, change #665 added 01:22, 12 October 2026, `site/index.html:105`), or no proof beat runs.
6. The words "enterprise," "governed," "compliant," "auditable," "secure" and "autonomous" do not appear.

| # | ~s | Beat | Repo source (file:line) |
|---|---|---|---|
| 1 | 0–9 | "A person still decides whether any change happens. This just does the work overnight." | `README.md:3`, `README.md:33` |
| 2 | 9–19 | Pre-teach: **a written request** · **a draft for sign-off** · **the automatic checks** | `GLOSSARY.md:11`, `README.md:3`, `docs/guide.md:29` |
| 3 | 19–28 | The problem: AI writes plenty, and few companies have a rule for what to accept | Deloitte 21% (skill note); `site/how-it-works/index.html:119` |
| 4 | 28–42 | A separate copy of the files, the AI does the work, a second pair of eyes checks it against the house rules and what was asked, the automatic checks run, and it is marked ready only when all passed | `docs/guide.md:37`, `:38`, `:39`, `:731`, `site/how-it-works/index.html:109` |
| 5 | 42–52 | Proof: a scheduled run took request #663 at 00:21 on 12 October 2026, and the change was added at 01:22 | `site/index.html:105` |
| 6 | 52–62 | When it can't finish: the work is kept, and the draft goes back unfinished, so "ready" always means something | `docs/guide.md:7`, `:730-731` |
| 7 | 62–74 | The limit, spoken: it runs as the person who set it up, with whatever that account can reach; then what helps get the work right | `README.md:17`, ADR 0013, `site/how-it-works/index.html:91`, `:110` |
| 8 | 74–86 | "Have someone on your team point it at one request in a project you own." Open source, at thirdshift.app | `README.md:60` |
