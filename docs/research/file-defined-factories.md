# How agent-factory tools define a factory process in files

Research date: 2026-10-26. For [#877](https://github.com/JacobStephens2/thirdshift/issues/877), with [#108](https://github.com/JacobStephens2/thirdshift/issues/108) and [#118](https://github.com/JacobStephens2/thirdshift/issues/118) in view.

**Question.** When a tool lets you define an agent pipeline in files, what does each file actually declare, and what stays fixed in the engine? Specifically: sequence, the per-step prompt or skill, gates between steps, triggers, caps and retries, and the model or harness per step. Plus how the files are validated and versioned, and how a user overrides a default.

**Method.**

- Primary sources only: vendor docs, source at pinned commit SHAs, and published JSON Schemas. Repository metadata comes from `gh api`, run on 2026-10-26. Warp's live schema registry was fetched the same day.
- **[unverified]** marks a claim I could not confirm from a primary source. **[3rd-party]** marks anything from a secondary write-up. **[interpretation]** marks my reading rather than a quoted statement.
- Facts about thirdshift cite files in this repository.
- Nothing was signed up for or run.

**Scope note.** This note is about *file format*. It deliberately does not re-cover what the existing notes already settle: whether Warp could replace thirdshift, its auth and pricing, and the self-hosting story, are in [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md) §3; the practice-by-practice comparison of six open-source factories, Anneal and Archon among them, is in [open-source-factories.md](open-source-factories.md). Read those first for context; this note goes one level down into the schemas.

## TL;DR

- **The whole field splits on one question: is the step sequence declared, or is it prose in an orchestrator's prompt?** Most tools, Warp Factories included until recently, put the sequence in prose. The foreman's `agent.md` body says "dispatch triage, then spec, then implement", and the engine just runs the foreman.
- **Warp now has both.** A `factory.yaml` may declare a `blueprint`: a named, nested stage graph with `worker` (which agent runs the stage), `prompt`, `inputs`/`outputs`, and `to` edges. "When present, the foreman executes this graph instead of the workflow in its own prompt, and every Agent's `spawnableBy` is derived from it" ([live v1alpha1 schema](https://app.warp.dev/api/v1/factory-files/schemas/v1alpha1), `factory.schema.json`). It is "Not yet available on every deployment" and is not in the docs page or the examples repo yet.
- **Even in the declarative form, the conditions are prose.** A blueprint edge is `{stage, when}` where `when` is "Prose condition the scope's agent evaluates to decide whether to take this edge". The graph is structural; the branching is still a model judgement.
- **Archon is the opposite end, and the reference implementation to read.** A workflow is a DAG of nodes with no `type:` field (the kind is inferred from which selector key is present) and no `next:` (flow is `depends_on` + a four-value `trigger_rule` + a `when:` mini-grammar over typed `output_format` outputs). Its design rule is one line in [its constitution](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/.archon/workflow-language-constitution.md): **"YAML coordinates. Code computes. Agents judge."** That document also states the risk #877 is taking on, better than I could: "Every workflow engine's configuration format faces the same gravitational pull: it grows until it becomes a bad programming language… individually reasonable feature grants, compounding into an informally-specified, untestable, half-language that is worse than code at computing and worse than configuration at declaring."
- **gh-aw is a third model nobody frames as one.** It declares no sequence at all — one agent session per file. What the file declares is a *capability envelope*: read-only `permissions`, an explicit `network.allowed`, caps, and `safe-outputs` naming every GitHub write the run may produce, each with its own policy (`allowed-files`, `protected-files`, `draft: true` by default, `max`). The agent picks its own path inside that envelope. Cheapest safety for the least language.
- **Caps split the field sharply.** Warp's whole v1alpha1 schema contains no retry, iteration or attempt limit — only `idleTimeoutMinutes`. Archon makes `max_iterations` **required** on every loop. gh-aw ships `max-turns`, `max-tool-calls`, `max-tool-denials`, `user-rate-limit`, `cooldown` and a per-run cost cap that is **on by default at 1000 credits** and must be disabled with `-1`. For an unattended factory, that default is the right way round.
- **Gates are the weakest point everywhere except Anneal.** Warp's human gates are a `## Human gates` prose section in the foreman's prompt. Anneal is the exception: a step declares `approvalGate: true` in frontmatter, and a validator refuses an approval gate inside a parallel layer. Archon's gate is a `wait:` node whose choices a preceding agent authors, and its pack states the doctrine "There is no in-run approval gate — the human gate is PR review and merge on the forge, outside the run."
- **Nobody but gh-aw lets a file cause a merge**, and it does so only behind `allowed-branches`, `allowed-labels`, `required-labels` and `required-title-prefix`. Warp refuses outright; Archon refuses by doctrine. That is the one place where thirdshift's Self-merge has no clean precedent to copy.
- **Archon's workflows are testable without running an agent** — `--dry-run --stubs`, and `archon workflow test` replaying `fixtures/*.stubs.yaml` against declared outcomes. Of everything here, that is the single most adoptable idea.
- **Anneal is the closest existing model to what #877 asks for**, and it is MIT and subscription-billed. A workflow is a directory of numbered Markdown files, one per step, each with structural frontmatter (`stepIndex`, `layer`, `agent`, `approvalGate`, `optional`, `outputKind`, `priorOutputKinds`, `baseFromStepIndex`, `spawnPolicy`) and the step prompt as the body. Fifteen named validation rules check the graph, and [ADR-0008](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/docs/adr/0008-templates-are-immutable-once-used.md) makes a template immutable once used.
- **The per-step agent is always a separate file.** Warp `agents/<name>/agent.md`, Anneal `agents/roles/<name>.md`, Claude Code `.claude/agents/<name>.md`, Factory `.factory/droids/<name>.md`, OpenCode `.opencode/agents/<name>.md`. All five are Markdown-with-frontmatter, all five put the model (and often the effort/reasoning level) in frontmatter and the prompt in the body. That convergence is strong enough to treat as a standard.
- **Validation is server-side or compiled, never trusted to a local copy.** Warp's own validator script refuses to parse YAML at all and posts the bytes to the server, on the stated grounds that "a stale copy does not degrade gracefully: it reports confident, wrong diagnostics that invite an agent to 'repair' valid configuration by deleting it". gh-aw bundles its schema and linters *with* the compiler so they cannot drift. Archon validates in three levels, and warns-and-drops unknown keys where Warp and gh-aw reject them.
- **Versioning is handled four different ways**, none of them a plain `version:` field in the file: Warp has one `schemaVersion` plus a registry endpoint; Archon has no version field at all and relies on `deprecated:` plus filename shadowing; gh-aw hashes the frontmatter into the lock file and refuses stale runs; Anneal makes a template immutable once used and requires a registered generation rollover for a prompt edit.
- **Overriding a default converges on filename shadowing plus one documented merge rule.** Archon's `bundled < global < project` by filename, with sidecar policy Markdown whose "lines add to the default and win where the two conflict", is the most thirdshift-shaped of these — it answers #118 without forking twenty prompts.
- **For thirdshift**, the live question is not "YAML or not" but *which of the five fixed Commands' behaviours become declarable*. §6 lays out four options — externalise prompts and skills only (#118), a named process over a fixed step catalogue, everything in the file, and a fourth the grill does not list: an envelope with no sequence — with what each costs in ADR terms. §5.3 names the three models the field actually uses, since #877 has to pick one or a blend.

## 1. Warp Factories

### 1.1 The tree

A factory definition is "a tree of YAML and Markdown files rooted at `factory.yaml`", and "Each path identifies a resource, so `agents/reviewer/agent.md` defines an agent named `reviewer`" ([Definitions as code](https://docs.warp.dev/factories/factory-as-code/)).

```
factory.yaml                       # root (required)
agents/<name>/agent.md             # one per agent; at least one required
agents/<name>/skills/<skill>/SKILL.md
skills/<skill>/SKILL.md            # shared skills
automations/<name>/automation.md
runners/<name>.yaml
routers/<name>.yaml
benchmarks/<suite-slug>/suite.yaml
benchmarks/<suite-slug>/tasks/<task-slug>.yaml
scorers/<name>/scorer.md
webhooks/<name>.yaml
self-improvement/agent.md
```

The layout rules are stated plainly in the examples repo: "A resource's name is its path… Renaming a resource means moving the file"; "Exactly one agent per factory declares `agentType: FOREMAN`"; "Skills under `skills/` are available to every agent in the factory. Skills under `agents/<name>/skills/` are available only to that agent" ([README.md](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/README.md)).

The reason given for files is review and agent access: "Any changes to the pipeline get checked into version control and reviewed through pull requests", and "Both humans and agents should be able to modify any part of the agent pipeline" ([blog, 2026-09-30](https://www.warp.dev/blog/software-factory-as-code)).

`warpdotdev/warp-factory-examples` is MIT, last pushed 2026-08-25, with eight example trees under `examples/` and no `webhooks/`, `benchmarks/` or `routers/` example (`gh api repos/warpdotdev/warp-factory-examples`; tree at `84e7c952f1506c4bd2784d96fd40a6f4d6398cad`).

**Warp's own factory is not public.** `warpdotdev/warp` (AGPL-3.0, default branch `master`) has no `factory.yaml` at the root or under `.warp/`, `.factory/` or `warp-factory/`. Its `.warp/workflows/*.yaml` are terminal command workflows (`run_unit_test.yaml`, `cherrypick_into_release.yaml`), not factory definitions, and `agents/` holds only `specs/`. So the dogfooded factory noted in [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md) §3.1 is presumably Warp-managed rather than GitHub-backed **[interpretation]**.

### 1.2 What `factory.yaml` declares

The live schema's `factory.schema.json` requires exactly `schemaVersion`, `name`, `repositories` and `agentDefaults`. Full property set, with the docs table as the readable version ([Definitions as code](https://docs.warp.dev/factories/factory-as-code/), [live schema](https://app.warp.dev/api/v1/factory-files/schemas/v1alpha1)):

| Field | Declares |
|---|---|
| `schemaVersion` | `v1alpha1`. "This document describes v1alpha1 only; fetch the document for another version rather than validating against this one." |
| `name`, `description`, `alias` | Identity. `alias` is the @-mention handle for the foreman. |
| `repositories` | `owner`/`name`, optionally per-entry `codeForge` |
| `codeForges` | `GITHUB`, `GITLAB`, `AZURE_DEVOPS` |
| `integrations` | `slack`, `microsoft-teams`, `linear`, `jira`; at most one issue tracker |
| `agentDefaults` | **Required.** Execution defaults every agent inherits; "Exactly one of `model` or `harness` is required" |
| `secrets`, `mcpServers` | Factory-wide; MCP entries take exactly one of `warpId`, `command` or `url`, with `{{SECRET_NAME}}` interpolation |
| `cloudProviders` | GCP workload identity federation, AWS `roleArn` |
| `scorerDefaults` | `runner`, `secrets`, `mcpServers` for scorers |
| `selfImprovement` | `failedRunThreshold` (1–50), `reviewerType` (`none`/`admins`/`team`/`custom`), `reviewerEmails` |
| `blueprint` | The declared stage graph — see §1.5 |
| `networkPolicy` | `allowedHosts`, `deniedHosts`, `extends` (e.g. `warp/claude-code@^1`). "An empty policy denies all network egress. If no policy applies to the agent, all egress is allowed. Network policies cannot block services required for the platform to function." |
| `github.factoryConfigChecks` | `main` (default) or `all`: which PR base branches get validated |
| `restrictFollowupsToStarter` | Only the starting user may send follow-ups |
| `experimental` | "Internal per-Factory experiments… unknown keys are preserved" |
| `benchmarkRepoSubstitutions` | Maps a source repo to a target repo for benchmark starting states |

A real one is short ([`02-sdlc-issue-to-pr/factory.yaml`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/02-sdlc-issue-to-pr/factory.yaml)):

```yaml
schemaVersion: v1alpha1
name: sdlc-issue-to-pr
alias: Acme
credentialStrategy: EXECUTOR
repositories:
  - owner: acme
    name: api-service
  - owner: acme
    name: webapp
integrations:
  - type: slack
  - type: linear
agentDefaults:
  model: auto
  runner: linux-standard
```

Note what is *not* here: no sequence, no gates, no stage list, no caps.

### 1.3 What `agent.md` declares

"YAML frontmatter of an Agent file. The Agent's name comes from its directory, never from a field. The Markdown body after the closing fence is the Agent's prompt" (schema `agent.schema.json`). All frontmatter keys are optional.

| Field | Declares |
|---|---|
| `description` | What the agent does |
| `agentType` | `CUSTOM` (default), `FOREMAN` (alias `MAIN`), `TRIAGE`, `SPEC`, `IMPLEMENT`, `REVIEW`, `VERIFY`. "Exactly one Agent in the tree must declare MAIN or FOREMAN." |
| `spawnableBy` | "Factory Agents allowed to spawn this Agent… Omit to allow only the Factory foreman. An empty list allows no Factory Agent… **Not valid when factory.yaml declares a blueprint, which derives every Agent's allowlist itself.**" |
| `model` *xor* `harness` | `model` is "Shorthand for `harness: {type: oz, model: <id>}`". `harness.type` is one of `claude`, `claude-code`, `codex`, `gemini`, `oz`, `pi`; plus `model`, `reasoningLevel` (rejected for `oz`) and `auth` (`managedSecret` + `secretName`, or `workerEnvironment`) |
| `runner`, `environmentId`, `workerHost` | Where it executes |
| `secrets`, `mcpServers` | Tool and credential access |
| `computerUseModel` | Computer-use model on the Oz harness |
| `idleTimeoutMinutes` | 1–60. "Defaults to 10 minutes, or 60 for orchestrated child runs." |
| `networkPolicy`, `prewarmedPool`, `required_repos`, `credentialStrategy` | Execution environment |

`agentType` is the *only* sequencing-adjacent field, and it names a role, not a position. The per-role model is a one-line frontmatter change — [`03-multi-harness`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/03-multi-harness/agents/reviewer/agent.md) puts the reviewer on Codex while the implementer is on Claude Code, for the stated reason that "You review on a different harness and model family than the one that wrote the change; this reduces correlated blind spots":

```yaml
---
description: Reviews the implementer's PRs on the Codex harness.
agentType: REVIEW
harness:
  type: codex
  model: gpt-5.3-codex
  reasoningLevel: high
  auth:
    source: managedSecret
    secretName: OPENAI_API_KEY
---
```

**Skills attach by path, not by a field.** There is no `skills:` key in `agent.schema.json`. A skill is shared or per-agent purely by where its `SKILL.md` sits. Warp is explicit that "Skills provide instructions only, not tool or repository access", that custom skills "extend the built-in skills… rather than replacing them", and that "Custom agents have no built-in skills, so a custom skill is how they learn their job" ([Factory skills](https://docs.warp.dev/factories/factory-skills/)). A `SKILL.md` carries `name` and `description` in frontmatter ([example](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/02-sdlc-issue-to-pr/skills/repository-conventions/SKILL.md)); the frontmatter is otherwise documented only on the general [Skills](https://docs.warp.dev/agents/capabilities/skills/) page.

### 1.4 Sequence and gates live in the foreman's prose

This is the central finding about the shipped form. The docs say the foreman "picks the shortest path that still meets your quality policy", skips stages for well-defined work, and sends work back to an earlier agent for revisions; approval behaviour "is written into the foreman's instructions, so editing those instructions changes when the factory checks in" ([How factories work](https://docs.warp.dev/factories/how-factories-work/)).

The default foreman's prompt body carries a numbered `## Procedure` and a `## Human gates` section ([`00-warp-default-agents/agents/foreman/agent.md`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/00-warp-default-agents/agents/foreman/agent.md)):

> Most steps continue automatically; however, you must stop and wait for a human at these points:
> - Spec approval: when a spec is necessary, do not start implementation until the user approves the spec.
> - Clarifying questions: when you or a subagent needs an answer from the user, ask the question and pause.
> - PR hand-off: when the implement-review loop is complete, give the PR to the user and stop.

Its one cap is a sentence: "The review runs only once per work item: do not dispatch the review subagent again on the revised PR." Likewise the dependency logic in the SDLC example is prose in the foreman body — "a change that spans the backend and the app needs one PR per repository: mobile-build opens its own PR on `acme/ios-app`" ([`02-sdlc-issue-to-pr/agents/foreman/agent.md`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/02-sdlc-issue-to-pr/agents/foreman/agent.md)).

So in the shipped form: the *cast* is declarative (agents, models, runners, skills, triggers), the *script* is a prompt.

### 1.5 `blueprint`: the declared stage graph

The live v1alpha1 schema carries a `factory.yaml` key the docs page and examples repo do not yet mention:

> **`blueprint`** — "Explicit workflow for the Factory. A stage is a leaf with a prompt, or a nested scope with its own start and stages; the agent named by a scope's `worker` drives that scope, executing same-worker stages inline and dispatching other workers as child runs. When present, the foreman executes this graph instead of the workflow in its own prompt, and every Agent's `spawnableBy` is derived from it. **Not yet available on every deployment.**"

Required: `worker`, `start`, `stages`. `worker` "Must name the Agent that declares agentType FOREMAN".

`stages` is a map keyed by name, "in workflow order". Names must be blueprint-wide unique, must not contain `/`, and `start` and `end` are reserved. "The parser checks blueprint-wide uniqueness, that every stage is reachable from `start`, and that `end` is reachable."

A `blueprintStage` declares exactly one of `prompt` (a leaf) or `stages`+`start` (a nested scope):

| Field | Declares |
|---|---|
| `worker` (req.) | "Agent directory name of the agent that executes this stage. When it matches the enclosing scope's `worker` the stage runs inline in that agent's conversation; otherwise the scope's agent dispatches it as a child run. Containment must be acyclic." |
| `to` (req.) | Outgoing edges, "evaluated in order once the stage finishes. Targets are sibling stages in the same scope or `end`" |
| `prompt` | "The procedure of a leaf stage" |
| `inputs` | "Prose describing what the stage needs from the task so far. The scope's agent binds it from context when it runs the stage." |
| `outputs` | "Prose describing what the stage must report back. The scope's agent reads it from the stage's result and uses it to evaluate the stage's edges." |
| `stages` + `start` | A nested scope run by this stage's worker |
| `displayName` | Label only; "the stage's identity for references and scope paths still comes from its mapping key" |

An edge (`blueprintEdges`) is either a bare stage name — "A single unconditional edge to the named stage" — or a list of `{stage, when}`, where `when` is **"Prose condition the scope's agent evaluates to decide whether to take this edge"**.

Three things follow, and they matter for #877:

1. **The topology is checkable; the branching is not.** Reachability, acyclic containment and name uniqueness are parser-enforced. Which edge is taken is a model judgement over a prose condition. There is no expression language, no typed step output, no comparison operators.
2. **There is still no cap anywhere.** Searching the whole v1alpha1 schema for `max|retr|limit|timeout|iteration|threshold|cap` keys yields only `idleTimeoutMinutes`, a `common.schema.json` `max` (a matcher bound), and `selfImprovement.failedRunThreshold`. A blueprint can declare a review→implement loop edge and nothing that stops it.
3. **There is no gate primitive.** Nothing in `blueprintStage` says "pause for a human". A gate remains a sentence in a stage's `prompt` or the foreman's.

An automation can target a blueprint stage directly instead of an agent: `stage` is "Name of the blueprint stage that receives the event. Requires a blueprint; not valid with `agent` or any per-run override" (`automation.schema.json`). That is how an external event re-enters a declared graph partway through.

**Treat blueprint as directional evidence, not a shipped feature.** It is in the live schema registry, flagged as not universally deployed, and absent from both the docs page and the examples. **[unverified]** whether any user-facing documentation or example exists yet.

### 1.6 Triggers: `automation.md`

`automations/<name>/automation.md` has frontmatter and "the Markdown body is the starting prompt". Only `triggers` is required ([Definitions as code](https://docs.warp.dev/factories/factory-as-code/), `automation.schema.json`).

| Field | Declares |
|---|---|
| `triggers` (req.) | List of `{provider, event, filter?, schedule?}` |
| `enabled` | Default `true` |
| `agent` *xor* `stage` | Which agent handles runs (default: the foreman), or which blueprint stage receives the event |
| `model`/`harness`, `runner`, `environmentId`, `secrets`, `mcpServers`, `workerHost`, `computerUseModel`, `idleTimeoutMinutes` | Per-run overrides of the target agent's settings |
| `displayName`, `required_repos` | Label; repos eagerly prepared for every run |

`provider` is an enum: `azure_devops`, `factory`, `github`, `gitlab`, `jira`, `linear`, `schedule`, `slack`, `teams`, `webhook`. `event` values are provider-conditional in the schema. The GitHub set is the broad one — `issue_assigned`, `issue_commented`, `issue_created`, `issue_labeled`, `issue_mentioned`, the twelve `pull_request_*` events, `pull_request_review_requested`/`_submitted`, `check_run_rerequested`, `check_suite_completed`, `check_suite_rerequested`, `commit_status`, `push`, `workflow_run_completed`. `factory` has exactly one event, `work_item_stage_changed`, which is how a factory reacts to its own lifecycle.

`filter` is a matcher language, not free text: "Filter fields combine with AND. An absent field is a wildcard. The accepted keys depend on the `(provider, event)` pair." A field takes a bare array as sugar for `{in: [...]}`, and most support `not_in`. Webhook deliveries get a structural `payloadPattern` over the body with `in`/`not_in`/`exists` and arbitrary nesting. There is a `botPolicy` enum (`all`/`self`/`none`) for skipping bot-authored events.

Schedules are inline on a `schedule`/`cron_fired` trigger: a required `cron` ("Standard five-field cron expression or a descriptor (`@daily`, `@hourly`, `@every 1h`). Always interpreted in UTC: a `CRON_TZ=` or `TZ=` prefix and the six-field seconds form are both rejected") and an optional `name` whose identity semantics are spelled out — "Changing `name` replaces the schedule; changing only `cron` updates it in place."

Two real ones ([`06-common-automations`](https://github.com/warpdotdev/warp-factory-examples/tree/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/06-common-automations/automations)):

```yaml
---
triggers:
  - provider: github
    event: workflow_run_completed
    filter:
      repos: [acme/api-service]
      branches: [main]
      conclusions: [failure]
---
A workflow run failed on the default branch. Read the run's logs and find
the failing step. If the cause is small and clear, open a fix PR. …
```

```yaml
---
triggers:
  - provider: schedule
    event: cron_fired
    schedule:
      name: weekly-dependency-audit
      cron: "0 9 * * 1"
---
Run the weekly dependency audit. …
```

The second is the shape of thirdshift's Passes: cron in a file, prompt in the body. The first is a CI-failure trigger thirdshift does not have as a trigger at all — its Repair loop is internal to a Ticket run ([README.md](../../README.md) step 6).

### 1.7 The supporting file types

- **`runners/<name>.yaml`** — compute. `description`, `setupCommands`, `failureSessionRetentionMinutes`, `instanceShape` (`vcpus`, `memoryGb`), `platform` (`os`, `arch`, OS-specific settings such as `linux.dockerImage` or `mac.version`). The macOS example is honest about why a runner is a per-stage choice: "Xcode builds cannot run on Linux, so this stage needs a macOS sandbox: a VM rather than a container, which is why it declares no `dockerImage`" ([`macos-build.yaml`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/02-sdlc-issue-to-pr/runners/macos-build.yaml)).
- **`routers/<name>.yaml`** — model routing. Required `type` (`complexity` or `prompt`) and `default` model, plus `routing` rules. Referenced as `router:<slug>` where a model ID would go. No example in the repo.
- **`scorers/<name>/scorer.md`** — an LLM-judge rubric. Required `name`, `agents`, `labels`, `model`, `passingScore`; plus `samplingRate` (default 25) and `selfImprovement`. Each label has a `value`, `description` and `score` 0–1. Body is the rubric ([`review-quality/scorer.md`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/02-sdlc-issue-to-pr/scorers/review-quality/scorer.md)).
- **`benchmarks/<suite>/suite.yaml`** and `tasks/<slug>.yaml` — `name`, `agent`, `configurations`, ordered `tasks`; a task declares `title`, `prompt`, required `successCriteria`, and optional `startingRepoRefs` (40-character SHAs), `tags`, `sourceRunId`, `linearSeed`.
- **`webhooks/<name>.yaml`** — `authMode` (`token` default, `url_token`, `signature`), required `secretName`, `signatureScheme`, `deliveryIdHeader`, `enabled`. "Auth mode and signature scheme can't be changed after creation."
- **`self-improvement/agent.md`** — "Execution resources for the platform-managed Oz self-improvement agent. The literal Markdown body is optional guidance, limited to 16,384 UTF-8 bytes. **Loop policy stays in `factory.yaml.selfImprovement`.**" A clean split: the file tunes the agent, the engine owns the loop.

### 1.8 What the engine keeps fixed

From [How factories work](https://docs.warp.dev/factories/how-factories-work/) and the schema:

- **Work-item identity and lifecycle.** A work item "keeps its identity from intake to handoff". Handoff is not completion: a task completes on a merge notification or confirmation, or is cancelled. The default foreman has to call `complete_task` itself, and "a task that you never complete stays open forever".
- **Merging.** "The factory never merges on your behalf" ([Factories](https://docs.warp.dev/factories/)); human-only merge is enforced by branch protection, not by the definition. This is the hard conflict with thirdshift's Self-merge ([ADR-0004](../adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)).
- **One foreman.** Exactly one agent declares `FOREMAN`/`MAIN`; it is the entry point and default automation target.
- **The built-in skills.** Role and integration skills, plus `oz-platform`, `factory-files`, `factory-mcp`, are extended, not replaced.
- **No caps.** No declarable retry, attempt or iteration limit (§1.5).
- **Network floor.** "Network policies cannot block services required for the platform to function."
- **Unknown keys are rejected.** `additionalProperties: false` throughout; "The schemas reject unknown keys."

### 1.9 Validation and versioning

- **`schemaVersion: v1alpha1`** is the only version knob, and the registry at `https://app.warp.dev/api/v1/factory-files/schemas` reports `{"current_version":"v1alpha1","versions":[…]}` (fetched 2026-10-26). Per-document schemas live at `/schemas/v1alpha1` as a bundle of eleven JSON Schema documents: `factory`, `agent`, `automation`, `runner`, `router`, `scorer`, `webhook`, `benchmark_suite`, `benchmark_suite_task`, `self_improvement_agent`, `common`.
- **The schemas are generated from the parser**, not hand-written: "Generated from the Factory file parser and the catalogues its apply path validates against. Edit `logic/factoryfile/schema`, then regenerate; a hand edit is reverted by the drift test."
- **Validation is server-side, on purpose.** [`validate_factory_files.py`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/scripts/validate_factory_files.py) collects resource files by path and posts their bytes to `/api/v1/factory-files/validate`. Its docstring is worth reading in full before thirdshift builds a validator:

  > "There is deliberately no local fallback. A bundled copy of the format is routinely older than the server it is used against, and a stale copy does not degrade gracefully: it reports confident, wrong diagnostics that invite an agent to 'repair' valid configuration by deleting it. Reporting that a tree could not be checked is strictly safer than reporting the wrong answer…"
  >
  > "That also means this script never parses YAML. It decides which files are resource files from their paths alone and sends their bytes verbatim, so it cannot disagree with the parser about what a document means."

  Exit codes separate the three outcomes: `0` validated clean, `1` validated with diagnostics, `2` **not validated** — and the CI job treats `2` as "This is not a pass" after one retry ([`validate.yml`](https://github.com/warpdotdev/warp-factory-examples/blob/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/.github/workflows/validate.yml)). That workflow also runs a weekly cron that fails if the server's `current_version` has moved past the pinned `v1alpha1`.
- **Two layers of checking.** Static parse plus "state-independent checks the apply path would run next". Explicitly *not* checked statically: "Model IDs, environment IDs, secret names, runner references, MCP server IDs, integration availability, and the values of provider name aliases are all checked when the plan is applied."
- **Caps on the tree.** 256 files, 256 KiB per file, 2 MiB total content. Symlinks are refused outright: "The server parses the repository tree, so it sees the link itself rather than its target and cannot accept this either."
- **Atomic apply.** "A definition is applied as a single unit. If validation fails, the last valid definition stays in effect."
- **Two edit modes.** *Warp-managed* (default): edit in the web app; Warp "validates, commits, and applies changes, and the repository is hidden". *GitHub-backed* (paid): the definition lives in your repo, the web app is read-only, and merges to the production branch update the factory after the `warp/factory-config` check passes. `github.factoryConfigChecks` chooses whether that check runs on PRs to all base branches or only to the production branch.
- **State is not in the definition.** "Work items, runs, and metrics are stored in the web app, not in the definition."

### 1.10 How a user overrides a default

Four distinct mechanisms, worth keeping apart:

1. **Inheritance.** Agents inherit `agentDefaults`; agent-level values override. Automations override their target agent's execution settings for their own runs. Scorers override `scorerDefaults`.
2. **Replace, not merge, for collections.** "Declaring `secrets` or `mcpServers` on an agent or automation replaces the inherited list; it does not merge." Factory-wide `secrets`/`mcpServers` still apply in addition.
3. **Sparse override for `harness`.** Agent-level `harness` is a sparse override of `agentDefaults.harness`; `auth: null` clears inherited authentication. `null` generally means "inherit", and an explicit `null` on `computerUseModel` or `idleTimeoutMinutes` inherits rather than clears.
4. **Materialise the defaults as files.** [`00-warp-default-agents`](https://github.com/warpdotdev/warp-factory-examples/tree/84e7c952f1506c4bd2784d96fd40a6f4d6398cad/examples/00-warp-default-agents) reproduces the five seeded agents "with their default prompts, descriptions, and models… verbatim as files", so overriding the stock foreman means copying it and editing. The stock models as of 2026-08-20 were `foreman: claude-5-opus-high`, `triage: grok-4-5-high`, `spec: gpt-5-6-sol-high`, `implement: auto-genius`, `review: gpt-5-6-terra-high` — i.e. a different family per role by default.

`model` and `harness` are "mutually exclusive at every level", which is the one inheritance rule that bites: you cannot set `agentDefaults.model` and then override one agent with a `harness` sub-field; you replace the whole choice.

## 2. Archon

`coleam00/Archon` is MIT, 23,770 stars, and now describes itself as "The first open-source harness builder for AI coding. Make AI coding deterministic and repeatable." **Its default branch is `dev`, not `main`**, and `main` lags (at the time of checking, `main` was a 2026-10-17 Homebrew-formula commit). Pinned commit for everything below: `ef04684469fe6d20a3d94bb2bda4407edbddc490` (`dev` HEAD, 2026-10-26). The engine is TypeScript/Bun, not Python.

**Two corrections to the premise of this research, and to a path cited elsewhere in these notes.** There is no `.archon/workflows/sdlc/loop/archon-loop.yaml` at this SHA — nothing in the tree matches `archon-loop`, and loops are now a *node type* (`loop:`/`loop_group:`) rather than a workflow file. The `archon-loop.yaml` and `sdlc/loop/commands/*.md` paths cited in [open-source-factories.md](open-source-factories.md) §2.4 were pinned to the older `99be58cd…`, so they remain valid for that commit but no longer describe the current pack. `.archon/workflows/sdlc/deliver/` does exist.

Of everything surveyed here, Archon's format is the most expressive, and it is the one whose design rules are written down most explicitly. If #877 ends up building a workflow language, this is the reference implementation to read.

### 2.1 Layout: a pack of packaged workflows

Only one pack ships. [`bundle-index.json`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/defaults/bundle-index.json) is the whole file:

```json
{
  "$comment": "Only listed packs ship. sdlc is the maintained reference pack. Client, maintainer, experimental and test workflows are project content.",
  "packs": ["sdlc"]
}
```

`.archon/workflows/sdlc/` is one directory per stage, each a **packaged workflow** holding exactly one YAML plus optional `commands/` (Markdown prompts), `scripts/` (bun/python), and `fixtures/` (`*.stubs.yaml`). The stages are `attention`, `build-queue`, `cleanup`, `deliver`, `deliver-entry`, `discoveries`, `forge-check`, `implement`, `intake`, `investigate`, `merge-queue`, `plan`(+`-entry`), `pr`, `review`(+`-entry`), `review-eval`, `ship`(+`-entry`), `simplify`, `triage`(+`-entry`), `upkeep`, `validate`. Shared code lives in `sdlc/.shared/` (28 TypeScript modules plus `policy/*.md`), with a `sdlc/CONSTITUTION.md` and a 917-line `sdlc/README.md`.

The package boundary is hard ([workflow-language.md](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/docs-web/src/content/docs/reference/workflow-language.md)):

> "The two directories form a fixed package boundary: `.archon/workflows/<pack>/<workflow>/`. A packaged workflow contains exactly one YAML definition; bare `command:` and named `script:` references resolve only from its own `commands/` and `scripts/` directories, with no shared or cross-scope fallback."

So there is no `.claude/commands`-style global prompt pool: a prompt belongs to the workflow that names it. (A legacy flat layout still resolves `repo > ~/.archon/commands/ > bundled`.)

**The `-entry`/`-block` split is the pack's central pattern.** An `*-block.yaml` is the pure graph; the `*-entry` wrapper wraps it in a `loop_group` together with the shared `archon-attention` pause block. From `archon-deliver-block.yaml`'s own description: "The graph of archon-deliver, as a building block for a workflow that composes the stage and owns what follows its ending. **It never pauses for a person**: run archon-deliver, which adds that pause, by hand." That is a clean separation of *what the stage does* from *when a human is asked* — directly relevant to #877's question of where gates live.

### 2.2 Top-level workflow keys

From `workflowBaseSchema` in [`schemas/workflow.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/workflow.ts) (Zod, not JSON Schema):

```ts
name, description,                                  // required
provider, model, modelReasoningEffort, webSearchMode,
effort, fallbackModel, betas,                       // model/harness defaults
sandbox, worktree, container,                       // execution
evidence_policy, budget, mutates_checkout,
persist_sessions, tags, requires,
inputs, returns, outcome_field, deprecated,
nodes                                               // the graph
```

- `inputs` is a map of `{required?, default?, description}`. `required: true` and `default` are mutually exclusive — "the loader drops any key that declares both" — and `description` is "human documentation only, unused by the engine".
- `returns` names "The node id whose output IS this workflow's result".
- `outcome_field` is a "Required boolean property on the declared `returns:` node that records the workflow author's verdict independently from the run lifecycle", and "the engine maps exact true/false values to succeeded/failed without inference". **This is the one idea here with no counterpart in Warp**: a workflow declares its own success predicate, separately from whether the process exited 0.
- `budget` is `.strict()` with `max_work_units` and/or `max_spend_usd`, and is mutually exclusive with dynamism: "Loops, `workflow:` children and fan-out are rejected at load time while a budget is declared."

### 2.3 Node schema: no `type:`, no `next:`

**There is no `type:` field.** A node's kind is inferred from which selector key it carries. `NODE_MODE_SHAPES` in [`schemas/dag-node.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/dag-node.ts) is "the single source for which mode a key belongs to", with eleven selectors: `command`, `prompt`, `bash`, `loop`, `loop_group`, `approval`, `wait`, `cancel`, `script`, `include`, `workflow`. They collapse to nine internal kinds (`AgentNode | ExecNode | GateNode | HaltNode | WaitNode | LoopNode | LoopGroupNode | WorkflowNode | ComposeFanOutNode`) and are mutually exclusive — declaring two is the load error "`… are mutually exclusive`".

**There is no `next:`, `on_success:` or `on_failure:`.** Flow is `depends_on` + `trigger_rule` + `when`, Airflow-style: "A node without `depends_on` starts in the first layer. A node starts when every node it depends on has settled and its `trigger_rule` and `when:` allow it. Nodes in the same layer run concurrently."

`trigger_rule` is a closed four-value enum, default `all_success`:

```ts
z.enum(['all_success', 'one_success', 'none_failed_min_one_success', 'all_done'])
```

Common node fields, from [`schemas/dag-node-base.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/dag-node-base.ts): `id`, `description`, `depends_on`, `when`, `trigger_rule`, `model`, `provider`, `context`, `output_format`, `allowed_tools`, `denied_tools`, `idle_timeout`, `retry`, `hooks`, `mcp`, `skills`, `plugins`, `agents`, `pi`, `effort`, `maxBudgetUsd`, `systemPrompt`, `fallbackModel`, `settingSources`, `betas`, `sandbox`, `always_run`, `mutates_checkout`, `persist_session`, `output_type`.

Note what that list gives per step that no other tool here does: `skills`, `allowed_tools`/`denied_tools`, `maxBudgetUsd`, `mutates_checkout`, `output_format`, and `settingSources`.

### 2.4 Conditions: a real but deliberately small grammar

`when:` is a string in a hardcoded mini-grammar, specified in the module docstring of [`condition-evaluator.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/condition-evaluator.ts):

```
 *   String equality:  "$nodeId.output == 'VALUE'"  / "$nodeId.output != 'VALUE'"
 *   Dot notation:     "$nodeId.output.field == 'VALUE'"
 *   Shorthand path:   "$nodeId.field == 'VALUE'"
 *   Named inputs:     "$INPUTS.mode == 'fast'"
 *   Numeric ops:      "$nodeId.output > '80'"  / ">=" / "<" / "<="
 *   Unquoted RHS:     "$nodeId.exit_code == 0"  / "$nodeId.passed == true"
 *   Compound AND/OR:  AND has higher precedence than OR. No parentheses.
```

No functions, no arithmetic, no parentheses. The two failure modes are chosen deliberately: "A malformed/unparseable EXPRESSION (bad syntax) is fail-closed → result false (skip the node)… An unresolvable REFERENCE THROWS, propagating to FAIL the node." A `when:` that resolves to an object or array throws, with the guidance to "emit a scalar routing field or inspect structured data in a script node". The atom grammar lives in `when-atom.ts`, shared with load-time validation "so the two can never disagree about what an atom means".

This is the crucial contrast with Warp's blueprint: **Archon's edge conditions are machine-evaluated over typed node outputs; Warp's are prose the agent judges.** Archon gets that by requiring nodes to declare `output_format` (a JSON Schema) so there is something typed to compare.

### 2.5 Loops and caps — the strongest cap story here

Two node types, sharing `loopControlSchema` in [`schemas/loop.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/loop.ts):

```ts
until, max_iterations, fresh_context, until_bash, interactive, gate_message, signal_completes
```

- **`max_iterations` is required**, not optional: every loop declares a cap, and "exceeding this fails the node".
- At least one completion channel is required (`until` or `until_bash`; `loop:` adds `until_field`, "the name of a property in this node's `output_format` schema whose validated value `true` ends the loop", which `loop_group:` does not have).
- `loop_group:` carries `nodes:` — a recursive sub-DAG, so "a `loop_group` body may contain any node type — including another `loop_group`".
- `loop_group:` accepts no `retry:` — "the group manages its own iteration".

Retries are their own schema, [`schemas/retry.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/retry.ts), with bounds in the schema rather than in prose: `max_attempts` 1–5, `delay_ms` 1000–60000 (doubling each attempt), `on_error: 'transient' | 'all'`; loops may only retry `transient`. Per the docs, AI nodes auto-retry transient errors with 2 retries and a 3 s base delay, while "**Deterministic nodes** (`bash:`, `script:`) do **not** auto-retry". Exec nodes take `timeout` (ms) plus `on_timeout: skip`; AI nodes take `idle_timeout`.

The real pack uses this exactly as thirdshift's Repair budget does. From `archon-deliver-block.yaml`:

```yaml
  - id: rounds
    depends_on: [forge]
    loop_group:
      until_bash: |
        done=$round.output.done
        test "$done" = "true"
      # round caps itself one below this, so the cap ends typed instead of failing.
      max_iterations: 5
      fresh_context: true
```

The comment is the decision-relevant part: the engine cap *fails* the node, so the pack keeps a softer self-cap one below it "so the cap ends typed instead of failing". thirdshift's "at most 5 Repairs per Run" is the same number and the same shape, in Rust.

Its round body is a chain of includes: `implement` → `pr` → `review` → `simplify` → `reshape` → `rereview` → `settle-threads` → `ci` → `threads` → `round`, with `when:` guards. One guard shows the grammar's practical reach:

```yaml
          # `&&` short-circuits, so a skipped node's output is never read.
          when: "$review.output.ready == true && $simplify.output.findings == false || $review.output.ready == true && $simplify.output.findings == true && $rereview.output.ready == true"
```

That expression is also the honest argument *against* this much expressiveness: it is already hard to read, and it has no parentheses to help.

### 2.6 Per-step prompt, model and harness

[`schemas/agent-node.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/agent-node.ts) makes the prompt source a discriminated union:

```ts
export const promptSourceSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('inline'), prompt: z.string() }),
  z.object({ kind: z.literal('command'), name: z.string(), with: nodeBindingsSchema.optional() }),
]);
```

with the stated rationale that this "is what makes `with:` on an inline-sourced node a TYPE ERROR instead of a value that silently parses and is dropped (#2478) — `with:` only exists on the 'command' variant, because only a command file has a `$INPUTS.<name>` surface for it to bind into." A nice piece of schema design: the file format itself prevents a whole class of silent misconfiguration.

`model:` accepts four shapes: a portable tier (`small|medium|large`), an `@alias`, a literal model id, or a **preference list** such as `model: ['@review-seams', '@reviewer', medium]`. Resolution order is documented: the node's own value → the workflow-level value → `assistants.*` in `.archon/config.yaml` → the provider's built-in default. Per-node inline sub-agents are declared as `agents:`, keyed by kebab-case id, Claude only.

### 2.7 Composition: `include:` versus `workflow:`

- **`include:`** is load-time inlining — "a load-time-only graph-construction directive, not a `DagNode`"; `expandWorkflowIncludes` consumes every directive at discovery time. It accepts only `id`, `description`, `depends_on`, `when`, `trigger_rule`, `include`, `with`. Writing `isolation:` or `input:` on an include is a named error, because "a composed block runs inside the run that composed it, so it has no checkout of its own to isolate".
- **`workflow:`** is a runtime governed child sub-run, with `input`, `with`, `isolation: 'inherit'|'worktree'`, and `fan_out`.
- **`fan_out`** declares `items` (a node output producing a JSON array), `as`, `max_parallel` (default 5, or `'all'`), and `join: 'all_success'|'all_done'|'first_success'` (default `all_done`).

The `include`/`workflow` distinction maps onto thirdshift's own: a Spec run dispatching child Ticket runs is `workflow:` with `isolation: worktree`; a Code review inside a Ticket run is `include:`.

### 2.8 Gates and pauses

Four mechanisms, one deprecated:

1. **`wait:` — the current one.** [`schemas/wait-node.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/schemas/wait-node.ts) allows exactly one of four strict variants: `{duration_ms}`, `{until}`, `{event, deadline_ms}`, or `{attention, form?, options?, labels?}`. `form` is `'message' | 'approval' | 'select' | 'multi_select'`, always a literal "because it fixes the node's output contract", and `ATTENTION_OUTPUT_FORMATS` **hardcodes** each form's output shape (`approval → {decision, text}` with `enum: ['approve','reject']`). `options` is required for the select forms and a load error on the others. The engine owns the lifecycle: "`output_format`, `retry`, and `always_run` cannot be set on a wait; the engine owns its output and continuation lifecycle." And `reject` carries no special meaning — "In the `approval` form `reject` is an answer like any other and never cancels the run; wire `cancel:` with `when: "$ask.output.decision == 'reject'"` if a rejection should end it."
2. **`approval:` — deprecated** in favour of `wait: {attention, form: approval}`. It had `message`, `decisions`, `capture_response`, `on_reject: {prompt, max_attempts}` (≤10), defaulting to `[{id:'approve'},{id:'reject'}]`.
3. **`loop.interactive: true` + `loop.gate_message`** — pauses between iterations.
4. **`cancel:` (`kind: 'halt'`)** — a 14-line schema: just a non-empty `reason`.

**How the pack actually gates is more interesting than the primitives.** It never uses `approval:`. Every public stage composes `archon-attention`, which ends in one `wait: {attention, form: select, options: "$settle.output.options"}` — a pause whose choices (`rerun | ready | finish`) are authored by a preceding agent node, and whose `decide` node emits a schema-validated recommendation with `choice`, `why`, `evidence`, `confidence: low|medium|high`, `would_change`, plus `if_unanswered` and `links`. So the human is asked once, at the end of a pass, with a structured recommendation and a stated cost of waiting — rather than at each step. The surrounding loop reads the answer back as `$LOOP_PREV.attend__ask.output.decision`.

And `archon-deliver-block`'s description states the doctrine plainly: **"There is no in-run approval gate — the human gate is PR review and merge on the forge, outside the run."** That is Warp's position too, and the opposite of thirdshift's Self-merge ([ADR-0004](../adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)).

### 2.9 What the engine keeps fixed

Archon writes its own rule down, and the document opens with the warning that matters most for #877. [`.archon/workflow-language-constitution.md`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/.archon/workflow-language-constitution.md):

> "Every workflow engine's configuration format faces the same gravitational pull: it grows until it becomes a bad programming language. Jenkins pipelines grew Groovy. GitHub Actions grew an expression language. Helm grew Turing-complete templating. Airflow grew so much Python-in-config that it eventually surrendered and became workflows-as-code. The pattern is always the same — individually reasonable feature grants, compounding into an informally-specified, untestable, half-language that is worse than code at computing and worse than configuration at declaring."

Its answer is a stated rule plus "the admissibility test applied to every proposed YAML feature, the known failure smells, and the management lever for each":

> **"YAML coordinates. Code computes. Agents judge."**
>
> "The workflow YAML exists to express what the **engine** must see in order to govern a run: ordering, gating, retrying, joining, pausing for humans, session identity, artifact identity, and reusable structure. Everything that *computes a value or transforms data* stays out of the YAML and lives inside a node's **body** — the `bash:`/`script:` source or the `prompt:` text… The YAML is the wiring between nodes — nothing more."

With a careful scoping note: "The partition governs **what may enter the YAML surface**, not what an agent may do inside a node. 'Code computes, agents judge' is shorthand for *the language does not compute* — it is not a prohibition on a prompt performing computation."

That is the best available answer to #877's "what goes in the file", and the opening paragraph is the best available argument for keeping the answer small. Concretely fixed:

| Fixed | Where |
|---|---|
| The eleven-selector node-mode table; no extension point; modes mutually exclusive | `dag-node.ts` |
| Node id `INPUTS` is reserved | `dag-node.ts` |
| `trigger_rule` closed to four values, default `all_success` | `dag-node-base.ts` |
| The `when:` grammar; no parentheses, functions or arithmetic; scalars only | `condition-evaluator.ts`, `when-atom.ts` |
| Bad `when:` syntax skips the node; an unresolvable reference **fails** it | `condition-evaluator.ts` |
| retry `max_attempts` 1–5, `delay_ms` ≤ 60 s, exponential doubling not configurable | `retry.ts` |
| Error buckets FATAL / TRANSIENT / UNKNOWN and their provider mappings; FATAL never retried even with `on_error: all`; capacity retry schedule 45/90/180/300/300 s ±50% jitter | `workflow-language.md` |
| Every attention form's output contract; `output_format`/`retry`/`always_run` forbidden on a wait | `wait-node.ts`, docs |
| Max durable wait: 1,000 years (`MAX_DURABLE_WAIT_MS`) | `durable-wait.ts` |
| `fan_out.max_parallel` default 5, `join` closed to three values — "Deciding **how many successes are enough** is judgement, and belongs in a downstream script or prompt node reading the aggregate — not in a YAML enum" | `workflow-node.ts`, constitution |
| `evidence_policy` gates only on file presence of `$ARTIFACTS_DIR/evidence.json`: "Deliberately narrow — no schema validation, no content checks, and no configurable path" | `workflow.ts` |
| Loops, `workflow:` children and fan-out rejected at load while `budget:` is declared | `workflow.ts` |
| Discovery precedence bundled < global < project | `workflow-discovery.ts` |
| Command/script resolution confined to the packaged workflow's own directories | docs |
| Repo-wide worktree knobs (`copyFiles`, `initSubmodules`, `path`, `baseBranch`) kept out of workflow YAML "because they are repo-wide, not per-workflow" | `workflow.ts` |

Three features were **retired with load errors rather than silence**: the old sequential `steps:` format, `thinking:` anywhere, and node-level `interactive:`. The last one's error text is a model for how to retire a field:

```text
Node 'plan': 'interactive' is only honoured inside 'loop:' or 'loop_group:';
  at the top level nothing reads it. Nothing on this node pauses. To ask
  for an answer, add a 'wait: { attention: <message> }' node; to ask on each iteration
  of a loop, use a 'loop_group' whose body ends in one.
```

### 2.10 Validation and versioning

- **No JSON Schema and no `version:` field.** The Zod schemas in `packages/workflows/src/schemas/` are the schema of record; the repo ships JSON Schemas for its chat and provider contracts but not for workflows. `version` is absent from `workflowBaseSchema` and from the loader. Versioning is handled instead by `deprecated: {message}` ("Metadata-only — never blocks or alters execution") plus scope shadowing.
- **Three validation levels.** Levels 1–2 (syntax, structure) are `parseWorkflow()` in `loader.ts`; level 3 is resource resolution in [`validator.ts`](https://github.com/coleam00/Archon/blob/ef04684469fe6d20a3d94bb2bda4407edbddc490/packages/workflows/src/validator.ts) — "checking that referenced resources actually exist on disk (command files, MCP configs, skill directories)" — run as `archon validate workflows <name>`. Its per-node checks: `validateOutputFormat`, `validateNodeModel`, `validateBundledWorkflowTarget`, `validateCommandReferences`, `validateMcp`, `validateSkills`, `validateProviderCapabilities`, `validateScriptNode`, `validateShellOutputRefs`. Bundled, global and installed workflows must additionally use portable model refs.
- **Unknown keys warn; they do not reject.** The opposite of Warp's `additionalProperties: false`: "A key Archon does not recognise is dropped, and the workflow still loads and runs… `WARNING [parse_warning] Node 'plan': unknown key 'contxt' will be ignored.`" Warnings are recorded per run as a `workflow_parse_warnings` event and belong to the declaring file — "If workflow A includes workflow B and B has an unknown key, running A reports nothing." Three blocks are exempt from the check (`output_format:` is free-form JSON Schema, `sandbox:` passes keys through, `hooks:` already errors).
- **Misplaced keys, by contrast, are hard load errors**: `runtime:`/`deps:` off a `script:`, `timeout:` off bash/script/loop, `with:` off command/script/include/workflow, `fan_out:` off workflow/include, and loop settings at a node's top level.
- **Workflows are testable as files.** `archon workflow run <name> --dry-run --stubs <file>` (with `--stubs-init` to scaffold), and `archon workflow test <name>`, which "replays the fixtures in the workflow's `fixtures/` folder and checks each declared outcome". That is what the ~40 `*.stubs.yaml` files in the SDLC pack are. This is the single most adoptable idea in Archon for thirdshift: a declared process becomes *unit-testable without running an agent*.

### 2.11 How a user overrides a default

Six distinct channels, and the layering is worth copying:

1. **Shadow by filename.** Scopes, lowest to highest: `bundled` (in the binary) < `global` (`~/.archon/workflows/`) < `project` (`<cwd>/.archon/workflows/`). "Same-named files at a higher scope override those at lower scopes." Installed packs (`archon plugin add`) sit outside that precedence, resolving includes within themselves and exposing only manifest entrypoints as `owner/plugin:<entrypoint>`. The docs tell you to copy the whole pack folder, since "its workflows include each other and share modules in `sdlc/.shared/`".
2. **Config file.** `.archon/config.yaml` (repo) and `~/.archon/config.yaml` supply `assistants.*`, `tiers:`, `aliases:`, `env:`, and the repo-wide container/worktree knobs.
3. **Run-time flags.** `--input <name>=<value>`, `--model @alias=provider/model`, `--branch`, `--from`, `--no-worktree`, `--container`, `--adopt`, `--detach`, `--dry-run --stubs`. Precedence is specified both ways: `worktree.enabled: false` makes `--branch` a hard error, and `true` makes `--no-worktree` one.
4. **Persistent model aliases.** `archon ai alias set @reviewer codex gpt-5.5`, or `aliases:` in config. The pack publishes a documented alias surface so you retarget models without editing YAML: `@reviewer` covers every agent node of review and simplify, `@review-<lens>` covers one lens, and "A lens alias overrides only its lens: with `@reviewer` on Codex and `@review-seams` on Claude, seams runs on Claude and every other lens on Codex."
5. **Sidecar policy documents — the most interesting one.** From the pack's README: "A project's policy for a decision the pack makes, such as what triage accepts, is one more sidecar document, kept at `.archon/sdlc/policy/<topic>.md`. The pack ships a default for each policy document a prompt names… A node reads the default first and then the project's own file, whose lines add to the default and win where the two conflict… To change one, write only the lines to add or override." Topics are `triage.md`, `rulings.md`, `build.md`, `merge.md`, `discoveries.md`. Anything that must be exact, such as label names, "stays typed configuration" (`maintainers.json`). That is prose-level override without forking a prompt — a direct answer to #118.
6. **Deprecation escape hatch.** A deprecation notice at run start carries "both ways out: switch to the replacement, or copy the workflow file into your project `.archon/workflows/`… A copy without the field has no notice."

## 3. GitHub Agentic Workflows (gh-aw)

`github/gh-aw` is MIT, 5,472 stars, canonical (not moved), with docs at `https://github.github.com/gh-aw/` built from `docs/src/content/docs/` in-repo. Pinned commit: `9a57ef760fda8457186126cf0f098ff595a3b5f7` (`main` HEAD, 2026-10-26).

**Scale warning.** The project is far bigger than its public description suggests: roughly 190 doc pages, about 400 ADRs under `docs/adr/`, and a single frontmatter JSON Schema of 890,279 bytes with 101 keys under `safe-outputs` alone. Any pre-existing mental model of gh-aw as "a bit of frontmatter over an agent prompt" is stale.

For #877, gh-aw is the *least* relevant on sequencing — it is one agent session per file — and the *most* relevant on two other things: how a file declares a capability budget rather than a procedure, and how much the engine can keep non-negotiable.

### 3.1 The file and the compile step

Workflows are `.github/workflows/*.md`, compiled to a sibling `*.lock.yml` GitHub Actions file ([workflow-structure](https://github.github.com/gh-aw/reference/workflow-structure/)):

> "Agentic workflows live in `.github/workflows` as Markdown files (`*.md`) and compile to GitHub Actions workflow files (`*.lock.yml`). Run `gh aw compile` to generate or refresh the lock file."

"Each workflow has two parts: YAML frontmatter wrapped in `---` for configuration and a markdown body for agent instructions." You commit both the `.md` and the `.lock.yml`.

One detail matters more than it looks: **"The markdown body is loaded at runtime and can be edited directly on GitHub.com without recompilation. Only frontmatter changes require recompilation."** So the prompt is live-editable while the *configuration* is compiled and pinned. That is a deliberate split between the part that needs review-and-rebuild and the part that does not — exactly the question #118 asks about thirdshift's prompts.

The cost is review surface. A lock file carries a machine-readable first line:

```
# gh-aw-metadata: {"schema_version":"v3","frontmatter_hash":"...","strict":true,"agent_id":"copilot"}
```

plus "Secrets used" and "Custom actions used" sections. But real lock files in this repo run 100–230 KB from a 16 KB source, so the PR diff is dominated by generated YAML. If thirdshift ever compiles a definition to something committed, that is the trap to avoid.

### 3.2 Frontmatter: `on` is the only required key

The schema (`pkg/parser/schemas/main_workflow_schema.json`, `$id: https://github.com/github/gh-aw/schemas/main_workflow_schema.json`, `version: "1.0.0"`) has exactly two hard structural facts: `required: ["on"]` and `additionalProperties: false`. An unknown top-level key is a compile error — Warp's stance, not Archon's.

The 70-odd top-level keys group into: identity (`name`, `description`, `emoji`, `intent`, `metadata`, `tracker-id`, `source`, `private`, `redirect`, `check-for-updates`), triggers (`on`, `if`, `run-name`), model and engine (`engine`, `model`, `models`), capability (`permissions`, `tools`, `mcp-servers`, `mcp-scripts`, `skills`, `plugins`, `lsp`, `network`, `sandbox`, `enclaves`, `secrets`, `secret-masking`, `env`, `excluded-env`), execution (`runner`, `runs-on`, `runs-on-slim`, `container`, `services`, `runtimes`, `cache`, `checkout`, `concurrency`, `environment`, `timeout-minutes`), hooks and jobs (`pre-steps`, `steps`, `pre-agent-steps`, `post-steps`, `jobs`), outputs (`safe-outputs`), caps (`max-turns`, `max-tool-calls`, `max-tool-denials`, `max-turn-cache-misses`, `max-ai-credits`, `max-daily-ai-credits`, `max-runs`, `user-rate-limit`, `work-queue-policy`), composition (`imports`, `inlined-imports`, `import-schema`, `ambient-folders`), evaluation (`evals`, `graders`, `observability`), and `strict`.

Note the spelling: **`timeout-minutes`, `safe-outputs`, `max-turns`, `runs-on` are hyphenated**, not underscored.

### 3.3 Triggers

`on:` takes the standard GitHub Actions events plus a large gh-aw-specific set. The ones with no counterpart elsewhere in this note:

- **`slash_command:`** (formerly `command:`, now deprecated) — fires on `/name` in a comment; `null` "defaults to using the workflow filename (without .md extension) as the command name"; `'*'` matches any slash command, and a `'smoke*'` suffix enables prefix matching.
- **`label_command:`** — "fires when a specific label is added to an issue, pull request, or discussion. **The triggering label is automatically removed at workflow start so it can be applied again to re-trigger.**" That is a neat answer to a problem thirdshift solves differently with its Claim labels.
- **`roles:`** — "Repository access roles required to trigger agentic workflows. Defaults to `['admin', 'maintainer', 'write']` for security." Crucially, **"no privilege hierarchy — `roles: [write]` rejects `admin` and `maintainer` actors"**. `all` "disables permission checking entirely". This is gh-aw's version of thirdshift's Trusted account check, declared in the file.
- **`cooldown:`** — "Minimum time after the most recent completed workflow run where the agent job started before another agent run may start… must be at least 5 minutes, and does not support GitHub Actions expressions."
- **`stop-after:`** — an expiry for the whole workflow: absolute dates, or relative deltas like `+3d`, capped at 365 days, and "Minute unit 'm' is not allowed for stop-after; minimum unit is hours 'h'."
- **`manual-approval:`** — an environment name that must approve before the run starts.
- **`stale-check:`** — `true` (default, frontmatter hash) / `false` / `"full"` ("check both frontmatter and body hashes"), i.e. whether a run refuses to proceed against a stale lock file.
- **`schedule:`** accepts fuzzy strings — `'daily around 14:00'`, `'weekly on friday around 5pm'`, `'every 2h'` — and "Fuzzy schedules distribute execution times to prevent load spikes… Minimum interval is 5 minutes." Array form takes `{cron, timezone}`.
- `skip-if-match` / `skip-if-no-match` / `skip-if-check-failing`, `skip-bots`, `bots`, `labels`, `repos`, `orgs`, `status-comment`, `reaction` (default `"eyes"`), plus non-GitHub `jira-issue`, `linear-issue`, `azure-devops-issue`.
- `on.steps:` injects steps into the pre-activation job, whose results become `needs.pre_activation.outputs.<id>_result` — a declarative hook for a custom pre-flight check.

### 3.4 Engine choice

`engine:` is a string (`claude`, `codex`, `copilot`, `gemini`, `pi`, or a named catalog entry) or an object with `agent`, `api-target`, `args`, `auth`, `bare`, `checksums-sha256`, `command`, `concurrency`, `config`, `context-window`, `copilot-sdk`, `cwd`, `driver`, `dynamic-workflows`, `env`, `extensions`, `harness`, `id`, `max-continuations`, `mcp`, `model`, `model-provider`, `model-routing`, `permission-mode`, `user-agent`, `version`.

GitHub Copilot is the default. Each engine takes an API key or workload identity federation — `ANTHROPIC_API_KEY`, `CODEX_API_KEY`/`OPENAI_API_KEY`, `GEMINI_API_KEY`, or `copilot-requests: write`. **No subscription path**, consistent with [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md) §2.

Fields worth noting against thirdshift's own harness handling:

- `engine.permission-mode` (Claude only) is an enum of `default`, `dontAsk`, `auto`, `acceptEdits`, `plan`, `bypassPermissions`, and "**Defaults to `dontAsk` for unattended runs**". thirdshift passes `--permission-mode auto` ([prompts/README.md](../../prompts/README.md)); gh-aw makes the choice a declared field with an unattended-safe default.
- `engine.bare` "disables automatic loading of context and custom instructions… Copilot uses `--no-custom-instructions`, Claude uses `--bare`" — a declared answer to the "don't load the user's personal config" problem raised in [open-source-factories.md](open-source-factories.md) §4 follow-up 3.
- `engine.fallback-models` is an "Ordered alternative models after model-specific failures… Each model receives its own harness retry budget", and "Cannot be combined with model-routing or a custom driver or harness".
- `engine.model-routing` takes `{goal: cost|cost-speed, mode: economy|balanced|robust|auto}` and "requires the AWF firewall and ignores fixed model and effort settings".
- `engine.max-turns` is deprecated in favour of top-level `max-turns`, "so the AWF turn cap applies consistently".
- `engine.checksums-sha256` pins the "literal SHA256 digest of the release's SHA256SUMS.txt manifest… bypasses the runner toolcache so the downloaded archives are verified".
- `engine.concurrency` "Defaults to single job per engine across all workflows (group: `gh-aw-{engine-id}`)".

### 3.5 `safe-outputs`: the file declares *effects*, not steps

This is gh-aw's central idea and the one most worth stealing. The agent job never has write permission. Instead the file declares which GitHub writes the run is allowed to produce:

> "Safe output processing configuration that automatically creates GitHub issues, comments, and pull requests from AI workflow output **without requiring write permissions in the main job**"

There are 101 keys. The GitHub-side ones include `create-issue`, `add-comment`, `create-pull-request`, `push-to-pull-request-branch`, `add-labels`, `remove-labels`, `replace-label`, `update-issue`, `close-issue`, `merge-pull-request`, `submit-pull-request-review`, `create-pull-request-review-comment`, `resolve-pull-request-review-thread`, `mark-pull-request-as-ready-for-review`, `add-reviewer`, `assign-to-user`, `assign-milestone`, `link-sub-issue`, `create-check-run`, `create-code-scanning-alert`, `autofix-code-scanning-alert`, `upload-asset`, `upload-artifact`, plus Jira/Linear/Azure DevOps equivalents, plus the orchestration ones in §3.7, plus reporting (`report-missing-tool`, `report-missing-data`, `report-incomplete`, `report-failure-as-issue`, `report-failed-jobs`, `noop`).

Each handler carries its own policy, and the defaults are conservative. For `create-pull-request`: `draft` **defaults to `true`**, `signed-commits` to `true`, `stacked` to `true`, `fallback-as-issue` to `true`, `auto-merge` to `false`; plus `allowed-files`, `excluded-files`, `protected-files` (`request-review` default, or `blocked`/`allowed`/`fallback-to-issue`), `allowed-branches`, `allowed-base-branches`, `allowed-labels`, `allowed-repos`, `max` (default 1), `max-patch-size`, `max-patch-files`, and `if-no-changes` (`warn` default, `error`, `ignore`). For `add-labels`: `max` default 5, `max-labels` default 10, `create-if-missing` off by default so "Default (omitted or false) rejects labels that don't already exist", and `blocked` globs "Applied before allowed list filtering for security". For `create-issue`: `max` default 1, `deduplicate-by-title` taking either exact matching or "a non-negative integer (0–100) to deduplicate by Levenshtein edit distance", `expires` with a 2-hour minimum, and `group` to file sub-issues under a parent with "a maximum of 64 sub-issues per parent".

Two of those are directly the "protected paths" follow-up in [open-source-factories.md](open-source-factories.md) §4.2: `protected-files` and `allowed-files` are exactly that, as declared per-handler policy.

`merge-pull-request` does exist, but only "under configured policy gates" (`allowed-branches`, `allowed-labels`, `required-labels`, `required-title-prefix`). So unlike Warp and Archon, gh-aw does not rule out an agent-triggered merge — it constrains it declaratively. That is the closest precedent for thirdshift's Self-merge being expressed in a file rather than in Rust.

### 3.6 Caps — the richest declarable set here

| Field | Semantics |
|---|---|
| `max-turns` | "AWF turn cap (`max_turns`) applied consistently across all agentic engines" |
| `max-tool-calls` | "Maximum aggregate number of tool dispatches for the workflow run, including nested calls… **Custom drivers cannot enforce this contract.**" |
| `max-tool-denials` | "Safeguard threshold for repeated tool denials before stopping inference… Defaults to 5 for Copilot" |
| `max-turn-cache-misses` | consecutive cache misses before the API proxy blocks further requests |
| `max-ai-credits` | "Per-run AI Credits budget control… **Enabled by default at 1000 (1k) when omitted.** Set to -1 to disable both budget enforcement and token steering." |
| `max-daily-ai-credits` | "24-hour AI Credits guardrail for all runs of this workflow. Omit the field to leave the guardrail disabled." |
| `user-rate-limit` | `max-runs-per-window` (1–10), `window` 1–180 minutes (default 60), `events`, `ignored-roles` (default `[admin, maintain, write]`; "To apply rate limiting to all users, set to an empty array") |
| `timeout-minutes` | agent step, default 20; the agent job is separately bounded at 60 and the detection job at 10, each overridable by a repo variable |
| `safe-outputs.timeout-minutes` | default 45 |
| `on.cooldown` | ≥5 minutes between agent runs |

A cost budget that is **on by default** and must be explicitly disabled with `-1` is the opposite of Warp, which declares no cap at all. For an unattended factory on a metered plan, that default is the right way round.

Egress is declared too. `network:` takes ecosystem identifiers (`dotnet, python, node, go, java, ruby, rust, swift, php, dart, haskell, perl, containers, github, terraform, linux-distros, playwright, defaults`) and wildcard domains, with "Empty array is valid and means deny all network access", and a warning that "'defaults' alone only covers basic infrastructure, not package registries". `sandbox.agent` defaults to `awf`, the Agent Workflow Firewall, which "containerizes the agent, binds it to a Docker network, and uses iptables to redirect HTTP/HTTPS traffic through a Squid proxy container… The AWF setup process drops its iptables capabilities before launching the agent", and emits a `firewall-audit-logs` artifact (`squid-logs/access.log`, `audit.jsonl`, `policy-manifest.json`). **"The MCP gateway is always enabled and cannot be disabled."**

### 3.7 One session per file, and four ways around it

The compiled graph has exactly one `agent` job. Multi-session work is therefore composition, not sequencing:

1. **Autopilot continuations.** `engine.max-continuations` (Copilot only): "Default is 1 (single run, no autopilot). Values greater than 1 enable `--autopilot` mode… with `--max-autopilot-continues` set to this value."
2. **Inline sub-agents.** Declared *in the Markdown body*, delimited by `` ## agent: `name` `` with optional `` ## end agent: `name` ``, each carrying mini-frontmatter (`model`, `description`, `tools`). The compiler stages them per engine to `.github/agents/<name>.agent.md`, `.claude/agents/<name>.md`, `.codex/agents/<name>.md`, and so on ([inline sub-agents](https://github.github.com/gh-aw/reference/inline-sub-agents/)). A single-file way to ship the per-agent files every other tool keeps separate.
3. **Workflow→workflow fan-out, as a safe output.** `call-workflow`: "The compiler validates that each worker declares `workflow_call`, generates a typed MCP tool per worker from its inputs, and emits a conditional `uses:` job. At runtime the worker whose name the agent selected executes as part of the same workflow run — preserving `github.actor` and billing attribution." `dispatch-workflow` instead runs workers "asynchronously as independent workflow runs". The guidance: "Use `call-workflow` when actor attribution matters, workers must finish before the orchestrator concludes… Use `dispatch-workflow` when workers should run asynchronously, outlive the parent run" ([orchestrator-ops](https://github.github.com/gh-aw/patterns/orchestrator-ops/)). Also `create-agent-task` — "Allows workflows to spawn new agent sessions for follow-up work."
4. **Deterministic jobs.** Top-level `jobs:`, `safe-outputs.jobs`, `safe-outputs.needs`, `on.needs`, and the four step hooks.

So the *sequence* lives in the GitHub Actions graph and in which safe outputs the agent chooses to emit — not in a stage list. Worth noting for #877: this is a third model, distinct from both "prose in a foreman prompt" and "a declared graph". The engine declares a *capability envelope*, and the agent picks the path inside it.

**Composition via `imports` has explicit merge semantics**, which is the part worth copying if thirdshift ever has more than one definition file ([imports](https://github.github.com/gh-aw/reference/imports/)):

> "Files without a trigger event are shared workflow components. They are validated, importable by other workflows, and not compiled into standalone GitHub Actions."

| Block | Merge rule |
|---|---|
| `tools:` | "Deep merge; `allowed` arrays concatenate and deduplicate" |
| `mcp-servers:` | "Imported servers override same-named main servers; first-wins across imports" |
| `network:` | "`allowed` domains union (deduped, sorted)" |
| `permissions:` | "Validation only. Main must declare all imported permissions at sufficient levels (`write` ≥ `read` ≥ `none`)" |
| `safe-outputs:` | "Each type may be defined once; main overrides imports. Duplicate types across imports fail" |
| `jobs:` | "Not merged; define only in the main workflow. Use `safe-outputs.jobs` for importable jobs" |
| `env:` | "Duplicate keys across imports fail compilation" |

Resolution is "a deterministic breadth-first traversal… Visited files are tracked to detect cycles", a file "can appear at most once in an import graph", and "Importing the same file with **different** `with` values is a compile-time error". Typed parameters come from `import-schema` plus `uses`/`with`, read as `${{ github.aw.import-inputs.<key> }}`. Only a fixed subset of fields is importable; "Other fields generate warnings and are ignored".

Note that only Warp's "replace, not merge" and gh-aw's per-block table attempt this at all. Inheritance rules are where a file format quietly becomes confusing, and gh-aw's answer — a different documented rule per block, rather than one global rule — is the honest one.

### 3.8 Gates

Five declarative mechanisms:

1. **`safe-outputs.staged: true`** — dry run for effects. "Staged mode lets you run a workflow and preview the built-in safe outputs it would create… without applying those writes. Staged built-in operations are replaced with a detailed 🎭 preview in the GitHub Actions step summary." The precedence is one-way: "Type-level `staged: false` cannot disable global staging, including staging forced by compile `--staged` or `--dry-run`." With an honest limit: **"Staging is not a sandbox: custom jobs, scripts, external MCP servers, custom credentials, memory/cache persistence, and diagnostic exports require separate controls"** — custom jobs merely receive `GH_AW_SAFE_OUTPUTS_STAGED="true"` and "their code must honor the staging signal" ([staged mode](https://github.github.com/gh-aw/reference/staged-mode/)).
2. **`on.manual-approval: <environment>`** — GitHub environment approval before the run starts, plus `environment:` on the job and on `safe-outputs`.
3. **Draft by default** — `create-pull-request.draft: true`, with `mark-pull-request-as-ready-for-review` as a separate declared output.
4. **Role, label and title gating** — `on.roles`, `on.label_command`, and per-handler `required-labels` / `required-title-prefix` / `allowed-*`.
5. **Threat detection as a job-level gate** — `safe-outputs.threat-detection`, whose `continue-on-error` defaults to `true` ("detection failures produce warnings and allow safe outputs to proceed with a caution notice and 'needs-review' label"); set `false` and failures "block safe outputs entirely". The reasoning is explicit about why it is a job boundary: "The `safe_outputs` job condition `needs.detection.outputs.success == 'true'` prevents the runner from starting at all if detection fails. Step-level `if` checks within one job are weaker."

Plus `tools.github.min-integrity` (`merged`, `approved`, `unapproved`, `none`) with `approval-labels`, `trusted-users`, `blocked-users`, and "For public repositories, `min-integrity: approved` is applied automatically".

### 3.9 What the compiler always emits

This is the strongest "fixed in the engine" story of anything surveyed, and the reasoning is published ([compilation process](https://github.github.com/gh-aw/reference/compilation-process/)).

**A fixed job topology.** `pre_activation` (gating), `activation` (always: "Prepares workflow context, sanitizes event text, validates lock file freshness"), `agent` (always), `detection` (when threat detection is configured), the per-output safe-output jobs, and `conclusion` (always, if safe outputs exist).

**Permission separation is structural and not overridable:**

> "These three jobs form a **sequential security pipeline** rooted in Plan-Level Trust — AI reasoning (read-only) is separated from write operations. They cannot be merged because GitHub Actions permissions are per-job and immutable for the duration of a job."
>
> "A combined job would hold write permissions while running threat detection, defeating least privilege and letting a compromised agent bypass the gate."
>
> "Artifact-based handoff. The agent writes `agent_output.json`; detection emits `success`; safe_outputs only downloads the artifact if approved. A shared filesystem in a single job would allow output tampering between phases."

**Always-on, non-overridable processing:**

- **Input sanitization** at the activation boundary: `@user` → `` `@user` ``, `fixes #123` → `` `fixes #123` ``, `<script>` → `(script)`, non-HTTPS/`data:`/`javascript:`/untrusted-domain URLs redacted, Unicode homoglyphs normalised, ANSI escapes stripped, and "Enforces 0.5MB max size, 65k lines max". The allowed-domain list is derived from the workflow's own `network:` config.
- **Secret redaction**: "all files in the `/tmp/gh-aw` directory are scanned for secret values and redacted… executes unconditionally (with `if: always()`), ensuring that secrets are protected even if the workflow fails at an earlier stage."
- **Markdown body scanning**: rejects Unicode abuse, hidden content, obfuscated links, dangerous HTML tags, embedded executables, and "social engineering patterns (prompt injection, base64-encoded commands, pipe-to-shell patterns). **These checks cannot be overridden.**"
- **The MCP gateway**, which also "intercepts tool calls and filters content below the configured `min-integrity` threshold before the AI engine sees it".
- **Action SHA pinning**: "All GitHub Actions are pinned to commit SHAs… Resolution order is cache (`.github/aw/actions-lock.json`) → GitHub API → embedded pins."

The sanitization table is directly relevant to thirdshift's own Trusted-account filtering: thirdshift writes the issue to `.thirdshift-issue.md` with untrusted comments left out ([skills/thirdshift-implement/SKILL.md](../../skills/thirdshift-implement/SKILL.md)), which is the same idea at a coarser grain. gh-aw additionally neutralises mentions and closing keywords inside text it does pass through.

### 3.10 `strict` mode, validation and versioning

`strict` is a boolean **defaulting to `true`**, and it is the clearest example of a format making the safe thing the default:

> "Strict mode enforces: (1) Write Permissions - refuses `contents:write`, `issues:write`, `pull-requests:write`; requires safe-outputs instead, (2) Network Configuration - requires explicit network configuration with no standalone wildcard '*'…, (3) Action Pinning - enforces actions pinned to commit SHAs instead of tags/branches, (4) MCP Network - requires network configuration for custom MCP servers with containers, (5) Deprecated Fields - refuses deprecated frontmatter fields… CLI flag takes precedence over frontmatter."

These are enforced in Go (`pkg/workflow/strict_mode_validation.go`, `validateStrictMode()`) rather than in the schema — the schema's own `$comment` says so. **[unverified]**: I did not read that Go file.

Validation:

- `gh aw compile --validate`, and `gh aw validate` which is "Equivalent to `gh aw compile --validate --no-emit --zizmor --actionlint --poutine`", with "Built-in Actions/AW AST lint, external linters… `--validate`, and `--no-emit` are always-on defaults and **cannot be disabled**".
- `--dry-run` "enables `--strict`, `--staged`, `--validate`, `--shellcheck`, and `--models`. These settings are forced regardless of frontmatter; explicitly disabling a required flag is rejected."
- Compile-time AST lint runs by default "against the generated execution graph and authored source before writing the lock file… Error findings fail compilation; warnings remain advisory", with rule IDs mapped to a published threat spec (`action-pin` CTR-002, `template-injection` CTR-006, `permission-scope` CTR-001, `network-scope` CTR-011, `push-target` CTR-012, and so on).
- Schemas and linters are "bundled with the CLI; they do not download schemas or install linters during compilation" — the opposite choice from Warp's deliberately server-side validator, and defensible because gh-aw's schema ships *with* the compiler that reads it.

Versioning:

- The schema carries `version: "1.0.0"`; the lock file carries `schema_version: "v3"` and a `frontmatter_hash`. Staleness is checked at run time via `on.stale-check` (`true` default / `false` / `"full"` to hash the body too).
- Generated lock files pin `github/gh-aw-actions` by SHA, and the docs are blunt about who owns those pins: "Dependabot may open PRs to bump `github/gh-aw-actions` to a newer SHA. **Do not merge them** — pin updates must come from `gh aw compile`, which coordinates pins across all compiled workflows from a single release." `gh aw compile` auto-inserts the Dependabot ignore rule.
- Provenance fields on the workflow itself: `source:` ("Format: `owner/repo/path@ref`"), `redirect:`, `check-for-updates:`, and `private:` ("preventing it from being added to other repositories via `gh aw add`").

### 3.11 How a user overrides a default

Frontmatter overrides the engine default; the CLI overrides frontmatter, in one direction only. `strict: false` is honoured, but `gh aw compile --strict` and `--dry-run` force strict regardless. `sandbox.agent: false` removes the firewall but is "Not allowed in strict mode". Model resolution is top-level `model:` → `engine.model:` → per-nested-engine (e.g. `safe-outputs.threat-detection.model`). Repo variables supply defaults for the three timeouts. And there is a CLI editor for the file itself: `gh aw edit <wf> --set engine.model=small`, with repeatable `--set/--unset/--add/--remove` frontmatter paths, plus `--add-import`, `--add-skill`, `--schedule`.

Not overridable at all: the MCP gateway, input sanitization, secret redaction, markdown body scanning, action SHA pinning, and the job-level permission separation.

### 3.12 A real example

`.github/workflows/changeset.md`, frontmatter verbatim ([at the pinned SHA](https://github.com/github/gh-aw/blob/9a57ef760fda8457186126cf0f098ff595a3b5f7/.github/workflows/changeset.md)):

```yaml
---
emoji: "📋"
name: Changeset Generator
description: Automatically creates changeset files when PRs are labeled with 'changeset' or 'smoke' to document changes for release notes
on:
  pull_request:
    types: [labeled]
    names: ["changeset", "smoke"]
  workflow_dispatch:
  reaction: "rocket"
max-daily-ai-credits: 10000
if: github.event.pull_request.base.ref == github.event.repository.default_branch
permissions:
  contents: read
  pull-requests: read
  issues: read
model: copilot/gpt-5.6-sol
engine:
  id: codex
  model-provider: github
strict: true
safe-outputs:
  push-to-pull-request-branch:
    patch-format: bundle
    commit-title-suffix: " [skip-ci]"
    allowed-files:
      - .changeset/*.md
    protected-files:
      policy: blocked
      exclude:
        - .changeset/
  update-pull-request:
    title: false
    operation: append
  threat-detection: {}
timeout-minutes: 20
network:
  allowed: [defaults, github, node, go]
tools:
  cli-proxy: true
  github:
    mode: local
  bash: ["*"]
  edit:
imports:
  - shared/changeset-format.md
  - ../skills/jqschema/SKILL.md
  - shared/otlp.md
  - shared/graders.md
evals:
  - id: changeset-created
    question: Did the agent create a valid changeset file for the labeled pull request?
  - id: content-accurate
    question: Does the changeset accurately summarize the changes described in the pull request?
---
```

Read that as a statement of the whole philosophy: read-only `permissions`, every write named and bounded under `safe-outputs`, an explicit `network.allowed`, shared config by `imports`, `strict: true`, and `evals` declared in the same file as questions in plain English. There is no sequence anywhere in it.

## 4. Others, briefly

### 4.1 Anneal — the closest thing to a declared, gated, capped pipeline

`mosonlab/anneal` is MIT, pushed 2026-09-26, and self-describes as "coding agent chains plan, review, implement, verify and merge unattended on your own machine, on the GPT and Claude subscriptions you already have" (`gh api repos/mosonlab/anneal`). Pinned commit `bfa821b066355dfd321d15e43162a9047cfa4091`. [open-source-factories.md](open-source-factories.md) §2.3 covers its practices; this is only its file format, which that note does not.

**A workflow is a directory of numbered Markdown steps.** `agents/templates/<workflow-name>/NN-<slug>.md`:

```
agents/templates/compound-engineer-workflow/   01-write-a-spec.md … 12-merge-execution.md
agents/templates/direct-engineer-workflow/     01-revalidate.md … 08-merge-execution.md
agents/templates/pr-engineer-workflow/         01-implementation.md … 04-apply-review-fixes.md
```

([tree at `bfa821b`](https://github.com/mosonlab/anneal/tree/bfa821b066355dfd321d15e43162a9047cfa4091/agents/templates))

**Each step declares its structure in frontmatter and its procedure in the body** ([`03-plan-review.md`](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/agents/templates/compound-engineer-workflow/03-plan-review.md)):

```yaml
---
stepIndex: 3
layer: 3
agent: review-coordinator-sol-high
approvalGate: false
optional: false
outputKind: plan-review
priorOutputKinds: [spec, plan]
attachmentsFromPrevious: true
opensPullRequest: false
requiresCommit: false
provisionDependencies: true
baseFromStepIndex: null
spawnPolicy: null
---
Review every vertical slice against the specification of record and frozen base: …
```

What each field buys, from [`template-sources.ts`](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/packages/db/src/template-sources.ts) (`TemplateStepSource`) and [`template-step-fields.ts`](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/packages/db/src/template-step-fields.ts):

- `stepIndex` + `layer` — sequence *and* parallelism. Steps in the same layer run together; layers run in order. The compound workflow's layers are `[1,2,3,4,5,6,6,7,8,9,10,11]`, so steps 6 and 7 (code review and blind code review) are the one parallel pair.
- `agent` — the role file, `agents/roles/<name>.md`. Those names encode model and effort by convention (`code-reviewer-sol-high`, `senior-dev-astra-xhigh`, `plan-executor-astra-low`), and 24 of them exist.
- **`approvalGate: true|false`** — a declared human gate. This is the field no other tool here has.
- `optional` — the step may be skipped.
- `outputKind` / `priorOutputKinds` — a **typed hand-off contract**. A step declares the artifact kind it produces and the kinds it consumes; the validator checks every consumed kind is produced in an earlier layer.
- `attachmentsFromPrevious`, `opensPullRequest`, `requiresCommit`, `provisionDependencies` — side-effect declarations.
- `baseFromStepIndex` — a review step pins its diff base to a named earlier step's end commit. [ADR-0008](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/docs/adr/0008-templates-are-immutable-once-used.md) K10: "Review steps pin their base with `baseFromStepIndex` to the predecessor's end commit… This provides fetch-level isolation with fail-stop semantics and no branch-head fallback."
- `spawnPolicy` — a JSON object; its contents are **[unverified]**.

**The agent file is separate and carries the model** ([`code-reviewer-sol-high.md`](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/agents/roles/code-reviewer-sol-high.md)):

```yaml
---
name: code-reviewer-sol-high
title: Code Reviewer
model: openai-codex/gpt-5.6-sol:high
runner: pi
inboxAccess: false
collaborators: []
---
You are an independent code reviewer. …
```

**Validation is a named rule set, not a schema.** [`template-authoring.ts`](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/packages/api/src/template-authoring.ts) numbers fifteen rules, with the reasoning in a comment above each: `graph_empty`, `first_step_not_agent`, `first_step_optional`, `first_layer_not_single`, `layer_order_invalid`, `base_step_invalid`, `base_step_optional`, `gate_slot_step_optional`, `optional_step_precedes_merge_tail`, `prior_kind_unproduced`, `output_kind_duplicate`, `prior_kind_duplicate`, `approval_gate_in_parallel_layer`, `assignee_invalid`, `integrator_binding_invalid`. Two examples of the reasoning:

> "4. `first_layer_not_single`. A parallel first layer has no unique starting [step]"
>
> "13. `approval_gate_in_parallel_layer`. Approval is a single decision point; placing it alongside another Step would leave the layer with no unique [point]"

Validation has two tiers — errors block, warnings pass (ADR-0008 Consequences).

**Versioning is the strictest of anything here.** A template with instantiation history is immutable; authoring is clone-then-edit; the step graph is replaced whole by one atomic `PUT /task-templates/:id/steps`; and "A prompt revision on a step with history rides a registered generation rollover. Unregistered prompt drift refuses deployment", because "The generation registry carries prompt digests in both directions, so outgoing and successor prompts are pinned" ([ADR-0008](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/docs/adr/0008-templates-are-immutable-once-used.md) K8, K9).

**What the engine keeps fixed.** The three canonical workflows are hardcoded specs in [`template-sources.ts`](https://github.com/mosonlab/anneal/blob/bfa821b066355dfd321d15e43162a9047cfa4091/packages/db/src/template-sources.ts) — `CANONICAL_TEMPLATE_SOURCE_SPECS` pins each one's `name`, `stepCount`, `layers` array and `stepNames`, and the source files are checked against it. So the shipped workflows are *not* user-editable in place: the files are the source of truth for a canonical template's content, and a drift test enforces agreement. A user gets a custom pipeline by cloning a template through the API, not by editing `agents/templates/` **[interpretation]**.

**Caps**: none visible in the step frontmatter. The concurrency bound is runner count, not a declared retry limit **[unverified]** — I did not exhaustively search the scheduler.

### 4.2 Claude Code subagents

`.claude/agents/<name>.md` (project), `~/.claude/agents/` (user), plugin `agents/`, managed settings, or `--agents` JSON. "Identity comes from the `name` field, not the path or filename", and the precedence order is managed settings → `--agents` → project → user → plugin ([Subagents](https://code.claude.com/docs/en/sub-agents)).

Frontmatter: `name`, `description` (both required), `tools`, `disallowedTools`, `model` (`sonnet`/`opus`/`haiku`/`fable`/a full ID/`inherit`), `permissionMode`, **`maxTurns`**, `autoCompactWindow`, `skills` (up to 32, preloaded), `mcpServers`, `hooks`, `memory`, `background`, `omitClaudeMd`, `effort`, `isolation: worktree`, `color`, `initialPrompt`, `experimental`. Body is the system prompt. "Unrecognized fields are ignored without an error."

Notable against #877: it has the two fields Warp lacks — `skills:` as an explicit per-agent attachment, and `maxTurns` as a declared cap ("Output is marked partial and can be resumed"). It also has `isolation: worktree`, which is thirdshift's execution model as a frontmatter flag.

**No sequence.** "Frontmatter has no field for order, dependencies, triggers, or a 'next agent.' Claude decides sequencing at runtime from the prompt and descriptions." Chaining is a natural-language request. Nesting defaults to 3 layers (`CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`), concurrency to 20 (`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`) — both env vars, not file fields. `SubagentStart`/`SubagentStop` hooks "can react to lifecycle events but don't define an ordering". Some tools are always stripped from subagents and `tools` cannot add them back.

### 4.3 OpenCode agents

`.opencode/agents/<name>.md` or `~/.config/opencode/agents/`; "The markdown file name becomes the agent name" ([Agents](https://opencode.ai/docs/agents/)). Frontmatter: `description` (required), `mode` (`primary`/`subagent`/`all`), `model` (`provider/model-id`), `temperature`, `top_p`, **`steps`** ("Maximum agentic iterations before the agent must respond with text only"; legacy `maxSteps` deprecated), `disable`, `hidden`, `color`, `permission`, deprecated `tools`. Unknown keys pass through to the provider as model options.

`permission` is the interesting part: per-capability `ask`/`allow`/`deny` over `read`, `edit`, `glob`, `grep`, `list`, `bash`, `task`, `webfetch`, `websearch`, `lsp`, `skill`, `question`, `todowrite`, `external_directory`, `doom_loop`, some accepting glob objects, last match wins. `permission.task` is what controls which subagents an agent may invoke — a capability-shaped equivalent of Warp's `spawnableBy`.

There is a JSON equivalent in `opencode.json` under an `agent` key, where `prompt` can be a file reference (`{file:./prompts/review.txt}`) resolved relative to the config file. No sequence or pipeline is declarable; coordination is runtime.

### 4.4 Factory.ai droids

`.factory/droids/<name>.md` (project, wins) or `~/.factory/droids/<name>.md`; "The CLI scans only the top level of each `droids/` folder" ([Custom droids](https://docs.factory.com/cli/configuration/custom-droids)). Frontmatter: `name` (required; sets `subagent_type`), `description`, `model` (`inherit` default, a public ID, or `custom:` + BYOK), `reasoningEffort` (`low`/`medium`/`high`, ignored when `model: inherit`), `tools` (omit for all, or a category `read-only`/`edit`/`execute`/`web`/`mcp`, or explicit IDs), `mcpServers`. Body is the system prompt and cannot be empty.

Fixed: `TodoWrite` and `Skill` always included; `ExitSpecMode` and `GenerateDroid` cannot be enabled; `tools: all` is rejected; subagents cannot spawn subagents. No sequencing fields — "Delegation happens at runtime: the parent agent calls droids through the Task tool."

### 4.5 Not what they sound like

- **Warp `.warp/workflows/*.yaml`** in `warpdotdev/warp` are saved terminal commands, not factory definitions.
- **Anneal `.agents/skills/*/agents/openai.yaml`** is per-skill agent config inside a skill, not a pipeline.

## 5. Comparison

### 5.1 What the file declares

| | **Warp Factories** | **Warp `blueprint`** | **Archon** | **gh-aw** | **Anneal** | **Claude Code / OpenCode / droids** |
|---|---|---|---|---|---|---|
| Root file | `factory.yaml` | same | one YAML per packaged workflow | `.github/workflows/*.md` | DB rows seeded from `agents/templates/<wf>/` | none |
| **Sequence** | prose in foreman prompt | **declared**: named `stages` + `to` edges, nestable | **declared**: DAG of `depends_on` + `trigger_rule` | not declared (one agent job; GH Actions graph around it) | **declared**: `stepIndex` + `layer` | not declarable |
| **Parallelism** | foreman's choice | nested scopes | same `depends_on` layer; `fan_out` with `max_parallel` | GH Actions jobs | same `layer` | runtime, env-var capped |
| **Branching** | prose | `{stage, when}` where `when` is **prose the agent judges** | `when:` mini-grammar over typed node outputs | `if:`, `skip-if-*`, handler policy | none (linear + `optional`) | none |
| **Per-step prompt** | agent's `agent.md` body | stage `prompt` | `prompt:` inline or `command:` file in the workflow's own `commands/` | body (live-editable) + inline sub-agents | step file body | agent file body |
| **Per-step skill** | by file path only (`agents/<n>/skills/`) | inherits the agent's | `skills:` per node | `skills:`, `imports` of a `SKILL.md` | via the role file | `skills:` (CC only) |
| **Per-step model/harness** | agent frontmatter `model` xor `harness` (6 types) | the stage's `worker` agent | per node `model`/`provider`/`effort`/`fallbackModel`, tiers + `@alias` + preference list | `engine` + `model` per file | role file `model:`/`runner:` | frontmatter `model` |
| **Gates** | **prose** (`## Human gates`) | none (prose in a stage prompt) | `wait: {attention, form}`; pack prefers one structured pause per pass | `staged`, `manual-approval`, `draft`, `roles`, threat gate | **`approvalGate: true`** | none |
| **Caps / retries** | **none** (only `idleTimeoutMinutes`) | none | `max_iterations` **required**; `retry.max_attempts` 1–5; `timeout`/`on_timeout`; `budget` | the richest: `max-turns`, `max-tool-calls`, `max-ai-credits` (**on by default**), `user-rate-limit`, `cooldown`, 3 timeouts | none in frontmatter | `maxTurns` / `steps` |
| **Triggers** | `automation.md`: 10 providers, ~60 events, AND-matchers, inline UTC cron | plus `stage:` to re-enter mid-graph | not in the file (CLI / scheduler) | the richest: slash/label commands, fuzzy cron, roles, cooldown, `stop-after` | not in the file | none |
| Typed step I/O | no | `inputs`/`outputs` as **prose** | `output_format` (JSON Schema) + `output_type` | `safe-outputs.data` schema | `outputKind` / `priorOutputKinds` | no |
| Declared success | no | no | `returns` + `outcome_field` | `evals` (English questions) | `outputKind` + validator | no |
| Effects allowlist | secrets/MCP per agent | same | `allowed_tools`/`denied_tools`, `mutates_checkout` | **`safe-outputs`** (101 keys, per-handler policy) | `opensPullRequest`, `requiresCommit` | `tools`/`permission` |
| Merge by the engine | **never** | never | **never** ("the human gate is… on the forge, outside the run") | `merge-pull-request` under declared gates | `12-merge-execution` step | n/a |

### 5.2 Format mechanics

| | **Warp** | **Archon** | **gh-aw** | **Anneal** |
|---|---|---|---|---|
| Schema of record | published JSON Schema (11 docs), **generated from the parser** | Zod in `packages/workflows/src/schemas/` | one 890 KB JSON Schema in the CLI | TypeScript types + a 15-rule validator |
| Version field | `schemaVersion: v1alpha1`, one version, registry endpoint | **none** (`deprecated:` + scope shadowing) | schema `1.0.0`, lock `schema_version: v3` + `frontmatter_hash` | immutable-once-used + registered generation rollovers |
| Unknown keys | **rejected** (`additionalProperties: false`) | **warned and dropped** | **rejected** | n/a |
| Misplaced keys | rejected | **hard load error**, with remediation text | rejected | n/a |
| Where validated | **server-side only, deliberately no local fallback** | `archon validate workflows` (3 levels) | `gh aw compile --validate` (+ zizmor/actionlint/poutine, always on) | API on authoring; drift test vs. files |
| Testable without an agent | no | **yes** — `--dry-run --stubs`, `workflow test` over `fixtures/` | `--dry-run` forces staged+strict | validator only |
| Atomicity | "applied as a single unit"; last valid definition stays | per-file | per-compile | one atomic `PUT …/steps` |
| Override layering | inheritance; collections **replace not merge**; sparse `harness` | bundled < global < project by **filename**; config; flags; `@alias`; **sidecar policy `.md` that adds to a default** | frontmatter < CLI; per-block **merge table**; repo vars; `gh aw edit --set` | clone-then-edit |
| Safe default | — | `when:` fail-closed | **`strict: true`**; read-only agent job; cost cap on | approval gate validated against parallel layers |

### 5.3 The three models, named

It is worth separating these, because #877 has to pick one (or a blend):

1. **Prompt-orchestrated** (Warp as shipped, Claude Code, OpenCode, droids, Factory). The file declares the *cast*; an orchestrator prompt decides the *script*. Cheapest to build, most flexible, least inspectable. No cap or gate can be enforced, only requested.
2. **Graph-declared** (Warp `blueprint`, Archon, Anneal). The file declares stages and edges; the engine owns ordering, joining, retrying and pausing. Archon's constitution states the dividing line as "**YAML coordinates. Code computes. Agents judge.**" Costs a language, buys enforceable caps, static checks, and dry-run testability.
3. **Envelope-declared** (gh-aw). The file declares the *capability budget* — permissions, network, tools, caps, and the exact set of writes allowed — and lets one agent session choose its own path inside it. Cheapest safety for the least language. Says nothing about multi-session order.

thirdshift today is none of these: it is **engine-declared**, with the script in Rust. Each of the three is a different direction out of that.

## 6. Implications for thirdshift — options, not decisions

These are options with their costs, for the Day shift and for Round 2 of [the #877 grill](../grill-with-docs/877-composable-by-file.md). Nothing here is a recommendation about what to build.

### 6.1 Where thirdshift stands

Five fixed Commands in Rust (`src/run.rs`, `src/spec_run.rs`, `src/architect.rs`, `src/pickup.rs`, `src/security.rs`; see GLOSSARY **Command**), about twenty Session prompts as Rust functions in `src/prompt.rs`, eleven Factory skills in `skills/` compiled in by `include_dir!` ([ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md)), one per-machine `~/.thirdshift/config.toml` of flat settings (`src/config.rs`), and no per-repository definition at all.

Two existing facts make a file-defined process cheaper than it looks:

- **The skills are already SKILL.md files on disk** with `name`/`description` frontmatter, matching the convention all five tools here use. Only the embedding makes them fixed, and ADR-0001 already lists the cost: "Editing a skill has no effect until the binary is rebuilt and reinstalled".
- **`prompts/` already renders one Markdown file per Session prompt**, generated from `src/prompt.rs` by `src/prompts_page.rs`. The files exist with the right granularity and names; they are just outputs rather than inputs. Inverting that direction is a smaller change than designing a format from scratch.

### 6.2 The scope options

These map onto the grill's Q1.1 (a)/(b)/(c). What this research adds is what each costs in practice.

**Option A — externalise prompts and skills only (#118).** A repository or machine file replaces the prompt or skill a named session uses, and perhaps its Harness, Model and Effort. No new sequence.

- *Closest precedents:* Archon's sidecar policy documents, and gh-aw's live-editable body.
- *The precedent worth copying is Archon's, not a plain override.* A full prompt fork drifts from the shipped one forever. Archon's rule instead is: "A node reads the default first and then the project's own file, whose lines add to the default and win where the two conflict… To change one, write only the lines to add or override", with anything that must be exact kept as typed config. That gives #118 its answer without creating twenty forked prompts to maintain.
- *Cost:* small. No graph, no validation beyond "does this file exist and parse".
- *Open question it leaves:* nothing new can be composed, which is what #877's title asks for.

**Option B — a named process over a fixed catalogue of steps.** A file names a process and lists steps thirdshift ships, plus what happens between them. Steps that carry the safety rules stay whole.

- *Closest precedent:* Anneal, almost exactly. A step declares `agent`, `layer`, `approvalGate`, `optional`, `outputKind`/`priorOutputKinds`, and the engine refuses a graph that violates fifteen named rules. Warp's `blueprint` is the same shape with prose conditions instead of typed outputs.
- *What to take from Anneal:* `approvalGate` as a field rather than a sentence; `layer` for parallelism (thirdshift's Spec run already parallelises Tickets, [ADR-0006](../adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md)); `baseFromStepIndex` for pinning a review's diff base, which thirdshift already does for its Code review but in code; and the typed `outputKind` contract, which is what lets a validator catch "this step consumes a Spec that nothing produces" before a run starts.
- *What to take from Archon:* `max_iterations` being **required** on every loop, and the comment that the engine cap *fails* the node while the pack self-caps one below it "so the cap ends typed instead of failing". thirdshift's Repair budget is the same mechanism, so a declared loop would need the same two-level treatment.
- *Cost:* a step catalogue, a graph validator, and a decision about every Command whose behaviour is currently implicit. Medium.
- *Open questions:* whether the five built-in Commands get rewritten as shipped definitions (the grill's Round 2 question), and whether a Self-merge step can appear in a user-authored process at all.

**Option C — everything in the file.** The sessions, the gates, the repair rounds and the merge rules all declared.

- *Nobody surveyed does this.* Warp fixes merge entirely ("The factory never merges on your behalf"), Archon fixes it by doctrine ("the human gate is PR review and merge on the forge, outside the run"), and gh-aw is the only one that lets a file cause a merge — behind `allowed-branches`, `allowed-labels`, `required-labels` and `required-title-prefix`.
- *If this is ever revisited*, gh-aw's `safe-outputs` is the model: not "the file declares the merge step", but "the file declares that this process may merge, and under exactly these conditions". That keeps ADR-0004's checks inside the engine while making the *permission* declarative.
- *Cost:* high, and it exposes rules that took ADRs 0004, 0008, 0010, 0014, 0015 and 0020 to settle.

**Option D — an envelope, no sequence (the one the grill doesn't list).** Keep the five Commands in Rust, and let a file declare only the budget and the allowed effects: Harness/Model/Effort per session kind, Repair cap, Claim limit, protected paths, whether Self-merge is permitted, which labels may be applied.

- *Closest precedent:* gh-aw entirely.
- *This is the cheapest thing that is not just #118*, and it would absorb several open follow-ups at once: protected paths that hold a Self-merge ([open-source-factories.md](open-source-factories.md) §4.2), per-command Self-merge settings (#966), and moving today's per-machine settings into the repository where they can be reviewed.
- *It does not make the process composable*, so it is a complement to A or B rather than an alternative.

### 6.3 Questions the comparison settles, and ones it doesn't

**Settled, in the sense that every tool surveyed agrees:**

- **The per-step agent belongs in its own file**, Markdown with frontmatter, model in the frontmatter and prompt in the body. Five of five do it this way. If thirdshift externalises anything, this is the uncontroversial shape.
- **Collections need an explicit merge rule.** Warp says lists "replace, not merge"; gh-aw publishes a per-block table. Leaving this implicit is where formats become confusing.
- **State does not go in the definition.** Warp: "Work items, runs, and metrics are stored in the web app, not in the definition."
- **Read the process from a committed, reviewed source, not from the working tree.** Warp applies on merge to a production branch; the grill's Q1.2 already reaches the same answer via [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md).

**Genuinely contested, so thirdshift has a real choice:**

- **Prose conditions or typed ones.** Warp's `blueprint` edges are prose the agent judges; Archon's are a tiny grammar over declared `output_format`. The typed route needs every step to declare its output shape — which is a real cost, and also the thing that makes a process testable without an agent.
- **Unknown keys: reject or warn.** Warp and gh-aw reject; Archon warns and drops, with a per-run `workflow_parse_warnings` event. For a format only Jacob uses (the grill's Q1.3 recommendation), rejecting is cheaper to reason about; warning is kinder to forward compatibility.
- **Where validation lives.** Warp's validator refuses to parse YAML locally at all, for a reason worth quoting in full before thirdshift writes one: "a stale copy does not degrade gracefully: it reports confident, wrong diagnostics that invite an agent to 'repair' valid configuration by deleting it. Reporting that a tree could not be checked is strictly safer than reporting the wrong answer." thirdshift has no server, so its equivalent is that the binary that *runs* a definition is the only thing that validates it — which falls out naturally from a single binary, and argues against any separate linter.
- **Triggers in the file or not.** Warp and gh-aw both put them there. The grill's Q1.4 recommends (a), no triggers, on [ADR-0009](../adr/0009-the-operating-system-schedules-thirdshift.md) grounds, and nothing in this research undermines that: Warp needs triggers in the file because the engine is a SaaS control plane with no crontab to read, and gh-aw because GitHub Actions *is* the trigger mechanism. thirdshift has a scheduler already. Note though that `src/cronjobs.rs` reads crontab entries "without changing its crontab", so a file could *document* the schedule a pass expects and `thirdshift status` could report drift, without thirdshift installing anything.

**Worth separate issues regardless of which option wins:**

- **Fixture-based testing of a declared process.** `archon workflow test <name>` "replays the fixtures in the workflow's `fixtures/` folder and checks each declared outcome", with ~40 `*.stubs.yaml` in the shipped pack. For thirdshift this is the difference between a definition format you can trust and one you can only find bugs in at 3am.
- **A declared success predicate.** Archon's `outcome_field` records "the workflow author's verdict independently from the run lifecycle", and "the engine maps exact true/false values to succeeded/failed without inference". thirdshift's Ending enum (`src/command.rs`) already distinguishes outcomes per Command kind in code; a declared process would need the same distinction in the file.
- **A capability allowlist for writes**, per gh-aw `safe-outputs` and §6.2 Option D.
- **Retiring a field with a load error, not silence.** Archon's retired `interactive:` emits an error naming the replacement and how to express the old intent. thirdshift's glossary already does this for renamed terms ([ADR-0022](../adr/0022-ticket-runs-and-plain-schedules.md)); a file format would need the same discipline.

### 6.4 Conflicts to be aware of

| thirdshift decision | What a file-defined process would press on |
|---|---|
| [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md) skills embedded in the binary | Options A–C all want skills and prompts read from disk. The ADR's own stated cost ("Editing a skill has no effect until the binary is rebuilt") is the thing #118 wants gone, so this is the ADR most directly in tension |
| [ADR-0004](../adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md) Self-merge | Warp and Archon both refuse to merge at all. Only gh-aw's gated `merge-pull-request` is a precedent for declaring it |
| [ADR-0009](../adr/0009-the-operating-system-schedules-thirdshift.md) the OS schedules thirdshift | Warp and gh-aw both declare triggers in the file. Partly revisited by Q1.4 option (b), reversed by (c) |
| [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md) sessions run fully trusted | A definition read from the worktree could be rewritten by the session running it; reading from the Base branch is the mitigation, and is what Warp does by applying only on merge to a production branch |
| [ADR-0022](../adr/0022-ticket-runs-and-plain-schedules.md) stable stored kinds | A file that names processes creates a second naming surface alongside the stored `Run` kind and the glossary's display names |

## 7. Uncertainties

- **Warp's `blueprint` is undocumented.** It is in the live `v1alpha1` schema registry, flagged "Not yet available on every deployment", and absent from both the docs page and the examples repo. Whether any user-facing documentation, example or changelog entry exists is **[unverified]**. Everything in §1.5 is quoted from the schema itself. It may also change without a `schemaVersion` bump, since it is already inside `v1alpha1`.
- **Warp's own factory definition is not public.** No `factory.yaml` exists in `warpdotdev/warp`, so the dogfooded factory is presumably Warp-managed. **[interpretation]**
- **Anneal's `spawnPolicy`** is a JSON object whose contents I did not establish. Its caps and retry handling outside the step frontmatter were not exhaustively searched. **[unverified]**
- **Anneal's user-facing authoring path** — whether a user can add a workflow directory under `agents/templates/` or must clone a template through the API — is **[interpretation]** from `CANONICAL_TEMPLATE_SOURCE_SPECS` plus the drift test, not from a stated rule.
- **Archon's executors** were not read line by line. Every "fixed in the engine" claim in §2.9 cites a schema file, the loader, or the reference doc; the retry delay schedules and the `max_iterations`/terminal-wait interaction come from `workflow-language.md` prose rather than from the implementing code. **[unverified]**
- **Archon's `main` branch lags `dev` badly** (a week-old Homebrew commit at the time of checking). Anything pinned to `main` may describe a much older format. The `archon-loop.yaml` paths cited in [open-source-factories.md](open-source-factories.md) §2.4 no longer exist on `dev`.
- **gh-aw's `strict_mode_validation.go`** was not read; the strict-mode enforcement list is quoted from the schema's own `description` and `$comment`, which name that file. **[unverified]**
- **gh-aw's `agy` ("antigravity") engine** and the apparent Gemini deprecation: the doc page and ADR filename exist but their bodies were not read, and `$defs/engine_config` still lists only `claude, codex, copilot, gemini, pi` as built-ins. **[unverified]**
- **Whether `safe-outputs.threat-detection` is on by default** when omitted is **[unverified]**; the schema gives no default, and the repo's own workflows always set it explicitly.
- **Nothing was run.** No factory was created, no workflow compiled, no template instantiated. Every claim about behaviour is from a schema, a source file or a doc, not from observation. **[needs live check]** applies to anything where the parser and the docs might disagree — which is precisely the failure mode Warp's validator script is designed around.
- **A note on one source.** The gh-aw research pass tripped an automated instruction-pattern filter, because `engine.permission-mode` has a legitimate enum value named `bypassPermissions` (its documented default for unattended runs is `dontAsk`). That is a field name in a published JSON Schema, quoted here as data; no instruction was followed from any fetched page.
