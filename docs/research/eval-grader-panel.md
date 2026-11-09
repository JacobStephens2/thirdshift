# The pairwise model Grader panel: Harnesses, family diversity, size and cost

Research date: 2026-11-09.

**Question.** [#1484](https://github.com/JacobStephens2/thirdshift/issues/1484)'s grill (Q3.8, Q4.2) puts a panel of model Graders into implement Experiments. Each Grader reads a Baseline/Candidate pair in both presentation orders, and a call that flips with the order counts as a tie. It gives a per-criterion rubric and a merge-ready call per side. The panel majority stands in for a human on unsampled pairs. Jacob wants panels of 1 to about 11 Models, some several per family. Four questions:

1. Which existing Harness should run Kimi K3.1 Pro on Fireworks and DeepSeek V4.1 Flash 1017 on Baseten, and what must thirdshift change?
2. Do models from one family make correlated judging errors, or share a self-/family-preference? Is a second same-family Model independent signal?
3. What does the literature say on panel size, aggregation and position bias?
4. What does a grading session cost per Model?

**Method.** Code read at this branch's HEAD: [`src/proxy.rs`](../../src/proxy.rs), [`src/sandbox.rs`](../../src/sandbox.rs), [`src/harness/opencode.rs`](../../src/harness/opencode.rs), [`src/evals/experiment/model_grader.rs`](../../src/evals/experiment/model_grader.rs), [`docs/guide.md`](../guide.md). Provider facts come from the providers' own docs and from [models.dev](https://models.dev), the catalog OpenCode reads, whose provider and model TOML files were read raw from [`sst/models.dev`](https://github.com/sst/models.dev). Prices are from the vendors' own pricing pages, read live on 2026-11-09, except where marked. Papers were read on arXiv. Where a paper was already read in full for [multi-family-review.md](multi-family-review.md), I cite that note's reading and re-checked only the numbers marked *re-checked*. **[interpretation]** marks my reading rather than a documented fact.

**Out of scope.** Whether model Graders' calls count toward the Verdict at all, and the human sample size (grill Q4.1). The rubric's criteria. Prompt design for the Grader.

## TL;DR

- **Run both through OpenCode.** Both providers are built into OpenCode's catalog: `fireworks-ai` with `FIREWORKS_API_KEY`, base `https://api.fireworks.ai/inference/v1/`, and `baseten` with `BASETEN_API_KEY`, base `https://inference.baseten.co/v1`. Both list tool calling for these exact Models. The Model strings would be `fireworks-ai/accounts/fireworks/models/kimi-k3p1-pro` and `baseten/deepseek-ai/DeepSeek-V4.1-Flash-1017`. Both are API-key billed. §1.
- **A new Harness isn't worth it yet.** Moonshot's Kimi CLI has a headless `--print --output-format stream-json` mode, but documents no usage output. DeepSeek's own Harness is a preview desktop/web app with no headless mode. A Grader's short session gains little from a vendor's tool loop. Revisit Kimi CLI if Kimi becomes an Arm. §1.4.
- **thirdshift needs three changes.** (1) Two lines in `proxy.rs`'s `provider_hosts` (`api.fireworks.ai`, `inference.baseten.co`). Today an OpenCode Grader naming either provider fails before its session, with "unknown provider". (2) Per-provider OpenCode keys in the Credentials, since there is only one `opencode.api_key` and a panel would use two providers. (3) The panel itself: `model_grader.rs` picks one Grader today. §1.3.
- **Same-family judges are only slightly more correlated than cross-family ones, and at the frontier no more so.** Kohli (2026): same-family pairs φ = 0.437 and 0.435, cross-family mean 0.389. The three most correlated pairs were all cross-family. Hossain et al. (2026): three Gemini judges 0.42 against 0.56 for GPT + Claude + Grok. A second same-family Model adds a little independent signal, about as much as a second family would. §2.1.
- **Family preference is the real reason to cap a family's weight.** Errors are not the problem, but bias is directional. Judges over-credit their own family's code (Pombal et al.: family-level ratios 1.62–11.91, 11 of 12 judges above 1), and a five-family committee shrank it without removing it. §2.2.
- **Panels saturate early.** Nine judges from seven families carried 2.18 effective votes, with the first five giving 90% of that, and the best single judge matched or beat the panel. Reweighting did not recover the loss. §3.
- **Cost at API rates:** about $0.04 (DeepSeek) to $1.90 (GPT-6 Astra, Gemini 4 Argon after 1 December) per session, on an assumed 400k-input/15k-output session. That is roughly $900 for 11 Models × 120 sessions. Only the two OpenCode Models are necessarily per-token; the CLI Harnesses bill to their logins. §4.

## 1. A Harness for Kimi on Fireworks and DeepSeek on Baseten

### 1.1 What the code allows

- **OpenCode already takes any provider/Model in its config.** It passes `-m <provider>/<model>[#effort]` ([`opencode.rs`](../../src/harness/opencode.rs) `session`). The guide says the Model id comes "from your provider config, as `<provider>/<model>`" ([guide](../guide.md#harness-model-and-effort)).
- **Model Graders run sandboxed.** `model_grader.rs` runs the Grader through `session::private::run`, which builds `sandbox::for_session`. So a Grader gets the same allowlisting proxy as a Trial.
- **The proxy allows per Harness.** In [`proxy.rs`](../../src/proxy.rs) `allowed_hosts`:
  - `claude`: `api.anthropic.com`, `claude.ai`, `platform.claude.com`.
  - `codex`: `api.openai.com`, `chatgpt.com`, `auth.openai.com`.
  - `grok`: `api.x.ai`. `muse`: `api.meta.ai`. `agy`: `antigravity-unleash.goog`, `play.googleapis.com`.
  - `opencode`: `opencode.ai` and `models.opencode.ai`, plus the hosts of the provider its Model names. `opencode_provider` takes the text before the first `/`, and `provider_hosts` knows only `anthropic`, `openai`, `xai` and `meta`. Anything else fails with "unknown provider … the Trial's Model must name a provider this proxy allows".
- **Hosts are exact names**, matched case-insensitively before DNS (`allowed_name`). Adding a provider means one match arm and one `const` list.
- **Keys.** A sandboxed OpenCode session gets its key only from `opencode.api_key` in `~/.thirdshift/credentials.toml`. It goes through `THIRDSHIFT_OPENCODE_API_KEY`, and an `OPENCODE_CONFIG_CONTENT` sets *the named provider's* `apiKey` to `{env:…}`, with web fetch and web search denied ([`sandbox.rs`](../../src/sandbox.rs) `opencode_into`, `provider_config`). There is one key for the installation.

### 1.2 What the providers and OpenCode document

- **Fireworks.**
  - models.dev provider `fireworks-ai`: `env = ["FIREWORKS_API_KEY"]`, `npm = "@ai-sdk/openai-compatible"`, `api = "https://api.fireworks.ai/inference/v1/"` ([provider.toml](https://github.com/sst/models.dev/blob/dev/providers/fireworks-ai/provider.toml)).
  - The Kimi K3.1 Pro entry is `accounts/fireworks/models/kimi-k3p1-pro`. It cites Fireworks' [model card](https://fireworks.ai/models/fireworks/kimi-k3p1-pro): "Function Calling Supported", 1,040k context. Its reasoning efforts are `low | medium | high | max`.
  - Fireworks documents the base URL on its [OpenAI-compatibility page](https://docs.fireworks.ai/tools-sdks/openai-compatibility), and tools on its [function-calling guide](https://docs.fireworks.ai/guides/function-calling).
- **Baseten.**
  - models.dev provider `baseten`: `env = ["BASETEN_API_KEY"]`, `api = "https://inference.baseten.co/v1"` ([provider.toml](https://github.com/sst/models.dev/blob/dev/providers/baseten/provider.toml)).
  - The `deepseek-ai/DeepSeek-V4.1-Flash-1017` entry has `tools = { supported = true }`, 1,048,576 context, and efforts `none | low | medium | high | max`.
  - Baseten's [Model APIs overview](https://docs.baseten.co/inference/model-apis/overview) states tool calling for its Model APIs.
- **OpenCode** reads its provider catalog from models.dev, and documents custom OpenAI-compatible providers for anything missing ([providers docs](https://opencode.ai/docs/providers/)). Neither provider needs one.
- **Alternatives considered.**
  - Moonshot's [Kimi CLI](https://moonshotai.github.io/kimi-cli/en/configuration/providers.html) can reach any Chat Completions endpoint (`type = "openai_legacy"`).
  - DeepSeek's [Harness](https://www.deepseek.com/en/harness/) is "in preview" and documented as a desktop app or web UI (`npx @deepseek-ai/dsh web`), with no headless mode or third-party provider documented.
  - Each would be a seventh and eighth Harness adapter (stream, export, sandbox home, Setup) for one Grader seat apiece. **[interpretation]** Not worth it while OpenCode covers both.

### 1.3 Recommendation and changes

| Model | Harness | Model string | Auth |
|---|---|---|---|
| Kimi K3.1 Pro (Fireworks) | `opencode` | `fireworks-ai/accounts/fireworks/models/kimi-k3p1-pro` | Fireworks API key |
| DeepSeek V4.1 Flash 1017 (Baseten) | `opencode` | `baseten/deepseek-ai/DeepSeek-V4.1-Flash-1017` | Baseten API key |

Changes thirdshift needs:

1. **Proxy.** Add `const FIREWORKS_HOSTS = ["api.fireworks.ai"]` and `BASETEN_HOSTS = ["inference.baseten.co"]`, and match arms `"fireworks-ai"` and `"baseten"` in `provider_hosts`, with a test like the existing unknown-provider one.
2. **Keys per provider.** For example, a `[opencode.keys]` table keyed by provider id, with `opencode.api_key` kept as the fallback. `provider_config` already scopes the key to the provider the Model names, so only the lookup changes. Without this, a panel with both Models fails one of them.
3. **The panel.** `model_grader.rs` stores one `grader.json`. Its rule picks the first configured Harness whose Model string differs from both Arms' and falls back to the default Harness. That is an exact-string rule, not a family rule. A panel needs a list of (Harness, Model, Effort, family) seats fixed in the Bar.
4. **Effort.** **[interpretation]** OpenCode's `#effort` variant naming for these Models was not checked live. The minimal test call the OpenCode check already makes would catch a wrong one.

### 1.4 Adding a new Harness instead

Jacob is open to new Harnesses, so here is the cost weighed against the benefit.

**What a Harness costs to add.**
- [`adapter.rs`](../../src/harness/adapter.rs)'s `Adapter` trait has 16 methods: session, Resume, Trial and code-review invocations, summary, check, Setup questions, stream interpretation, stop signals, environment, instruction fallback and skill loading.
- The existing modules run about 450–1,450 lines with tests. OpenCode is 979 (`opencode.rs` plus `events`, `export` and `stream`). Muse is 1,444. Grok is 423.
- Each also needs:
  - a proxy host list (§1.1);
  - a sandbox home and login path in `sandbox.rs`;
  - Credentials or login handling;
  - Setup's Model and Effort questions;
  - a `[harness.<name>]` section and guide text, under AGENTS.md's hand-written-site rule.
- OpenCode and Muse each landed as one pull request ([#439](https://github.com/JacobStephens2/thirdshift/pull/439), [#438](https://github.com/JacobStephens2/thirdshift/pull/438)).

**Candidates.**
- **Kimi CLI (Moonshot)** is vendor-maintained and headless-capable. Its [command reference](https://moonshotai.github.io/kimi-cli/en/reference/kimi-command.html) documents:
  - `--print` (non-interactive) and `--output-format stream-json` (JSONL, with no event schema documented);
  - `--yolo` auto-approval, `--session/--resume [ID]` and `--model`.
  - It reaches Fireworks through an `openai_legacy` provider with `base_url` and `api_key` ([providers](https://moonshotai.github.io/kimi-cli/en/configuration/providers.html)).
  - The page says nothing on usage or token output, so thirdshift's usage accounting could not be assumed.
  - Moonshot's newer [kimi-code](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/configuration/config-files.md) docs list different provider types (`kimi, anthropic, openai, openai_responses, …`) and a different config path. The CLI surface is still moving.
- **DeepSeek Harness** is not viable today. It is "in preview, with evolving core plugins and APIs", is launched as a desktop app or web UI (`npx @deepseek-ai/dsh web`), and documents no headless mode and no third-party provider ([deepseek.com/harness](https://www.deepseek.com/en/harness/)). The terminal agents DeepSeek's own [awesome-deepseek-agent](https://github.com/deepseek-ai/awesome-deepseek-agent/blob/main/docs/deepcode.md) list links to are community-built.
- **Other generic OpenAI-compatible agent CLIs** would duplicate what OpenCode already does, at a full adapter's cost.

**Benefit for a Grader seat.**
- **[interpretation]** The vendor's own agent loop matters most for long, tool-heavy Trials, where tool-call formatting and the prompt scaffold shape outcomes.
- A pairwise Grader reads a packet and writes `scores.json`: few turns, few tools. Tool-use fidelity therefore buys little.
- The serving provider (Fireworks, Baseten) is the same either way, so the vendor CLI would not change the Model's behaviour at the endpoint.

**Leaning:** OpenCode now. Revisit a Kimi CLI Harness only if Kimi becomes an *Arm*, since that is where its own loop would matter. If so, measure first whether its `stream-json` carries usage. No DeepSeek Harness until one ships headless.

## 2. Within-family diversity

### 2.1 Correlated errors

- **Kohli, "Nine Judges, Two Effective Votes"** ([arXiv:2605.29800](https://arxiv.org/abs/2605.29800), Apple, May 2026, preprint; *re-checked*). Nine judges from seven families, two each from OpenAI and Meta. Not code: NLI and RewardBench.
  - Same-family error correlation: OpenAI–OpenAI φ = 0.437, Meta–Meta 0.435, cross-family mean 0.389 (§5.3).
  - "The three most correlated pairs are all cross-family": Claude × Gemini 0.603, GPT-4o × Claude 0.588, Mistral × DeepSeek 0.564.
  - Keeping one judge per family (7) lowered effective votes from 2.18 to 1.93: "Family diversity alone does not recover independence."
- **Hossain, Yousefi, Lim, "Agreement Overstates Evidence"** ([arXiv:2609.22512](https://arxiv.org/abs/2609.22512), September 2026, preprint; *re-checked*). Pairwise preference judging.
  - Three Gemini judges' mean error correlation was 0.42 on RewardBench and 0.32 on PKU-SafeRLHF. GPT + Claude + Grok had 0.56 and 0.60 (Table 1). §4.2's text gives 0.40 vs 0.42 for a differently built comparison.
  - "Judges from different providers are about as correlated as judges from the same provider." Frontier–open-weight pairs were the least correlated (0.07).
- **Kim et al., ICML 2025** ([arXiv:2506.07962](https://arxiv.org/abs/2506.07962)), as read in [multi-family-review.md §3.2](multi-family-review.md#32-correlated-errors-across-models). Being from the same company adds +0.02 to +0.07 to error agreement. Accuracy terms dominate: "Larger and more accurate models have highly correlated errors, even with distinct architectures and providers."

**[interpretation]** On errors, a second same-family Model is worth about what a second family is worth: a little. What buys independence is a different capability tier or training lineage, more than a different logo. None of these studies is on code review.

### 2.2 Self- and family-preference

- **Panickssery, Bowman, Feng** ([NeurIPS 2024](https://arxiv.org/abs/2404.13076)). Self-recognition drives self-preference. GPT-4's out-of-the-box self-preference was 0.705–0.912, where 0.5 is neutral. Summarisation only.
- **Li et al., "Preference Leakage"** ([arXiv:2502.01534](https://arxiv.org/abs/2502.01534), ICLR 2026). Defines relatedness as "being the same model, having an inheritance relationship, and belonging to the same model family". Leakage was 23.6% for the same model and 8.9% for the same family and series (Table 2, via [multi-family-review.md §3.1](multi-family-review.md#31-self-preference-and-family-preference-in-llm-judges)). Family preference is real but smaller than self-preference.
- **Pombal, Rei, Martins** ([arXiv:2604.06996](https://arxiv.org/abs/2604.06996)), on code (same note, §3.1).
  - Judges over-credited relatives' failing code: family-level ratios GPT-5 11.91, Claude Haiku 1.70, Claude Sonnet 1.62. 11 of 12 judges were above 1.
  - A five-family majority committee cut GPT-5's self ratio from 20.15 to 4.18. It "does not fully eliminate it".
- **Roytburg et al.** ([arXiv:2601.22548](https://arxiv.org/abs/2601.22548), ICML 2026) find most measured self-preference is the judge being wrong on items it also got wrong. That is the correlated-error effect again, not identity.

**[interpretation]** Errors and bias point different ways for vote weighting:

- **Errors:** same-family votes are nearly as independent as cross-family ones, which argues for one vote per Model.
- **Bias:** preference is directional and shared within a family. If an Arm's Model belongs to a family with three seats, three correlated nudges land the same way.
- **The same-family-as-Arm rule (Q4.2 (a)) handles the bias** whatever the vote unit. Once that family is out of the vote, the remaining argument for one vote per family is composition. Without it, a panel of three Claude, three GPT and one each of five others gives two vendors six of eleven votes.

## 3. Panel size, aggregation and position bias

- **Size saturates around five.**
  - Kohli: the first five judges reach 90% of the nine-judge panel's 2.18 effective votes, and judges 6–9 add about 0.22 (§4.4). The best single judge "matches or outperforms the full panel across all conditions".
  - Jury-on-Demand ([arXiv:2512.01786](https://arxiv.org/abs/2512.01786)) reports accuracy peaking around K = 5–8, then slipping.
  - For repeated samples of one Model, about 70% of the gain arrives by k = 3 ([arXiv:2604.13717](https://arxiv.org/abs/2604.13717)).
  - PoLL itself used three Models from three families. Against a single GPT-4 judge, κ with humans was 0.763 vs 0.627 (NQ) and 0.906 vs 0.841 (TQA), at "seven to eight times less expensive". It did not vary panel size (§5) ([arXiv:2404.18796](https://arxiv.org/html/2404.18796)).
- **Aggregation: plain majority, chosen on a trusted sample.**
  - Kohli: accuracy weighting closed under 1% of the gap, and Dawid–Skene was unstable. "No weighting scheme can extract a third independent perspective" (§5.4).
  - Hossain et al.'s selector picked plain majority in every external transfer. Their advice is to "validate aggregation choices on a trusted sample". That is the human-agreement sample #1484 already plans.
- **Position bias makes both orders worth their cost.**
  - Zheng et al. ([arXiv:2306.05685](https://arxiv.org/abs/2306.05685)): "only declare a win when an answer is preferred in both orders", else "call it a tie". Consistency was 65.0% for GPT-4 and 23.8% for Claude-v1 (Table 2).
  - Shi et al. ([arXiv:2406.07791](https://arxiv.org/abs/2406.07791)): on DevBench, position consistency ranged from 0.23 (Claude-3-Haiku) to 0.89 (Llama-3.3-70B). Bias grows when answers are close: "of similar quality are difficult to judge". That describes the near-tie pairs an A/B Experiment produces most.
  - **[interpretation]** A Grader with a high flip rate mostly abstains, so report each seat's flip rate beside its human agreement.

## 4. Cost per grading session

There is no measured pairwise Grader session yet. **[interpretation]** Assume an agentic session that reads two diffs, the Ticket and the rubric over about 20 turns: about 400k input tokens, 80% of them cache hits, and 15k output tokens including reasoning. Cache writes are ignored. Measure one pilot pair before budgeting, since thirdshift already records Grader usage.

| Model | Price in / cached / out per Mtok | Source | ≈ $/session | ×120 sessions |
|---|---|---|---|---|
| Fable 5.2 | 8 / 0.20 / 40 | [Anthropic](https://platform.claude.com/docs/en/about-claude/pricing) | 1.30 | 156 |
| Opus 5.5 | 4 / 0.20 / 20 | same | 0.68 | 82 |
| Sonnet 5.5 | 2 / 0.10 / 10 | same | 0.34 | 41 |
| GPT-6 Astra | 10 / 1.00 / 50 | [OpenAI](https://developers.openai.com/api/docs/pricing) | 1.87 | 224 |
| GPT-6.1 Sol | 2 / 0.10 / 10 | same | 0.34 | 41 |
| GPT-6.1 Terra | 1 / 0.05 / 5 | same | 0.17 | 20 |
| Gemini 4 Argon | 2 / 10 until 30 Nov; 4 / 20 from 1 Dec (cache price not read; full input assumed) | [Google](https://aistudio.google.com/docs/pricing) | 0.95–1.90 | 114–228 |
| Grok 4.7 (<200k prompt) | 2 / 0.50 / 6 | [xAI](https://docs.x.ai/docs/models) | 0.41 | 49 |
| Muse Wonder (API: `muse-wonder-1.0`) | 4 / 0.20 / 20 | [Meta](https://dev.meta.ai/resources/blog/build-with-muse-wonder) | 0.68 | 82 |
| Kimi K3.1 Pro, Fireworks standard | 2 / 0.20 / 10 | [Fireworks](https://docs.fireworks.ai/serverless/pricing) | 0.37 | 44 |
| DeepSeek V4.1 Flash 1017, Baseten | 0.30 / 0.007 / 1.20 | [models.dev entry](https://github.com/sst/models.dev/blob/dev/providers/baseten/models/deepseek-ai/DeepSeek-V4.1-Flash-1017.toml) citing [Baseten](https://www.baseten.co/library/deepseek-v41-flash-1017/) | 0.04 | 5 |

- **Totals.** 60 pairs × 2 orders × all 11 ≈ **$860–970 at API rates.** A five-family panel of Sonnet 5.5, Sol, Argon, Kimi and DeepSeek comes to about $250–360.
- **Billing.** Only the two OpenCode rows are necessarily per-token.
  - `claude` sessions use the Credentials' `claude.setup_token`, a subscription login. `codex` allows `chatgpt.com`, a ChatGPT login. Both are **subscription-billed**: they draw on quota, not dollars.
  - `agy`, `grok` and `muse` bill to whatever login the CLI holds.
  - Muse Wonder *Full* is "Muse Code subscription only" per Meta.
- **Long prompts.** A Grok prompt of 200k tokens or more doubles every token's price in that request.

## Options for #1484 Round 5

**Q5.a — Vote unit.**
- (a) **One vote per Model.** Simplest. The error evidence (§2.1) says same-family seats are nearly as independent as cross-family ones. But panel composition then decides weight, and two vendors with three seats each can outvote five families.
- (b) **One vote per family.** A family's Models first take their own majority of order-consistent calls; a split is that family's tie. Each seat's call is still reported. Robust to stacking, and Kohli shows it costs little information (1.93 vs 2.18 effective votes).
- (c) Per-Model votes with a family cap, for example at most two.
- *Leaning:* (b). It makes "add Opus and Sonnet to see how they differ" free of side effects on the Verdict, which is what Jacob's within-family wish needs.

**Q5.b — Panel size.**
- (a) **1 to 11 seats, but the panel decides only with at least three voting families, an odd number.** Smaller panels are reported, not counted.
- (b) **At least two voting families**, as Q4.2 agreed. With two, majority means unanimity and ties are frequent.
- (c) **A cap on voting families at five,** with extra seats reported only.
- *Leaning:* (a), with no hard maximum. Cost is linear (§4) and past five the information gain is small (§3), so say so in the Bar rather than forbid it.

**Q5.c — Same-family-as-Arm rule.**
- (a) **Keep Q4.2 (a):** a seat whose family matches either Arm's Model is reported and doesn't vote. Under (b) of Q5.a that removes one family vote, not three seats.
- (b) Drop it when both Arms share a family. The self-preference then cancels between the Arms, though only if both Arms are equally "own".
- (c) Also exclude seats whose family is related by distillation (Preference Leakage), which is hard to know for Kimi and DeepSeek.
- *Leaning:* (a). Today's code compares exact Model strings, so a `family` field per seat is new configuration, not inferred.

**Q5.d — Harnesses for Kimi and DeepSeek.**
- (a) **OpenCode for both,** with the proxy hosts and per-provider keys of §1.3.
- (b) **New Harnesses** (§1.4).
  - Kimi CLI is headless-capable and vendor-maintained, but undocumented on usage output and its CLI surface is still moving.
  - DeepSeek's Harness has no headless mode yet.
  - Each costs a roughly 1,000-line adapter, plus a sandbox home, Setup and docs, for one Grader seat whose short packet-reading session gains little from a vendor's tool loop.
- (c) Route both through one aggregator, such as OpenRouter: one key and one host, but not the serving providers Jacob named.
- *Leaning:* (a).
