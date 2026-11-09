# Non-inferiority pass rules: does the Bar's "upper bound reaches zero" clause have a precedent?

Research date: 2026-10-27.

**Question.** thirdshift's **Bar** ([ADR 0024](../adr/0024-pass-and-fail-use-separate-levels.md), `Intervals::decide` in `src/verdict/levels.rs`) judges a paired Dirichlet posterior on Candidate-minus-Baseline score:

- **fail** when the upper bound of the central interval at the fail level (95%) is below zero;
- otherwise **pass** when the lower bound at the pass level is above the margin (-0.10) **and** the upper bound at the pass level is at least zero;
- otherwise **inconclusive**.

The new default pass level is 70%. A simulation of A/A Experiments (identical Arms) found that the `upper >= 0` clause caps a true no-op's pass probability at about `((1+L)/2)^k` for `k` noisy metrics — about 72% at `L` = 70% with two metrics, however many Eval tasks there are. A follow-up Ticket will decide whether to keep, drop or replace that clause. This note asks what regulatory statistics says:

1. In a non-inferiority (NI) test, how is a result handled when the whole interval lies between the margin and zero — "non-inferior but statistically worse"? Is it still declared non-inferior?
2. Is it standard to require only the lower bound against the margin? What do the sources say about one-sided versus two-sided levels?
3. For Bayesian NI, what posterior-probability thresholds are used, and is anything like an "upper bound must reach zero" condition used?
4. What guidance covers co-primary endpoints that must both pass, and their effect on power?

**Method.**

- **Primary documents only, read in full text.** Each guidance PDF was downloaded from the issuing agency and converted with `pdftotext -layout`; quotes below are copied from that text, and section and page references are the documents' own printed numbering (verified page-by-page where the number is load-bearing). The documents:
  - FDA, *Non-Inferiority Clinical Trials to Establish Effectiveness* (November 2016, CDER/CBER).
  - EMA/CPMP, *Points to Consider on Switching Between Superiority and Non-Inferiority* (CPMP/EWP/482/99, 27 July 2000).
  - EMA/CHMP, *Guideline on non-inferiority and equivalence comparisons in clinical trials* — **draft** (EMA/301654/2025, 24 October 2025). Included because the EMA itself states this draft replaces the 2000 Points to Consider; it is a draft under consultation, not adopted guidance.
  - ICH E9, *Statistical Principles for Clinical Trials*.
  - ICH E10, *Choice of Control Group and Related Issues in Clinical Trials*.
  - FDA/CDRH, *Guidance for the Use of Bayesian Statistics in Medical Device Clinical Trials* (issued 5 February 2010).
  - FDA, *Use of Bayesian Methodology in Clinical Trials of Drug and Biological Products* — **draft** (January 2026, docket FDA-2025-D-3217). This is the 2026 draft the question asked about; it exists.
  - FDA, *Multiple Endpoints in Clinical Trials* (October 2022, docket FDA-2016-D-4460).
- **The CONSORT extension** (Piaggio et al., *JAMA* 2012;308(24):2594-604, doi 10.1001/jama.2012.87802) was read from the JAMA full-text page. Figure 1 and its caption and footnote were read directly. The rest of the article body was only partly readable through the fetch, so claims from it are limited to the figure and caption; this is flagged where it matters.
- **Verified absences.** Where this note says a document contains nothing on a topic, that is from a case-insensitive search of the full extracted text, not from skim-reading. Those are marked "searched, no hit".
- **Judgment.** Everything under §5 and "Implications for the Bar" is my reasoning about thirdshift, not a cited claim. Arithmetic marked "my computation" is mine.

**Out of scope.** Choosing a margin (the whole `M1`/`M2` apparatus), assay sensitivity, prior selection, and whether the Bar should use a Dirichlet posterior at all.

---

## 1. An interval wholly between the margin and zero is still non-inferior

This is answered head-on, and in the same direction, by both of the sources that enumerate interval scenarios.

