# Grilling: #427, security work

Issue: https://github.com/JacobStephens2/thirdshift/issues/427
Research: `docs/research/security-agent-passes.md` (in the `427-security-grill` worktree, not yet committed)

## Round 1 (open)

**Q1 - Scope of #427.** What the factory is responsible for in security, and what goes elsewhere. It matters because each kind of problem has a better tool, and an LLM adds something only for flaws in a repo's own code. Prompt injection counts as one of those in thirdshift and vaulted-agent, whose job is passing text to agents. The factory's own attack surface is a separate problem: sessions read issue comments (`docs/agents/issue-tracker.md` says `gh issue view --comments`), which anyone can write on the five public repos, and Codex runs unsandboxed (ADR 0012) with a `repo`-scoped `gh` token. The options:
- (a) **Own-code vulnerabilities only.** Dependencies and committed secrets go to GitHub's features and to CI checks that compare against the base (Q3). The factory's own attack surface gets its own issue, filed now.
- (b) **Own-code vulnerabilities, dependencies and secrets,** all as factory work.
- (c) **All of that plus the factory's own attack surface,** in #427.

*Recommended:* (a). Deterministic tools find known-vulnerable dependencies and committed secrets precisely and for free. The factory's attack surface is open today: one benchmark found 66.5% of malicious issues got past every guardrail in Claude Code, Codex and Cursor, so it shouldn't wait for #427.

*Answer:* agree

**Q2 - Which shape first.** #427 proposes a Security axis in each Run's review (shape 2) and a security pass shaped like an Architect run (shape 1). The order decides what protects the repos first. The options:
- (a) **Shape 2 first,** shape 1 later.
- (b) **Shape 1 first.**
- (c) **Both** in one Spec.

*Recommended:* (a), for three reasons:
- **It targets the measured risk.** Agents write vulnerabilities into Merge runs that nobody reviews (`merge.always = true`), and keeplore, chart35 and cascade's web build deploy on merge.
- **There's nothing to disclose.** A vulnerability caught in an unmerged diff was never on the Base branch.
- **It's cheap.** It adds about one sub-agent per Run, the same order as the Standards or Spec one.

Shape 1 has the opposite profile: it can't confirm findings on this machine, it would hold one of the two build slots for hours, and it runs straight into the disclosure problem in Q4.

*Answer:* c

**Q3 - GitHub's free security features.** These are settings, separate from anything the factory does:
- Private vulnerability reporting on the five public repos.
- Secret scanning and push protection on thirdshift, cascade and keeplore. They're already on for muxboard and vaulted-agent.
- Dependabot alerts on all seven. This is the only free feature for chart35 and clave.
- CodeQL default setup on thirdshift, cascade, muxboard and vaulted-agent. Not keeplore: CodeQL has no PHP.
  - Its check fails any PR that adds a high or critical alert, so a vulnerability an agent writes stops the Self-merge. That's a deterministic gate for free.
  - Until the CI-fix Repair is told to read check-run annotations, such a Run fails and is left for you.
  - It also adds CodeQL's run time to each Run's CI wait.
- Dependabot security updates open PRs the factory never picks up. keeplore commits `vendor/`, which composer bumps wouldn't update.

The options:
- (a) **Everything except Dependabot security updates.**
- (b) **(a) without CodeQL,** until the CI-fix Repair reads check-run annotations.
- (c) **None for now.**

*Recommended:* (a). CodeQL failing a Run is the gate working, not a bug, and teaching the Repair about annotations can be a small follow-up.

*Answer:* agree, though file this as a separate issue on my https://github.com/JacobStephens2/ideas repo, as I'll want to probably turn this on separately for all my repos, but keep that work separate from this spec and tickets

**Q4 - Vulnerabilities already on the Base branch.** What a pass would find: who fixes them, and where they're written up. On a public repo, the public fix itself discloses the vulnerability, and GitHub has no private fix path the factory can run alone. The options:
- (a) **Fix unattended** when a proof-of-concept test reproduces the finding, through a normal Run. On a public repo, that means a public PR.
- (b) **Report privately and fix nothing unattended at first.**
  - The report goes to a draft security advisory on the public repos and to the Run notification everywhere.
  - The Day shift grades each finding as real, not real, or hardening, which gives you precision numbers for your own repos.
  - Then you decide on an opt-in for unattended fixes.
