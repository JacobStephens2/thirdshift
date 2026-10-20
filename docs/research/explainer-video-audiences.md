# What the thirdshift explainer can claim to executives and to engineers

Research date: 2026-10-19.

The general findings — METR and DORA, length evidence, answer first, the FTC substantiation rule, and how an executive cut differs from an engineer cut — are in the explainer-video skill's [`research/audiences.md`](https://github.com/JacobStephens2/skills/blob/main/skills/explainer-video/research/audiences.md). This note keeps what is thirdshift's own: what it can and cannot claim, and the executive cut's beats with their sources in this repository.

**Method.** Product claims are grounded only in this repository, read-only: `README.md`, `docs/guide.md`, `CONTEXT.md`, `docs/adr/`, `site/index.html`, `site/how-it-works/index.html`.

## Proof

thirdshift's proof is an artifact, not a benchmark, and the site already uses it: "A scheduled Pickup run took issue #663 at 00:21, and its Merge run merged pull request #665 at 01:22," captioned as "lines thirdshift printed on stderr during a real Run, overnight on 12 October 2026" (`site/index.html`).

## Claims

**Safe, each substantiated.** (1) *Humans stay in control by default* — the PR is left "ready for your review" (`README.md:33`), merging is opt-in, and ADR 0004 rejects an agent merging its own work as "not a deterministic check." (2) *Ready means the factory stands behind it* — a Failed run "Converts its open pull request … back to a draft, so a pull request only claims to be ready when the factory stands behind it" (`docs/guide.md:731`). (3) *Failure preserves work* — it "Commits any uncommitted work … and pushes the Issue branch, so nothing is lost," keeping the worktree if the push fails (`:730`). (4) *Your checkout is never touched* (`:37`, `CONTEXT.md:16`). (5) *Only reviewed code merges* — ADR 0004: "Only code an agent wrote or reviewed gets merged, and it merges only once thirdshift has seen it pass." (6) *Every prompt and skill is readable* (`prompts/`, `skills/`).

**Cost visibility is narrower than it sounds.** "The `total_cost_usd` in the session logs is the API-price equivalent, not a charge," and on a claude.ai subscription "Runs use the plan's usage allowance, not API billing" (`docs/guide.md:264`). Codex reports token totals but "no turns or cost" (`:505`), and `status`/`show --json` carry **no cost field**. Say *"per-session token and cost-equivalent figures are in the logs"* — never "you can see your spend."

**Must not claim.** (a) Any productivity multiplier, time saved or throughput figure — none is measured for thirdshift, and the 40-point perception gap makes an author's impression worthless. (b) Guaranteed correctness: the site already says the reviews "do not guarantee correctness or replace your judgment." (c) "Replaces engineers." (d) **Security sandboxing** — ADR 0013: sessions run "with approvals off, its sandbox off, and the worktree trusted … The worktree is the only boundary"; the site says "A checkout is not a sandbox." (e) Comparisons to other tools. (f) Enterprise governance — no SSO, audit trail or org policy controls appear in the docs, and it is a personal MIT project with no support commitment; **saying so is a credibility feature, not a gap to hide.** (g) Reliability — an unattended Run "that hits the plan's usage limit stalls or fails partway through."

## How thirdshift's two cuts differ

| | Executive cut | Engineer cut |
|---|---|---|
| **Opening** | The answer in one sentence | Pre-teach the nouns ([explainer-newcomer-concepts.md](explainer-newcomer-concepts.md)), then the causal flow |
| **Unit** | One decision: adopt or not | One Run, start to finish |
| **Proof** | The real 12 Oct 2026 Run, timestamped | The same, plus prompts, logs, ADRs |
| **Avoid** | Worktree, Harness, Repair, Claim, Self-merge, Base fix, Inherited failure, Spec/Ticket, stderr, `gh pr ready` | Nothing — these are the content |
| **Cut** | Spec runs, Architect/Weeding, Pickup, Security runs, Base fix, Continuation, Repair budgets, config, install | Nothing |
| **Keep** | Review stays human; ready = stood behind; failure preserves work; checkout untouched; not a sandbox | All of it, with mechanism |
| **CTA** | "Read How it works, then try one issue on a repo you own" — a 10-minute evaluation, not a signup | "Install it, read the prompts, run one issue" |

## Executive cut: thirdshift rules and outline

These add to the general rules in the skill's note.

1. Every factual on-screen claim traces to a line in `README.md`, `docs/guide.md`, `CONTEXT.md` or an ADR; list the source per beat before recording.
2. "Sandbox" appears only in a negation, or not at all (ADR 0013).
3. Worktree, Harness, Repair, Claim, Self-merge, Base fix, Inherited failure and Ticket do not appear.
4. "Ready for review" is stated as the default and merging as opt-in within the first 20 s (ADR 0004).
5. The spoken risk disclosure: sessions run unattended as your user, with your credentials.
6. The proof beat names the real Run with its date and both numbers (#663 → #665, 12 Oct 2026), or no proof beat runs.
7. Any cost mention says "API-price-equivalent figures in the session logs," never "cost" or "spend" unqualified.
8. It states that this is a personal MIT-licensed open-source project with no support commitment.

| # | ~s | Beat | Purpose | Repo source |
|---|---|---|---|---|
| 1 | 0–8 | "Give it a GitHub issue. It gives back a pull request ready for review — and you still decide whether it ships." | Answer first: the governing thought | `README.md:3`, `CONTEXT.md:3` |
| 2 | 8–18 | The problem: agent output that is almost right, and review that absorbs the gain | Name the believed risk before any benefit | `site/how-it-works/index.html`: "A pull request existing is not the finish line" |
| 3 | 18–30 | The answer: the agent implements, reviews its own change on two axes, then deterministic Git and CI checks run — and the PR is marked ready only when the factory stands behind it | The differentiated, fully substantiated claim | `README.md:7`, `docs/guide.md:29` steps 4–6, `docs/guide.md:731` |
| 4 | 30–40 | Proof: issue #663 taken at 00:21 on 12 October 2026; pull request #665 merged at 01:22 | A verifiable artifact instead of a benchmark | `site/index.html` |
| 5 | 40–52 | Failure: work is committed and pushed, the PR drops back to draft, rerun the same URL to continue | Answers the exec's real question | `docs/guide.md:730-733`, ADR 0002 |
| 6 | 52–62 | Limits, spoken: sessions run unattended as your user; the separate checkout is not a sandbox; the reviews don't guarantee correctness | Credibility, and FTC-safe | ADR 0013, `site/how-it-works/index.html:91` |
| 7 | 62–72 | CTA: "Read How it works, then point it at one issue in a repo you own. MIT-licensed, open source, a personal project." | Low-commitment evaluation, honest support expectations | `README.md` install + license |