**FDA 2016, §III.A, Figure 2, scenario 6 (p. 6).** The guidance writes the difference as control minus test (`C-T`), so "upper bound below `M`" is its NI criterion and "interval above zero" means the test drug is worse:

> 6. Point estimate of C-T is 1, favoring the control. The upper bound of the 95% CI for C-T is <M1, demonstrating NI (the entire effect of C has not been lost) but at the same time the 95% CI for C-T is above zero, indicating that T is actually inferior to C, even while meeting the NI standard.

Non-inferiority is **demonstrated**. The two findings — NI met, and inferiority to the control established — are reported side by side; the second does not cancel the first.

**CONSORT 2012 extension, Figure 1, scenario D and footnote a.** Figure 1 is titled "Possible Scenarios of Observed Treatment Differences for Adverse Outcomes (Harms) in Noninferiority Trials"; its caption states "Error bars indicate 2-sided 95% CIs", "The blue dashed line at x = Δ indicates the noninferiority margin" and "the blue tinted region to the left of x = Δ indicates the zone of inferiority." Scenario D:

> D, If the CI lies wholly to the left of Δ and wholly to the right of zero, the new treatment is noninferior in the sense already defined … but also inferior in the sense that a null treatment difference is excluded.

Footnote a:

> This CI indicates noninferiority in the sense that it does not include Δ, but the new treatment is significantly worse than the standard. … Such a result is unlikely because it would require a very large sample size.

The caption calls this a puzzling but rare circumstance, and notes it can also arise from a non-inferiority margin that is too wide. Again: the conclusion is "noninferior **and** significantly worse", not "inconclusive".

**ICH E10 §1.5.1.1 (p. 9)** states the criterion as a single exclusion, with no second condition:

> If the confidence interval for the difference between the test and control treatments excludes a degree of inferiority of the test treatment as large as, or larger than, the margin, the test treatment can be declared non-inferior; if the confidence interval includes a difference as large as the margin, the test treatment cannot be declared non-inferior.

**EMA 2000 §II.3 (pp. 3-4)** likewise: "the two-sided 95% confidence interval should lie entirely to the right of the value -∆", illustrated by Figure 3 with the two labels "NON-INFERIORITY SHOWN" and "NON-INFERIORITY NOT SHOWN" — two outcomes, decided by one bound.

**EMA 2025 draft, §5 "Basic analysis concepts", Figure 2 (p. 6)** restates the same shape for the modern draft: non-inferiority testing is superiority testing with the comparison line moved from 0 to -Δ, and

> instead of checking that the two-sided 95% confidence interval lies above 0 (1 for ratios), the confidence interval should lie above a number less than 0 … called the non-inferiority margin.

**No primary source found makes "the interval must reach zero" a condition of a non-inferiority claim.** Searched, no hit: FDA 2016, EMA 2000, EMA 2025 draft, ICH E9, ICH E10, FDA 2010 Bayesian devices, FDA 2026 Bayesian draft. What the sources do with the zero line is the opposite — crossing it is an *additional, stronger* claim (superiority), never a precondition for the weaker one:

**EMA 2000 §IV.1 (p. 5).**

> If the 95% confidence interval for the treatment effect not only lies entirely above -∆ but also above zero then there is evidence of superiority in terms of statistical significance at the 5% level (p<0.05). … There is no multiplicity argument that affects this interpretation because, in statistical terms, it corresponds to a simple closed test procedure.

**FDA 2016 §III.H (p. 18)** and **§IV.G (p. 31)** agree: a superiority finding arising in an NI study "can be interpreted without adjustment for multiplicity", and "a trial that is planned to demonstrate non-inferiority may also be used to test for superiority without concern about inflating the Type I error rate. This sequential testing procedure has the Type I error rates for tests of both non-inferiority and superiority controlled at the 2.5% level."

