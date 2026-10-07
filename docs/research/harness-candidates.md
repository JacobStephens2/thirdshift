# Four more Harness candidates: Muse Code, Grok Build, MiMo Code and agy, run headless

Research date: 2026-10-05.

**Question.** Issue #428 asks for four more harness and model pairs:

- Muse Spark 1.3 in Muse Code
- Grok 4.7 in Grok Build
- MiMo-V2.6-Pro in MiMo Code
- Gemini 3.8 Flash in agy (Google's Antigravity CLI)

thirdshift needs five things from a Harness's CLI:

- a one-shot unattended run in a git worktree whose `.git` lives in the main checkout
- a Model and Effort it can check before any work
- an event stream it can parse for the session id, the final message, errors and usage
- a Resume by session id
- the Factory skills, loaded from the worktree by name

What does each candidate offer for these, and where are the gaps? [codex-headless-harness.md](codex-headless-harness.md) answered the same question for Codex.

**Method.**

- Each CLI was run on one Ubuntu 24.04 machine where it was installed and signed in. Each run was inside a throwaway git repository with a linked worktree, using one-line prompts ("Reply with OK.") and at most six model calls per CLI.
- Versions: `muse` 1.4.2-R4684.1, `grok` 1.0.46, `mimo` 0.1.15 and `agy` 1.2.17. §6 adds `opencode` 2.0.24 and `claude` 2.1.291, and notes that agy is now 1.3.0.
- Facts also come from `--help`, the docs bundled with each install (Grok's `~/.grok/docs/user-guide/`), the changelog (agy) and the installed package's source (mimo).
- Each fact says how it was confirmed: **test** (a run on this machine), **help**, **docs** or **source**.
- No user config was changed. The trust experiments used scratch config directories.
- Auto-update was off for every call (`MUSE_NO_AUTO_UPDATE=1`, `GROK_DISABLE_AUTOUPDATER=1`), and every version stayed the same.

**Out of scope.**

- Whether to add each Harness, the design, and the order. Those are #428's to decide.
- How strong each model is. That is in [model-strength-for-review.md](model-strength-for-review.md) §0.

## TL;DR

- **All four do the basics.** Each one:
  - runs one prompt with no approval prompts and exits
  - takes a Model and an Effort
  - streams JSON events with a session id
  - resumes a session by id with a new prompt
  - loads project skills from `.agents/skills/` when the prompt names one as `/name`
  - can spawn sub-agents

  `.agents/skills/` is where thirdshift already links the Factory skills for Codex (ADR 0012).
- **How close each is:**

  | Harness · Model | Gaps for thirdshift |
  |---|---|
  | **agy** · Gemini 3.8 Flash | **None found.** Its exit codes are meaningful, its final `result` event carries the session id, text, usage and error, and it stops cleanly on SIGTERM. But start-up is slow and varies (8 s to 95 s), and it never reads `CLAUDE.md` or `.claude/skills/`. |
  | **Grok Build** · `grok-4.7` | Stopping it leaves the running shell command alive. Each repository needs folder trust once. |
  | **Muse Code** · `muse-spark-1.3` | Its sandbox can't start here, so `--yolo` is needed. Every run needs a trust flag. Its own JSONL carries no usage, and its final text runs replies together. Nothing lists the models for free. Auto-update must be turned off. |
  | **MiMo Code** · `mimo-v2.6-pro` | Not on cron's PATH. Exits 0 when it fails. No final event. Effort did nothing on the provider tested. Stopping it leaves the running shell command alive. |

- **Problems that recur:**
  - **Stopping:** Grok and MiMo start shell commands outside their own process group, so stopping the CLI's group orphans the command.
  - **Self-update:** Muse, Grok and agy update themselves unless told not to.
  - **Trust:** Muse and Grok load project instructions and skills only from a trusted folder.
  - **Instruction files differ:**
    - agy reads `AGENTS.md` and `GEMINI.md`, never `CLAUDE.md`.
    - Muse, Grok and MiMo also read the user's `~/.claude/CLAUDE.md`.
    - MiMo reads `CONTEXT.md` when a repository has neither `AGENTS.md` nor `CLAUDE.md`.

## 1. agy (Antigravity CLI 1.2.17), Gemini 3.8 Flash

**Working command** (exit 0, replied "OK"):

```
agy -p "<prompt>" --model gemini-3.8-flash --effort medium --output-format stream-json --dangerously-skip-permissions </dev/null
```

The log shows `model alias "gemini-3.8-flash" resolved to "gemini-3.8-flash-medium"`, so `--model gemini-3.8-flash-medium` alone does the same.

| Need | Finding | Confirmed by |
|---|---|---|
| Headless run | `agy -p/--print "<prompt>"`. `--input-format stream-json` reads one prompt per line from stdin, for several turns. Exit 0 on success, and 1 for a bad model or flag or when interrupted. The changelog says a model or API failure exits 3 with an `AGY_ERROR: {...}` line on stderr, and `--print-timeout` expiry exits 0 with partial output. | test (0, 1); help; changelog (3) |
| Model | `--model <slug>`. Gemini 3.8 Flash is `gemini-3.8-flash-low`, `-medium` or `-high` (High by default), or `gemini-3.8-flash` with `--effort`. `agy models` prints the catalog (`id<TAB>label`) without a turn. A wrong model exits 1 before any turn, listing the available models on stderr, with a `result` event whose `"status"` is `"ERROR"`. | test |
| Effort | `--effort low\|medium\|high\|xhigh\|max`. Gemini 3.8 Flash supports only low, medium and high: `--effort max` exits 1 ("has no "max" effort (available: low, medium, high)"), and the bare slug without `--effort` exits 1 ("requires --effort"). `agy -p /effort --output-format json` lists the levels without a turn. | help; test |
| Unattended | `--dangerously-skip-permissions` ("Auto-approve all tool permission requests"). The `init` event then shows `permission_mode: "always-proceed"`. Without it, print mode denies anything that needs approval. The sandbox is opt-in (`--sandbox`) and, per the changelog, makes `.git` read-only, which would block commits, so never pass it. No workspace-trust prompt appeared in print mode. A commit in a linked worktree was not tried. | help; test; changelog |
| Stream | `stream-json` is NDJSON.<br>• `init` carries `conversation_id`, model, cwd, tools and `permission_mode`.<br>• `step_update` carries a step index, state (`ACTIVE`/`DONE`), step type (`user_input`, `agent_response`, `tool`, `system_message`), streamed `text_delta`, the tool's name and info, and per-step usage.<br>• A final `result` carries `conversation_id`, `status` (`SUCCESS`/`ERROR`), `response` (the final text), `error`, `duration_seconds`, `num_turns` and usage (input, output, thinking, cache-read and total tokens). Usage is cumulative on a resumed conversation.<br>`--output-format json` prints only the `result` object. | test |
| Resume | `agy -p "<prompt>" --conversation <conversation_id>` kept the same id, quoted turn 1 word for word, and reported `num_turns` 2. `-c` continues the latest conversation in the workspace. | test |
| Skills | Reads `.agents/skills/<name>/SKILL.md` from the working directory up to the repository root. The docs also list `.agent/`, `_agents/` and `_agent/`. User-level skills live in `~/.gemini/config/skills/`. Symlinked skill directories are followed. It does **not** read `.claude/skills/` or `.gemini/skills/`. A prompt starting `/name [args]` is expanded in print mode, and the log shows `expanded slash command "probe-skill" (skill)`. A thirdshift-shaped prompt (`/probe-skill extra-argument` and a second line) worked. `--disable-slash-commands` turns expansion off. `agy -p /skills --output-format json` lists skills without a turn. | test |
| Sub-agents | `invoke_subagent`, `define_subagent` and `manage_subagents` are in the tool list. Custom agents go in `.agents/agents/`. The changelog says sub-agents get their own git worktrees. | test (tool list); changelog |
| Instruction files | `GEMINI.md` and `AGENTS.md` from the working directory up to the repository root, `.agents/rules/*.md`, and the globals `~/.gemini/GEMINI.md`, `~/.gemini/AGENTS.md` and `~/.gemini/config/`. **Never `CLAUDE.md`**: a test repository's `CLAUDE.md` codeword was unknown to it. | test; docs |
| Stopping | Clean on SIGTERM. A group SIGTERM gave exit 1 in 0.7 s, a final `result` with `"error":"interrupted"`, and `error: interrupted` on stderr. It also killed its own child process. | test |
| PATH under cron | A self-contained binary, found with a minimal PATH. Under `env -i` the system keyring lookup fails and it falls back to its token file; sign-in still worked. | test |

**Caveats.**

- **Latency.** Google's backend made wall time vary. Two of four one-line replies took 68 s and 95 s: one hit a 503 after about 60 s, the other a slow sign-in call. The other two took 8 s.
- **Auto-update.** It updates itself in the background at most every 15 minutes. The updater started once during the tests, but the binary stayed at 1.2.17.
- **Quota.** A personal Google sign-in carries a quota, which `agy -p /usage` shows.

## 2. Grok Build (`grok` 1.0.46), Grok 4.7

`grok` is a link to a static binary under `~/.grok/downloads/`. `agent` is the same binary. Full docs ship in `~/.grok/docs/user-guide/`.

**Working command** (exit 0, 5.5 s). `GROK_FOLDER_TRUST=0` was needed only because the scratch repository was untrusted.

```
GROK_DISABLE_AUTOUPDATER=1 GROK_FOLDER_TRUST=0 grok -p "<prompt>" -m grok-4.7 --reasoning-effort low --output-format streaming-messages-json </dev/null
```

**Unattended form that committed in a linked worktree:**

```
grok -p "<prompt>" -m grok-4.7 --reasoning-effort low --always-approve --output-format streaming-json
```

| Need | Finding | Confirmed by |
|---|---|---|
| Headless run | `grok -p "<prompt>"`, or `--prompt-file` / `--prompt-json`. It doesn't read stdin. Without `-p` it tries to start its full-screen interface and fails with no terminal. Exit codes 0, 1, 130 (SIGINT) and 143 (SIGTERM) were all seen. | docs; test |
| Model | `-m grok-4.7`. `grok models` lists grok-4.7 (the default), grok-4.7-build-fast, grok-4.6 and grok-4.5 without a turn. The JSON cache is `~/.grok/models_cache.json`. A wrong model is refused before any request: `{"type":"error","message":"Couldn't set model 'grok-9.9-bogus': Invalid params: \"unknown model id\"..."}`, exit 1. | test |
| Effort | `--reasoning-effort` (alias `--effort`). grok-4.7 takes low, medium, high (the default) and xhigh. An invalid value fails before any request ("use one of: xhigh, high, medium, low"), exit 1. The config key is `models.default_reasoning_effort`. | catalog; test |
| Unattended | `--always-approve`, or `--yolo`, or `--permission-mode bypassPermissions`. A commit in a linked worktree worked. A user config may already default to always-approve, so pass the flag explicitly rather than depend on it. The sandbox is off by default. `--sandbox workspace` limits writes to the working directory, `~/.grok` and `/tmp`, which would block writes to the main checkout's `.git`. | help; docs; test |
| Stream | Three formats:<br>• `json` is one object.<br>• `streaming-json` is NDJSON with text, thought, `tool_call` (the command shown up front), usage, end and error lines.<br>• `streaming-messages-json` has the shape of Claude Code's stream-json: system/init, assistant, user, result.<br>The session id is in `session_id` on `init` and `result`, in `end.sessionId`, and in `sessionId`. The final message is `result.result` or `text`. Errors are `{"type":"error"}` with a non-zero exit. Usage and cost are in `result.usage`, `modelUsage` and `total_cost_usd`. Nothing final is written when it is killed by a signal. | test |
| Resume | `grok -p "<prompt>" -r <session-id>` kept the same id and recalled the earlier reply. `-c` continues the latest session, and `--fork-session` exists. | test |
| Skills | Reads `.agents/skills/`, `.claude/skills/` and `.grok/skills/` (plus `commands/`), and user skills under `~/.grok`, `~/.claude` and `~/.cursor`. Symlinked skill directories are accepted. Grok expands a `/probe-skill` prompt itself, and the session history shows the skill body injected. Project skills need folder trust. | test |
| Sub-agents | A `spawn_subagent` tool is in the `init` tool list, and the docs say children run in parallel. Also `--agents <JSON>` and `--no-subagents`. | test; docs |
| Instruction files | From the repository root down: `AGENTS.md`, `AGENT.md`, `CLAUDE.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md`, and the `rules/` directories under `.grok`, `.claude` and `.cursor`. Plus `~/.grok/rules`, `~/.claude/CLAUDE.md` and `~/.claude/rules`. Project files load only in a trusted folder. | docs; `grok inspect`; test |
| Stopping | SIGTERM exits in 0.4 s with 143, and SIGINT with 130. **In both cases the shell command it was running kept running.** Grok starts each command in a process group of its own, so a signal to Grok's group doesn't reach it. | test |
| PATH under cron | Found with a minimal PATH. `--version` and `grok models` work under `env -i`. It relies on `~/.grok` for sign-in, config and sessions. It skips update checks when stderr isn't a terminal, or with `GROK_DISABLE_AUTOUPDATER=1`. Leader mode, a shared background process, is off by default. | test; docs |

**Trust.** Folder trust is needed for `AGENTS.md` and project skills. A linked worktree inherits it from its main checkout: in a scratch `GROK_HOME`, trusting the main checkout trusted its worktree, while trusting the worktree path itself or its parent did not. A repository needs a one-time `grok --trust` in its main checkout, which writes `~/.grok/trusted_folders.toml`, or `GROK_FOLDER_TRUST=0`.

**Cost.** The three one-line calls that finished normally reported $0.012, $0.003 and $0.007.

## 3. Muse Code (`muse` 1.4.2-R4684.1), Muse Spark 1.3

`muse` is a bash launcher that runs the static binary `muse-bin-<version>` beside it. Once an hour it updates itself in the background, unless `MUSE_NO_AUTO_UPDATE=1` is set.

**Working command** (exit 0):

```
MUSE_NO_AUTO_UPDATE=1 muse exec --json --trust-workspace --model muse-spark-1.3 --reasoning-effort low "<prompt>" </dev/null
```

**Unattended form that committed in a linked worktree:**

```
muse exec --json --yolo --model muse-spark-1.3 --reasoning-effort low "<prompt>"
```

| Need | Finding | Confirmed by |
|---|---|---|
| Headless run | `muse exec [options] PROMPT`. The prompt is an argument or `--prompt-file <file>`; stdin, `-` and `/dev/stdin` are refused. Exit 0 on success, 1 on a failed run, 2 on a usage error, 143 on SIGTERM. A shell command failing inside the run still exits 0. | test |
| Model | `--model muse-spark-1.3`. No command lists the models: `muse model-profile show` accepts any string. After any run, a JSON catalog is cached under `~/.local/share/muse/model-catalog/`, listing `muse-spark-1.3`, `muse-spark-1.3-contributor` (marked default; "content may be used for product improvement"), `-1.2` and `-1.2-contributor`. A wrong model reaches the API and fails in 1.6 s with exit 1: "model `muse-spark-9.9-bogus` does not exist or you lack access". | test |
| Effort | `--reasoning-effort none\|minimal\|low\|medium\|high\|xhigh\|max\|ultra`, default high. The settings key is `reasoning_effort`. The catalog lists minimal to max for 1.3, and `ultra` means max reasoning plus proactive multi-agent work. An invalid value fails before any call ("unsupported reasoning effort `bogus`", exit 2). | help; test |
| Unattended | `--yolo` turns off approval and the sandbox and trusts the workspace for that run. With it, `git commit` in a linked worktree worked with no prompts. The sandbox (bubblewrap) is on by default, and **on this Ubuntu 24.04 machine every sandboxed shell command fails** ("bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted") because `kernel.apparmor_restrict_unprivileged_userns = 1`. That is the same reason Codex runs unsandboxed (ADR 0012). | test |
| Stream | `--json` emits JSONL event envelopes of its own.<br>• The session id is `stream.id` on every line.<br>• Text arrives as `run.output.delta`.<br>• `run.terminal.completed.text` joins every reply in the run with no separator. When a built-in verify step made the model answer twice, it gave "MANGOOK MANGO".<br>• Errors are in `run.terminal.failed.reason`, plus one stderr line.<br>• A tool event says only `task_kind: tool.bash` until the tool finishes, then `tool.result` carries the command, exit code and output.<br>**Token usage is not in the stream.** It is in `~/.local/share/muse/sessions/<date>/<id>/session.jsonl` (`model_completed.usage`), or `muse export --session <id> --out <file>`, which works offline. | test |
| Resume | `muse exec --session-id <uuid> "<prompt>"` kept the same id and recalled the earlier reply. `--allow-workspace-switch` is needed if the session belongs to another workspace. `muse resume` is interactive only. | test |
| Skills | Reads `.agents/skills/` and `.claude/skills/`, but not `.muse/skills/`. Symlinked skill directories pointing outside the repository, as thirdshift links them, are accepted. User skills live in `~/.config/muse/skills`. Skills load only in a trusted workspace. The CLI doesn't expand a `/probe-skill` prompt itself; the model called its `read_skill` tool and replied correctly. | test |
| Sub-agents | `subagent_spawn`, `subagent_wait` and `subagent_cancel` tools, up to 8 in parallel (64 at max effort). Also `--agents <JSON>` and `--subagent-worktree-isolation`. Delegation is off in an untrusted workspace. Every run also starts hidden "reminder" child sessions. | strings in the binary; stderr |
| Instruction files | The project's `AGENTS.md`, or `CLAUDE.md` when there is no `AGENTS.md`. It also reads `~/.claude/CLAUDE.md` as user-level rules (`--no-foreign-personal-context` turns that off). Project files need trust. | test (offline echo provider and session log) |
| Stopping | SIGTERM exits in 0.4 s with 143, prints "received SIGTERM; flushed session logs", and kills the running child command. | test |
| PATH under cron | Found with a minimal PATH. `--version` and `exec --provider echo` run there. It needs bash through `/usr/bin/env`, and `curl` and `sha256sum` for updates. After an update it deletes the old `muse-bin-*` files. | test |

**Trust.** It is granted per exact path. Trusting the parent directory or the main checkout did not cover a linked worktree (tested in a scratch `XDG_CONFIG_HOME`), so every run and every Resume needs `--yolo` or `--trust-workspace`.

**Cost.** Each model call starts at about 26,500 input tokens. Muse reports no cost.

## 4. MiMo Code (`mimo` 0.1.15), MiMo-V2.6-Pro

MiMo Code is Xiaomi's fork of opencode, installed from npm (`@mimo-ai/cli`). On the test machine MiMo-V2.6-Pro was reachable only through a custom provider configured as the default model in `~/.config/mimocode/mimocode.jsonc`. The catalog also lists the first-party `xiaomi/mimo-v2.6-pro`, but no Xiaomi credential was configured, so that route was not tried.

**Working command** (exit 0, replied "OK"):

```
mimo run --format json -m <provider>/<model> --dangerously-skip-permissions "<prompt>" </dev/null
```

| Need | Finding | Confirmed by |
|---|---|---|
| Headless run | `mimo run [message..]`. If stdin is not a terminal, all of stdin is appended to the prompt, so stdin must be closed; thirdshift's `Stdio::null` is fine. Success exits 0, but **failures exit 0 too** (seen with an unknown model). An empty prompt exits 1. | test; source |
| Model | `-m <provider>/<model>`. `mimo models [provider] [--verbose]` prints the catalog without a turn. A wrong model prints `{"type":"error",...,"data":{"message":"Model not found: <provider>/no-such-model."}}` on stdout, and `ProviderModelNotFoundError` plus a dump of minified source on stderr, with **exit 0**. | test |
| Effort | `mimo run --variant <name>` ("provider-specific reasoning effort, e.g., high, max, minimal"). The valid names are the model's `variants` in `mimo models --verbose`: low, medium and high for `xiaomi/mimo-v2.6-pro`. The custom provider's entry has `variants: {}`, and the source ignores an unknown variant without a word. So on that route **effort does nothing**, unless a `variants` block is added to the provider's config. | help; source |
| Unattended | `--dangerously-skip-permissions` or `--yolo`, or the environment variable `MIMOCODE_DANGEROUSLY_SKIP_PERMISSIONS=1`, answers every permission request with "once". Without it, `run` rejects them all ("permission requested: …; auto-rejecting"). `run` sessions also deny `question` and `plan_exit`. There is no command sandbox. Writes outside the project are "ask" by default, which the flag approves. A commit in a linked worktree was not tried. | help; source; test |
| Stream | `--format json` emits JSONL objects `{type, timestamp, sessionID, part\|error}`.<br>• `step_start`<br>• `text`: the whole part once finished, with no streaming deltas<br>• `tool_use`: on completion or error<br>• `step_finish`: `part.tokens` (total, input, output, reasoning, cache read/write), `part.cost`, and `part.reason` (`"tool-calls"` or `"stop"`)<br>• `error`<br>• `reasoning`: only with `--thinking`<br>**There is no final result event.** The final message is the last `text` before the `step_finish` whose reason is `"stop"`, usage must be summed across steps, and the model id is not in the stream. | test |
| Resume | `mimo run -s <sessionID> "<prompt>"` kept the same id and quoted turn 1 word for word. Also `-c` (latest) and `--fork`. | test |
| Skills | `.agents/skills/**/SKILL.md`, in every `.agents` from the working directory up to the worktree root, plus `.mimocode/skills/` and the user-level `~/.agents/skills/`. Symlinked skill directories are followed, including a link pointing outside the repository. `.claude/skills/` is read **only** with `MIMOCODE_ENABLE_CLAUDE_CODE_SKILLS=1`. A `/name` anywhere in the prompt makes the server inject that `SKILL.md`, up to three per prompt; `--command <name>` and a model-called `skill` tool also work. `mimo debug skill` lists skills without a turn. | test; source |
| Sub-agents | An `actor` tool spawns `general` or `explore` sub-agents, "parallel + background, with lifecycle/cancel", up to `workflow.maxConcurrentAgents` at once. | bundled docs; `mimo agent list` |
| Instruction files | Every `AGENTS.md` from the working directory up to the worktree root. It reads `CLAUDE.md` when there is no `AGENTS.md`, **and also** when the `AGENTS.md` text totals under 500 characters. It reads `CONTEXT.md` only when neither exists. It also loads the first that exists of `$MIMOCODE_CONFIG_DIR/AGENTS.md`, `~/.config/mimocode/AGENTS.md` and `~/.claude/CLAUDE.md`. It never reads `GEMINI.md`. | test; source |
| Stopping | `run` has no SIGINT or SIGTERM handler, so either signal kills it outright: a group SIGTERM ended the wrapper and binary in 0.1 s, with no final event. **The shell command the agent was running survived as an orphan**, because it ran in a session of its own. | test; source |
| PATH under cron | **Not found** with a minimal PATH or a typical crontab PATH: `mimo` lives in an nvm node's `bin/`. That `mimo` is a `#!/usr/bin/env node` wrapper, which exits 127 when no `node` is on PATH. The real program, `.../@mimo-ai/cli/bin/.mimocode`, is a standalone binary that runs under `env -i` without node and finds its config and sign-in through `HOME`. | test |

**Side effects seen.**

- **Memory notes outside the worktree.** One test prompt was "Remember the number 417. Reply with OK." mimo's memory feature then wrote `~/.local/share/mimocode/memory/sessions/current_session_id/notes.md` with a shell command, which the permission flag approved.
- **Scheduled prompts.** These are on by default; `MIMOCODE_DISABLE_CRON` turns them off.

## 5. Against what thirdshift relies on

| thirdshift relies on | agy | Grok Build | Muse Code | MiMo Code |
|---|---|---|---|---|
| One-shot run, no approval prompts | `-p` + `--dangerously-skip-permissions` | `-p` + `--always-approve` | `exec` + `--yolo` | `run` + `--dangerously-skip-permissions` |
| Failure shown by exit code | yes | yes | yes | **no**: read the `error` events |
| Model and Effort checked before any work, free | `agy models`; a bad model or effort fails before a turn | `grok models`; both fail before a request | effort fails before a call; **a bad model reaches the API** | `mimo models`; **an unknown effort is ignored** |
| Session id, final text, usage in the stream | `init` and `result` | Claude-shaped `init` and `result` | id yes; **final text run together, usage only in the session log** | id yes; **no final event**, usage summed per step |
| Resume by id | `--conversation` | `-r` | `--session-id` | `-s` |
| Factory skills from the worktree | `.agents/skills/`, `/name` | `.agents/` or `.claude/skills/`, `/name` | `.agents/` or `.claude/skills/`, `/name`, trust | `.agents/skills/`, `/name` |
| Stop that ends its commands | yes | **no** | yes | **no** |
| Found on cron's PATH | yes | yes | yes | **no** |
| Needs turning off for unattended use | self-update | self-update (`GROK_DISABLE_AUTOUPDATER=1`) | self-update (`MUSE_NO_AUTO_UPDATE=1`) | scheduled prompts (`MIMOCODE_DISABLE_CRON`) |
| Per-repository setup | none seen | one-time `grok --trust` in the main checkout | none (trust flag on every run) | none seen |

## 6. Follow-up, 2026-10-06: OpenCode for MiMo-V2.6-Pro, and corrections

Grilling #428 raised these questions. The answers come from test runs on the same machine, using throwaway repositories with a linked worktree.

**Corrections to §1–§5.**

- **agy's self-update can be turned off.** The binary reads `AGY_CLI_DISABLE_AUTO_UPDATE`. With `=true` the log says "Auto-update disabled via environment variable". **`=1` is not honoured**, and the background updater ran. That run upgraded agy on the test machine from 1.2.17 to 1.3.0, so §1 describes 1.2.17. (test; strings in the binary)
- **Claude Code reads `AGENTS.md` when there is no `CLAUDE.md`.** In a repository whose only instruction file was an `AGENTS.md` holding a codeword, `claude -p` (2.1.291) answered with that codeword. (test)
- **MiMo Code can get Effort on the PrimaLabs route** by overriding the model entry per run through `MIMOCODE_CONFIG_CONTENT`, e.g. `{"provider":{"primalabs-ai":{"models":{"primalabs-ai/MiMo-V2.6-Pro":{"reasoning":true}}}}}`. With it, `mimo models --verbose` lists low, medium and high. An unknown variant is still ignored without a word. MiMo Code also has `MIMOCODE_DISABLE_AUTOUPDATE`. (test, catalog only)

**OpenCode 2.0.24 (`~/.opencode/bin/opencode`), MiMo-V2.6-Pro.** It reaches the model through the same PrimaLabs provider as MiMo Code, configured in `~/.config/opencode/opencode.json` as `primalabs/primalabs-ai/MiMo-V2.6-Pro`. By default each command is a client of a shared background service (`opencode serve --service`). `--standalone` starts a private server in the run's own process group instead.

Working command (exit 0; committed in a linked worktree; prompt on stdin):

```
OPENCODE_DISABLE_AUTOUPDATE=1 opencode run --standalone --format json -m 'primalabs/primalabs-ai/MiMo-V2.6-Pro#high' --auto <<< "<prompt>"
```

| Need | Finding | Confirmed by |
|---|---|---|
| Headless run | `opencode run`. Stdin is appended to the prompt, so the prompt goes on stdin and is stored exactly as sent. A prompt passed as an argument is stored inside literal quotes when it contains a space. Success exits 0; a bad model, a bad variant or a failed step exits 1. A run that fails still creates a session. | test; source |
| Model | `-m <provider>/<model>`. A wrong model exits 1 after about 2.5 s with `{"type":"error","error":{"type":"provider.no-route",...}}` and no model call. **No free catalog works without the service:** `models --standalone` and `api --standalone GET /api/model` returned nothing on every try. Through the service, `opencode api GET /api/model` lists each model with its variants. | test |
| Effort | A `#<variant>` suffix on the model. The PrimaLabs entry gets low, medium and high (`reasoningEffort`) with no config change. An unknown variant exits 1 before any turn: "Variant unavailable for …". Whether PrimaLabs honours the effort was not measured. | test |
| Unattended | `--auto` approves every permission that is not explicitly denied. A commit in a linked worktree worked. | help; test |
| Stream | JSONL `{type, timestamp, sessionID, part\|error}`: `step_start`, `text` (whole parts), `tool_use` (on completion), `step_finish` (tokens, cost, reason) and `error`. There is no final event, and **the last `step_finish` was missing in 4 of 7 successful runs**. `opencode session export --standalone <id>` works offline in about 1 s. It gives `info.outcome` (`succeeded`/`failed`), `info.tokens` and `info.cost` (which include the automatic title call), `info.model.variant`, and `messages[]`, where the final text is the last assistant message's last `text` part. | test; source |
| Resume | `run -s <id>` recalled turn 1, in standalone, for sessions made both in standalone and through the service. If the id doesn't exist, `-s` creates a new session. | test; help |
| Skills | Found in `.claude/`, `.agents/` and `.opencode/` from the working directory up, plus `~/.claude/` and `~/.agents/`. Symlinked skill directories are followed. **`run` does not expand `/name`.** The model loaded the skill by calling its `skill` tool, which shows in the stream as a `tool_use` event with `part.tool == "skill"` and `state.input.id` set to the skill's name. No flag injects a skill. | test; help |
| Instruction files | `AGENTS.md` from the working directory up to the project root, plus `~/.config/opencode/AGENTS.md`. **It never reads `CLAUDE.md`, and never reads `~/.claude/CLAUDE.md`.** | test; source |
| Stopping | **Through the service:** a group SIGTERM ends the client, but the turn and its shell command keep running in the service. SIGINT, or `POST /api/session/<id>/interrupt`, stops both. **Standalone:** a group SIGTERM exits 130 in 0.4 s and kills the command, and no process is left behind. | test |
| PATH under cron | A standalone binary that only `~/.bashrc` puts on PATH. Called by its absolute path under `env -i HOME=$HOME PATH=/usr/bin:/bin`, it works in standalone, and the credential loads with no service running. | test |
| Start-up | Standalone adds about 2–3 s. While it runs, the private server watches `/`, `/tmp`, the cwd and its ancestors, and `$HOME`, and stops every watcher on exit. | test |
| Needs turning off | `OPENCODE_DISABLE_AUTOUPDATE=1`. A snapshot of the working tree is taken into its own git store at every step (`~/.local/share/opencode/snapshot`). | source; logs |

**OpenCode compared with MiMo Code for MiMo-V2.6-Pro.** OpenCode fixes four of MiMo Code's gaps:

- Its exit codes are meaningful.
- The Model and the Effort both fail before any turn.
- Effort works on PrimaLabs without a config change.
- It runs from a minimal environment.

What it adds instead:

- It has no free catalog without the service.
- It doesn't expand `/name`.
- Its stream isn't reliable for usage, which `session export` covers.

## Sources

- **CLI help on the test machine:** `agy --help`, `agy models`, `grok --help`, `grok models`, `grok inspect --json`, `muse --help`, `muse exec --help`, `mimo --help`, `mimo run --help`, `mimo models --verbose`, `mimo debug skill`, `mimo agent list`.
- **Bundled docs:** Grok Build's `~/.grok/docs/user-guide/`; MiMo Code's package docs under `node_modules/@mimo-ai/`; agy's changelog.
- **Source:** MiMo Code's installed package, for stdin handling, the variant lookup, signal handling and instruction-file loading.
- **Test runs:** one-line prompts in throwaway repositories with linked worktrees, on 2026-10-05.
