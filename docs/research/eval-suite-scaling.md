# Scaling the Eval suites: tasks, Trials, synthetic bugs and contamination

Research date: 2026-10-27.

**Question.** Three questions about growing thirdshift's [Eval suites](../../GLOSSARY.md) past their present size:

1. **Tasks vs Trials.** At 15–30 Eval tasks, does another Trial per Eval task or another Eval task buy more statistical power per token? What does our own A/A data say, and what do the primary sources recommend?
2. **Synthetic Eval tasks.** Can code-review (and a future implement) suite be grown by *injecting* bugs rather than waiting for real ones? How valid are seeded faults compared with real ones?
3. **Contamination.** `JacobStephens2/thirdshift` is public and every Eval task's fix is in its history. How do others detect and mitigate memorisation, and what is the cheapest probe for our Eval tasks?

**Method.** Each claim is followed back to the party that owns it.

- **Statistics.** Evan Miller (Anthropic), [*Adding Error Bars to Evals*](https://arxiv.org/abs/2411.00640), arXiv 2411.00640v1, 1 November 2024. The PDF was downloaded and read in full as text; every formula and section number below is from it.
- **Agent-eval practice.** Anthropic's engineering post [*Demystifying evals for AI agents*](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents), read live on 2026-10-27. Quotes are tagged with the post's own headings.
- **pass^k.** Yao et al., [*τ-bench*](https://arxiv.org/abs/2406.12045), arXiv 2406.12045v1, read as HTML. Its pass^2…pass^7 values exist only as a plotted curve (Figure 4) and are therefore not quoted.
- **Benchmarks built from real pull requests.** [SWE-bench](https://arxiv.org/html/2310.06770v3) (arXiv 2310.06770v3), [SWE-bench-Live](https://arxiv.org/html/2505.23419v2) (arXiv 2505.23419v2), [SWE-Gym](https://arxiv.org/html/2412.21139v1) (arXiv 2412.21139v1), all read as arXiv HTML.
- **Synthetic bugs at scale.** [SWE-smith](https://arxiv.org/html/2504.21798v1) (arXiv 2504.21798v1), read as arXiv HTML.
- **Rust mutation tooling.** [cargo-mutants' own book](https://mutants.rs/), the [mutation-operator](https://mutants.rs/mutants.html) and [limitations](https://mutants.rs/limitations.html) chapters, read live on 2026-10-27.
- **SWE-bench Verified could not be read.** `openai.com` returns **HTTP 403** to every automated fetch from this network, including the post itself and its localised mirrors; `web.archive.org` is blocked here. The figures in §1.6 therefore come from a search engine's reading of that first-party page, restricted to `openai.com`, and the Hugging Face [dataset card](https://huggingface.co/datasets/princeton-nlp/SWE-bench_Verified), which says only that the 500 samples "have been human-validated for quality". Treat the percentages as high-confidence paraphrase, not verbatim, and re-read the page before quoting it anywhere user-facing. The annotation files on Hugging Face are third-party re-uploads, not OpenAI's release.
- **Our own data.** Read read-only from the `thirdshift` host (`ssh jacob@thirdshift.app`): the two A/A Experiments' full per-Trial results (`thirdshift experiments <id> --json` for `560af5c5…` and `199d80f2…`) and the suite manifests under `~/.thirdshift/evals/suites/`. Nothing was started, resumed or modified there. The Bar's reasoning comes from [#1252](https://github.com/JacobStephens2/thirdshift/issues/1252), [#1267](https://github.com/JacobStephens2/thirdshift/issues/1267), [#1370](https://github.com/JacobStephens2/thirdshift/issues/1370) and [#1434](https://github.com/JacobStephens2/thirdshift/issues/1434), and the code from [`src/verdict.rs`](../../src/verdict.rs).
- **Simulation.** The paired Dirichlet Verdict of `src/verdict.rs` was reimplemented in about 30 lines of Python and checked against the published intervals before being used (§1.4). Scripts ran under `python3 -I` in a scratch directory outside the repository; nothing was kept.
- **[interpretation]** marks my reading rather than a documented fact.

**Out of scope.** Which eval framework runs the Trials ([eval-runners.md](eval-runners.md) settled that: thirdshift's own Rust runner). Grader choice and calibration ([model-strength-for-review.md](model-strength-for-review.md)). Sandboxing ([eval-sandboxes.md](eval-sandboxes.md)). The Scoring run, which watches production rather than changes. Whether `thirdshift-code-review` should grow a correctness axis; [#1370](https://github.com/JacobStephens2/thirdshift/issues/1370) decided it stays two-axis and [#1371](https://github.com/JacobStephens2/thirdshift/issues/1371) owns the rest.

## TL;DR

- **Per token, more Eval tasks beat more Trials, and this follows from Miller's own sample-size formula.** His Equation 9 sizes a paired comparison as `n = (z_α/2 + z_β)² (ω² + σ_A²/K_A + σ_B²/K_B) / δ²`, where `ω²` is the variance of the *per-task effect of the change* and `σ²` the within-task Trial variance. Total Trials per Arm is `N = nK`, so `N = (z_α/2 + z_β)² (Kω² + 2σ²) / δ²` — **increasing in `K`**. More Trials per Eval task is therefore never cheaper than more Eval tasks for detecting a mean shift, and is strictly worse whenever the change helps some Eval tasks more than others. §1.3.
- **Our A/A data agrees, with one exception.** Estimated from the per-Trial results, code-review's within-task variance is `E[σ²] = 0.043` against a between-task variance of `Var(x) = 0.126`; at the suite's three Trials, within-task noise is **10% of the per-task variance**, so Miller's stopping condition (`E[σ²]/K ≪ Var(x)`) is already satisfied and a fourth Trial would buy almost nothing. Spec review at one Trial is the exception: within-task noise is **43%** of the total there. §1.2.
- **Eval tasks that never pass do not merely waste tokens — they launder regressions past the Bar.** Simulating the real Verdict: 10 solvable Eval tasks plus 30 always-failing ones pass a Candidate carrying a genuine regression **63% of the time**, against **3%** for the same 10 Eval tasks unpadded, because padding shrinks the mean paired difference (−0.205 → −0.052) against a fixed −0.10 margin while leaving the regression's detectability untouched. This is the strongest argument for #1434's admission rule. §1.5.
- **The floor is most of today's suite.** Across both A/A Experiments, 7 of code review's 10 Eval tasks (14 of 17 in the earlier run) never passed in 6 Trials, and 17 of Spec review's 30 never passed in 4. Code review measures 3 Eval tasks and Spec review about 9. §1.2.
- **Anthropic's own post says a zero pass rate is usually a broken Eval task, not a hard one:** "a 0% pass rate across many trials (i.e. 0% pass@100) is most often a signal of a broken task". It also splits evals into capability evals, which "should start at a low pass rate", and regression evals, which "should have a nearly 100% pass rate" — and thirdshift uses one suite built like a capability eval to do a regression eval's job. §1.6, Recommendation 7.
- **SWE-bench Verified is the precedent for screening Eval tasks, and the rejection rate is high:** 93 developers screened 1,699 samples down to 500; roughly **38%** were flagged for under-specified problem statements and **61%** for tests that could reject valid solutions, with three annotators per sample ensembled by taking the worst severity. §1.6 — but see the Method note: that page could not be read from here.
- **Synthetic bugs are how everyone else escapes the real-PR supply limit, and the scale difference is three orders of magnitude.** SWE-smith produces **50,137** task instances from 128 repositories by five strategies (LM Modify, LM Rewrite, procedural AST transformation, combining bugs, and PR mirroring), validated the same way as real ones: "only keep patches that break one or more existing, passing tests". We have 81 closed `bug` issues and [#1252](https://github.com/JacobStephens2/thirdshift/issues/1252) found only 12 usable pairs in the 39 it examined. §2.1, §2.4.
- **Seeded faults are a defensible but imperfect proxy, and the literature is explicit about where they fail.** Just et al., on 357 real Java faults: mutant detection and real-fault detection are correlated "independently of code coverage" and 73% of real faults are coupled to a mutant — but "17% of real faults, mostly involving algorithmic changes or code deletion, are not coupled to any mutants", which they call "a fundamental limitation". Learned seeding does much better than mutation operators (SemSeed reproduces "47 out of 53" held-out real bugs exactly, and "98.2%" of its bugs lie outside the 23 standard operators). Nothing I could read establishes that synthetic bugs **rank two agent configurations** the way real ones do, and the one paper that controls for test-suite size reports that "all correlations between mutation scores and real fault detection are weak". §2.2.
- **For code review specifically, most injectable bugs would be out of scope.** `thirdshift-code-review` judges Standards and Spec; `CODING_STANDARDS.md` is one line pointing at the codebase-design skill. A flipped comparison operator is a correctness bug and would join the six Eval tasks already parked for #1371. The injectable, in-scope class is a **Spec contradiction**: change the code so it no longer does what its own `.thirdshift-issue.md` says. §2.4.
- **Contamination is real in code benchmarks but second-order for us, for a measured reason: it inflates scores, and ours are on the floor.** LiveCodeBench measured a base model dropping "from Pass@1 ∼ 60 in May problems to Pass@1 ∼ 0 in September LeetCode problems" either side of its cutoff, and Yang et al. showed n-gram decontamination is defeated by paraphrase (a 13B reaching 85.9 on MMLU "while being undetectable by n-gram overlap", and 81.1 on HumanEval against GPT-4's 67.0). But a suite where 7 of 10 Eval tasks fail every Trial cannot also be memorised. The real exposure is a Harness/Model comparison where one provider has seen more of the repository than the other — which an A/A run with identical Arms cannot see at all. Note too that SWE-bench-Live, built expressly against contamination, found "the trend does not indicate a clear correlation between task recency and success rate". §3.1, §3.2.
- **The cheapest useful probe is free and retrospective:** plot each Eval task's measured pass rate against its merge date and each Arm's Model cutoff — LiveCodeBench's "scrolling through time" on Experiment results we already have. Next cheapest is a rename-and-reread probe at one Trial per *passing* Eval task. A guided-completion probe in the *Time Travel* style (which reports "between 92% and 100%" agreement with expert labels) comes last, because it needs a control prompt and our fixes are short enough to near-match by chance. §3.3.
- **Synthetic bugs are uncontaminated by construction**, which is the strongest argument for §2: a defect injected today on the host and never pushed cannot be in any training set, and stays that way as models are retrained. §3.4.

- **The Verdict survives an independent cross-check.** A cluster bootstrap over Eval tasks and McNemar's exact test agree with the paired Dirichlet in 14 of 16 method-by-rule comparisons on the two A/A Experiments, nobody fails either one, and in simulation the Dirichlet rule's false-fail rate is **2%** against the bootstrap's 3% at 15, 30 and 40 Eval tasks — nominal for a one-sided 2.5% test. The two disagreements are the prior's pull at small suite sizes, worth about **0.02** of lower bound at 17 Eval tasks, which is enough to flip one Verdict. Appendix.

## 1. Eval tasks versus Trials

### 1.1 What Miller's framework actually recommends

[Miller](https://arxiv.org/abs/2411.00640)'s five recommendations are listed verbatim in §1 of his paper; three bear on this question:

> 3. Reducing variance by resampling answers and by analyzing next-token probabilities
> 4. When two models are being compared, conducting statistical inference on the question-level paired differences, rather than the population-level summary statistics
> 5. Using power analysis to determine whether an eval (or a random subsample) is capable of testing a hypothesis of interest

thirdshift already does (4): the Verdict pairs the Arms on each Eval task ([`src/verdict.rs`](../../src/verdict.rs), `counts[candidate + trials - baseline]`). Miller's reason is that pairing is free money: "Because eval question scores are likely to be positively correlated, even across unrelated models, paired differences represent a 'free' reduction in estimator variance when comparing two models" (§4.2). Recommendation (3)'s second half, next-token probabilities, does not apply — it is for evals "that do not utilize chain-of-thought reasoning" (§3.2), which a review session is not.

The variance decomposition is §3's:

> Var(µ̂) = Var(s)/n = (Var(x) + E[σᵢ²])/n

where `Var(x)` is the between-question variance of true difficulty ("a property of the super-population and therefore immutable") and `E[σᵢ²]` the mean within-question variance of a single sampled answer. With `K` samples per question, "Var(sᵢ) = σᵢ²/K", and Miller states the stopping rule plainly (§3.1):

> This relation should clarify the issue of how many times to resample a given question. Once E[σᵢ²]/K ≪ Var(x), increasing K further will have little effect on the standard error of µ̂.

He also warns against pooling: "computing a pooled standard error across all KN answers will be inconsistent, as multiple answers to the same question would violate the assumption of independent draws" (§3.1). thirdshift is safe here — it reduces each Eval task's Trials to one paired difference before the Dirichlet sees them.

### 1.2 The decomposition in our own A/A data

Both A/A Experiments ran identical Arms, so each Eval task's six Trials (3 per Arm) are six draws from one Bernoulli rate. Estimating per-Eval-task rates from the full per-Trial results, with the within-task term debiased by `s(k−s)/(k(k−1))`:

| Suite (source) | Eval tasks | Obs/task | Mean rate | `Var(x)` | `E[σ²]` | At the suite's `K`, within-task share |
|---|---|---|---|---|---|---|
| code-review (`199d80f2…`, #1370) | 10 | 6 | 0.217 | 0.126 | 0.043 | `K=3`: **10.3%** |
| code-review (`560af5c5…`, #1267) | 17 | 6 | 0.157 | 0.113 | 0.020 | `K=3`: **5.5%** |
| spec-review (both runs pooled) | 30 | 4 | 0.267 | 0.112 | 0.083 | `K=1`: **42.6%** |

Per-Eval-task pass rates, which is the part that matters most:

- **code-review, #1370**: `0, 0, 0, 0, 0, 0, 0, 0.333, 0.833, 1.0`. Seven Eval tasks at the floor, one at the ceiling, **three in between**.
- **code-review, #1267**: fourteen at the floor, `0.833, 0.833, 1.0`.
- **spec-review**: seventeen at the floor, four at the ceiling, nine in between (`0.25 ×4, 0.5 ×3, 0.75 ×2`).

Reading this against Miller's stopping rule, for each suite in turn:

- **code-review at three Trials is already past the point of diminishing returns.** `E[σ²]/K = 0.014` against `Var(x) = 0.126`. Raising `K` from 3 to 6 would cut the per-Eval-task variance from 0.141 to 0.134, a **5% variance reduction for double the tokens**. Adding a single Eval task to a 10-task suite cuts it by 9%, for a third of the Trials.
- **spec-review at one Trial is the only place extra Trials buy real precision.** `E[σ²]/K = 0.083` against `Var(x) = 0.112`: Miller's `≪` does not hold. Going to `K=2` would cut the per-Eval-task variance by 21% — but at twice the tokens, which the same tokens spent on twice the Eval tasks would beat (§1.3). The argument for a second Spec-review Trial is robustness, not precision: at `K=1` one timeout produces a full ±1 difference on an Eval task, which is exactly what happened in [#1267](https://github.com/JacobStephens2/thirdshift/issues/1267) ("The one task that went the candidate's way did so because its baseline Trial hit the 30-minute timeout").

Note that `Var(x)` here is *mostly the floor*: it is large because the rates are a bimodal pile at 0 with a few Eval tasks elsewhere, not because the suite spans a rich difficulty range. That is a measurement of the problem, not of the suite's resolving power.

### 1.3 The paired design changes the question — and settles it

There is a trap in applying §3's decomposition directly. For a **paired** comparison, the between-task difficulty variance *cancels*. Miller's power section (§5) defines

> ω² = Var(x_A) + V ar(x_B) − 2Cov(x_A, x_B)

and the sample size needed for Type I rate `α`, Type II rate `β` and Minimum Detectable Effect `δ` (Equation 9):

> n = (z_α/2 + z_β)² (ω² + σ_A²/K_A + σ_B²/K_B)/δ²

`ω²` is not the spread of Eval-task difficulty; it is the spread of the **per-Eval-task effect of the change**. In an A/A Experiment the Arms are the same system, so `Var(x_A) = Var(x_B) = Cov(x_A, x_B)` and `ω² = 0` exactly: *every* bit of spread in our observed paired differences is within-task Trial noise. That is consistent with what we see — #1370's `difference_counts` of `[0,1,1,8,0,0,0]` have a variance of 0.046, against the `2E[σ²]/K = 0.029` the model predicts, which is as close as ten Eval tasks can tell.

Now put Miller's formula in token terms. Trials per Arm is `N = nK`, so with `K_A = K_B = K` and equal Arm variances:

```
N = (z_α/2 + z_β)² (K·ω² + 2σ²) / δ²
```

**`N` is increasing in `K`.** The conclusions follow directly:

1. For detecting a *mean* shift, `K = 1` is token-optimal. More Eval tasks is weakly better than more Trials at every budget, and strictly better by a factor of `(Kω² + 2σ²)/(ω² + 2σ²)` whenever the change's effect varies by Eval task.
2. When the change has the same effect everywhere (`ω² = 0`), Eval tasks and Trials are *interchangeable* at equal `n·K`. This is the regime an A/A run sits in, and it is why #1268's A/A simulations were insensitive to the split.
3. So the case for `K > 1` has to be made on grounds other than power per token. There are three good ones, below.

**Where `K > 1` earns its tokens.**

- **Reliability is a different quantity from accuracy.** [τ-bench](https://arxiv.org/html/2406.12045v1) §3 defines pass^k as "the chance that all k i.i.d. task trials are successful", against pass@k, "the chance that at least one out of k i.i.d. task trials is successful", and gives unbiased estimators `pass^k = E_task[C(c,k)/C(n,k)]` and `pass@k = 1 − E_task[C(n−c,k)/C(n,k)]`. Its motivation is ours: "pass^k can capture the reliability of the agent at handling variations in conversations." Their headline decay is severe — "Even for the best-performing gpt-4o function calling agent which has a >60% average task success, pass^8 drops to <25%." Anthropic's post agrees that the choice is a product decision: "which to use depends on product requirements", and "This metric especially matters for customer-facing agents where users expect reliable behavior every time." thirdshift merges unattended, so `pass_cubed` is the metric that matches the product, and it needs `K = 3` by definition.
- **Suite maintenance needs per-Eval-task rates.** You cannot tell a broken Eval task from a hard one at `K = 1`. τ-bench used far more than its reporting budget for exactly this: "Each task has at least 40 gpt-4-turbo trials to ensure reliable per-task success rates", in order to "check all tasks with zero or low success rates" (§4, Appendix A). Its main results then used "at least 3 trials per task" (§5).
- **One flaky Trial is a whole Eval task at `K = 1`.** Timeouts, capacity errors and refusals count as failures under the Bar. At `K = 3` a single one moves an Eval task by a third; at `K = 1` it flips it entirely.

### 1.4 The simulation

The Verdict was reimplemented in Python to simulate the real decision rule rather than a normal approximation: a Dirichlet over the `2K+1` possible paired differences with the Bar's prior of one pseudo Eval task spread evenly across the bins, the mean-difference statistic `Σ (d/K)·θ_d`, and linear quantiles. Checked against the published numbers before use:

| Reported | Published | Reimplementation |
|---|---|---|
| #1370 code-review accuracy, 95% | [−0.268, +0.067] | [−0.270, +0.068] |
| #1370 code-review accuracy, 70% | [−0.171, −0.018] | [−0.171, −0.019] |
| #1370 spec-review accuracy, 95% | [−0.117, +0.184] | [−0.119, +0.187] |
| #1370 spec-review accuracy, 70% | [−0.042, +0.108] | [−0.041, +0.107] |
| #1267 code-review accuracy, 95% | [−0.128, +0.050] | [−0.125, +0.052] |

Agreement is within ±0.005, the Monte Carlo error at these draw counts. The decision rule applied is today's Bar as [#1268](https://github.com/JacobStephens2/thirdshift/issues/1268) and [#1443](https://github.com/JacobStephens2/thirdshift/issues/1443) left it: **fail** if the 95% interval lies wholly below zero, **pass** if the 70% interval lies wholly above the −0.10 margin, otherwise **inconclusive**. (Note for whoever implements this: [`src/verdict.rs`](../../src/verdict.rs) on this branch still carries a single `credible_level` and still requires `interval.upper >= 0.0` for a pass, so it predates both issues.)

Candidate Arms are simulated as a **logit drop** `δ` on the Baseline's per-Eval-task rate, which keeps rates inside [0,1] and leaves floor Eval tasks at the floor — the shape a real skill regression has. A drop of 1.0 is about −0.12 absolute on a `p = 0.5` Eval task.

**Budget-matched Eval tasks against Trials** (Eval tasks drawn `p ~ U(0.3, 0.7)`, 400 simulations per cell):

| Trials/Arm | Eval tasks | `K` | A/A pass | fail at δ=0.5 | fail at δ=1.0 |
|---|---|---|---|---|---|
| 30 | 30 | 1 | 0.41 | 0.17 | 0.47 |
| 30 | 10 | 3 | 0.42 | 0.12 | 0.41 |
| 60 | 60 | 1 | 0.54 | 0.20 | 0.73 |
| 60 | 20 | 3 | 0.56 | 0.21 | 0.66 |
| 90 | 90 | 1 | 0.61 | 0.38 | 0.86 |
| 90 | 30 | 3 | 0.66 | 0.35 | 0.89 |

As §1.3 predicts, the two designs are within simulation noise of each other at matched token cost, with a slight edge to Eval tasks at small budgets. **Nothing here justifies paying for more Trials per Eval task.** The one thing the table does show is how expensive a −0.10-scale regression is to detect at all: even 90 Trials per Arm catches a δ=0.5 drop only about a third of the time.

**Sizing a discriminating suite** (`p ~ U(0.2, 0.8)`, `K = 3`, 500 simulations):

| Eval tasks | A/A pass (70%) | A/A false fail | fail at δ=1.0 | fail at δ=0.5 |
|---|---|---|---|---|
| 10 | 0.48 | 0.02 | 0.34 | 0.14 |
| 20 | 0.59 | 0.03 | 0.68 | 0.22 |
| 30 | 0.71 | 0.03 | 0.84 | 0.31 |
| 40 | 0.73 | 0.03 | 0.94 | 0.46 |
| 60 | 0.83 | 0.03 | 0.99 | 0.57 |

So **30–40 genuinely discriminating Eval tasks** is the target: at 40, a no-op passes 73% of the time and a logit-1.0 regression is failed 94% of the time. This is a sharper statement than #1434's "about 26 more tasks", because it counts only Eval tasks that can move. It also matches #1268's finding that no pass level reaches 80% at today's sizes: the ceiling on an A/A pass rate is set by Eval task count, not by the level.

### 1.5 The floor effect: padding does not merely waste tokens

Algebraically, floor Eval tasks are invisible to a *z*-score. With `m` solvable Eval tasks out of `n`, each carrying true effect `τ` and variance `v`, the mean difference is `mτ/n` and its standard deviation `√(mv)/n`, so `z = τ√(m/v)` — `n` cancels. Floor Eval tasks are free padding as far as detectability goes.

But thirdshift's Bar is not a *z*-score. It compares an interval against a **fixed absolute margin of −0.10**, and padding shrinks the estimate and the interval together. Simulating the real rule on 10 solvable Eval tasks (`p ~ U(0.2,0.8)`, `K = 3`) plus `F` always-failing ones, with a genuine logit-1.0 regression in the Candidate:

| Always-failing added | Total Eval tasks | pass | inconclusive | fail | mean difference |
|---|---|---|---|---|---|
| 0 | 10 | **0.03** | 0.62 | 0.35 | −0.205 |
| 5 | 15 | 0.09 | 0.58 | 0.33 | −0.140 |
| 10 | 20 | 0.17 | 0.46 | 0.37 | −0.104 |
| 20 | 30 | 0.36 | 0.27 | 0.37 | −0.070 |
| 30 | 40 | **0.63** | 0.04 | 0.33 | −0.052 |

The fail column is flat, exactly as the algebra says. The **pass** column goes from 3% to 63%. A regressed Candidate that a 10-Eval-task suite would almost never pass sails through a 40-Eval-task suite that is three-quarters floor. Padding also inflates the A/A pass rate, from 0.47 to 0.98 across the same range, which would look like the suite getting *better*.

This reframes [#1434](https://github.com/JacobStephens2/thirdshift/issues/1434). Its stated problem is that "a task that always fails ties 0/3 against 0/3, so it adds nothing to the Verdict" — true but too kind. Such Eval tasks are not neutral; against a fixed margin they are a thumb on the scale in the Candidate's favour. Growing the suite with Eval tasks whose pass rate is unknown is therefore a risk, not just a cost, and the admission rule in Recommendation 2 should be a gate, not a preference.

Two corollaries:

- **The prior is doing visible work at this size.** Re-running #1370's code-review counts with smaller priors: prior 1 gives [−0.270, +0.068] (width 0.339), prior 0.5 gives [−0.266, +0.034], prior 0.1 gives [−0.259, −0.009] (width 0.251). With 10 Eval tasks and only two non-zero differences, one pseudo Eval task spread over seven bins contributes about a quarter of the interval's width and is what lifts the upper bound above zero. At Spec review's 30 Eval tasks the same check moves the interval by 0.005. The prior is a reasonable default, but it should not be read as evidence; **[interpretation]** growing the suite is the fix, not changing the prior.
- **Ceiling Eval tasks are not the same as floor Eval tasks.** An Eval task at `p = 1` can still fall, so it carries information about a regression, which is what a release gate is for. The simulated near-ceiling population (`p = 0.95`, `K = 3`) failed a logit-1.0 drop only 6% of the time, though, because a logit shift barely moves an absolute rate near 1. Mid-difficulty Eval tasks maximise information per Trial: the per-Trial Fisher information for a logit shift is proportional to `p(1−p)`, which is zero at both ends and maximal at `p = 0.5`.

### 1.6 What the benchmarks do about Eval task difficulty

**Anthropic's own guidance is the most directly applicable, and it says to split the suite.** From [*Demystifying evals for AI agents*](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents):

- On sizing, under *Step 0. Start early*: "In reality, 20-50 simple tasks drawn from real failures is a great start", "this large effect size means small sample sizes suffice", and "More mature agents may need larger, more difficult evals to detect smaller effects."
- On the two kinds, under *Capability vs. regression evals*: capability evals "should start at a low pass rate, targeting tasks the agent struggles with"; regression evals "should have a nearly 100% pass rate".
- On zero pass rates, under *Step 2*: "a 0% pass rate across many trials (i.e. 0% pass@100) is most often a signal of a broken task".
- On saturation, under *Step 7*: "Eval saturation occurs when an agent passes all of the solvable tasks, leaving no room for improvement", and "As evals approach saturation, progress will also slow, as only the most difficult tasks remain." Also, flatly: "As a rule, we do not take eval scores at face value".
- On trials: "Because model outputs vary between runs, we run multiple trials to produce more consistent results", because "a task that passed on one eval run might fail on the next".

**[interpretation]** The mismatch is plain. thirdshift's code-review suite is built like a capability eval — past bugs, chosen because they were hard enough to ship — and used like a regression eval, as the gate on a release. Anthropic's advice for the gating job is a suite the Baseline nearly always passes. Recommendation 7 takes this up.

**SWE-bench Verified is the precedent for screening, and the screen rejects most of what it sees.** Per the Method note, this page returned HTTP 403 and the figures below are a search engine's reading of it rather than my own:

- 93 developers experienced in Python screened 1,699 random SWE-bench test samples down to the 500 "verified to be non-problematic by human annotators".
- Two criteria: whether the issue description is **under-specified**, and whether the **FAIL_TO_PASS** tests "would reject valid solutions".
- The one passage that came back close to literal: "Each annotation criterion has a label ranging [0, 1, 2, 3] in increasing severity. Labels 0 and 1 are minor; labels 2 and 3 are severe, and the team conservatively ensembles annotations by taking the highest-severity label amongst the 3 annotators."
- Roughly **38%** of samples were flagged for under-specified problem statements and **61%** for tests that could unfairly fail a valid solution. A widely repeated "68.3% filtered out overall" figure could not be confirmed first-party and should not be used.

The original [SWE-bench](https://arxiv.org/html/2310.06770v3) paper is thin on exactly this, which is why Verified exists: it concedes only that "relying solely on this method is insufficient to guarantee reliable performance of model generations" (§7) and that "resolving tests such as these may be an impossible task even for human developers" (§A.1). It also ran **one sample per instance** — "Since generations are relatively expensive, we only generate a single patch file per instance" and "we simply use greedy decoding for all models" (§D.2) — so it reports no variance at all. Its execution filter is the same idea as our mechanical checks: "We filter out task instances without at least one test where its status changes from a fail to pass" (§2.1, Stage III), a step that "generally removes half of the candidate task instances" (§A.3).

**[interpretation]** The practice worth copying is the *shape*: screen every candidate Eval task against written criteria before it can gate anything, record the screen, and take the worst reading when readers disagree. thirdshift already has the hook — `state = "proposed"` / `"approved"` in the Eval task manifest, and Day-shift approval. What it lacks is a measured difficulty reading at approval time.

## 2. Synthetic Eval tasks

### 2.1 Why everyone else injects bugs

thirdshift's real-bug supply is nearly exhausted. The repository has **92 `bug` issues, 81 of them closed** (`gh issue list`, 2026-10-27) and **701 merged pull requests**. [#1252](https://github.com/JacobStephens2/thirdshift/issues/1252) traced 39 of those bugs back to the pull request that introduced them and found **12 usable pairs, 7 of them "strong"** — a yield under a third. The suite on the host already holds **21 Eval tasks, 20 approved and one proposed** (`~/.thirdshift/evals/suites/code-review/tasks/`, read 2026-10-27), of which 14 are thirdshift's own and 7 come from other repositories the factory works on (3 chart35, 2 keeplore, 2 vaulted-agent). At #1252's yield the remaining closed bugs are worth perhaps a dozen more Eval tasks, ever, and §1.4 asks for 30–40 that *discriminate*.

This is exactly the wall the benchmark authors hit. [SWE-smith](https://arxiv.org/html/2504.21798v1) §1: "the scalability of this approach is severely limited by the challenges associated with SWE-bench's collection strategy", and "setting up execution environments for each instance requires a substantial amount of human intervention". [SWE-Gym](https://arxiv.org/html/2412.21139v1) §5.2 is blunter: "There is no existing dataset suitable for training software engineering agents." The mining funnels show the cost. SWE-bench crawled **93,139 pull requests to ship 2,294** Eval tasks (Table 10). [BugSwarm](https://arxiv.org/pdf/1903.06725) inspected "10,179,558 jobs across 670 projects", mined "447,790 fail-pass pairs", and "successfully reproduced 3,091 out of 55,586 attempted pairs" — about **5.6%** (§IV-B). BugsInPy's 493 bugs "required significant effort, and took an estimated 831 man-hours" (Abstract). SWE-Gym's 2,438 environments took "around 200 human annotation hours and 10k CPU core hours" and 6 TB of images (§2.1).

**Synthetic injection changes the scale by three orders of magnitude.** SWE-smith applies five strategies (§2.1):

| Strategy | What it does (verbatim) | Instances (Table 1) | Yield |
|---|---|---|---|
| LM Modify | "provide an LM with the function and prompt it to introduce errant modifications" | 17,887 | 56.0% |
| Procedural | AST transforms: "randomly perform one or more transformations" | 15,641 | 40.2% |
| Combine bugs | "creates a task instance by aggregating candidates from the same file(s) or module(s)" | 10,092 | 96.9% |
| LM Rewrite | "given only the function header and docstring, ask the LM to rewrite it" | 4,173 | 35.0% |
| PR Mirror | "Per PR, we attempt to undo its revisions in the current version of the repository." | 2,344 | 33.8% |
| **Total** | across 128 Python repositories | **50,137** | |

The validation rule is the one thing every pipeline shares, and it is the same rule thirdshift's `checks` field already implements: SWE-smith keeps only patches that "break one or more existing, passing tests" (§2.1), SWE-bench filters "task instances without at least one test where its status changes from a fail to pass" (§2.1), BugsInPy requires tests that "should fail on the faulty source code version and run successfully on the fixed source code version" (§2), Defects4J calls a bug reproducible when "at least one test case fails on Vbug due to the fault" (§2.2).

### 2.2 How valid are seeded faults?

This literature is older than the agent literature and more honest about its limits.

**The case for.** [Just et al. (2014)](https://homes.cs.washington.edu/~mernst/pubs/mutation-effectiveness-fse2014.pdf), on 357 real faults in 5 Java programs totalling 321 KLOC and 230,000 mutants: "The results show a statistically significant correlation between mutant detection and real fault detection, independently of code coverage." Their boxed finding adds that this "correlation is stronger than the correlation between statement coverage and real fault detection" (§3.3). The coupling result: "73% of real faults are coupled to the mutants generated by commonly used mutation operators" and, controlling for coverage, "on average 2 mutants are coupled to a single real fault" (§3.1). Note that the paper reports **no single correlation coefficient** — the quantified results are Â₁₂ effect sizes per program (Table 5, 0.66–0.81, where "An effect size of Â₁₂ ≥ 0.71 is typically interpreted as large").

**The case against, from the same paper.** "For 95 out of 357 real faults (27%), none of the triggering tests detected any additional mutants" (§3.2), and the limitation is structural, not fixable by better operators:

> 27% of real faults are not coupled to the mutants generated by commonly used mutation operators. The set of commonly used mutation operators should be enhanced. However, 17% of real faults, mostly involving algorithmic changes or code deletion, are not coupled to any mutants.

Of the 63 uncoupled faults, "Algorithm modification or simplification (37): Most of the real faults not coupled to mutants were due to incorrect algorithms" (§3.2), with code deletion, wrong-method-called, context sensitivity and invariant violations making up the rest. Their conclusion: "we discovered that 17% of faults are not coupled to any mutants, which reveals a fundamental limitation of mutation analysis." And on the uncoupled subset, "results derived from mutation analysis do not generalize to those real faults."

**The ranking question is where the evidence is weakest.** Papadakis, Shin, Yoo & Bae, *Are Mutation Scores Correlated with Real Fault Detection?* (ICSE 2018) is the paper that controls for test-suite size. **It could not be read**: ACM DL and CORE returned 403, the Luxembourg repository failed TLS, IEEE Xplore is paywalled. I used the [official ICSE 2018 programme page](https://conf.researchr.org/details/icse-2018/icse-2018-Technical-Papers/78/Are-Mutation-Scores-Correlated-with-Real-Fault-Detection-A-Large-Scale-Empirical-stu), whose author-supplied abstract states: **"all correlations between mutation scores and real fault detection are weak when controlling for test suite size."** The same abstract says that comparing top-ranked-by-mutation-score suites against random suites *of the same size*, higher mutation scores still significantly improve fault detection. Treat both as the authors' abstract, not as verified internals. These two papers are not in direct contradiction: coupling (does any mutant resemble this fault) and correlation (does mutation score predict fault detection across suites) are different measures, and only Papadakis et al. control for size.

**Learned seeding is markedly more realistic than mutation operators.** [SemSeed](https://software-lab.org/publications/fse2021.pdf) (Patra & Pradel, FSE 2021) opens with the charge: "Prior work seeds bugs based on syntactic transformation patterns, which often results in unrealistic bugs and typically cannot introduce new, application-specific code tokens." Its method is "to imitate how a given real-world bug would look like in other programs by semantically adapting the bug pattern to the local context" (Abstract). The realism measurement is the most direct in this literature: on held-out real bugs, "SemSeed reproduces 47 out of 53 bugs that are in scope for the approach", "i.e., the seeded bug is exactly the same as the original bug" (§5.1). Against mutation operators: "98.2% of the SemSeed-generated bugs go beyond the 23 pre-defined mutation operators" (§5.4), and a semantics-unaware variant reproduces only 16 of 53. Downstream, a bug detector trained on SemSeed bugs rather than artificial ones went "from 7% to 53%" detected real bugs at matched precision (§5.5).

And SemSeed's own threat to validity is the one to design around:

> Our evaluation measures the realism and the usefulness of the seeded bugs, but it does not confirm that each seeded bug indeed changes the behavior of the target program.

**On synthetic difficulty, SWE-smith's findings are mildly reassuring and mildly unsettling.** "we observe that SWE-smith task instances are non-trivial for the top agent systems today" (§4), and training on them produced "40.2% Pass@1 resolve rate on the SWE-bench Verified benchmark" — so synthetic bugs transfer to real ones for *training*. But §4.1 reports "we do not observe strong correlation between increased difficulty and downstream performance", and "training on task instances with LM generated issues is empirically comparable to using the original issue text". None of this is evidence that synthetic bugs **rank two agent configurations** the same way real bugs do. **[interpretation]** That question — the one that matters for an Experiment — is unanswered in the literature I read, and the Papadakis abstract is the closest thing to a negative signal.

**[interpretation] Read.** Seeded faults are a defensible substrate for *growing* a suite and a poor substrate for *replacing* one. A suite made only of injected bugs will systematically omit the hardest fifth of real faults — algorithmic and deletion faults — which is the class most worth catching in review. Mixing is the answer: keep every real Eval task that discriminates, and inject to reach the count §1.4 asks for.

### 2.3 A pipeline for thirdshift's own repository

The Eval task format already supports this with no new machinery ([docs/eval-tasks.md](../eval-tasks.md)): an Eval task is a local `repository`, a `base` commit, a `change` commit that "supplies the working-tree diff under review", written `criteria`, and optional `checks` run "with an isolated copy of the prepared repository, captured before the session". A synthetic Eval task is just one whose `change` commit was written by us.

**For code review (bugs injected into a merged pull request):**

1. **Pick the substrate.** A merged pull request whose issue body states a checkable requirement, with `base` the merge commit's first parent and the pull request's head as the starting change — the convention #1252 already used.
2. **Inject.** Build a new `change'` commit on `base` carrying the pull request's diff plus one defect. Two injector kinds, in increasing order of trust: `cargo-mutants` for procedural AST edits (its documented operators are function-body replacement, binary-operator replacement such as "`==` with `!=`, `&&` with `||`, `<` with `>`", unary-operator deletion, match-arm deletion, match-guard replacement, and struct-literal field deletion — [mutants.rs/mutants.html](https://mutants.rs/mutants.html)); or an agent session told to make one change, in the SWE-smith "LM Modify" style.
3. **Verify the defect is real and detectable.** Three mechanical gates, all cheap:
   - it builds — cargo-mutants' own term for the alternative is "unviable", mutants that "may not compile for a given type";
   - **some existing test fails at `change'` and passes at the pull request's head.** This is SWE-smith's rule and it is also cargo-mutants' notion of a *caught* mutant. Worth stating plainly: in mutation testing a caught mutant is good news about the tests and a missed one is the finding; **for building Eval tasks the polarity inverts — caught mutants are the usable Eval tasks, and missed ones have no grading signal at all.**
   - `git log --all` shows nothing later, which the Q6.7 preparation already enforces. A synthetic `change'` commit satisfies this for free.
4. **Write the criteria from the injection, not from the code.** The injector records what it broke; that sentence becomes `criteria`, in the shape the existing Eval tasks use ("A catching finding says …. Report no wrong findings.").
5. **Calibrate before admitting** — run the Baseline Arm on it and admit only inside a difficulty band (Recommendation 2).
6. **Day-shift approval**, unchanged: `state = "proposed"` until `thirdshift evals approve` or the Console.

The injected commits live on a private branch of the host's clone under `~/.thirdshift/evals/`, never pushed. Preparation "never fetches from the network", so nothing else is needed — and §3.4 explains why not pushing is the whole contamination story.

**The scope problem, which is the one that decides whether this is worth doing for code review.** `thirdshift-code-review` reviews two axes, Standards and Spec, and [#1370](https://github.com/JacobStephens2/thirdshift/issues/1370) decided it stays that way: "A repository that wants correctness, robustness or performance rules checked can write them in its `CODING_STANDARDS.md`". thirdshift's `CODING_STANDARDS.md` is a single line — "Follow principles and ideas in the .agents/skills/codebase-design skill." A flipped `==` is a *correctness* bug against no documented standard, so it would be out of scope and would join the six Eval tasks already parked for [#1371](https://github.com/JacobStephens2/thirdshift/issues/1371). That is the same trap that produced the seven never-passing Eval tasks.

So for code review the injectable, in-scope classes are narrow and specific **[interpretation]**:

- **Spec contradiction.** Alter the diff so it no longer does what its own `.thirdshift-issue.md` says — a default changed, a stated condition dropped, an enumerated case unhandled. The Spec axis is asked to catch exactly this; the criterion writes itself from the issue line; and the issue text is already an Eval task input. This is the highest-value injection for this suite.
- **Documented-standard violation.** A deep-module principle the codebase-design skill names: a pass-through wrapper, a leaked abstraction, duplicated logic across the seam. Harder to inject mechanically and harder to grade, but in scope by construction.
- **Not** operator flips, off-by-ones or deleted error handling, unless `CODING_STANDARDS.md` grows rules that cover them.

**For a future implement suite the fit is much better**, because the Grader is `cargo test` rather than a review report. Q6.7 already settled the real-bug shape: prepared once at the base commit with network, checked "that the merged fix's tests fail there and pass at the fix", then read-only for each Trial. A synthetic Eval task inverts the same check — tests pass at `base`, fail at `base + injection` — and the Eval task is to make them pass again. That is SWE-smith's construction exactly, on our own toolchain, with no per-repository images (Q6.7's condition: images "come only when a commit stops building on the host").

### 2.4 Risks

1. **The missing fifth.** Just et al.'s 17% — algorithmic and deletion faults — cannot be injected by these methods, and #1252's own discard list is full of that class ("spread across many files", "two interacting causes", "the causing code isn't in the diff"). A synthetic suite will quietly be a suite about local faults.
2. **Ranking validity is unestablished.** The Papadakis abstract's "weak when controlling for test suite size" is a direct warning that performance on seeded faults may not order systems the way real faults do. An Experiment's whole job is ordering two Arms. **[interpretation]** Until something settles this, a release gate should not rest on synthetic Eval tasks alone.
3. **Injector-family bias.** If a Claude session writes the bug and a Claude Arm reviews it, the bug may be unusually legible to that family, which biases a Harness or Model comparison. Procedural (cargo-mutants) injection is neutral; LM injection is not. Prefer procedural, or a Model from neither Arm, for anything that gates a Harness/Model change — the same rule the Grader already follows.
4. **Behaviourless bugs.** SemSeed's own caveat: a seeded change may not change behaviour. Step 3's failing-test gate is the defence, and it must be a gate, not a warning.
5. **Grader circularity.** If the injection's own description becomes the criterion and a model Grader judges the catch, a shared blind spot passes unnoticed. Mechanical checks first, Day-shift reading of borderline Scores as now.
6. **Unviable and flaky.** cargo-mutants' own limitations chapter: it "sometimes generates unviable mutants", and "can only help if the test suite is hermetic", otherwise "it will draw the wrong conclusions about whether the tests caught a bug". Our suite is not fully hermetic — one 30-minute timeout appeared in each A/A run.
7. **The suite stops being about our bugs.** Real Eval tasks come with a Bug origin and a reason someone cared. Synthetic ones do not, and nothing stops the suite drifting toward whatever is easy to inject.

## 3. Contamination

### 3.1 What the evidence actually shows

**Time-windowed evaluation detects large drops in code benchmarks.** [LiveCodeBench](https://arxiv.org/pdf/2403.07974) tags every problem with its contest date and "scrolls through time", evaluating each model only on problems released after its cutoff (§1, §3.1). Its finding, §1: "We observe a stark drop in the performance of DeepSeek, GPT-4-O, and Codestral on LeetCode problems released after Aug 2023, Oct 2023, and Jan 2024". The sharpest number, on a base model (§5.1): "even the DS-Base-33B model also suffers from contamination dropping from Pass@1 ∼ 60 in May problems to Pass@1 ∼ 0 in September LeetCode problems." Codestral: "Pass@1 36.5 on problems released between May'23 and Jan'24 and Pass@1 28.3 on problems since Feb'24." Their negative control matters as much as the finding: the drop "primarily occurs for the LeetCode problems only", and for GPT-4-Turbo, Gemini-Pro, Mistral-L and Claude-3 "Irrespective of the release or cutoff dates, we do not find any drastic performance variations across the months".

**But a stale-versus-fresh score gap is not by itself evidence of contamination.** [SWE-bench-Live](https://arxiv.org/html/2505.23419v2) exists for this reason — "To ensure freshness and reduce the risk of data contamination from pretraining" (§3.4), 1,319 instances created after January 2024 across 93 repositories, refreshed "on a monthly basis" — and reports a huge gap: agents "report a resolved rate exceeding 60% on the SWE-bench Verified subset" against "the highest resolved rate on SWE-bench-Live is only 19.25%" (§4.2). Yet its own per-date analysis (§4.3) finds performance "remains relatively stable over time, fluctuating only modestly across quarters" and that "the trend does not indicate a clear correlation between task recency and success rate". The authors do not attribute the gap to recency.

**Decontamination by string matching does not work.** Yang et al., [*Rethinking Benchmark and Contamination for Language Models with Rephrased Samples*](https://arxiv.org/pdf/2311.04850): "we show that these methods are insufficient, and simple variations of test data (e.g., paraphrasing, translation) can easily bypass these decontamination measures." Measured: a Llama-2 13B trained on *rephrased* MMLU reaches 85.9 "while being undetectable by n-gram overlap" (Figure 1), MMLU 45.3 → 88.5 for the 7B (Table 2), GSM-8K 28.7 → 95.3 (Table 4), and on HumanEval 36.0 → 81.1 against "GPT-4 can only achieve 67.0" (Table 3). They also name the code-specific trick — dead-code injection — noting "current detection methods do not use compilers to remove dead code from coding datasets" (§3.1).

**The best-documented detector is a completion probe with a control.** Golchin & Surdeanu, [*Time Travel in LLMs*](https://arxiv.org/pdf/2308.08493) (ICLR 2024), use "guided instruction: a prompt consisting of the dataset name, partition type, and the random-length initial segment of a reference instance, asking the LLM to complete it. An instance is flagged as contaminated if the LLM's output either exactly or nearly matches the latter segment of the reference." The two parts that make it work are a **control prompt** that omits the dataset name, compared "under a non-parametric bootstrap resampling test", and a GPT-4 few-shot judge for near-matches, because ROUGE-L and BLEURT "fall short of pinpointing near-exact matches". Reported accuracy: "between 92% and 100% in detecting if an LLM is contaminated with seven datasets … when contrasted with manual evaluation by human experts" — agreement with expert labels, note, not with ground truth about any training set.

### 3.2 How much should thirdshift care?

Less than the public-repository fact suggests, for one measured reason: **contamination inflates scores, and our scores are on the floor.** Seven of code review's ten Eval tasks failed all 42 of their Trials, and 17 of Spec review's 30 failed all four. A suite cannot be simultaneously memorised and unsolvable. **[interpretation]** Contamination is not today's problem; it is a hazard to the suite's future, in three specific shapes:

- **Asymmetry between Arms.** A Harness or Model comparison is the case that matters: if one provider's training window includes more of this repository than the other's, a pass difference can be recall rather than skill. Today's Arms are Codex · gpt-6.1-sol on both sides, so A/A runs cannot see this at all.
- **A pass that arrives for the wrong reason.** If one of the three passing Eval tasks starts passing more often after a Model change, memorisation and improvement look identical.
- **Decay.** As models absorb more of the public history, older Eval tasks drift toward the ceiling, which is saturation by another route. Anthropic: "As a rule, we do not take eval scores at face value".

One thing cuts the exposure and one thing I expected to does not. A code-review Eval task asks for a *finding*, not a patch, so the memorised artefact would have to be the fix's reasoning rather than the diff — a weaker thing to recall. But **the other repositories are mostly public too**, which is worth correcting here because it is easy to assume otherwise: `gh repo view` on 2026-10-27 reports `chart35` PRIVATE and `keeplore`, `vaulted-agent`, `cascade` and `thereish` all PUBLIC. So only 3 of the 21 code-review Eval tasks (the chart35 ones) sit in a repository whose history was never published, and #1370's note that the Spec-review "tasks come from private repositories" holds for their *contents* being unpublished in the issue, not for the repositories themselves. The public-history exposure is therefore broader than the suite's provenance suggests, which raises the value of §3.3's probe 1 and of §3.4.

### 3.3 A cheap probe

Three, in increasing cost. **[interpretation]** I would run the first now and the second at approval time, and reach for the third only if one of those fires.

1. **A time-split read of the data we already have — free.** Every Eval task records its `pr_url` and its commits, so each has a merge date. Plot per-Eval-task pass rate against merge date, and against each Arm's Model cutoff where it is published. This is LiveCodeBench's "scrolling through time" on existing Experiment results, at zero session cost, and it is the only probe that can run retrospectively. Expect a null result — SWE-bench-Live found none — so treat a visible step at a cutoff date as the alarm, not the baseline.
2. **A rename-and-reread probe, one Trial per passing Eval task.** For an Eval task that *passes*, prepare a variant whose reviewed diff is mechanically perturbed without changing behaviour: rename the local identifiers and types the fix touches, reorder independent statements, and optionally add dead code in the Yang et al. style. Run one Trial of the Baseline Arm. A pass that survives perturbation is skill; a pass that collapses depended on surface recall. This is the code analogue of the rephrasing attack, and it costs one Trial per Eval task on the only three Eval tasks where it currently means anything. It also doubles as a difficulty check, since the perturbed variant is a candidate Eval task in its own right.
3. **A guided-completion probe, modelled on *Time Travel* — one short session per Eval task.** Prompt a fresh session with the repository name, the issue number and the first half of the fix commit's message or diff, and ask it to continue; then run the identical prompt with the repository and issue identifiers removed, as the control. Flag an Eval task when the named version's overlap with the real fix is significantly higher than the control's. The control is the part that cannot be skipped: small idiomatic Rust fixes will near-match by chance. Our fixes are short, so this probe is weaker here than on contest problems, and it is the one I would run last.

### 3.4 Synthetic bugs are uncontaminated by construction

This is the strongest single argument for §2, and it is structural rather than probabilistic. A bug injected on the host today, into a branch that is never pushed, cannot be in any model's training data — no probe needed, no cutoff to track, and it stays true as models are retrained. It is the same argument SWE-bench-Live makes with dates ("only issues created after January 2024") and SWE-smith makes with construction, except that we control the clock rather than waiting on it.

Two caveats keep this honest. The *surrounding* code is still public, which is fine: what must be unseen is the defect and its fix, not the repository. And a synthetic Eval task buys freedom from contamination at the price of §2.4's validity risks — it trades a hazard we cannot measure for ones we can.

## Recommendations

### (a) Repairing and growing the code-review suite (#1434)

1. **Measure difficulty before classifying, and record it.** #1434's step 1 asks for a judgment per never-passing Eval task from existing reports. Add the cheap measurement first: run the Baseline Arm alone, 6 Trials per Eval task, on the never-passing Eval tasks and on every Eval task added since #1370. Counting against the host as read on 2026-10-27: of #1370's 7 never-passing Eval tasks, 5 are still in the suite (bug-303-pr-293 and bug-559-pr-449 have already gone), the 3 that ever passed remain, and the other **13 of the 21 Eval tasks now present have no measured pass rate at all**. So 18 Eval tasks × 6 Trials ≈ **108 Baseline-only Trials** for a reading that decides which Eval tasks are allowed to gate a release. Anthropic's rule supplies the prior: "a 0% pass rate across many trials (i.e. 0% pass@100) is most often a signal of a broken task" — so read 0/6 as *probably broken or out of scope*, not as *hard*.
2. **Make the admission rule a measured gate, not a preference.** An Eval task may be `approved` for the gating suite only when (i) its target bug violates a documented standard or contradicts a Spec line — #1370's scope decision, applied as a checklist — and (ii) its measured Baseline pass rate over at least 6 Trials falls in roughly **0.15–0.95**. Record the measured rate and the Trial count in the Eval task manifest beside `state`. §1.5 is the reason this must bind: a suite padded to 40 Eval tasks with 30 at the floor passes a genuinely regressed Candidate 63% of the time against 3% unpadded.
3. **Park, don't keep, the floor.** Move every Eval task measuring 0/6 out of `suites/` beside the six already parked for #1371, with its measured rate recorded. This shrinks the suite and widens its interval, as #1370 saw — accept that. A wide honest interval reads "inconclusive"; a narrow padded one reads "pass".
4. **Target 30–40 discriminating Eval tasks, and track that number rather than the suite size.** §1.4: at 40, a no-op passes 73% of the time and a logit-1.0 regression is failed 94% of the time; at 10 those are 48% and 34%. Report "discriminating Eval tasks" (measured rate strictly between 0 and 1) alongside `retained_tasks` in `experiments --json`, so the suite's real size is visible in the Verdict.
5. **Answer #1434's step 4 yes, and say why in the measured terms.** A suite below the discriminating-Eval-task floor should report but not block. §1.5 shows the failure mode is a *false pass*, not a false block, so a suite that cannot discriminate must not be allowed to supply a pass.
6. **Fill the gap by injection, Spec contradictions first.** §2.3: take a merged pull request whose issue states a checkable requirement, alter the diff so it no longer meets that line, gate on build plus a failing existing test plus Day-shift confirmation that the finding is visible in the diff, then calibrate under Recommendation 2. Prefer procedural (cargo-mutants) injection, or a Model from neither Arm, for anything gating a Harness or Model change (§2.4 risk 3). Keep injected Eval tasks a minority of the gating suite until something establishes that they rank Arms the way real bugs do (§2.4 risk 2).
7. **Split the suite along Anthropic's line, and gate on the regression half.** Capability evals "should start at a low pass rate"; regression evals "should have a nearly 100% pass rate". Today one capability-shaped suite does a regression gate's job. Keep a **regression** suite of Eval tasks the Baseline passes reliably (measured ≥ 0.8), which is what a release gate needs, and a **capability** suite of harder Eval tasks that is reported and never blocks. The existing `state` field plus a suite-level flag is enough; no new concepts.
8. **Run the free contamination probe once.** Recommendation 1's per-Eval-task rates, plotted against each Eval task's merge date, cost nothing extra and answer §3.2's asymmetry question for the first time (§3.3, probe 1).

### (b) Trials per Eval task

9. **Keep code review at three Trials, and stop there.** Not for power — §1.2 shows within-task noise is already only 10% of the per-Eval-task variance, so a fourth Trial buys a ~5% variance reduction for a third more tokens, and §1.3 shows Trials are never cheaper than Eval tasks per token. Keep three because `pass_cubed` needs three by definition (τ-bench's pass^k: "the chance that all k i.i.d. task trials are successful"), because a flaky Trial then costs a third of an Eval task rather than all of it, and because per-Eval-task difficulty cannot be measured at one Trial.
10. **Keep Spec review at one Trial for Experiments, and buy Trials only for calibration.** It is the one place Miller's condition fails (within-task noise is 43% of the total at `K = 1`), but the fix is still more Eval tasks, not more Trials. What one Trial cannot do is tell a broken Eval task from a hard one, and 17 of its 30 Eval tasks never pass. So run a one-off Baseline-only calibration at 6 Trials per Eval task — about 180 Trials — apply Recommendation 2's band, and leave the Bar at one.
11. **Spend new tokens on Eval tasks, with one exception.** The exception is a change whose effect is expected to be *uniform* across Eval tasks (`ω² ≈ 0`), where §1.3 makes Eval tasks and Trials interchangeable at equal `n·K` and Eval tasks are merely not worse. There is no case where more Trials are the better buy for power.
12. **Record `trials_per_arm` reasoning in the suite manifest.** The Bar already fixes Trials per Eval task; add a one-line comment in `suite.toml` saying which of Recommendation 9's three reasons applies, so a future change to it is argued rather than tuned. Note also that [`src/verdict.rs`](../../src/verdict.rs) hard-restricts Trials per Eval task to 1 or 3 (`matches!(bar.trials_per_task, 1 | 3)`); that restriction is now a documented choice rather than an accident, and §1.3 says it costs nothing.

### (c) A future implement suite

13. **Keep Q6.7's real-bug construction as the suite's spine and its validity anchor.** Base commit prepared once with network, checked that the merged fix's tests fail there and pass at the fix, time-safe clone, read-only copy per Trial. This is Defects4J's and SWE-bench's construction and it is the only part of the suite whose faults are certainly real.
14. **Grow it with SWE-smith-style injection, since the Grader is already mechanical.** Inject into the current code, keep only injections that "break one or more existing, passing tests", and grade with `cargo test` and no network, as Q6.7 specifies. The polarity note from §2.3 is the operational rule: **a mutant the tests catch is a usable Eval task; a mutant they miss is not.** `cargo-mutants` enumerates candidates and labels them for free; the "missed" column is the discard pile, and the "caught" column is the Eval task supply.
15. **Bound the per-Eval-task runtime at generation, as SWE-smith does** ("we also limit testing runtime to two minutes"), and prefer injections in crates whose targeted selectors are cheap — this repository's are `cargo test --bin thirdshift <module>::tests` and `cargo test --test <file>` ([AGENTS.md](../../AGENTS.md)). The host has 4 cores and 8 GB; a suite of Eval tasks that each need a full build will not run beside the factory.
16. **Write the Spec's admission rule as Recommendation 2's gate, not as prose.** Mixed real and synthetic, each Eval task carrying its provenance (`real` / `injected-procedural` / `injected-model`), its measured Baseline pass rate, its Trial count and its injector's Model where applicable. Report the Verdict split by provenance, so the ranking-validity question of §2.4 can be answered from our own data over time instead of taken on faith: if real and injected Eval tasks start disagreeing about which Arm is better, that is the finding.
17. **Set Trials per Eval task to three from the start, for the same three reasons as code review**, and expect the implement suite to be the one where `pass_cubed` matters most — it is the suite closest to what a Run actually does, and the factory merges unattended.
18. **Screen like SWE-bench Verified, at our scale.** Its two criteria translate directly: is the task under-specified (would a competent engineer know what is wanted?), and would the grading tests reject a valid alternative fix? Record a severity per criterion and, where two readers disagree, take the worse — that is the one procedural detail worth copying verbatim from a page I could not read, and worth re-reading before it is written into a Spec.

## Appendix: an independent cross-check of the Verdict (2026-10-27)

§1.4 re-implemented the **Bar**'s paired Dirichlet in Python and matched the published intervals. That checks the arithmetic, not the method. This appendix checks the method, by deciding the same two A/A **Experiments** with two unrelated estimators.

**Method.** The per-**Trial** results of both Experiments were read again read-only from the host (`ssh jacob@thirdshift.app`, `~/.local/bin/thirdshift experiments <id> --json`); nothing there was started, resumed or modified. Each **Eval task** was reduced to one paired difference, exactly as [`src/verdict.rs`](../../src/verdict.rs) does: for accuracy, `(candidate passes − baseline passes) / K`; for wrong findings, `(baseline wrong − candidate wrong) / K`, so positive is better in both. The reconstructed `difference_counts` equal the published ones in all eight cells, which fixes the data as identical. Three estimators then ran on them:

- **Paired Dirichlet**, the Verdict's own method, re-implemented per §1.4 (prior 1 spread over all `2K+1` bins, 20,000 draws, linear quantiles), at 95% and 70%.
- **Cluster bootstrap** of the mean paired difference, resampling **Eval tasks** with replacement (20,000 resamples), percentile and BCa. The Eval task is the cluster, so code review's three Trials per Eval task travel together. BCa needs a mid-*p* bias correction here: the differences are coarse multiples of `1/K`, about 18–30% of resamples tie exactly with the point estimate, and the plain `P(θ* < θ̂)` then puts `z₀` at −0.34 to −0.46 and returns intervals that sit wholly below zero. Those were artefacts; every BCa figure below uses `P(θ* < θ̂) + ½P(θ* = θ̂)`.
- **McNemar's exact test**, two ways: pairing Trial *i* of the **Baseline arm** with Trial *i* of the Candidate (the naive pairing, which treats three Trials on one Eval task as independent), and pairing Eval tasks on pass-on-all-`K`.

Decisions are read two ways: the **main rule** as `src/verdict.rs` has it on `main` (fail if the 95% upper is below zero, pass if the 95% lower is above the −0.10 margin *and* the upper is at least zero), and the **Spec rule** behind #1279 (fail if the 95% upper is below zero, pass if the 70% lower is above the margin). Scripts ran under `python3 -I` with numpy and scipy only, in a scratch directory outside the repository; nothing was kept. Both Experiments are A/A — identical **Arms** — so the true difference is zero and every pass is correct by construction.

### A.1 Accuracy

| Experiment, suite | Counts | Dirichlet 95% | Dirichlet 70% | Bootstrap pct 95% | BCa 95% | Bootstrap pct 70% | McNemar *p* (Trial / task) |
|---|---|---|---|---|---|---|---|
| #1267 code-review, 17 tasks | `[0,0,2,15,0,0,0]` | [−0.125, +0.052] | [−0.076, −0.003] | [−0.098, +0.000] | [−0.118, +0.000] | [−0.059, −0.020] | 0.50 / 0.50 |
| #1267 spec-review, 30 tasks | `[2,27,1]` | [−0.163, +0.086] | [−0.091, +0.024] | [−0.133, +0.067] | [−0.167, +0.067] | [−0.100, +0.033] | 1.00 / 1.00 |
| #1370 code-review, 10 tasks | `[0,1,1,8,0,0,0]` | [−0.270, +0.068] | [−0.171, −0.019] | [−0.233, +0.000] | [−0.333, +0.000] | [−0.167, −0.033] | 0.25 / 1.00 |
| #1370 spec-review, 30 tasks | `[2,25,3]` | [−0.119, +0.187] | [−0.041, +0.107] | [−0.100, +0.167] | [−0.100, +0.167] | [−0.033, +0.100] | 1.00 / 1.00 |

Decisions, accuracy:

| Experiment, suite | main rule: Dirichlet / pct / BCa | Spec rule: Dirichlet / pct / BCa |
|---|---|---|
| #1267 code-review | inconclusive / **pass** / inconclusive | pass / pass / pass |
| #1267 spec-review | inconclusive / inconclusive / inconclusive | **pass** / inconclusive / inconclusive |
| #1370 code-review | inconclusive / inconclusive / inconclusive | inconclusive / inconclusive / inconclusive |
| #1370 spec-review | inconclusive / inconclusive / inconclusive | pass / pass / pass |

### A.2 Wrong findings

| Experiment, suite | Counts | Dirichlet 95% | Dirichlet 70% | Bootstrap 95% (pct and BCa) | McNemar *p* | Decisions, main / Spec rule |
|---|---|---|---|---|---|---|
| #1267 code-review | `[0,0,0,17,0,0,0]` | [−0.079, +0.078] | [−0.023, +0.023] | [+0.000, +0.000] | 1.00 | pass / pass, all three methods |
| #1267 spec-review | `[0,30,0]` | [−0.057, +0.056] | [−0.016, +0.016] | [+0.000, +0.000] | 1.00 | pass / pass, all three methods |
| #1370 code-review | `[0,0,0,10,0,0,0]` | [−0.128, +0.127] | [−0.039, +0.039] | [+0.000, +0.000] | 1.00 | Dirichlet **inconclusive**, bootstrap pass / all pass |
| #1370 spec-review | `[1,28,1]` | [−0.110, +0.108] | [−0.047, +0.047] | [−0.100, +0.100] | 1.00 | inconclusive / Dirichlet pass, BCa inconclusive |

Three of the four wrong-findings cells have **no discordant Eval task at all**: both Arms reported the same number of wrong findings on every Eval task. McNemar has nothing to test and the bootstrap, resampling identical values, returns a point.

### A.3 Where the methods disagree, and why

**Fourteen of the sixteen method-by-rule comparisons agree.** No method fails either Experiment on either metric, which is correct: the Arms are identical. The three disagreements are all between *pass* and *inconclusive*, and each has a traceable cause.

1. **#1267 code-review accuracy, main rule: the Dirichlet's lower bound is −0.125 and the bootstrap's is −0.098, either side of a −0.10 margin.** A 0.002 difference decides it. The gap is the prior: re-running the same counts at prior 0.5 gives [−0.116, +0.026] and at prior 0.1 [−0.104, −0.003], which is the bootstrap's answer to within Monte Carlo error. One pseudo Eval task spread evenly over seven bins — including the ±1 bins, which no real Eval task occupied — is worth about 0.021 of lower bound at 17 Eval tasks. §1.5 already said the prior does visible work at this size; this is that, at the exact point where it changes a Verdict.
2. **#1370 code-review wrong findings: the Dirichlet is inconclusive on perfectly tied data.** Ten Eval tasks, zero discordance, and the 95% interval is still [−0.128, +0.127] — wider than the margin, so no pass. Every figure in it comes from the prior (prior 0.5 gives [−0.094, +0.094], prior 0.1 gives [−0.030, +0.029]). The bootstrap's [0.000, 0.000] is not a better answer, though: it is uninformative, because a cluster bootstrap of identical values cannot express uncertainty at all. The honest frequentist bound is the rule of three: 0 discordant Eval tasks in 10 puts a one-sided 95% upper bound of **0.259** on the chance an Eval task is discordant, so the mean difference could be as large as 0.086 if a discordant Eval task moves by one Trial, or 0.259 if it moves by three. The Dirichlet's ±0.128 sits inside that range. **[interpretation]** So the Verdict is being *conservative on a small suite*, not wrong; the bootstrap is simply silent here, and the fix is more Eval tasks, as §1.5 says.
3. **#1267 spec-review accuracy and #1370 spec-review wrong findings, Spec rule: the Dirichlet passes where the bootstrap does not.** Both turn on a 70% lower bound within 0.01 of the margin (−0.091 against −0.100; −0.047 against −0.100 at 95% but −0.100 on the BCa). At 30 Eval tasks the prior moves the interval by about 0.003, so this is not the prior — it is the Dirichlet smoothing a three-bin multinomial where the bootstrap resamples a three-valued variable with a standard deviation of 0.32–0.41 and lands on lattice points. **[interpretation]** This is the known small-sample coarseness of the percentile bootstrap on discrete data, and the Dirichlet is the better-behaved of the two here.

**Clustering does not bite in this data, but the naive McNemar would mislead at scale.** Pairing Trial-by-Trial gives code review 51 and 30 pairs instead of 17 and 10, which is the independence violation Miller warns about (§1.1). It changes one *p* value — #1370 code-review accuracy, 0.25 Trial-paired against 1.00 task-paired — because the three discordant Trials sit on only two Eval tasks. Discordance is sparse enough here that the inflation is invisible; on a suite with real per-Eval-task effects it would not be, and the Verdict's reduction to one difference per Eval task is the right design.

**Coverage, as far as two A/A Experiments can show it.** Across the four accuracy cells, every 95% interval from every method covers the true value of zero. Two of the four 70% intervals do not (#1267 code-review, [−0.076, −0.003]; #1370 code-review, [−0.171, −0.019]) — and both bootstraps miss the same way on the same two cells. At a nominal 30% miss rate per cell that is unremarkable, and it is the pass level, not the fail level, so it costs an inconclusive rather than a false fail.

### A.4 Synthetic decision rates: the Dirichlet rule against the bootstrap

**Method.** Eval task rates drawn `p ~ U(0.2, 0.8)`, `K = 3`, Candidate rate `p − 0.15` clipped at zero, 2,000 simulations per cell, 4,000 Dirichlet draws and 3,000 bootstrap resamples each. The bootstrap is scored with the same two decision rules, on percentile intervals.

| Tasks | Rule | A/A pass | A/A inconclusive | A/A fail | −0.15 pass | −0.15 inconclusive | −0.15 fail |
|---|---|---|---|---|---|---|---|
| 15 | Dirichlet, main | 0.19 | 0.79 | **0.02** | 0.01 | 0.70 | 0.30 |
| 15 | bootstrap, main | 0.21 | 0.75 | **0.03** | 0.01 | 0.63 | 0.35 |
| 15 | Dirichlet, Spec | 0.53 | 0.45 | 0.02 | 0.07 | 0.63 | 0.30 |
| 15 | bootstrap, Spec | 0.53 | 0.44 | 0.03 | 0.07 | 0.57 | 0.35 |
| 30 | Dirichlet, main | 0.31 | 0.67 | 0.02 | 0.01 | 0.42 | 0.57 |
| 30 | bootstrap, main | 0.31 | 0.66 | 0.03 | 0.00 | 0.39 | 0.61 |
| 30 | Dirichlet, Spec | 0.66 | 0.32 | 0.02 | 0.05 | 0.38 | 0.57 |
| 30 | bootstrap, Spec | 0.64 | 0.33 | 0.03 | 0.04 | 0.35 | 0.61 |
| 40 | Dirichlet, main | 0.40 | 0.58 | 0.02 | 0.00 | 0.30 | 0.70 |
| 40 | bootstrap, main | 0.40 | 0.58 | 0.03 | 0.00 | 0.28 | 0.71 |
| 40 | Dirichlet, Spec | 0.74 | 0.24 | 0.02 | 0.03 | 0.27 | 0.70 |
| 40 | bootstrap, Spec | 0.72 | 0.25 | 0.03 | 0.03 | 0.26 | 0.71 |

The two estimators track each other to within 0.05 in every cell, and within 0.02 in most. The differences are systematic and small: the bootstrap fails about one point more often under A/A (0.03 against 0.02) and about four points more often against a real −0.15 regression, which is the same conservatism the prior buys, read from the other side. **The false-fail rate is the number to keep.** A fail needs a 95% interval wholly below zero, which is a one-sided 2.5% test; the Dirichlet rule delivers 2% and the bootstrap 3%, at every suite size. That is nominal.

Note also that the Spec rule's pass level is what moves with suite size — 0.53 to 0.74 for a no-op between 15 and 40 Eval tasks — and that it costs 3–7% passes on a genuine −0.15 regression, against the main rule's 0–1%. Both estimators agree on that trade to within a point, so it is a property of the two-level rule and the margin, not of the Dirichlet.

### A.5 Verdict on the Verdict

**No evidence of miscalibration.** The paired Dirichlet, a cluster bootstrap and McNemar's exact test reach the same conclusion about both A/A Experiments on both metrics: nothing is detectably worse, and nothing can be declared equivalent at a −0.10 margin on suites this small. Where they part, the Dirichlet is the *more cautious* method in two of three cases and the *better-behaved* one in the third, and its false-fail rate is at nominal. **[interpretation]** Two conclusions for #1434 and #1279: the prior is worth 0.02 of lower bound at 17 Eval tasks and should not be left to decide a Verdict at that size, which is another argument for growing the suite rather than retuning the Bar; and the prior's mass in the ±1 bins — differences no Eval task has ever shown — is the part of the Bar most worth revisiting, since a prior concentrated on small differences would narrow the small-suite intervals without weakening the fail rule.