**EMA 2025 draft, §10 (p. 20)** frames it the same way: "testing for superiority after absolute or relative efficacy have been demonstrated is a special case of reducing the non-inferiority margin to zero".

So the standard structure is: one test against the margin, and optionally a second, separate test against zero — whose outcomes are reported independently. A rule that makes *failing* the zero test block the margin claim has no counterpart in any of these documents.

One genuine caution the sources do raise: FDA 2016 §III.E (p. 14) says the tested bound "is not, however, the only measurement of interest … The point estimate of the treatment effect and the width of its confidence interval are also relevant." That supports *reporting* a non-inferior-but-worse result as such — not withholding the non-inferiority conclusion.

## 2. Only the margin-side bound is tested, and NI is intrinsically one-sided

**ICH E9 §3.3.2 (pp. 14-15)** is the cleanest statement, on both halves of the question:

> For the active control equivalence trial, both the upper and the lower equivalence margins are needed, while only the lower margin is needed for the active control non-inferiority trial.

and

> For equivalence trials, two-sided confidence intervals should be used. … For non-inferiority trials a one-sided interval should be used. The confidence interval approach has a one-sided hypothesis test counterpart for testing the null hypothesis that the treatment difference (investigational product minus control) is equal to the lower equivalence margin versus the alternative that the treatment difference is greater than the lower equivalence margin. The choice of type I error should be a consideration separate from the use of a one-sided or two-sided procedure.

Two bounds is the *equivalence* design, not non-inferiority — and equivalence bounds the difference on both sides at **±Δ**, not at "the margin and zero" (EMA 2025 draft, §5, Figure 3, pp. 6-7: "the two-sided 95% confidence interval should lie within both a lower bound (-Δ) and an upper bound (+Δ)").

FDA 2016 §III.A also notes the asymmetry in prose (p. 4): "The intent of an NI trial, however, is not to show that the new drug is equivalent, but rather that it is not materially worse than the control. Therefore, the interest is one-sided."

**On level bookkeeping, all the sources give the same mapping.**

- FDA 2016 §III.A (p. 4), for the superiority case: "the lower bound of the two-sided 95% confidence interval (equivalent to the lower bound of a one-sided 97.5% confidence interval)".
- FDA 2016 §III.E (p. 14), for NI: "ensuring that the upper bound of the 95% two-sided confidence interval for C-T in the NI trial is smaller than M1. Use of this interval corresponds to a one-sided test size (alpha level) of 0.025 in testing the NI hypothesis".
- EMA 2000 §II.4 (p. 4): "It will be assumed throughout this document that two-sided 95% confidence intervals are to be used for all clinical trials whatever their objective. … If one-sided intervals are used, then they should be used with a coverage probability of 97.5%."
- EMA 2025 draft, §9 "Statistical considerations", under "Confidence intervals" (pp. 16-17): "These confidence intervals should typically be two-sided (even in non-inferiority comparisons) and have a two-sided coverage probability of 95%. The two-sided coverage probability of 95% should be symmetric. This means that the lower confidence limit has a 97.5% probability of being below the parameter of interest … This property ensures that the 95% confidence interval corresponds to a two-sided significance test at 5% or a one-sided significance test at 2.5%."
- ICH E9 §5.5 (p. 25): "The approach of setting type I errors for one-sided tests at half the conventional type I error used in two-sided tests is preferable in regulatory settings. This promotes consistency with the two-sided confidence intervals that are generally appropriate".

So the convention is: report a two-sided 95% interval, but *test* the NI hypothesis with one of its bounds, which is a one-sided 2.5% test. The two-sided label is a reporting convention; the one-sided level is the operating characteristic. The level is also required to be pre-specified: EMA 2000 §III (p. 4) says the margin and "the corresponding coverage probability for the confidence interval (usually 95%) should also be chosen at this time", i.e. in the protocol.

