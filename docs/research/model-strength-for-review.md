# Model strength for review: is the Artificial Analysis Intelligence Index a good guide?

Research date: 2026-10-05.

**Question.** [multi-family-review.md](multi-family-review.md) found that a reviewer's strength relative to the author mattered more than its model family (TL;DR, §1, §3.5, §8):

- In the July 2026 LiveCodeBench study ([arXiv:2607.21656](https://arxiv.org/abs/2607.21656)), Claude Opus 4.7 reviewing GPT-5.5's drafts helped, and GPT-5.5 reviewing Opus 4.7's drafts hurt. Opus 4.7 was the stronger solo model there, 91.4% against 71.6%.
- In SWE-Review ([arXiv:2607.06065](https://arxiv.org/abs/2607.06065)), a strong reviewer helped weak generators by up to 25 points, and a weak reviewer of a strong generator's PRs cost 7.3 points.

So the second-opinion reviewer should be at least as strong as the author. How should thirdshift's user judge which model is stronger? Is the [Artificial Analysis (AA) Intelligence Index](https://artificialanalysis.ai/leaderboards/models) a good measure, or is something better? This note covers:

1. What the AA index is today, and the current scores of the six models thirdshift's user cares about:
   - `claude-opus-5-5` (Claude Code; run at medium);
   - `gpt-6.1-sol` (Codex; xhigh);
   - Gemini 3.8 Flash (agy; low, medium, high);
   - `grok-4.7` (Grok Build; low to xhigh);
   - `muse-spark-1.3` (Muse Code);
   - `mimo-v2.6-pro` (MiMo Code).
2. Whether a general index predicts review strength, tested against the two studies above.
3. Better alternatives, and how to measure on thirdshift's own work.
4. How much reasoning effort and the agent harness move a model's strength.

**Method.**

- **AA data.** Read from the pages' server-rendered HTML and the model records embedded in it, parsed by script on 5 October 2026: the leaderboard, model pages, evaluation pages, methodology pages and articles. No screenshots; the numbers are the page data.
  - AA's API needs an account key, which I did not create (§1.2).
- **What AA showed in July 2026.** Wayback Machine snapshots fetched raw (`id_`) and parsed the same way: 1–25 July 2026 for the July study's models, 1 July for SWE-Review's, and 15 October 2025 for SWR-Bench's.
- **Papers.** Read in full from arXiv HTML, with methods and results; venues checked against the arXiv comment field or Crossref.
- **Delegated reading, spot-checked.** Three background agents read the code-review benchmarks and critique literature, the evaluation-methodology literature, and the other leaderboards. I re-checked the numbers this note leans on hardest against the sources myself:
  - CRJudgeBench Table 2 and Table 3;
  - SWR-Bench Table 8 and its ranking quote;
  - Epoch AI's ECI file;
  - Vals' LiveCodeBench data, including the per-model effort settings;
  - LMArena's current and July 2026 ratings;
  - Datacurve's DeepSWE board;
  - the Lewis harness paper;
  - all arXiv IDs and first authors, against the arXiv API.
- **Vendor material.** Anthropic, OpenAI, Google and SpaceXAI documentation.
- **My computations.** Correlations, discordance rates, harness deltas and power figures are mine unless attributed, and are marked "my computation".

**Evidence grades used below.**

| Grade | Meaning |
|---|---|
| Peer-reviewed | Journal or main conference |
| Workshop | Accepted, light review |
| Preprint | Not reviewed |
| Third-party leaderboard | Run by an organisation that does not sell the models scored |
| Vendor leaderboard | Run by a company that sells a scored product |
| Vendor docs | Official documentation |
| Vendor blog | Official posts |
| Anecdote | Case study or single campaign |

**Out of scope.** Choosing a reviewer configuration for thirdshift, and anything only a live run could show.

## TL;DR

- **The AA Intelligence Index (v4.3.2, September 2026) is a broad composite, not a coding or reviewing measure.** (§1; third-party leaderboard)
  - **Composition.** Ten evaluations: Agents 30% (mostly knowledge-work deliverables), Coding 20%, General knowledge and long documents 30%, Scientific reasoning 20%.
  - **The coding share** is Terminal-Bench 4.0 (10%) and SciCode (10%). There is no code review, no SWE-bench-style repository repair, and no LiveCodeBench (dropped January 2026).
  - **How it is run.** AA calls each model through its API in AA's own harnesses (Stirrup, mini-swe-agent), not the vendor CLIs thirdshift uses. Each reasoning effort is a separate row.
  - **Churn.** AA published eleven index versions (4.0 to 4.3.2) between January and September 2026, so scores are not comparable across versions. AA claims a 95% interval under ±1 point but has not published the analysis.
- **All six models are on it. The pair thirdshift runs most is a tie.** (accessed 5 October 2026)
  - **Claude Opus 5.5 at medium** scores 51.2 (low 42.3, high 53.6, xhigh 56.0, max 57.6). **GPT-6.1 Sol at xhigh** scores 51.0 (low 42.1, max 51.8).
  - Muse Spark 1.3 scores 45.1 at xhigh and 48.1 at max. MiMo-V2.6-Pro scores 46.3 (one row). Grok 4.7 scores 42.2 to 46.4. Gemini 3.8 Flash scores 33.5 to 40.9.
- **AA also has a Coding Agent Index that scores model + CLI + effort** (DeepSWE, Terminal-Bench 4.0, SWE-Atlas-QnA). It is the closest public measure to thirdshift's setup, but it covers the six models only partly.
  - Opus 5.5 appears only at max (66.0); there is no medium row. GPT-6.1 Sol at xhigh in Codex scores 62.9. Grok 4.7 at xhigh in Grok Build scores 56.3. Muse Spark 1.3 at max in Muse Code scores 54.3. Gemini 3.8 Flash at high in the Antigravity SDK, not the agy CLI, scores 41.9.
  - MiMo-V2.6-Pro is absent. (§0, §1.3)
- **Retrodiction: AA would not have predicted the July asymmetry, and no general index did.** (§2.1, §3.1)
  - **AA's index called it a tie.** In July 2026 AA had Opus 4.7 (max) at 53.5 and GPT-5.5 (high) at 53.1, inside AA's own interval. GPT-5.5 at xhigh scored 54.8.
  - **AA's model-plus-CLI coding index pointed the other way.** It put Codex + GPT-5.5 ahead of Claude Code + Opus 4.7 by 11–13 points.
  - **Epoch's ECI leaned GPT-5.5** (158.5 vs 156.1 in July).
  - **Most other boards called the pair level or favoured GPT-5.5.** That includes Vals' own LiveCodeBench run, 85.3 vs 85.1, against the study's 91.4% vs 71.6%.
  - **Three narrow measures pointed the study's way.** LMArena's human-preference Code Arena (1558 vs 1482). Epoch and METR's MirrorCode (31% vs 10%, both at high; 25 programs; possibly published after the study). And, narrowly, GSO (44% vs 40%).
  - **The study's problems may be contaminated.** Both models' published training cutoffs postdate its AtCoder problems.
- **The SWE-Review pattern was predictable, because the gaps were huge.** AA put the helpful reviewers 19–37 points above their authors and the harmful one 23–33 points below. The only near-peer pair was too close to call, and it was a draw in the study (+3.0 vs +2.8). (§2.2)
- **Inside AA's own data, the index orders pairs reliably only when the gap is large.** (§2.3; my computation)
  - Across 26 model-plus-CLI rows, an index gap of 8+ points never ordered the coding-agent scores wrongly. Gaps under 1 point were wrong 54% of the time, and gaps of 1–3 points 26–32%.
  - Against the repository tasks the index does not share with the agentic index, the rank correlation is 0.73. Against DeepSWE alone it is 0.42.
- **Review ability is its own axis.** (§2.4, §3.2)
  - **None of the six models appears on any code-review benchmark.**
  - **SWR-Bench** (FSE 2026): review rankings "do not consistently mirror" SWE-bench or LiveCodeBench. Its ten models correlate with AA's then-current index at only ρ 0.08–0.35 (my computation).
  - **CRJudgeBench** (September 2026): GLM-5.3 and Kimi K3 beat Claude Opus 5 and GPT-5.5 at spotting invalid review comments.
  - **Critique literature:** verification ability tracks general ability loosely, and least when the author is the stronger model.
- **Effort and harness move scores as much as model choice does.** (§4)
  - **Effort, index:** Opus 5.5 spans 15 points from low to max.
  - **Effort, agentic:** Claude Sonnet 5.5 in Claude Code spans 26 agentic points. GPT-6.1 Sol in Codex spans only 6, and its xhigh beats its max.
  - **Effort, vendor sweep:** Anthropic's sweep has Opus 5.5 at medium 2.5 points below high on its SWE-bench Pro subset.
  - **Harness, Terminal-Bench 4.0 (AA):** the same model at the same effort moved from −22 to +18 points between AA's fixed harness and a CLI, though mostly within ±5.
  - **Harness, neutral vs vendor CLI (Vals, SWE-rebench):** the vendor CLI usually scored 3–19 points lower.
  - **Harness, July 2026 (AA):** Opus 4.7 at medium scored 37.5 in Claude Code and 45.5 in OpenCode.
- **Confidence.**
  - High that a gap of about 8+ index points at the efforts actually run identifies the stronger model.
  - Low for anything closer, which includes thirdshift's main pair. No public number ranks near-peer frontier models as reviewers.
- **What to use (my recommendation; §6).**
  - Use AA as a coarse filter at the efforts you run, and prefer its Coding Agent Index where a row exists.
  - Give the reviewer at least the author's effort.
  - Settle near-peer pairs with a paired, blind "who catches whose bugs" test on thirdshift's own Runs. Size it for the difference that would change the decision: about 170–330 known bugs to detect a 10-point catch-rate difference, and 60–85 for a 20-point one (§3.3).

## 0. The six models at a glance

**AA Intelligence Index v4.3.2** (model through its API, in AA's harnesses):

- Read from [artificialanalysis.ai/leaderboards/models](https://artificialanalysis.ai/leaderboards/models) and the model pages' embedded data, accessed 5 October 2026.
- The leaderboard rounds to whole numbers; one decimal is shown here.
- Anthropic rows were run "with Anthropic's default fallback enabled" (§1.2).

| Model | Effort rows on AA | Intelligence Index | Terminal-Bench 4.0 (mini-swe-agent) | SciCode | Cost per index task | Release (AA) |
|---|---|---|---|---|---|---|
| Claude Opus 5.5 | low / **medium** / high / xhigh / max | 42.3 / **51.2** / 53.6 / 56.0 / 57.6 | 31.3 / 52.5 / 56.6 / 59.6 / 59.6 | 58.6 / 59.3 / 60.4 / 65.0 / 66.9 | $0.55 / $1.34 / $1.82 / $3.46 / $5.98 | 22 Sep 2026 |
| GPT-6.1 Sol | low / medium / high / **xhigh** / max | 42.1 / 47.8 / 50.2 / **51.0** / 51.8 | 30.8 / 48.0 / 51.5 / 54.0 / 56.1 | 53.2 / 53.2 / 55.8 / 55.7 / 54.2 | $0.13 / $0.21 / $0.32 / $0.39 / $0.72 | 29 Sep 2026 |
| Gemini 3.8 Flash | low / medium / high | 33.5 / 39.8 / 40.9 | 10.1 / 19.7 / 19.7 | 55.0 / 55.1 / 56.6 | n/a / $0.93 / $1.24 | 2 Sep 2026 |
| Grok 4.7 | low / high / xhigh (no medium row) | 42.2 / 46.3 / 46.4 | 16.2 / 24.7 / 25.8 | 54.9 / 57.8 / 57.4 | $1.25 / $2.73 / $3.74 | 21 Sep 2026 |
| Muse Spark 1.3 | xhigh / max | 45.1 / 48.1 | 16.7 / 33.3 | 59.7 / 58.8 | $1.37 / $1.60 | 2 Sep 2026 |
| MiMo-V2.6-Pro | one row, no effort label | 46.3 | 34.8 | 60.9 | $0.13 | 21 Sep 2026 |

**AA Coding Agent Index v1.5** (model in a coding-agent CLI; DeepSWE v1.1, Terminal-Bench 4.0 and SWE-Atlas-QnA, equal weights). Read from [artificialanalysis.ai/agents/coding-agents](https://artificialanalysis.ai/agents/coding-agents), accessed 5 October 2026.

| Harness (version) | Model (effort) | Index | DeepSWE | Terminal-Bench 4.0 | SWE-Atlas-QnA | API cost per task |
|---|---|---|---|---|---|---|
| Claude Code 2.1.280 | Opus 5.5 (max) | 66.0 | 68.4 | 63.1 | 66.4 | $13.04 |
| Codex 0.154.0 | GPT-6.1 Sol (low / medium / high / **xhigh** / max) | 57.2 / 61.4 / 60.1 / **62.9** / 60.1 | 67.6 / 72.0 / 70.5 / 73.2 / 69.6 | 49.0 / 51.5 / 50.0 / 54.5 / 53.0 | 55.1 / 60.8 / 59.9 / 61.0 / 57.8 | $0.50 / $0.70 / $0.89 / $1.04 / $1.55 |
| Antigravity SDK 0.1.12–0.1.16 | Gemini 3.8 Flash (high) | 41.9 | 65.8 | 14.6 | 45.2 | $2.47 |
| Grok Build 1.0.40 | Grok 4.7 (xhigh) | 56.3 | 72.6 | 33.3 | 62.9 | $8.82 |
| Muse Code 1.0.3 | Muse Spark 1.3 (max) | 54.3 | 71.7 | 31.8 | 59.4 | $3.98 |
| Muse Code 1.0.2 RC | Muse Spark 1.3 (xhigh) | 48.3 | 73.2 | 17.2 | 54.6 | $3.47 |

**Missing from the Coding Agent Index:**

- Claude Opus 5.5 at medium, thirdshift's setting. Only max is listed.
- Gemini 3.8 Flash in the agy (Antigravity) CLI. Only Gemini 4 Argon has an Antigravity CLI row.
- Grok 4.7 below xhigh.
- MiMo-V2.6-Pro in MiMo Code, or in any harness.

**Epoch AI Capabilities Index (ECI)** (one score per model, no effort split; [eci_scores.csv](https://epoch.ai/data/eci_scores.csv), accessed 5 October 2026; third-party leaderboard):

| Model | ECI | 95% interval |
|---|---|---|
| Claude Opus 5.5 | 167.3 | 164.1–171.7 |
| GPT-6.1 Sol | 166.1 | 162.9–170.6 |
| Muse Spark 1.3 | 156.8 | 154.8–159.3 |
| Gemini 3.8 Flash | 156.7 | 154.4–160.3 |
| Grok 4.7 | 153.5 | 151.8–155.9 |
| MiMo-V2.6-Pro | absent | |

**The two aggregators disagree in the middle.** Epoch puts Gemini 3.8 Flash level with Muse Spark 1.3 and above Grok 4.7. AA has Gemini 3.8 Flash (high) 5.5 points below Grok 4.7 (xhigh).

**Other coding leaderboards that list these models.** Accessed 5 October 2026. A background agent read them, and I re-checked the LMArena, DeepSWE, FrontierCode top-score and Vals LiveCodeBench values from its saved pages. Details and grades are in §3.1.

| Leaderboard (setup) | Opus 5.5 | GPT-6.1 Sol | Gemini 3.8 Flash | Grok 4.7 | Muse Spark 1.3 | MiMo-V2.6-Pro |
|---|---|---|---|---|---|---|
| LMArena Code Arena (human votes on building web apps; 1 Oct) | 1815 (max) | 1758 (max, Codex harness) | 1583 (high) | 1638 (xhigh) | 1657 (max); 1624 (xhigh) | 1618 |
| LMArena text, coding category (2 Oct) | 1538 (high) | 1542 (max) | 1530 (high) | 1488 (xhigh) | 1539 (max) | 1540 |
| Vals Vibe Code Bench v1.1 (OpenHands; 2 Oct) | 90.3 ± 1.5 (effort not stated) | 88.9 ± 1.9 (max) | 78.7 ± 3.9 (high) | 86.2 ± 2.2 (xhigh) | 85.9 ± 2.5 (max); 82.9 ± 2.9 (xhigh) | 85.2 ± 3.4 |
| Vals Terminal-Bench 4.0 (mini-SWE-agent; 1 Oct; no error or effort shown) | 65.2 | 55.1 | 19.2 | 28.8 | 24.8 (max) | 31.3 |
| tbench.ai Terminal-Bench 4.0 (agent + model; 21 Sep) | not listed | not listed | 19.1 ± 3.4 (high, mini-SWE-agent) | 37.6 ± 3.5 (xhigh, Grok Build) | not listed | not listed |
| Datacurve DeepSWE v1.1 (mini-swe-agent; 22 Sep) | not listed | not listed | **74 ± 1 (high), tied for the top score** with Claude Opus 5 (max); 71 (medium) | not listed | not listed | not listed |
| CursorBench (vendor; Cursor's harness) | 43.7 / 52.5 / 56.0 / 56.0 / 57.8 (low → max) | not listed | 37.3 (medium); 39.6 (high) | 33.1 / 41.6 / 43.9 / 46.3 (low → xhigh) | 24.3 (minimal) … 41.6 (max) | not listed |
| FrontierCode 1.1 (vendor, Cognition; each model in its vendor's agent) | 54.6 (medium, Claude Code) | 50.2 (medium, Codex) | 41.2 (medium) | 47.6 (high, Grok Build) | not listed | not listed |

**Not listing any of the six:**

- SWE-bench Verified: frozen. Vals stopped running it because it "has saturated".
- Scale's SWE-Bench Pro: newest entry July 2026.
- METR time horizons: last updated May 2026.
- The official LiveCodeBench board (stale since August 2025) and Aider Polyglot (since October 2025).
- SWE-rebench's current window.

**What the boards say about the main pair.**

- **They roughly agree on the top two.** Opus 5.5 and GPT-6.1 Sol lead almost everywhere they both appear. The exception is LMArena's text-coding category, where all six sit within overlapping intervals.
- **They split on the rest.** Gemini 3.8 Flash is last on AA's index and on Terminal-Bench. Yet it ties for first on Datacurve's DeepSWE, the board closest to long repository changes, and scores 89.5% on Vals' LiveCodeBench.
- **Opus 5.5 at medium against GPT-6.1 Sol at medium.** The only public row is FrontierCode, in each vendor's CLI: 54.6 vs 50.2, on a vendor-run board with no intervals.

## 1. The Artificial Analysis Intelligence Index today

### 1.1 Version, components and weights

- **Version.** v4.3.2, current since September 2026 ([methodology](https://artificialanalysis.ai/methodology/intelligence-benchmarking), "Version History"). AA calls v4.2 and v4.3 "a continuation of our rollout of Intelligence Index v5" ([v4.3 announcement](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3), 7 September 2026).
- **Formula.** A weighted average over four categories: Agents 30%, Coding 20%, Scientific Reasoning 20%, General 30%.

| Category | Evaluation | Items × repeats | Weight | Tools | Private | What it is |
|---|---|---|---|---|---|---|
| Agents | AA-Briefcase v1.1 | 91 tasks × 1 | 15% | ✓ | ✓ | Multi-week business projects with file deliverables; Elo from rubric and pairwise LLM judging |
| Agents | GDPval-AA v2.1 | 220 × 1 | 10% | ✓ | ✗ | OpenAI's GDPval tasks across 44 occupations; Elo from a three-model judge panel |
| Agents | AutomationBench-AA | 657 × 1 | 5% | ✓ | ✓ | Zapier's SaaS workflows over REST APIs; zero for any guardrail violation |
| Coding | Terminal-Bench 4.0 | 66 × 3 | 10% | ✓ | ✗ | Terminal tasks (software engineering, sysadmin, ML, security) in the mini-swe-agent harness |
| Coding | SciCode | 288 subproblems × 3 | 10% | ✗ | ✗ | Single-turn scientific Python, unit-tested |
| General | AA-Omniscience | 6,000 × 1 | 15% | ✗ | ✓ | Knowledge accuracy (10%) and 1 − hallucination rate (5%) |
| General | GDP.pdf | 100 × 5 | 10% | ✗ | ✗ | Surge AI's reasoning over long professional PDFs |
| General | AA-LCR v1.1 | 100 × 3 | 5% | ✗ | ✗ | Reasoning over ~100k-token document sets |
| Scientific Reasoning | Humanity's Last Exam | 2,158 × 1 | 10% | ✗ | ✗ | Text-only frontier academic questions |
| Scientific Reasoning | CritPt | 70 × 5 | 10% | ✗ | ✓ | Research-level physics |

- **What is not in it.** Code review, repository-scale bug fixing, and competitive programming. LiveCodeBench was "Removed from the Intelligence Index in v4.0" (January 2026) and is "Retired from our active reporting". Terminal-Bench 4.0 is the only agentic coding component, at 10%.
- **Precision AA claims.** "We estimate a 95% confidence interval for Artificial Analysis Intelligence Index of less than ±1%". The estimate rests on repeats "on certain models", and the analysis is unpublished: "We look forward to disclosing further detail".
- **Two components under review.** On the model pages, SciCode and CritPt currently carry an "Under review" badge. I found no explanation on the pages I read.

### 1.2 Who runs it, and how

- **Who.** Artificial Analysis, a venture-backed company that calls itself "the independent benchmarking company for AI" ([about](https://artificialanalysis.ai/about)). It sells data and evaluation products alongside the free leaderboard. "All evaluations are conducted independently by Artificial Analysis." Grade: third-party leaderboard.
- **Through each model's API, in AA's harnesses, not the vendors' CLIs.**
  - Agentic evaluations run in AA's open-source harness [Stirrup](https://github.com/ArtificialAnalysis/Stirrup) in E2B sandboxes.
  - Terminal-Bench 4.0 runs in mini-swe-agent with its defaults: native bash tool, no context compaction, up to 500 steps.
  - AA's own wording, in its Grok 4.7 post: the Coding Agent Index results "are separate from the Intelligence Index results, which standardize the evaluation harness used across models" ([article](https://artificialanalysis.ai/articles/benchmarking-grok-4-7), 21 September 2026).
- **Sampling settings.** Temperature 0.6 for reasoning models "unless another temperature is recommended by the model lab"; the vendor's maximum output tokens; up to 30 retries on API failures.
- **LLM judges.** Several components are LLM-graded:
  - GDPval-AA and the AA-Briefcase pairwise matches: a panel of Claude Opus 5 (high), GPT-5.6 Sol (medium) and Gemini 3.8 Flash (high).
  - AA-Briefcase rubrics: Claude Opus 4.8 (max), GPT-5.5 (high) and Gemini 3.1 Pro Preview (high).
  - HLE, AA-LCR, AA-Omniscience and GDP.pdf: GPT-5.6 Luna (medium) as grader.
- **Effort.** Each reasoning-effort setting is a separate row with its own score, for example "Claude Opus 5.5 (Adaptive Reasoning, Medium Effort, Default Fallback)".
  - Anthropic's models were run "with Anthropic's default fallback enabled" ([AA on Opus 5.5](https://artificialanalysis.ai/articles/claude-opus-5-5), 22 September 2026).
  - Under that fallback, a request a safety classifier declines is retried on the model Anthropic recommends for its category: Claude Opus 4.8 for cyber, where "Benign cybersecurity work can also trigger this category" ([Refusals and fallback](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback); vendor docs).
  - So part of an "Opus 5.5" score can be another model's work (§1.3 has the rates).
- **Cadence.** New frontier models are scored within days of release: GPT-6.1 Sol on 29 September, Claude Opus 5.5 on 22 September, Grok 4.7 on 21 September 2026.
- **The index itself changes often.** Between January and September 2026 AA published eleven versions, 4.0 to 4.3.2.
  - Four of them (4.0, 4.1, 4.2, 4.3) added, removed or replaced component evaluations. The rest changed graders, judge panels or scoring.
  - Scores are not comparable across versions. Claude Opus 4.7 (max) was 53.5 under v4.1 in July and is 40.7 under v4.3.2 now.
  - Older models that were not re-run on new components get an "* Estimated" score. The pages I read do not say how the estimate is made.
- **Data access.** The leaderboard and model pages are server-rendered with the full model records embedded, which is how this note read them.
  - AA's free API (`GET /api/v2/data/llms/models`, 1,000 requests a day) needs an `x-api-key` from an account on its Insights Platform ([API reference](https://artificialanalysis.ai/api-reference)). I did not sign up.
  - The API's example response still shows `artificial_analysis_coding_index`.

### 1.3 Coding Index, Agentic Index, and the Coding Agent Index

**The Coding and Agentic Indexes (v4.1, until September 2026).**

- In July 2026 each model page had "Coding Index" and "Agentic Index" tabs, and the model records carried `codingIndex` and `agenticIndex` (archived pages).
- **The Coding Index was the v4.1 Coding category:** Terminal-Bench 2.1 (16%) and SciCode (8%), weighted 2:1.
  - Claude Opus 4.7 (max): (2 × 83.15 + 54.51) / 3 = 73.60, the stored value.
  - GPT-5.5 (high): (2 × 79.40 + 55.90) / 3 = 71.57, also the stored value (my computation).
- **I could not reconstruct the Agentic Index** from the archived fields, and the archived methodology does not define it.
- **Neither tab exists any more.** Current model pages have neither, and the current methodology mentions the Coding Index only in a legacy note: Terminal-Bench 2.1 "remains part of the Coding Index".
- The current "Capability Indexes" (Finance, Legal, Engineering and others) are weightings of the same components by profession; none is about software ([methodology](https://artificialanalysis.ai/methodology/capability-indices)).

**The Coding Agent Index v1.5 (current; a separate product).** This is the closest thing AA publishes to what thirdshift does. "Each public row is an agent variant, not a model": a model in a named CLI at a stated effort ([methodology](https://artificialanalysis.ai/methodology/coding-agents-benchmarking); [leaderboard](https://artificialanalysis.ai/agents/coding-agents)).

- **Components, equally weighted.**
  - DeepSWE v1.1 (Datacurve): 113 long-horizon changes to real repositories.
  - Terminal-Bench 4.0: 66 tasks.
  - SWE-Atlas-QnA (Scale AI): 124 questions about repositories, judged by Claude Opus 4.5.
- **Scoring.** Three attempts per task, pass@1.
- **Zero-scored attempts.** Attempts blocked by a safety refusal score zero. So do Terminal-Bench attempts that an agent judge (Claude Code with Claude Sonnet 5) flags as reward hacking.
- **Fallbacks are recorded.** In Claude Code with Opus 5.5 (max), another model (Claude Opus 4.8 or Opus 5) finished:
  - 0.6% of DeepSWE attempts;
  - 12.1% of Terminal-Bench attempts;
  - 14.0% of SWE-Atlas-QnA attempts.

  Codex with GPT-6.1 Sol had 0–4% safety refusals and no fallback.
- **History.** Versions run from v1.0 (May 2026) to v1.5 (September 2026). v1.1 replaced SWE-Bench-Pro-Hard-AA with DeepSWE. A "Harness Comparison" tab is marked "Coming soon".

## 2. Does a general index predict review strength?

### 2.1 Retrodiction: the July 2026 LiveCodeBench study

**What AA, and other boards, showed when the study ran.**

- AA pages archived 1–25 July 2026, index v4.1.
- Neither model has an AA LiveCodeBench score: AA had dropped LiveCodeBench in January 2026, before either model was released.
- The rows after the Coding Agent Index come from other boards (§3.1). The MirrorCode and GSO values are from Epoch's current benchmark file.

| Measure (July 2026 unless noted) | Claude Opus 4.7 | GPT-5.5 | Gap | Points to |
|---|---|---|---|---|
| Intelligence Index v4.1 | 53.5 (max) | 53.1 (high); 54.8 (xhigh) | +0.4 vs high; −1.3 vs xhigh | Tie, inside AA's ±1 |
| Coding Index (Terminal-Bench 2.1 + SciCode, 2:1) | 73.6 (max) | 71.6 (high); 74.9 (xhigh) | +2.0; −1.3 | Tie, or slightly Opus at mismatched effort |
| Terminal-Bench 2.1 (AA harness) | 83.1 | 79.4 (high); 84.3 (xhigh) | +3.7; −1.1 | Mixed |
| Terminal-Bench Hard (AA harness) | 51.5 | 59.8 (high); 60.6 (xhigh) | −8.3 | GPT |
| SciCode | 54.5 | 55.9 (high); 56.1 (xhigh) | −1.4 | Tie |
| Coding Agent Index, vendor CLI (21 July) | Claude Code: 37.5 (medium); 45.2 (max) | Codex: 50.4 (medium); 56.6 (xhigh) | −12.9 at medium; −11.4 at top effort | **GPT, clearly** |
| Epoch ECI, 22 July (archived data; one score per model, whatever the effort) | 156.1 | 158.5 | −2.4 | GPT. The current file has 156.3 [154.4, 158.4] vs 159.1 [156.7, 162.3], intervals overlapping |
| Vals' own LiveCodeBench run (all splits; hard split; run date not shown, page updated 1 September) | 85.1 ± 1.0; 66.9 ± 2.5 (max) | 85.3 ± 1.0; 69.7 ± 2.5 (xhigh) | −0.2; −2.9 | Tie |
| LMArena Code Arena, 17 July (human votes on building web apps) | 1558 [1551, 1565] (thinking) | 1482 [1475, 1489] (high, Codex harness); 1504 (xhigh) | +76 | **Opus** |
| MirrorCode (Epoch AI with METR: reimplement 25 programs without their source; current data, not in Epoch's 22 July archive) | 31.1% (high) | 10.0% (high) | +21.1 | **Opus** |
| GSO (OpenHands, evaluated 27 April) | 44.1% (high) | 40.2% (xhigh) | +3.9 | Opus, narrowly |
| **The study: LiveCodeBench solo pass rate, both at high** | **91.4%** | **71.6%** | **+19.8** | **Opus** |

**Notes on the table.**

- **Effort mismatch.** AA listed Opus 4.7 only at max and at non-reasoning high. The study ran both models at high effort (`--effort high` and `model_reasoning_effort="high"`, per the artifact), so no AA row matches the study's Claude setting. Comparing Opus at max with GPT-5.5 at high flatters Opus.
- **Same CLIs as the study.** The Coding Agent Index rows ran Claude Code 2.1.x and Codex 0.120–0.137. In July that index was DeepSWE v1.0, SWE-Atlas-QnA and Terminal-Bench 2.0 (archived leaderboard, 21 July 2026).

**Would AA have predicted the asymmetry? No.**

- **The Intelligence Index called the pair a tie.** The gap was 0.4 points, and GPT-5.5 at xhigh was ahead.
- **The Coding Index leaned Opus by 2 points,** but only by comparing Opus at max with GPT-5.5 at high.
- **AA's model-plus-CLI coding index pointed the other way,** by 11–13 points.
- **The study's task type was not measured.** The reviewer asymmetry followed strength **on contest programming**, which AA did not measure.
- **Even a LiveCodeBench leaderboard would not have predicted it.** Vals' run of the same benchmark had the two level, with GPT-5.5 slightly ahead on hard problems.
- **The study's gap belongs to its own configuration:** both models in their CLIs at high effort, one sample per task, complete-case filtering.
- **Three narrow measures pointed the right way.**
  - LMArena's Code Arena, a human-preference vote on building web apps.
  - MirrorCode, long-horizon reimplementation of 25 programs, which I could not confirm was published before the study.
  - Narrowly, GSO.
  - None is a general index, and one correct call is not a track record.
- **Reading.** "Stronger" was specific to the task and the configuration. The coding measures closest to thirdshift's setting ranked the pair the opposite way from the study.

**A contamination caveat on the study.** The paper says its problems were released "after the training cutoffs of Claude Opus 4.7 and Codex GPT-5.5".

- **Task dates.** The artifact samples from 1 January 2025 (`min_date = "2025-01-01"`). Its AtCoder tasks come from ABC 387–400 and ARC 190–196, contests held 4 January to 6 April 2025 ([AtCoder Problems contest list](https://kenkoooo.com/atcoder/resources/contests.json)).
- **Model cutoffs.** Anthropic gives Claude Opus 4.7 a training-data cutoff of January 2026 ([model page](https://platform.claude.com/docs/en/models/opus-4-7/overview)). OpenAI gives GPT-5.5 a knowledge cutoff of 1 December 2025 ([model page](https://developers.openai.com/api/docs/models/gpt-5.5)); both are vendor docs.
- **So the AtCoder tasks predate both cutoffs by eight months or more.** The 20-point lead may be partly memorisation, and memorisation could differ between the models. I did not check the LeetCode tasks' dates.

### 2.2 Retrodiction: SWE-Review

**Model identities** (paper §3.1, read in full by a background agent):

- **GLM-5:** a locally deployed GLM-5-FP8. Whether thinking was on for generation is not stated.
- **Qwen3-Coder-30B-A3B:** the Instruct release.
- **Qwen3-30B-A3B:** the original April 2025 hybrid-thinking release, not 2507. Thinking mode is not stated. "(base)" means before the authors' fine-tuning.
- **Claude Opus 4.6:** effort and thinking are not reported.
- **Reviewer setup.** An OpenHands-SDK agent with 100 iterations that can browse the repository and run commands. One revision round. Table 2 is a single run with no significance tests.

**AA as of 1 July 2026** (index v4.1; AA marked all of these "estimated"; archived page):

| Model | AA Intelligence Index | Role and result in SWE-Review (Table 2) |
|---|---|---|
| Claude Opus 4.6 | 43.7 (adaptive, max) or 37.8 (non-reasoning, high) | Reviewer: +3.0 on GLM-5's PRs, +16.4 on Qwen3-Coder's, +25.1 on Qwen3-30B's |
| GLM-5 | 39.5 (reasoning) or 32.4 (non-reasoning) | Generator, 72.2% resolved. Reviewing itself +2.8; reviewing the two Qwens +9.1 and +20.0 |
| Qwen3-Coder-30B-A3B | 13.6 | Generator, 50.9% resolved |
| Qwen3-30B-A3B | 9.3 (reasoning) or 6.8 (non-reasoning) | Generator, 27.5% resolved. As reviewer: −7.3 on GLM-5's PRs, −1.8 on Qwen3-Coder's, +0.7 on its own |

**Would AA have predicted the pattern? Yes, where it mattered, because the gaps were huge.**

- **Reviewers 19–37 points above the author helped.** That covers Opus 4.6 and GLM-5 reviewing either Qwen. The range spans the variants each model might have run as.
- **A reviewer 23–33 points below the author hurt.** That is Qwen3-30B reviewing GLM-5. Its −7.3 came partly from producing no parseable review 16% of the time.
- **The one near-peer pair was "too close to call", and it was a draw.**
  - On AA, Opus 4.6 and GLM-5 in reasoning mode are −1.7 to +4.2 apart, depending on which unknown Opus setting was used. The paper's review teacher ran GLM-5 "with thinking enabled".
  - In the study, Opus 4.6 reviewing GLM-5 gave +3.0, and GLM-5 reviewing itself +2.8.
- **The order of the generators matches, but not the size of the gap.** AA ordered them as SWE-bench did, but put the two Qwens only 4 points apart against a 23-point resolve-rate gap. A general index under-rates a coding-specialised model on coding.
- **Epoch's ECI gives the same order:** Opus 4.6 155.2, GLM-5 145.8, Qwen3-30B-A3B 136.2 in the current file. On 22 July: 155.4 and 146.4, with Qwen3-30B-A3B not yet scored. Qwen3-Coder is absent.

**Both retrodictions together.**

- **A general index separates a frontier model from a 30B one.** That is not the decision thirdshift faces.
- **In the near-peer frontier case, thirdshift's case, it failed.** None of AA's index, AA's agentic index, Epoch's ECI or a LiveCodeBench leaderboard identified the model that turned out stronger in the study's configuration.
- **The measures that pointed the right way were narrow, task-specific ones.** That is an argument for measuring on your own tasks (§3.3), not for adopting whichever board happened to agree once.

### 2.3 Inside AA's own data: does the general index track agentic coding?

My computation from AA's current pages (accessed 5 October 2026).

- **Data.** 26 Coding Agent Index rows in which a vendor's own CLI runs its own model, each matched to the Intelligence Index row for the same model and effort.
- **Excluded.** Three rows where one vendor's CLI runs another vendor's model.
- **Correlation with the whole Coding Agent Index.** Spearman ρ = 0.92 (Pearson 0.93).
- **Most of that comes from a shared component.** Both indexes contain Terminal-Bench 4.0. Against the two components the Intelligence Index does not contain:
  - the mean of DeepSWE and SWE-Atlas-QnA: ρ = 0.73;
  - DeepSWE alone (long-horizon changes to real repositories, the most thirdshift-like): ρ = 0.42.
- **How often a gap of a given size gets the order wrong.** For every pair of rows, the share in which the Intelligence Index orders the pair the opposite way from the agentic score:

| Intelligence Index gap | Pairs | Wrong order vs Coding Agent Index | Wrong order vs DeepSWE + SWE-Atlas-QnA |
|---|---|---|---|
| < 1 point | 28 | 54% | 46% |
| 1–2 | 28 | 32% | 39% |
| 2–3 | 27 | 26% | 33% |
| 3–5 | 58 | 9% | 33% |
| 5–8 | 74 | 4% | 18% |
| ≥ 8 | 110 | 0% | 5% |

**Reading.**

- **Gaps of 8 or more points are a reliable signal.** That holds even for repository-level agentic work.
- **Gaps under about 3 points are close to a coin flip.** Against the components the two indexes do not share, gaps under 5 still order a third of pairs wrongly.
- **Caveats.**
  - The pairs are not independent: one model appears at several efforts.
  - Most rows are Anthropic and OpenAI models.
  - The agentic scores have noise of their own. DeepSWE's 113 tasks give a standard error of roughly 3–4 points at these pass rates (my estimate). Some "wrong orders" are noise in the target, not failures of the index.

### 2.4 Published evidence: do general scores predict reviewing?

**General benchmarks agree across a wide range of models, and disagree at the top.**

- **Observational Scaling Laws** (Ruan, Maddison, Hashimoto, NeurIPS 2024 spotlight, [arXiv:2405.10938](https://arxiv.org/abs/2405.10938); peer-reviewed).
  - Across models before mid-2024, one principal component explains nearly 80% of the variance on standard benchmarks.
  - Agent benchmarks were predictable from those components.
- **"A Rosetta Stone for AI Benchmarks"** (Ho et al., Epoch AI, [arXiv:2512.00193](https://arxiv.org/abs/2512.00193); preprint; the method behind ECI).
  - The stitched capability score predicts METR time horizons with R² 0.85.
  - SWE-bench Verified alone has negative R².
  - The authors: "AI capabilities cannot be fully described by a single number". Claude models beat their predicted SWE-bench scores.
- **BenchBench** (Perlitz et al., [arXiv:2407.13696](https://arxiv.org/abs/2407.13696); preprint). Benchmarks that agree over many models can show low agreement "over a few top-ranked models". Kendall τ between two leaderboards ran from about 0.65 to 0.99 depending on which models were compared.
- **AA itself:** "While model intelligence generally translates across use cases, specific evaluations may be more relevant for certain use cases" (model pages).

**Review rankings often invert general rankings.**

- **SWR-Bench** (FSE 2026, [arXiv:2509.01494](https://arxiv.org/abs/2509.01494), doi:10.1145/3808144; peer-reviewed). 1,000 real PRs and ten 2025 models.
  - Best F1 was GPT-5 at 20.85. GPT-4o (18.73) and Claude 3.7 Sonnet (18.23) beat Claude 4 Opus (16.99).
  - The authors: the ranking "does not consistently mirror trends observed in other SE benchmarks (e.g., SWE-Bench and LiveCodeBench)".
  - **My computation.** I matched its models to AA's index as of 15 October 2025 (archived leaderboard, v3), using SWR-Bench's own split of reasoning and standard models.
    - Spearman ρ between AA's index and review F1 was 0.08–0.35, depending on which AA row stands for GPT-5, DeepSeek-R1 and DeepSeek-V3. It was 0.32 for the API defaults.
    - For AA's Coding Index and for AA's LiveCodeBench it was 0.10–0.35 and 0.13–0.35.
    - Claude 4 Opus is excluded: AA had no index for its non-reasoning row.
  - **Noise on the review side.** SWR-Bench's F1 values span only 15.3–20.9, and only 27 review items were found in all five runs of one model. Part of the weak correlation is noise in the review benchmark.
- **CRJudgeBench** ([arXiv:2609.37216](https://arxiv.org/abs/2609.37216), 29 September 2026; preprint, under review at ICLR 2027). Deciding whether a review comment is trustworthy, in mini-swe-agent.
  - GLM-5.3 and Kimi K3 beat Claude Opus 5 and GPT-5.5:

    | Model | Accuracy | Untrustworthy comments caught |
    |---|---|---|
    | GLM-5.3 | 70.5% | 20.8% |
    | Kimi K3 | 69.9% | 19.2% |
    | Claude Opus 5 | 67.1% | 11.5% |
    | GPT-5.5 | 65.5% | 9.2% |

  - AA ranks Opus 5 above both open models at every effort.
  - Opus 5 and GPT-5.5 called 95.0% of comments trustworthy when 63.8% were.
  - A fine-tuned Qwen3-Coder-30B-A3B beat every frontier model (76.6%; 37.7%).
  - Efforts are not stated. Depending on which effort rows are assumed, the rank correlation with AA's index over the five frontier models runs from −0.60 to +0.90 (my computation).
- **SWE-PRBench** ([arXiv:2603.26130](https://arxiv.org/abs/2603.26130); preprint, vendor-adjacent).
  - Claude Haiku 4.5, Sonnet 4.6, DeepSeek V3 and Mistral Large 3 were statistically indistinguishable.
  - Every model got worse as more context was added.
- **"Bigger Isn't Always Better"** ([arXiv:2606.15689](https://arxiv.org/abs/2606.15689); preprint). Claude Haiku 4.5 beat Sonnet 4.6 (F1 0.365 vs 0.343).
- **Counter-examples.** In CR-Bench ([arXiv:2603.11078](https://arxiv.org/abs/2603.11078); preprint) and in SWE-Review, the bigger or stronger model did review better.

**Critique ability tracks generation ability only loosely, and least when checking a stronger author.**

- **CriticBench** ([arXiv:2402.14809](https://arxiv.org/abs/2402.14809), Findings of ACL 2024; peer-reviewed).
  - Generation, critique and correction rise together roughly linearly.
  - Yet "weaker models can surprisingly surpass stronger ones in their self-critique".
- **"Mind the Gap"** ([arXiv:2412.02674](https://arxiv.org/abs/2412.02674), ICLR 2025; peer-reviewed; maths).
  - The generation–verification gap grows with verifier strength and shrinks as the generator gets stronger.
  - For a weak model checking a strong one, a positive gap "cannot always be assured".
- **"Variation in Verification"** ([arXiv:2509.17995](https://arxiv.org/abs/2509.17995), ICLR 2026; peer-reviewed).
  - Verification ability "is generally correlated with the verifier's own problem-solving capability".
  - Strong generators' errors are harder to catch. A Qwen2.5-72B verifier's true-negative rate fell from 0.68 on Llama-3.1-8B's solutions to 0.17 on Qwen3-32B's.
- **RealCritic** ([arXiv:2501.14492](https://arxiv.org/abs/2501.14492); preprint). Models comparable at generation differ a lot as critics.
- **CriticGPT** ([arXiv:2407.00215](https://arxiv.org/abs/2407.00215); OpenAI preprint). Critique-specific training gave gains that naive extrapolation puts at about 30× pre-training compute. Review ability depends on review training, not only on general capability.

**What this means for the "stronger reviewer" rule (inference).** The general ordering is trustworthy when the gap is large, which matches SWE-Review. Among near-peers or specialised models, reviewing is its own axis and has to be measured.

### 2.5 Saturation, contamination and the gap to real work

- **SWE-bench Verified is contaminated and partly mis-graded.**
  - Models name the buggy file from the issue text alone 76% of the time, against 53% on repositories outside SWE-bench ("The SWE-Bench Illusion", [arXiv:2506.12286](https://arxiv.org/abs/2506.12286); preprint).
  - 7.8% of patches counted as correct fail the developers' own tests ("Are 'Solved Issues' in SWE-bench Really Solved Correctly?", ICSE 2026, doi:10.1145/3744916.3764576; peer-reviewed).
  - Stronger tests changed 24.4% of Verified leaderboard entries (UTBoost, [arXiv:2506.09289](https://arxiv.org/abs/2506.09289), ACL 2025; peer-reviewed).
- **Passing tests is not the same as mergeable code.** Claude 3.7 Sonnet passed maintainers' tests 38% of the time, but none of the 15 test-passing PRs METR reviewed was mergeable as-is ([METR research note](https://metr.org/blog/2025-08-12-research-update-towards-reconciling-slowdown-with-time-horizons/), August 2025; third-party).
- **Saturation hides differences** (vendor docs, [Optimizing for cost and intelligence](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence)).
  - On Anthropic's SWE-bench Pro subset "every model solves most tasks and the upgrade steps are small".
  - On Terminal-Bench 3 the same Opus line (4.7, 4.8, 5) solved 7%, 15% and 41%.
- **Harness effects** are covered in §4.2.

## 3. Better alternatives, and what each measures

### 3.1 Agentic and real-repository coding leaderboards

A background agent read these from the owners' pages and data files; I spot-checked the values marked in §0. Accessed 5 October 2026.

| Leaderboard | Owner and grade | Current? | Lists the six? | Scores | Notes |
|---|---|---|---|---|---|
| [AA Coding Agent Index](https://artificialanalysis.ai/agents/coding-agents) | Artificial Analysis; third-party | Yes (v1.5, September 2026) | 5 of 6; Opus 5.5 only at max | Model + vendor CLI + effort | Closest to thirdshift's setup (§1.3) |
| [SWE-bench Verified](https://www.swebench.com) | Benchmark owners; third-party | Effectively frozen (newest site entries February 2026) | None | Model in a fixed scaffold (mini-SWE-agent "bash-only") or submitted agents | Contaminated and partly mis-graded (§2.5). Vals: "Since performance on this benchmark has saturated, we no longer run this benchmark on new model releases" |
| [SWE-Bench Pro](https://labs.scale.com/leaderboard/swe_bench_pro) | Scale AI; vendor leaderboard | Newest entry 9 July 2026 | None | Model in SWE-Agent (or mini-swe-agent, starred), with intervals | Meta holds a minority stake in Scale (Scale's June 2025 announcement), and Meta's Muse Spark 1.1 tops it |
| [Terminal-Bench](https://www.tbench.ai/leaderboard/terminal-bench/4.0) | Benchmark owners; third-party | Yes (4.0, 21 September 2026) | 2 of 6 (Grok 4.7, Gemini 3.8 Flash) | Agent + model, 66 tasks × 5 trials | Its ± treats trials as independent; with 66 tasks the real uncertainty is wider (agent's inference) |
| [METR time horizons](https://metr.org/time-horizons/) | METR, nonprofit; third-party | Last updated 8 May 2026 | None | Model in a scaffold chosen per model | Intervals too wide to rank close models (Opus 4.6: 719 min [317, 3,634]); "Measurements above 16 hrs are unreliable" |
| [LiveCodeBench](https://livecodebench.github.io/leaderboard.html) | Benchmark owners | Stale since August 2025 | None | Model alone | Vals runs a current copy (below) |
| [Aider Polyglot](https://aider.chat/docs/leaderboards/) | Aider; practitioner | Stale since October 2025 | None | Model in Aider | n/a |
| [LMArena / Code Arena](https://arena.ai/leaderboard/text/coding) | Arena; third-party, human preference | Yes (1–2 October 2026) | All six | Model, some "in Codex harness" | Votes measure preference on web-app building, not correctness. "The Leaderboard Illusion" ([arXiv:2504.20879](https://arxiv.org/abs/2504.20879); preprint) documents private-variant testing |
| [Epoch AI ECI](https://epoch.ai/data/eci_scores.csv) | Epoch AI; third-party | Yes | 5 of 6 (no MiMo) | Statistical fit over 50+ benchmarks, one score per model with no effort split | Its software-engineering view renders client-side and was not read |
| [Vals.ai](https://www.vals.ai/benchmarks/vibe-code) | Vals; third-party | Yes (September–October 2026) | All six on Vibe Code and Terminal-Bench 4.0 | Model in mini-SWE-agent or OpenHands; some vendor-CLI rows | Reports standard errors on most boards, but not on Terminal-Bench |
| [SWE-rebench](https://swe-rebench.com) | Nebius; third-party | Yes (fresh-task windows) | None in the current window | Model in a fixed scaffold; some CLI rows | Fresh tasks limit contamination; older windows are flagged as possibly contaminated |
| DeepSWE (deepswe.datacurve.ai) | Datacurve, which builds the tasks; data vendor | Yes (22 September 2026) | 1 of 6 | All models in mini-swe-agent | Tasks written from scratch ("Contamination free") |
| [CursorBench](https://cursor.com/cursorbench) | Cursor; vendor | Yes | 4 of 6, by effort | Model in Cursor's harness | No intervals |
| [FrontierCode 1.1](https://cognition.com/frontiercode) | Cognition; vendor (its own SWE-2 is on the board) | Yes | 4 of 6 | Each model in its vendor's agent; graded on tests plus mergeability rubrics | Mean of 5 runs. Effort settings via Epoch's mirror |

**Retrodiction across these boards** (around July 2026; agent's reading, LMArena and Vals values re-checked by me):

- **Opus 4.7 clearly ahead:**
  - LMArena's human-preference boards. Code Arena on 17 July: Opus 4.7 (thinking) 1558 [1551, 1565] against GPT-5.5 (high, Codex harness) 1482 [1475, 1489].
  - Epoch and METR's MirrorCode, both at high: 31.1% vs 10.0% (current Epoch data, re-checked by me; not in the 22 July archive).
  - Narrowly, GSO: 44.1% vs 40.2%.
- **Roughly tied:**
  - Vals' own LiveCodeBench: GPT-5.5 (xhigh) 85.3 ± 1.0, Opus 4.7 (max) 85.1 ± 1.0. On the hard split, 69.7 ± 2.5 against 66.9 ± 2.5.
  - Vals SWE-bench Verified: 82.6 vs 82.0.
  - Epoch's SWE-bench Verified: Opus 4.7 at max 83.5 against GPT-5.5 at xhigh 80.6.
- **GPT-5.5 ahead:**
  - Epoch ECI on 22 July: 158.5 vs 156.1.
  - Vals Terminal-Bench 2.0 and 2.1: 73.2 vs 68.5, and 76.4 vs 68.5.
  - SWE-rebench at xhigh, in both March and April windows.
  - FrontierCode: 43.0 (xhigh) vs 38.5 (max).
- **No board showed anything like the study's 91.4% vs 71.6%.** That includes an independent LiveCodeBench run. The study's gap belongs to its own configuration: both CLIs at high effort, one sample, complete-case filtering. A leaderboard could not have revealed it.
- **The SWE-Review ordering held on every board that lists those models:**
  - Opus 4.6 > GLM-5 > Qwen3-30B-A3B on ECI, SWE-bench Verified, LMArena text coding and Terminal-Bench 2.0.
  - For example, SWE-bench Verified (mini-SWE-agent, February 2026) has Claude Opus 4.6 at 75.6 and GLM-5 (high) at 72.8.

### 3.2 Code-review benchmarks

**None lists any of the six models.** The newest models on any of them are Claude Opus 5 and GPT-5.5 (CRJudgeBench, 29 September 2026). A background agent read these; I spot-checked SWR-Bench and CRJudgeBench. Accessed 5 October 2026.

| Benchmark | Owner and grade | Maintained? | What it scores | Model or model+harness | Newest models | Notes |
|---|---|---|---|---|---|---|
| SWR-Bench ([arXiv:2509.01494](https://arxiv.org/abs/2509.01494)) | Peking University; FSE 2026, peer-reviewed | Repo last pushed June 2026; no leaderboard | 1,000 real PRs; finding the issues reviewers actually raised; F1 with a Gemini 2.5 Flash judge (κ vs humans 0.53–0.62) | Model inside fixed harnesses, one of them agentic | GPT-5, Claude 4 Opus, Gemini 2.5 Pro | Best F1 about 21%; ranking does not mirror SWE-bench or LiveCodeBench |
| SWE-PRBench ([arXiv:2603.26130](https://arxiv.org/abs/2603.26130)) | Independent researcher, vendor-adjacent; preprint | Repo last pushed March 2026 | 350 real PRs against human review comments; GPT-5.2 judge | Model alone, no tools, three context levels | Claude Sonnet 4.6, Haiku 4.5 | Top four indistinguishable; more context made every model worse |
| CR-Bench ([arXiv:2603.11078](https://arxiv.org/abs/2603.11078)) | Nutanix; preprint | One-off | 584 SWE-bench bugs recast as review tasks; recall, precision, signal-to-noise | Model inside two fixed agents | GPT-5.2 | Pushing for recall cut signal-to-noise from 5.11 to 1.95 |
| CRJudgeBench ([arXiv:2609.37216](https://arxiv.org/abs/2609.37216)) | UCL, Amazon, ZJU; preprint | Data on Hugging Face, September 2026 | Whether a review comment is trustworthy, i.e. adjudication | Model in mini-swe-agent; efforts not stated | Claude Opus 5, GPT-5.5, GLM-5.3, Kimi K3 | Open models beat Opus 5 and GPT-5.5 (§2.4) |
| SWE-Review ([arXiv:2607.06065](https://arxiv.org/abs/2607.06065)) | Huawei, NTU, HKU; preprint | No data or leaderboard released | 1,384 SWE-bench Verified PRs; approve or request changes, then resolve rate after one revision | Model in OpenHands-SDK | Claude Opus 4.6, GLM-5 | Reviewer ranking matched general capability |
| c-CRAB ([arXiv:2603.23448](https://arxiv.org/abs/2603.23448)) | NUS, ZJU, SonarSource; preprint | Repo last pushed March 2026 | 184 PRs; do generated tests pass once a review is applied | Products (Claude Code, Codex, Devin, PR-Agent); models unnamed | n/a | Claude Code 32.1%, Codex 20.1%; union of all four 41.5% |
| Martian Code Review Bench ([repo](https://github.com/withmartian/code-review-benchmark)) | Martian, a model-routing company; third-party leaderboard | Updated September 2026 | Offline: 50 PRs against golden comments, three judges. Online: developers' later fixes as ground truth | Products only: "You can't separate model quality from harness quality" | Review bots; models unnamed | Judges agree on rank but move F1 by up to about 9 points |
| Greptile ([benchmarks](https://www.greptile.com/benchmarks); [model inversion](https://www.greptile.com/blog/model-inversion)) | Greptile; vendor leaderboard and blog | 2025 benchmark not updated; inversion post July 2026 | Bugs traced back to the PRs that introduced them; recall only | Products, or each vendor's `/review` | Claude Opus 4.7, GPT-5.5 | The vendor wins its own benchmark; no false-positive scoring |

**What they show together.**

- **Absolute review quality on real PRs is low.** The best F1 is about 0.2 on SWR-Bench. Catch rates are 15–31% with diff-only context on SWE-PRBench.
- **Rankings are unstable.** On SWR-Bench, only 27 issues were found by all five runs of the same model, and changing the judge moves F1 by several points.
- **Rankings often invert general capability** (§2.4).
- **No public benchmark answers "which of these six reviews the others' code best".**

### 3.3 Measuring on thirdshift's own work

I found no published method specifically for "reviewer A on author B's code", though the agents' search was not exhaustive. The pieces exist separately.

**Design**

- **Hold the draft fixed.**
  - Both candidate reviewers review the same diffs: each author's real PRs, reviewed by both.
  - Pair results by draft. Independently sampled pipelines confounded the July study (§1.4 of [multi-family-review.md](multi-family-review.md)).
  - Greptile (500 PRs per author family × 2 reviewers × 3 runs) and SWE-Review (3 generators × several reviewers, one run) are the worked examples.
- **Run the configuration you will deploy:** the same CLI, effort, prompt and adjudication. The harness and effort effects in §4 are as large as most model differences.
- **Shadow mode first.** Log the candidate's findings without acting on them. Anthropic's advice: "Run the winner in shadow on a traffic slice before cutover" ([Optimizing for cost and intelligence](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence); vendor docs).

**Ground truth**

- **Count a finding as real when a test, a later fix or a revert confirms it.**
  - Greptile built its set from "sentiment analysis, upvote/downvote ratios, and git archaeology".
  - Martian's methodology traces bug-labelled issues to their fix PRs, then uses `git blame` to find the change that introduced the bug.
- **The standard tool for that tracing, SZZ, is noisy.**
  - Against a developer-informed oracle, the best SZZ variant reached precision 0.66 and recall 0.57 (Rosa et al., ICSE 2021, [arXiv:2102.03300](https://arxiv.org/abs/2102.03300); peer-reviewed).
  - On 76,046 Linux `Fixes:` pairs, every variant landed near F1 0.5, and 17.5% of fixes could not be traced at all (Lyu et al., IEEE TSE, [arXiv:2308.05060](https://arxiv.org/abs/2308.05060); peer-reviewed).
  - Use it to find candidates, then confirm by hand.
- **Prevent leakage.** The reviewer's checkout must not contain the later fix. Anthropic reports Claude "gaining an unfair advantage on some tasks by examining the git history from previous trials" ([Demystifying evals for AI agents](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents), 9 January 2026; vendor blog).
- **"Acted on" is a weaker proxy.** A change after a comment does not prove the comment found a real bug.
  - Google's AutoCommenter counted comments resolved by merge: about 40% after a hand check ([arXiv:2405.13565](https://arxiv.org/abs/2405.13565), AIware 2024; peer-reviewed).
  - Atlassian's RovoDev counted comments followed by code changes: 38.7%, against 44.5% for human comments ([arXiv:2601.01129](https://arxiv.org/abs/2601.01129), ICSE-SEIP 2026; peer-reviewed).
- **Seeded bugs are cheap but optimistic.** One review study scored F1 0.847 on synthetic mutations and 0.066 on real PRs ("Bigger Isn't Always Better"; preprint).

**Blind grading**

- **Hide which model wrote each finding,** and grade findings rather than whole reviews.
- **Prefer pass/fail or pairwise judgments.** OpenAI recommends a "randomized, blinded test" for human grading and warns of position and verbosity bias in LLM judges ([Evaluation best practices](https://developers.openai.com/api/docs/guides/evaluation-best-practices); vendor docs).
- **Validate any LLM matcher on a hand-labelled sample.**
  - SWR-Bench's judge, which decides finding by finding whether each matches a known issue, agreed with humans about as often as humans agreed with each other (κ 0.53–0.62 vs 0.56–0.63).
  - A judge giving each review a holistic score reached only 0.31–0.45.
  - Two independent human labellers of LLM review comments still disagreed on about 20% of usefulness labels (Crupi et al., ICPC 2026, [arXiv:2602.11925](https://arxiv.org/abs/2602.11925); peer-reviewed).
- **Read transcripts.** Opus 4.5 first scored 42% on CORE-Bench. After Anthropic fixed grading bugs and ambiguous tasks, and loosened the scaffold, it scored 95% (Anthropic, Demystifying evals).

**Sample size**

- **Treat each known bug as a paired item:** reviewer A caught it or not, reviewer B caught it or not.
  - Test with McNemar's test or the paired standard error.
  - Cluster the errors when several bugs share a PR (Miller, "Adding Error Bars to Evals", [arXiv:2411.00640](https://arxiv.org/abs/2411.00640); Anthropic preprint).
- **Items needed for 80% power at two-sided α = .05.** Exact McNemar, the background agent's computation. My normal-approximation check gives 5–8% fewer items. ψ is the share of items where exactly one reviewer catches the bug.

| True difference in catch rate | ψ = 0.2 | ψ = 0.3 | ψ = 0.4 |
|---|---|---|---|
| 5 points | 658 | 975 | 1,291 |
| 10 points | 168 | 249 | 329 |
| 20 points | n/a | 61 | 84 |

- **Small samples only detect large gaps.**
  - With 50 items the smallest reliably detectable difference is 18–26 points; with 100 items, 13–19 points.
  - The July study (n = 116) had about 27% power for its cross- vs same-family contrast.
  - Anthropic's advice that "20-50 simple tasks drawn from real failures is a great start" fits finding large gaps, not ranking near-peers.
- **Repeats help.** Sampling each item several times cuts noise. In Miller's example at n = 198, going from 1 to 10 samples per item lowered the minimum detectable difference from 13.2 to 7.5 points.
- **Overlap estimates.** From how much two reviewers' findings overlap, capture–recapture methods from the software-inspection literature estimate the total defect count and each reviewer's detection rate (Briand et al., IEEE TSE 26(6), 2000). Paywalled; read only in summary.

## 4. Effort and harness

### 4.1 How much effort moves a model

**On AA's Intelligence Index** (same model, AA's harness):

| Model | Lowest → highest effort listed | Spread |
|---|---|---|
| Claude Opus 5.5 | low 42.3 → max 57.6 (medium 51.2) | 15.3 |
| GPT-6.1 Sol | low 42.1 → max 51.8 (xhigh 51.0) | 9.7 |
| Gemini 3.8 Flash | low 33.5 → high 40.9 | 7.4 |
| Grok 4.7 | low 42.2 → xhigh 46.4 (no medium row) | 4.2 |
| Muse Spark 1.3 | xhigh 45.1 → max 48.1 | 3.0 |
| MiMo-V2.6-Pro | one row | n/a |

**On AA's Coding Agent Index** (model in its vendor's CLI):

- **Claude Sonnet 5.5 in Claude Code:** low 42.1, medium 45.9, high 55.0, xhigh 62.9, max 68.4. A 26-point spread.
- **GPT-6.1 Sol in Codex:** low 57.2, medium 61.4, high 60.1, xhigh 62.9, max 60.1. A 5.7-point spread, not monotonic. AA: "We observed the xhigh effort setting to outperform the max effort setting by 3 points" ([article](https://artificialanalysis.ai/articles/gpt-6-1-sol-replaces-gpt-6-sol-after-just-7-days-with-near-astra-intelligence), 29 September 2026).
- **Opus 5.5 in Claude Code** has only a max row (66.0). There is no public agentic score for Opus 5.5 at medium.

**Vendor measurements and guidance** (vendor docs):

- **Anthropic, Claude Opus 5.5, internal SWE-bench Pro subset.** 478 problems, "not comparable to the public leaderboard", run 19–20 September 2026 ([Optimizing for cost and intelligence](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence)).
  - Measured against high: medium "about 2.5 points lower … for about 70% of the cost"; low "about 8 points lower … for about a third of the cost"; xhigh "about 1.4 points higher for 2.5 times the cost of high".
  - Absolute rates: low 87.4%, medium 92.8%, high about 95.3%.
  - "Long-horizon coding is where effort genuinely buys accuracy". On research and knowledge-work benchmarks the curve was "nearly flat".
  - Opus 5.5's API default is medium ([What's new in Claude Opus 5.5](https://platform.claude.com/docs/en/models/opus-5-5/whats-new-opus-5-5)).
- **OpenAI: guidance, no numbers** ([reasoning guide](https://developers.openai.com/api/docs/guides/reasoning)).
  - GPT-6.1 Sol supports low, medium (the API default), high, xhigh and max.
  - xhigh is for agentic tasks with long runs: "Only use when your evals show a clear benefit … Common use cases include security and code review".
- **Google.** Gemini 3.8 Flash has thinking levels low, medium (default) and high ([Gemini thinking docs](https://ai.google.dev/gemini-api/docs/thinking)). No measured sweep found.
- **SpaceXAI (xAI).** Grok 4.7 has low, medium, high (the default) and xhigh ([reasoning guide](https://docs.x.ai/docs/guides/reasoning)). AA scored low, high and xhigh only.

**Reading.** Within one model, effort moves the AA index by 3–15 points and the agentic index by up to 26. That is as much as the gap between most pairs of the six models. A ranking read at the wrong effort can be wrong. A reviewer run at lower effort than the author can be "the weaker model" even when it is the same model.

### 4.2 How much the harness moves a model

**Same model, same effort, same benchmark, different harness** (my tabulation from AA's pages). Terminal-Bench 4.0 runs in AA's Intelligence Index under mini-swe-agent, and in the Coding Agent Index under each vendor's CLI. Each is 66 tasks × 3 attempts.

| Model (effort) | mini-swe-agent | Vendor or other CLI | Δ |
|---|---|---|---|
| Claude Opus 5.5 (max) | 59.6 | Claude Code 63.1 | +3.5 |
| Claude Sonnet 5.5 (low / medium / high / xhigh / max) | 20.7 / 29.8 / 43.9 / 57.1 / 63.6 | Claude Code 25.3 / 27.3 / 41.9 / 58.1 / 66.2 | +4.6 / −2.5 / −2.0 / +1.0 / +2.6 |
| GPT-6.1 Sol (low / medium / high / xhigh / max) | 30.8 / 48.0 / 51.5 / 54.0 / 56.1 | Codex 49.0 / 51.5 / 50.0 / 54.5 / 53.0 | **+18.2** / +3.5 / −1.5 / +0.5 / −3.1 |
| Gemini 3.8 Flash (high) | 19.7 | Antigravity SDK 14.6 | −5.1 |
| Grok 4.7 (xhigh) | 25.8 | Grok Build 33.3 | +7.5 |
| Muse Spark 1.3 (max) | 33.3 | Muse Code 31.8 | −1.5 |
| Qwen3.8 Max | 38.9 | Claude Code (another vendor's CLI) 16.7 | **−22.2** |

- **Most native-harness differences are within ±5 points,** about the noise floor for 66 tasks.
- **The exceptions are large.** GPT-6.1 Sol at low effort gains 18 points in Codex, Grok 4.7 gains 7.5 in Grok Build, and Qwen3.8 Max in Claude Code loses 22.
- **Caveats.** The Coding Agent Index zeroes reward-hacked attempts, and the Intelligence Index methodology does not say whether it does. The runs were on different dates.

**Same model and effort, several harnesses** (AA Coding Agent Index, 21 July 2026, archived):

- **Claude Opus 4.7 (medium).**
  - Claude Code: 37.5 (DeepSWE 27.4).
  - OpenCode: 45.5 (DeepSWE 39.5).
  - Cursor CLI: 41.2 (DeepSWE 31.6).
  - An 8-point spread on the index and 12 on DeepSWE.
- **GPT-5.5 (medium).**
  - Codex: 50.4 (DeepSWE 56.6).
  - Cursor CLI: 42.8 (DeepSWE 37.2).
  - 7.6 points on the index and 19.4 on DeepSWE.

**Other leaderboards: neutral harness against vendor CLI** (agent's reading of the owners' data; third-party leaderboards):

| Model | Benchmark | Neutral harness | Vendor CLI | Source |
|---|---|---|---|---|
| GPT-5.5 | SWE-bench Verified | 82.6 (mini-SWE-agent) | Codex 76.4 | Vals |
| GPT-5.5 | Terminal-Bench 2.1 | 76.4 | Codex 57.3 | Vals |
| GPT-5.5 | Vibe Code Bench | 69.8 (OpenHands) | Codex 58.2 | Vals |
| Claude Opus 4.8 | SWE-bench Verified | 88.6 | Claude Code 85.8 | Vals |
| Claude Opus 4.8 | Vibe Code Bench | 82.7 ± 3.1 | Claude Code 77.5 ± 3.7 | Vals |
| Claude Sonnet 4.6 | Vibe Code Bench | 51.5 (OpenHands) | Claude Code 55.8 (higher) | Vals |
| Claude Fable 5 (high) | SWE-rebench, same 111 problems | 64.5 ± 1.4 (own scaffold) | Claude Code 60.4 ± 1.0 | SWE-rebench |
| GPT-5.6 Sol (medium) | SWE-rebench, same 111 problems | 62.3 ± 1.8 | Codex 58.0 ± 1.3 | SWE-rebench |

- **On these boards the vendor CLI usually scored lower,** by 3–19 points. Sonnet 4.6 on Vibe Code was the exception. On AA's Terminal-Bench 4.0 comparison above, the vendor CLI was usually level or higher.
- **One reason given by SWE-rebench's owners:** vendor CLIs sometimes treated an issue as informational and produced empty patches, until an explicit instruction to change the code was added. Instructions written for one harness do not transfer to another.
- **Spread across agents on Terminal-Bench 2.0** (archived 17 July 2026):
  - Opus 4.6 ran from 58.0 in Claude Code to 76.4 in the best third-party agent.
  - GPT-5.5 ran from 66.1 to 84.7.

**The same model and effort, Terminal-Bench 4.0, four runners:**

| Model (effort) | AA, mini-swe-agent | Vals, mini-SWE-agent | tbench.ai, official | AA, vendor CLI |
|---|---|---|---|---|
| Grok 4.7 (xhigh) | 25.8 | 28.8 | 37.6 (Grok Build) | 33.3 (Grok Build) |
| Claude Fable 5.1 (max) | 52.0 | 58.1 | 57.9 (Claude Code) | 57.6 (Claude Code) |
| GPT-6 Astra (max) | 59.1 | 59.6 | 58.2 (Codex) | 55.6 (Codex) |
| Gemini 3.8 Flash (high) | 19.7 | 19.2 | 19.1 (mini-SWE-agent) | 14.6 (Antigravity SDK) |
| Muse Spark 1.3 (max) | 33.3 | 24.7 | not listed | 31.8 (Muse Code) |

- **Same harness, different runners:** these agree closely (Gemini 3.8 Flash), or differ by 8.6 points (Muse Spark 1.3).
- **Switching harness** moved scores by about −5 to +12 points, in a direction that depended on the model.

**Lewis, "Same Model, Different Harness"** ([arXiv:2608.26218](https://arxiv.org/abs/2608.26218), 26 August 2026; single-author preprint by the builder of the harness tested).

- **Design.** One harness in two configurations, holding the model fixed.
  - The treatment shortens old tool results, adds a stall detector, and adds command safeguards.
  - Model: Qwen3.6-35B-A3B at 4 bits, greedy decoding, one run per task per arm. Paired McNemar and sign tests.
- **Results under a tight context window:**
  - SWE-bench Verified (169 tasks, 20,480-token window): complete solutions 43 → 72.
  - SWE-bench Pro (316 tasks, 49,152-token window): 31 → 72.
  - Both paired McNemar p < .0001.
  - Repository-level Holm correction: the Verified resolution gain loses significance. The gains in fail-to-pass fraction and in Pro resolution stay significant.
- **With ample context (262,144 tokens) the effect vanished.** On Verified, complete solutions were 102 vs 101, and the fail-to-pass difference was −0.3 points [−4.5, +3.9].
- **A side check on frontier models.** GPT-5.5 through the Codex CLI and through mini-SWE-agent agreed on 87.6% of the 500 Verified task outcomes (κ 0.66).
- **Limits the author states:** local 4-bit open-weight models only, settings tuned on the same benchmark families, no run-to-run variance.
- **Reading.** Harness design matters most when context is the binding constraint. For frontier models with large windows, the measured harness effects above come from prompts, tools and defaults as much as from context handling.

**What this means for thirdshift.**

- **thirdshift runs models through vendor CLIs at a configured effort.** An API-run score such as the Intelligence Index measures a different configuration.
- **The size and even the sign of the harness effect vary.**
  - The vendor's own CLI was usually within 5 points of AA's harness on terminal tasks.
  - It was usually 3–19 points below a neutral harness on Vals and SWE-rebench.
  - On DeepSWE the spread across CLIs was 12–19 points in July.
- **The harness effect is as large as most gaps between models.** It does not cancel when comparing two models that run in different CLIs, as an Opus author and a GPT reviewer do.
- **Rank the configuration, not the model.** A reviewer should be ranked as model + CLI + effort.

## 5. What this does not tell you

- **Whether any public number predicts review strength for these six models.** No benchmark scores them as reviewers. The review benchmarks that exist test older models (§3.2). The two studies behind the "stronger reviewer" rule each test a handful of pairs.
- **Whether "at least as strong" is the right threshold.**
  - SWE-Review's near-peer reviewer (Opus 4.6 on GLM-5's PRs) gained +3.0, against +2.8 for GLM-5 reviewing itself: equal, not better.
  - The July study's same-strength self-review of Claude gained nothing.
  - The evidence supports "not much weaker". It does not say how much stronger a reviewer must be to add value.
- **Whether strength as an author equals strength as a reviewer.** Every leaderboard here scores solving tasks. Finding defects in someone else's diff without raising false positives is a different skill, and it diverges from generation ability most when the author is the stronger model (§2.4).
- **Whether any ordering holds on thirdshift's tasks.** "Stronger" changed with the task in the July case, and rankings move with effort and harness (§4).
- **What Claude Opus 5.5 scores at medium in Claude Code.** AA's agentic index lists only max. Anthropic's effort sweep is internal and on a saturated subset.
- **How AA estimates the scores it marks "* Estimated",** and what its "Under review" badges on SciCode and CritPt will lead to.
- **How long any of this lasts.** AA has published eleven index versions since January 2026. A new release of either the author or the reviewer model reopens the question.
- **Intervals.** AA's ±1 claim for its index is unpublished. Most other numbers here are single runs with no interval reported.

## 6. Implications for thirdshift (recommendation, not evidence)

This section is my recommendation. It rests on the evidence above but goes beyond it.

1. **Use the AA Intelligence Index as a coarse filter, not a ranking.**
   - A gap of 8+ points, at the efforts thirdshift actually runs, is a usable sign that one model is stronger. Below about 5 points, treat the pair as unranked.
   - Pairs with large gaps today:
     - Opus 5.5 at medium (51.2) or GPT-6.1 Sol at xhigh (51.0) against Gemini 3.8 Flash (40.9 or lower).
     - Either of those against Opus 5.5 or GPT-6.1 Sol at low (about 42).
   - Pairs too close to call:
     - Opus 5.5 at medium against GPT-6.1 Sol at xhigh (51.2 vs 51.0), the pair thirdshift runs most.
     - Grok 4.7, Muse Spark 1.3 and MiMo-V2.6-Pro against each other (45–48).
2. **Prefer the measure closest to the job.**
   - For a reviewer that runs inside a vendor CLI on a repository, AA's Coding Agent Index (model + CLI + effort) and its DeepSWE component are closer than the Intelligence Index. Read the row at the effort you will configure.
   - Where AA has no row (Opus 5.5 at medium, MiMo Code, Gemini 3.8 Flash in the agy CLI), the public data cannot rank that configuration.
   - Where AA and Epoch disagree (Gemini 3.8 Flash against Grok 4.7), treat the pair as unranked.
3. **Run the reviewer at least at the author's effort, and consider one step higher.**
   - Effort moved Opus 5.5 by 15 index points and Sonnet 5.5 in Claude Code by 26 agentic points.
   - OpenAI names code review as an xhigh use case.
   - A same-model reviewer at lower effort than the author is the easiest way to end up with the weaker reviewer of the SWE-Review −7.3 cell.
4. **Settle near-peer pairs on thirdshift's own Runs, pairwise** (§3.3).
   - Each candidate reviews the other's diffs on the same issues.
   - Graders see findings without knowing which model wrote them.
   - Ground truth comes from tests, later fixes and reverts.
   - Size the sample for the difference that would change the configuration.
   - This is the only measurement that holds task, harness, effort and adjudication at thirdshift's values.
5. **Re-check on every model or index release.** Record the AA version and access date beside any number thirdshift uses in configuration or docs.

## Sources

All accessed 5 October 2026 unless dated otherwise. "Archived" means read from the Wayback Machine at the snapshot named.

**Artificial Analysis (third-party leaderboard)**

- LLM leaderboard: https://artificialanalysis.ai/leaderboards/models
- Intelligence benchmarking methodology, v4.3.2: https://artificialanalysis.ai/methodology/intelligence-benchmarking
- Methodology index and definitions: https://artificialanalysis.ai/methodology
- Capability indexes methodology: https://artificialanalysis.ai/methodology/capability-indices
- Coding Agent Index methodology, v1.5: https://artificialanalysis.ai/methodology/coding-agents-benchmarking
- Coding agents leaderboard: https://artificialanalysis.ai/agents/coding-agents
- Intelligence Index evaluation page: https://artificialanalysis.ai/evaluations/artificial-analysis-intelligence-index
- LiveCodeBench evaluation page (legacy): https://artificialanalysis.ai/evaluations/livecodebench
- Model pages:
  - https://artificialanalysis.ai/models/claude-opus-5-5-medium (embedding the records of all models)
  - https://artificialanalysis.ai/models/claude-opus-4-7
  - https://artificialanalysis.ai/models/gpt-5-5-high
  - https://artificialanalysis.ai/models/claude-opus-4-6-adaptive
  - https://artificialanalysis.ai/models/glm-5
  - https://artificialanalysis.ai/models/qwen3-coder-30b-a3b-instruct
  - https://artificialanalysis.ai/models/qwen3-30b-a3b-instruct-reasoning
- API reference: https://artificialanalysis.ai/api-reference
- About: https://artificialanalysis.ai/about
- Articles:
  - Intelligence Index v4.3 (7 September 2026): https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3
  - Claude Opus 5.5 (22 September 2026): https://artificialanalysis.ai/articles/claude-opus-5-5
  - Grok 4.7 (21 September 2026): https://artificialanalysis.ai/articles/benchmarking-grok-4-7
  - GPT-6.1 Sol (29 September 2026): https://artificialanalysis.ai/articles/gpt-6-1-sol-replaces-gpt-6-sol-after-just-7-days-with-near-astra-intelligence
- Archived:
  - Claude Opus 4.7 model page, 19 July 2026: https://web.archive.org/web/20260719232550/https://artificialanalysis.ai/models/claude-opus-4-7
  - GPT-5.5 (high), 25 July 2026: https://web.archive.org/web/20260725000233/https://artificialanalysis.ai/models/gpt-5-5-high
  - GPT-5.5 (xhigh), 23 July 2026: https://web.archive.org/web/20260723174348/https://artificialanalysis.ai/models/gpt-5-5
  - Methodology v4.1, 17 July 2026: https://web.archive.org/web/20260717210114/https://artificialanalysis.ai/methodology/intelligence-benchmarking
  - Model records as of 1 July 2026, via the Qwen3-Coder page: https://web.archive.org/web/20260701091615/https://artificialanalysis.ai/models/qwen3-coder-30b-a3b-instruct
  - Claude Opus 4.6, 19 June 2026: https://web.archive.org/web/20260619052400/https://artificialanalysis.ai/models/claude-opus-4-6-adaptive
  - Coding agents leaderboard, 21 July 2026: https://web.archive.org/web/20260721230023/https://artificialanalysis.ai/agents/coding-agents
  - LLM leaderboard, 15 October 2025: https://web.archive.org/web/20251015143059/https://artificialanalysis.ai/leaderboards/models

**Other leaderboards and data**

- Epoch AI ECI scores: https://epoch.ai/data/eci_scores.csv
- Epoch AI per-benchmark scores (MirrorCode, GSO, FrontierCode and others): https://epoch.ai/data/eci_benchmarks.csv
- MirrorCode (Epoch AI with METR): https://epoch.ai/benchmarks/mirrorcode
- AtCoder Problems contest list (third-party mirror of AtCoder contest data): https://kenkoooo.com/atcoder/resources/contests.json
- Martian Code Review Bench: https://github.com/withmartian/code-review-benchmark
- Greptile benchmarks: https://www.greptile.com/benchmarks; model inversion (21 July 2026): https://www.greptile.com/blog/model-inversion
- Epoch AI benchmark data archived 22 July 2026 (`benchmark_data.zip`, `epoch_capabilities_index.csv`)
- SWE-bench: https://www.swebench.com, with data at https://raw.githubusercontent.com/SWE-bench/swe-bench.github.io/master/data/leaderboards.json
- Scale AI, SWE-Bench Pro (vendor leaderboard): https://labs.scale.com/leaderboard/swe_bench_pro
- Terminal-Bench: https://www.tbench.ai/leaderboard/terminal-bench/4.0, and Terminal-Bench 2.0 archived 17 July 2026
- METR time horizons: https://metr.org/time-horizons/
- LiveCodeBench: https://livecodebench.github.io/leaderboard.html
- Aider Polyglot: https://aider.chat/docs/leaderboards/
- LMArena (now arena.ai): https://arena.ai/leaderboard/text/coding, the WebDev/Code Arena board, and the Code Arena archived 17 July 2026
- Singh et al., "The Leaderboard Illusion": https://arxiv.org/abs/2504.20879
- Vals.ai:
  - LiveCodeBench, including the per-model settings in the page data: https://www.vals.ai/benchmarks/lcb
  - Vibe Code Bench: https://www.vals.ai/benchmarks/vibe-code
  - Terminal-Bench 4.0: https://www.vals.ai/benchmarks/terminal-bench-4
  - Terminal-Bench 2.1: https://www.vals.ai/benchmarks/terminal-bench-2-1
  - SWE-bench: https://www.vals.ai/benchmarks/swebench
- SWE-rebench (Nebius): https://swe-rebench.com
- Datacurve DeepSWE (data-vendor leaderboard): https://deepswe.datacurve.ai
- CursorBench (vendor): https://cursor.com/cursorbench
- Cognition FrontierCode 1.1 (vendor): https://cognition.com/frontiercode

**Studies retrodicted**

- Xiang et al., "Cross-Model LLM Code Review", arXiv:2607.21656, and artifact https://github.com/shawnzxiang/cross-model-review-code (`experiments/configs/all_conditions.yaml`, `harness/core/cli_runner.py`)
- Wang et al., "SWE-Review", arXiv:2607.06065: https://arxiv.org/abs/2607.06065

**Code-review benchmarks**

- SWR-Bench, FSE 2026: https://arxiv.org/abs/2509.01494 (doi:10.1145/3808144)
- SWE-PRBench: https://arxiv.org/abs/2603.26130
- CR-Bench: https://arxiv.org/abs/2603.11078
- CRJudgeBench: https://arxiv.org/abs/2609.37216
- c-CRAB: https://arxiv.org/abs/2603.23448
- "Bigger Isn't Always Better": https://arxiv.org/abs/2606.15689

**Benchmark agreement, critique and verification**

- Ruan, Maddison, Hashimoto, "Observational Scaling Laws", NeurIPS 2024: https://arxiv.org/abs/2405.10938
- Ho et al., "A Rosetta Stone for AI Benchmarks": https://arxiv.org/abs/2512.00193
- Perlitz et al., "BenchBench": https://arxiv.org/abs/2407.13696
- Lin et al., "CriticBench", Findings of ACL 2024: https://arxiv.org/abs/2402.14809
- Song et al., "Mind the Gap", ICLR 2025: https://arxiv.org/abs/2412.02674
- Zhou et al., "Variation in Verification", ICLR 2026: https://arxiv.org/abs/2509.17995
- Tang et al., "RealCritic": https://arxiv.org/abs/2501.14492
- McAleese et al., "LLM Critics Help Catch LLM Bugs" (CriticGPT): https://arxiv.org/abs/2407.00215

**Saturation and contamination**

- "The SWE-Bench Illusion": https://arxiv.org/abs/2506.12286
- "Are 'Solved Issues' in SWE-bench Really Solved Correctly?", ICSE 2026: doi:10.1145/3744916.3764576
- UTBoost, ACL 2025: https://arxiv.org/abs/2506.09289
- METR, research update (12 August 2025): https://metr.org/blog/2025-08-12-research-update-towards-reconciling-slowdown-with-time-horizons/

**Evaluation methodology**

- Miller, "Adding Error Bars to Evals": https://arxiv.org/abs/2411.00640
- Rosa et al., "Evaluating SZZ Implementations Through a Developer-informed Oracle", ICSE 2021: https://arxiv.org/abs/2102.03300
- Lyu et al., "Evaluating SZZ Implementations: An Empirical Study on the Linux Kernel", IEEE TSE: https://arxiv.org/abs/2308.05060
- AutoCommenter, AIware 2024: https://arxiv.org/abs/2405.13565
- RovoDev Code Reviewer, ICSE-SEIP 2026: https://arxiv.org/abs/2601.01129
- Crupi, Tufano, Bavota, ICPC 2026: https://arxiv.org/abs/2602.11925
- Briand et al., capture–recapture for inspections, IEEE TSE 26(6), 2000: doi:10.1109/32.852741 (paywalled; read in summary only)
- Anthropic, "Demystifying evals for AI agents" (9 January 2026): https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
- Anthropic docs, "Define success criteria and build evaluations": https://platform.claude.com/docs/en/test-and-evaluate/develop-tests
- OpenAI, "Evaluation best practices": https://developers.openai.com/api/docs/guides/evaluation-best-practices

**Vendor documentation**

- Anthropic:
  - Optimizing for cost and intelligence: https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence
  - Refusals and fallback: https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback
  - What's new in Claude Opus 5.5: https://platform.claude.com/docs/en/models/opus-5-5/whats-new-opus-5-5
  - Models overview: https://platform.claude.com/docs/en/about-claude/models/overview
  - Claude Opus 4.7: https://platform.claude.com/docs/en/models/opus-4-7/overview
- OpenAI:
  - Reasoning guide: https://developers.openai.com/api/docs/guides/reasoning
  - Latest model guide: https://developers.openai.com/api/docs/guides/latest-model
  - GPT-6.1 Sol: https://developers.openai.com/api/docs/models/gpt-6.1-sol
  - GPT-5.5: https://developers.openai.com/api/docs/models/gpt-5.5
- Google, Gemini thinking: https://ai.google.dev/gemini-api/docs/thinking
- SpaceXAI:
  - Reasoning: https://docs.x.ai/docs/guides/reasoning
  - Grok 4.7: https://docs.x.ai/docs/models/grok-4.7

**thirdshift files**

- [multi-family-review.md](multi-family-review.md)
- [codex-headless-harness.md](codex-headless-harness.md)
