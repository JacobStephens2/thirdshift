# Codex CLI as a Harness: running `codex exec` headless

Research date: 2026-10-04.

**Question.** thirdshift runs each unattended Session as `claude -p --permission-mode auto --plugin-dir <Factory skills dir> --output-format stream-json --verbose [--resume <id>] <prompt>` ([src/session.rs](../../src/session.rs) `claude_args`). It then parses the stream ([src/progress.rs](../../src/progress.rs)) for four things: progress lines from tool uses, the session id (`system`/`init`), the final message and turn/cost summary (`result`), and background tasks killed when the session ended (`task_started` / `task_updated` / `task_notification`). The Factory skills are written to a temp dir as a Claude plugin and removed afterwards ([src/plugin.rs](../../src/plugin.rs)). Can `codex exec` stand in as a second Harness? What are the equivalents, and where are the gaps?

**Method.**

- Source: `openai/codex` at tag **`rust-v0.160.0`** (commit `a956835`), which matches the local `codex-cli 0.160.0`. All source paths below are relative to `codex-rs/` at that tag, and the links point there.
- Docs: the official Codex docs. Every `developers.openai.com/codex/...` page now returns a 308 redirect to `learn.chatgpt.com/docs/...`, which is OpenAI's own site. The links below are the redirect targets.
- Cross-checked against `codex exec --help`, `codex exec resume --help`, `codex debug --help` and `codex debug models --bundled` on this machine. No model session was run (another agent is doing the live experiments).

**Out of scope.** Choosing whether to add a Codex Harness, and live behaviour that only a real turn can show. Those items are flagged **[needs live check]**.

## TL;DR

- **A drop-in command line exists:**
  `codex exec --json -m <slug> -c model_reasoning_effort="<effort>" --dangerously-bypass-approvals-and-sandbox -C <worktree> <prompt>`
  and for the one Resume:
  `codex exec --json -m <slug> -c model_reasoning_effort="<effort>" --dangerously-bypass-approvals-and-sandbox -C <worktree> resume <thread_id> <prompt>`.
- **The model is a slug, matched case-sensitively.** Effort is a free string with no `--effort` flag; set it with `-c model_reasoning_effort=...`. **Nothing is validated at startup.** An unknown model (say `GPT-6.1-Sol` instead of `gpt-6.1-sol`) or an unknown effort (`High`) is sent to the API as written, so it can only fail on the first request. The cheap pre-check is `codex debug models`, which prints the catalog and runs no turn.
- **Skills: there is no exec flag or config key for an extra skill root.** The only runtime extra-roots hook is the app-server's `skills/extraRoots/set` RPC. For one session without touching the repo or the global install, the workable route is a **temp `CODEX_HOME`** with the skills in `$CODEX_HOME/skills/` and `auth.json` (plus `config.toml`) symlinked in from `~/.codex`. Rollouts then live in that temp home, so the Resume must use the same one.
- **Permissions: `codex exec` never asks for approval (`approval_policy = never`).** Any approval request is rejected. `workspace-write` turns network off and keeps `.git` and a worktree's gitdir read-only. Writes into the main repo's shared `.git` sit outside the worktree's writable root. So `git commit`, `git push` and `gh` from a thirdshift worktree need **`--dangerously-bypass-approvals-and-sandbox`** (or `-s danger-full-access`). The closest thing to Claude's auto mode is **`--approve-for-me`**: a model reviewer judges escalation requests under `workspace-write` and `on-request`.
- **JSONL schema:** `thread.started{thread_id}`, `turn.started`, `item.started` / `item.updated` / `item.completed` (item types `agent_message`, `reasoning`, `command_execution`, `file_change`, `mcp_tool_call`, `collab_tool_call`, `web_search`, `todo_list`, `error`), `turn.completed{usage}`, `turn.failed{error}`, `error{message}`. Usage is a token count only: no cost and no turn count.
- **Background work:** `exec_command` (unified exec) processes outlive the tool call and the turn. `codex exec` exits as soon as the turn completes, and shutdown kills every live process. The JSONL has no event for this. The only trace is a `command_execution` item that is still `in_progress` when the turn ends.
- **`codex exec resume` does not keep the model or effort.** exec always sends a model provider on resume, which counts as an explicit override, so the stored model and effort are skipped. Pass `-m`, `-c model_reasoning_effort` and the sandbox flags again on every Resume.
- **Exit code 1** on a failed or interrupted turn, a non-retryable error, a rejected server request, or a startup or config error. Otherwise 0.