The only place a tightened level appears is genuine multiplicity. FDA 2016 §IV.G (p. 31): "it would not be appropriate to apply 95% confidence intervals for multiple tests of non-inferiority and superiority across multiple endpoints or multiple doses because of the potential to inflate the Type I error rate for the trial; use of larger intervals that reflect adjustment for multiple tests (e.g., 97.5% confidence intervals or higher) may be needed."

## 3. Bayesian non-inferiority: a posterior-probability threshold on one tail

**FDA 2026 draft, §III.B (pp. 6-7)** gives the canonical form, and confirms it is one-sided and margin-anchored:

> specification of a success criterion is most often based on the posterior probability that the true treatment effect size exceeds some threshold. In mathematical notation, such a criterion might take the form Pr(d > a) > c, where d is a population-level summary of the size of the treatment effect, a is a minimum threshold for the treatment effect to be considered beneficial, and c is a minimum probability level that would support a conclusion of effectiveness.

§III.B.1, "Calibration to Type I Error Rate" (p. 6):

> For some Bayesian approaches, a and c can be chosen such that the overall FWER is controlled at a given level, typically 0.025 one-sided. This is referred to as calibrating the success criteria to Type I error rate. … When calibrating Bayesian success criteria to Type I error rate, a is chosen to be 0 for superiority designs or is based on the non-inferiority margin, m, for non-inferiority designs. Determining the appropriate choice of c to control FWER then becomes a computational problem only.

So in the calibrated case the Bayesian NI criterion is `P(diff > -margin) > c` with `c` set so the one-sided type I error is 0.025 — i.e. roughly `c` = 0.975 for a flat-prior, well-behaved posterior, exactly the frequentist threshold the question asked about. §III.B.2, "Direct Interpretation of Posterior Probability" (p. 7), offers the alternative when the prior genuinely encodes external information: the threshold is chosen by asking "whether a 1 – c chance of ineffectiveness is sufficiently small in the specific context", with 0.98 used as the worked illustration. §III.B.3 (p. 7) adds benefit-risk and decision-theoretic (expected-loss) threshold setting as a third option — i.e. the level is allowed to follow from the consequences of being wrong.

**FDA 2010 Bayesian device guidance** gives the same two ideas in older language. §5.3, "Interval estimation": "Bayesian interval estimates are based on the posterior distribution and are called credible intervals. If the posterior probability that an endpoint lies in an interval is 0.95, then this interval is called a 95 percent credible interval." It names both HPD and central posterior intervals as acceptable constructions. §5.8, "Decision analysis" (p. 36), on how high to set the bar:

> The more critical the endpoint, the more information about that endpoint will be necessary, and the sharper should be its posterior distribution. … A reasonable degree of certainty could be 90%, 95% or even 99%. These values will be reflected in the pre-specified posterior probabilities that the endpoints are below a certain target.

And §4.8 (pp. 28-29) requires the design's **operating characteristics** — type I rate, type II rate, power, sample-size distribution — to be evaluated by simulation as part of trial planning, and notes that "When using prior information, it may be appropriate to control type I error at a less stringent level than when no prior information is used."

**Nothing resembling an "upper bound must reach zero" condition appears in either Bayesian guidance.** Searched, no hit in both. The only place the zero line enters the 2010 guidance is as an *adaptive* feature — §4.7 lists "Switching the hypothesis of non-inferiority to superiority or vice-versa" (p. 30) among things a Bayesian adaptive trial can do, i.e. choosing which of the two claims to pursue, not requiring both.

The EMA 2025 draft contains no Bayesian material at all (searched, no hit for "Bayes").

## 4. Co-primary endpoints: the power multiplies, and the fix is sample size, not a looser level

**FDA 2022 *Multiple Endpoints*, §III.B, "Type II Error Rate and Sample Size" (p. 8)** is the direct analogue of the Bar's `k`-metric problem:

