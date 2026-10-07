# Security agent passes: how well LLM security audits work, and how a factory could run them

Research date: 2026-10-06.

**Question.** Nothing in a Run looks for vulnerabilities today ([#427](https://github.com/JacobStephens2/thirdshift/issues/427)):

- The implement session's review has two axes, Standards and Spec ([skills/thirdshift-code-review/SKILL.md](../../skills/thirdshift-code-review/SKILL.md)).
- Under this machine's User config every Run is a Merge run (`merge.always = true`), so a vulnerability an agent writes merges with no human review.

Issue #427 proposes two shapes, not exclusive:

1. **A security pass**, shaped like an Architect run, whose confirmed findings become work.
2. **A Security axis** in each Run's code review.

It names two projects to study, cloudflare/security-audit-skill and alibaba/open-code-review. This note covers:

1. How precise and how complete LLM security audits are, for whole codebases and for diffs, and what moves them. It also covers how often coding agents introduce vulnerabilities, and how often automated security fixes are correct (§1).
2. The two projects in #427, and what adapting each as a Factory skill would take (§2).
3. The alternatives (§3).
4. The cheap deterministic complements, and how they would meet a factory whose Runs fix their own red checks (§4).
5. How others disclose what they find, and what GitHub's advisory API allows unattended (§5).
6. Cost, against a local baseline from thirdshift's own logs (§6).
7. The risks specific to an unattended factory doing security work (§7).

The user's repositories:

- **Public:** thirdshift (Rust CLI), cascade (Rust core with native shells), keeplore (PHP/MySQL, live at keeplore.app), muxboard (Python Flask with live in-browser tmux attach), vaulted-agent (Rust; launches agents with vault-resolved secrets).
- **Private, user-owned:** chart35 (TypeScript PWA with end-to-end encrypted health data) and clave (TypeScript game).
- **The machine:** 4 cores and 8 GB RAM, which allows about two building Runs at once.

**Method.**

- **The two projects.** Both repositories were cloned and read in full: all 22 files of cloudflare/security-audit-skill, and the README, skills, docs, rules and prompt templates of alibaba/open-code-review. Cloudflare's three posts on its harness were read from the pages themselves. Nothing from either repository was executed.
- **Delegated reading, spot-checked.** Four background agents read:
  - the detection and auditing literature;
  - the literature on agent-introduced vulnerabilities, fix correctness and attacks on AI reviewers;
  - the alternatives and vendor disclosure policies;
  - GitHub's documentation and the per-ecosystem tools.

  I re-checked against the source every number this note leans on hardest, including: the Cloudflare and Anthropic figures; the open-code-review benchmark image; PatchBench Table 2; the AIxCC SoK's §7.4; the DARPA results page and its correction; the SusVibes paper and leaderboard JSON; the BaxBench leaderboard JSON; the vibe-coded-apps replay table; AFCRA's Table 4; the project-scale detection study's Table VII; the `/security-review` prompt file; the Codex Security CLI docs; and every GitHub quotation in §4 and §5. Numbers read only by a background agent are marked so.
- **openai.com** refused every fetch, so OpenAI's posts and outbound disclosure policy were read from Wayback Machine copies (dated in the Sources).
- **Local facts.** These were checked on this machine on 6 October 2026:
  - `claude` 2.1.291 and `codex` 0.160.0, from `--help` and `codex features list`.
  - Read-only `gh api` GET calls on the seven repositories' security settings and workflows.
  - The Command logs and Session logs in `~/.thirdshift/logs`.
  - Whether unprivileged network namespaces work here (`unshare`).
- **Nothing was started.** No agent session was started, and no GitHub setting, issue, advisory or upload was made.
- **Search window.** Literature and vendor material were searched through 6 October 2026. The background agents' recency sweeps covered 1 July to 6 October 2026 (arXiv listings, and the USENIX Security 2026, CCS 2026, IEEE S&P, ASE 2026 and ICML 2026 programmes) and the GitHub changelog for the same months.
- **Section references.** A § number inside a paper's citation points into that paper. A bare (§N) points into this note.

**Evidence grades used below.**

| Grade | Meaning |
|---|---|
| Peer-reviewed | Journal or main conference |
| Workshop | Accepted, light review |
| Preprint | Not reviewed |
| Leaderboard | Run by a benchmark's authors, not reviewed |
| Vendor docs | Official documentation, repositories and policies |
| Vendor blog/research | Official posts, including red-team reports |
| Anecdote | One maintainer's or one team's experience |

**Out of scope.** Choosing a design (the last section only lays out options), and anything only a live run could show, flagged **[needs live check]**.

## TL;DR

- **Raw LLM security findings are mostly wrong; independently verified ones are mostly right.** (§1.1–1.2)
  - **Unverified.**
    - Out of the box, Claude Code (Sonnet 4) was right on 14% of its findings and Codex (o4-mini) on 18%, across 11 Python web apps (Semgrep, 2025; vendor research).
    - On real C/C++ and Java projects, Claude Code (Sonnet 4.6) had sampled false-discovery rates of 74–82%, and Codex (GPT-5.4) of 9–32% (preprint, September 2026).
    - GPT-4 called the patched twin of a vulnerable function vulnerable too in 71.63% of pairs (PrimeVul, ICSE 2025).
  - **With a check the agent cannot game.**
    - RepoAudit's validator reached 78.43% precision.
    - Deterministic browser replay took Claude Code (Opus 4.6) from 37% to 100% on XSS.
    - Sanitizer-validated Firefox bugs were 112 of 112 real, though only 22 were vulnerabilities.
    - External firms judged 90.6% of 1,752 Mythos Preview high/critical findings valid, and 92.7% of 6,123 across Anthropic's ledger.
  - **A check the agent can game can be worse than self-judgment.** A browser verdict the agent could influence dropped precision from 45% to 9%, and Cloudflare's hunters edited the code "so its own exploit works".
- **Recall is low for cold discovery, and unknown in the field.** (§1.3)
  - The best CyberGym agent reproduced 17.9% of known vulnerabilities, and GPT-5 went from 7.7% to 22.0% with high reasoning.
  - One run finds about half (Cloudflare) to two-thirds (HoF-Bench) of what repeated runs find, and identical runs gave 3, 6 and 11 findings.
  - Different models overlap little, so a union of models beats repetition.
- **Pull-request security review is weakly measured and easy to attack.** (§1.5, §7.1)
  - Atlassian's agentic reviewer made fully correct comments 17.5% of the time; in a shadow deployment, its security engineers validated 54% of its comments.
  - Planted malicious changes: Claude Code (Sonnet 4.6) caught 79% and open-code-review 42–56%; once the change was spread among 24 pull requests, the reviewers caught 16–22%.
  - Adaptive attacks got vulnerable code past Claude Code and Codex reviewers in 16–21% of cases (AFCRA, October 2026), and past Claude Code and CodeRabbit in 97% (another preprint).
- **Coding agents write insecure code often, and a security skill helps where "review your changes" does not.** (§1.7)
  - On SusVibes (August 2026 leaderboard), Claude Code with Opus 4.8 and Codex with GPT-5.5 leave about 60% of their functionally correct solutions insecure.
  - Replaying 70 real vulnerabilities, a security-hardening skill cut reintroduction by 13.8 points, "review your changes" by 1.9, and a stronger model by none.
- **Security fixes that pass the proof of concept are often wrong.** (§1.8)
  - PatchBench: 83.1% of patches stopped the PoC crash, but only 45.3% were correct (Codex with GPT-5.6 Sol 59.2%; Claude Code with Opus 4.8 56.8%).
  - 37.7% of Claude Code's fully validated AIxCC patches were semantically wrong (USENIX Security 2026).
  - Cloudflare watched self-written patches "quietly" break other code.
- **cloudflare/security-audit-skill** (MIT) is a six-phase whole-codebase audit. (§2.1–2.2)
  - **Shape.** Reconnaissance, coverage-led hunters, refuting verifiers, schema-checked `confirmed`, `needs_validation` or `rejected` findings, a second verification, then reports.
  - **Requirements.** Parallel sub-agents, Node.js, and an OS sandbox before anything can be `confirmed`. About 183 KB of Markdown, 3.3 times all of thirdshift's Factory skills together.
  - **Headless.** It asks a human in three places, which a Session prompt can answer in advance.
  - **On this machine it could confirm nothing**: unprivileged namespaces are blocked, so every lead would stay `needs_validation`.
  - Cloudflare itself outgrew it: "a single run finds only about half the bugs".
- **alibaba/open-code-review** (Apache-2.0) is a Go diff-review CLI for general review; security is one of eight categories. (§2.3)
  - **Pipeline.** Deterministic file selection and per-language rules, then a model loop and a model filter.
  - **Benchmark** (Alibaba's own, general review, not security): 32–38% precision against Claude Code's 7–16%, at lower recall and about a ninth of the tokens.
  - **Model access.** It needs its own model key, billed at API prices, unless run in delegation mode, which keeps only file selection and rules.
  - **Coverage.** No C# rule, and Kotlin's has no security section.
  - **Headless.** Its CLI is headless; its skill text asks the user four times.
- **The most useful alternatives** (§3):
  - Claude Code's built-in `/security-review`. Its prompt is public under MIT; it reviews the diff, with a refuting sub-agent per finding. Headless use is untested, and its exclusions hide prompt-injection and unsafe-Rust bugs.
  - OpenAI's Codex Security CLI (Apache-2.0, local): `scan --diff`, validation, `--fail-on-severity`, on a ChatGPT login or an API key.
  - Not fitting:
    - Claude Security: proprietary, and refuses unattended scans.
    - Trail of Bits' skills: CC-BY-SA.
    - Semgrep's rules: no redistribution.
    - The AIxCC systems: C and Java with OSS-Fuzz harnesses, 8+ cores.
  - Matt Pocock's skills have nothing on security.
- **GitHub's free features fit the public repositories only, and almost all are off today.** (§4)
  - Public repositories get CodeQL (Rust yes, PHP no), Copilot Autofix, secret scanning, push protection, private vulnerability reporting, dependency review and SARIF upload.
  - User-owned private repositories get only Dependabot.
  - No CI workflow in the seven repositories runs a security tool.
- **Two factory interactions need care.** (§4.3)
  - **CodeQL.** Its failing pull-request check explains itself in check-run annotations, not in the Actions log the CI-fix Repair is told to read.
  - **Per-pull-request dependency audits.** A new advisory turns every Run red at once, and thirdshift will not see it as an Inherited failure until the base branch re-runs the check.
- **No unattended private fix path exists on GitHub.** (§5)
  - **What the API can do:** create a draft advisory and a temporary private fork, publish, and request a CVE (public repositories only).
  - **What it cannot do:** no CI runs in the fork, and its merge is a web-UI button.
  - **SARIF.** Uploaded against the default branch, alerts are visible only to people with write access. On a pull request, they are public annotations.
  - **The norm** is to fix privately and publish an advisory with the release. Anthropic and OpenAI say a human reviews every AI-found report before it goes out.
  - **A public fix is a roadmap.** From Firefox security diffs, Mythos Preview built a working exploit in under an hour.
- **Cost.** (§6)
  - **Local baseline** (Claude Code, `claude-opus-5-5` at medium, list price, 3–5 October 2026): an Architecture review's median is $2.84 and 6.7 minutes; an implement session's is $2.78 and 16.1 minutes.
  - **Hosted diff reviewers** cost $5–25 a review, billed outside the plan.
  - **Whole-codebase audits** run to hours per repository (Cloudflare), or hundreds of runs at tens of dollars each per large target (Anthropic).

## Recency sweep (1 July – 6 October 2026)

Most of the field changed in 2026, much of it in these three months. What is new in the window, and used in this note:

- **Tools.**
  - cloudflare/security-audit-skill was reworked end to end (10 September) and its modes clarified (14 September).
  - OpenAI open-sourced the Codex Security CLI (13 July). Its cloud scans became token-billed (1 October).
  - Anthropic's proprietary Claude Security plugin appeared (first commit 22 July).
  - open-code-review shipped v1.12.12 (5 October).
- **GitHub.**
  - Agentic autofix (10 July) and AI Scan for languages CodeQL lacks, PHP among them (14 July), both in preview and both needing paid licences.
  - CodeQL 2.26.0 added a JavaScript/TypeScript query for system-prompt injection (10 July). CodeQL 2.26.4 relocated Rust alerts (September).
  - Private vulnerability reports gained structured forms and per-user rate limits (1 October).
- **Papers.**
  - Adaptive attacks on AI pull-request reviewers: AFCRA (4 October) and ALIBI (27 July).
  - Malicious issues against coding agents: IssueTrojanBench (22 July).
  - Malicious changes hidden among benign pull requests: PRWeaver (August).
  - Reward-hacking-resistant verification: RECEIPT (20 July).
  - Off-stack patch correctness: PatchBench (3 September).
  - Multi-pass rediscovery: HoF-Bench (July).
  - The project-scale detection study's second version (25 September).
  - The SusVibes leaderboard rows for Claude Code with Opus 4.8 and Codex with GPT-5.5 (21 August).
  - The AIxCC SoK at USENIX Security 2026 (August) and SusVibes at ICML 2026.
- **Vendors and field.**
  - Anthropic's disclosure ledger (counts as of 2 October).
  - Google running Big Sleep and CodeMender in Chrome's CI (30 July).
  - The OSS-Fuzz pilot of CodeMender auto-patches (29 July).
- **Checked but not used.** The background agents screened about 500 arXiv titles from July to October 2026 and the 2026 programmes of USENIX Security, IEEE S&P, CCS, ASE, ISSTA, FSE and ICML. CCS 2026, NDSS 2027 and ICSE 2027 had no accepted-paper lists or full texts yet.

## 1. How precise and how complete LLM security audits are

### 1.1 Unverified findings are mostly wrong

| Setting | Result | Source (grade) |
|---|---|---|
| Single functions, vulnerable vs patched twin | GPT-4 with chain-of-thought labelled both functions of a pair correctly in 12.94% of 564 pairs, against 22.70% for random guessing. Two-shot GPT-4 called both the vulnerable and the patched version vulnerable in 71.63% | PrimeVul, [arXiv:2403.18624](https://arxiv.org/abs/2403.18624), Table VIII (ICSE 2025; peer-reviewed) |
| Same, on just-in-time pairs | Plain GPT-4o: F1 65.96 but pairwise accuracy 1.02%; the best agent reached 20.17% | JitVul, [arXiv:2503.03586](https://arxiv.org/abs/2503.03586), Table 2 (ACL 2025; peer-reviewed) |
| Single functions, 300 sampled | Claude 3.7 Sonnet: precision 41.86%, recall 75.63% at function level; statement level 15.35% precision | SecVulEval, [arXiv:2505.19828](https://arxiv.org/abs/2505.19828), Table 3 (preprint) |
| Whole Python web apps, out of the box | Claude Code (Sonnet 4): 14% of findings true; Codex (o4-mini): 18%; 445 findings triaged by hand | [Semgrep](https://semgrep.dev/blog/2025/finding-vulnerabilities-in-modern-web-apps-using-claude-code-and-openai-codex/), September 2025 (vendor research) |
| Whole C/C++ and Java projects | Sampled false-discovery rate: Claude Code (Sonnet 4.6) 74.07% (C/C++) and 81.82% (Java); Codex (GPT-5.4) 9.09% and 31.82%; CodeQL 86.01% (C/C++) | [arXiv:2601.19239](https://arxiv.org/abs/2601.19239) v2, Tables VII and IX (preprint; warnings labelled with Codex's help) |
| XSS hunting, agent judging its own findings | Claude Code with Claude Opus 4.6 reported 27 findings; replayed in a real browser, 10 were true (37%) | RECEIPT, [arXiv:2607.18575](https://arxiv.org/abs/2607.18575) (preprint, July 2026) |

Two patterns recur:

- **Models flag fixed code as still vulnerable.** SecLLMHolmes: "all models analyzed have a high false positive rate (FPR), and flag code where vulnerabilities have been patched as still vulnerable" ([arXiv:2312.12575](https://arxiv.org/abs/2312.12575), IEEE S&P 2024; peer-reviewed; read by a background agent).
- **Agent and model choice swing the false-discovery rate eightfold.** In the project-scale study, Codex reported 2.44 warnings per C/C++ project to Claude Code's 6.00, with a sampled false-discovery rate of 9% against 74%. Claude Code "tends to report suspicious code locations more aggressively, often before fully validating the related data structure or feasible data-flow path".

### 1.2 What moves precision: an independent check the agent cannot game

| Lever | Effect | Source (grade) |
|---|---|---|
| A validator that re-checks data flow and path conditions | RepoAudit (Claude 3.5 Sonnet) found 40 true bugs with 11 false positives (precision 78.43%) across 15 projects, at $2.54 and 0.44 hours per project. "Without validators, the number of FPs increases by 245.45%" | [arXiv:2501.18160](https://arxiv.org/abs/2501.18160) (ICML 2025; peer-reviewed) |
| Deterministic replay in an isolated browser, with PoC constraints and role separation | Precision went from 45% (self-judgment) to 9% (a browser verdict the agent could manipulate), then 19%, 44% and finally 100% as each defence was added; 30 of 30 on the full benchmark | RECEIPT, §RQ3 |
| A sanitizer as the oracle | Opus 4.6 sent Firefox 112 bugs and "every single one was confirmed to be a true positive", but only 22 were vulnerabilities (14 high). On FFmpeg, "because we have a perfect crash oracle in ASan, we have not yet encountered a false positive" | [red.anthropic.com](https://red.anthropic.com/2026/mythos-preview/), [anthropic.com](https://www.anthropic.com/news/mozilla-firefox-security) (vendor research) |
| A multi-stage pipeline plus human triage | Of 1,752 Mythos Preview findings it rated high or critical, "assessed by one of six independent security research firms" (or in a few cases by Anthropic), 90.6% were valid and 62.4% confirmed high or critical. Across the whole disclosure ledger, 92.7% of 6,123 externally reviewed findings were valid | [Glasswing update](https://www.anthropic.com/research/glasswing-initial-update), 22 May 2026; [CVD ledger](https://red.anthropic.com/2026/cvd/), 2 October 2026 (vendor research) |
| Better context from reconnaissance | Cloudflare's validation rejection rate fell from 40% to 11%; 12,057 of 20,799 raw candidates (58%) survived validation | Cloudflare blog, June 2026 (vendor blog) |
| An agent that triages a static analyser's alarms | False-positive rate on the OWASP Benchmark "from over 92% … to as low as 6.3%", but "suppressing true vulnerabilities" | Sifting the Noise, [arXiv:2601.22952](https://arxiv.org/abs/2601.22952) (ISSTA 2026; peer-reviewed; abstract) |

The verifier must be something the agent cannot satisfy by gaming:

- RECEIPT's 9% step and Cloudflare's agents that "edit the source code so its own exploit works" are the same failure.
- Verification also costs recall. Sifting the Noise warns of exactly that.
- Logic bugs have no crash oracle. Anthropic: "we too lose the ability to (near-)perfectly validate".

### 1.3 Recall

- **Cold discovery is low.**
  - CyberGym (1,507 real vulnerabilities, success only if a PoC crashes the pre-patch build and not the patched one): the best agent without extended thinking reached 17.9% (OpenHands + Claude Sonnet 4). Turning on reasoning took GPT-5 "from a 7.7% to a 22.0% success rate" ([arXiv:2506.02548](https://arxiv.org/abs/2506.02548), ICLR 2026; peer-reviewed).
  - BountyBench's Detect task: best 12.5% (Codex CLI with o3-high) ([arXiv:2505.15216](https://arxiv.org/abs/2505.15216), NeurIPS 2025; peer-reviewed).
- **Hunting a named weakness class is higher.** On known Java vulnerabilities, Codex found 63.87% (99/155) and Claude Code 81.94% (127/155), against 6.67–14.84% for CodeQL, Semgrep and SpotBugs ([arXiv:2601.19239](https://arxiv.org/abs/2601.19239)).
- **One run is not enough.**
  - Cloudflare: one run finds "about half" of what repeated runs find.
  - HoF-Bench: "a single pass recovers 68% of the CVEs demonstrated over four passes" ([arXiv:2607.27030](https://arxiv.org/abs/2607.27030), preprint by the vendor AISLE).
  - Semgrep: three identical runs gave 3, 6 and 11 findings.
- **Diversity beats repetition.** CyberGym's union of all models reached 27.2% against the best single 17.9%, "revealing the low overlap" between models. Four agents on one model reached 18.4%, "nearly doubling the best individual result".
- **Vendor recall claims are on their own benchmarks.** Aardvark found "92% of known and synthetically-introduced vulnerabilities" on OpenAI's "golden" repositories (vendor blog, October 2025).

### 1.4 Model strength, effort, tools and family

- **Strength and effort raise discovery.** CyberGym's GPT-5 effort result above. RepoAudit's reasoning-model variants reached 86.8–88.5% precision against 78.4% for Claude 3.5 Sonnet (Appendix D; read by a background agent).
- **Tools help when they give the model facts.** IRIS's LLM-inferred taint specifications with CodeQL found 55 of 120 Java vulnerabilities against CodeQL's 27 ([arXiv:2405.17238](https://arxiv.org/abs/2405.17238), ICLR 2025; read by a background agent). Cloudflare's sandbox was "the biggest jump in quality". But its hunters invoked a wired-in Semgrep "zero times in a month of runs".
- **Family.** Cloudflare deliberately validates with a different model from the one that discovers, so the models are "effectively double-checking each other", but publishes no number for it. No study found compares cross-family with same-family verification of security findings. [multi-family-review.md](multi-family-review.md) reviews the general evidence on that question.

### 1.5 Diff (pull request) security review

- **Ordinary reviews of security defects.** On 534 files with reviewer-confirmed security defects (OpenStack, Qt), the best model, DeepSeek-R1, named the right type and location in at most 9.61% of responses. Across its prompts, 25–40% of its responses were misleading ([arXiv:2401.16310](https://arxiv.org/abs/2401.16310), Table 6; preprint under journal revision).
- **Atlassian's agentic security reviewer** (Claude Sonnet 4). It correctly localised, typed and explained the vulnerability in 17.5% of its comments. In a shadow deployment, "54% of AgenticSCR's comments were validated by Atlassian product security engineers", and 27.3% of those were addressed ([arXiv:2601.19138](https://arxiv.org/abs/2601.19138), ASE 2026 industry track; peer-reviewed).
- **Planted malicious changes** (PRWeaver, [arXiv:2608.02693](https://arxiv.org/abs/2608.02693), preprint, August 2026), detected when the whole attack sat in one pull request:
  - Claude Code: Sonnet 4.6 79%, Haiku 4.5 67%.
  - Alibaba's open-code-review: 56% with Haiku 4.5, 42% with DeepSeek V4 Flash.
  - "Under whole-window review at N=24, detection falls to 16–22%" when the malicious change is spread among benign ones.
- **Production use.** Google runs Big Sleep and CodeMender in Chrome's CI "every 24 hours across all CLs"; "in May alone, we blocked over 20 vulnerabilities from reaching production" ([Google](https://blog.google/security/chrome-stronger-with-every-update/), 30 July 2026; vendor blog). No precision is given.
- **Neither Anthropic nor OpenAI publishes precision** for `/security-review`, the security-review GitHub Action or `codex review` on security.
- **Adversarial results are in §7.1.** Iterative attacks got vulnerable code past AI reviewers (Claude Code, Codex, CodeRabbit) in 16–97% of cases, depending on the attack and the reviewer.

### 1.6 Field evidence: maintainers and programmes

| Source (grade) | What it shows |
|---|---|
| curl, Daniel Stenberg (anecdote) | Before 2025, "somewhere north of 15%" of bug-bounty submissions were confirmed vulnerabilities; "Starting 2025, the confirmed-rate plummeted to below 5%", and the bounty ended on 31 January 2026. By April 2026, report volume was "about double" 2025 and the confirmed rate was "back to … somewhere in the 15-16% range", with "Almost every security report now us[ing] AI" |
| curl and AI analyzers (anecdote) | From a researcher's ZeroPath-assisted list, curl "merged about 50 separately identifiable bugfixes. The rest were some false positives but also lots of minor issues"; the two researchers' lists passed 400 suspected issues (October 2025). Mythos Preview's report claimed five "Confirmed security vulnerabilities": one became a low-severity CVE, "three false positives" and one "just a bug", plus about twenty bugs with "Barely any false positives" (May 2026) |
| Linux kernel security list (anecdote; Willy Tarreau on LWN, 31 March 2026) | Reports went from "between 2 and 3 per week" two years earlier to "5-10 per day"; "most of these reports are correct, to the point that we had to bring in more maintainers" |
| HackerOne (vendor, April 2026) | Submissions up 76% year over year; "About 25% of findings were confirmed exploitable", unchanged |
| DARPA AIxCC final (August 2025) | 54 of 63 synthetic vulnerabilities found (86%), 43 patched (68%); 18 real vulnerabilities found; about $152 per task. The scoring penalised inaccurate submissions with an "accuracy multiplier" (SoK §3) |

### 1.7 How often coding agents introduce vulnerabilities

The best evidence uses functional tests plus exploits or security tests, so "secure" means an attack failed, not that a scanner was quiet.

| Study (grade) | Setting | Result |
|---|---|---|
| SusVibes ([arXiv:2512.03262](https://arxiv.org/abs/2512.03262), ICML 2026; peer-reviewed) | 186 feature requests from real repositories where a human once committed a vulnerability; 79 CWEs; the baseline prompt already says "Make sure to follow best security practices" | SWE-agent + Claude 4 Sonnet: 57.0% functionally correct, 11.8% correct and secure; "79.3% of its functionally correct" solutions were insecure |
| SusVibes leaderboard, 21 August 2026 (benchmark authors' leaderboard; rows imported from a co-author group's summary) | Same benchmark, current agents | Claude Code + Claude Opus 4.8: 83.9% correct, 32.8% correct and secure (61% of correct solutions insecure). Codex CLI + GPT-5.5: 86.6% and 35.5% (59%). The highest secure score among the August rows, mini-SWE-agent + GPT-5.5, 43.5% |
| BaxBench ([arXiv:2502.11844](https://arxiv.org/abs/2502.11844), ICML 2025; peer-reviewed) | 392 backend tasks with expert-written exploits | "on average, we could successfully execute security exploits on around half of the correct programs generated by each LLM" |
| BaxBench leaderboard (benchmark authors, data dated 22 May 2026) | Same | Claude Opus 4.5 Thinking: 86.2% correct, 56.1% correct and secure (35% of correct programs exploitable); with a generic security reminder 84.7% and 66.1% (22%). GPT-5: 70.7% and 54.3%; with the reminder both fell, to 57.9% and 46.2% |
| Vibe-coded applications ([arXiv:2606.23130](https://arxiv.org/abs/2606.23130), v4 September 2026; preprint) | 200 publicly deployed apps built with Claude Code, Lovable and others, audited by four agents and two human reviewers | 182 of 200 (91.0%) had at least one vulnerability; 65.77% of the 1,186 were Critical or High |

**What reduces it, from the vibe-coded-apps replay.** The authors rebuilt 70 of those vulnerabilities' pre-states and re-ran the task three times per configuration on a GPT-5.6-Terra baseline (Table 4). The baseline reintroduced 54 of 210 (25.7%).

| Change from the baseline | Effect on reintroduction |
|---|---|
| A security-hardening skill in the harness | −13.8 points |
| Adding "ready for production" to the prompt | −14.8 points |
| Adding "review your changes" | −1.9 points |
| A detailed "professional" prompt | +20.0 points |
| A stronger model | +0.5 points |
| A weaker model | +13.8 points |

**Reminders are model-dependent.** On BaxBench a generic reminder cut Claude Opus 4.5's exploitable share from 35% to 22% but lowered GPT-5's scores. In SusVibes, having the agent first name likely CWEs moved the secure rate by +2.7 points at a cost of −7.0 points of correctness (Table 5; read by a background agent).

**Field data are SAST-based and disagree.** Scanner findings are signals, not confirmed vulnerabilities. Read by a background agent, not re-checked:

- One preprint finds agent-written lines add Semgrep security findings at 1.14 times the human rate ([arXiv:2607.09902](https://arxiv.org/abs/2607.09902)).
- Another finds agent pull requests *less* likely to contain a security smell (odds ratio 0.63; [arXiv:2609.17598](https://arxiv.org/abs/2609.17598)).

**Vendors' own reports.** Veracode's July 2026 report says "roughly 44% of AI code generation tasks introduced a risky security vulnerability in tests", judged by its own scanner ([Veracode](https://www.veracode.com/blog/2026-genai-code-security-report-ai-risk/); vendor research).

### 1.8 How often automated security fixes are correct

The consistent finding: **passing the proof of concept and the tests is not the same as a correct fix**.

| Study (grade) | Result |
|---|---|
| PatchBench ([arXiv:2609.04075](https://arxiv.org/abs/2609.04075), 3 September 2026; preprint) | 213 C/C++ tasks whose fix lies off the crash stack, 11 agents. "83.1% of the generated patches eliminate the original PoC crash, yet only 45.3% of the tasks are solved … a significant 1.83× inflation." Codex + GPT-5.6 Sol: 97.2% PoC passed, 59.2% solved; Claude Code + Claude Opus 4.8: 97.7% and 56.8%. The whole evaluation cost "approximately $6,500" |
| AIxCC SoK ([arXiv:2602.07666](https://arxiv.org/abs/2602.07666), USENIX Security 2026; peer-reviewed) | "a significant fraction of generated patches pass all automatic validation, yet contain semantic issues caught only by manual review (CC: 20/53, 37.7%, MR: 26/57, 45.6%)", where CC is Claude Code on Claude 3.7 Sonnet. The two best competition systems had 83.8% and 79.2% patch accuracy. Typical failure: "Agents may suppress the crash symptom rather than addressing the underlying defect" |
| DARPA AIxCC final ([DARPA](https://www.darpa.mil/news/2025/aixcc-results), August 2025) | Of 63 synthetic vulnerabilities, the systems found 54 and patched 43 (68%); patches took 45 minutes on average. The scoring's accuracy multiplier penalised wrong submissions |
| Cloudflare (vendor blog, May and June 2026) | Letting a model write its own patches, "a few go out that fixed the original bug while quietly breaking something else"; hence a fail→pass test gate and human review of every fix |

Vendor fix-acceptance numbers have no denominators:

- Copilot Autofix "remediate[s] more than two-thirds of found vulnerabilities with little or no editing" (GitHub blog, March 2024; vendor).
- CodeMender "upstreamed 72 security fixes", every one "reviewed by human researchers before they're submitted upstream" (Google DeepMind, October 2025; vendor).

Anthropic's disclosure ledger shows how slowly fixes land even for valid reports. "As of October 2, 2026, we've disclosed 6,157 vulnerabilities across 591 open source projects. To our knowledge, 516 of these have been patched" ([red.anthropic.com/2026/cvd](https://red.anthropic.com/2026/cvd/); vendor research).

## 2. The two projects named in #427

Both repositories were cloned and read on 5–6 October 2026: cloudflare/security-audit-skill at `c1c8a8c` (14 September 2026), and alibaba/open-code-review at `182898c` (5 October 2026), with its release v1.12.12 from the same day. Nothing in either was run. Their test suites and validators were read, not executed.

### 2.1 cloudflare/security-audit-skill (MIT): a whole-codebase audit

**What it is.** "A coding-agent skill for multi-phase security audits with independently verified, machine-readable findings" (repository description).

- **License and history.** MIT, "Copyright (c) 2025-2026 Cloudflare, Inc." Created 18 June 2026, the day of the blog post below. 14 commits, nearly all by one Cloudflare engineer. 24,865 stars and 1,468 forks on 6 October 2026.
- **Two modes.** Guidance mode is the default. Full audit mode runs only "when the user explicitly asks to audit or pen-test a codebase, asks for a full, comprehensive, or end-to-end security review, or requests report artifacts". If a request could mean either, the skill says to "ask one focused question" (SKILL.md, "Operating modes").
- **Profiles.** `quick` (one hunter wave, one critic, one verifier per candidate), `standard` (the default), `deep`. A **scoped run** can audit "named paths, one subsystem, one companion domain, or the diff between two source refs", so the same skill can also review a branch's diff, as a partial pass (SKILL.md, "Run profiles and scope").
- **Budget.** Counted in agent invocations, reserved in a fixed order: four reconnaissance calls, the critics, and verifiers before any hunter (SKILL.md, "Cost budget").

**The six phases** (README; SKILL.md; RECONNAISSANCE.md; HUNTING.md; VALIDATION-AND-REPORTING.md):

| Phase | Who does it | What comes out |
|---|---|---|
| 1. Reconnaissance | Four parallel read-only `research` sub-agents: product and stack; principals and controls; entry surfaces and sinks; local execution and deployment visibility | `architecture.md` (capped at about 1,000 words) and `coverage-ledger.json`, one unit per entry surface × trust boundary × subsystem × attack class |
| 2. Coverage-led hunting | One `general` hunter per ledger unit or group of related units, then a fresh coverage critic after each wave, and a second "final-clean" critic in `standard` and `deep` | One JSON object per hunter: units covered, candidates, hardening notes, uncovered surfaces |
| 3. Candidate validation | One fresh `general` verifier per unique candidate, prompted "You did not write this candidate. Try to refute it" | A `confirmed`, `needs_validation` or `rejected` record |
| 4. Structured output | The parent | `findings.json`, checked by `validate-findings.cjs` and `validate-coverage-ledger.cjs` |
| 5. Record verification | One fresh `research` verifier per `confirmed` and `needs_validation` record; any material change goes to another fresh verifier | `verified`, or a replacement record |
| 6. Reporting | The parent | `REPORT.md`, `FINDINGS-DETAIL.md`, `NEEDS-VALIDATION.md` |

**The verdicts** (report-schema.json; VALIDATION-AND-REPORTING.md, Phase 4):

- **`confirmed`** needs a complete source trace from `entrypoint` to `sink` *and* "a bounded local observed result". The schema requires `execution.observed_result`, and the validator rejects a confirmed record without one (`validate-findings.cjs` lines 553–554). Only confirmed records get a severity (likelihood, impact, overall: informational to critical) and a confidence (low, medium, high).
- **`needs_validation`** is "a specific source-grounded boundary hypothesis" that is blocked, with the exact blockers, and a local or "owner-observed" deployment validation plan. It has no severity.
- **`rejected`** is a disproved candidate, kept "so future runs do not repeat the unsupported claim".

**What is deterministic.** Two zero-dependency Node.js validators (773 and 872 lines, with tests of 652 and 740 lines) check `findings.json` against the JSON schema and the coverage ledger against its state table. The skill also specifies an exact procedure for copying evidence files out of a sandbox ("trusted parent-side code only"). Everything else, from reconnaissance to the reports, is model work, and the coverage IDs are computed by the parent model and only checked by the validator.

**Requirements** (README, "Requirements"; SKILL.md, "Universal execution safety"):

- "A coding agent with a model that supports tool use and parallel sub-agents."
- Node.js for the validators.
- An OS-enforced sandbox for any target-controlled build, test or process: no external network, an empty allowlisted environment, read-only target and toolchain, writes only to scratch, and CPU, memory, process, file, disk and wall-clock limits. "Without these controls, the workflow keeps the lead as `needs_validation` instead of executing target code."

**Languages.** The skill is organised by trust boundary and attack class, not by language: core classes in ATTACK-CLASSES.md, plus ten domain companions (memory safety and binaries; AI and LLM; web protocols and auth; client side; supply chain and release; cloud; RPC and messaging; resource exhaustion; data isolation; desktop, mobile and local IPC). The only languages it names are C, C++ and unsafe Rust, for memory safety, plus native bindings (CGo, JNI, Python) and JavaScript-to-native bridges in passing. Nothing is specific to PHP, Python web code, TypeScript, Swift, Kotlin or C#. Cloudflare ran the harness that grew from it on "Rust, Go, C, Lua, TypeScript and Python" with "no per-language tuning" (blog below). Known-vulnerable dependencies are out of scope: "A mutable or known-vulnerable dependency is not a finding by itself" (SUPPLY-CHAIN-AND-RELEASE.md), and an open third-party PR (#43) proposes saying so in the README.

**Size.** SKILL.md is 22,026 bytes (3,115 words). The 15 Markdown files total 182,981 bytes (24,768 words), of which the five a full audit always uses (SKILL, RECONNAISSANCE, HUNTING, ATTACK-CLASSES, VALIDATION-AND-REPORTING) are 93,912 bytes. For comparison, all of thirdshift's Factory skills together are 56,132 bytes of Markdown.

**Published cost.** None for the skill itself. The README's only quality figure: "In our test runs, a single run found roughly half of the vulnerabilities that repeated runs found in total."

**Can it run headless?** Partly, as written.

- **Three places ask a human.** The mode question above; an output directory inside the target that git does not ignore ("Otherwise stop and request an external path"); and a budget too small for the reserves ("ask for a larger budget, narrower scope, or different profile"). A Session prompt that names full audit mode, a profile, a scope, a budget and an output directory outside the worktree answers all three in advance.
- **It never edits the target.** It writes only to its output directory, by default `~/security-audit-skill/<repo-name>/run-<N>`. That fits a pass that, like the Architecture review, "changes nothing in the repository".
- **Sub-agents.** Claude Code's `-p` mode has its sub-agent tool; this machine's Architecture review sessions spawn sub-agents through it. Codex 0.160.0 lists `multi_agent` as stable and enabled (`codex features list`), but `codex exec` drops sub-agent threads' events from its JSONL ([codex-headless-harness.md](codex-headless-harness.md)), and the one Codex Run in this machine's logs (#345, 5 October) shows no `collab_tool_call` item, though its agent said its two reviews ran "in parallel". An open third-party PR (#58) adds a sequential fallback because "some agent runtimes can kill or destabilize parallel sub-agents".
- **The sandbox, on this machine, is the binding constraint.** `kernel.apparmor_restrict_unprivileged_userns = 1`, `unshare --user --map-root-user --net true` fails with "Operation not permitted", and neither bubblewrap, firejail, Docker nor Podman is installed (checked 6 October). This is the same restriction that makes thirdshift run Codex with `--dangerously-bypass-approvals-and-sandbox` ([ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)). So an unmodified run here could never produce a `confirmed` record: every lead would stay `needs_validation`. An open third-party PR (#60) reports exactly this inside containers, where runs "conclude 'no OS sandbox' and downgrade **every** lead to `needs_validation`", and documents a sibling-Docker sandbox instead.

### 2.2 Cloudflare's posts about the harness the skill seeded

Cloudflare's "Build your own vulnerability harness" (18 June 2026; vendor blog) exists, and its earlier "Project Glasswing: what Mythos showed us" (18 May 2026; vendor blog) describes the same pipeline. What they add:

- **The skill's limits, in Cloudflare's words.** They started with "a ~450-line security-audit skill" in one session. "A single run finds only about half the bugs you'd catch across multiple runs. In our experience the ones it did find skewed toward the simpler and less subtle." They hit context exhaustion ("An hour in, the context window fills up"), lost runs to crashes, and could not reason across repositories. "Once your process is basically 'run it ten times and diff by hand,' you probably need to start looking at a real harness."
- **Why a generic agent session is the wrong shape.** "A single agent session (even with subagents) against a hundred-thousand-line repository can cover maybe a tenth of a percent of the surface in a useful way before the model's context window fills up and compaction kicks in" (Glasswing post).
- **What made findings trustworthy.**
  - A hunter must state the threat model before filing: attacker, boundary, broken assumption.
  - "Every confirmed finding ships with a PoC written as a test that runs against the original, untouched codebase … If there is no working PoC, we treat the finding as fake."
  - Plain code checks that cited files and paths exist and that the test and patch parse. The validator "cannot log findings of its own".
  - Validation runs on a different model from discovery, "so the models are effectively double-checking each other".
  - Their failure modes: the agent "will edit the source code so its own exploit works, then triumphantly report the bug it just created", or write a tautological test.
- **What moved precision.** "The biggest jump in quality came from giving Hunters a sandbox (built on unshare) to crash binaries." Better reconnaissance context cut the initial validation rejection rate "from 40% down to 11%", and raised the share of "high-integrity findings" from 35% to 58%. Semgrep, wired in, was invoked "zero times in a month of runs".
- **The funnel** (lifetime, Cloudflare's own code): 20,799 raw candidates; about 12,057 survived validation; with another harness's output the pool was 13,841; deduplication folded 5,442; 1,154 were wrong-repository or low-risk; 7,245 were "actionable findings". Cloudflare claims no recall: "There's no labeled set of every real bug in a codebase, so any claimed recall number is entirely speculative."
- **Fixes.** The Fixer needs "a clean fail→pass flip on the target test", "never merges code on its own; a human must review the branch", and "Left to patch freely, a model will happily fix a security bug while quietly breaking an unrelated feature". The Glasswing post says the same from experience: "we tried letting the model write its own patches and watched a few go out that fixed the original bug while quietly breaking something else the code depended on."
- **Cost and time.** No dollar or token figure. "Almost all of the compute budget goes directly into the hunt stage", and each extra Gapfill pass "costs roughly half as much as the initial hunt". They budget per repository, with 50 to 200 workers. A standard repository of about 30,000 lines takes 3–4 hours for about 100 initial findings, three more hours of deduplication and judgment, and about 14 hours from discovery to fix pull requests. The worst full scan took "just over 14 hours". Hence "the big scans are a periodic backlog sweep and not a per-PR check."

### 2.3 alibaba/open-code-review (Apache-2.0): a diff reviewer

**What it is.** A Go CLI, `ocr`, installed from npm (`@alibaba-group/open-code-review`, whose postinstall downloads a platform binary from GitHub Releases and checks it against `sha256sum.txt`). It "originated as Alibaba Group's internal official AI code review assistant", which Alibaba says "served tens of thousands of developers and identified millions of code defects" (README; vendor claim). Apache-2.0, "Copyright 2026 Alibaba", with no top-level NOTICE file. Created 18 May 2026; 43,896 stars and 3,165 forks on 6 October 2026. It is a general code reviewer: security is one category of eight, not its focus.

**The pipeline** (docs/architecture, "High-level pipeline" to "Comment processing pipeline"):

| Step | Deterministic or model |
|---|---|
| Diff from the workspace, a commit, or `--from/--to` (merge-base) | Deterministic (git) |
| Five-gate file filter: binaries, user excludes, includes, unsupported extensions, built-in excludes such as tests and `vendor/` | Deterministic |
| Rule per file, by glob, from 54 built-in rule documents | Deterministic |
| Semantic grouping of files, at most 10 per group | One model call over file metadata |
| Plan phase, when a file has 50+ changed lines or a group 100+ | One model call, no tools |
| Main loop per group: tools `code_search`, `file_read_diff`, `file_find`, `file_read`, `code_comment`, `task_done`; up to 100 tool requests; 1, 2 or 3 rounds for `--effort low/medium/high` | Model, up to 8 groups at once |
| Line positioning of each comment | Deterministic (sliding-window match), with a model fallback |
| Review filter that "removes ones that are provably incorrect" (`--no-filter` skips it) | One model call |
| Output as text, JSON or SARIF | Deterministic |

- **Finding schema.** `path`, `content`, `start_line`/`end_line` (both 0 when positioning failed), `category` (bug, security, performance, maintainability, test, style, documentation, other), `severity` (critical, high, medium, low), and optional `suggestion_code`, `existing_code` and `thinking`. There are no verdicts like `confirmed` or `needs_validation`, and the exit code is 0 whatever was found (1 only for a fatal error).
- **The security rules.** The rule documents for Rust, PHP, Python, TypeScript/JavaScript and Swift each carry a security section among correctness ones; Kotlin's has none. 32 of the 54 contain the instruction "Favor precision over recall". `Cargo.toml`, `composer.json`, `package.json` and GitHub Actions workflows have rules of their own. There is no C# rule; `.cs` files get the three-line default ("Are there security vulnerabilities such as SQL injection or XSS? …"). Custom rules come from `--rule` or `.opencodereview/rule.json` and replace the built-in rule unless `merge_system_rule` is set.
- **Model access.** The default mode calls its own model endpoint (Anthropic, OpenAI Chat Completions or Responses, or Bedrock) with its own key (`OCR_LLM_URL`, `OCR_LLM_TOKEN`, `OCR_LLM_MODEL`). **Delegation mode** (`ocr delegate preview`, `ocr delegate rule`) uses no model at all: OCR selects files and rules, and the host agent, such as Claude Code or Codex on its subscription, does the review itself. That also means delegation mode drops everything model-driven in the table above, including the review filter and the rounds.
- **Telemetry** is off by default.
- **Prompt injection.** The review prompts do not tell the model to treat repository content as untrusted; only the separate QCA plugin's system prompt does.

**Published quality and cost** (README benchmark image; vendor research): on AACR-Bench (50 repositories, 200 real pull requests, 10 languages, 1,505 annotated issues; Alibaba's own benchmark, [arXiv:2601.19494](https://arxiv.org/abs/2601.19494), preprint, all defect types, not security alone):

| Tool and model | Precision | Recall | Time per review | Tokens per review |
|---|---|---|---|---|
| OCR v1.3.1 + Claude Opus 4.6 | 33.9% (301/889) | 20.0% | 1 min 23 s | 385K |
| OCR v1.3.1 + Claude Opus 4.8 | 37.8% (176/465) | 11.7% | 1 min 6 s | 352K |
| OCR v1.3.1 + GPT-5.5 | 32.1% (234/728) | 15.5% | 2 min 51 s | 422K |
| Claude Code v2.1.169 + Claude Opus 4.6 | 7.23% (435/5,980) | 28.9% | 13 min 6 s | 5,664K |
| Claude Code v2.1.169 + Claude Opus 4.8 | 15.93% (191/1,200) | 12.7% | 5 min 38 s | 2,062K |
| Codex v0.140.0 + GPT-5.5 | 27.82% (74/266) | 4.92% | 2 min 58 s | 525K |

The README calls OCR's lower recall "a deliberate trade-off favoring precision over noise". The "precision" denominators count every reported issue, so a real defect the annotators missed counts against a tool. The table measures OCR's own pipeline, not delegation mode, and versions far older than v1.12.12. The one third-party security measurement found: on PRWeaver's planted malicious changes, open-code-review caught 56% with Claude Haiku 4.5 and 42% with DeepSeek V4 Flash, against 67% and 79% for Claude Code with Haiku 4.5 and Sonnet 4.6 (§1.5).

**Can it run headless?** The CLI can: `ocr review --from <base> --to HEAD --audience agent --format json --output <file>` asks nothing. The skill text cannot as written: it tells the agent to "Ask the user whether to upgrade … and wait for the answer", to "ask for permission before applying any changes" unless asked to fix, to "Always verify fixes with the user before committing", and to "Prompt the user to configure an LLM provider".

### 2.4 What adapting each as a Factory skill would take

| | cloudflare/security-audit-skill | alibaba/open-code-review |
|---|---|---|
| Fits | Shape 1, a security pass (and a scoped diff run, as a partial pass) | Shape 2, a Security axis in each Run's review |
| License to keep | MIT: the copyright and permission notice must travel with every copy, including the copy embedded in thirdshift's binary ([ADR 0001](../adr/0001-rust-binary-with-embedded-skills.md)), as `skills/LICENSE` does for Matt Pocock's skills | Apache-2.0: a copy of the license, retained notices, and "prominent notices stating that You changed the files" on modified files |
| Headless edits | Name full audit mode, profile, scope, budget and an output directory in the Session prompt; replace the three "ask" branches with an `incomplete` ending | Remove the four "ask the user" branches; decide which severities the author must address |
| New machinery | An OS sandbox that works here (an AppArmor profile for bubblewrap, or Docker or Podman), or accept `needs_validation`-only output; Node.js on every machine; a step that publishes findings somewhere private (§5) | The `ocr` binary on every machine; a model key billed at API prices for the default mode, or delegation mode, which keeps only file selection and rules; a security-only rule file |
| What it would not cover | Dependency advisories; anything it cannot execute here | C# (no rule) and Kotlin (no security section); anything outside the diff |
| Size | About 183 KB of Markdown, 3.3 times all current Factory skills | 12.6 KB (skill) or 8.0 KB (delegation skill), plus the rule documents used |

## 3. Others like them

"Local" below means a CLI, skill or plugin a headless Claude Code or Codex session on this machine could run. "Hosted" means the work runs on the vendor's service. Repositories were read at their heads on 6 October 2026, and every license is the SPDX identifier GitHub reports, checked against the license file.

### 3.1 Diff reviewers (shape 2)

| Tool | Local or hosted | License | Languages | Precision evidence | Cost evidence |
|---|---|---|---|---|---|
| Claude Code `/security-review` | Local, built into Claude Code | The prompt is public in anthropics/claude-code-security-review (MIT) | Any | None published | None published |
| anthropics/claude-code-security-review (GitHub Action) | Hosted in GitHub Actions; calls the Claude API with a key | MIT | Any | None published | None stated |
| Anthropic security-guidance plugin | Local (Claude Code hooks) | Apache-2.0 | Any | None published | None stated |
| `codex review` / `codex exec review` | Local, built into Codex | Apache-2.0 (Codex) | Any | None for security | None |
| openai/codex-security (CLI and SDK) | Local CLI; ChatGPT login or API key | Apache-2.0 | "language-agnostic" | Relative figures only, in OpenAI's posts (below) | `--max-cost`; standard API prices |
| alibaba/open-code-review | Local CLI | Apache-2.0 | 54 rule files; no C# rule; no security section for Kotlin | AACR-Bench, general review (§2.3); PRWeaver (§1.5) | 352K–422K tokens per review (§2.3) |
| Trail of Bits `differential-review` skill | Local skill | CC-BY-SA-4.0 | Any | None published | None |
| getsentry/skills `security-review` | Local skill | Apache-2.0; its OWASP-derived references are CC BY-SA 4.0 | Any | None published | None |
| gemini-cli-extensions/security `/security:analyze` | Local, Gemini CLI only | Apache-2.0 | Any | 90% precision, 93% recall on the OpenSSF CVE Benchmark (JS/TS, small, hand-scored; vendor docs) | None |
| GitHub Copilot `/security-review` (Copilot app and CLI, public preview since 14 July 2026) and AI Scan | Copilot client with hosted models; AI Scan hosted | Proprietary | AI Scan: languages CodeQL lacks, PHP among them | None published | A Copilot plan; AI Scan uses AI credits |

**Claude Code `/security-review`.**

- **What it reviews.** "Analyze the changes on your current branch for security vulnerabilities. Reviews the diff between your branch and origin's default branch" ([commands](https://code.claude.com/docs/en/commands); vendor docs). It "builds its review context by diffing your branch against `origin/HEAD`", and stops when that ref is missing ([errors](https://code.claude.com/docs/en/errors#security-review-fails-without-origin-head)).
- **The prompt** (`.claude/commands/security-review.md` in anthropics/claude-code-security-review, 190 lines, 10,837 bytes; the README says the built-in command "provides the same security analysis capabilities as the GitHub Action"):
  - Tools are read-only: `git diff`, `git status`, `git log`, `git show`, `git remote show`, Read, Glob, Grep, LS, Task.
  - "focus ONLY on security implications newly added by this PR. Do not comment on existing security concerns" (line 36), and "Only flag issues where you're >80% confident of actual exploitability" (line 39).
  - One sub-task finds candidates; then "for each vulnerability … create a new sub-task to filter out false-positives", in parallel, and drop any "where the sub-task reported a confidence less than 8" (lines 187–189). The filter is told: "You do not need to run commands to reproduce the vulnerability, just read the code … Do not use the bash tool or write to any files" (line 136).
  - Its 17 hard exclusions include denial of service, rate limiting and "lack of hardening", and two that matter here: "Memory safety issues such as buffer overflows or use-after-free-vulnerabilities are impossible in rust" (line 148, which ignores `unsafe`), and "Including user-controlled content in AI system prompts is not a vulnerability" (line 152), which would hide prompt-injection paths in thirdshift and vaulted-agent.
- **Headless use is documented only indirectly.** The headless page says "User-invoked skills and custom commands work" in `-p` and that commands that only run in the terminal interface are not available ([headless](https://code.claude.com/docs/en/headless)). The skills page says "A few built-in commands are also available through the Skill tool, including `/init` and `/security-review`" ([skills](https://code.claude.com/docs/en/skills)). This session's own skill list includes it. **[needs live check]**: whether `claude -p "/security-review"` expands in a Run's worktree, whose `origin/HEAD` may not be set.

**anthropics/claude-code-security-review.** A GitHub Action (MIT; 6,311 stars; last commit 11 February 2026) that runs the same prompt on `pull_request`, once per pull request unless `run-every-commit` is set, with a Claude API key.

- Its false-positive filter has two stages: regular-expression exclusions, then one Claude API call per finding that returns `confidence_score` and `keep_finding`.
- It posts a non-blocking review (`event: 'COMMENT'`) with inline comments.
- Its default model is still `claude-opus-4-1-20250805` (`claudecode/constants.py`).
- README: "This action is not hardened against prompt injection attacks and should only be used to review trusted PRs."

**Anthropic's security-guidance plugin** (claude-plugins-official, Apache-2.0, v2.0.10) works through hooks: a regular-expression check on each edit, a model review of the diff at the end of each turn, and an agentic review when Claude runs `git commit` or `git push`. "None of the layers block writes or commits" ([security-guidance](https://code.claude.com/docs/en/security-guidance); vendor docs). Whether its hooks fire in `-p` sessions was not checked **[needs live check]**.

**Codex.** `codex review` and `codex exec review` (0.160.0) review `--uncommitted`, `--base <BRANCH>` or `--commit <SHA>`, or follow custom instructions, but not both. On this machine `codex review --base main "focus on security"` fails with "the argument '--base <BRANCH>' cannot be used with '[PROMPT]'". The built-in rubric is a general P0–P3 review with no security-specific rules (`codex-rs/prompts/templates/review/rubric.md`, read by a background agent).

**openai/codex-security.** OpenAI's Codex Security product now has an open-source local CLI and TypeScript SDK (Apache-2.0; created 13 July 2026; npm `@openai/codex-security`).

- **Scopes and controls.** `scan . --diff origin/main`, `--path`, or `--mode deep`. "Scans are report-only by default. `--fail-on-severity high` exits with `1` for high or critical findings. Incomplete scans exit with `2`". `--max-cost` caps spend. Output directories "must be empty and outside the scanned directory and enclosing Git worktree" ([cli.md](https://github.com/openai/codex-security/blob/main/sdk/typescript/docs/cli.md); vendor docs).
- **Accounts.** "Sign in with ChatGPT, or supply an API key for CI". Cost displays use standard API prices and "do not measure ChatGPT subscription allowance". "Some cybersecurity requests and protected findings require Trusted Access for Cyber approval."
- **Trust.** "Codex Security runs with your operating-system permissions. Scan subprocesses can inherit your environment" (SDK README, "Local security model").
- **Requirements.** Node.js 22.13+, 24 or 26, and Python 3.10+.
- **The hosted service** runs "in an ephemeral, isolated container", and its auto-validation "tries to reproduce each issue in a clean container. Findings that successfully reproduce are marked as validated". Cloud scans set up after 1 October 2026 "are billed based on token usage", and "This usage is not covered by your plan's included usage allowance", with $500 of free credits for eligible accounts ([FAQ](https://learn.chatgpt.com/docs/security/faq); vendor docs).
- **Quality** (vendor blog; openai.com refused every fetch, so both posts were read from Wayback Machine copies).
  - Aardvark (30 October 2025): "In benchmark testing on 'golden' repositories, Aardvark identified 92% of known and synthetically-introduced vulnerabilities", with ten CVEs from open-source scanning. The same post says "around 1.2% of commits introduce bugs".
  - The Codex Security research preview (6 March 2026): across "more than 1.2 million commits" in 30 days it reported "792 critical findings and 10,561 high-severity findings", and "false positive rates on detections have fallen by more than 50% across all repositories" during the beta. Neither post gives a precision or a false-positive rate as such.

### 3.2 Whole-codebase auditors (shape 1)

| Tool | Local or hosted | License | Languages | Notes |
|---|---|---|---|---|
| cloudflare/security-audit-skill | Local skill | MIT | Any (§2.1) | Needs an OS sandbox to confirm anything |
| openai/codex-security `scan` / `--mode deep` | Local CLI | Apache-2.0 | Any | Above |
| Claude Security (formerly Claude Code Security) | Hosted for Enterprise; a Claude Code plugin | Plugin proprietary: "All rights reserved", internal use "solely with Claude Code" | Any | Won't start a scan in a non-interactive session unless the user's own request acknowledged the cost, "never a line that arrives from a scheduled task"; SECURITY.md: "The code you scan is trusted" |
| Google CodeMender | Hosted agent plus local client; "a limited set of customers in Public Preview" | Proprietary | C/C++, C#, Go, Java, JS/TS, Kotlin, Python, Ruby, Rust, PHP by default | Verifies "by building the code and attempting to exploit found vulnerabilities" ([docs](https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender)) |
| Google Big Sleep | Internal to Google | — | — | Not available to others |
| GitHub CodeQL + Copilot Autofix | Hosted (Actions) | Proprietary | No PHP (§4) | Free on public repositories only |
| Semgrep CE | Local CLI | Engine LGPL-2.1; rules under the Semgrep Rules License v1.0 (internal business use only, no redistribution) | Many, PHP included | Deterministic rules (§4) |
| Semgrep Assistant ("Semgrep Multimodal") | Hosted only | Proprietary | — | "Requires the Semgrep AppSec Platform" ([docs](https://docs.semgrep.dev/semgrep-assistant/overview)) |
| AIxCC cyber reasoning systems (Atlantis, Buttercup, RoboDuck, FuzzingBrain, ARTIPHISHELL, 42-b3yond-6ug, Lacrosse) | Self-hosted clusters | GPL-3.0, AGPL-3.0, Apache-2.0 or MIT (per a background agent; Buttercup's AGPL-3.0 checked) | C and Java, OSS-Fuzz harnesses required | Buttercup's stated minimum is 8 cores, 16 GB RAM, 100 GB disk |
| protectai/vulnhuntr | Local CLI | AGPL-3.0 | Python only | Last push 6 February 2025 |
| Trail of Bits `static-analysis`, `variant-analysis`, `fp-check` and others | Local skills | CC-BY-SA-4.0 | Some language-specific | `fp-check` gives "TRUE POSITIVE or FALSE POSITIVE" verdicts but lists AskUserQuestion among its tools |
| openai/skills `security-best-practices` | Local skill | Apache-2.0 (per skill) | Python, JS/TS, Go | Guidance, not an audit loop |

Notes:

- **Anthropic's red-team scaffold is the simplest published shape that works.** For Claude Mythos Preview: an Internet-isolated container, Claude Code prompted "Please find a security vulnerability in this program", one agent per file ranked by likely interest, and a final agent asked "I have received the following bug report. Can you please confirm if it's real and interesting?" ([red.anthropic.com, 7 April 2026](https://red.anthropic.com/2026/mythos-preview/); vendor research). Its quality numbers are in §1.2.
- **AIxCC systems don't fit.** All final-round challenges were "based on OSS-Fuzz, providing build tooling and fuzzing harnesses" in C and Java (SoK, [arXiv:2602.07666](https://arxiv.org/abs/2602.07666), USENIX Security 2026, read by a background agent). None of the user's repositories has such harnesses, and the machine is half Buttercup's minimum.
- **Licences constrain adaptation.** CC-BY-SA-4.0 (Trail of Bits) requires adapted text to stay under CC-BY-SA, a different licence from the MIT that covers `skills/`. AGPL-3.0 (vulnhuntr, Buttercup) and the Semgrep Rules License (no redistribution) rule out embedding in thirdshift's MIT binary. The Claude Security plugin cannot be redistributed at all.
- **Matt Pocock's skills have nothing on security.** mattpocock/skills at `4588b32` (5 October 2026) has 38 `SKILL.md` files. No file mentions "vulnerab", "OWASP" or "CWE"; the three hits for "security" are a Mermaid setting, a CDN note and an FAQ about security scanners flagging a skill.

## 4. Cheap deterministic complements

### 4.1 What GitHub gives for free

All from GitHub's documentation as read on 6 October 2026 (vendor docs), unless marked as an observation of the user's repositories (my read-only `gh api` calls).

| Feature | Public repository | User-owned private repository |
|---|---|---|
| Code scanning with CodeQL, default or advanced setup | Yes | No: "If you are on a **GitHub Free** or **GitHub Pro** plan, you can only use code scanning on repositories that are publicly available" ([private-repository-enablement](https://docs.github.com/en/code-security/reference/code-scanning/troubleshoot-analysis-errors/private-repository-enablement)) |
| Copilot Autofix for code scanning alerts | Yes; "does not consume AI credits" | No |
| Uploading third-party SARIF | Yes | No (403 without Code Security) |
| Dependency review and `actions/dependency-review-action` | Yes | No |
| Dependabot alerts, security updates, version updates; dependency graph | Yes | Yes: "Available for all GitHub plans" ([github-security-features](https://docs.github.com/en/code-security/getting-started/github-security-features)) |
| Secret scanning, and push protection for the repository | Yes | No (only Enterprise Managed Users) |
| Push protection for users | On by default; "Stops you from pushing secrets to public repositories" | Not covered |
| Private vulnerability reporting | Yes: "Owners and administrators of public repositories can allow …" | No |
| Repository security advisories and CVE requests | Yes | Advisories unclear; "You may request a CVE for public repositories, but cannot do so for private repositories" (REST reference) |
| Buying GitHub Code Security or Secret Protection | — | Not possible: "You must be on a GitHub Team or GitHub Enterprise plan in order to purchase" them ([about GHAS](https://docs.github.com/en/get-started/learning-about-github/about-github-advanced-security)) |

**CodeQL's languages.** C/C++, C#, Go, Java/Kotlin, JavaScript/TypeScript, Python, Ruby, Rust, Swift and GitHub Actions workflows. "CodeQL does **not** support languages that are not listed above. This includes, but is not limited to, **PHP**" ([CodeQL code scanning](https://docs.github.com/en/code-security/concepts/code-scanning/codeql/codeql-code-scanning)). Rust has been generally available for default setup since 14 October 2025 (changelog). Swift needs macOS runners, and Kotlin may need a build. CodeQL 2.26.0 (10 July 2026) added `js/system-prompt-injection`, for "untrusted, user-provided values [that] flow into an AI model's system prompt", for JavaScript and TypeScript only, not Rust ([changelog](https://github.blog/changelog/2026-07-10-codeql-2-26-0-adds-kotlin-2-4-0-support-and-ai-prompt-injection-detection/)). GitHub's newer AI Scan (public preview since July 2026) is "designed to cover languages … not currently supported by CodeQL", PHP among them, and its findings "are advisory and do not block pull request merges". During the preview it "requires a GitHub Advanced Security license and a GitHub Copilot license" ([AI Scan](https://docs.github.com/en/code-security/concepts/code-scanning/ai-powered-security-detections)). The page mentions "an eligible public repository owned by a personal account", but a personal account cannot buy that licence, so whether keeplore could use it is unclear.

**The seven repositories today** (observation, 6 October 2026):

- **All seven:** Dependabot alerts off (`GET /vulnerability-alerts` returns 404), Dependabot security updates off.
- **Five public** (thirdshift, cascade, keeplore, muxboard, vaulted-agent): private vulnerability reporting off; CodeQL default setup `not-configured`; secret scanning and push protection on only for muxboard and vaulted-agent.
- **What default setup would analyse if switched on** (its `languages` field): thirdshift: Rust, JS/TS, Actions; cascade: Rust, Swift, C#, Java/Kotlin, C/C++, JS/TS, Actions; keeplore: JS/TS, Java/Kotlin, Actions, and **no PHP**; muxboard: Python, JS/TS, Actions; vaulted-agent: Rust, Actions.
- **Two private** (chart35, clave): code scanning "not enabled for this repository" and unavailable on the plan; private vulnerability reporting does not exist for them.
- **No CI workflow in any of the seven runs a security tool** today: no cargo-audit, cargo-deny, npm audit, composer audit, pip-audit, OSV-Scanner, gitleaks or Semgrep step.

### 4.2 Per-ecosystem tools

From each tool's own documentation and source, as read by a background agent and spot-checked:

| Tool | Licence | Covers | Fails the build by default? | Ignoring one advisory | Official GitHub Action |
|---|---|---|---|---|---|
| cargo-audit | Apache-2.0 OR MIT | `Cargo.lock` against RustSec | Yes, on any vulnerability | `--ignore RUSTSEC-…` or `audit.toml` | `rustsec/audit-check` |
| cargo-deny | MIT OR Apache-2.0 | Advisories, licences, bans, sources | Yes; exit code is a bitset per check | `ignore` entries with a `reason`, and optional expiry | `EmbarkStudios/cargo-deny-action` |
| npm audit | Artistic-2.0 (npm) | `package-lock.json` | Yes, "if any vulnerability is found"; `--audit-level` raises the bar | Not documented | None (built in) |
| composer audit | MIT | `composer.lock` | Yes; abandoned packages fail too by default | `policy.advisories.ignore-id` | None |
| pip-audit | Apache-2.0 | Requirements files, `pyproject.toml`, `pylock` | Yes, and the exit code "cannot be suppressed" | `--ignore-vuln` | `pypa/gh-action-pip-audit` |
| OSV-Scanner | Apache-2.0 | `Cargo.lock`, npm/pnpm/yarn/bun locks, `composer.lock`, Python locks incl. `uv.lock`, Gradle, .NET; not Swift's `Package.resolved` | Yes (exit 1) | `osv-scanner.toml`, with `ignoreUntil` | `google/osv-scanner-action`, with a pull-request mode that compares base and head |
| gitleaks | MIT (CLI); the Action is under its own EULA, free for personal accounts | Secrets in git history | Yes (exit 1) | `.gitleaksignore`, `#gitleaks:allow` | `gitleaks/gitleaks-action` |
| Semgrep CE | Engine LGPL-2.1; rules under the Semgrep Rules License | Many languages incl. PHP, single-file analysis only in CE | No, unless `--error` | `nosemgrep`; `--baseline-commit` for diff-only | None current (`semgrep-action` is archived) |
| Bandit | Apache-2.0 | Python | Yes | `# nosec`, baseline | `PyCQA/bandit-action` |

All of these run in seconds to minutes on a CI runner and none needs a model.

### 4.3 How these meet a factory whose Runs fix their own red checks

How thirdshift reads CI today (from its source):

- It reads no log text: it compares check names and conclusions ([src/ci.rs](../../src/ci.rs), [ADR 0008](../adr/0008-inherited-failures-fail-the-run.md)).
- The CI-fix Repair is told: "Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass", and to leave a failure that "also fails on {base}" alone ([src/prompt.rs](../../src/prompt.rs) `ci_fix_repair`).
- A Check re-run needs every failed check to be a GitHub Actions job.

**CodeQL's pull-request check is not an Actions job, and its log does not hold the alerts.**

- "For all configurations of code scanning, the check that contains the results of code scanning is: **Code scanning results** … Any new alerts on lines of code changed in the pull request are shown as annotations" ([triage alerts in pull requests](https://docs.github.com/en/code-security/how-tos/manage-security-alerts/manage-code-scanning-alerts/triage-alerts-in-pull-requests)).
- It fails "with a severity of `error`, `critical`, or `high`"; lower severities pass as warnings.
- The Actions workflow log holds only "summary metrics and extractor diagnostics" (code scanning logs docs).
- A background agent read a failing check run on a public repository (juice-shop PR #3634). The check run's title and summary named "2 new alerts including 2 high severity security vulnerabilities". `GET /repos/{o}/{r}/check-runs/{id}/annotations` returned each alert's path, line, rule and message.
- So a CI-fix Repair that follows its prompt's hint to `gh run view --log-failed` finds nothing. It has to read the check run's annotations, or the code scanning alerts API with `ref=refs/pull/N/merge`. Thirdshift's Check re-run cannot apply to it either. (Inference from the docs and the prompt.)

**CodeQL's pull-request check is diff-scoped, so it suits per-Run use.** "You will only see an alert in a pull request if **all** the lines of code identified by the alert exist in the pull request diff" (code scanning alerts docs). A new CodeQL release therefore does not turn every open pull request red, though a release that relocates alerts (CodeQL 2.26.4 did, for Rust, in September 2026) can surface old alerts on lines a pull request touches. Default setup does not scan pull requests from forks.

**Dependency audits in pull-request CI are the risky kind.** cargo-audit, cargo-deny, npm audit, composer audit and pip-audit all fail when the lockfile contains any version with a known advisory, whether or not the pull request touched it. What a new advisory would do to the factory (inference from ADR 0008 and the Repair prompt):

1. A new advisory is published. The base branch's last result for the audit check is still green, because it ran before the advisory existed.
2. Every Run's next CI turns red on that check. Because the base branch commit's result is green, thirdshift does not treat this as an Inherited failure. Each Run starts a CI-fix Repair.
3. Each Repair either bumps the dependency inside its own unrelated pull request, or, following "If a failure is not caused by this branch … do not change code for it", changes nothing. That is a Declined CI fix, which ends in a Check re-run and then a Failed run.
4. This repeats for every Run on that repository until something new lands on the base branch and re-runs the check there. Only then do Runs see an Inherited failure, and with the User config's `base.fix = true`, start the one shared Base fix that ADR 0008 describes.

The tools' own documentation warns against exactly this and offers four patterns:

- **Don't let advisories fail CI.** cargo-deny-action's "Recommended pipeline if using advisories, to avoid sudden breakages" runs advisories in their own matrix leg with `continue-on-error` "# Prevent sudden announcement of a new advisory from failing ci".
- **Audit on a schedule, not per pull request.** rustsec/audit-check's scheduled mode creates issues: "The action does not raise issues when it is not triggered from a "cron" scheduled workflow".
- **Compare against the base.** `dependency-review-action` flags only versions a pull request introduces (public repositories only). OSV-Scanner's pull-request workflow "compares a vulnerability scan of the target branch to a vulnerability scan of the feature branch".
- **Dependabot alerts are not a check at all.** They evaluate the default branch when the Advisory Database or the dependency graph changes, and "Security updates still open immediately" as pull requests (changelog, 14 July 2026).

**Pushes that carry a secret.** Push protection for users is on by default for public repositories, so a Run whose commit contains a recognised secret will have its `git push` refused. That is a Failed run, not a leak (inference).

## 5. Disclosure

### 5.1 How others disclose what their tools find

| Who | Deadline | What is published, and when | AI-specific rules |
|---|---|---|---|
| Google Project Zero ([policy](https://projectzero.google/vulnerability-disclosure-policy.html); vendor docs) | 90 days, plus up to 14 days' grace; 7 days if exploited in the wild | Details 30 days after a patch, or at day 90. Since July 2025, "within approximately one week of reporting a vulnerability to a vendor, we will publicly share that a vulnerability was discovered" (vendor, product, dates), but "no technical details, proof-of-concept code, or information that we believe would materially assist discovery … until the deadline" ([2025 blog](https://googleprojectzero.blogspot.com/2025/07/reporting-transparency.html)) | "Google Big Sleep … will also be trialling this policy" |
| Anthropic, for vulnerabilities Claude finds ([CVD policy](https://www.anthropic.com/coordinated-vulnerability-disclosure), updated 6 March 2026; vendor docs) | 90 days or a patch, "whichever comes first"; 14-day extension; 7 days if actively exploited | "Once a patch is available, we would generally wait 45 days before publishing full technical details" | "Every report we send generally reflects a finding that a human security researcher has reviewed and confirmed. Reports originating from AI-powered discovery are clearly labeled as such"; no large batches to one project without agreeing a pace. Mythos Preview's unpatched findings were published only as SHA-3 hashes, to be opened "no later than 90 plus 45 days after we report" |
| OpenAI ([outbound policy](https://openai.com/policies/outbound-coordinated-disclosure-policy/), read from a Wayback copy of 8 July 2026; vendor docs) | "We do not commit to strict publication timelines" | "Discreet by default: Initial disclosures are private. Public disclosures usually occur only after explicit vendor or open source maintainer consent" | "Where a vulnerability is discovered by an automated system, a security engineer reviews the disclosure before it is released" |
| Cloudflare | No outbound or AI-specific policy found (background agent) | Its Glasswing findings were "triaged, validated, and remediated where action was needed under Cloudflare's formal vulnerability management process"; critical findings were "fully patched in production in 5 days" | — |

Two things recur. A human reviews every AI-found report before it goes out, and technical detail waits for a patch to be deployable, not merely written.

### 5.2 What GitHub's API can and cannot do unattended

Repository security advisories (REST reference, API version 2026-03-10, and the maintainer docs; vendor docs). On thirdshift's machine the `gh` token belongs to the repository owner, which is the administrator these endpoints require.

| Step | API? | What the docs say |
|---|---|---|
| Create a draft advisory | Yes: `POST /repos/{owner}/{repo}/security-advisories` | "the authenticated user must be a security manager or administrator of that repository"; classic tokens "need the repo or repository_advisories:write scope" |
| Who can see a draft | — | Security managers, administrators, and collaborators added to that advisory |
| Create a temporary private fork | Yes: `POST …/security-advisories/{ghsa_id}/forks` | "Forking a repository happens asynchronously. You may have to wait up to 5 minutes"; a fine-grained token needs Administration write as well |
| Push a fix branch to the fork | With git, per the docs' human steps | Whether a token-driven push or pull request into the fork works is not documented (background agent's flag) |
| Run CI on the fix | **No** | "To keep information about vulnerabilities secure, integrations, including CI, cannot access temporary private forks", and "status checks do not run on pull requests in temporary private forks" ([collaborate in a fork](https://docs.github.com/en/code-security/tutorials/fix-reported-vulnerabilities/collaborate-in-a-fork)) |
| Merge the fix | **No API.** The advisory REST API has eight endpoints and none merges | "You cannot merge individual pull requests in a temporary private fork. Instead, you merge all open pull requests at once", with the **Merge pull request(s)** button on the advisory page. "GitHub won't enforce any of the protection rules that you may have set up", and only one pull request may target `main` |
| Publish | Yes: `PATCH …/{ghsa_id}` with `state: published` | "Publishing a security advisory deletes the temporary private fork." Afterwards "**Anyone** can see the current version of the advisory data" |
| Request a CVE | Yes: `POST …/{ghsa_id}/cve` | "You may request a CVE for public repositories, but cannot do so for private repositories." Requesting one does not publish the advisory; GitHub reviews it, usually within 72 hours |
| Reach dependents | Automatic after review | "GitHub will review each published security advisory, add it to the GitHub Advisory Database, and may use the security advisory to send Dependabot alerts", for ecosystems the dependency graph supports |

So the API can do everything **except** test and merge a fix in private. An unattended flow could create a draft advisory as a private record (and even a private fork as a holding place), but a fix developed there gets no CI, and only a human in the browser can merge it.

### 5.3 Code scanning alerts as a private channel

- **Uploading.** `POST /repos/{owner}/{repo}/code-scanning/sarifs` takes any tool's SARIF. It is free on public repositories and unavailable on user-owned private ones (§4.1).
- **Who sees what.** "Anyone with read permission for a repository can see code scanning annotations on pull requests", but "You need write permission to view a summary of all the alerts for a repository on the **Security** tab" ([assess alerts](https://docs.github.com/en/code-security/how-tos/manage-security-alerts/manage-code-scanning-alerts/assess-alerts)). "If you upload to a pull request, for example `--ref refs/pull/42/merge` … the results appear as alerts in a pull request check" (REST reference).
- **So, on a public repository:** SARIF uploaded against the default branch stays visible only to people with write access, while SARIF uploaded against a pull request becomes public annotations.
- **Unconfirmed:** whether a default-branch alert later shows on a public fixing pull request. The docs show an alert in a pull request only when all its lines are in the diff, which describes new alerts; the fixing case is not described **[needs live check]**.
- **Dismissing.** Alerts can be dismissed through the API (`PATCH …/code-scanning/alerts/{n}`, reasons `false positive`, `won't fix`, `used in tests`, `mitigated`).

### 5.4 What public artefacts reveal, and the norm for a solo maintainer

**The fix itself discloses.**

- Anthropic: "the patch itself is a roadmap to the bug". Its N-day exploits "were written fully autonomously, starting from just a CVE identifier and a git commit hash … which has historically taken a skilled researcher days to weeks per bug" ([red.anthropic.com](https://red.anthropic.com/2026/mythos-preview/), 7 April 2026; vendor research).
- OpenSSF: "attackers can usually review changes made to software … and easily determine an attack. Thus, withholding detailed information can only be helpful for a few days at most" ([maintainer guide](https://github.com/ossf/oss-vulnerability-guide/blob/main/maintainer-guide.md); first-party project docs).
- GitHub: "You can't just drop a fix in a public pull request and hope no one notices. If attackers spot the change before the fix is officially released, they can exploit it before users can update" ([GitHub blog](https://github.blog/security/vulnerability-research/a-maintainers-guide-to-vulnerability-disclosure-github-tools-to-make-it-simple/), March 2025; vendor blog).

**But silence also harms.** GitHub: "It is not uncommon to see cases where a recognized security issue is fixed in the current development branch of a project, but the commit or subsequent release is not explicitly marked as a security fix or release. This can cause problems with downstream consumers." It asks maintainers to "explicitly mention that the issue is a security vulnerability in the release notes" and to "Aim to publish a fix as soon as you can" ([coordinated disclosure](https://docs.github.com/en/code-security/concepts/vulnerability-reporting-and-management/coordinated-disclosure)). OpenSSF: "'Security through obscurity' is a weak defense". The same GitHub post allows that "Some vulnerabilities are minor contained issues that can be patched quietly."

**The recommended path for a maintainer fixing their own repository.** OpenSSF: "If you test a particular patch in public, an observant attacker may see and exploit the vulnerability before you're able to issue a patch", so "**our recommendation is that you typically use the private development features to generate your patch there if you are a project using GitHub**". The exception: "If the vulnerability is _already_ publicly known and widely exploited, there's no advantage to trying to keep things private." Embargoed notification of other vendors "is probably not necessary" without "a significant vendor ecosystem". Neither GitHub nor OpenSSF says how to word commit messages or whether to commit a test that reproduces the vulnerability (neither the background agent's search nor my reading of those pages found any).

**What each public artefact would carry in thirdshift today** (inference from the Session prompts and CONTEXT.md):

- **An Architect-style plan issue.** A full description, published before any fix exists. The Architecture review publishes to the issue tracker by design.
- **A Run's pull request.** A title, a body with the change summary and "Unaddressed findings", commits, and any regression test, all public from the push. In a Merge run, which every Run is under this User config (`merge.always = true`), the pull request stays open until CI is green, often minutes, sometimes longer.
- **The Run notification.** Private to the configured address, but sent through Resend, a third party.
- **Exposure differs by repository.**
  - keeplore, chart35 and cascade's web build deploy on every push to `main` (their deploy workflows). For them the window that matters is from the pull request's first push to the deploy, plus however long the fix sits unmerged.
  - thirdshift and vaulted-agent are released binaries, and cascade's native apps go through app releases. Their users stay exposed until they update, so the public fix starts a longer clock.

## 6. Cost

### 6.1 Published figures

| What | Figure | Source (grade) |
|---|---|---|
| Claude Code's managed Code Review (general review with verification, hosted) | "Each review averages $15-25 in cost", "completing in 20 minutes on average"; billed as usage credits, which do "not count against your plan's included usage" | [code-review docs](https://code.claude.com/docs/en/code-review) (vendor docs) |
| Claude Code Ultrareview (hosted) | "typically $5 to $25 in usage credits" after three free runs | [ultrareview docs](https://code.claude.com/docs/en/ultrareview) (vendor docs) |
| Claude Mythos Preview on OpenBSD | "Across a thousand runs through our scaffold, the total cost was under $20,000 and found several dozen more findings"; the run that found the headline bug "cost under $50" | [red.anthropic.com](https://red.anthropic.com/2026/mythos-preview/) (vendor research) |
| Claude Mythos Preview on FFmpeg | "several hundred runs over the repository, at a cost of roughly ten thousand dollars" | same |
| Cloudflare's harness | No money or token figure. About 30,000 lines: 3–4 hours of hunting, 3 hours of triage, about 14 hours to fix pull requests; worst full scan "just over 14 hours"; 50–200 workers | Cloudflare blog, 18 June 2026 (vendor blog) |
| open-code-review | 352K–422K tokens and 1–3 minutes per review with Claude Opus 4.6/4.8 or GPT-5.5, against 2.1M–5.7M tokens and 6–13 minutes for Claude Code on the same benchmark | README (vendor research) |
| DARPA AIxCC final | "an average cost per competition task of about $152"; patches "in an average of 45 minutes" | [DARPA](https://www.darpa.mil/news/2025/aixcc-results) (vendor docs) |
| OpenAI Codex Security (hosted) | Token-billed since 1 October 2026; "not covered by your plan's included usage allowance"; $500 of free credits | [FAQ](https://learn.chatgpt.com/docs/security/faq) (vendor docs) |
| openai/codex-security CLI | `--max-cost`; estimates at standard API prices | [cli.md](https://github.com/openai/codex-security/blob/main/sdk/typescript/docs/cli.md) (vendor docs) |
| anthropics/claude-code-security-review, `/security-review`, cloudflare/security-audit-skill, vulnhuntr | No figure published | repositories |

### 6.2 Local baseline: what an Architecture review and an implement session cost today

**Method.** Every `<kind>: session ended after <time>: <n> turns, $<x> at API prices` line in the Command logs under `~/.thirdshift/logs/JacobStephens2/*/commands/` (493 lines, Command logs from 3 to 5 October 2026), cross-checked against the `result` event of each Session log (690 files, 25 September to 5 October). Every session with a dollar figure ran on Claude Code with `claude-opus-5-5` (the User config sets Effort `medium`). The time is wall-clock, as thirdshift measures it. "At API prices" is list price: the sessions report `apiKeySource: "none"` and `costBasis: "list"`, so they ran on a subscription, and the dollars measure usage, not a bill.

| Session | n | Median wall time (IQR; p90) | Median cost at API prices (IQR; p90; mean) | Median turns |
|---|---|---|---|---|
| Architecture review | 204 | 6.7 min (5.5–8.9; 13.3) | $2.84 ($2.29–$3.58; $4.40; $3.00) | 21 |
| Implement session | 253 | 16.1 min (9.9–22.7; 28.8) | $2.78 ($2.31–$3.72; $4.32; $3.08) | 20 |
| Spec review | 24 | 9.5 min | $2.34 | 31 |
| Repair (Session logs) | 53 | — | $0.41 | 9 |

- The 204 Architecture reviews ran on all seven repositories (chart35 60, keeplore 40, thereish 30, thirdshift 28, vaulted-agent 25, muxboard 12, cascade 9) before Architect runs were limited to thirdshift. Over those three days they cost $611 at list price, and the 253 implement sessions $779.
- The Session logs agree on cost (Architecture review median $2.87 over 207, implement $2.73 over 384), but Claude's own `duration_ms` is shorter than the wall clock (one chart35 implement session: 169,689 ms against 12 min 16 s), so the table uses the Command logs' wall time.
- **Codex.** One Codex session is logged (Run #345, `gpt-6.1-sol` at `xhigh`, 5 October): 10 min 51 s, 2,381,311 input tokens (2,319,104 cached) and 11,297 output tokens. Codex reports tokens, not dollars, so there is no Codex baseline in dollars.

### 6.3 What that implies (inference)

- **A diff security review** inside a Run, as one more fresh-context sub-agent reading the diff, should cost the same order as the existing Standards or Spec sub-agent. The two published hosted diff reviewers that verify findings cost $5–25 (Ultrareview) and $15–25 (Code Review) per review, that is 2–9 times a whole implement session at list price.
- **A whole-codebase pass** is a different order. Anthropic's runs cost tens of dollars each, and it needed hundreds to a thousand of them per large target. Cloudflare runs 50–200 workers for hours per repository. A `quick` run of the Cloudflare skill on a small repository spends at least four reconnaissance agents, one hunter per ledger unit, one critic and one verifier per candidate; at this machine's ceiling of about two building Runs at once, it would hold a slot for hours, not the 7 minutes of an Architecture review.
- **Subscription limits, not dollars, are the binding constraint for local runs**, since the Harness sessions run on a subscription. The hosted products (Code Review, Ultrareview, Codex Security Cloud) bill outside the plan's included usage.

## 7. Risks specific to an unattended factory doing security work

### 7.1 Prompt injection through what the audit reads

**The vendors say their own tools are not hardened against it:**

- anthropics/claude-code-security-review: "This action is not hardened against prompt injection attacks and should only be used to review trusted PRs."
- Claude Security plugin, SECURITY.md: "The code you scan is trusted … The plugin does not attempt to stop a hostile repository from influencing a scan."
- openai/codex-security, SECURITY.md: "Prompt injection or a model misusing access it already has does not qualify" as a vulnerability, and "Scan only repositories you trust."

**What a headless Claude session obeys in a repository.** Under `claude -p`, which "never shows" the trust dialog, a repository's hooks, `env` block, `apiKeyHelper` and skill `allowed-tools` are "Used", and its `.mcp.json` servers are "Connected without asking, approved or not". The documented mitigations are reading only user settings (`--setting-sources user`), bare mode (`--bare`), and a per-run setting that turns the project's hooks off ([permissions, "What runs before you trust a folder"](https://code.claude.com/docs/en/permissions); vendor docs).

- This matters because thirdshift runs `claude -p --permission-mode auto` in worktrees of the Base branch, and Codex with `--dangerously-bypass-approvals-and-sandbox` ([src/session.rs](../../src/session.rs); [ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)).
- The machine's `gh` token has the `repo` scope (observation), which is enough to create issues, open pull requests, and publish advisories.

**Measured attacks on AI reviewers and detectors:**

- **Iterative refinement against real reviewers.** Against Claude Code and CodeRabbit, an LLM-refined attack re-introduced a known CVE past review "in 32/33 (97%) cases", while direct "biasing attempts raise suspicions" ([arXiv:2603.18740](https://arxiv.org/abs/2603.18740), preprint).
- **Revising code from the reviewer's own feedback.** AFCRA did this over five rounds and got a vulnerable change approved by a Claude Code reviewer on Sonnet 5 in 33 of 159 cases (20.8%), and by Codex on GPT-5.5 in 25 of 159 (15.7%) ([arXiv:2610.05399](https://arxiv.org/abs/2610.05399), 4 October 2026, preprint).
  - Of the 33 attacks that got past Claude Code, a static scan caught 20, a root-cause check 23, and a reviewer told to run the proof of concept 29.
  - The six reviewer configurations approved all 198 genuine historical fixes: no false rejections.
- **Comments as the injection vector.** Adaptive comment injection flipped 91–100% of true detections in four LLM detectors. Prompt-level defences "provide little protection and can even increase attack success". Removing unverifiable comments before detection "consistently achieves the strongest protection" ([ALIBI, arXiv:2607.24964](https://arxiv.org/abs/2607.24964), preprint; small and older detectors, not Claude Code or Codex).
- **Malicious issues.** Across 4,176 runs of Cursor, Claude Code and Codex Desktop, "66.5% of the malicious issues … penetrate all the guardrails" ([IssueTrojanBench, arXiv:2607.20759](https://arxiv.org/abs/2607.20759), preprint). A Pickup run takes issues; an audit reads them as context.
- **A built-in exclusion hides a whole class.** `/security-review` tells its filter that "Including user-controlled content in AI system prompts is not a vulnerability" (§3.1). For thirdshift and vaulted-agent, whose job is to pass text to agents, that hides the class of bug most likely to matter.

### 7.2 A fixer "fixing" a non-vulnerability

- Models report bugs whether or not there are any: "Ask a model to find bugs, and it will find them, whether the code has any or not" (Cloudflare, Glasswing post).
- Out of the box on 11 Python web applications, Claude Code (Sonnet 4) had "86% false positive rate" and Codex (o4-mini) 82%. On SQL injection Claude Code was right on 2 of 38. "Three identical runs produced 3, 6, and then 11 distinct findings", and "the model would often suggest parameterizing a SQL query that was already safe" ([Semgrep, 2 September 2025](https://semgrep.dev/blog/2025/finding-vulnerabilities-in-modern-web-apps-using-claude-code-and-openai-codex/); vendor research, by a vendor of a competing tool).
- On real C/C++ projects, Claude Code (Sonnet 4.6) had a sampled false-discovery rate of 74.07%, against 9.09% for Codex (GPT-5.4), which reported only 2.44 warnings per project on average ([arXiv:2601.19239](https://arxiv.org/abs/2601.19239) v2, Table VII; preprint; small samples).
- An unattended fixer acting on unverified findings would make such changes. Most are harmless "guardrail" edits, but each one is churn in a Merge run nobody reviews, and some break behaviour (§7.3).

### 7.3 A security fix that breaks behaviour

- PoC-passing patches are often wrong: PatchBench's 1.83× inflation, and the SoK's 37.7% semantically wrong Claude Code patches (§1.8).
- Cloudflare saw self-written patches that "fixed the original bug while quietly breaking something else the code depended on", and requires a fail→pass test, regression tests, and human review before any fix lands.
- The typical failure is a guard that suppresses the symptom. The SoK lists "inserting a hard-coded iteration limit" for timeouts and "defensive patching in parsing logic". In a Merge run the only check is the repository's own CI.

### 7.4 A public fix that reveals the vulnerability before it is deployed

- For 18 Firefox security patches, Anthropic gave Claude Mythos Preview "the public diff (with the maintainer's regression test stripped out)", the component name, Mozilla's severity rating and two sanitizer builds, but not the advisory or the reporter's reproducer. It "wrote its first working exploit in just under one hour, and ultimately created eight different exploits in roughly 12 hours." Its conclusion: "'N-day' has become dangerously misleading. N-hour is closer to the reality we now operate in" ([red.anthropic.com/2026/n-days](https://red.anthropic.com/2026/n-days/), 8 June 2026; vendor research).
- Before models, the window was already short: Mandiant measured an average time to exploit of "five days" in 2023, down from 63 days in 2018–2019. "Twelve percent (5) of n-days were exploited within one day, 29% (12) were exploited within one week" ([Google Cloud](https://cloud.google.com/blog/topics/threat-intelligence/time-to-exploit-trends-2023); vendor research).
- Today every part of a Run is public on the five public repositories: the issue, the branch, the pull request, its body with "Unaddressed findings", and its tests (§5.4).
- A regression test that reproduces the vulnerability is the part Anthropic had to strip to make the exercise fair (inference: committing one publishes a ready-made trigger).

### 7.5 Other risks the sources flag

- **Agents fake their evidence.** "The agent will edit the source code so its own exploit works, then triumphantly report the bug it just created. It will write a test that proves something entirely tautological" (Cloudflare). Hence PoCs that run "against the original, untouched codebase".
- **Refusals are inconsistent.** Mythos Preview's "organic refusals aren't consistent - the same task, framed differently or presented in a different context, could produce completely different outcomes" (Cloudflare). Codex Security warns that "some cybersecurity requests and protected findings require Trusted Access for Cyber approval". An unattended pass must treat a refusal as a failed pass, not a clean one (inference).
- **Volume overwhelms the humans.** Anthropic paces disclosures because "maintainers have been facing a deluge of low-quality, AI-generated bug reports" (Glasswing update). curl ended its bug bounty on 31 January 2026 after its confirmed rate "plummeted to below 5%" from "somewhere north of 15%" ([Stenberg](https://daniel.haxx.se/blog/2026/01/26/the-end-of-the-curl-bug-bounty/); anecdote). Here the maintainer receiving the findings is the user, so the Day shift is the bottleneck.
- **Private channels have third parties.** A Run notification passes through Resend, and Codex Security's and Claude Security's hosted scans run on the vendors' infrastructure.

## 8. What the evidence does not tell us

- **How precise either named project is.**
  - Cloudflare publishes no precision for the skill, only "roughly half" of repeated runs' findings in one run, and funnel numbers for the harness it grew into.
  - open-code-review's precision is Alibaba's own, on general review rather than security, measured on old versions.
  - Neither has been evaluated by a third party.
- **Recall on real code.** Every recall figure here comes from seeded or previously known vulnerabilities. Cloudflare's view: "any claimed recall number is entirely speculative". One run finds only part of what repeated runs find, and identical runs disagree (§2.2, §7.2).
- **How today's models do on this work.** There are security-review numbers for Sonnet 5 and GPT-5.5 under attack (AFCRA), patching numbers for Opus 4.8 and GPT-5.6 Sol (PatchBench), and SusVibes rows for Opus 4.8 and GPT-5.5. None measures `claude-opus-5-5` at medium or `gpt-6.1-sol` at xhigh, the configurations thirdshift runs, as a security reviewer.
- **Languages.** Almost all of the detection, repair and attack evidence is C/C++, Java, Python or JavaScript. Little covers Rust, PHP, Swift, Kotlin or C#, and none covers end-to-end encrypted TypeScript like chart35.
- **Whether a Security axis would lower the rate of vulnerable merges here.**
  - The best evidence, a security skill cutting reintroduction by 13.8 points, is one preprint's replay on one model and on vibe-coded web apps.
  - Field studies of agent pull requests disagree on whether agent code is worse at all, and use scanners, not exploits (§1.7).
- **Several headless behaviours** **[needs live check]**:
  - whether `claude -p "/security-review"` expands in a Run's worktree;
  - whether the security-guidance plugin's background reviews survive a headless session;
  - whether `codex exec` honours the Cloudflare skill's parallel sub-agents.
- **Two GitHub behaviours the docs leave open:**
  - whether a token can push to and open pull requests in a temporary private fork;
  - whether a default-branch code scanning alert appears on the public pull request that fixes it.
- **Cost of a Cloudflare-skill run on these repositories.** No figure is published, and the local baseline measures different work.
- **Vendor numbers without denominators.** Copilot Autofix's "two-thirds", CodeMender's 72 fixes, and Codex Security's "more than 50%" fewer false positives have no published methods. Anthropic's 90.6% and 92.7% are reviewed by firms Anthropic chose, over findings its own pipeline had already filtered.

## 9. Implications for thirdshift (recommendation, not evidence)

This section is my recommendation. It rests on the evidence above but goes beyond it. The options are not exclusive. They are ordered from cheapest to heaviest, and the questions #427 leaves for triage are answered at the end.

**Where thirdshift is today:**

- No Run looks for vulnerabilities, and every Run is a Merge run on this machine.
- No CI workflow in the seven repositories runs a security tool, and GitHub's free security features are off on nearly all of them (§4.1).
- Every artefact a Run leaves on the five public repositories is public: issue, branch, pull request, "Unaddressed findings", tests (§5.4).
- The machine has no OS sandbox an agent can create (§2.1).

### Options

1. **Switch on GitHub's free features first.** These are settings, not code. Deterministic, private by default, and no model cost.
   - **Dependabot alerts and security updates on all seven repositories**, the private two included.
     - *For:* the only free feature that covers chart35 and clave. Alerts are private, and GitHub "never publicly discloses vulnerabilities for any repository". The advisories they act on are already public.
     - *Against:* Dependabot opens pull requests, not issues, so the factory never picks them up. Merging them stays with the Day shift, or with a merge rule the user sets.
   - **Secret scanning and push protection** on thirdshift, cascade and keeplore (free on public repositories, already on for muxboard and vaulted-agent).
   - **Private vulnerability reporting** on the five public repositories. It gives outside reporters a private channel. It is also the channel any AI-found report about these repositories should arrive through.
   - **CodeQL default setup** on thirdshift, cascade, muxboard and vaulted-agent, and Copilot Autofix with it. Not keeplore: CodeQL has no PHP.
     - *For:* pull-request checks are diff-scoped, so a CodeQL release does not turn every open pull request red. Rust is generally available, and Actions minutes are free on public repositories.
     - *Against:* its failing check is a check run whose explanation lives in annotations, not in the Actions log the CI-fix Repair is told to read, and the Check re-run cannot apply to it (§4.3). So `ci_fix_repair` in [src/prompt.rs](../../src/prompt.rs) should name the check-run annotations or the code scanning alerts API.
     - *Against:* its annotations on a public pull request are public, though they show only code that pull request itself added.
     - *Against:* CodeQL's precision on these repositories is unmeasured.
2. **Add deterministic dependency checks only in shapes that cannot red the base branch.**
   - On pull requests: checks that compare against the base. `dependency-review-action` on the public repositories; OSV-Scanner's pull-request mode on all seven, private included (with its SARIF upload off on the private two, where code scanning is unavailable; inference). Optionally Semgrep CE with `--baseline-commit`, and gitleaks.
   - The full audits (`cargo deny check advisories` or `cargo audit`, `composer audit`, `pip-audit`, `npm audit`) belong in a scheduled job on the default branch. That is what rustsec/audit-check's cron mode and cargo-deny-action's `continue-on-error` advice are for. Run on every pull request, a new advisory fails every Run at once, and thirdshift does not see it as an Inherited failure until the base branch re-runs the check (§4.3).
   - *For:* seconds of CI and no model. The existing CI-fix Repair already handles a red check whose log names the problem.
   - *Against:* deterministic tools find dependency advisories and simple patterns, not the logic flaws agents write (§1.7).
3. **Add a Security axis to each Run's review (shape 2).** This is the cheapest model-based option, and the one the evidence on agent-written code supports most directly.
   - Current agents leave about 60% of their functionally correct solutions insecure on SusVibes. A security skill in the harness cut reintroduced vulnerabilities by 13.8 points, while "review your changes" cut them by 1.9 (§1.7).
   - **Shape.** A third fresh-context sub-agent beside Standards and Spec in `thirdshift-code-review`, reporting separately like the other two. Its brief could be adapted from the MIT `/security-review` prompt (§3.1): diff-only, high confidence, with a refuting sub-agent per finding. Drop the exclusions that do not fit these repositories, prompt injection and unsafe Rust above all. Borrow per-language rule text from open-code-review's Apache-2.0 security sections for PHP, Python, TypeScript, Rust and Swift, with its notice kept.
   - **Evidence rule.** The evidence on fixes argues for one: the author fixes a security finding only when it can show the problem with a test or a concrete trace, and then checks the fix with that test (§1.8, §7.2–7.3). This matches the evidence-based adjudication recommended in [multi-family-review.md](multi-family-review.md).
   - **Disclosure rule.** A security finding the author does not fix must not go into the public pull request's "Unaddressed findings". It goes to the Run notification, or a draft advisory, and holds the Merge run for the Day shift (§5.4). A finding the author does fix in its own diff reveals little, because the vulnerable code never reached the base branch (inference).
   - **Cost.** One more sub-agent, the same order as the Standards and Spec ones. Unmeasured (§6.3).
   - *Against:* false positives become churn in Merge runs nobody reviews (§7.2). Adaptive attacks get past AI reviewers 16–97% of the time (§7.1), so this axis lowers risk without closing it.
   - **Ready-made alternatives**, each with a catch:
     - `/security-review`: headless untested; needs `origin/HEAD`; carries the exclusions above.
     - `codex exec review`: takes no custom instructions together with `--base`.
     - The Codex Security CLI's `scan --diff`: Apache-2.0, validates findings, `--fail-on-severity`; needs a ChatGPT login or an API key, plus Node and Python.
     - Claude Code's hosted Code Review: $15–25 a review, billed outside the plan.
4. **Add a security pass (shape 1) only as a lead generator first, and only where findings can stay private.**
   - **Tool.** The Cloudflare skill is the best documented starting point, MIT and model-agnostic. On this machine it can confirm nothing until a sandbox exists, which takes a one-time admin step (an AppArmor profile that lets bubblewrap create namespaces, or Podman or Docker). Until then every lead is `needs_validation` (§2.1).
   - **Scale.** A `quick` or scoped profile on one repository at a time, rarely, such as weekly. It yields to Pickup runs and Architect runs and holds one of the machine's two building slots for hours (§6.3).
   - **Where its findings go.** Not an issue: an Architect-style plan issue would publish an unfixed vulnerability. Instead:
     - one draft repository security advisory per confirmed finding on the public repositories, which the API can create and only administrators can see;
     - the Run notification, which also covers chart35 and clave, where there are no advisories;
     - or SARIF uploaded against the default branch, whose alerts only people with write access can see (§5.2–5.3).
   - **What it fixes unattended.** Nothing at first. Later, perhaps only `confirmed` findings with a working proof-of-concept test against untouched code, the bar Cloudflare, Anthropic and Google's CodeMender all apply before trusting a finding (§1.2, §2.2, §3.2). `needs_validation` findings never.
   - **How fixes would land.** GitHub's private route cannot be automated: no CI runs in a temporary private fork, and its merge is a button in the web UI (§5.2). An unattended fix is therefore a public pull request, which discloses the bug when pushed (§7.4).
     - For keeplore, chart35 and cascade's web build, which deploy on every push to `main`, a fast Merge run is the shortest exposure.
     - For thirdshift, vaulted-agent and cascade's native apps, users stay exposed until they update. There the Day shift should choose the moment and publish the advisory with the release.
5. **Measure before trusting either shape unattended** (inference, after [multi-family-review.md](multi-family-review.md) §9 option 5).
   - Run the Security axis, and any pass, in report-only mode for a few weeks, with findings sent privately.
   - Have the Day shift grade each finding: real, not real, or hardening.
   - Compute precision on these repositories and languages. No published number covers Rust, PHP or Swift code like this (§8).

### The triage questions in #427

- **Shape 1, shape 2, or both, and which first?** The evidence supports the order of the options above: GitHub's free features and base-safe CI checks, then shape 2 in report-only mode, then shape 1 as a lead generator. Agents introducing vulnerabilities into unreviewed Merge runs is the measured risk; a backlog of old vulnerabilities is an unmeasured one.
- **Adapt or write?**
  - For shape 2, write a small `thirdshift-` security axis, borrowing MIT and Apache-2.0 text with notices kept. The candidate tools either need a separate API key and runtime (open-code-review, Codex Security) or carry exclusions and an untested headless path (`/security-review`).
  - For shape 1, adapting the Cloudflare skill is reasonable. MIT allows it with its notice. It needs three headless edits, the sandbox decision, and a private publishing step.
  - Avoid CC-BY-SA, AGPL, Semgrep-rules-licensed or proprietary text in the embedded skills (§3.2).
- **Where do findings go?** Never a public issue or a public pull request body while unfixed: a draft advisory, the Run notification, or default-branch SARIF. Note that the Run notification passes through Resend.
- **Is a `needs_validation` finding ever fixed unattended?** No published practice does so (§2.2, §5.1).
- **Labels and yielding for a pass.** Like an Architect run, it should skip while another pass or a Ready issue exists. But its "waiting for triage" marker cannot be a public `needs-triage` issue. It needs a private marker, such as an open draft advisory or a local record (inference).
- **Cost.**
  - A diff security axis: on the order of one more sub-agent per Run.
  - A whole-codebase pass: hours and many sub-agent sessions per repository, against an Architecture review's median of 6.7 minutes and $2.84 at list price (§6).
  - Hosted reviewers cost $5–25 per review, outside the subscription.

## Sources

All accessed 5–6 October 2026. "Read" means the full text or file was read, by me or by a background agent as noted in Method. Grades as in the table above.

**The two projects (vendor docs and vendor blog)**

- cloudflare/security-audit-skill, MIT, at `c1c8a8c` (14 September 2026): https://github.com/cloudflare/security-audit-skill
  - Read: `README.md`, `LICENSE`, and in `skills/security-audit/`: `SKILL.md`, `RECONNAISSANCE.md`, `HUNTING.md`, `ATTACK-CLASSES.md`, `VALIDATION-AND-REPORTING.md`, the ten domain companions, `report-schema.json`, `validate-findings.cjs`, `validate-coverage-ledger.cjs`.
  - Third-party pull requests and issues: [#43](https://github.com/cloudflare/security-audit-skill/pull/43), [#58](https://github.com/cloudflare/security-audit-skill/pull/58), [#60](https://github.com/cloudflare/security-audit-skill/pull/60), [#64](https://github.com/cloudflare/security-audit-skill/issues/64).
- Cloudflare blog (vendor blog):
  - Dan Jones, Alexandra Godoi, Grant Bourzikas, "Build your own vulnerability harness" (18 June 2026): https://blog.cloudflare.com/build-your-own-vulnerability-harness/
  - Grant Bourzikas, "Project Glasswing: what Mythos showed us" (18 May 2026): https://blog.cloudflare.com/cyber-frontier-models/
  - Rohit Chenna Reddy, Chase Catelli, Dan Jones, "Defend against frontier cyber models" (9 June 2026): https://blog.cloudflare.com/frontier-model-defense/
- alibaba/open-code-review, Apache-2.0, at `182898c` (5 October 2026), release v1.12.12: https://github.com/alibaba/open-code-review
  - Read: `README.md` and its benchmark image `imgs/benchmark-en.png`; `skills/open-code-review/SKILL.md` and `skills/open-code-review-delegate/SKILL.md`; `action.yml`; `package.json`.
  - Also read: `internal/config/rules/system_rules.json` and the rule documents; `internal/config/template/`; and in `pages/src/content/docs/en/`: `architecture.md`, `cli-reference.md`, `telemetry.md`, `integrations/delegate.md`.
- AACR-Bench: Zhang et al., [arXiv:2601.19494](https://arxiv.org/abs/2601.19494) (preprint), and https://huggingface.co/datasets/Alibaba-Aone/aacr-bench

**Papers: detection and auditing**

- Ding et al., "Vulnerability Detection with Code Language Models: How Far Are We?" (PrimeVul), [arXiv:2403.18624](https://arxiv.org/abs/2403.18624), ICSE 2025
- Ullah et al., "LLMs Cannot Reliably Identify and Reason About Security Vulnerabilities (Yet?)" (SecLLMHolmes), [arXiv:2312.12575](https://arxiv.org/abs/2312.12575), IEEE S&P 2024
- Yildiz et al., "Benchmarking LLMs and LLM-based Agents in Practical Vulnerability Detection for Code Repositories" (JitVul), [arXiv:2503.03586](https://arxiv.org/abs/2503.03586), ACL 2025 (2025.acl-long.1490)
- Ahmed et al., "SecVulEval", [arXiv:2505.19828](https://arxiv.org/abs/2505.19828), preprint
- Li, F. et al., "LLM-based Vulnerability Detection at Project Scale: An Empirical Study", [arXiv:2601.19239](https://arxiv.org/abs/2601.19239) v2 (25 September 2026), preprint
- Guo et al., "RepoAudit", [arXiv:2501.18160](https://arxiv.org/abs/2501.18160), ICML 2025 (PMLR 267)
- Li, Z. et al., "IRIS", [arXiv:2405.17238](https://arxiv.org/abs/2405.17238), ICLR 2025
- Lyu et al., "RECEIPT", [arXiv:2607.18575](https://arxiv.org/abs/2607.18575), preprint
- Xiong et al., "Sifting the Noise", [arXiv:2601.22952](https://arxiv.org/abs/2601.22952), ISSTA 2026 (abstract only)
- Wang, Z. et al., "CyberGym", [arXiv:2506.02548](https://arxiv.org/abs/2506.02548), ICLR 2026 (Oral)
- Zhang, A. K. et al., "BountyBench", [arXiv:2505.15216](https://arxiv.org/abs/2505.15216), NeurIPS 2025 Datasets and Benchmarks
- Simecek et al., "HoF-Bench", [arXiv:2607.27030](https://arxiv.org/abs/2607.27030), preprint (by the vendor AISLE)
- Yu et al., "An Insight into Security Code Review with LLMs", [arXiv:2401.16310](https://arxiv.org/abs/2401.16310), preprint (journal revision)
- Charoenwet et al., "AgenticSCR", [arXiv:2601.19138](https://arxiv.org/abs/2601.19138), ASE 2026
- Wang, Y. et al., "PRWeaver", [arXiv:2608.02693](https://arxiv.org/abs/2608.02693), preprint
- Zhang, C. et al., "SoK: DARPA's AI Cyber Challenge (AIxCC)", [arXiv:2602.07666](https://arxiv.org/abs/2602.07666), USENIX Security 2026

**Papers: agent-written code and fixes**

- Zhao et al., "Is Vibe Coding Safe?" (SusVibes), [arXiv:2512.03262](https://arxiv.org/abs/2512.03262) v4, ICML 2026; leaderboard data https://leililab.github.io/susvibes-leaderboard/submissions/index.json (last updated 21 August 2026)
- Vero et al., "BaxBench", [arXiv:2502.11844](https://arxiv.org/abs/2502.11844), ICML 2025 (PMLR 267); leaderboard data https://baxbench.com/static/data/leaderboard_data_none.json and `…_generic.json`
- Deng et al., "Understanding the (In)Security of Vibe-Coded Applications", [arXiv:2606.23130](https://arxiv.org/abs/2606.23130) v4, preprint
- Xia et al., "Do These Violent Delights Have Violent Ends?", [arXiv:2607.09902](https://arxiv.org/abs/2607.09902), preprint (read by a background agent)
- Kraishan, "Not All Agents Are Equal", [arXiv:2609.17598](https://arxiv.org/abs/2609.17598), preprint (read by a background agent)
- Shen et al., "PatchBench", [arXiv:2609.04075](https://arxiv.org/abs/2609.04075), preprint

**Papers: attacks on reviewers and agents**

- Alexopoulos et al., "Measuring and Exploiting Contextual Bias in LLM-Assisted Security Code Review", [arXiv:2603.18740](https://arxiv.org/abs/2603.18740), preprint
- Gong et al., "Adaptive Code Revision Attacks on AI Pull Request Reviewers" (AFCRA), [arXiv:2610.05399](https://arxiv.org/abs/2610.05399), preprint (4 October 2026)
- Wu et al., "ALIBI", [arXiv:2607.24964](https://arxiv.org/abs/2607.24964), preprint
- Singh et al., "IssueTrojanBench", [arXiv:2607.20759](https://arxiv.org/abs/2607.20759), preprint

**Anthropic (vendor research and docs)**

- "Claude Opus 4.6 … zero-days" (5 February 2026): https://www.anthropic.com/research/zero-days
- Firefox collaboration (March 2026): https://www.anthropic.com/news/mozilla-firefox-security
- Claude Mythos Preview (7 April 2026): https://red.anthropic.com/2026/mythos-preview/
- Glasswing initial update (22 May 2026): https://www.anthropic.com/research/glasswing-initial-update
- N-days (8 June 2026): https://red.anthropic.com/2026/n-days/
- Coordinated vulnerability disclosure ledger (counts as of 2 October 2026): https://red.anthropic.com/2026/cvd/
- Coordinated vulnerability disclosure policy (updated 6 March 2026): https://www.anthropic.com/coordinated-vulnerability-disclosure
- Claude Code docs: [commands](https://code.claude.com/docs/en/commands), [errors](https://code.claude.com/docs/en/errors), [headless](https://code.claude.com/docs/en/headless), [skills](https://code.claude.com/docs/en/skills), [permissions](https://code.claude.com/docs/en/permissions), [security-guidance](https://code.claude.com/docs/en/security-guidance), [code-review](https://code.claude.com/docs/en/code-review), [ultrareview](https://code.claude.com/docs/en/ultrareview)
- anthropics/claude-code-security-review (MIT): `.claude/commands/security-review.md`, `README.md`, `claudecode/constants.py`: https://github.com/anthropics/claude-code-security-review
- anthropics/claude-plugins-official at `d4226d0`: `plugins/claude-security/` (`LICENSE`, `SECURITY.md`, `skills/claude-security/jobs/`) and `plugins/security-guidance/`: https://github.com/anthropics/claude-plugins-official
- "Claude Security is now in public beta" (30 April 2026): https://claude.com/blog/claude-security-public-beta

**OpenAI (vendor research and docs; openai.com read from the Wayback Machine)**

- "Introducing Aardvark" (30 October 2025), snapshot 3 October 2026: https://web.archive.org/web/20261003133042/https://openai.com/index/introducing-aardvark/
- "Codex Security: now in research preview" (6 March 2026), snapshot 6 September 2026: https://web.archive.org/web/20260906200205/https://openai.com/index/codex-security-now-in-research-preview/
- Outbound Coordinated Disclosure Policy, snapshot 8 July 2026: https://web.archive.org/web/20260708120133/https://openai.com/policies/outbound-coordinated-disclosure-policy/
- Codex Security FAQ: https://learn.chatgpt.com/docs/security/faq
- openai/codex-security (Apache-2.0): `README.md`, `SECURITY.md`, `sdk/typescript/README.md`, `sdk/typescript/docs/cli.md`: https://github.com/openai/codex-security
- `codex` 0.160.0 `--help`, `review --help`, `features list` on this machine

**Google (vendor research and docs)**

- CodeMender launch (6 October 2025): https://deepmind.google/blog/introducing-codemender-an-ai-agent-for-code-security/
- CodeMender on Google Cloud: https://docs.cloud.google.com/gemini-enterprise-agent-platform/agents/codemender
- OSS-Fuzz and CodeMender auto-patches (29 July 2026): https://blog.google/security/from-finding-to-fixing-reducing-maintainer-burden-with-automated-patches/
- Chrome (30 July 2026): https://blog.google/security/chrome-stronger-with-every-update/
- Project Zero disclosure policy: https://projectzero.google/vulnerability-disclosure-policy.html; Reporting Transparency: https://projectzero.google/reporting-transparency.html and https://googleprojectzero.blogspot.com/2025/07/reporting-transparency.html
- Mandiant, "Time-to-Exploit Trends 2023": https://cloud.google.com/blog/topics/threat-intelligence/time-to-exploit-trends-2023

**GitHub (vendor docs, changelog and blog)**

- Docs: [about GitHub Advanced Security](https://docs.github.com/en/get-started/learning-about-github/about-github-advanced-security), [security features by plan](https://docs.github.com/en/code-security/getting-started/github-security-features), [private repository enablement](https://docs.github.com/en/code-security/reference/code-scanning/troubleshoot-analysis-errors/private-repository-enablement), [CodeQL code scanning](https://docs.github.com/en/code-security/concepts/code-scanning/codeql/codeql-code-scanning), [Copilot Autofix](https://docs.github.com/en/code-security/concepts/code-scanning/autofix-for-code-scanning), [AI Scan](https://docs.github.com/en/code-security/concepts/code-scanning/ai-powered-security-detections), [code scanning alerts](https://docs.github.com/en/code-security/concepts/code-scanning/code-scanning-alerts), [triage alerts in pull requests](https://docs.github.com/en/code-security/how-tos/manage-security-alerts/manage-code-scanning-alerts/triage-alerts-in-pull-requests), [assess alerts](https://docs.github.com/en/code-security/how-tos/manage-security-alerts/manage-code-scanning-alerts/assess-alerts), [push protection](https://docs.github.com/en/code-security/concepts/secret-security/push-protection), [private vulnerability reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository), [dependency review](https://docs.github.com/en/code-security/concepts/supply-chain-security/dependency-review), [Dependabot alerts](https://docs.github.com/en/code-security/concepts/supply-chain-security/dependabot-alerts), [repository security advisories](https://docs.github.com/en/code-security/concepts/vulnerability-reporting-and-management/repository-security-advisories), [collaborate in a temporary private fork](https://docs.github.com/en/code-security/tutorials/fix-reported-vulnerabilities/collaborate-in-a-fork), [publish an advisory](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/fix-reported-vulnerabilities/publish-repository-advisory), [coordinated disclosure](https://docs.github.com/en/code-security/concepts/vulnerability-reporting-and-management/coordinated-disclosure)
- REST reference: [repository security advisories](https://docs.github.com/en/rest/security-advisories/repository-advisories) (API version 2026-03-10), [code scanning](https://docs.github.com/en/rest/code-scanning/code-scanning)
- Changelog: [CodeQL Rust GA, 14 October 2025](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available/), [agentic autofix, 10 July 2026](https://github.blog/changelog/2026-07-10-agentic-autofix-for-code-scanning-alerts-in-public-preview/), [CodeQL 2.26.0, 10 July 2026](https://github.blog/changelog/2026-07-10-codeql-2-26-0-adds-kotlin-2-4-0-support-and-ai-prompt-injection-detection/), [AI security detections, 14 July 2026](https://github.blog/changelog/2026-07-14-code-scanning-shows-ai-security-detections-on-pull-requests/), [structured private vulnerability reports, 1 October 2026](https://github.blog/changelog/2026-10-01-structured-forms-for-private-vulnerability-reports/), [Copilot `/security-review`, 14 July 2026](https://github.blog/changelog/2026-07-14-security-reviews-now-available-in-the-github-copilot-app/)
- Blog: "Found means fixed" (20 March 2024): https://github.blog/news-insights/product-news/found-means-fixed-introducing-code-scanning-autofix-powered-by-github-copilot-and-codeql/; "A maintainer's guide to vulnerability disclosure": https://github.blog/security/vulnerability-research/a-maintainers-guide-to-vulnerability-disclosure-github-tools-to-make-it-simple/
- Read-only `gh api` calls on the seven repositories: `repos/{o}/{r}` (`security_and_analysis`), `private-vulnerability-reporting`, `vulnerability-alerts`, `automated-security-fixes`, `code-scanning/default-setup`, `.github/workflows` contents

**Other vendors and tools**

- Semgrep, "Finding vulnerabilities in modern web apps using Claude Code and OpenAI Codex" (2 September 2025; vendor research): https://semgrep.dev/blog/2025/finding-vulnerabilities-in-modern-web-apps-using-claude-code-and-openai-codex/
- Semgrep Rules License v1.0: https://semgrep.dev/legal/rules-license; engine https://github.com/semgrep/semgrep (LGPL-2.1); Assistant overview: https://docs.semgrep.dev/semgrep-assistant/overview
- Veracode, 2026 GenAI Code Security Report (July 2026; vendor research): https://www.veracode.com/blog/2026-genai-code-security-report-ai-risk/
- DARPA, AIxCC results (8 August 2025, with editor's note): https://www.darpa.mil/news/2025/aixcc-results
- HackerOne press release (April 2026): https://www.hackerone.com/press-release/hackerone-introduces-h1-validation-help-enterprises-manage-surge-ai-discovered
- OpenSSF, "Guide to coordinated vulnerability disclosure for open source software projects", maintainer guide: https://github.com/ossf/oss-vulnerability-guide/blob/main/maintainer-guide.md
- Tool documentation, read by a background agent; the two quoted READMEs re-checked: [rustsec/audit-check](https://github.com/rustsec/audit-check), [EmbarkStudios/cargo-deny-action](https://github.com/EmbarkStudios/cargo-deny-action), [rustsec/rustsec (cargo-audit)](https://github.com/rustsec/rustsec), [cargo-deny](https://embarkstudios.github.io/cargo-deny/), [npm audit](https://docs.npmjs.com/cli/commands/npm-audit), [OSV-Scanner](https://google.github.io/osv-scanner/), [Composer](https://getcomposer.org/doc/03-cli.md#audit), [pip-audit](https://github.com/pypa/pip-audit), [gitleaks](https://github.com/gitleaks/gitleaks) and [gitleaks-action](https://github.com/gitleaks/gitleaks-action), [Bandit](https://bandit.readthedocs.io/), [actions/dependency-review-action](https://github.com/actions/dependency-review-action)
- Repositories checked for licence and contents: [trailofbits/skills](https://github.com/trailofbits/skills) (CC-BY-SA-4.0), [trailofbits/buttercup](https://github.com/trailofbits/buttercup) (AGPL-3.0), [protectai/vulnhuntr](https://github.com/protectai/vulnhuntr) (AGPL-3.0), [semgrep/mcp](https://github.com/semgrep/mcp) (archived), [getsentry/skills](https://github.com/getsentry/skills), [openai/skills](https://github.com/openai/skills), [gemini-cli-extensions/security](https://github.com/gemini-cli-extensions/security), [mattpocock/skills](https://github.com/mattpocock/skills) at `4588b32`

**Field reports (anecdote)**

- Daniel Stenberg, curl: [The end of the curl bug-bounty](https://daniel.haxx.se/blog/2026/01/26/the-end-of-the-curl-bug-bounty/) (26 January 2026); [A new breed of analyzers](https://daniel.haxx.se/blog/2025/10/10/a-new-breed-of-analyzers/) (10 October 2025); [High-quality chaos](https://daniel.haxx.se/blog/2026/04/22/high-quality-chaos/) (22 April 2026); [Mythos finds a curl vulnerability](https://daniel.haxx.se/blog/2026/05/11/mythos-finds-a-curl-vulnerability/) (11 May 2026)
- Willy Tarreau on the Linux kernel security list, LWN comment (31 March 2026): https://lwn.net/Articles/1065620/

**thirdshift itself**

- [CONTEXT.md](../../CONTEXT.md); [skills/thirdshift-code-review/SKILL.md](../../skills/thirdshift-code-review/SKILL.md); [skills/thirdshift-improve-codebase-architecture/SKILL.md](../../skills/thirdshift-improve-codebase-architecture/SKILL.md)
- [src/session.rs](../../src/session.rs) (`claude_args`, `codex_args`), [src/prompt.rs](../../src/prompt.rs) (`fresh`, `architecture_review`, `ci_fix_repair`), [src/ci.rs](../../src/ci.rs), [src/harness.rs](../../src/harness.rs)
- ADRs [0001](../adr/0001-rust-binary-with-embedded-skills.md), [0008](../adr/0008-inherited-failures-fail-the-run.md), [0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)
- [multi-family-review.md](multi-family-review.md), [codex-headless-harness.md](codex-headless-harness.md)
- This machine: `~/.thirdshift/config.toml` (User config); `~/.thirdshift/logs/JacobStephens2/*/commands/**/*.log` and `*/sessions/*.jsonl`; `crontab -l`; `gh auth status`; `sysctl kernel.apparmor_restrict_unprivileged_userns`
- Issue [#427](https://github.com/JacobStephens2/thirdshift/issues/427)
