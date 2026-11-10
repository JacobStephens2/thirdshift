# Process Arms, external runners and blind human review

Research date: 2026-11-09.

**Question.** [#1484](https://github.com/JacobStephens2/thirdshift/issues/1484)'s goal has widened (grill file [`1484-implement-suite.md`](../grill-with-docs/1484-implement-suite.md), "Settled by earlier answers", Q1.1). It now covers *process comparison* Experiments: the Built-in Ticket run against another multi-session process, on real past issues, in any repository thirdshift serves. Three things need answers for Round 2:
- What should an **external runner** Arm look like?
- Should a **blind human pairwise merge call** live in the Console?
- Should thirdshift **adopt a tool such as Langfuse** rather than build further?

**Method.**
- **What this builds on.** The generic lessons from one hand-run process comparison: [#1468](https://github.com/JacobStephens2/thirdshift/issues/1468)'s last triage comment, [process-comparison-cost-accounting.md](process-comparison-cost-accounting.md) and [process-comparison-grading-and-evidence.md](process-comparison-grading-and-evidence.md).
- **Not repeated here.** [eval-build-vs-buy.md](eval-build-vs-buy.md) (TL;DR, §5–8) and [eval-viewers.md](eval-viewers.md) are cited rather than redone. They covered the hosting shapes, telemetry and OTLP fit of every tool below, and §5.4's finding that no tool blinds reviewers to the system that produced an output. This note re-checks §5.4 against the *human*-review features and the pairwise UIs.
- **thirdshift was read at `0e8cc135`.**
- **Outside sources were read live on 2026-11-09** by four parallel read-only agents. They used official docs, first-party changelogs and GitHub source pinned to commits. Their quotations are relayed here, and I did not re-open every page. Where an agent said a fact came only from a search snippet, this note marks it **[unverified]**. **[interpretation]** marks my own judgement.

## TL;DR

- **Langfuse doesn't fit a process Arm, and nothing else in the field has blind human pairwise review either.** It ingests externally produced experiment items over OTLP, but "expects exactly one trace per experiment item". Its official Claude Code and Codex integrations make one trace *per turn*. It has no diff view, no pairwise annotation, no blind mode and no significance test. Adopting it would mean writing the external-runner glue *and* building the blind review page outside it (§1.1).
- **Re-checked across seven tools: none hides which Arm produced an output from a *human*, or randomises left and right.** LangSmith's pairwise queue still puts "Run A on the left and Run B on the right", "in chronological order". Its only shuffle (`randomize_order`) is for LLM judges, and it defaults to off (§1.2). [eval-build-vs-buy.md §5.4](eval-build-vs-buy.md) stands.
- **There is one new lead.** Harbor plugins for Phoenix, Braintrust and LangSmith turn a Harbor job into an experiment, with each trial as a row and its ATIF trajectory as a trace (§1.2). That is the field's nearest thing to "a process run elsewhere, viewed as an experiment". It is still neither blind nor statistical.
- **The field already treats the scaffold as the unit under test, through a narrow interface** (§2):
  - **In:** a per-trial isolated repository at the base commit, plus one instruction string.
  - **Out:** a patch or final state, graded by a separate verifier.
  - **Alongside:** a trajectory carrying token and cost totals per model and per sub-agent.

  SWE-bench is the extreme: the scaffold is fully external and only `model_patch` is graded. Harbor's `AgentContext` and ATIF are the most complete telemetry contract. Unattended runs answer questions with a scripted refusal or a simulated user, or the run ends.
- **The field's blind pairwise practice is small and consistent** (§3): anonymous outputs, randomised order, A / B / tie / both-bad, and the identity revealed only after the vote. One warning: in Copilot Arena, "users selected the first completion 86% of the time" *despite* randomised order.
- **Inside thirdshift, the Day shift already grades in the Console.** The `/scores` page is a human Grader form that hides the model's Scores until the human's are saved. But it shows the producing Harness, Model and Effort. Experiment rechecks show "Baseline" or "Candidate" and are entered from the CLI. Nothing grades pairwise (§4).
- **Recommendation [interpretation]: build the blind pairwise page in the Console, keep the runner and the Verdict, and define the external-runner Arm on Harbor's and SWE-bench's shape.** Keep OTLP export as the route to any outside viewer. See the options in §5.

## 1. Off-the-shelf tools, against a process comparison

### 1.1 Langfuse (v4.77.0, 2026-11-09, [releases](https://github.com/langfuse/langfuse/releases))

[eval-viewers.md §3.4](eval-viewers.md) covered hosting: six containers, at least 16 GiB, MIT outside `ee/`, a spans-only OTLP route and an update check with no switch. New here:

- **(a) External items: yes, but only one trace per item.**
  - The [experiments data model](https://langfuse.com/docs/evaluation/experiments/data-model) documents "Direct OTEL ingestion: … custom OTLP pipelines, or re-ingesting experiment traces". It uses `langfuse.experiment.id`, `langfuse.experiment.item.id` and `…item.root_observation_id` ([OTel experiments](https://langfuse.com/integrations/native/opentelemetry/experiments)), with the Arm's output on the root span's `langfuse.observation.output`.
  - "There is no public REST endpoint for creating new experiment runs". The legacy `dataset-run-items` route is deprecated, and the SDK's `item.link()` was removed ([via SDK](https://langfuse.com/docs/evaluation/experiments/experiments-via-sdk)).
  - "Langfuse expects exactly one trace per experiment item". A process Trial that ran anywhere from 4 to 72 sessions (#1468) would have to be re-emitted as one synthetic root trace. That is the same mismatch [eval-viewers.md](eval-viewers.md) found against ADR 0019's one trace per command.
- **(b) Diffs: no.**
  - [Compare](https://langfuse.com/docs/evaluation/experiments/compare-experiments) shows "the baseline and candidate outputs side by side", but no text diff is documented.
  - Large observation content is now kept intact ([changelog 2026-07-24](https://langfuse.com/changelog/2026-07-24-large-observation-content)). Attachments have no patch type ([multi-modality](https://langfuse.com/docs/observability/features/multi-modality)).
- **(c) Blind human review: no.**
  - [Annotation queues](https://langfuse.com/docs/evaluation/evaluation-methods/annotation-queues) hold traces, observations or sessions, and annotators score them on configured dimensions. Nothing on the page hides metadata, randomises order, offers a blind mode or collects a pairwise preference.
  - [Annotating from the compare view](https://langfuse.com/changelog/2025-10-23-annotate-from-compare-view) shows which experiment a cell belongs to.
  - Langfuse's own suggestion for custom review flows is to [build your own annotation UI](https://langfuse.com/blog/2025-11-25-vibe-coding-custom-annotation-ui).
  - Annotation queues, experiments and datasets are free in the open-source self-host ([pricing-self-host](https://langfuse.com/pricing-self-host)).
- **(d) Claude Code and Codex: official integrations, built on hooks rather than OTLP.**
  - [Claude Code](https://langfuse.com/integrations/developer-tools/claude-code) uses a Stop hook and writes one trace per turn, grouped by `session_id`.
  - [Codex](https://langfuse.com/integrations/developer-tools/codex) uses a Stop-hook plugin that reads the rollout file.
  - These traces are observability, not experiment items.
- **(e) Licence and cost.** Covered in [eval-viewers.md §3.4](eval-viewers.md).
- **(f) Statistics.** [Score analytics](https://langfuse.com/docs/evaluation/evaluation-methods/score-analytics) (beta) offers correlation, Cohen's κ, F1 and MAE. It has no confidence interval, test or verdict. The compare docs themselves say "The average alone does not tell you whether the candidate is safe to release."

**[interpretation]** The one place Langfuse would help is agreement statistics, such as κ between the human and the model Graders. That is a few lines of code next to the Verdict.

### 1.2 The others, briefly

None of these documents a way to hide which Arm produced an output from a human, or to randomise left and right for one. Hosting and licences are in [eval-build-vs-buy.md §5.8](eval-build-vs-buy.md).

- **Braintrust.**
  - **Ingest:** `experiment.log()` writes any rows ([SDK](https://www.braintrust.dev/docs/reference/libs/python)). The [Harbor integration](https://docs.braintrust.dev/integrations/sdk-integrations/harbor.md) makes "each trial … one experiment row" with its ATIF trajectory as a trace.
  - **Compare:** a side-by-side diff toggle and a pairwise win/loss view ([compare](https://docs.braintrust.dev/evaluate/compare-experiments.md)). A knowledge-base article caps the diff at 4,096 characters per field ([KB](https://www.braintrust.dev/docs/kb/prompt-diff-feature-has-4096-character-limit)), which is too small for a pull request.
  - **Integrations:** Claude Code and Codex ([CC](https://docs.braintrust.dev/integrations/developer-tools/claude-code.md), [Codex](https://docs.braintrust.dev/integrations/developer-tools/codex.md)).
  - **Statistics:** deltas and counts only ([interpret results](https://docs.braintrust.dev/evaluate/interpret-results.md)).
- **LangSmith.**
  - **Ingest:** "upload experiments run outside of LangSmith" ([upload](https://docs.langchain.com/langsmith/upload-existing-experiments)).
  - **Compare:** a diff mode in the comparison view ([compare](https://docs.langchain.com/langsmith/compare-experiment-results.md)).
  - **Pairwise queues:** fixed left and right, paired chronologically ([annotation queues](https://docs.langchain.com/langsmith/annotation-queues)). `randomize_order` applies to LLM judges only and defaults to False ([`evaluate_comparative`](https://langsmith-sdk.readthedocs.io/en/stable/evaluation/langsmith.evaluation._runner.evaluate_comparative.html)).
  - **Integrations:** Claude Code and Codex ([CC](https://docs.langchain.com/langsmith/trace-claude-code), [Codex](https://docs.langchain.com/langsmith/trace-with-codex)).
  - **Statistics:** counts of better and worse.
- **Arize Phoenix (ELv2).**
  - **Ingest:** a REST route creates experiment runs from outside ([API](https://www.arize.com/docs/phoenix/sdk-api-reference/rest-api/api-reference/experiments/create-run-for-an-experiment)). A [Harbor plugin](https://arize.com/docs/phoenix/release-notes/09-2026/09-08-2026-harbor-plugin-for-agent-benchmarks.md) (2026-09-08) maps each agent configuration to an experiment and each trial to a run.
  - **Human review:** [2026-11-02](https://arize.com/docs/phoenix/release-notes/11-2026/11-02-2026-authentication-by-default-and-decision-annotations.md) added DECISION annotations and authentication by default. Neither is blind or pairwise.
  - **Diff:** a diff output mode is **[unverified]**.
- **promptfoo.** Externally logged outputs come in through the [echo provider](https://www.promptfoo.dev/docs/providers/echo/). It compares in side-by-side columns, has no text diff and no blinding.
- **Inspect AI and Inspect Scout.**
  - The [log viewer](https://inspect.aisi.org.uk/log-viewer.html) has no side-by-side or diff view.
  - Its statistics stop at confidence intervals, with no paired test ([metrics](https://inspect.aisi.org.uk/metrics.html)).
- **Harbor (Apache-2.0).**
  - It is a runner, not a review store, and has no annotation feature.
  - [`harbor view`](https://docs.harborframework.com/results/view-job-results.md) shows trajectories and a file tree, with no git diff and no comparison of jobs in the official docs.

**[interpretation]** What the tools share is a viewer for *one* system's runs plus a comparison table of scores. None of them treats "which pull request would you merge, without knowing whose it is" as a feature. Every one of them would need that page built on top.

## 2. The scaffold as the unit under test

Each framework below has an agent interface that thirdshift can learn from.

- **SWE-bench: fully external.**
  - **Input:** a predictions JSONL of `{instance_id, model_name_or_path, model_patch}`. The harness applies the patch in Docker and runs the tests ([evaluation guide](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/docs/guides/evaluation.md#L51-L60)).
  - **Leaderboard submission:** it must include `trajs/` that are "human-readable" and produced during inference. A best@k system must show every rollout and the selector. `metadata.yaml` names the `agent` and says `model_display: Multiple` for mixed models ([experiments README](https://github.com/SWE-bench/experiments/blob/40f164d5b8f1d249bf95a6df8b74b577fd8e519d/README.md#L127-L224), [checklist](https://github.com/SWE-bench/experiments/blob/40f164d5b8f1d249bf95a6df8b74b577fd8e519d/checklist.md#L28-L83)).
  - **Forbidden:** the `hints` field, and web lookup of solutions.
  - **Cost:** no cost reporting is required (not found in either file).
- **Harbor: the fullest contract.**
  - **Interface:** `BaseAgent.run(instruction: str, environment, context: AgentContext)` ([base.py](https://github.com/harbor-framework/harbor/blob/9c7a39b991d7e2205019f6be0598cbd22d84bfc3/src/harbor/agents/base.py#L653-L710)). `BaseInstalledAgent` adds `install()` and `configure()` and runs the CLI inside the container ([installed/base.py](https://github.com/harbor-framework/harbor/blob/9c7a39b991d7e2205019f6be0598cbd22d84bfc3/src/harbor/agents/installed/base.py#L1149-L1175)).
  - **Usage:** `AgentContext` carries `n_input_tokens` (cache included), `n_cache_tokens`, `n_output_tokens`, `cost_usd`, and `model_usage` per model "including subagents" ([context.py](https://github.com/harbor-framework/harbor/blob/9c7a39b991d7e2205019f6be0598cbd22d84bfc3/src/harbor/models/agent/context.py#L8-L52)). It is backfilled from the ATIF trajectory when the agent leaves it empty. ATIF v1.8 has per-step `model_name` and `metrics`, nested `subagent_trajectories`, and a `final_metrics.total_cost_usd` that includes subagents.
  - **Grading:** the verifier writes `/logs/verifier/reward.txt` or `reward.json`.
  - **Isolation:** one environment per trial.
  - **Questions:** Harbor has simulated-user trials ([simulated_user.py](https://github.com/harbor-framework/harbor/blob/9c7a39b991d7e2205019f6be0598cbd22d84bfc3/src/harbor/trial/simulated_user.py#L14-L17)).
- **OpenHands benchmarks.**
  - **Agents:** `agent_type` is one of `default`, `acp-claude`, `acp-codex` or `acp-gemini` ([models.py](https://github.com/OpenHands/benchmarks/blob/542cb71c860c11d6a70238a1dd4b4ac7a3e0d4d2/benchmarks/utils/models.py#L118)), routed through a LiteLLM proxy for accounting.
  - **Output:** `git diff` goes into `test_result["git_patch"]`.
  - **Cost:** `report_costs.py` sums `accumulated_cost`, critic attempts included.
  - **Questions:** an agent that asks gets "YOU SHOULD NEVER ASK FOR HUMAN HELP" ([fake_user_response.py](https://github.com/OpenHands/benchmarks/blob/542cb71c860c11d6a70238a1dd4b4ac7a3e0d4d2/benchmarks/utils/fake_user_response.py#L59-L63)).
- **Inspect.**
  - **Interface:** `Agent` is `async (state: AgentState) -> AgentState`. `sandbox_agent_bridge()` proxies the Anthropic, OpenAI and Google APIs inside the sandbox, and `model_aliases` maps a sub-agent's model names to roles ([agent-bridge](https://github.com/UKGovernmentBEIS/inspect_ai/blob/033726e73482aa87fd65139fcc61c3a2fea0ce6d/docs/agent-bridge.qmd#L249-L279)).
  - **Usage:** each sample log keeps `model_usage` and `role_usage` ([_log.py](https://github.com/UKGovernmentBEIS/inspect_ai/blob/033726e73482aa87fd65139fcc61c3a2fea0ce6d/src/inspect_ai/log/_log.py#L328-L331)). This is the only per-*role* cost field in the field, and it is exactly #1468's per-role accounting.
  - **The catch:** the bridge works by holding the API keys, so it can't serve subscription logins ([eval-build-vs-buy.md §5.1](eval-build-vs-buy.md)).
- **METR.**
  - Its Task Standard deliberately leaves "How agents are defined" open ([README](https://github.com/METR/task-standard/blob/83ad6fd1e9d9cba9885341d1efd23a0aff472fe1/README.md#L146-L152)).
  - Its [2026-02-13 note](https://metr.org/notes/2026-02-13-measuring-time-horizon-using-claude-code-and-codex/) treats the scaffold (ReAct, Triframe, Claude Code, Codex) as the variable. It runs them "without any human input" and counts a run that ends by addressing a user against the scaffold.

**[interpretation] What this suggests for an external-runner Arm.** These are inputs for Round 2, not decisions.
- **In:** a time-safe prepared repository, as `evals prepare` already builds, plus the issue text as it was at the base. It gets no hints and no access to the shipped fix.
- **Out:** a branch, or `git diff` against the base. Graded only by thirdshift's code Graders (Q1.5) and blind Graders.
- **Telemetry:** a directory of per-session traces, each labelled with its role, provider, Model and token usage. Harbor's `AgentContext`/ATIF and Inspect's `role_usage` show what one could look like. Cost is priced by thirdshift from a price table dated and fixed with the Bar, never by the runner ([cost accounting §4](process-comparison-cost-accounting.md)).
- **Pinned in the Arm record:** the command's hash, every skill commit, each Harness's path and version, and Model and Effort per role (#1468).
- **Questions:** the Arm declares a policy. It is the scripted refusal (OpenHands), a brief, or "a question ends the Trial as no delivery". METR and OpenHands both treat asking as the scaffold's own outcome, so the third policy counts as a failure rather than a Void Trial.
- **Isolation:** one sandbox for the whole runner, holding every child session and every provider login.

## 3. Blind human pairwise review: what practice says

- **Chatbot Arena** ([Chiang et al. 2024](https://arxiv.org/html/2403.04132)).
  - **Pairing:** "two anonymous models are sampled", and keyword filtering removes identity leaks.
  - **Votes:** A, B, "tie" or "both are bad", with identities "revealed only after voting".
  - **Statistics:** Bradley–Terry.
  - **Style tells:** in the [style-control post](https://lmsys.org/blog/2024-08-28-style-control/), "Style indeed has a strong effect". It is handled by modelling style as a covariate, length above all.
- **Copilot Arena** ([2502.09328](https://arxiv.org/html/2502.09328)).
  - **Order:** "we randomize the ordering of the completions to remove top-bottom bias", yet "Users selected the first completion 86% of the time".
  - **Timing:** both completions are shown only once both have finished, because latency changes votes.
  - **Reveal:** the model pair is revealed after the vote.
- **BigCodeArena** ([2510.08697](https://arxiv.org/html/2510.08697v2)). It shows "two anonymized responses" with execution results and runnable artifacts. Its four classes are A better, B better, Tie and Both Bad, with optional sub-criteria.
- **SWE-Agent-Arena** ([space](https://swe-arena-swe-agent-arena.hf.space/)) is the closest match to two pull requests. Each agent "works in its own isolated git repo. You see actual code changes", and the votes are A / B / Tie / Tie (Both Bad). It was not found in a peer-reviewed paper.
- **Human agreement.** Annotators agreed about 81% of the time with ties excluded, and about 63% with ties counted ([MT-Bench](https://arxiv.org/html/2306.05685)). On blinded patch pairs, three experienced developers reached Fleiss' κ 0.690 on reviewability ([2609.04061](https://arxiv.org/html/2609.04061v2)).
- **Anthropic** ([Demystifying evals for AI agents](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)). Human graders are "Gold standard quality" but slow, and are "Used to calibrate model-based graders". The post says "You won't know if your graders are working well unless you read the transcripts." It does not cover blind human review.

Biases among model Graders and the ten-pair statistics are in [process-comparison-grading-and-evidence.md §1–2](process-comparison-grading-and-evidence.md).

**[interpretation] A minimal page for blind pairwise review.**
1. **One pair at a time.** It shows the issue as it was at the base, and two diffs labelled Change A and Change B, rendered identically.
2. **Order** is randomised per pair, using the Bar's seed. The side each Arm was shown on is recorded, so position bias can be measured afterwards. Copilot Arena's 86% shows it will be there.
3. **Attribution is stripped before rendering.** That covers commit messages and trailers, branch names, Built-with and Generated-with lines, pull-request bodies, CI check names and timestamps, as the Scoring run's packets already do (guide.md:3645–3652). Cross-test and CI results are shown only when they are produced the same way for both Arms.
4. **Decision:** A, B, Tie, or Neither mergeable. A short reason is required. An optional "I could tell which process made this" box measures how far the blinding leaked.
5. **Reveal** only after the decision is saved. The decision is append-only, like recheck history.
6. **Recorded with the decision:** reviewer, start and decision times, seed, sides, and hashes of the two diffs. Covariates such as diff size are stored for style adjustment.
7. **No running tally** is visible before the reviewer decides.

## 4. What thirdshift already has

- **The Day shift is a Grader by definition.** GLOSSARY.md:404–405 says a Grader is "a check in code, an agent session, or the **Day shift**".
- **Scoring run human scoring is a Console form, but it is not blind to the producer.**
  - `/scores` takes all five Scores with reasons through a CSRF-protected form (src/console/scores.html:16–17). It shows "Model Scores hidden until all five of yours are saved" (scores.html:15). Re-entry is "blind too" (src/console/scoring.rs:44).
  - Each row prints `harness · model · effort · version` (scores.html:11).
  - So the human is blind to the *model Grader*, not to *who made the work*. It grades one pull request on absolute criteria, not a pair.
- **Experiment rechecks are human Scores entered in the CLI, and they show the Arm.**
  - The Experiment page's "Waiting on you" callout lists each pending recheck as "task · Baseline/Candidate Trial n" (src/console/experiments/render.rs:552–561, `arm_title` at :646). It offers a copyable `thirdshift recheck` command (:509–517).
  - The guide says "Rechecks stay in the CLI" and "the Console records no recheck itself" (docs/guide.md:1441, :1547–1548).
  - The API table nonetheless lists `POST /experiments/<id>/rechecks` with `act` scope (guide.md:762), so a write route already exists.
- **Model Graders are already blind and shuffled, but they grade each Trial separately, not as pairs.** Each packet holds every Trial's reports "with shuffled neutral labels" and the Arm lines removed (guide.md:1366–1369). The shuffle is seeded from the Bar's seed per Eval task (src/evals/experiment/model_grader.rs:146–147). Each Trial gets a pass/fail, so pairing happens in the Verdict, not in the Grader.
- **The Experiment page has no diff view.** Review Trials produce reports, not code (render.rs:279), and a Trial's Peek shows Grader reasons, rechecks, void causes and session links (guide.md:1536–1537).

**[interpretation]** Three pieces of a blind pairwise review page already exist:
- **a form pattern:** `/scores`, with CSRF, an `act` scope and reveal-after-entry;
- **a packet builder that strips attribution:** the Scoring run's;
- **a seeded shuffle:** the model Grader's.

What is new is the pairwise *unit* (two Trials of one Eval task), hiding the Arm, and a diff renderer. This is roughly the "Replay pages" share of the 2,100–4,000-line Console estimate in [eval-viewers.md §1.5](eval-viewers.md), and probably less, since nothing here needs a span tree.

## 5. Options for #1484 Round 2

**A. Build it in.** **[interpretation]**
- **What it is:** an external-runner Arm kind (§2) plus a blind pairwise page in the Console (§3).
- **Where the human call goes:** the pairwise call is a Day-shift Grader whose decision becomes a paired Score. It goes into the Verdict, or is reported beside it, as the code and model Graders already are.
- **For:**
  - It is the only option where the human call shares the Bar's seed, the void rules and the attribution stripping.
  - Every tool in §1 would need this page built anyway.
  - It reuses `/scores`' form, the model Grader's shuffle and the packet stripper.
  - It keeps ADR-0015 and the no-hosted-service constraint intact.
- **Against:**
  - More Console code to maintain.
  - A diff renderer to write.
  - The external-runner sandbox must hold several providers' logins at once, which is new work under any option.

**B. Adopt Langfuse, or a peer, as the experiment store and review UI.**
- **For:**
  - Free annotation queues.
  - Ready-made trace views.
  - κ analytics.
- **Against:**
  - One trace per item doesn't fit a 4–72-session Trial.
  - No diff view, no pairwise human annotation, no blinding and no statistics, so the blind page *and* the Verdict still have to be built.
  - Six containers and 16 GiB.
  - Two stores of truth.

  **[interpretation]** It brings the cost of B and still needs A's work.

**C. Keep the runner and Verdict; export to an outside viewer for browsing only.**
- **What it is:**
  - thirdshift keeps Arms, Trials, blinding, the human pairwise page and the Verdict.
  - It exports Trial trajectories (OTLP today, [eval-build-vs-buy.md §6.1](eval-build-vs-buy.md)) or ATIF to Phoenix, Langfuse or Scout for unblinded *post-hoc* trace reading.
- **For:**
  - Reversible.
  - No new dependency in the accuracy path.
  - The Harbor-plugin pattern (§1.2) shows the mapping already exists elsewhere.
- **Against:**
  - The human page still has to be built (A's page).
  - The viewer must only ever show traces *after* the pairwise decisions are recorded, or it breaks blinding.

**D. Make Harbor the external runner, with thirdshift grading.**
- **What it is:** a process Arm is a Harbor `BaseInstalledAgent`, whose `AgentContext` and ATIF give the per-model, per-sub-agent usage. thirdshift consumes the branch and the trajectory.
- **For:** a documented, versioned contract instead of a home-made one.
- **Against:**
  - Docker images per repository. The new host makes this possible ([§8.1](eval-build-vs-buy.md)).
  - Subscription logins only for Claude and Codex.
  - A Python dependency on the Trial path.

  **[interpretation]** A thirdshift-native external-runner contract *shaped like* Harbor's, with optional ATIF output, gets most of the benefit without the dependency.

**[interpretation] Leaning:** A for the human page and the Arm. C as the route to any outside viewer, since the OTLP export already exists. D as the shape to copy, not the dependency to take. B only if the Day shift wants a general trace UI for other reasons. Even then it would sit beside thirdshift's Verdict, not replace it.