> Using two or more endpoints for which demonstration of an effect on each is recommended to support regulatory approval (called co-primary endpoints; see section III.C.1. below) will increase the Type II error rate and decrease study power. For example, assume two endpoints have the same effect size and the study sample size is selected to provide 80% power to show success on each of these two endpoints. If the endpoints are independent, the power to show success on both will be approximately 64% (0.8 x 0.8); i.e., the likelihood of the study failing to support a conclusion of a favorable drug effect when such an effect existed (the Type II error rate) would be 36%. To maintain desired study power, a larger sample size is recommended, and the individual endpoints could be powered at approximately 90% to ensure the probability of success is at least 80%. The calculation would be different if the endpoints were highly positively correlated or the power was not equal for each endpoint.

**§III.C.1 (p. 9)** adds the two constraints that bear on the Bar's options:

> As discussed in section III.B., there is no multiplicity problem when the study is designed to demonstrate efficacy on all of the separate endpoints. However, co-primary endpoint testing increases the Type II error rate. In general, unless clinically very important, the use of more than two co-primary endpoints should be carefully considered because of the loss of power.

> There have been suggestions that the statistical testing criteria for each co-primary endpoint could be increased (e.g., testing at an [alpha] of 0.06 or 0.07) when the targeted [alpha] is 0.05 to accommodate the loss in statistical power arising from the need to show an effect on both endpoints. Increasing [alpha] for each co-primary endpoint is not acceptable because doing so may undermine the ability to interpret a treatment effect on each disease aspect considered critical.

ICH E9 §3.5, "Sample Size", adds the NI-specific sample-size warning: "When the power of a non-inferiority trial is assessed at a zero difference, then the sample size needed to achieve that power will be underestimated if the effect of the investigational product is less than that of the active control."

## 5. What transfers to thirdshift, and what does not

My assessment, not a cited claim.

**Transfers.**

- **The shape of the criterion.** A margin, a pre-specified level, one bound tested against the margin, a separate optional test against zero. ADR 0024's split of a pass level from a fail level maps onto exactly this: a lenient NI test against -0.10, and a strict test against zero, reported separately.
- **Pre-specification.** ADR 0024 already fixes both levels before the first Trial, matching EMA 2000 §III.
- **Co-primary arithmetic.** `0.8 × 0.8 = 0.64` and "power each endpoint at ~90% for ≥80% joint" is literally the arithmetic the A/A simulation rediscovered, including the correlation caveat. The guidance's answer — raise the per-metric level and enlarge the study, never relax the per-metric criterion — is directly applicable.
- **Simulate the operating characteristics.** FDA 2010 §4.8 and FDA 2026 draft (on evaluating operating characteristics) make simulation of the design's behaviour part of planning a Bayesian decision rule. The A/A measurement that set the 70% pass level is that exercise.

**Does not transfer.**

- **The whole `M1`/`M2`, assay-sensitivity, constancy-assumption apparatus** — most of FDA 2016 and all of ICH E10 §1.5. It exists because an NI trial has no placebo arm and so must import the control's effect from history. thirdshift has a concurrent Baseline Arm and a direct paired comparison. There is no indirect inference to protect.
- **The asymmetry of consequences.** The 0.025 one-sided level encodes the public-health cost of approving an ineffective drug (FDA 2016 §III.E: "there is a powerful tendency to be conservative"). The cost of adopting a review setup that is at most 0.10 worse than the incumbent is not of that kind. FDA 2026 draft §III.B.3 explicitly licenses setting the threshold from the decision's consequences, so a lower pass level is within the spirit of the guidance even though 0.975 is the regulatory convention.
- **The third outcome.** A trial has "NI shown" and "NI not shown"; the Bar has pass, fail and **inconclusive**. Nothing in these documents tells you how to split "not shown" into "fail" and "inconclusive". The Bar's fail clause — an upper bound at 95% below zero — is a superiority-of-Baseline test at one-sided 2.5%, which does have a counterpart (EMA 2000 §IV.1; FDA 2016 §IV.G), but the guidance's purpose for that test is an extra claim, not a demotion rule.
- **Repetition.** A Bar runs on every Experiment; a confirmatory trial runs once. The relevant quantity for thirdshift is the long-run pass rate under A/A and the long-run fail rate under a real regression — operating characteristics, not a single trial's type I error. Guidance offers the vocabulary but no target numbers for a gate used this way.
- **The target itself.** An "80% pass target for a true no-op" is a **power** target, not a type I error budget: under A/A the true difference is 0, which is greater than -0.10, so the non-inferiority hypothesis is **true** and declaring pass is the *correct* conclusion. Measured against the guidance, the Bar's A/A pass probability is power at the null of zero difference — precisely the quantity ICH E9 §3.5 warns is miscalculated and FDA 2022 §III.B says to protect with more data and higher per-endpoint power. Nothing in the sources suggests the correct answer under A/A should be anything other than "pass, with high probability".

