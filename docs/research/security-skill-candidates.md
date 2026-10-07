# Security skill candidates: which upstream skill the Security review and the security pass should embed

Research date: 2026-10-07.

**Question.** [security-agent-passes.md](security-agent-passes.md) surveyed the field on 6 October 2026. The grilling session on [#427](https://github.com/JacobStephens2/thirdshift/issues/427) then settled the shape:

- **A Security review.** When the user turns it on, a separate session in each Run reviews the Run's branch diff against the Base branch. It fixes the findings it can show with a failing test.
- **A security pass.** A command shaped like `thirdshift architect` audits a repository's Base branch. It records each finding privately: a draft GitHub security advisory on a public repository, a private issue on a private one.
- **One upstream skill for both.** It is embedded in thirdshift's MIT binary with its licence notice and kept as close to upstream as possible, so that readers can trust it because they trust its publisher. thirdshift adds only its process. The current pick is cloudflare/security-audit-skill, "unless research finds a better upstream skill".
- **Harness-agnostic and language-agnostic.** The skill must work under `claude -p` and `codex exec`, ideally under other agent CLIs, and on any codebase.
- **Proofs of concept run as ordinary tests** in the Run's worktree, unsandboxed like every Run's tests. This machine cannot create an OS sandbox, and thirdshift will run on a dedicated VM that serves no production code.

This note answers three questions:

1. Is there a better upstream skill than Cloudflare's for these two uses (§1)?
2. How exactly would Cloudflare's skill serve them? That covers a headless scoped diff run, its cost, what ties it to one CLI, its sandbox rule, its finding record against GitHub's advisory fields, threat-model documents, and its licence (§2).
3. What do primary sources do to keep per-pull-request security review precise (§3)?

**Method.**

- **Cloudflare's skill, read in full by me.** cloudflare/security-audit-skill was cloned on 7 October 2026 at `c1c8a8c`.
  - I read `README.md`, `LICENSE`, `SKILL.md`, `RECONNAISSANCE.md`, `HUNTING.md`, `VALIDATION-AND-REPORTING.md`, `ATTACK-CLASSES.md`, `report-schema.json` and the relevant checks in both validators, and searched the ten companions.
  - Read-only `gh api` calls gave me every open pull request and issue that bears on these questions, including the diffs of #58 and #60 and the third-party measurement linked from #20.
- **Candidates.** Each candidate was cloned for reading at its head on 7 October 2026; the commits are in Sources.
  - Licences are GitHub's SPDX identifier, checked against the licence files.
  - Stars, forks, open issues and dates come from `gh api repos/…` on 7 October.
  - Install counts come from skills.sh skill pages and its search API.
- **Delegated reading, re-checked.** Three background agents read:
  - the named candidates;
  - vendor skills, OWASP, registries and GitHub searches;
  - the per-pull-request practice of Google, Atlassian, Anthropic, OpenAI and Cloudflare.

  The second agent was stopped partway by Claude Code's cybersecurity safeguard (§2.3), and its report was cut off. I read what it had not reached myself. I re-checked against the source every number and quotation this note leans on. Facts read only by a background agent are marked so.
- **Primary sources only.** Every claim cites the repository file, the vendor's page or the paper.
- **Local facts.** These were checked on this machine on 7 October 2026: `claude` 2.1.292 and `codex` 0.160.1 (`--help` only), Node.js v26.10.0, and `kernel.apparmor_restrict_unprivileged_userns = 1`.
- **Nothing was run or changed.** No code from any cloned repository was executed, and no agent session was started. No GitHub setting, issue, advisory, label or fork was created.

**Evidence grades used below.**

| Grade | Meaning |
|---|---|
| Peer-reviewed | Journal or main conference, including industry tracks |
| Preprint | Not reviewed |
| Vendor docs | A publisher's own repository, documentation or policy, read at a stated commit or date |
| Vendor blog/research | A publisher's own posts and benchmarks, including a vendor's benchmark of competitors |
| Third-party report | A measurement by someone other than the publisher, not reviewed; their interest is stated where known |
| Registry data | Counts from GitHub's API or a skill registry, on 7 October 2026 |
| Local | A fact checked on this machine |

**Out of scope.** Designing the Security review or the security pass (§5 only lays out options), choosing a model, and anything only a live run could show, flagged **[needs live check]**. The precision, recall and disclosure evidence in [security-agent-passes.md](security-agent-passes.md) is cited, not repeated.

## TL;DR

- **Keep cloudflare/security-audit-skill.** No candidate meets all the constraints (§1). Ranked shortlist:
  1. **cloudflare/security-audit-skill** (MIT). The only self-contained, agent-neutral Markdown skill that does both a whole-repository audit and a diff-scoped run, with independent verifiers and a schema-checked verdict.
  2. **openai/codex-security's plugin skills** (Apache-2.0). The most capable alternative: diff and full scans, validation by focused test, and a reference for creating draft advisories. But they need the Codex runtime, their own MCP server and helper scripts, so they cannot be embedded as a skill.
  3. **The two review prompts in Anthropic's security-guidance plugin** (Apache-2.0). The best diff-review text, written for "External agentic harnesses". But they are Python constants, diff-only, and verify by reading, not running.
  4. **OpenSSF Alpha-Omega's Scrutineer** (MIT). A strong multi-CLI pipeline, but an application with its own API and containers, not a skill.
  5. **Google's Mantis** (Apache-2.0). A rigorous campaign toolkit, but "intended for demonstration purposes only", 575 KB of Markdown, with no pull-request mode.
  6. **getsentry/skills `security-review`.** Small and neutral, but its `LICENSE` points to CC BY-SA 4.0, and it cites language guides that do not exist.

  The rest fail on licence, Harness or scope: Trail of Bits, the `/security-review` prompt, Anthropic's defending-code harness, Vercel's deepsec, Gemini's extension, OpenAI's deprecated skills, vendor wrappers, Microsoft's Copilot agent, and OWASP's playbook.
- **A headless scoped diff run must be asked for in words** (§2.1). "Security review of this branch" would most likely get guidance mode: no verifiers and no `findings.json`. The Session prompt must name:
  - full audit mode with report artifacts;
  - the profile;
  - "the diff between two source refs" with both SHAs;
  - a budget that funds the reserves, or none;
  - an absolute output directory outside the worktree;
  - which prior runs to read, since the skill otherwise pulls earlier runs' open leads into its work.
- **Cost** (§2.2). Even scoped, every run spends four whole-repository reconnaissance agents, at least one hunter, one critic and one verifier per candidate: at least seven agent invocations. The one third-party measurement put a `quick` run at a median of $29.95 and 40 minutes with Claude Opus 5, against $2.06 for a plain session that found as much (one target, n = 3, an interested party).
- **Tied to one CLI** (§2.3). Nothing names a CLI, a tool or a `.claude/` path, and the skill declares itself "agent-neutral".
  - What remains: two role names that no Harness ships (`research`, `general`), "parallel sub-agents", and Node.js for the validators.
  - Sequential sub-agents would satisfy every evidence rule in principle. PR #58 proposes that, for Phase 5 only, and is unmerged. PR #60 documents a sibling-Docker sandbox and is unmerged. No community pull request has merged since 4 July.
- **The sandbox rule** (§2.4): "Run target-controlled builds, tests, processes, browsers, emulators, fuzzers, and fixture processing only inside an OS-enforced sandbox that provides all of these controls … If every control cannot be enforced, do not execute target code".
  - No operator declaration satisfies it.
  - The repository's own tests count as evidence only inside that sandbox, and new tests go in a scratch copy, never the target.
  - Unchanged, the skill confirms nothing on this machine.
  - Google's CodeMender documents the opposite policy: "Turn off the sandbox only in isolated, disposable environments". A minimal upstream change would add an opt-in execution policy of that kind.
  - Or thirdshift keeps the skill verbatim and runs the proof-of-concept tests itself.
- **Findings to advisories** (§2.5). `title` → `summary` and `overall_severity` → `severity` map cleanly. Everything else goes into `description`. The record has no CWE, package, ecosystem, version range or credit; thirdshift must supply those or leave them null. `needs_validation` records have no severity. OpenAI's Apache-2.0 GHSA reference is a ready-made rulebook for this step.
- **Threat models** (§2.6). No instruction tells reconnaissance to read one. Chrome, Codex Security and Anthropic's harness all feed one in.
- **Licence** (§2.7). MIT: keep Cloudflare's notice with every copy. thirdshift's `thirdshift-<skill>` naming forces a one-line change to the frontmatter `name`.
- **Per-pull-request practice** (§3):
  - Review the changed code plus bounded context.
  - Keep whole-repository scans off the pull-request path. Cloudflare: "the big scans are a periodic backlog sweep and not a per-PR check".
  - Suppress or label pre-existing issues.
  - Exclude low-signal classes, and refute every candidate in a fresh context.
  - Feed in the owner's `SECURITY.md` or threat model.
  - Start advisory-only.

  Cloudflare's own per-merge-request reviewer is a different, cheaper system: $1.19 per review on average, all reviewers included.

## 1. Is there a better upstream skill than Cloudflare's?

### 1.1 What the two uses need

To replace Cloudflare's skill, a candidate has to clear all of these:

- **Licence.** Embeddable in an MIT binary with a notice. CC-BY-SA, GPL/AGPL, FSL, proprietary, "no redistribution" or no licence rules it out.
- **Scope.** A diff-scoped review *and* a whole-repository audit, from one skill.
- **Harness.** Nothing that only one agent CLI has: no `AskUserQuestion`, `Workflow` tool, hooks, plugin runtime, private MCP server or `!`-command injection.
- **Language.** Usable on Rust, PHP, Python, TypeScript and Swift, not only on a list of web frameworks.
- **Headless.** Every question it asks can be answered in the Session prompt.
- **Verification.** Independent refutation, machine-readable verdicts, ideally backed by execution.
- **Trust.** A publisher a reader would trust, a maintained repository, adoption, and quality evidence of any kind.

### 1.2 The candidates

Repositories were read at their heads on 7 October 2026; all counts are registry data from that day.

**Fit.**

| Candidate (publisher) | Licence | Diff / whole repo | Tied to one CLI? | Languages | Asks the user? |
|---|---|---|---|---|---|
| cloudflare/security-audit-skill (Cloudflare) | MIT | Both: a scoped run over "the diff between two source refs", or a full audit | No CLI named. Needs sub-agents ("parallel" as written) and Node.js | Any: organised by trust boundary and attack class | Three places, all answerable in the prompt (§2.1) |
| openai/codex-security plugin skills (OpenAI) | Apache-2.0; no NOTICE file | Both: `security-diff-scan`; `security-scan`, `deep-security-scan` | **Yes.** Codex runtime, its own MCP server (`.mcp.json`), Node and Python helper scripts, `$skill` mentions | Any | In a non-interactive session, "never request user input"; phase skills otherwise "stop and ask the user" |
| security-guidance plugin, `hooks/review_api.py` (Anthropic) | Apache-2.0 | Diff only: each turn, commit and push | The plugin is Claude Code hooks. The two review prompts are importable constants, written so that "External agentic harnesses can import this directly" | Any | No |
| alpha-omega-security/scrutineer (OpenSSF Alpha-Omega) | MIT | Both: full scans, and diff rescans against a baseline scan | No single CLI ("claude-code by default, or codex, opencode, or GitHub Copilot CLI"). But every skill needs Scrutineer's own API, workspace and containers | Any | No; it is a web application |
| google/mantis (Google; "not an officially supported Google product") | Apache-2.0 | Whole-repository campaigns, with incremental re-review of files changed between passes; no pull-request mode | Says it "is platform agnostic" (Gemini CLI, Antigravity CLI, ADK). Reproduction needs Docker or gVisor | Any | Designed for unattended runs |
| getsentry/skills `security-review` (Sentry) | Repository Apache-2.0. **The skill's `LICENSE` is CC BY-SA 4.0** (OWASP-derived references) | "Report on: Only the specific file, diff, or code provided by the user" | `allowed-tools: Read Grep Glob Bash Task` (Claude names); the body is neutral | Guides for Python and JavaScript only. It links Go, Rust and Java guides that do not exist | No |
| Claude Code `/security-review` prompt, anthropics/claude-code-security-review (Anthropic) | MIT | Diff only | **Yes.** ``!`git diff …` `` injection, `allowed-tools`, Task sub-tasks, `origin/HEAD` | Any, but it excludes memory safety "in rust" and prompt injection | No |
| anthropics/defending-code-reference-harness (Anthropic) | **A non-standard Apache-2.0 variant**; GitHub reports NOASSERTION (background agent's diff; §9 wording re-checked) | Whole target directory only | **Yes.** `.claude/skills/`, Task sub-agents, AskUserQuestion in triage | Any | Triage interviews unless `--auto` |
| vercel-labs/deepsec (Vercel Labs) | Apache-2.0 with NOTICE | Both: `process --diff`, or `scan` → `process` | A CLI with codex, claude and pi back ends. It writes `.deepsec/` into the repository | Web applications | No; it is a CLI |
| trailofbits/skills (Trail of Bits) | **CC-BY-SA-4.0** | Per skill: `differential-review`, or whole-repository skills | **Yes.** Since August its main audit skills run as dynamic workflows through Claude Code's `Workflow` tool; several use AskUserQuestion | Per skill | Several |
| gemini-cli-extensions/security (Google) | Apache-2.0 | Both: `/security:analyze`, `/security:analyze-full` | **Yes.** A Gemini CLI extension with two MCP servers and `ask_user` | Any | Yes: twice in `analyze`, and above 20,000 lines in `analyze-full` |
| openai/skills `security-best-practices` (OpenAI) | Apache-2.0 per skill | A whole-codebase report | No | "python, javascript/typescript, go" only | Yes |

**Evidence.**

| Candidate | Verification and verdicts | Size | Maintenance and adoption | Published quality evidence |
|---|---|---|---|---|
| Cloudflare | A fresh refuting verifier per candidate, then a record verifier. `confirmed` needs sandboxed execution. JSON schema and two validators. `confirmed` / `needs_validation` / `rejected` | 183 KB of Markdown (15 files), plus validators | One maintainer; last commit 14 Sep; no community pull request merged since 4 Jul. 25.6K stars, 1.5K forks, 24.5K skills.sh installs | Third party, interested: recall 0.467, equal to one plain session's, at precision 0.90; $29.95 a run (n = 3, §2.2). README: one run finds "roughly half" of what repeated runs find |
| Codex Security skills | Validation prefers reproduction (a proof of concept, or "the smallest focused test"), else static tracing with a stated "proof gap"; then attack-path analysis. JSON and SARIF, with severity, confidence and CWE | 41 Markdown files, 461 KB, plus 44 helper scripts and an MCP application (background agent) | Created 13 Jul 2026; commits daily. 11.0K stars, 846 forks. 61 installs of `security-diff-scan` | Snyk VulnBench V2 (Snyk sells a competing scanner): recall 29–41% and precision 14–25% across three models on 20 application fixtures |
| security-guidance prompts | Investigate ("your job is RECALL, not precision"), then refute ("Default = SURVIVES unless you find concrete refuting evidence"). JSON schemas; nothing is run | Investigate prompt 10.6 KB; the refute prompt is built per call | v2.0.10 (5 Oct 2026) | Qualitative only |
| Scrutineer | `verify` re-runs a finding's reproduction against HEAD; `critic`; triage. Containers with an egress allowlist | 50 skills, 590 KB of Markdown | Created 17 Apr 2026; commits daily. 231 stars | None found |
| Mantis | Review verdicts, a critic, then tiered reproduction up to a sandboxed `reproduced` verdict | 19 skills, 575 KB of Markdown | Created 15 Jun 2026. 2.4K stars | Google's own, one withheld C driver: precision rose from 22% (2 of 9) to 75% (3 of 4) with its structural index |
| Sentry | The same agent researches and decides. HIGH, MEDIUM ("Needs verification") or LOW. Markdown output | 22 files, 218 KB | 1.0K stars. 19.1K installs; skills.sh security audits read Fail, Fail, Pass | None. The missing guides were reported in #165, closed "not_planned" on 22 Sep 2026 |
| `/security-review` | A parallel refuting sub-task per finding; drops anything below 8 of 10; read-only | 10.8 KB | Prompt unchanged since August 2025. The Action's defaults name retired models (§3) | VulnBench V2 (vendor of a competitor): Opus 5 at xhigh, recall 79% and precision 78% |
| Defending-code harness | Blind verifiers vote `true_positive`, `false_positive` or `cannot_verify`. `/patch` adds a regression test | `vuln-scan` 12 KB, `triage` 45 KB, `patch` 26 KB | "This repo is not maintained and is not accepting contributions." 7.6K stars | Anthropic, no sample size: an adversarial verifier "roughly halved the rate of non-exploitable findings"; a verifier that builds a proof of concept brought false positives "to near zero" |
| deepsec | `revalidate` re-checks findings, "cuts false-positive rate" | A CLI | 8.1K stars | VulnBench V2: localised recall 37.5% and precision 9.4% with Claude Opus 5 |
| Trail of Bits | `fp-check` gives TRUE or FALSE POSITIVE through gates; `differential-review` gives APPROVE, REJECT or CONDITIONAL | `differential-review` 33 KB (background agent) | Active. 7.4K stars | None published |
| Gemini extension | A self-review checklist; optional proof-of-concept skill | `analyze.toml` 11 KB, `GEMINI.md` 19 KB | Last commit 28 Apr 2026. 794 stars | 90% precision and 93% recall on the OpenSSF CVE Benchmark: JS/TS only, scored by hand, no counts, version of September 2025 |
| openai/skills | None | 401 KB of Markdown | **Repository deprecated** on 22 Jun 2026 | None |

### 1.3 The strongest alternatives, in detail

**openai/codex-security's skills (vendor docs).** The 6 October note treated this as a CLI; the repository also ships 15 `SKILL.md` files under `plugins/codex-security/skills/`.

- **Scope.** `security-diff-scan` says "Review every changed source file, including deleted files. Follow changed behavior into supporting code without expanding into an unrelated repository audit". It runs `$threat-model`, `$finding-discovery`, `$validation` and `$attack-path-analysis`, and finishes with `report.md` and SARIF.
- **Validation runs code.** "Prefer targeted, non-interactive reproduction or falsification when it is feasible and proportionate". One method: "add or adapt the smallest focused test that exercises the vulnerable code and asserts the vulnerable behavior" (`validation/SKILL.md`, lines 12 and 37). That is close to thirdshift's failing-test rule.
- **Not portable.**
  - The skills call the plugin's own MCP tools (`prepare_codex_security_review_items`, `record_codex_security_discovery_candidates` and others).
  - The SDK drives each scan as a Codex thread: "Use the installed $codex-security:${skillName} skill …" and "Run this Codex Security scan non-interactively." (`sdk/typescript/src/api.ts`, lines 4005–4006). It depends on `@openai/codex` and `@openai/codex-sdk` 0.162.0-alpha.16.
  - A "terminal workflow" exists for other hosts, but it still needs the helper scripts.
  - Non-interactive runs may also change the user's Codex configuration: "Automatically apply only the helper's concrete `value` or `remove` patches to its writable `user_config_path`" (`references/config-preflight.md`, line 83).
- **One reference worth borrowing.** `skills/track-findings/references/github-security-advisories.md` is a rulebook for exactly the security pass's advisory step (§2.5).
- **Licence.** The copy of the plugin in openai/plugins is labelled `"license": "Proprietary"` (background agent). Only this repository's Apache-2.0 copy is usable.

**The security-guidance review prompts (vendor docs).** `hooks/review_api.py` describes itself as the "importable surface for callers that want to run the same two-stage agentic security review as the CC plugin (investigate → self-refute) without going through the CC hook protocol".

- The investigate prompt counts "prompts sent to LLMs" among its sinks, which matters for thirdshift. The refute prompt rejects findings that are "PRE-EXISTING" (not on a `+` line), and demands "the specific +/- line in the diff that ENABLES the off-diff sink" for findings outside the diff.
- **It truncates.** It sends the diff as `diff_text[:8000]`.
- **It verifies by reading only.** Refutation defaults to "SURVIVES".
- **It is not a skill.** It is diff-only, so it cannot serve the security pass.

**Scrutineer (OpenSSF Alpha-Omega; vendor docs).** "A local tool for scanning open source repositories for security vulnerabilities and managing the disclosure process".

- **Isolation by default.** "A host without the selected runtime fails startup rather than silently weakening isolation; `--no-container` is the explicit, Claude-only escape hatch".
- **Diff rescans reuse earlier context.** They "reuse prior scan context and focus the next run on the changes between a baseline commit and the new commit". They fall back to a full scan when "the patch is too large" or "too many files changed" (`docs/diff-based-rescans.md`).
- **Not standalone.** Its skills read `./context.json` with the API base and token, and declare `scrutineer.requires`, so they are parts of its pipeline.

**Mantis (Google; vendor docs).**

- **Not production software.** The README says "This project is intended for demonstration purposes only. It is not intended for use in a production environment."
- **Reproduction is tiered.** It goes from a "Micro-Harness" up to a sandboxed `reproduced` verdict (background agent).
- **Its precision gain** comes from a structural index: "baseline 2 real findings out of 9 reported (22% precision) at 10.7M tokens; structural 3 real out of 4 reported (75% precision) at 5.0M tokens" (`reference/evals/README.md`, lines 98–99).

**Others, in one line each:**

- **Vendor skills wrap hosted scanners.**
  - AWS's `diff-scanning-with-aws-security-agent` needs an `agent_space_id`.
  - Snyk's `agent-scan` scans agents, MCP servers and skills, not code.
  - Endor Labs, Socket, Aikido and ZeroPath ship agent kits or MCP servers for their own services.
  - Semgrep's `semgrep/skills` is under the Semgrep Rules License v1.0, which forbids redistribution.
- **Microsoft's `hve-core` Security Reviewer** is a GitHub Copilot agent: Copilot tool names, "Interactive: Yes", and audit, diff and plan modes. Its OWASP skills declare `license: CC-BY-SA-4.0`.
- **The OWASP Secure Agent Playbook** is CC BY 4.0, but bundles ASVS data under CC BY-SA 4.0 (its `THIRD_PARTY_NOTICES.md`), and is packaged as a Claude Code plugin. OWASP's other agent-facing projects are risk lists and standards, not review skills, and are CC BY-SA 4.0: the Agentic Skills Top 10 covers "Security Risks and Mitigations for AI Agent Skills", and AISVS is a verification standard.
- **github/awesome-copilot's `security-review`** is a community contribution of 30 March 2026, not GitHub's own: one agent, scanning a path or the whole project.
- **Ghost Security's `ghost-scan-code`** ships as a Claude Code plugin marketplace.
- **GitHub Security Lab's taskflows** run on its own Python agent and the Copilot API. They are not a skill, but they publish the clearest field funnel ([GitHub blog](https://github.blog/security/how-to-scan-for-vulnerabilities-with-github-security-labs-open-source-ai-powered-framework/), 6 March 2026; vendor blog):
  - Over 40 repositories, "the LLM suggested 1,003 issues"; 139 survived the audit stage, and 91 after deduplication.
  - Of those 91, humans rejected 20 (22%) as false positives they "couldn't reproduce manually" and 52 (57%) as low severity.
- **Sentry's Warden** is FSL-1.1-ALv2, so it cannot be embedded. It is the pull-request runner Sentry uses.
  - Its benchmark scans "only the files tied to known vulnerabilities" in 86 historical Sentry vulnerabilities. Claude Opus 4.8 at high effort, on the Pi runtime, found 21 of 86 for $21.31 (vendor research).
  - Its MIT companion, getsentry/warden-skills, has seven diff-review skills (57 stars; not assessed in depth, background agent).

### 1.4 What registries, awesome lists and GitHub show

- **skills.sh** (registry data).
  - Cloudflare's skill page shows 24.5K installs.
  - The search API, which does not return Cloudflare's skill at all, ranks product-specific and checklist skills first. Examples: `firebase-security-rules-auditor` (128,554 installs), `addyosmani/agent-skills/security-and-hardening` (52,621), `getsentry/skills/security-review` (19,035).
  - Install counts measure popularity, not suitability: most of these are guidance or product checklists, not audits.
- **Awesome lists.**
  - VoltAgent/awesome-agent-skills (35K stars) lists Cloudflare's skill.
  - travisvn/awesome-claude-skills (15K) lists Trail of Bits.
  - hesreallyhim/awesome-claude-code (55K) lists anthropics/claude-code-security-review.
  - ComposioHQ/awesome-claude-skills (77K) lists no code-audit skill.
- **GitHub search.** A search for "security audit skill" sorted by stars puts Cloudflare first (25,665 stars), then Trail of Bits (7,408), then two repositories near 1,100 stars, one of them AGPL-3.0.
  - Sorting by date created from 1 July 2026 turns up offensive-security frameworks and single-author Claude Code plugins, but no new agent-neutral audit skill.
  - The serious arrivals of 2026 are all vendor-backed, and all are runtimes rather than skills: Codex Security (13 July), Mantis (15 June), Anthropic's harness (22 May), deepsec (30 April) and Scrutineer (17 April).
- **Official skill collections.** google/skills has only cloud-product security skills (GKE, WAF). microsoft/skills has none for code review. OpenAI deprecated openai/skills: "**This repository is deprecated.**" (README; commit `778b0e6`, 22 June 2026).

### 1.5 Ranked shortlist and the answer

1. **cloudflare/security-audit-skill. Keep it.** It is the only candidate that is at once:
   - MIT;
   - a self-contained Markdown skill with no CLI-specific tools;
   - language-agnostic by design;
   - able to do a whole-repository audit and a diff-scoped run;
   - checked by independent verifiers, with a schema and validators;
   - from a publisher with real security operations behind it;
   - the most adopted skill of its kind.

   Its weaknesses are real (§2): per-diff cost, a sandbox rule that blocks `confirmed` on this machine, no CWE, no threat-model input, and a single maintainer who has merged no community pull request since July. None of them is fixed by switching.
2. **openai/codex-security's skills.** Better on capability and on pull-request fit, but bound to the Codex runtime. Borrow its GHSA-draft rules as a design reference, with attribution, not its skills.
3. **Anthropic's security-guidance prompts.** The best second source if per-Run cost forces a lighter diff review, at the price of a second upstream text.
4. **Scrutineer.** Worth watching as the agent-neutral pipeline closest in spirit, but it is an application.
5. **Mantis.** Rigorous but demonstration-grade and large.
6. **getsentry `security-review`.** Excluded by its CC BY-SA licence pointer and missing guides.

## 2. Cloudflare's skill in thirdshift's two uses

cloudflare/security-audit-skill was cloned again on 7 October 2026. Its head is still `c1c8a8c` (14 September 2026), the commit [security-agent-passes.md](security-agent-passes.md) §2.1 read, and GitHub reports no push since. Line numbers below are at that commit, in `skills/security-audit/`.

### 2.1 Asking for a scoped diff run, headless

**The request is prose.** The diff appears in one sentence (SKILL.md, "Run profiles and scope", line 115):

> A **scoped run** audits a subset: named paths, one subsystem, one companion domain, or the diff between two source refs. Seed ledger units only for in-scope surfaces and record everything else as `out_of_scope` — never as `covered`. A scoped or `quick` run must present itself as partial coverage.

There is no flag, command or file format for it. `run-metadata.json` records `source_ref` ("the reviewed commit and whether the worktree is dirty") and `scope_paths`, and has no field for a second ref (lines 52 and 86).

**Asked for loosely, a diff review gets guidance mode** (SKILL.md, "Operating modes", lines 12–17):

- "This skill is guidance by default." Guidance mode covers "security questions, focused reviews, methodology, triage, or investigation of specific findings". In it the agent must "not automatically run all six phases, create an output directory, or write audit artifacts".
- Full audit mode runs only "when the user explicitly asks to audit or pen-test a codebase, asks for a full, comprehensive, or end-to-end security review, or requests report artifacts".
- "If the request could mean either mode, ask one focused question before creating files or starting the complete workflow."

A "security review of this branch" reads as a focused review. It would get no `findings.json`, no verifiers and no validators (inference).

**What a Session prompt must state so that nothing is asked:**

| Item | What the skill does if the prompt is silent | What to state |
|---|---|---|
| Mode | Asks "one focused question" when the request could mean either mode (SKILL.md 17) | Full audit mode, with its report artifacts written |
| Profile | "pick a profile from the user's request or propose one from the target's size and stakes … The default is `standard`" (109) | `quick` or `standard`, by name |
| Scope | Audits the whole target | "A scoped run over the diff between `<base sha>` and `<head sha>`", or nothing for a whole-repository pass |
| Budget | None is required. If one is set and cannot fund the reserves: "ask for a larger budget, narrower scope, or different profile" (123). Before wave 1: "propose either a tighter scope or a coarser profile" (132) | No budget, or one that funds at least the reserves, plus: if the budget cannot cover the plan, end the run `incomplete` as the skill describes and do not ask |
| Output directory | Uses `~/security-audit-skill/<repo-name>/run-<N>`. A directory inside the target is allowed only if the user chose it and git ignores it, "Otherwise stop and request an external path" (51) | An absolute path outside the worktree that the session may write to |
| Prior runs | Reads "every compatible `coverage-ledger.json` and `findings.json`" from earlier runs and turns their open leads into current work (SKILL.md 96–101; RECONNAISSANCE.md 61–69) | Which earlier runs to read, or that there are none |

**The last row matters for a per-Run review.**

- With the default output root, every Run's review would find the security pass's runs and every other Run's review of the same repository.
- SKILL.md line 101 says "Make prior `needs_validation`, `deferred`, `blocked`, `out_of_scope`, and any changed-source unit current work". RECONNAISSANCE.md line 137 limits that to units "now in scope".
- Either way, a Run's review would inherit work it was not asked to do (inference).

**Two permissions also have to hold** [needs live check]:

- **The session must be able to write the output directory.** In a third party's weakest-model re-runs (§2.2), "the complete final report was written to the session's working scratch instead of the requested output directory, because that directory sat outside the access the run had been granted" ([issue #20](https://github.com/cloudflare/security-audit-skill/issues/20), gap 6). Claude Code takes extra directories with `--add-dir` (`claude --help`, 2.1.292).
- **Sub-agents must be allowed to run commands unprompted.** The same runs saw "worker sub-agents hitting "permission prompts are not available in this context"".

### 2.2 What a `quick` run and a scoped diff run spend

The skill counts spend in agent invocations: "record `budget` in `run-metadata.json` as a maximum number of agent invocations across all phases" (SKILL.md 121).

| Phase | `quick` | `standard` | Where |
|---|---|---|---|
| Reconnaissance | Four `research` agents (1a–1d), plus focused ones for "materially distinct deployment modes or subsystems that these four do not map" | Same | RECONNAISSANCE.md 7–57 |
| Hunting | Exactly one wave: a `general` hunter per ledger unit or group of related units, with units coarsened to surface × boundary × attack class | Waves until a critic pass comes back clean | SKILL.md 111–113; HUNTING.md 5 |
| Coverage critics | Exactly one, after the wave | One after each wave, plus "one distinct final-clean critic" | HUNTING.md 223–249 |
| Validation | One fresh verifier per candidate, which also does the Phase 5 record check | A `general` verifier (Phase 3) and a `research` verifier (Phase 5) per candidate, plus another for each material replacement | VALIDATION-AND-REPORTING.md 5, 122–124, 145 |
| Reserved before any hunter launches | 4 + 1 critic + 1 verifier = 6 | 4 + 2 critics + 1 verifier = 7 | SKILL.md 123 |

So a `quick` run with *H* hunters and *C* candidates spends 4 + *H* + 1 + *C* invocations. It spends more if reconnaissance adds focused agents, or if a malformed result is re-run with a fresh agent (VALIDATION-AND-REPORTING.md 89).

**A scoped diff run saves hunters and verifiers, not reconnaissance.**

- The four reconnaissance prompts read the whole target. Agent 1a: "Read the target at <target>". Agent 1c: "Inventory every source-visible place external or lower-trust input enters" (RECONNAISSANCE.md 12 and 38).
- A scoped ledger keeps "discovered excluded surfaces as `out_of_scope` units so later full runs can turn them into current work" (line 92). That presumes the whole map.
- The budget gate reserves "the four baseline reconnaissance calls" for every run (SKILL.md 123).

So the floor for a `quick` diff review is seven invocations: four whole-repository reconnaissance agents, at least one hunter, one critic and at least one verifier. thirdshift's code review runs two sub-agents today.

**The one cost measurement (third-party report).**

- **Who and how.** TheColliery ran the skill at `c1c8a8c` three times on one seeded multi-tenant web service, in the `quick` profile ([report](https://github.com/TheColliery/.github/blob/main/benchmarks/CoalBoard/SECURITY-AUDIT-2026-09-16.md), 16 September 2026; summarised in [issue #20](https://github.com/cloudflare/security-audit-skill/issues/20)).
  - Each run was a fresh `claude -p` session in `auto` permission mode, the mode thirdshift uses, on Claude Code 2.1.273 with Claude Opus 5.
  - It ran beside the authors' own tool and a plain single-session control.

| Arm (n = 3) | Median cost (range) | Median wall time | Median recall (15 seeded bugs) | Median strict precision |
|---|---|---|---|---|
| Cloudflare skill, `quick` | $29.95 ($21.97–$32.27) | 2,392 s | 0.467 | 0.900 |
| One plain `claude -p` session | $2.06 ($1.74–$2.87) | 412 s | 0.467 | 1.000 |

- **It finishes unattended.** All nine runs exited 0, and the skill's runs wrote their artifacts, so the `quick` profile does run to the end under `claude -p`.
- **Neither arm hit either of the two decoys.**
- **Misses.** The report names classes the skill missed:
  - an advisory against a pinned dependency published after the model's training cutoff;
  - locale-specific correctness;
  - a defect it marked as hardening because its reachability depended on deployment.
- **Read with care.** One target, three runs per arm, scored by the authors, who build a competing tool (it scored 0.867 recall at $7.66).
- **Scale.** An implement session's median here is $2.78 and 16.1 minutes at list price ([security-agent-passes.md](security-agent-passes.md) §6.2). So a `quick` run cost about ten implement sessions, on a purpose-built target of unstated size, with a different model (inference).

### 2.3 What is tied to one agent CLI

**Nothing names a CLI.**

- A search of every file at `c1c8a8c` finds none of `.claude`, `claude`, `codex`, `anthropic`, `openai`, `gemini`, and no tool name such as `AskUserQuestion`, `TodoWrite`, `WebFetch`, `Glob`, `Grep` or `Bash`.
- The frontmatter has only `name` and `description`, with no `allowed-tools`.
- The skill declares itself neutral (SKILL.md, "Platform terminology", lines 21–29):

> This skill is agent-neutral:
>
> - **Parent** is the agent that coordinates the run and owns shared state.
> - **Task tool** is the platform's delegation or sub-agent mechanism.
> - **`research` agent** is a delegated agent for focused source exploration and factual verification.
> - **`general` agent** is a delegated agent for broad investigation and bounded local execution.
> - **`subagent_type:`** in a heading names which of these two delegated agent roles runs that work.
>
> Use equivalent platform capabilities while preserving role, write-isolation, prompt, and independence boundaries.

**What remains is vocabulary and requirements:**

- **Two role names that no Harness ships.** "Task tool" and `subagent_type` are Claude Code's words, defined generically above.
  - Claude Code's built-in sub-agents are Explore, Plan, general-purpose and a few helpers. Custom ones can be passed per session with `--agents` ([sub-agents](https://code.claude.com/docs/en/sub-agents); `claude --help`).
  - Codex's built-in roles are `default`, `explorer` and `worker` ([`codex-rs/core/src/agent/role.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/agent/role.rs), main at `7ac954e`, 6 October 2026).
  - Antigravity's "`invoke_subagent` built-ins are `research`, `browser`, and `self`. There is no `general` type", according to an open pull request ([#13](https://github.com/cloudflare/security-audit-skill/pull/13)).
  - The parent maps the roles itself. TheColliery's runs show Claude Code doing so to completion (§2.2). Codex is untested **[needs live check]**.
- **Sub-agents, and parallel ones.**
  - The README asks for "a model that supports tool use and parallel sub-agents".
  - Reconnaissance says "Launch several `research` agents in parallel" (RECONNAISSANCE.md 7). Phase 5 says "Launch one fresh `research` verifier per final `confirmed` and `needs_validation` record, in parallel" (VALIDATION-AND-REPORTING.md 122).
  - Claude Code and Codex both have sub-agents. `codex exec` drops sub-agent threads' events from its JSONL ([codex-headless-harness.md](codex-headless-harness.md)), which affects the Session log, not the run.
- **Node.js** for the two validators (`node <skill-dir>/validate-findings.cjs`). It is installed here (v26.10.0).

**Could it run without parallel sub-agents?**

- **Sequentially, yes in principle.** No evidence rule depends on parallelism, only on fresh agents: "The agent that checks a finding is never the agent that found it" (README).
- **With no sub-agents at all, no.** The independence rule cannot be met inside one context.

**The two open pull requests.** Both were unmerged on 7 October 2026.

- **[#58](https://github.com/cloudflare/security-audit-skill/pull/58), "Add sequential fallback for unstable parallel agents"** (third party, 25 September).
  - It changes one line, in Phase 5 only: "When the platform supports parallel delegates, these can run in parallel; when it destabilizes or kills parallel sub-agents, run the same verifier tasks sequentially in fresh agents with the same prompts and evidence boundaries."
  - Reconnaissance keeps its "in parallel".
  - It says it fixes #11, "Antigravity kills the parallel subagents".
  - Its one approval is from an account outside Cloudflare; no maintainer has responded.
- **[#60](https://github.com/cloudflare/security-audit-skill/pull/60), "Add sibling-Docker sandbox provisioning guide"** (third party, 28 September).
  - It adds `SANDBOX-DOCKER.md`, which maps each control to a `docker run` flag: `--network none`, `--env-file /dev/null`, `--read-only` with the target mounted `:ro`, tmpfs scratch, memory, process, CPU and file-size limits, an outer `timeout`, and `--cap-drop ALL --security-opt no-new-privileges`.
  - It adds one paragraph to SKILL.md: "**Before declaring the sandbox unavailable, try to provision one.**"
  - It reports that inside containers, runs had concluded "no OS sandbox" and downgraded "**every** lead to `needs_validation`", and that the sibling container "promoted several leads from `needs_validation` to confirmed".
  - It needs a Docker daemon, which this machine lacks. It has no review.

**Maintenance.** One Cloudflare engineer (`literally-dan`) wrote 11 of the 14 commits. Community pull requests were merged until 4 July (#2, #3, #4), and none since. 55 issues and pull requests are open.

**Not the skill's doing, but security work under Claude Code meets a model safeguard.**

- **Docs.** "Fable models, Opus 5.5, Sonnet 5.5, and Opus 5 run with safety classifiers, which most often flag cybersecurity and biology content."
  - For Opus 5.5, "cybersecurity-flagged requests re-run on Opus 4.8", and "After a fallback, the session continues on the fallback model" ([model configuration, "Automatic model fallback"](https://code.claude.com/docs/en/model-config#automatic-model-fallback)).
  - Of the API's `cyber` refusal category: "Benign cybersecurity work can also trigger this category" ([refusals and fallback](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback)).
- **TheColliery's receipts** show `claude-opus-4-8` beside `claude-opus-5` in all nine runs, and the report's addendum puts this down to the fallback.
- **This research hit it.** On this machine, one of this note's three background agents (Claude Code, Opus 5.5) was stopped mid-task. It was reading security-skill repositories, and stopped with "API Error: Opus 5.5's safeguards flagged this session … Claude Code can't respond to your last message with Opus 5.5 … Details: `[cyber]`" (Local, one occurrence). No fallback happened.
- So a security session can change Model partway or end outright.

### 2.4 The sandbox rule

**The rule** (SKILL.md, "Universal execution safety", lines 33–40, abridged only where marked):

> These rules apply in both operating modes. Source inspection is read-only. Run target-controlled builds, tests, processes, browsers, emulators, fuzzers, and fixture processing only inside an OS-enforced sandbox that provides all of these controls:
>
> - no external network; use only an isolated loopback namespace when the check needs local client/server traffic;
> - an empty environment populated from an explicit allowlist with safe values, with scratch-local `HOME`, temporary directories, and caches;
> - a read-only target and toolchain, with the target-controlled process able to write only inside its assigned `scratch/` directory; and
> - explicit low CPU, memory, process, file-size, disk, and wall-clock limits.
>
> The agent, outside the target-controlled process, may make a disposable source copy in an assigned `scratch/` directory when a build must write beside source. […] Do not install dependencies or let builds fetch them. Use only tools and dependencies already available locally. If every control cannot be enforced, do not execute target code: report the missing sandbox capability as a needs-validation blocker and give a safe validation plan.

**The rule is repeated in three more places:**

- inside the prompt blocks copied verbatim into every hunter: "If any control is unavailable, do not execute: return needs_validation with that exact blocker" (HUNTING.md 77–78);
- into every verifier: "If any control is unavailable, do not execute; retain the exact missing capability as a needs_validation blocker" (VALIDATION-AND-REPORTING.md 16–17);
- in the README: "Without these controls, the workflow keeps the lead as `needs_validation` instead of executing target code."

**`confirmed` needs execution.**

- The candidate gate says "A proposed confirmed record needs a bounded local observed result" (HUNTING.md 145).
- The schema requires `execution.observed_result`, and the validator rejects a confirmed record without one (`validate-findings.cjs` lines 553–554).
- The validator cannot tell how that result was obtained: "Validator success proves format and ledger consistency only" (VALIDATION-AND-REPORTING.md 118).

**Can an operator declare the environment? No.**

- The rule names no exception, and the skill reads no operator statement about the host.
- The only related field is `execution_policy: "sandboxed-source-and-local-only"`, a fixed value the parent writes into `run-metadata.json` (SKILL.md 86).
- A declaration that the machine is a dedicated, disposable VM would not supply the four controls either. The VM keeps external network (the Harness needs it), the worktree is writable, and the session's home directory holds the machine's GitHub, model and Resend credentials (inference).

**Can a proof of concept run by the repository's own test suite count? As a method yes, as thirdshift plans to run it no.**

- **The repository's own tests are valid checks.** The skill names "a minimal function harness, existing unit test, small parser fixture, dummy-tenant integration test, locally rendered configuration, or bounded isolated-loopback client" (SKILL.md 144). Hunters are told to "Prefer an existing unit test, minimal function harness, dummy-tenant service call, small malformed fixture, deterministic race schedule, or locally rendered policy" (HUNTING.md 78–79).
- **But they must run inside the sandbox, and new test code goes in a scratch copy, never the target.**
  - "The audit describes fixes; it does not modify target source" (SKILL.md 166).
  - "Agents may not change shared files, target source, retained artifacts, or another agent's directory" (SKILL.md 66).
- **And a `local` check's evidence must be promoted by "trusted parent-side code".** It is a file "promoted only by trusted parent-side code", copied by an eleven-step no-follow descriptor procedure (SKILL.md 68–80; RECONNAISSANCE.md 152).
  - The repository ships no such code: only the two validators and their tests.
  - Something outside the skill has to supply it, or the parent model has to write it (inference).

So a failing test run unsandboxed in the Run's worktree breaks two rules: it writes to the target, and it runs target code outside the sandbox. Run verbatim on this machine, the skill would end with every lead `needs_validation` or `rejected`, as [security-agent-passes.md](security-agent-passes.md) §2.1 found.

**Precedent for the opposite policy.** Google's CodeMender runs its own continuous-integration pipelines unsandboxed and says so:

> The pipelines turn off the CodeMender sandbox with --sandbox=false. Commands that the agent runs, such as builds and tests when it fixes a finding, have the same access as the job, including its Google Cloud credentials and network access. The Linux sandbox uses user namespaces, which some CI environments, such as unprivileged containers, don't support. Turn off the sandbox only in isolated, disposable environments.

([CodeMender, integrate with CI/CD](https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender/integrate-with-cicd), updated 6 October 2026; vendor docs.) Scrutineer, by contrast, refuses to start without a container runtime and keeps `--no-container` as an explicit escape hatch (§1.3).

**What a minimal upstreamable change could look like.** Three shapes, from most to least likely to be accepted. This is my judgement: no maintainer has said anything.

1. **Let the environment provide the controls.**
   - One paragraph in "Universal execution safety": the controls may be enforced by the environment the agent runs in, when the request names which ones; `run-metadata.json` and `REPORT.md` record it.
   - It weakens nothing, and #60 shows the demand.
   - It does not help thirdshift's VM, which enforces none of the four.
2. **An opt-in policy for the target's own tests.**
   - A second `execution_policy` value, such as `operator_accepted_host_tests`, that only an explicit request can select, worded after CodeMender's "only in isolated, disposable environments".
   - Under it the agent may run the target's existing test command, and new minimal tests it writes in a scratch copy, without an OS sandbox. It must still use an empty allowlisted environment, a wall-clock limit, no dependency fetches and dummy data.
   - `REPORT.md` and each confirmed record's `execution.instructions` would state the policy.
   - The text would change wherever the rule is repeated: SKILL.md lines 33–40 and 86, HUNTING.md 71–80, VALIDATION-AND-REPORTING.md 12–17 and 159, and the README's "Requirements".
   - It relaxes a safety rule, so acceptance is uncertain.
3. **No upstream change.**
   - Keep the skill verbatim and accept that its verdicts are `needs_validation` and `rejected`.
   - thirdshift's own process, outside the skill, turns each `validation_plan.local` into a failing test in the Run's worktree.
   - The skill text stays untouched, but `needs_validation` records carry no severity and no remediation (§2.5).

### 2.5 From `findings.json` to a draft security advisory

**The record.** Every branch of `report-schema.json` has `additionalProperties: false`.

- **Every verdict:**
  - `verdict` and `fingerprint` (stable, pattern `^[A-Za-z0-9][A-Za-z0-9._:/@+-]*$`);
  - `title` and `description`;
  - `trace`: an ordered list of `{kind: entrypoint|propagation|sink, file, line, scope, description}`;
  - `evidence`: a list of `{file, line, description}`.
- **`confirmed` adds:**
  - `root_cause` and `intended_behavior`;
  - `conditions` (`{kind, description}`, with kinds such as `authentication_level`, `user_interaction` and `system_configuration`);
  - `execution` (`attacker_perspective`, `payloads`, `instructions`, `observed_result`);
  - `remediation` (`strategy`, and optionally `code_changes: [{file_name, fixed_code}]`);
  - `severity`: `likelihood` and `impact`, each a `score` and `reason`, plus `overall_severity` (informational, low, medium, high or critical);
  - `confidence`: a `score` (low, medium or high) and a `reason`.
- **`needs_validation` adds** `claimed_root_cause`, `blockers` and `validation_plan` (`local` and/or `deployment`). It has no severity.
- **`rejected` adds** `claimed_root_cause` and `reason`.

There is no CWE, CVSS, affected-version, package or credit field anywhere in the skill: a search for "cwe" across all 20 files finds nothing.

**The advisory.** `POST /repos/{owner}/{repo}/security-advisories` ([REST reference](https://docs.github.com/en/rest/security-advisories/repository-advisories#create-a-repository-security-advisory), API version 2026-03-10; vendor docs).

- **Required:** `summary` (at most 1,024 characters), `description` (at most 65,535) and `vulnerabilities`.
  - `vulnerabilities` is a list of `{package: {ecosystem, name}, vulnerable_version_range, patched_versions, vulnerable_functions}`.
  - `ecosystem` is one of `rubygems`, `npm`, `pip`, `maven`, `nuget`, `composer`, `go`, `rust`, `erlang`, `actions`, `pub`, `other`, `swift`.
- **Optional:** `cve_id`, `cwe_ids`, `credits` (`{login, type}`), `severity` (`critical`, `high`, `medium`, `low` or null), `cvss_vector_string`, and `start_private_fork`. Of severity and CVSS: "You must choose between setting this field or severity".
- **Public repositories only, per the docs.** "Repository security advisories allow maintainers of public repositories to privately discuss and fix a security vulnerability in a project" ([about repository security advisories](https://docs.github.com/en/code-security/concepts/vulnerability-reporting-and-management/repository-security-advisories)).

| Advisory field | From the finding | Gap |
|---|---|---|
| `summary` | `title` | None |
| `description` | Composed Markdown: `description`, `root_cause`, `intended_behavior`, the `trace` as `file:line` steps, `evidence`, `conditions`, `execution` (with `observed_result`), `remediation`, the severity reasons and `confidence`. Add the reviewed commit (`source_ref`) and the `fingerprint`, so a later pass can find the draft again | thirdshift writes the template |
| `severity` | `severity.overall_severity` | `informational` has no advisory value: map it to `low`, or skip the advisory. `needs_validation` records have no severity |
| `cvss_vector_string` | None | Leave null and use `severity` |
| `cwe_ids` | None | The skill classifies by its own attack classes (the ledger's `attack_class`, such as `ATTACK-CLASSES.md#Injection`), not by CWE. A CWE has to come from another step, or stay null |
| `vulnerabilities[].package` | None | thirdshift knows the ecosystem and name from the manifest (`Cargo.toml` → `rust`, `composer.json` → `composer`, …). An application that is not a package uses `other` |
| `vulnerable_version_range` | None | The finding names one commit. A range has to come from release tags, such as `<= 0.9.0` |
| `patched_versions` | None | Null until a fix ships |
| `vulnerable_functions` | The `scope` of the `trace` steps, especially the sink's | `scope` is free text, not always a function name |
| Vulnerable paths | `trace[].file`, `evidence[].file` | No advisory field; they go in `description` |
| `credits` | None | `login` must be a GitHub user. A credit would be the operator's own account, or nothing |
| `start_private_fork` | — | `false`: CI cannot run in the fork ([security-agent-passes.md](security-agent-passes.md) §5.2) |

**OpenAI has already written the rules for this step.** Codex Security's `track-findings/references/github-security-advisories.md` (Apache-2.0; vendor docs) says:

- "Create one maintainer-owned private draft for one validated finding."
- "A scanned commit does not establish affected releases."
- "Provide exactly one of a validated `cvss_vector_string` or GitHub `severity`. Do not derive a vector from a score or prose or map informational severity. Include only high-confidence root-cause CWEs. Leave `cve_id`, `credits`, and `start_private_fork` unset."
- "The description is eventually public."
- On duplicates: "The API has no idempotency key or full-text advisory search. Paginate all four states", matching "exact finding-id and fingerprint bindings first".

### 2.6 Does reconnaissance read the repository's own threat model?

**No instruction says so.**

- The four reconnaissance prompts read source, build and test files. The only mention of documentation is Agent 1a's "Comparable software or protocol visible from local documentation and dependencies" (RECONNAISSANCE.md 17).
- "Prior evidence" in the phase list means earlier runs' ledgers and findings (SKILL.md 96–105).
- Nothing in the skill mentions a threat model, `SECURITY.md` or a design document.
- Intended behaviour is derived from source, and anything else becomes a `needs_validation` fact: "If they are required and absent from the repository, do not assume either presence or absence" (SKILL.md 146–148).
- An agent may open such a file on its own, but nothing directs it to.

**Others feed one in:**

- **Chrome** encourages "developers to add SECURITY.md files, which help models better understand trust boundaries and develop an accurate view of the threat model", and added "a "critic" agent with a separate context to consume these SECURITY.md files" (§3). Chromium's FAQ for its developers: "Adding a `SECURITY.md` file to your directory that describes your security boundaries will be read by the tools and will cause bugs to be filtered out based on this."
- **Codex Security's** diff skill applies "relevant `SECURITY.md` guidance". Its hosted review takes "the path to a threat model file checked into the repository".
- **Anthropic's harness** `vuln-scan` "Reads a target directory (and THREAT_MODEL.md if present)".
- **Why it matters.** Anthropic's write-up gives the reason: one team's findings "were reproducible and the PoCs proved exploitability", yet the owners "dismissed them as false positives because the bugs didn't fit the project's threat model" (anthropics/defending-code-reference-harness, `docs/blog-post.md` line 52; vendor blog).

### 2.7 Licence, and what embedding it verbatim requires

**The licence.** `LICENSE` is the standard MIT text, "Copyright (c) 2025-2026 Cloudflare, Inc.". Its one condition: "The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software." There is no NOTICE file. Unlike Apache-2.0, MIT does not require marking changed files.

**For thirdshift that means** (inference from the licence and [ADR 0001](../adr/0001-rust-binary-with-embedded-skills.md)):

- **Keep the notice inside the skill's own directory.** Then `include_dir!` embeds it in the binary, and every worktree that links the skill gets the notice with it. `skills/LICENSE` covers Matt Pocock's skills in the same way.
  - Cloudflare's notice cannot sit beside that file: the Prompts and skills page asserts that the only file outside a skill directory is `LICENSE` ([src/prompts_page.rs](../../src/prompts_page.rs), `licence_section`).
- **Credit it** in the README's "Credits and license" and on the Prompts and skills page.
- **Nothing more is required** for verbatim use, and changes are allowed.

**One edit is forced by thirdshift's own rule.**

- Factory skills are named `thirdshift-<skill>` in both directory and frontmatter, so that they cannot collide with a target repository's own skills ([ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)). The upstream frontmatter says `name: security-audit`.
- Either that line changes, or thirdshift accepts a collision with any repository that installed the same skill itself. It has 24.5K installs on skills.sh.

An open issue asks Cloudflare to relicense the skill as MIT-0, which would drop the notice requirement ([#10](https://github.com/cloudflare/security-audit-skill/issues/10), 12 August 2026). It has no reply.

## 3. Per-pull-request security review that keeps precision high

[security-agent-passes.md](security-agent-passes.md) §1.5 has the precision evidence for diff review and §3.1 the tools; this section adds what five primary sources scope to, exclude, verify, spend and do with large diffs.

| Source (grade) | Scope | Excludes | Verification | Budget per diff | Large diffs |
|---|---|---|---|---|---|
| **Google Chrome** ([blog](https://blog.google/security/chrome-stronger-with-every-update/), 30 July 2026; vendor blog) and **CodeMender** ([docs](https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender/find-modes), updated 6 October 2026; vendor docs) | Chrome runs Big Sleep and CodeMender "every 24 hours across all CLs". In its commit queue, models "automatically scan diffs" for specific classes. CodeMender's diff scan covers the changed hunks plus callers and callees: "Don't use a deep scan as a synchronous check that blocks pull requests" | CodeMender "isolates pre-existing vulnerabilities in untouched code so that legacy issues don't cause the scan to fail", labelling them `[LEGACY / UNTOUCHED]`; it gates on `--fail-on` `CRITICAL,HIGH`. Chromium's "Security for Agents" lists non-bugs "regardless of how easy they are to trigger", among them DoS and "missing layers of hardening" | Chrome's harness has "a "critic" agent with a separate context" that reads SECURITY.md, and runs models "multiple times to account for model non-determinism". Triage "checks for a proof of concept". Fixes pass a critic and test-writing agents before "a developer reviews the fix" | CodeMender: "typically finishing in under 2 minutes"; four workers by default. Chrome publishes none | CodeMender inspects at most 10 dependent files (`--diff-max-neighbors`); `--fail-on-truncation` fails the check when there are more |
| **Atlassian AgenticSCR** ([arXiv:2601.19138](https://arxiv.org/abs/2601.19138) v2, ASE 2026 industry track; peer-reviewed) | Commits before they are pushed: "a diff-centric strategy" that uses `git diff` "to focus on modified files and relevant line ranges, rather than ingesting the entire repository", while exploring repository context | None stated. The benchmark kept changes of at most 112 modified lines, in Python, JavaScript and TypeScript | A SAST-rule-guided detector, then a separate validator that maps each comment to a CWE entry and drops the rest, because "conflating both objectives within a single agent risks optimizing for pattern recognition at the expense of precision". Nothing is executed | Not reported (Claude Sonnet 4, on Atlassian's Rovo Dev CLI) | Not addressed |
| **Anthropic** (vendor docs and blog): the `/security-review` prompt, its GitHub Action, the security-guidance plugin, and Anthropic's own practice ([article](https://claude.com/resources/articles/how-anthropic-secures-its-ai-native-software-development-lifecycle), 21 July 2026) | The diff: "focus ONLY on security implications newly added by this PR". Anthropic's team uses "a CLAUDE.md file that instructs the agent to run /security-review as a final step before opening a PR", with the hard gate "at the test/CI stage" | 18 hard exclusions in the prompt (numbered to 17, with 16 used twice), plus the Action's regular-expression filter. That filter drops memory-safety findings in any file not ending `.c`, `.cc`, `.cpp` or `.h`. The plugin refutes "PRE-EXISTING" findings | A refuting sub-task per finding, told "You do not need to run commands to reproduce the vulnerability"; findings below 8 of 10 are dropped. At Anthropic, the share of pull requests with substantive review comments grew "from 16 to 54%" as the team "gained confidence in the findings by requiring the agents to write a proof that their finding is valid" | Action: one run per pull request by default, since re-runs "may lead to more false positives"; 20-minute subprocess timeout. Plugin: at most 30 files a turn, 18 agent turns | Action: on "Prompt is too long" it reruns without the diff ("PR diff was omitted due to size constraints"). Plugin: skips diffs over 300 files as "pathological", and sends the refuter the first 8,000 characters of the diff |
| **OpenAI Codex Security** (CLI and plugin at `07429cf`; [hosted review docs](https://learn.chatgpt.com/docs/security/security-review); vendor docs) | "Review every changed source file … without expanding into an unrelated repository audit". The hosted review reads "the pull request diff, supporting repository context, and configured threat models or security guidance". "Deep mode does not support diff scans" | Its file inventory skips tests, docs, examples, fixtures and vendored code. Hosted defaults: "automatic Codex Security Reviews report High and Critical findings, while manually requested reviews report Medium, High, and Critical" | Validation prefers reproduction, including "the smallest focused test"; the hosted service reproduces "in a clean container" | `--max-cost` "stops the scan and workers after estimated cost exceeds the limit"; report-only unless `--fail-on-severity` | "Diff scans do not rank or drop changed files before deep review"; "Divide large changes among available workers without overlap" |
| **Cloudflare's per-merge-request reviewer** ([blog](https://blog.cloudflare.com/ai-code-review/), 20 April 2026; vendor blog) and harness ([blog](https://blog.cloudflare.com/build-your-own-vulnerability-harness/), 18 June 2026) | Every merge request, re-reviewed on each push. A security sub-reviewer looks at "Authentication/authorisation bypasses in changed code". Files touching "auth/, crypto/, or file paths that sound even remotely security-related always trigger a full review". The audit harness is "a periodic backlog sweep and not a per-PR check … Cheaper, smaller harnesses are the right tool for that job" | "What NOT to Flag": "Theoretical risks that require unlikely preconditions", "Defense-in-depth suggestions when primary defenses are adequate", "Issues in unchanged code that this MR doesn't affect" | A coordinator on a stronger model drops "Speculative issues, nitpicks, false positives", and "If the coordinator isn't sure, it uses its tools to read the source code and verify". It blocks only on "Any critical item" | All reviewers together: $1.19 a review on average, median $0.98, P99 $4.45; median 3 min 39 s. Time limits: 5 minutes per task, 25 overall | "when the coordinator's prompt exceeds 50% of the estimated context window, we emit a warning" |

**What most of them do:**

- **Changed code plus bounded context.** None reviews the bare diff alone, and whole-repository scans are kept off the pull-request path: CodeMender, Cloudflare, Codex ("Deep mode does not support diff scans"), and Chrome ("Bulk scanning of a code base cannot keep pace").
- **Gate only on what the change introduced or affects.** Pre-existing issues are suppressed (Anthropic, Cloudflare), or reported without gating: CodeMender's `[LEGACY / UNTOUCHED]`, and Claude Code Review's "Pre-existing" severity ([code review](https://code.claude.com/docs/en/code-review)).
- **Explicit "do not report" lists** for low-signal classes, and a high bar: a confidence threshold or a High/Critical gate.
- **A refuting pass in a fresh context.**
  - At pull-request time it mostly reads code: Anthropic, AgenticSCR, Cloudflare's coordinator.
  - Execution-backed proof is the strongest lever. Anthropic says "Requiring that verifier to also build a proof of concept confirming the exploit brought the false positive rate to near zero" (vendor blog; no sample size). Codex attempts it inside the diff scan, and Chrome and CodeMender do it in separate stages.
- **The owner's `SECURITY.md` or threat model as context:** Chrome, Codex Security, Anthropic's harness.
- **Advisory first.**
  - GitHub's AI Scan "findings are advisory and do not block pull request merges" ([security-agent-passes.md](security-agent-passes.md) §4.1).
  - Codex is report-only by default.
  - Anthropic puts "Shadow mode for all new AI reviewers. New agents post comments for human approval until trust is earned".

**Where they disagree:**

- **Pre-existing issues:** suppressed (Anthropic, Cloudflare) or reported without gating (CodeMender, Claude Code Review).
- **How to verify:** by reading (the `/security-review` filter) or by reproduction (Codex).
- **Which classes to exclude.**
  - The 2025 `/security-review` prompt excludes "user-controlled content in AI system prompts" and memory safety "in rust". Codex Security's discovery skill says "Do not suppress SSRF because the fetch/callback is an intended feature".
  - The 2026 security-guidance plugin treats agent capability gates as a boundary: "the model is the attacker, the user is the victim".
- **Precision with denominators** is almost never published:
  - AgenticSCR: 17.5% of 211 comments fully correct; in shadow deployment, 22 of 41 comments judged worth developers' attention.
  - GitHub Security Lab's whole-repository funnel (§1.3).

**One caution about a reference implementation.** The Anthropic Action's verification may no longer run as documented [needs live check]:

- Its API filter first probes `claude-3-5-haiku-20241022` (`claudecode/claude_api_client.py` line 61). Anthropic retired that model on 19 February 2026: "Requests to retired models will fail" ([model deprecations](https://platform.claude.com/docs/en/about-claude/model-deprecations)).
- If the probe fails, the filter sets `use_claude_filtering = False` (`claudecode/findings_filter.py` lines 188–192).
- The Action's default model, `claude-opus-4-1-20250805`, was retired on 5 August 2026.

## 4. What the evidence does not tell us

- **How precise or complete any candidate is on code like the user's.**
  - The only third-party measurement of Cloudflare's skill is one seeded target, three runs, scored by an interested party (§2.2).
  - Snyk's VulnBench V2 covers Claude Code's `/security-review`, Codex Security and deepsec on 20 application fixtures, one run each, and Snyk sells a competing scanner.
  - Nothing measures Rust, PHP or Swift.
- **What a scoped diff run costs.** The $22–32 per `quick` run was a whole-target run with Claude Opus 5 on a purpose-built service of unstated size. A scoped run skips some hunters and verifiers but none of the four reconnaissance agents. Its cost on these repositories, with `claude-opus-5-5` or `gpt-6.1-sol`, is unmeasured **[needs live check]**.
- **Whether the skill runs under `codex exec`.** Nothing in its text is Claude-specific, but no run under Codex was found. Codex has no `research` or `general` role **[needs live check]**.
- **How often the cybersecurity safeguard will interrupt a Security review or pass.** The docs describe a fallback; this research saw one session end instead (§2.3).
- **Whether Cloudflare would accept an upstream change.** The maintainer has not merged a community pull request since 4 July, and has not commented on #58 or #60.
- **Whether a `quick` run finds what matters here.** The third-party run found the skill's recall equal to one plain session's, and named classes it missed. Whether that holds beyond one web service was not measured.
- **Whether a user-owned private repository can hold a draft advisory.** The docs scope repository advisories to public repositories. The user's plan, a private issue there, sidesteps the question.

## 5. Implications for thirdshift (recommendation, not evidence)

This section is my recommendation. It rests on the evidence above but goes beyond it.

1. **Keep cloudflare/security-audit-skill as the one upstream skill.**
   - Nothing else is MIT, self-contained, agent-neutral, language-agnostic and able to do both jobs (§1.5).
   - If per-Run cost turns out too high in the trial below, the least-bad second source for the diff review is Anthropic's Apache-2.0 security-guidance prompts. Using them would mean giving up "one skill".

2. **Ask for the run in words the skill cannot misread.** A Session prompt for the Security review would say, in effect:

   > Use the security-audit skill in full audit mode and write its report artifacts. Profile: `quick`. This is a scoped run over the diff between `<merge-base sha>` and `<head sha>`; record everything else as `out_of_scope`. Output directory: `<absolute path outside the worktree>`. There are no prior runs to read. Do not ask questions: where the skill says to ask or propose, take its documented `incomplete` or `deferred` outcome and finish.

   - **The security pass** would use the same wording with `standard`, no scope, and its own output root, whose earlier runs it *should* read: the skill's additive runs are made for a recurring pass.
   - **Each Run's review** gets its own output directory, so that it does not inherit the pass's open leads (§2.1).
   - **Claude Code** needs that directory passed with `--add-dir`.

3. **Settle the sandbox question before building, because it decides what the skill can report.** In my order of preference:
   - **Give the VM a sandbox the skill accepts.** On a dedicated VM where the user has root, one admin step makes the rule satisfiable with no change to the skill: an AppArmor profile that lets bubblewrap create user namespaces, or Podman or Docker (PR #60's route).
     - Then the skill runs verbatim, and `confirmed` records carry a severity.
     - Proof-of-concept code also runs away from the VM's GitHub, model and Resend credentials. A disposable VM limits what a bad test can break; it does not stop a test from reading those tokens.
   - **Keep the skill verbatim and put execution in thirdshift's process.**
     - The skill returns `needs_validation` records with exact `validation_plan.local` steps (§2.4, shape 3).
     - The Security review session then writes each plan as a failing test in the worktree, runs it as it runs every Run's tests, and fixes only what fails.
     - This matches the user's plan exactly, but the skill never assigns a severity, so draft advisories carry none until someone sets one.
   - **Propose the opt-in policy upstream** (§2.4, shape 2), citing CodeMender's precedent. But do not depend on it.

4. **Use the failing test to separate "introduced by this diff" from "already on the Base branch".**
   - The skill's scoped run audits in-scope surfaces, not only added lines.
   - A proof-of-concept test that also fails at the merge base shows a vulnerability the Base branch already has. Fixing that in the Run's public pull request would disclose it ([security-agent-passes.md](security-agent-passes.md) §7.4). It belongs in a draft advisory or a private issue, like the pass's findings.
   - A test that passes at the merge base and fails at the head was introduced by the Run, and the Run can fix it in the open.
   - This is the same distinction CodeMender draws with `[LEGACY / UNTOUCHED]`.

5. **Feed the skill the owner's threat model.** The skill does not look for one (§2.6). thirdshift's prompt can name `SECURITY.md` or a threat-model file, where a repository has one, as context for reconnaissance. That adds process, not text, to the skill.

6. **Write advisories by OpenAI's rules** (§2.5):
   - one draft per finding;
   - `severity` from `overall_severity`, and none for `needs_validation` or `informational`;
   - `cwe_ids` only when certain;
   - the package from the manifest;
   - no version range claimed from a single commit;
   - the fingerprint and commit in the description;
   - duplicates matched across all four states before creating.

7. **Embed it as one directory with its own licence** (§2.7):
   - `skills/thirdshift-security-audit/` holds the upstream files byte for byte, except the frontmatter `name`, with Cloudflare's `LICENSE` beside them;
   - the pinned upstream commit is recorded;
   - a test fails if any other byte drifts from that commit.

8. **Treat the Security review as an opt-in cost, and trial it first.**
   - Every diff review spends at least seven agent invocations, four of them whole-repository reconnaissance (§2.2). The one measurement put a `quick` run at about ten implement sessions of list-price spend.
   - Cloudflare's own tiering suggests a cheaper trigger: run the review only when the diff touches security-sensitive paths, or always for a repository the user marks.
   - Run it in report-only mode on a few repositories first. Read the Command logs' costs and grade the findings before turning it on widely, as [security-agent-passes.md](security-agent-passes.md) §9 option 5 recommends.

9. **Expect model changes and refusals on security content.**
   - Under Claude Code, Opus 5.5 sessions that trip the cybersecurity classifier continue on Opus 4.8, or, as seen here, end (§2.3).
   - thirdshift should log the models a security session actually used, and treat a refusal as a failed review, not a clean one ([security-agent-passes.md](security-agent-passes.md) §7.5).
   - Anthropic's error page points to a Cyber Verification Program "to reduce these interruptions" ([errors](https://code.claude.com/docs/en/errors)).

## Sources

All accessed 7 October 2026. Grades as in the table above. Repositories were read at the commits given, by me unless marked "(background agent)".

**cloudflare/security-audit-skill (vendor docs)**

- Repository, MIT, at `c1c8a8c` (14 September 2026): https://github.com/cloudflare/security-audit-skill
  - Read: `README.md`, `LICENSE`, and in `skills/security-audit/`: `SKILL.md`, `RECONNAISSANCE.md`, `HUNTING.md`, `VALIDATION-AND-REPORTING.md`, `ATTACK-CLASSES.md`, `AI-AND-LLM.md`, `MEMORY-SAFETY-AND-BINARY.md`, `report-schema.json`, `validate-findings.cjs`, `validate-coverage-ledger.cjs`.
- Pull requests [#13](https://github.com/cloudflare/security-audit-skill/pull/13), [#58](https://github.com/cloudflare/security-audit-skill/pull/58) and [#60](https://github.com/cloudflare/security-audit-skill/pull/60) (bodies and diffs); issues [#10](https://github.com/cloudflare/security-audit-skill/issues/10), [#11](https://github.com/cloudflare/security-audit-skill/issues/11) and [#20](https://github.com/cloudflare/security-audit-skill/issues/20), with comments.
- skills.sh page: https://skills.sh/cloudflare/security-audit-skill/security-audit (registry data).
- TheColliery, "CoalBoard audit mode vs cloudflare/security-audit-skill vs a solo control — blind comparison" (16 September 2026, with addendum; third-party report, interested party): https://github.com/TheColliery/.github/blob/main/benchmarks/CoalBoard/SECURITY-AUDIT-2026-09-16.md and `results/security-audit-2026-09-16/orchestrate.log`.

**Cloudflare blog (vendor blog)**

- "Orchestrating AI Code Review at scale" (20 April 2026): https://blog.cloudflare.com/ai-code-review/
- "Build your own vulnerability harness" (18 June 2026): https://blog.cloudflare.com/build-your-own-vulnerability-harness/

**Other candidates (vendor docs unless stated)**

- openai/codex-security, Apache-2.0, at `07429cf` (7 October 2026): https://github.com/openai/codex-security
  - Read: `plugins/codex-security/.codex-plugin/plugin.json`, `.mcp.json`, `skills/security-diff-scan/SKILL.md`, `skills/validation/SKILL.md`, `skills/track-findings/references/github-security-advisories.md`, `references/config-preflight.md`, `references/core-scan.md`, `scripts/generate_rank_input.py`, `sdk/typescript/docs/cli.md`, `sdk/typescript/README.md`, `sdk/typescript/package.json`, `github-action/README.md`.
  - Hosted docs: [Security Review](https://learn.chatgpt.com/docs/security/security-review).
- anthropics/claude-plugins-official at `d4226d0`, `plugins/security-guidance/` (v2.0.10): `hooks/review_api.py`, `hooks/llm.py`, `hooks/security_reminder_hook.py`, `LICENSE`: https://github.com/anthropics/claude-plugins-official
- anthropics/claude-code-security-review, MIT, at `0c6a49f` (11 February 2026): https://github.com/anthropics/claude-code-security-review
  - Read: `.claude/commands/security-review.md`, `action.yml`, `claudecode/github_action_audit.py`, `claudecode/findings_filter.py`, `claudecode/claude_api_client.py`, `claudecode/prompts.py`, `claudecode/constants.py`.
- anthropics/defending-code-reference-harness at `d3bea6b` (6 August 2026): `README.md`, `LICENSE`, `.claude/skills/*/SKILL.md`, `docs/blog-post.md`: https://github.com/anthropics/defending-code-reference-harness. Licence diff against apache.org: background agent.
- alpha-omega-security/scrutineer, MIT, at `cce10ee` (7 October 2026): `README.md`, `docs/diff-based-rescans.md`, `skills/*/SKILL.md`: https://github.com/alpha-omega-security/scrutineer
- google/mantis, Apache-2.0, at `2b3bbdc` (6 October 2026): `README.md`, `README_AGENTS.md`, `reference/evals/README.md`: https://github.com/google/mantis
- getsentry/skills at `d18b7aa` (29 September 2026): `skills/security-review/SKILL.md`, `skills/security-review/LICENSE`, `warden.toml`; issue [#165](https://github.com/getsentry/skills/issues/165): https://github.com/getsentry/skills
- getsentry/warden (FSL-1.1-ALv2): `README.md`, `packages/docs/src/content/docs/benchmarking.mdx` at `651f855` (22 September 2026): https://github.com/getsentry/warden
- trailofbits/skills, CC-BY-SA-4.0, at `82fe822` (28 September 2026): `LICENSE`, `README.md`, `AGENTS.md`, `plugins/{differential-review,c-review,rust-review,insecure-defaults}/`; pull requests [#224](https://github.com/trailofbits/skills/pull/224), [#228](https://github.com/trailofbits/skills/pull/228), [#257](https://github.com/trailofbits/skills/pull/257): https://github.com/trailofbits/skills
- gemini-cli-extensions/security, Apache-2.0, at `2227f3c` (28 April 2026): `README.md`, `gemini-extension.json`, `commands/security/*.toml`, `skills/poc/SKILL.md`: https://github.com/gemini-cli-extensions/security
  - Google Developers Blog, "An important update: Transitioning Gemini CLI to Antigravity CLI": https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/
- openai/skills at `49f948f` (24 June 2026): `README.md`, `skills/.curated/security-best-practices/`, `security-threat-model/`, `security-ownership-map/`: https://github.com/openai/skills
- vercel-labs/deepsec, Apache-2.0, at `4fa6722` (29 September 2026): `README.md`, `NOTICE`, `SKILL.md`, `docs/reviewing-changes.md`: https://github.com/vercel-labs/deepsec
- microsoft/hve-core: `.github/agents/security/security-reviewer.agent.md`, `.github/skills/security/owasp-top-10/SKILL.md`, `docs/reference/agents/security/security-reviewer.md`: https://github.com/microsoft/hve-core
- OWASP/secure-agent-playbook: `README.md`, `LICENSE.md`, `THIRD_PARTY_NOTICES.md`: https://github.com/OWASP/secure-agent-playbook. Also [OWASP/www-project-agentic-skills-top-10](https://github.com/OWASP/www-project-agentic-skills-top-10) (`README.md`) and [OWASP/AISVS](https://github.com/OWASP/AISVS) (licence).
- github/awesome-copilot at `3a68501` (6 October 2026): `skills/security-review/SKILL.md`: https://github.com/github/awesome-copilot
- aws/agent-toolkit-for-aws: `plugins/aws-agents-for-devsecops/skills/diff-scanning-with-aws-security-agent/SKILL.md`: https://github.com/aws/agent-toolkit-for-aws
- semgrep/skills, licence "Semgrep Rules License v1.0": https://github.com/semgrep/skills
- ghostsecurity/skills at `25fdf06` (28 September 2026), `README.md`: https://github.com/ghostsecurity/skills
- GitHub blog, "How to scan for vulnerabilities with GitHub Security Lab's open source AI-powered framework" (6 March 2026; vendor blog): https://github.blog/security/how-to-scan-for-vulnerabilities-with-github-security-labs-open-source-ai-powered-framework/
- Snyk VulnBench V2 (vendor research): `docs/vulnbench-v2-run-handoff.md` and `publications/vulnbench-v2/` at `a8718aa` (1 October 2026): https://github.com/snyk-labs/snyk-vulnbench
- Registries and lists (registry data): skills.sh skill pages and `https://skills.sh/api/search`; [VoltAgent/awesome-agent-skills](https://github.com/VoltAgent/awesome-agent-skills), [travisvn/awesome-claude-skills](https://github.com/travisvn/awesome-claude-skills), [ComposioHQ/awesome-claude-skills](https://github.com/ComposioHQ/awesome-claude-skills), [hesreallyhim/awesome-claude-code](https://github.com/hesreallyhim/awesome-claude-code); `gh api search/repositories` queries; [google/skills](https://github.com/google/skills), [microsoft/skills](https://github.com/microsoft/skills).

**Per-pull-request practice**

- Google, "Stronger with every update: How we're making Chrome and the web safer in the AI Era" (30 July 2026; vendor blog): https://blog.google/security/chrome-stronger-with-every-update/
- CodeMender docs (updated 6 October 2026; vendor docs): [find modes](https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender/find-modes), [integrate with CI/CD](https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender/integrate-with-cicd), [scan and verify](https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender/scan-and-verify)
- Chromium (vendor docs): `docs/security/ai-generated-security-bugs-faq.md` at `956e061` (3 September 2026) and `docs/security/security-for-agents.md` at `b41ba61` (29 September 2026), via the GitHub mirror https://github.com/chromium/chromium
- Charoenwet et al., "AgenticSCR: An Autonomous Agentic Secure Code Review for Immature Vulnerabilities Detection", [arXiv:2601.19138](https://arxiv.org/abs/2601.19138) v2 (3 August 2026), ASE 2026 industry track (DOI 10.1145/3832783.3834523; peer-reviewed)
- Anthropic, "How Anthropic secures its AI-native software development lifecycle" (21 July 2026; vendor blog): https://claude.com/resources/articles/how-anthropic-secures-its-ai-native-software-development-lifecycle
- Claude Code docs, [code review](https://code.claude.com/docs/en/code-review) (vendor docs)
- Claude API docs, [model deprecations](https://platform.claude.com/docs/en/about-claude/model-deprecations) (vendor docs)

**GitHub (vendor docs)**

- REST: [create a repository security advisory](https://docs.github.com/en/rest/security-advisories/repository-advisories#create-a-repository-security-advisory) and [list repository security advisories](https://docs.github.com/en/rest/security-advisories/repository-advisories#list-repository-security-advisories) (API version 2026-03-10)
- [About repository security advisories](https://docs.github.com/en/code-security/concepts/vulnerability-reporting-and-management/repository-security-advisories)

**Harnesses (vendor docs, source, local)**

- Claude Code docs: [sub-agents](https://code.claude.com/docs/en/sub-agents), [model configuration](https://code.claude.com/docs/en/model-config) ("Automatic model fallback"), [errors](https://code.claude.com/docs/en/errors) ("Safety measures flagged a cybersecurity topic"); `claude --help` (2.1.292, local)
- Claude API docs: [refusals and fallback](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback)
- Codex: [`codex-rs/core/src/agent/role.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/agent/role.rs) at `7ac954e` (6 October 2026); `codex --help`, `codex exec --help` (0.160.1, local)
- This machine: `node --version`, `sysctl kernel.apparmor_restrict_unprivileged_userns`; the background agent's stop message of 7 October 2026 (local)

**thirdshift itself**

- [CONTEXT.md](../../CONTEXT.md), [security-agent-passes.md](security-agent-passes.md), [codex-headless-harness.md](codex-headless-harness.md)
- [ADR 0001](../adr/0001-rust-binary-with-embedded-skills.md), [ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md), [src/skills.rs](../../src/skills.rs), [src/prompts_page.rs](../../src/prompts_page.rs), [skills/LICENSE](../../skills/LICENSE), [README.md](../../README.md) ("Credits and license")
- Issue [#427](https://github.com/JacobStephens2/thirdshift/issues/427)