- (c) **Split by repo:**
  - Unattended where merging deploys the fix (keeplore, cascade's web build) or the repo is private (chart35, clave).
  - Report-only where users must update before they're protected (thirdshift, vaulted-agent, cascade's native apps).

*Recommended:* (b) now, with (c) as the likely shape of the later opt-in. Either way, an unfixed vulnerability is never described in a public issue or PR body.

*Answer:* a and b, depending on user config, like the base fix option

## Research results (2026-10-06)

- Unverified LLM security findings are mostly wrong: across 11 Python web apps, Claude Code (Sonnet 4) was right on 14% of its findings and Codex (o4-mini) on 18%, confirmed by source (Semgrep's 2025 study).
- Findings verified by a check the agent can't game are mostly right: 92.7% of 6,123 Anthropic findings were valid on external review, and Cloudflare treats a finding with no working proof-of-concept test against the untouched code as fake, confirmed by source (Anthropic's disclosure ledger; Cloudflare's blog).
- Current agents leave about 60% of their functionally correct solutions insecure (Claude Code with Opus 4.8, Codex with GPT-5.5, on the SusVibes leaderboard), confirmed by source.
- In a replay of 70 real vulnerabilities, a security-hardening skill cut reintroduction by 13.8 points, and "review your changes" by 1.9, confirmed by source (arXiv:2606.23130, Table 4).
- Security fixes that pass the proof of concept are often wrong: PatchBench solved 45.3% of tasks correctly, and 37.7% of Claude Code's fully validated AIxCC patches were semantically wrong, confirmed by source.
- A public fix discloses the vulnerability: from a public Firefox security diff, Mythos Preview wrote a working exploit in under an hour, confirmed by source (Anthropic's red team).
- GitHub's API can create a draft advisory and a temporary private fork, but no CI runs in the fork, and merging it is a web-UI button, confirmed by docs.
- This machine can't create a sandbox: `kernel.apparmor_restrict_unprivileged_userns = 1`, `unshare` fails, and bwrap, Docker and Podman aren't installed, confirmed by test.
- cloudflare/security-audit-skill (MIT, about 183 KB) confirms a finding only by running a proof of concept in an OS sandbox, asks a human in three places, and finds in one run about half of what repeated runs find, confirmed by source (the repository).
- alibaba/open-code-review (Apache-2.0) is a general diff reviewer in Go in which security is one of eight categories. It needs its own API key unless run in delegation mode, and its skill asks the user four times, confirmed by source (the repository).
- Claude Code's `/security-review` excludes prompt injection ("not a vulnerability") and calls Rust memory bugs impossible, and it runs only on Claude, confirmed by source (its MIT prompt) and docs.
- OpenAI's Codex Security CLI (Apache-2.0) runs `scan --diff` with `--fail-on-severity` on a ChatGPT login or an API key, confirmed by docs.
- A median Architecture review takes 6.7 minutes and $2.84 at list price, and a median implement session 16.1 minutes and $2.78 (Claude Code, `claude-opus-5-5` at medium, 3–5 October), confirmed by test (this machine's Command logs).
- Public repos get CodeQL (Rust yes, PHP no), secret scanning, private vulnerability reporting and SARIF upload for free; user-owned private repos get only Dependabot, confirmed by docs.
- On the seven repos, private vulnerability reporting, Dependabot alerts and CodeQL are off, and secret scanning is on only for muxboard and vaulted-agent, confirmed by test (`gh api`).
- Every Run on this machine is a Merge run (`merge.always = true`), and keeplore, chart35 and cascade's web build deploy on every push to `main`, confirmed by source (the User config and the deploy workflows).
- A failing CodeQL check explains itself in check-run annotations, not in the Actions log the CI-fix Repair is told to read, confirmed by docs.
- Dependency audits run on every PR would turn every Run red when a new advisory lands, and thirdshift wouldn't treat that as an Inherited failure until the Base branch re-runs the check, confirmed by source (inferred from `src/ci.rs`, ADR 0008 and the tools' docs).
- Sessions read issues with `--comments`, Codex runs unsandboxed, and the `gh` token has `repo` scope, confirmed by source (`docs/agents/issue-tracker.md`, ADR 0012) and test (`gh auth status`).
- 66.5% of malicious issues got past every guardrail in Cursor, Claude Code and Codex Desktop, confirmed by source (the IssueTrojanBench preprint).