## Implications for the Bar

My recommendation, from the sources above.

**The sources support dropping the `upper >= 0` clause as a *pass condition*, and keeping its information as a reported annotation.**

- No primary source makes reaching zero a requirement for a non-inferiority claim (§1; verified absence across all eight documents). Two sources rule on exactly this interval and both declare non-inferiority: FDA 2016 Figure 2 scenario 6 and CONSORT 2012 Figure 1 scenario D. Both also note the situation is rare in trials because it needs a very large sample — which is why a small-suite Bar meets it far more often than a trial would, and why importing a rule the guidance never wrote is doubly unsafe here.
- Both sources nonetheless *name* the finding ("inferior in the sense that a null treatment difference is excluded"; "significantly worse than the standard"), and FDA 2016 §III.E insists the point estimate and interval width matter alongside the tested bound. The right home for that is the Verdict's reported detail — a "non-inferior, but measurably worse than Baseline" note on a PASS — not a demotion to inconclusive.
- **Replacing** it with a two-sided rule is the only standard alternative, and it is a *different claim*: equivalence, with the interval inside **±Δ** (ICH E9 §3.3.2; EMA 2025 draft §5, Figure 3). That costs more power than the current clause, not less, and would assert the Candidate is no *better* than Baseline either — not what a release gate wants.
- **Keeping** it has no support. The one thing guidance does say about the zero line is that testing it is a separate, multiplicity-free claim (EMA 2000 §IV.1; FDA 2016 §IV.G) — which is what the Bar's *fail* clause already is. Dropping `upper >= 0` and keeping the fail clause leaves the Bar in exactly the standard shape: one NI test against the margin at the pass level, one zero test at the fail level, each reported on its own.

**The pass level should be stated one-sided.** ICH E9 §3.3.2 says non-inferiority uses a one-sided interval; EMA 2000 §II.4, EMA 2025 draft §9 and FDA 2016 §III.A all give the 95%-two-sided = 97.5%-one-sided mapping; FDA 2026 draft §III.B.1 states the Bayesian threshold `c` directly against a one-sided 0.025. A central interval at level `L` has one-sided coverage `(1+L)/2` (my computation), so today's 70% central pass level is an **85% one-sided** bound on the margin. Saying "85% one-sided" makes the distance from the 97.5% regulatory convention visible, matches how every source states the NI level, and removes the need for readers to translate.

Stating it that way also explains the cap the A/A simulation found. For a posterior symmetric about zero, `P(upper bound >= 0)` is `1 - (1-L)/2 = (1+L)/2` — the clause's per-metric pass probability under A/A *is* the one-sided coverage of the pass interval (my computation). The `((1+L)/2)^k` ceiling is therefore not a quirk of the simulation but an identity, and at the regulatory level (`L` = 95% two-sided) the ceiling would be `0.975² ≈ 95%`, high enough to be invisible. The clause only became binding when the pass level was lowered — the two changes in ADR 0024 interact, and the clause is the half without precedent.

