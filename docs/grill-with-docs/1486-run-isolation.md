# Grilling: #1486, isolation for Runs

Issue: https://github.com/JacobStephens2/thirdshift/issues/1486
Research: [run-isolation.md](../research/run-isolation.md) (PR #1515), [eval-sandboxes.md](../research/eval-sandboxes.md), [sandcastle-fit.md](../research/sandcastle-fit.md)

## Round 1 (open)

**Q1.1 - Hosted providers.** The research says no to Daytona, E2B, Vercel and the rest. Every one of them works as "code in, commits out", which breaks the shared `.git` a Run delivers through. The terms for subscription logins on a third-party VM are also unsettled. The options:
- (a) Decline hosted providers for Runs. Record the decline in the note, and retitle #1486 to local isolation for Runs.
- (b) Keep a hosted provider as an option for a later clone-per-Run design (§7.1 step 6).

*Recommended:* (a). Option (b) is only alive if Q1.3 picks a clone per Run, and the note already says to revisit at that point.

*Answer:* 

**Q1.2 - The push credential, first and on its own.** A per-Run GitHub App installation token lasts one hour and is scoped to one repository, with `contents: write` and `pull_requests: write`. It bounds a leaked token, and no sandbox can do that. thirdshift's own deterministic steps keep your `gh` login, including the Pickup reservation (ADR-0017). It does mean a GitHub App to register and run. The options:
- (a) File it as its own issue now, independent of the sandbox work, so it can land first.
- (b) Make it the first Ticket of the Run sandbox Spec.
- (c) Skip it for now; your `gh` login stays inside every session.

*Recommended:* (a). It doesn't depend on any sandbox decision, and it bounds the worst outcome today.

*Answer:* 

**Q1.3 - The shared `.git`: protect `config` and `hooks/`, or a clone per Run.** A writable `<main>/.git` lets code run on the host, because git runs `config` settings and `hooks/` as you, in every worktree. The options:
- (a) **Deny list.** Bind `<main>/.git` read-write, and bind `config` and `hooks/` read-only on top. Do the same for the other worktrees' `worktrees/<name>` directories. One design for every Run kind.
- (b) **Clone per Run.** Commits come back as patches, and thirdshift applies them. That removes the hole, but costs a clone per Run, the shared object store, and an apply step that needs its own recovery story.

*Recommended:* (a). The live check passed (see Research results). Commit, plain push, fetch and `worktree add` all work, and both writes to the protected paths were refused. Nothing in thirdshift's prompts, skills or git code writes `config` from a session.

*Answer:* 

**Q1.4 - Which Run kind first.** The research's staged path starts with a `thirdshift sandbox -- <cmd>` command. A Security review session would call it to run proof-of-concept code, and it would fold in #726. This sandboxes one command that a session runs, not the session itself. The options:
- (a) Security run proof-of-concept command first (#726), then opt-in session wrapping for the Architecture and Security reviews, then the implement session.
- (b) Session wrapping first, for the reviews. Leave #726 separate.
- (c) Session wrapping for every Run kind at once, behind one setting.

*Recommended:* (a). It is the one place the factory already fights for a boundary by hand: six of eight reviews rebuilt a bwrap call by trial and error. It also needs almost no network allowlist.

*Answer:* 

**Q1.5 - When the sandbox can't start.** A Trial becomes a Void Trial. A Run has no void. The options:
- (a) The Run fails, and the reason names the sandbox. Opting in means you asked for the boundary, so thirdshift never runs without it silently.
- (b) Fall back to today's unsandboxed session with a warning.

*Recommended:* (a). Claude Code has `sandbox.failIfUnavailable` because a silent fallback is a trap.

*Answer:* 

**Q1.6 - macOS.** On macOS the only mechanism for a native process is Seatbelt (`sandbox-exec`), which is deprecated and undocumented. The options:
- (a) Linux only. Turning the setting on for macOS is an error, which `thirdshift setup` or a validation step reports, and the docs say so.
- (b) Seatbelt on macOS, with a weaker boundary that the docs state.
- (c) A Linux VM on macOS.

*Recommended:* (a). The factory host is Linux. Your Mac runs Pickup, but a weaker boundary that looks like the Linux one is worse than an honest "not here".

*Answer:* 

**Q1.7 - ADR-0013.** The options:
- (a) Amend it, as §7.3 sets out:
  - Keep the flag table.
  - Add the stronger reason: every Harness's own sandbox makes `.git` read-only.
  - Replace "The worktree is the only boundary" with "the only boundary by default, plus an opt-in outer sandbox thirdshift controls".
  - Record the AppArmor fix.
  - Add that credential scoping, not isolation, bounds a credential leak.
- (b) Write a new ADR that supersedes it and restates the flag table.

*Recommended:* (a). The decision itself is unchanged, and the outer sandbox already exists for Trials.

*Answer:* 

**Q1.8 - Anthropic's `sandbox-runtime` (`srt`).** The options:
- (a) A reference only. thirdshift's own `src/sandbox.rs` and `src/proxy.rs` stay the implementation.
- (b) A dependency. That adds Node to the factory's runtime, against ADR-0001 and ADR-0003, and `srt` is a "beta research preview".

*Recommended:* (a).

*Answer:* 

**Q1.9 - Antigravity clause 6 and `agy`.** Google's terms call "using third party software, tools, or services to access the Service" a breach. They don't distinguish launching the official, unmodified binary. This applies today, with or without a sandbox. The options:
- (a) File it as its own `needs-triage` issue and keep it out of this grill.
- (b) Decide it here: read the clause as not covering the unmodified binary, move `agy` to API-key billing, or drop `agy`.

*Recommended:* (a). It is a terms decision about one Harness, not an isolation decision.

*Answer:* 

**Q1.10 - Codex's shared `auth.json`.** Codex says not to share one `auth.json` across concurrent jobs. `spec.parallel = 3` with Codex already does that. The options:
- (a) File it as its own issue, which proposes a `CODEX_HOME` per concurrent session (the way Grok Trials get a Trials home).
- (b) Record "it works" as the position and do nothing.
- (c) Fold it into the Run sandbox work, since a Run sandbox binds the Harness home anyway.

*Recommended:* (a). It applies today without a sandbox. Option (c) would tie a live risk to the slowest-moving work.

*Answer:* 

**Q1.11 - Naming the boundary.** `GLOSSARY.md` has no sandbox term. It mentions one only inside the Void Trial entry. This work introduces an opt-in boundary around a Run's sessions. The options:
- (a) Add **Run sandbox** ("an opt-in boundary thirdshift puts around a Run's sessions, beyond the Worktree") and **Trial sandbox** (the one every Trial runs in). Say that the Harnesses' own sandboxes are neither.
- (b) Add one term, **Sandbox**, covering both, with the kind given by context.

*Recommended:* (a). The two have different allowlists, homes and failure rules (Q1.5). A single word would blur exactly the distinction this grill depends on.

*Answer:* 

## Settled by earlier answers (shout if any is wrong)

- The goal of #1486 is isolation for Runs, not Trials (triage answer "2"). Trials and their sandbox belong to #1484 and eval-sandboxes.md.
- Isolation can't protect the Harness login or the push credential inside a session. The honest claim is a smaller blast radius: other repositories, other Runs' worktrees, `~/.ssh`, the Console on loopback, and `sudo`.
- #838 stays closed.
- The question in §8.1 Q0 is settled (see Research results): the Herdr on this Mac doesn't put thirdshift's Runs in a Vercel Sandbox.

## Research results (2026-10-28)

- **Seams exist.** `Sessions::within_sandboxed` is at `src/session.rs:170`, `trait Outside` at `:448`, and `OnMachine` with `sandbox: Option<Launch>` at `:472`. Confirmed by reading the source.
- **Herdr.** `herdr` 0.9.5 on this Mac is herdr.dev, a "terminal workspace manager for AI coding agents". Its integrations are pi, omp, claude, codex, copilot, devin, droid, kimi and opencode. Its help has no Vercel option, and its config has no Vercel mention. thirdshift's Runs on the host don't go through it. Confirmed by `herdr --help`, `herdr integration`, and a grep of `~/.config/herdr`.
- **Deny-list live check, on thirdshift.app with bwrap.** The setup: a worktree with `<main>/.git` bound read-write and `config` and `hooks/` bound read-only on top. Confirmed by running each command:
  - **Works:** `git commit`, a plain `git push`, `git fetch`, and `git worktree add`.
  - **Refused:** `git config core.hooksPath` ("could not write config file … Device or resource busy"), and writing `hooks/pre-commit` ("Read-only file system"). The host's config was unchanged afterwards.
  - **Breaks:** `git push -u` sends the branch but can't record its upstream, and it errors.
- **No session-side config writes.** No `push -u`, `--set-upstream` or `git config` appears in `src/prompt.rs`, `src/git.rs`, `src/worktree*` or `skills/`. Confirmed by `rg`. The Factory skills' `.git/info/exclude` entry is written by thirdshift's own code before a session starts, so it stays outside any sandbox.

## Waiting on research

- Nothing.
