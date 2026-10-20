# The thirdshift marketing / launch video: lead claim and outline

Research date: 2026-10-19.

The general findings — platform specs, length and the hook, sound-off viewing and captions, what resonates with developers, and the render recommendation — are in the explainer-video skill's [`research/marketing.md`](https://github.com/JacobStephens2/skills/blob/main/skills/explainer-video/research/marketing.md). This note keeps what is thirdshift's own: the claim to lead with, and the 25-second cut's beats with their sources in this repository. Honest-claims constraints are in [explainer-video-audiences.md](explainer-video-audiences.md).

## The claim to lead with

The overnight Run — the only repo proof with two real numbers and public artefacts: "A scheduled Pickup run took issue #663 at 00:21, and its Merge run merged pull request #665 at 01:22" (`site/index.html:105`, stderr at `:207`, `:301`). One caution: that Run's own log says "sessions run on codex · gpt-6.1-sol · high" (`site/index.html:207`), so **the proof beat must not be captioned as Claude Code**.

## Marketing cut: thirdshift rules and outline

These add to the general rules in the skill's note.

1. Every terminal frame is real, unretouched stderr from the Run in `site/index.html`: no mocked timestamps, no invented PR numbers.
2. The proof beat names the date and both numbers (#663 → #665, 12 Oct 2026) or does not run — and never attributes that Run to Claude Code.
3. The on-screen limit: sessions run unattended as your user, and a checkout is not a sandbox (ADR 0013, `site/how-it-works/index.html:91`).
4. "Worktree", "Harness", "Repair", "Claim", "Self-merge" and "Ticket" do not appear; "MIT, open source, a personal project" does.

| # | ~s | On-screen text | Narration | Repo source |
|---|---|---|---|---|
| 1 | 0–4 | `$ thirdshift https://github.com/acme/widgets/issues/7` typing into a real terminal; below it **"Give it an issue."** | none | `README.md:33` |
| 2 | 4–8 | **"Get a pull request ready for review."** over `PR …/pull/665 is ready for review` | none | `README.md:3`, `docs/guide.md:315` |
| 3 | 8–13 | Four units label up — **"Prepare · Implement · Review · Finish"** — with **"reviewed on two axes, then Git and CI"** anchored beside unit 3 | none | `docs/guide.md:38`, `:39`, ADR 0004 |
| 4 | 13–17 | **"Ready only when the factory stands behind it."** / smaller: *"otherwise the work is pushed and the PR goes back to draft"* | none | `README.md:5`, `docs/guide.md:7`, `:730–731` |
| 5 | 17–21 | Real stderr, timestamps visible: `00:21:05 taking Ready issue #663` → `01:22:56 PR …/pull/665 is merged`, with **"Overnight, 12 Oct 2026"** | none | `site/index.html:105`, `:207`, `:301` |
| 6 | 21–24 | **"Sessions run unattended as your user. A checkout is not a sandbox."** | none | ADR 0013, `site/how-it-works/index.html:91` |
| 7 | 24–27 | **"thirdshift.app — MIT, open source, a personal project. Point it at one issue in a repo you own."** | none | `README.md` install + license |
