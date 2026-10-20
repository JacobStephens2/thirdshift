# Sandcastle as a replacement for thirdshift, or a primitive under it

Research date: 2026-10-16.

**Question.** How well would Sandcastle (`mattpocock/sandcastle`) fit as a replacement for thirdshift: fully, as a primitive that thirdshift sits on top of, as a source of ideas to borrow, or not at all? This builds on [warp-factories-and-alternatives.md](warp-factories-and-alternatives.md) (the "earlier survey") and asks what [openhands-fit.md](openhands-fit.md) asked of OpenHands. It reuses the earlier survey's §1 list of what a replacement must cover and its §2 reading of Anthropic's and OpenAI's subscription terms. The earlier survey did not cover Sandcastle. [naming.md](naming.md) mentions the same project ("Matt ships his own agent orchestrator") only to explain why thirdshift's name leaves out "Pocock". The criteria, in order:

1. **Subscription auth.** Can it run on Claude Pro/Max (the `claude` CLI) and ChatGPT (the `codex` CLI) rather than per-token API keys, while staying within both vendors' terms?
2. **Open source and self-hosted**, preferred.
3. **Buy over build**, preferred.

**Method.**

- Source code was read from a shallow clone made on 2026-10-16: `mattpocock/sandcastle@7a4e7f7` (`7a4e7f7d93056600d979d79a090256feddf653ea`, committed 2026-10-08). `gh api repos/mattpocock/sandcastle/commits?sha=main` shows it is still the head of `main`. Code citations link to files at that commit.
- Repository metadata, releases, contributors, issues and pull requests come from `gh api` on 2026-10-16. Package facts come from `npm view @ai-hero/sandcastle` and npm's [download-count API](https://api.npmjs.org/downloads/point/last-week/@ai-hero/sandcastle).
- Sandcastle has no hosted docs. Its `homepage` is empty in `gh api`, and its `docs/` site holds three short pages. The README is the reference. It is cited at the pinned commit, which is the text [github.com/mattpocock/sandcastle](https://github.com/mattpocock/sandcastle) shows today.
- The author's comments on Sandcastle issues and the [aihero.dev skills changelog](https://www.aihero.dev/skills-changelog-v13-implement-spec-pr-retro-and-glossary-md) count as the author's statement of intent. Behaviour is confirmed in the code.
- Vendor terms come from the earlier survey's §2 and from openhands-fit.md §2.4, which re-fetched them today. Only Claude Code's [authentication](https://code.claude.com/docs/en/authentication) page was fetched again, because Sandcastle's container path rests on `CLAUDE_CODE_OAUTH_TOKEN`.
- Host facts come from read-only commands on 2026-10-16: `hostname`, `which docker podman`, `nproc`, `free -h`, `swapon --show`, `sysctl kernel.apparmor_restrict_unprivileged_userns` and `bwrap --unshare-all --ro-bind / / true`.
- **[unverified]** marks a claim I could not confirm from a primary source. **[3rd-party]** marks a claim from a secondary source; none is relied on here. **[interpretation]** marks my reading of the terms or the code. **[needs live check]** marks something only a real run would settle.
- Nothing from the clone was installed, built or run.

**Out of scope.** Model quality; the full design of any migration; Sandcastle's interactive mode and its Windows and macOS paths; the Vercel AI SDK harness the author is charting a move onto (§1.3).

## TL;DR

- **Sandcastle is a library that runs one agent session in a sandbox. It is not a factory.** It is an MIT TypeScript package, `@ai-hero/sandcastle` 0.12.0, by Matt Pocock.
  - `run()` starts the vendor's own CLI headless, in Docker, Podman, a Vercel or Daytona VM, or on the host (`noSandbox()`), usually on a branch in a git worktree. Then it brings the commits back.
  - Its README says "no opinions about workflow, task management, or context sources are imposed". Issue pickup, review and merging live in prompt templates and in the script you write.
  - The author names it as an example of "the deterministic loop", the way the author recommends most people work through tickets. thirdshift is a loop of the same kind. (§1)
- **Subscription auth: yes for Claude, and for Codex only on the host. Both go through the unmodified official CLIs.**
  - **Claude** always runs as `claude --print … -p -`. In a container it uses your own `claude setup-token` token in `CLAUDE_CODE_OAUTH_TOKEN`, which `sandcastle init` scaffolds. With `noSandbox()` it uses the host's own login.
  - **Codex** runs as `codex exec --dangerously-bypass-approvals-and-sandbox`. With `noSandbox()` it uses the host's `~/.codex/auth.json`. In a container you mount a login yourself. "Figure out Codex + Subscription" is still an open issue, and init scaffolds `OPENAI_KEY`, which `codex exec` never reads.
  - **On the terms** both are route (a): the vendor's unmodified CLI under your own login, on your own machine, and through `claude -p` rather than the Agent SDK. That is the plainest case in the earlier survey, and the same as thirdshift **[interpretation]**. (§2)
- **Its local isolation needs Docker or Podman, and the factory host has neither.**
  - `noSandbox()` has thirdshift's trust model, with three regressions. `run()` passes `--dangerously-skip-permissions` unless you set `permissionMode: "auto"`. Every run writes the host's global git config. And nothing kills the agent's process tree on teardown.
  - The container path has a confirmed, open hole: through the shared `.git`, the agent can run code on the host.
  - The remote providers drop the prompt for Claude and Codex.
  - There is no bwrap provider. (§3)
- **As a replacement it covers little of the earlier survey's §1.**
  - **What it covers:** the worktree, the headless session, resume, and a free choice of agent per step.
  - **What it lacks:** Claims, PR handling, the Repair loop, Inherited failures, the Check re-run, the Self-merge, Spec runs, scheduled Passes, Security work and notifications.
  - **How its templates finish:** an agent merges the branches locally and closes the issues.
  - **The author's own factory** for the repository is GitHub Actions YAML around the library, plus a private daemon. (§4)
- **The conventions overlap in the repository, not in the product.**
  - **What matches:** Sandcastle's own repository uses the same mattpocock/skills setup and five triage labels as thirdshift, and the author's own loop picks `ready-for-agent` issues.
  - **What doesn't:** `sandcastle init` filters on a `Sandcastle` label, and no shipped prompt calls a skill. A `/skill` prompt doesn't expand through the Claude provider either, which is an open bug. (§5)
- **Maturity: popular, pre-1.0, and the library is idle.**
  - **Reach:** 8,480 stars and about 95,000 npm downloads a week.
  - **Activity:** nothing under `src/` has changed since v0.12.0 on 2026-06-29, and 108 issues and 70 PRs are open.
  - **Direction:** the author has an open plan to rebuild execution on the Vercel AI SDK harness. (§1.3)
- **Recommendation: borrow ideas only. It is not a replacement, and not a primitive now.** Putting it under thirdshift would add Node, a container runtime and a pre-1.0 dependency, in return for container isolation this host can't run. Things worth borrowing:
  - The `agent:explore` split between an agent that holds no credentials and a deterministic step that validates and applies its plan. It fits thirdshift's open #691 and #637.
  - The bind-mount provider seam, with the lesson of Sandcastle's `.git` hole, if thirdshift adds an opt-in sandboxed Run (#398). A bwrap provider would fit this host.
  - Schema-checked structured output, with a bounded Resume to repair it.

  Revisit when a release fixes the open bugs above and the AI SDK work settles what Sandcastle is. (§7)

## 1. What Sandcastle is

### 1.1 Purpose, architecture and how it is driven

| Aspect | Sandcastle | Source |
|---|---|---|
| **Purpose** | "A TypeScript library for orchestrating AI coding agents in isolated sandboxes": `sandcastle.run()` invokes the agent, "Sandcastle handles sandboxing the agent with a configurable branch strategy", and "The commits made on the branches get merged back". "Great for parallelizing multiple AFK agents, creating review pipelines, or even just orchestrating your own agents" | [README L9-L17](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L9-L17) |
| **What it leaves to you** | "You write the prompt, and the engine executes it — no opinions about workflow, task management, or context sources are imposed" | [README L560-L562](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L560-L562) |
| **Unit of work** | An **iteration**: "A single invocation of the agent inside the sandbox, producing at most one commit against one task". `maxIterations` defaults to 1, and `<promise>COMPLETE</promise>` in the output ends the loop early | [CONTEXT.md L85-L95](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/CONTEXT.md#L85-L95); [README L186-L187](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L186-L187), [L643-L647](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L643-L647) |
| **Branch strategies** | `head` writes in the host checkout, and is the default for bind-mount providers. `merge-to-head` uses a temporary branch merged back into HEAD. `branch` uses a named branch in a worktree under `.sandcastle/worktrees/` | [README L548-L558](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L548-L558); [WorktreeManager.ts L277-L297](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/WorktreeManager.ts#L277-L297) |
| **How it's driven** | A library API (`run()`, `createSandbox()`, `createWorktree()`, `interactive()`) called from your own TypeScript script, run with `npx tsx .sandcastle/main.ts`. The CLI only scaffolds (`sandcastle init`) and builds or removes images | [README L28-L64](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L28-L64), [L764-L831](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L764-L831); [src/index.ts](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/index.ts) |
| **Configuration** | A per-repository `.sandcastle/` directory: a Dockerfile, `main.ts`, prompt files and a `.env` of tokens | [README L786-L797](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L786-L797) |
| **Runtime and language** | TypeScript on Node, bundled with tsup and built on Effect. It has one runtime dependency (`@clack/prompts`); `@vercel/sandbox` and `@daytona/sdk` are optional peers | [package.json L65-L99](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/package.json#L65-L99) |
| **Licence** | MIT, "Copyright (c) 2026 Matt Pocock" | [LICENSE L1-L3](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/LICENSE#L1-L3) |

### 1.2 What ships, and what the author runs

- **Sandbox providers.** Docker and Podman (bind-mount), Vercel and Daytona (isolated), and `noSandbox()` ([README L66-L75](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L66-L75); [package.json L8-L33](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/package.json#L8-L33)). The list is "deliberately curated". Any other backend is a `SandboxProvider` the user writes ([built-in-sandbox-providers.md L3-L9](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.out-of-scope/built-in-sandbox-providers.md#L3-L9)).
- **Agent providers.** `claudeCode`, `codex`, `pi`, `cursor`, `opencode` and `copilot` ([InitService.ts L409-L476](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L409-L476)), also "deliberately curated" ([built-in-agent-providers.md L3-L11](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.out-of-scope/built-in-agent-providers.md#L3-L11)).
- **Templates.** `blank`, `simple-loop`, `sequential-reviewer`, `parallel-planner` and `parallel-planner-with-review` ([README L750-L760](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L750-L760)). They are kept "deliberately minimal, framework-agnostic starting points" ([bundled-workflow-templates.md L7](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.out-of-scope/bundled-workflow-templates.md#L7)).
- **Not shipped: the author's own factory.** Three pieces live in the repository but not in the package:
  - **GitHub Actions workflows.** Five of them, triggered by four labels: `agent:explore` on issues, `agent:implement` on issues or PRs, and `agent:review` and `agent:update-branch` on PRs. All five run the library with `noSandbox()` on GitHub-hosted runners ([implement.ts L15-L27](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/agent-workflows/implement/implement.ts#L15-L27)). The pushing, PR opening and labelling happen in YAML ([agent-implement.yml L97-L211](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-implement.yml#L97-L211)).
  - **A local loop.** `.sandcastle/run.ts` plans, implements, reviews and merges in Docker ([run.ts](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/run.ts)).
  - **A daemon entry point.** `.factory/` is the entry point for a separate "factory daemon" built from `~/repos/ai/software-factory`. The daemon "spawns this script once per task", and commits on its branch "will be pushed and opened as a PR automatically" ([implement-task.ts L1-L20](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.factory/implement-task.ts#L1-L20); [run-daemon.sh](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.factory/run-daemon.sh)). No public repository for that daemon turned up under `mattpocock` **[unverified]**.
- **How the author positions it.** The author's skills changelog lists three ways to work through tickets ([aihero.dev](https://www.aihero.dev/skills-changelog-v13-implement-spec-pr-retro-and-glossary-md), updated 2026-10-05):
  - The second is "The deterministic loop. A script reads each ticket and runs /implement on it. This is what I recommend for most people. … But it takes patience to set up and tune. Sandcastle is one."
  - The third is the agent loop, `/implement-spec`, which "is worse than a deterministic loop".

  thirdshift made the same call in [ADR-0006](../adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md), which rejected "An agent driving the whole Spec, as the `implement-spec` skill does".

### 1.3 Maturity

- **Age and reach.**
  - **Repository:** created 2026-03-17, with 8,480 stars and 904 forks (`gh api`). That is up from 8,135 stars on 2026-09-24 ([naming.md](naming.md)).
  - **npm:** 94,565 downloads from 2026-10-06 to 2026-10-12, and 458,482 in the 30 days to 2026-10-12 ([npm downloads API](https://api.npmjs.org/downloads/point/last-week/@ai-hero/sandcastle)).
- **Releases.**
  - **GitHub:** 43 releases, from v0.1.0 on 2026-03-26 to v0.12.0 on 2026-06-29, and none since (`gh api …/releases`).
  - **npm:** `latest` is 0.12.0, published 2026-06-29 (`npm view`).
  - **Pending:** a "Version Packages" PR for a README-only 0.12.1 has been open since 2026-10-07 ([#1034](https://github.com/mattpocock/sandcastle/pull/1034)).
- **Activity.**
  - **Commits on `main` by month:** 372 in March, 520 in April, 240 in May and 61 in June. None from July to September, then 16 in October (`gh api …/commits`).
  - **Since v0.12.0:** those 16 commits touch the author's own workflows, the README and one skill, and nothing under `src/` (`gh api …/compare/v0.12.0...main`).
  - **Resuming?** Two new PRs from the author on 2026-10-16 suggest work is starting again ([#1044](https://github.com/mattpocock/sandcastle/pull/1044), [#1045](https://github.com/mattpocock/sandcastle/pull/1045)).
- **Backlog.** 108 open issues and 70 open PRs. Since 2026-07-01, 105 issues were opened and 41 closed, and 64 PRs were opened and 8 merged (`gh api search/issues`).
- **People.** There are 11 contributors. The author has 1,038 commits; the next human has 4 (`gh api …/contributors`).
- **Stability.** It is pre-1.0: "Make all bugfixes `patch`, all new features or breaking changes `minor` (since we're pre-1.0)" ([AGENTS.md L5](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/AGENTS.md#L5)). CI runs only on pushes to `main`, not on PRs ([ci.yml L3-L6](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/ci.yml#L3-L6)).
- **Direction.** Two open issues from the author point at large changes:
  - [#884](https://github.com/mattpocock/sandcastle/issues/884), opened 2026-07-08, charts "running Sandcastle's agent execution on the AI SDK harness". It leaves "The broader 'what do we lose by becoming a thin wrapper over the harness' adoption decision" out of scope, as "a larger strategic effort".
  - [#1035](https://github.com/mattpocock/sandcastle/issues/1035), opened 2026-10-07, proposes "Remove the prompt interpolation feature", because "It's a security risk."
- **Verdict.** It is very popular, and has one maintainer. The library has been unchanged for three and a half months, and a rebuild may be ahead. Like OpenHands it is a **moving target**, for a different reason.

## 2. Subscription auth: the concrete answer

### 2.1 How Sandcastle reaches a model

Sandcastle always shells out to the vendor's own CLI, and does no OAuth of its own:

- **Claude Code:** `claude --print --verbose <permission flag> --output-format stream-json --model <m> [--effort <e>] [--resume <id>] -p -`, with the prompt on stdin ([AgentProvider.ts L1190-L1216](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/AgentProvider.ts#L1190-L1216)).
- **Codex:** `codex exec --json --dangerously-bypass-approvals-and-sandbox -m <m>`, with the prompt on stdin ([AgentProvider.ts L782-L814](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/AgentProvider.ts#L782-L814)).

Credentials reach the CLI in one of two ways:

- **As environment variables**, from `.sandcastle/.env`, from the provider's `env` option, or, under `noSandbox()`, from the whole host environment.
- **As files you mount.**

| Path | What it is | Claude Pro/Max | ChatGPT |
|---|---|---|---|
| **A. `noSandbox()`** | The CLI runs on the host, with `process.env` merged in ([no-sandbox.ts L45-L51](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/no-sandbox.ts#L45-L51)). No credential is required: nothing in the code checks for one ([EnvResolver.ts L49-L73](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/EnvResolver.ts#L49-L73)) | **Yes.** The host's own login in `~/.claude/.credentials.json` | **Yes.** The host's `~/.codex/auth.json`, the login thirdshift's Codex sessions use |
| **B. Docker or Podman on the host** | A container built from `.sandcastle/Dockerfile`, which installs Claude Code with the official installer, or Codex from npm ([InitService.ts L228-L229](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L228-L229), [L295-L296](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L295-L296)). Variables are passed as `docker run -e KEY=value` ([DockerLifecycle.ts L126-L170](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/DockerLifecycle.ts#L126-L170)) | **Yes.** Init scaffolds `CLAUDE_CODE_OAUTH_TOKEN`: "get one by running `claude setup-token` on your host. Lets the agent use your Claude subscription instead of an API key" ([InitService.ts L416-L420](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L416-L420); [README L42](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L42)) | **Do it yourself.** Init scaffolds `OPENAI_KEY` ([InitService.ts L438-L440](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L438-L440)), "whose key nothing reads": `codex exec` "takes an API key from the environment only as `CODEX_API_KEY`" ([PR #1044](https://github.com/mattpocock/sandcastle/pull/1044), open). A ChatGPT login means mounting `~/.codex`, or a project-scoped `auth.json`, yourself. Users' workarounds are in [#488](https://github.com/mattpocock/sandcastle/issues/488), "Figure out Codex + Subscription", open and labelled `documentation` |
| **C. Vercel or Daytona** | A remote VM. Code goes in, and commits come out through `git format-patch` and `git am` ([ADR 0017](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0017-sandbox-owned-sync-base-ref.md#L5-L9)) | **Broken today.** Both providers drop stdin, so "claudeCode gets an empty prompt" ([#936](https://github.com/mattpocock/sandcastle/issues/936)). Neither [vercel.ts](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/vercel.ts) nor [daytona.ts](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/daytona.ts) mentions `stdin`; a fix, [PR #989](https://github.com/mattpocock/sandcastle/pull/989), is open | The same, since `codex` reads its prompt from stdin too |
| **D. API keys** | `ANTHROPIC_API_KEY`, or `CODEX_API_KEY` | Per token | Per token |

### 2.2 The Claude token path in detail

- **What Anthropic documents.** `CLAUDE_CODE_OAUTH_TOKEN` is "A long-lived OAuth token generated by `claude setup-token`. Use this for CI pipelines and scripts where browser login isn't available." The token "authenticates with your Claude subscription and requires a Pro, Max, Team, or Enterprise plan" ([authentication](https://code.claude.com/docs/en/authentication), fetched 2026-10-16).
- **A precedence trap.** `ANTHROPIC_API_KEY` ranks above the token, and "In non-interactive mode (`-p`), the key is always used when present" (same page). Init comments the key out ([InitService.ts L419-L420](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L419-L420)). But `noSandbox()` passes the whole host environment through ([no-sandbox.ts L51](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/no-sandbox.ts#L51)), so a key set on the host wins. thirdshift has the same exposure today.
- **Where the token sits.** `.sandcastle/.env` is inside the repository, ignored by a `.gitignore` that init writes ([README L788-L795](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L788-L795)). The Docker provider puts every variable on `docker run`'s command line ([DockerLifecycle.ts L126-L129](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/DockerLifecycle.ts#L126-L129), [L156-L170](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/DockerLifecycle.ts#L156-L170)), so `docker inspect` shows it **[interpretation]**. `noSandbox()` needs neither.
- **The author's own history with it.**
  - On 2026-03-27 the author wrote: "What I can say is that Anthropic publicly documents how to do this in Claude Code itself, via `claude setup-token` and `CLAUDE_CODE_OAUTH_TOKEN`. I can't legally recommend you use it. But you are able to do it."
  - After Anthropic's notices the author closed the issue, on 2026-06-15: "Acquire an oauth token by using `claude setup-token` … Paste it to CLAUDE_CODE_OAUTH_TOKEN in your `.sandcastle/.env` file … And you're good to go" ([#191](https://github.com/mattpocock/sandcastle/issues/191)).
  - The author's own CI runs this way, with the token in a repository secret ([agent-implement.yml L147-L153](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-implement.yml#L147-L153); [common.ts L55-L60](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/agent-workflows/shared/common.ts#L55-L60)).

### 2.3 The other agents

- **Their credentials.** Init scaffolds an API key or token for each ([InitService.ts L423-L475](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L423-L475)):
  - **Pi:** `ANTHROPIC_API_KEY`.
  - **Cursor:** `CURSOR_API_KEY`.
  - **OpenCode:** `OPENCODE_API_KEY`.
  - **GitHub Copilot CLI:** a GitHub token with the "Copilot Requests" permission, so its usage counts against GitHub Copilot rather than Claude or ChatGPT **[interpretation]**.
- **The bar for new ones.** A new provider must offer "Env-based authentication. Auth via environment variables (API key) — no interactive login required at first run" ([adding-an-agent-provider.md L22](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/agents/adding-an-agent-provider.md#L22)). New built-in providers aren't added on request ([built-in-agent-providers.md L3](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.out-of-scope/built-in-agent-providers.md#L3)).
- **Against thirdshift's six Harnesses,** only Claude Code, Codex and OpenCode overlap. There is no Antigravity CLI, Grok Build or Muse Code. The factory's User config on this host sets `harness.default = "codex"` and `security.harness = "muse"` (read on 2026-10-16), so the Codex and Muse gaps are the ones that matter here.

### 2.4 Against the vendors' terms

The earlier survey's §2 citations still hold, as openhands-fit.md §2.4 re-checked today.

| Path | Anthropic | OpenAI |
|---|---|---|
| **A, `noSandbox()`** | Route (a): the unmodified binary under your own login, on your own machine, through `claude -p`. The same as thirdshift (earlier survey §2.1, §2.3) | Route (a): a ChatGPT `auth.json` on "trusted private infrastructure" (earlier survey §2.2). The public-repository caveat there applies equally |
| **B, a container on your own box** | Still (a): the unmodified binary with your own `setup-token` token, which Anthropic documents for "CI pipelines and scripts" | (a) with a mounted `auth.json`, if you set one up; otherwise an API key |
| **C, Vercel or Daytona** | (a′): the legal page allows the unmodified binary "including where a platform hosts Claude Code" (earlier survey §2.3), but your token leaves your machine | "Don't expose Codex execution in untrusted or public environments" (earlier survey §2.2) |
| **D, API key** | Allowed, per token | Allowed, per token |

**Bottom line for criterion 1** **[interpretation]**.

- **The verdict.** Sandcastle on your own box stays on both subscriptions within the terms, at the same risk as thirdshift.
- **Compared with OpenHands.** On Claude it is a cleaner case, because it runs `claude -p` rather than the Agent SDK (openhands-fit.md §2.4).
- **Why a third-party tool still fits.** Sandcastle tells its users to paste in a token, but it neither offers a Claude.ai login nor routes requests on its users' behalf. Each user runs the unmodified binary with their own credentials, which the legal page leaves open (earlier survey §2.1). Sandcastle never takes route (b).
- **The weak spots are practical, not legal.** Codex subscription use in a container is undocumented, and its scaffold is wrong. The remote path is broken.

## 3. Sandbox and runtime requirements

### 3.1 Providers

| Provider | Kind | Needs | Notes |
|---|---|---|---|
| `docker()` | Bind-mount: the worktree and the repository's `.git` are mounted in | Docker, and an image from `sandcastle docker build-image` | Runs `docker run -d … --user <uid>:<gid>`, plus optional `--network`, `--group-add`, `--device` and `--cpus`. There is no memory limit and no capability drop ([DockerLifecycle.ts L131-L170](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/DockerLifecycle.ts#L131-L170)) |
| `podman()` | Bind-mount, rootless, `--userns=keep-id` | Podman 4.1 or later ([ADR 0005](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0005-remove-chown-uid-alignment.md#L5)) | The same options ([podman.ts L201-L232](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/podman.ts#L201-L232)) |
| `vercel()`, `daytona()` | Isolated: their own filesystem, synced through git | A Vercel or Daytona account and token | Broken for Claude and Codex ([#936](https://github.com/mattpocock/sandcastle/issues/936)) |
| `noSandbox()` | None | Nothing | "opt in to running the agent directly on the host when container isolation is undesired" ([README L68](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L68)) |

- **Containers are the default.** The README says "Sandcastle needs an isolated environment to run agents in" ([README L19-L26](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L19-L26)). `run()` has accepted `noSandbox()` only since [ADR 0015](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0015-no-sandbox-in-run-and-create-sandbox.md#L5-L11). Before that, it says, "subscription-billed Claude users (and anyone running Sandcastle inside an already-isolated environment …) had no path to AFK orchestration".
- **No bwrap.** The author's sandbox survey files bubblewrap under "Future considerations": "'Fast local mode' for Linux users who don't want Docker. Near-zero overhead. No image ecosystem though — agent tools must be installed on the host" ([sandbox-provider-research.md L200-L213](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/research/sandbox-provider-research.md#L200-L213), [L576](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/research/sandbox-provider-research.md#L576)). Built-in providers aren't added on request. A user can write one with `createBindMountSandboxProvider` ([README L1025-L1156](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L1025-L1156)).

### 3.2 On the factory host

- **What the host has** (2026-10-16):
  - `hostname` prints `thirdshift`. It has 4 cores (`nproc`) and 7.8 GiB of RAM (`free -h`).
  - `which docker podman` finds neither. The `lxc` and `lxd` in `/usr/sbin` are Ubuntu's snap-installer stubs.
  - `/usr/bin/bwrap` exists. `bwrap --unshare-all --ro-bind / / true` succeeds under `kernel.apparmor_restrict_unprivileged_userns = 1`, so the `/etc/apparmor.d/bwrap` profile is in force.
  - Node is v26.10.0, under nvm.
- **The memory notes, checked.**
  - **What they say.** The capacity note (2026-09-30) describes 4 cores, about 8 GB, no swap and about two building Runs. The sandbox note describes bwrap plus the AppArmor profile on "the `thirdshift` host".
  - **Which machine.** This machine matches both, so I take it to be the factory host that the notes call tracewake **[interpretation]**.
  - **What is stale.** The host now has 4 GiB of swap: a `/swapfile` created on 2026-10-04 (`swapon --show`; `ls -l /swapfile`).
- **With `noSandbox()`,** Sandcastle adds a Node process to the same `claude` or `codex` processes and builds that thirdshift runs today, so the load still comes from builds and tests. Its extra memory is not measured **[unverified]**. Node would also have to be on cron's `PATH`; here it lives under nvm.
- **With Docker or Podman,** the host first needs a container runtime. Then:
  - Each sandbox is a long-lived container (`ENTRYPOINT ["sleep", "infinity"]`) built on `node:22-bookworm` ([InitService.ts L207-L240](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L207-L240)). A target repository's toolchains have to be baked into a per-repository image.
  - Caches start cold unless you mount or copy them in. The templates copy the host's `node_modules` into every worktree and run `npm install` in every sandbox ([main.mts L46-L55](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/parallel-planner-with-review/main.mts#L46-L55)).
  - With no memory limit, two containers can still exhaust 7.8 GiB **[interpretation]**.
  - Concurrency is the caller's job. The shipped parallel template starts every unblocked issue at once ([main.mts L114-L162](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/parallel-planner-with-review/main.mts#L114-L162)). The author's own loop allows four at a time ([run.ts L5](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/run.ts#L5), [L41-L55](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/run.ts#L41-L55)), twice this host's ceiling.
- **The idle timeout.**
  - **The default.** `idleTimeoutSeconds` defaults to 600, and a run fails when the agent prints nothing for that long ([README L240-L241](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L240-L241); [Orchestrator.ts L104-L117](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/Orchestrator.ts#L104-L117)). It is configurable.
  - **Why it matters here.** A full `cargo test` took 15–16 minutes on this host under load (memory note).
  - **What is open.** Whether Claude's stream stays silent through a long foreground command is a **[needs live check]**.

### 3.3 Isolation compared with thirdshift's worktrees plus bwrap

| | thirdshift today | Sandcastle, `noSandbox()` | Sandcastle, Docker or Podman |
|---|---|---|---|
| **Boundary** | The worktree. Sessions run "with the full permissions of that user account" ([README](../../README.md) "Unattended sessions"; [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md)) | The same, with the worktree under `.sandcastle/worktrees/` | A container that shares the host kernel ([#682](https://github.com/mattpocock/sandcastle/issues/682)). The worktree and the host `.git` are mounted read-write |
| **What the agent can reach** | Everything the account can, the `gh` login included | The same | The mounted worktree and `.git`, the variables in `.sandcastle/.env`, any extra mounts, and the network. The `.env` holds, for example, `CLAUDE_CODE_OAUTH_TOKEN` and a `GH_TOKEN` with issue write access ([InitService.ts L540-L543](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L540-L543)) |
| **Claude's permissions** | `claude -p --permission-mode auto` ([claude.rs L90-L103](../../src/harness/claude.rs#L90-L103)) | `run()` passes `dangerouslySkipPermissions: true` whatever the provider ([Orchestrator.ts L140-L146](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/Orchestrator.ts#L140-L146)). So Claude gets `--dangerously-skip-permissions` unless `permissionMode: "auto"` is set ([AgentProvider.ts L1196-L1203](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/AgentProvider.ts#L1196-L1203)). ADR 0015 says the flag is not passed under `noSandbox()` ([L25](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0015-no-sandbox-in-run-and-create-sandbox.md#L25)); the code says it is **[needs live check]** | `--dangerously-skip-permissions`, which the provider bar requires ([adding-an-agent-provider.md L20](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/agents/adding-an-agent-provider.md#L20)) |
| **Codex** | `--dangerously-bypass-approvals-and-sandbox` (ADR-0013) | The same ([AgentProvider.ts L794-L797](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/AgentProvider.ts#L794-L797)) | The same |
| **Side effects on the host** | Its own worktree and Issue branch, and `.git/info/exclude` entries ([README](../../README.md) "What sessions leave behind") | `git config --global` adds a `safe.directory` entry, and sets `user.name` and `user.email`, on every run ([SandboxLifecycle.ts L232-L256](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/SandboxLifecycle.ts#L232-L256)). The `safe.directory` entries pile up ([#846](https://github.com/mattpocock/sandcastle/issues/846)), and concurrent runs collide on `~/.gitconfig.lock` ([#917](https://github.com/mattpocock/sandcastle/issues/917)) | The same commands, run inside the container |
| **Interruption** | Stops every session's process tree ([README](../../README.md) "Harness, Model and Effort") | Nothing kills the agent: "there is no `proc.kill()` anywhere in the codebase" ([#766](https://github.com/mattpocock/sandcastle/issues/766)) | `docker rm -f` on teardown ([docker.ts L233-L240](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/docker.ts#L233-L240)) |
| **Known holes** | None comparable | None comparable | Code the agent plants runs on the host the next time Sandcastle calls git: a `.git/hooks` file, a `core.fsmonitor` setting, or a rewritten worktree `.git` file. The repository's triage bot calls it a "confirmed trust-boundary vulnerability" and says "partial read-only mounts are provably bypassable" ([#1010](https://github.com/mattpocock/sandcastle/issues/1010); a partial fix, [PR #1014](https://github.com/mattpocock/sandcastle/pull/1014), is open). There are no container hardening flags ([#682](https://github.com/mattpocock/sandcastle/issues/682); [PR #1015](https://github.com/mattpocock/sandcastle/pull/1015) is open) |
| **Running target code in Security reviews** | bwrap with the AppArmor profile (memory note `tracewake-security-sandbox`) | Nothing | The container |

**[interpretation]** So:

- A container is a stronger boundary for a Run than thirdshift's worktree, apart from the `.git` hole.
- `noSandbox()` is weaker than thirdshift's Claude setup, by default.
- Neither has anything like the bwrap sandbox that thirdshift's Security reviews use.

## 4. Coverage of the earlier survey's §1 checklist

"Sandcastle" here means the library and the templates it ships. The author's own, unshipped workflows are noted where they reach further.

| §1 capability | Sandcastle | Status |
|---|---|---|
| **Issue in, PR out**: pre-flight checks, Claim, worktree, headless session with Factory skills, self-review, a PR listing Unaddressed findings | **Covered:** a worktree on a named branch ([README L1264-L1278](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L1264-L1278)); a headless session; a prompt that pulls in the issue, through a `` !`gh …` `` line or by telling the agent to run `gh issue view`. **Missing:** Origin match and Claims. Nothing under `src/` pushes or opens a PR; neither `git push` nor `gh pr` appears there. **Review:** the templates' review is one iteration that sets out to "improve code clarity, consistency, and maintainability" ([review-prompt.md L3](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/parallel-planner-with-review/review-prompt.md#L3)). **Unshipped:** the author's `agent:implement` YAML opens a draft PR and swaps in `agent:in-progress` ([agent-implement.yml L97-L102](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-implement.yml#L97-L102), [L161-L176](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-implement.yml#L161-L176)) | **Partial**: the session and the worktree only |
| **Unattended sessions on your own logins** | Yes, with `noSandbox()`; in a container, with a token (§2) | **Covered**, with a weaker Claude default (§3.3) |
| **Subscription billing** | Claude everywhere except the remote providers; Codex on the host only (§2.1) | **Covered** for Claude, **partial** for Codex |
| **Multiple harnesses, with a Model and Effort per command** | **Agents:** six providers, and a Model on every call. **Effort:** Claude goes from `low` to `max`. Codex goes from `low` to `xhigh`, with no `max` ([AgentProvider.ts L751](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/AgentProvider.ts#L751); [PR #993](https://github.com/mattpocock/sandcastle/pull/993) is open). **Checks:** none before work; the provider interface has no such method ([AgentProvider.ts L264-L277](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/AgentProvider.ts#L264-L277)). **Choice:** the agent can differ per step, where thirdshift has one Harness per Command ([GLOSSARY.md](../../GLOSSARY.md) "Harness") | **Partial** |
| **CI and conflict Repair loop** | Nothing in the library. The author's `agent:update-branch` merges the base branch when someone adds the label, and calls an agent only on a conflict ([update-branch.ts L31-L100](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/agent-workflows/update-branch/update-branch.ts#L31-L100)). Nothing anywhere handles CI | **Missing** |
| **Self-merge** | The templates have an agent `git merge` the branches into the current branch, run the tests and close the issues ([merge-prompt.md L1-L26](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/parallel-planner-with-review/merge-prompt.md#L1-L26)). That is the option [ADR-0004](../adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md) rejected: "The agent merges from inside a session" | **Missing** |
| **Spec runs** | `parallel-planner` has an agent infer a dependency graph from the open issues ([plan-prompt.md L13-L25](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/parallel-planner-with-review/plan-prompt.md#L13-L25)). Nothing reads GitHub sub-issues or "blocked by" links. The author's `agent:implement` refuses issues that have sub-issues: "this MVP only supports standalone issues" ([agent-implement.yml L57-L62](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-implement.yml#L57-L62)) | **Missing**: the shape only |
| **Passes on a schedule** | No scheduler and no Pass lock. The worktree lock is per branch ([ADR 0007 L13-L17](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0007-worktree-locking.md#L13-L17)). The author's own triggers are GitHub label events | **Missing** |
| **Security work** | None. The template reviewer has one line on "injection vulnerabilities, credential leaks" ([review-prompt.md L28-L32](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/parallel-planner-with-review/review-prompt.md#L28-L32)) | **Missing** |
| **Failure handling** | **Kept:** `close()` keeps a dirty worktree. **Re-runs:** a re-run on the same branch reuses the worktree, and fast-forwards it from `origin` when that is safe ([ADR 0003 L13-L28](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0003-reuse-worktree-by-default.md#L13-L28)). **Errors** carry the commits and the preserved path ([README L713](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L713)). **Missing:** the push, the draft and the Claim | **Partial** |
| **Notifications and logs** | A run log under `.sandcastle/logs/`, an `onAgentStreamEvent` callback, and session transcripts copied to the host ([README L216-L234](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L216-L234), [L886-L892](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L886-L892)). No notifications | **Partial** for logs, **missing** for notifications |
| **Multi-family review** (not built in thirdshift: [#429](https://github.com/JacobStephens2/thirdshift/issues/429)) | Any agent per step. Users already put a Codex reviewer behind a Codex implementer ([#488](https://github.com/mattpocock/sandcastle/issues/488)). No findings, adjudication or Unaddressed findings | **The mechanism only** |
| **Concurrency** (one Pass per repository per machine, the Claim limit, `spec.parallel`) | None built in. Concurrent worktrees have open bugs: "Concurrent branch-strategy worktrees delete each other" ([#849](https://github.com/mattpocock/sandcastle/issues/849)), and [#917](https://github.com/mattpocock/sandcastle/issues/917) | **Missing** |
| **Prompts and Factory skills** (Session prompts generated from `src/prompt.rs`, with a `/thirdshift-<skill>` first line, [ADR-0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)) | **Prompt files:** `{{KEY}}` arguments, and `` !`command` `` lines run inside the sandbox ([README L579-L641](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L579-L641)). The author now wants to remove that interpolation ([#1035](https://github.com/mattpocock/sandcastle/issues/1035)). **Skills:** the Claude provider pipes the prompt in with `-p -`, so a `/skill` prompt "never expands" ([#928](https://github.com/mattpocock/sandcastle/issues/928); a fix, [PR #1009](https://github.com/mattpocock/sandcastle/pull/1009), is open). thirdshift passes the prompt as the last argument instead ([claude.rs L90-L103](../../src/harness/claude.rs#L90-L103)) | **Partial** |

**Coverage summary.** Sandcastle covers the session layer: the sandbox, the worktree, the agent CLI and resume. The factory is left to your script, so the parts thirdshift's code is mostly made of are missing, as they were in OpenHands. OpenHands at least ships the shape of label → PR → review → merge gate (openhands-fit.md §3). Sandcastle has that shape only in its author's own, unshipped YAML.

## 5. Fit with this repository's conventions

- **Sandcastle's own repository uses the same setup.**
  - Its `AGENTS.md` has the same "Agent skills" sections as thirdshift's: issue tracker, triage labels, domain docs ([AGENTS.md L9-L21](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/AGENTS.md#L9-L21); thirdshift's [AGENTS.md](../../AGENTS.md)).
  - Its `docs/agents/triage.md` maps the same five roles ([L5-L11](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/agents/triage.md#L5-L11)).
  - Its `docs/agents/issue-tracker.md` has the same `gh` conventions, with one difference. External PRs are a triage surface there ("PRs as a request surface: yes", [L18](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/agents/issue-tracker.md#L18)) and not here ([issue-tracker.md](../agents/issue-tracker.md)).
  - It keeps `CONTEXT.md` and `docs/adr/`, and charts work with `wayfinder:*` issues ([#884](https://github.com/mattpocock/sandcastle/issues/884)).
- **The author's own loop picks `ready-for-agent`.** Its local planner runs `gh issue list --state open --label ready-for-agent` ([plan-prompt.md L7](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/plan-prompt.md#L7)). Its batch triage hands "needs-a-human" verdicts to `ready-for-human` ([batch.ts L13-L18](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/agent-workflows/explore/batch.ts#L13-L18)).
- **The product doesn't.**
  - `sandcastle init`'s GitHub tracker filters on a `Sandcastle` label that it offers to create, and closes issues with the comment "Completed by Sandcastle" ([InitService.ts L530-L544](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L530-L544); [cli.ts L396-L412](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/cli.ts#L396-L412)).
  - Declining the label strips the filter, so the agent sees every open issue ([InitService.ts L818-L838](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/InitService.ts#L818-L838)).
  - The author's CI workflows use labels of their own: `agent:implement`, `agent:in-progress`, `agent:blocked` and `agent:review` ([agent-implement.yml](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-implement.yml)).
- **No prompt uses the skills.**
  - **Shipped prompts** write out red-green-refactor, and a `RALPH:` commit convention, in full ([simple-loop prompt.md L28-L47](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/templates/simple-loop/prompt.md#L28-L47)).
  - **The author's own prompts** point at `CONTEXT.md` and `docs/adr/` ([implement prompt.md L13-L17](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.sandcastle/agent-workflows/implement/prompt.md#L13-L17)).
  - **None** calls a mattpocock/skills skill, though the changelog describes the deterministic loop as running `/implement` on each ticket (§1.2). And because of [#928](https://github.com/mattpocock/sandcastle/issues/928), a `/skill` first line wouldn't expand anyway.
- **What the overlap is worth.** Because the labels are shared, a Sandcastle loop could read the same `ready-for-agent` queue that thirdshift's Pickup runs take from. But Sandcastle makes no Claim, so on one repository the two would take the same issues **[interpretation]**. The overlap is in vocabulary and documents, not in behaviour, so it moves the verdict little.

## 6. Gaps both ways

### 6.1 What thirdshift does that Sandcastle doesn't

From [GLOSSARY.md](../../GLOSSARY.md) and the [README](../../README.md):

- **Taking work:** the Ready issue rule (settling window, blockers, sub-issues), the Claim, the Claim limit and the Sweep.
- **Starting a Run:** Origin match and the Base branch checks; Issue branch numbering and Continuation.
- **Delivery:** capturing the PR, marking it ready, and drafting it on failure.
- **The Repair loop:** its budgets, Inherited failures, the Check re-run, Declined CI fixes, Base fixes and the review of Foreign commits.
- **Merging:** the Self-merge on the exact head, and Policy refusals.
- **Spec runs:** the Spec branch, the Spec PR, the Tickets checklist and Unready Tickets.
- **Passes:**
  - Architect runs and Weeding;
  - Security runs, Fencing and the Security review, with private advisories;
  - the Pass lock.
- **Reporting:** Run notifications, and the Activity, Command and Session logs.
- **Harnesses:**
  - Setup and the User config, with Model and Effort checks;
  - agy, Grok Build and Muse Code;
  - a Resume when background work was killed;
  - interrupting whole process trees.
- **Packaging and safety:** worktree ownership rules ([ADR-0014](../adr/0014-successful-review-acquisition-authorizes-stale-disposal.md)), and one static binary that needs only `claude`, `gh` and `git` ([ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md), [ADR-0003](../adr/0003-static-binaries-through-github-releases.md)).

### 6.2 What Sandcastle does that thirdshift could borrow

1. **An agent with no credentials, then a deterministic apply step.**
   - **What Sandcastle does.** `agent:explore` runs its agent in a read-only job.
     - A comment in the workflow says "No GH_TOKEN in this step's env: the agent has no GitHub access at all".
     - The batch is uploaded "before the agent runs, so the apply job validates the plan against a batch the agent could not have tampered with".
     - A separate job with `issues: write` then "Validates the whole plan first; on any problem it changes nothing and fails" ([agent-explore.yml L28-L31](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-explore.yml#L28-L31), [L58-L65](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-explore.yml#L58-L65), [L93-L100](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-explore.yml#L93-L100), [L109-L161](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/.github/workflows/agent-explore.yml#L109-L161)).
   - **Why it fits.** That is the shape thirdshift's open [#691](https://github.com/JacobStephens2/thirdshift/issues/691) asks for: there, removing the token variables can't stop a session running as the same user from reusing that user's stored credentials. It also fits [#637](https://github.com/JacobStephens2/thirdshift/issues/637), where a publishing session can create a public issue itself.
   - **What thirdshift would need.** In GitHub Actions, the job permissions enforce the split. Locally, the enforcement would have to be a sandbox that doesn't mount `~/.config/gh`. bwrap now works on this host **[interpretation]**.
2. **The provider seam, with the `.git` lesson, for an opt-in sandboxed Run.**
   - **The seam.** Sandcastle splits bind-mount providers from isolated ones, and each returns a small handle: `exec`, `copyFileIn`, `copyFileOut` and `close` ([README L1032-L1043](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L1032-L1043)). That would be a clean way to add the sandboxed mode that [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md) left to "a separate issue" ([#398](https://github.com/JacobStephens2/thirdshift/issues/398)).
   - **The warning.** Mounting the shared `.git` read-write is what lets an agent plant hooks, `core.fsmonitor` or a rewritten worktree `.git` file that then run on the host. Read-only overlays and disabled hooks don't close the third route. So #1010's robust fix is to "give the container its own clone and fetch the branch back, so the host `.git` is never shared" ([#1010](https://github.com/mattpocock/sandcastle/issues/1010)).
   - **Where thirdshift stands.** It already pushes with `--no-verify` ([src/git.rs L216](../../src/git.rs#L216)), and runs some worktree checks with `core.fsmonitor=false` ([src/worktree/acquisition.rs L299-L312](../../src/worktree/acquisition.rs#L299-L312)). Protecting the shared `.git` only starts to matter once there is a boundary to protect **[interpretation]**.
3. **Schema-checked output with a bounded resume.**
   - **What Sandcastle does.** `Output.object({ tag, schema, maxRetries })` validates a tagged JSON payload. When validation fails, it "resumes the same agent session and feeds back a token-efficient description of the error" ([README L685-L728](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L685-L728)).
   - **Where it would help.** Today a final message that lacks the line the prompt asks for ends an Architect run as a failure ([README](../../README.md) "Architect runs").
   - **The idea.** One Resume that asks for the line again would fit thirdshift's existing Resume concept **[interpretation]**.
4. **An agent per step, as a precedent for #429.** A Sandcastle script chooses the agent for each step, and users already put a Codex reviewer behind a Codex implementer ([#488](https://github.com/mattpocock/sandcastle/issues/488)). That is the "reviewer Harness" that [multi-family-review.md](multi-family-review.md) §9 says thirdshift would need. Sandcastle shows the mechanism, not the review rules.

### 6.3 What it would take to use Sandcastle as a primitive under thirdshift

The plausible shape is a wrapper around a Harness. thirdshift would start a small Node shim for each session, and the shim would call `run()` with:

- `cwd` set to thirdshift's own worktree;
- `branchStrategy: { type: "head" }`, so that Sandcastle makes no worktree of its own ([README L1264-L1278](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L1264-L1278));
- the Docker provider;
- `claudeCode(model, { permissionMode: "auto", effort })`.

That needs:

- **Packaging.** Node and a pinned `@ai-hero/sandcastle` on every factory machine. That goes against ADR-0003's single static binary, whose install "should need nothing beyond `claude`, `gh` and `git`". It also needs Docker or Podman, and a per-repository image with the repository's toolchains and the Harness CLIs.
- **Prompt transport.** A way to keep `/thirdshift-<skill>` first lines working: [PR #1009](https://github.com/mattpocock/sandcastle/pull/1009), or a custom `AgentProvider` that passes the prompt as an argument ([#928](https://github.com/mattpocock/sandcastle/issues/928)).
- **Factory skills in the container.** thirdshift links the skills into the worktree from a temporary directory outside it (ADR-0012). Links that point outside a bind mount would dangle **[needs live check]**, so that directory would have to be mounted at the same path.
- **Credentials.** A `setup-token` token for Claude, and a mounted `auth.json` for Codex ([#488](https://github.com/mattpocock/sandcastle/issues/488)). agy, Grok Build and Muse have no provider at all.
- **Session plumbing.**
  - The raw stream lines would have to reach thirdshift's Session log, through `logging.verbose` or `onAgentStreamEvent` ([README L216-L234](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L216-L234)).
  - The session id comes from `iterations[].sessionId` ([README L869-L875](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L869-L875)).
  - A Resume would go through `resumeSession` ([README L894-L928](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/README.md#L894-L928)).
- **Bug work.** Fixes or workarounds for [#1010](https://github.com/mattpocock/sandcastle/issues/1010), [#766](https://github.com/mattpocock/sandcastle/issues/766), [#846](https://github.com/mattpocock/sandcastle/issues/846) and [#917](https://github.com/mattpocock/sandcastle/issues/917), and a longer idle timeout.
- **Dependency risk.** All of this would rest on a pre-1.0 library whose `src/` hasn't changed since June, and which may be rebuilt on the AI SDK harness ([#884](https://github.com/mattpocock/sandcastle/issues/884)).

What it would buy is container isolation for Runs, which this host can't run today, plus the Pi, Cursor and Copilot agents. That is a poor trade now **[interpretation]**.

## 7. Verdict

### 7.1 Options

| Option | What thirdshift would gain | What it would cost or lose | Verdict |
|---|---|---|---|
| **Replace thirdshift** with Sandcastle, its templates and a script of your own | A popular, MIT, self-hosted session layer, with optional container isolation | Everything in §6.1, rebuilt in TypeScript around a library that has no opinion on any of it. A container runtime the host lacks. Codex subscription use in containers is left to you | **No** |
| **Adopt the author's GitHub Actions workflows** (`agent:explore`, `agent:implement`, `agent:review`, `agent:update-branch`) | Label-driven triage, implementation, review and conflict merges | They aren't shipped or versioned. They keep a `setup-token` token on GitHub-hosted runners, which is route (a′), and spend Actions minutes. They have no CI repair, merge or Specs, and they refuse sub-issues. claude-code-action already covers this ground (earlier survey §4.2) | **No** |
| **Sandcastle as a primitive**: a sandbox wrapper around a Harness (§6.3) | Container isolation for Runs; three more agents | Node and a container runtime on every machine, against ADR-0001 and ADR-0003. Work on prompts, skills, credentials and teardown. A pre-1.0 dependency that may be rebuilt | **Not now** |
| **Borrow ideas** | The four in §6.2 | None | **Yes** |

### 7.2 Recommendation

**Borrow ideas only. Keep thirdshift as the orchestrator, and keep calling `claude` and `codex` directly.**

- **Criterion 1.** Sandcastle passes as well as thirdshift does: the unmodified CLIs under your own logins, with Claude through `claude -p`. A Codex subscription inside a container is left to you.
- **Criterion 2.** It passes. It is MIT and runs on your own machine. Vercel and Daytona are optional paid services.
- **Criterion 3.** "Buy" would cover the session runner and the sandbox layer. thirdshift already has those in Rust, for six Harnesses and with stricter worktree ownership. Everything else would still have to be built.

**Revisit when:**

- a release fixes [#928](https://github.com/mattpocock/sandcastle/issues/928), [#936](https://github.com/mattpocock/sandcastle/issues/936), [#1010](https://github.com/mattpocock/sandcastle/issues/1010) and [#766](https://github.com/mattpocock/sandcastle/issues/766), and documents Codex subscription auth ([#488](https://github.com/mattpocock/sandcastle/issues/488));
- the AI SDK harness work ([#884](https://github.com/mattpocock/sandcastle/issues/884)) settles what Sandcastle is;
- or thirdshift wants container-isolated Runs, on a host with Docker or Podman and memory to spare.

### 7.3 What only a live trial would settle

- **The credential in use under `noSandbox()` [needs live check].** With no `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` set, `claude` should run on the host's claude.ai login and draw on plan usage, not API billing. Repeat the check for Codex and `auth.json`.
- **Claude's permission flag under `run()` and `noSandbox()` [needs live check].** The code passes `--dangerously-skip-permissions` ([Orchestrator.ts L140-L146](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/Orchestrator.ts#L140-L146)). ADR 0015 ([L25](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/docs/adr/0015-no-sandbox-in-run-and-create-sandbox.md#L25)) and the provider's own comment ([no-sandbox.ts L8-L11](https://github.com/mattpocock/sandcastle/blob/7a4e7f7d93056600d979d79a090256feddf653ea/src/sandboxes/no-sandbox.ts#L8-L11)) say it doesn't.
- **Slash-command skills through `-p -` [needs live check].** Does a `/thirdshift-implement` first line expand on the installed `claude`? [#928](https://github.com/mattpocock/sandcastle/issues/928) says no. A commenter there says plain `-p`, with no trailing `-`, works on 2.1.278.
- **Linked Factory skills inside a bind mount [needs live check].** Do links into a directory outside the worktree resolve in a container?
- **The idle timeout [needs live check].** Does a long, silent foreground test run trip it?
- **Host side effects [needs live check].** What do two concurrent `noSandbox()` runs do to `~/.gitconfig` ([#846](https://github.com/mattpocock/sandcastle/issues/846), [#917](https://github.com/mattpocock/sandcastle/issues/917))? What processes are left after an abort ([#766](https://github.com/mattpocock/sandcastle/issues/766))?
- **Memory [unverified].** Neither the Node process under `noSandbox()` nor the Docker daemon and its containers were measured.
- **The private daemon [unverified].** Whether the `.factory/` daemon is public under another name.

### 7.4 A minimal trial plan

A trial is worth running only to keep the primitive option open; the verdict above doesn't depend on it. It needs no Docker, and about an hour:

1. **Set up a throwaway.**
   - In a new scratch directory, outside any real repository, make a local git repository with one commit.
   - Add a skill at `.claude/skills/probe/SKILL.md` that tells the model to say a codeword.
   - Copy `~/.gitconfig` aside, and unset `ANTHROPIC_API_KEY` and `CLAUDE_CODE_OAUTH_TOKEN`.
2. **Install the pinned package.** In a second scratch directory, read the published `package.json` for install scripts first, then run `npm install @ai-hero/sandcastle@0.12.0`.
3. **Run Claude.** Call `run()` with:
   - `cwd` set to the throwaway repository;
   - `sandbox: noSandbox()` and `branchStrategy: { type: "branch", branch: "trial/1" }`;
   - `agent: claudeCode("<model>", { permissionMode: "auto" })`;
   - an inline `prompt` whose first line is `/probe`, and which asks for one committed file and `<promise>COMPLETE</promise>`;
   - `logging: { type: "file", path: "<scratch>/run.log", verbose: true }`.

   Then check:
   - the `claude` flags, with `ps -eo args` while it runs;
   - the codeword in `run.log`, which shows whether the skill expanded;
   - the commit on `trial/1`;
   - that the session drew on plan usage, not API billing;
   - a `diff` of `~/.gitconfig` against the copy.
4. **Abort one.** Run again with `permissionMode` left unset, interrupt it partway, and look for leftover `claude` processes with `pgrep -a claude`.
5. **Run Codex.** Repeat step 3 with `codex("<model>")`, and confirm that the host's `auth.json` login is the one used.

From the code, I'd expect the flags and side effects in §3.3, and no skill expansion. If that is what the trial shows, the primitive option stays closed until upstream fixes land.