## 1. Model and reasoning effort

### 1.1 Flags and keys

- `-m/--model <MODEL>`: "Model the agent should use" ([utils/cli/src/shared_options.rs#L21-L23](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/cli/src/shared_options.rs#L21-L23)). exec marks it global, so it is accepted after `resume` as well ([exec/src/cli.rs#L140-L147](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/cli.rs#L140-L147)). Docs: "Override the configured model for this run" ([developer commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)).
- **There is no effort flag** on `codex exec` (`codex exec --help`). Effort is the config key `model_reasoning_effort` ([config/src/config_toml.rs#L392](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/config_toml.rs#L392)), set per run with `-c model_reasoning_effort="high"`. `-c` values are parsed as TOML, and a value that does not parse is used as a literal string (`codex exec --help`).
- Config reference: `model_reasoning_effort` is the "Reasoning effort advertised by the selected model, such as low, medium, high, xhigh, max, or ultra" ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference)).

### 1.2 Allowed effort values and case

`ReasoningEffort` knows `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` and `persistent`, plus `Custom(String)` for "a model-defined effort value that this client does not know yet" ([protocol/src/openai_models.rs#L59-L72](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/protocol/src/openai_models.rs#L59-L72)). Parsing is an exact, **case-sensitive** match. The empty string is the only rejected value, and anything else becomes `Custom` ([#L140-L157](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/protocol/src/openai_models.rs#L140-L157)). So `High` is accepted at config load as `Custom("High")`.

Before the request, two values are rewritten:

- `ultra` resolves to the model's `multi_agent_reasoning_effort`, else `max`, else the highest non-ultra level.
- `persistent` is sent as `disabled`.

Everything else passes through unchanged ([protocol/src/openai_models/reasoning_effort.rs#L10-L39](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/protocol/src/openai_models/reasoning_effort.rs#L10-L39)). Where the model client cannot take effort updates, or the effort is an unknown `Custom`, the selected effort is sent in the request as is ([core/src/session/reasoning_effort.rs#L94-L157](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/session/reasoning_effort.rs#L94-L157)). Per-model supported levels from the bundled catalog (`codex debug models --bundled`, [models-manager/models.json](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/models-manager/models.json)):

| slug | display name | efforts | default |
|---|---|---|---|
| `gpt-6.1-sol` | GPT-6.1-Sol | low, medium, high, xhigh, max, ultra | low |
| `gpt-6-sol` | GPT-6-Sol | low … ultra | medium |
| `gpt-6-astra` | GPT-6-Astra | low … ultra | low |
| `gpt-6-luna` | GPT-6-Luna | low … max | medium |
| `gpt-5.6-sol` / `-terra` / `-luna` | GPT-5.6-… | low … ultra (luna: … max) | low / medium / medium |
| `gpt-5.5` | GPT-5.5 | low … xhigh | medium |

The live catalog for an account can differ from the bundled one. `codex debug models` without `--bundled` refreshes it from the provider ([cli/src/main.rs#L2068-L2095](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/cli/src/main.rs#L2068-L2095)).

### 1.3 Slugs versus display names

The model string is matched against the catalog by **longest slug prefix** (`model.starts_with(&candidate.slug)`, which is case-sensitive). A miss gets one retry with a single `namespace/` prefix stripped ([models-manager/src/manager.rs#L855-L915](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/models-manager/src/manager.rs#L855-L915)). Display names are never consulted. `GPT-6.1-Sol` therefore misses. Codex then logs `Unknown model … This will use fallback model metadata`, at `warn` level, which exec's default stderr filter (`error`) hides. It carries on with generic metadata: no supported efforts, a 272k context window, and so on ([models-manager/src/model_info.rs#L98-L136](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/models-manager/src/model_info.rs#L98-L136); [exec/src/lib.rs#L174](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L174)).

### 1.4 When an invalid model or effort is rejected

**Not at startup.** The OpenAI models manager keeps a requested model unchanged: `if let Some(model) = model { return model.to_string(); }` ([models-manager/src/manager.rs#L204-L218](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/models-manager/src/manager.rs#L204-L218)). The lookup falls back instead of failing (§1.3), and effort parsing accepts any non-empty string (§1.2). exec puts the configured effort on the `turn/start` request (`effort: default_effort`, [exec/src/lib.rs#L881-L882](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L881-L882), [#L1173](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1173)). So a bad value can only be refused by the backend on the first model request. That would arrive as `error` and/or `turn.failed` with exit code 1 (§6.4). What the backend does with a display name or `High` is **[needs live check]**.

### 1.5 Cheap validation without a turn

- `codex debug models` prints the catalog as JSON (`{"models":[{"slug", "display_name", "supported_reasoning_levels":[{"effort",…}], "default_reasoning_level", "visibility", …}]}`) and starts no thread ([cli/src/main.rs#L2068-L2095](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/cli/src/main.rs#L2068-L2095)). thirdshift could check that `model` equals a `slug` exactly and that `effort` is in that slug's `supported_reasoning_levels[].effort`. `--bundled` skips the network.
- `codex debug prompt-input [PROMPT]` "Render[s] the model-visible prompt input list as JSON" (`codex debug prompt-input --help`). It is useful for checking which AGENTS.md and skills a configuration would load, still without a turn.

## 2. Loading a skills directory for one session

### 2.1 Where Codex looks

Docs ([build skills](https://learn.chatgpt.com/docs/build-skills)) list four scopes:

- REPO: `.agents/skills` in the cwd, its parents, and the repo root.
- USER: `$HOME/.agents/skills`.
- ADMIN: `/etc/codex/skills`.
- SYSTEM: bundled with Codex.

The docs add: "Codex supports symlinked skill folders and follows the symlink target." The source ([ext/skills/src/host_roots.rs#L48-L131](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/host_roots.rs#L48-L131)) adds two roots the docs leave out, and spells out one condition:

- `$CODEX_HOME/skills`, the "Deprecated user skills location … kept for backward compatibility", User scope.
- `$CODEX_HOME/skills/.system`, the System scope, kept as a cache.
- `<project>/.codex/skills`, for each project config layer.
- Repo `.agents/skills` is probed in every directory from the project root (the first root marker, default `.git`) down to the cwd ([#L137-L185](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/host_roots.rs#L137-L185)).
- Plugin skill roots and `extra_skill_roots` are appended too ([#L56-L70](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/host_roots.rs#L56-L70)).

Scanning rules:

- A skill is a `SKILL.md` up to 6 levels deep, at most 2000 per root ([ext/skills/src/loader/mod.rs#L19-L32](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/loader/mod.rs#L19-L32)).
- Directory symlinks are followed for User, Repo and Admin scopes, but not System ([ext/skills/src/loader/host.rs#L165-L166](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/loader/host.rs#L165-L166)).
- Hidden directories are skipped ([#L178](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/loader/host.rs#L178)).

### 2.2 No per-session flag or key

- `extra_skill_roots` is fed only by the app-server's `skills/extraRoots/set` request ([app-server/src/request_processors/catalog_processor.rs#L595-L604](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/app-server/src/request_processors/catalog_processor.rs#L595-L604); [ext/skills/src/host_service.rs#L154-L160](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/ext/skills/src/host_service.rs#L154-L160)). `codex exec` sends no such request.
- `[skills]` config has `bundled`, `include_instructions`, `max_context_tokens` and `config`. `[[skills.config]]` entries only *enable or disable* skills by path or name, and cannot add a root ([config/src/skills_config.rs#L20-L47](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/skills_config.rs#L20-L47)). The docs match ("disable skills without deletion").
- `--add-dir` only adds *writable* roots ([utils/cli/src/shared_options.rs#L74-L76](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/cli/src/shared_options.rs#L74-L76)).
- **Plugins** carry skills, and Codex reads a `.claude-plugin/plugin.json` manifest as an alternate ([core-plugins/src/manifest.rs#L685](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core-plugins/src/manifest.rs#L685)). They load only from the install cache, `$CODEX_HOME/plugins/cache/<marketplace>/<plugin>/<version>`, after a marketplace add and plugin install, which records state in config ([core-plugins/src/store.rs#L118-L159](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core-plugins/src/store.rs#L118-L159); [config/src/types.rs#L1004-L1121](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/types.rs#L1004-L1121)). There is no `--plugin-dir` equivalent.
- The `-p/--profile` layer is `$CODEX_HOME/<name>.config.toml`. Its config folder is still `$CODEX_HOME`, and the name must be a plain name ([core/src/config/mod.rs#L2025-L2033](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/config/mod.rs#L2025-L2033); [config/src/state.rs#L219-L231](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/state.rs#L219-L231); [protocol/src/config_types.rs#L127-L147](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/protocol/src/config_types.rs#L127-L147)).

### 2.3 Options that work, and what each costs

1. **Temp `CODEX_HOME`, recommended.** Write the skills to `<tmp>/skills/<name>/SKILL.md` and run with `CODEX_HOME=<tmp>`. `CODEX_HOME` must already exist and be a directory, or Codex exits ([utils/home-dir/src/lib.rs#L13-L40](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/home-dir/src/lib.rs#L13-L40)). Implications:
   - **Auth.** The default `cli_auth_credentials_store` is `file`, meaning `CODEX_HOME/auth.json` ([config/src/types.rs#L112-L125](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/types.rs#L112-L125); [login/src/auth/storage.rs#L154-L156](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/login/src/auth/storage.rs#L154-L156)). Saving it is an `OpenOptions::open(...).truncate(true)` on that path ([#L206-L223](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/login/src/auth/storage.rs#L206-L223)), which follows symlinks. So a **symlinked** `auth.json` keeps token refreshes in the real file. A copy would drift from it, and might leave the original holding a stale refresh token (inference, **[needs live check]**).
   - **Keyring.** With `keyring` or `auto`, the keyring entry name is a hash of the canonical `CODEX_HOME` path ([login/src/auth/storage.rs#L238-L250](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/login/src/auth/storage.rs#L238-L250)), so a temp home would not find it.
   - **API-key auth.** `CODEX_API_KEY` (or `OPENAI_API_KEY`) in the environment sidesteps `auth.json` entirely ([login/src/auth/manager.rs#L953-L966](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/login/src/auth/manager.rs#L953-L966); docs: "set `CODEX_API_KEY` inline" ([non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode))).
   - **Everything else in `CODEX_HOME` moves too:** `config.toml` (symlink it to keep the user's settings, for example `projects.*.trust_level`), the global `AGENTS.md`, `hooks.json`, the rollouts in `sessions/` ([rollout/src/lib.rs#L86](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/rollout/src/lib.rs#L86)) and the SQLite state. So **the Resume must run with the same temp `CODEX_HOME`**. thirdshift's `Sessions::within` lifetime already covers both runs. Codex also installs its system skills into `<tmp>/skills/.system` on first use ([skills/src/lib.rs#L63-L69](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/skills/src/lib.rs#L63-L69)).
2. **Repo `.agents/skills` or `.codex/skills` in the worktree.** Rejected by the brief, since it writes into the repo. Under `workspace-write`, `.agents` and `.codex` are read-only to the agent anyway (§3.2).
3. **A symlink in the real `~/.codex/skills` or `~/.agents/skills`** is a global install that every concurrent interactive Codex session would see.
4. **No discovery at all.** Point the prompt at the skill files ("read `<dir>/<skill>/SKILL.md`"). It works with any harness, but loses implicit, description-based invocation.

## 3. Unattended permissions with network (git push, gh)

### 3.1 What exec does by default

- **The approval policy is forced to `never`**: "Default to never ask for approvals in headless mode" ([exec/src/lib.rs#L571-L577](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L571-L577)). The one exception: when the effective `approvals_reviewer` is `auto_review`, the config is rebuilt without that override, so `on-request` holds ([#L758-L792](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L758-L792)).
- **Any approval request that does reach exec is rejected**: command, file-change, permissions and `request_user_input` alike. If the rejection itself fails, `error_seen` is set and exec exits 1 ([#L1990-L2131](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1990-L2131)).
- **The default sandbox comes from config.** Docs: "By default, `codex exec` runs in a read-only sandbox" ([non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)).

### 3.2 Sandbox modes

`-s/--sandbox read-only | workspace-write | danger-full-access` (`codex exec --help`).

Under `workspace-write`:

- **Network is off by default.** Enable it with `-c sandbox_workspace_write.network_access=true` ([approvals & security](https://learn.chatgpt.com/docs/agent-approvals-security); [config reference](https://learn.chatgpt.com/docs/config-file/config-reference)).
- **`.git` stays read-only "including gitdir pointers".** So do `.agents` and `.codex` (same doc page). The source protects a worktree's `.git` file and the gitdir it points to ([protocol/src/permissions.rs#L2346-L2380](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/protocol/src/permissions.rs#L2346-L2380)).
- **A commit from a worktree writes outside the writable root.** A thirdshift worktree's objects and refs live in the main repo's shared `.git`, outside the worktree's writable root. So `git commit` from a worktree also needs that directory, through `--add-dir` or `sandbox_workspace_write.writable_roots` (inference from git's layout, **[needs live check]**). Even then, the gitdir carve-out above stays read-only.

### 3.3 The unattended options

| Flag | Effect (source) | Fit for commit + push + gh |
|---|---|---|
| `--dangerously-bypass-approvals-and-sandbox` (alias `--yolo`) | Sandbox → `DangerFullAccess` ([exec/src/lib.rs#L328-L332](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L328-L332)), approval stays `never`, and the git-repo check is skipped too ([#L975-L983](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L975-L983)). Docs: "only use inside an isolated runner" ([developer commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)). | Works. The worktree is the only isolation. |
| `-s danger-full-access` | No sandbox, approval `never` (exec default), git-repo check still on | Works, and keeps the repo check |
| `--approve-for-me` (alias `--not-so-yolo`) | Adds `approvals_reviewer="auto_review"`, `approval_policy="on-request"`, `sandbox_mode="workspace-write"`, and conflicts with `-s` and `--yolo` ([utils/cli/src/shared_options.rs#L43-L50](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/cli/src/shared_options.rs#L43-L50), [#L80-L93](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/cli/src/shared_options.rs#L80-L93)). Docs: "routes eligible approval requests through an automatic reviewer agent" ([approvals & security](https://learn.chatgpt.com/docs/agent-approvals-security)). | The closest analogue to Claude's `--permission-mode auto`. Each push or escalation succeeds only if the reviewer model approves it, so it is non-deterministic **[needs live check]**. |
| `-s workspace-write -c sandbox_workspace_write.network_access=true --add-dir <main .git>` | Sandboxed, with network | Probably still blocked by the gitdir carve-out (§3.2) |

## 4. The `--json` JSONL stream

### 4.1 Event types

Event types are defined in [exec/src/exec_events.rs#L9-L94](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/exec_events.rs#L9-L94). Docs list the same set ([non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)). stdout carries only these lines ("In --json mode, stdout must be valid JSONL", [exec/src/lib.rs#L3](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L3)).

| `type` | Payload | When |
|---|---|---|
| `thread.started` | `thread_id` | First line, for a new thread **and for a resume** ([exec/src/lib.rs#L1116](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1116); [event_processor_with_jsonl_output.rs#L606-L613](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L606-L613)). This is the id `resume` takes (`codex exec resume <SESSION_ID>`), the analogue of Claude's `system/init.session_id`. |
| `turn.started` | `{}` | |
| `item.started` / `item.updated` / `item.completed` | `item: {id: "item_N", type, …}` | `agent_message` and `reasoning` arrive only as `item.completed` ([#L343-L368](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L343-L368)). |
| `turn.completed` | `usage: {input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens}` | Thread **totals** from the last token-usage update ([#L118-L129](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L118-L129)). **No cost and no turn count.** thirdshift's "N turns, $X" summary has no direct equivalent. |
| `turn.failed` | `error: {message}` | |
| `error` | `message` | Any error notification for the turn, **including ones with `will_retry`**, such as stream reconnects ([#L447-L458](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L447-L458)). Don't treat it as fatal; `turn.failed` and the exit code are the verdict. |

Item `type`s ([exec_events.rs#L104-L324](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/exec_events.rs#L104-L324)):

- `agent_message{text}`. The final message is the last of these, and `-o <file>` writes it out.
- `reasoning{text}`, a summary.
- `command_execution{command, aggregated_output, exit_code, status: in_progress|completed|failed|declined}`. This is the counterpart of Claude's `Bash` tool use, so thirdshift's commit/push detection can read `command`. `command` is the full shell string. Whether it carries a `bash -lc` wrapper is **[needs live check]**.
- `file_change{changes:[{path, kind: add|delete|update}], status}`.
- `mcp_tool_call{server, tool, arguments, result, error, status}`.
- `collab_tool_call{tool: spawn_agent|send_input|wait|close_agent, sender_thread_id, receiver_thread_ids, prompt, agents_states, status}`, for sub-agents.
- `web_search{query, action}`.
- `todo_list{items:[{text, completed}]}`.
- `error{message}`. Non-fatal warnings, config and deprecation notices, and "model rerouted" all arrive this way ([#L427-L507](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L427-L507)).

**Skill use has no item of its own.** Skills are read through ordinary commands, so a "skill X" progress line would mean pattern-matching `SKILL.md` reads in `command_execution` (inference).

Notifications from other threads, which includes sub-agent threads, are dropped. Only the primary thread's current turn is printed ([exec/src/lib.rs#L1591-L1653](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1591-L1653)).

### 4.2 One turn per process

exec sends one `turn/start` and leaves the loop on `turn.completed`, `turn.failed` or an interrupted turn (`CodexStatus::InitiateShutdown`, [event_processor_with_jsonl_output.rs#L513-L563](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L513-L563); [exec/src/lib.rs#L1284-L1305](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1284-L1305)). Claude can resume a session within one process after waiting on background agents. exec does nothing like that.

### 4.3 Background and long-running processes

- Shell commands run through **unified exec**, which is stable and on by default ([features/src/lib.rs#L990-L995](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/features/src/lib.rs#L990-L995)). An `exec_command` call returns after `yield_time_ms`, and a still-running process returns a `session_id` "to pass to write_stdin when the process is still running" ([core/src/tools/handlers/shell_spec.rs#L200-L225](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/tools/handlers/shell_spec.rs#L200-L225)). That is Codex's form of a background task.
- Those processes **outlive the turn**. At turn end Codex only *counts* them in a metric (`TURN_UNIFIED_EXEC_RUNNING_PROCESSES_METRIC`, [core/src/tasks/mod.rs#L796-L800](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/tasks/mod.rs#L796-L800)). They are terminated only on an explicit clean or on session shutdown (`terminate_all_processes`, [core/src/session/handlers.rs#L61-L63](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/session/handlers.rs#L61-L63), [#L307-L310](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/session/handlers.rs#L307-L310); [core/src/unified_exec/process_manager.rs#L1796-L1813](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/unified_exec/process_manager.rs#L1796-L1813)). exec shuts the in-process app-server down right after the turn completes ([exec/src/lib.rs#L1318-L1324](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1318-L1324)), **so a background process still running at turn end is killed**, much as Claude's are.
- **Spawned commands are not in thirdshift's process group.** Each one gets its own session or process group (`setsid`/`setpgid`) and, on Linux, `PR_SET_PDEATHSIG = SIGTERM` toward its parent ([utils/pty/src/process_group.rs#L7-L74](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/pty/src/process_group.rs#L7-L74); [utils/pty/src/child_command.rs#L310-L313](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/pty/src/child_command.rs#L310-L313)). thirdshift's `kill(-pgid)` on interrupt therefore reaches only `codex` itself, and the direct children get SIGTERM through the death signal.
- **The stream carries no "killed" event.** The JSONL drops the app-server's `process_id` from `command_execution` ([event_processor_with_jsonl_output.rs#L162-L181](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L162-L181)). At `turn.completed`, items started but never completed are re-emitted as `item.completed` with their turn-end state ([#L370-L385](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L370-L385)). So the detectable signal is **a `command_execution` with `status: "in_progress"` at, or reconciled at, turn end**. Whether a yielded process's item stays `in_progress` until it exits, or completes when the tool call returns, is **[needs live check]**. The exit watcher emits the end event when the process exits ([core/src/unified_exec/async_watcher.rs#L156-L160](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/unified_exec/async_watcher.rs#L156-L160)).
- **Sub-agents.** A `collab_tool_call` whose `agents_states` still shows `running` at turn end is the sub-agent analogue (inference).

## 5. `codex exec resume`

### 5.1 Arguments and flags

- Arguments: `[SESSION_ID] [PROMPT]`, `--last`, `--all`, `-i` ([exec/src/cli.rs#L182-L250](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/cli.rs#L182-L250)).
- Global exec flags also work after `resume`: `-m`, `--yolo`, `--dangerously-bypass-hook-trust`, `--json`, `-o`, `--skip-git-repo-check`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--output-schema`, `-c`, `--enable`/`--disable` ([#L20-L81](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/cli.rs#L20-L81), [#L140-L147](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/cli.rs#L140-L147); `codex exec resume --help`).
- **`-s`, `--approve-for-me`, `-C`, `--add-dir` and `-p` are not global.** Put them before `resume`, as in `codex exec -s danger-full-access -C <wt> resume <id> …`. The parse test covers this for `--approve-for-me` ([exec/src/cli_tests.rs#L108-L115](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/cli_tests.rs#L108-L115)). `--worktree` is refused with resume ([exec/src/lib.rs#L304-L307](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L300-L307)).
- A UUID id is used as is. A name is looked up, filtered by cwd unless `--all` ([#L1771-L1909](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1771-L1909)). If a *name* or `--last` lookup finds nothing, exec **silently starts a new thread** ([#L993-L1024](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L993-L1024)), so pass the UUID from `thread.started`.

### 5.2 Model and effort do not carry over

The app-server restores a thread's stored model and effort only when the resume request has no model override. `model_provider.is_some()` counts as an override ([app-server/src/request_processors/thread_processor.rs#L230-L283](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/app-server/src/request_processors/thread_processor.rs#L230-L283)), and exec always sends `model_provider: Some(config.model_provider_id)` plus `model: config.model` ([exec/src/lib.rs#L1386-L1412](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1386-L1412)). The turn then carries this run's `config.model_reasoning_effort` ([#L1173](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1173)).

**So a Resume without `-m` and `-c model_reasoning_effort` runs on the config default, not on what the first session used** (inference from source, **[needs live check]**). The sandbox, approval policy and cwd are likewise re-sent from this run's config (same lines).

## 6. Other differences for unattended work in a git worktree

### 6.1 AGENTS.md

Codex reads AGENTS.md from two places ([AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md)):

- `$CODEX_HOME/AGENTS.override.md` or `AGENTS.md`.
- One file per directory from the project root (normally the git root) down to the cwd: `AGENTS.override.md`, then `AGENTS.md`, then `project_doc_fallback_filenames`.

The combined size is capped at `project_doc_max_bytes` (32 KiB by default). The thirdshift worktree's own AGENTS.md is therefore picked up. `CLAUDE.md` is not read unless it is added to `project_doc_fallback_filenames`, which matters for repos that only have `CLAUDE.md`. A temp `CODEX_HOME` drops the user's global AGENTS.md unless it is symlinked in (§2.3).

### 6.2 Project trust

Project-local `.codex/config.toml`, hooks and exec policies are "loaded but disabled when the directory is untrusted" ([config/src/loader/mod.rs#L122-L134](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/loader/mod.rs#L122-L134), [#L1086-L1104](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/loader/mod.rs#L1086-L1104)). Trust is looked up for the directory, then the project root, then the **repo root**, so a linked worktree inherits the main checkout's `projects."<repo>".trust_level` ([#L1040-L1080](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/loader/mod.rs#L1040-L1080)). exec still runs in an untrusted directory; it just ignores those layers. Linked worktrees read hooks from the root checkout's `.codex/` ([#L1106-L1115](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/config/src/loader/mod.rs#L1106-L1115)).

### 6.3 Hooks

Hooks live in `hooks.json` or `[hooks]` in user and project config. Events: `SessionStart`/`SessionEnd`, `UserPromptSubmit`, `Stop`, `Interrupt`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `Pre/PostCompact`, `SubagentStart`/`SubagentStop`. Non-managed hooks run only once trusted by hash, or with `--dangerously-bypass-hook-trust` ([hooks](https://learn.chatgpt.com/docs/hooks); [utils/cli/src/shared_options.rs#L61-L64](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/utils/cli/src/shared_options.rs#L61-L64)). Hook events are not printed in the JSONL ([event_processor_with_jsonl_output.rs#L474-L476](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L474-L476)).

### 6.4 Git repo check and exit codes

- **The git repo check** refuses to run outside a git repo ("Not inside a trusted directory and --skip-git-repo-check was not specified.", exit 1) unless `--skip-git-repo-check` or `--yolo` is given ([exec/src/lib.rs#L975-L983](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L975-L983)). A thirdshift worktree passes it.
- **Exit 1** comes from any of:
  - a non-retrying `error` for the turn;
  - `turn.failed`;
  - an **interrupted** turn;
  - a failed rejection of a server request ([#L1258-L1324](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1258-L1324));
  - a startup failure, such as bad `-c`, a missing `CODEX_HOME`, rules or login problems ([#L334-L360](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L334-L360), [#L622-L642](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L622-L642));
  - an error returned from `run_main` ([exec/src/main.rs#L28-L39](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/main.rs#L28-L39)).
- **Exit 0** otherwise.

### 6.5 Signals

exec handles only **SIGINT** (`tokio::signal::ctrl_c`), turning it into `turn/interrupt` ([exec/src/lib.rs#L1126-L1132](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1124-L1130), [#L1226-L1248](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/lib.rs#L1226-L1248)). thirdshift's SIGTERM-then-SIGKILL `stop` would end it without a graceful interrupt, with children cleaned up by the death signal (§4.3). Sending SIGINT first would let Codex finish the rollout cleanly.

### 6.6 Other per-run flags

- `--ephemeral` skips rollout files, so it is incompatible with a later Resume.
- `--ignore-user-config` skips `config.toml`, but auth still comes from `CODEX_HOME`.
- `-o/--output-last-message <file>` writes the final message on success only ([event_processor_with_jsonl_output.rs#L631-L638](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/exec/src/event_processor_with_jsonl_output.rs#L624-L630)).
- The prompt can be a positional argument or come from stdin (`-` or no argument). thirdshift passes `Stdio::null()`, so the prompt must be the positional argument (`codex exec --help`).

## 7. Mapping to what thirdshift relies on

| thirdshift today (`claude`) | `codex exec` equivalent |
|---|---|
| `-p … <prompt>` | `codex exec <prompt>` |
| `--permission-mode auto` | `--approve-for-me` (reviewer-gated), or `--dangerously-bypass-approvals-and-sandbox` for deterministic git/gh |
| `--plugin-dir <skills>` | No flag. Temp `CODEX_HOME` with `skills/` plus a symlinked `auth.json` and `config.toml` |
| `--output-format stream-json --verbose` | `--json` |
| `--resume <id>` | `resume <thread_id>` subcommand, re-passing `-m`, `-c model_reasoning_effort`, sandbox flags and `CODEX_HOME` |
| `system/init.session_id`, `cwd` | `thread.started.thread_id`; there is no cwd field, so thirdshift already knows it |
| `assistant` `tool_use` (Skill, Bash, …) | `item.started` / `item.completed` with `command_execution`, `file_change`, `mcp_tool_call`, `web_search`, `collab_tool_call`. No Skill item. |
| `result.result` (final message) | Last `agent_message` item, or the `-o <file>` |
| `result.num_turns`, `total_cost_usd` | Token totals only (`turn.completed.usage`) |
| `result.is_error` | `turn.failed` and exit code 1 |
| `task_started` / `task_updated killed` / `task_notification stopped` | No event. A `command_execution` (or sub-agent) still `in_progress` at turn end, which exec then kills on shutdown |

## 8. Live checks on this machine (codex-cli 0.160.0, 2026-10-04)

Short real `codex exec --json` runs in a throwaway git repo and a worktree of it. These settle some of the **[needs live check]** items above.

- **Display names are rejected by the server.** `-m GPT-6.1-Sol` emits a fallback-metadata warning, then `turn.failed` with HTTP 400 `The 'GPT-6.1-Sol' model is not supported when using Codex with a ChatGPT account.`, and exits 1. An unknown slug fails the same way. `-m gpt-6.1-sol` works.
- **Effort case is rejected by the server.** `-c model_reasoning_effort=Max` and `=bogus` both fail on the first request with `Invalid value: 'Max'. Supported values are: 'none', 'minimal', 'low', 'medium', 'high', 'xhigh', and 'max'.` and exit 1. `max` and `ultra` both work. Before the failure, `thread.started` and `turn.started` have been emitted and a rollout has been written.
- **The sandbox cannot run here.** Ubuntu's `kernel.apparmor_restrict_unprivileged_userns = 1` blocks bubblewrap:
  - `--sandbox workspace-write` fails every command with `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`.
  - With `network_access=true` the error is `bwrap: setting up uid map: Permission denied`.
  - The exit code is still 0. The failed attempts show up only in the agent's message, not as `command_execution` items.
  - Under `--approve-for-me`, every command therefore escalates to the auto-reviewer. One `curl` to api.github.com was approved and rerun unsandboxed.
  - Only `--dangerously-bypass-approvals-and-sandbox` runs unattended without escalating every command.
- **Skills from a worktree symlink are found.** `<worktree>/.agents/skills -> <dir>` and `<worktree>/.codex/skills` both appear in `codex debug prompt-input`. A temp `CODEX_HOME` with a real `skills/` dir of per-skill symlinks also works. Codex writes `skills/.system/` into it, so `skills/` itself must not be a symlink to the source.
- **Stdin:** codex waits for stdin to close even when a prompt argument is given, so it must be spawned with stdin set to null.
- **Background work:** a command the agent left running (`sleep 188`) appeared as `item.started` `command_execution` `in_progress` with no `item.completed` before `turn.completed`, and it died with codex. `setsid nohup …` escaped and survived.
- **Resume** with `-m` set to another model switches model with a warning. Without the bypass flag the resumed turn fell back to `workspace-write` with network off, so the sandbox flag must be re-passed.
- **Trust:** inside a git repo there is no prompt. Codex adds `[projects."<repo root>"] trust_level = "trusted"` to `config.toml` on its own, and a new worktree of that repo needs no new entry.
- **Instructions:** Codex sessions don't read `~/.claude/CLAUDE.md`, `~/.claude/rules/` or Claude Code hooks.