**On growing the suite.** FDA 2022 §III.B's remedy for a `k`-endpoint power product is more data plus a higher per-endpoint power target, and §III.C.1 forbids loosening the per-endpoint criterion to compensate and warns against more than two co-primary endpoints. For thirdshift that reads: once `upper >= 0` is gone, more Eval tasks do buy pass probability on the margin test (the bound tightens toward the true zero difference), the number of must-pass metrics should stay small and deliberate, and each metric's pass level should be set from the joint target — for a joint 80% over two independent metrics, about 89% each (my computation).

---

## Sources

**Regulatory guidance (primary, read in full text)**

- FDA, *Non-Inferiority Clinical Trials to Establish Effectiveness*, Guidance for Industry, November 2016 (CDER/CBER). PDF: https://www.fda.gov/media/78504/download — landing page: https://www.fda.gov/regulatory-information/search-fda-guidance-documents/non-inferiority-clinical-trials
- FDA, *Multiple Endpoints in Clinical Trials*, Guidance for Industry, October 2022 (docket FDA-2016-D-4460). PDF: https://www.fda.gov/media/162416/download — landing page: https://www.fda.gov/regulatory-information/search-fda-guidance-documents/multiple-endpoints-clinical-trials
- FDA/CDRH, *Guidance for the Use of Bayesian Statistics in Medical Device Clinical Trials*, issued 5 February 2010. PDF: https://www.fda.gov/media/71512/download — landing page: https://www.fda.gov/regulatory-information/search-fda-guidance-documents/guidance-use-bayesian-statistics-medical-device-clinical-trials
- FDA, *Use of Bayesian Methodology in Clinical Trials of Drug and Biological Products*, **draft** guidance, January 2026 (docket FDA-2025-D-3217). PDF: https://www.fda.gov/media/190505/download — landing page: https://www.fda.gov/regulatory-information/search-fda-guidance-documents/use-bayesian-methodology-clinical-trials-drug-and-biological-products
- EMA/CPMP, *Points to Consider on Switching Between Superiority and Non-Inferiority*, CPMP/EWP/482/99, 27 July 2000. PDF: https://ema.europa.eu/documents/scientific-guideline/points-consider-switching-between-superiority-non-inferiority_en.pdf — landing page: https://www.ema.europa.eu/en/switching-between-superiority-non-inferiority-scientific-guideline (EMA marks this as replaced by the 2025 draft below)
- EMA/CHMP, *Guideline on non-inferiority and equivalence comparisons in clinical trials*, **draft**, EMA/301654/2025, 24 October 2025. PDF: https://www.ema.europa.eu/en/documents/scientific-guideline/draft-guideline-non-inferiority-equivalence-comparisons-clinical-trials_en.pdf
- ICH E9, *Statistical Principles for Clinical Trials*. PDF: https://database.ich.org/sites/default/files/E9_Guideline.pdf
- ICH E10, *Choice of Control Group and Related Issues in Clinical Trials*. PDF: https://database.ich.org/sites/default/files/E10_Guideline.pdf

**Reporting standard**

- Piaggio G, Elbourne DR, Pocock SJ, Evans SJW, Altman DG, for the CONSORT Group. "Reporting of Noninferiority and Equivalence Randomized Trials: Extension of the CONSORT 2010 Statement." *JAMA* 2012;308(24):2594-2604. doi:10.1001/jama.2012.87802. https://jamanetwork.com/journals/jama/fullarticle/1487502 — **partial read**: Figure 1, its caption and footnote a were read directly; the article body was only partly retrievable, so no claim here rests on its prose.

**thirdshift files**

- [ADR 0024 — Pass and fail use separate levels](../adr/0024-pass-and-fail-use-separate-levels.md)
- `src/verdict/levels.rs` (`Intervals::decide`)
- [Bar decisions, Q2.3 and Q3.4](../grill-with-docs/1252-1253-bar-and-rechecks.md)
