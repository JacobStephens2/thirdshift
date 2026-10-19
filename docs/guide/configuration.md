# Configuration

## User config

A **User config** at `~/.thirdshift/config.toml` sets this machine's defaults for every Run. It is optional: with no file, or one that says nothing about a setting, a Run does only what its command asks for.

```toml
[merge]
always = true   # every Run is a Merge run, without the merge word

[base]
fix = true      # every Run may start a Base fix, without the base-fix word

[launch]
pull = true     # every Run first fast-forwards your checkout of the Base branch

[email]
always = true                                 # every Run sends a Run notification, without the email word
to = "you@example.com"                        # where email goes when the command names no address
from = "thirdshift@your-verified-domain.com"  # the sender; onboarding@resend.dev if unset

[logs]
dir = "~/elsewhere/logs"   # the root of the logs, instead of ~/.thirdshift/logs

[activity]
quiet_skips = true   # a skipped Pass prints nothing

[spec]
parallel = 2   # how many Tickets a Spec run runs at once, instead of 3

[pickup]
limit = 5   # how many open issues labelled in-progress stop a Pickup run taking another, instead of 3
wait_minutes = 60   # minutes since the latest shaping event before an issue is Ready, instead of 30; 0 disables waiting

[security]
review = true   # review each Run's change for Security findings
harness = "claude"   # the Harness for Security runs; blank or missing follows harness.default

[harness]
default = "claude"   # the Harness every session runs on; claude unless set

[harness.claude]
model = "opus"   # the Model Claude Code's sessions run on; blank for its own default
effort = "high"  # how hard that Model reasons; blank for Claude Code's own default

[harness.codex]
model = ""
effort = ""

[harness.agy]
model = "gemini-3.8-flash"
effort = "medium"
```

`thirdshift setup` writes this file for you, listing every setting at its default so the file itself shows what can be changed:

```toml
[merge]
always = false   # every Run is a Merge run, without the merge word; default false

[base]
fix = false      # every Run may start a Base fix, without the base-fix word; default false

[launch]
pull = false     # every Run first fast-forwards your checkout of the Base branch; default false

[email]
always = false                  # every Run sends a Run notification, without the email word; default false
# to = "you@example.com"        # where email goes when the command names no address; no default
from = "onboarding@resend.dev"  # the sender; default onboarding@resend.dev, which only delivers to your Resend account's address

[logs]
dir = "~/.thirdshift/logs"   # the root of the logs, each repository's in <owner>/<repo>/; default ~/.thirdshift/logs

[activity]
quiet_skips = false   # a skipped Pass prints nothing, leaving only its Activity log line; default false

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3

[pickup]
limit = 3   # how many open issues labelled in-progress stop a Pickup run taking another; default 3
wait_minutes = 30   # minutes since the latest shaping event before an issue is Ready; 0 disables waiting; default 30

[harness]
default = "claude"   # the Harness every Run's sessions run on, claude, codex, agy, grok, muse or opencode; default claude

[harness.claude]
model = ""    # the Model Claude Code's sessions run on; default blank, for Claude Code's own
effort = ""   # how hard that Model reasons; default blank, for Claude Code's own

[harness.codex]
model = ""    # the Model Codex's sessions run on; default blank, for Codex's own
effort = ""   # how hard that Model reasons; default blank, for Codex's own

[harness.agy]
model = ""    # the Model Antigravity CLI's sessions run on; default blank, for agy's own
effort = ""   # how hard that Model reasons; default blank, for agy's own

[harness.grok]
model = ""    # the Model Grok Build's sessions run on; default blank, for Grok Build's own
effort = ""   # how hard that Model reasons; default blank, for Grok Build's own

[harness.muse]
model = ""    # the Model Muse Code's sessions run on; default blank, for Muse Code's own
effort = ""   # how hard that Model reasons; default blank, for Muse Code's own

[harness.opencode]
model = ""    # the Model OpenCode's sessions run on, provider/model; default blank, for OpenCode's own
effort = ""   # the Model's variant, passed as #effort; default blank, for OpenCode's own

[security]
fix = false   # Security runs may fix reproduced findings; default false
review = false   # Runs review their change for Security findings; default false
harness = ""   # the Harness a Security run uses unless its command names one; default blank; blank or missing uses harness.default, or Claude Code if harness.default is missing
```

Every key holds its real value, so a Run reading it does exactly what it does with no file. `email.to` has no default, so `setup` suggests one: the public email of your GitHub profile (from `gh api user`), else your global git `user.email`, unless that is a `@users.noreply.github.com` address, which can't receive mail. With neither, `email.to` is the only line written commented out, as above. `setup` never asks `gh` for more scopes, so a private GitHub email is not read, and a Run never looks the suggestion up: `--email` with no address and no `email.to` still stops the Run. From a terminal (stdin and stderr both terminals), `setup` first asks, on stderr:

1. The Harness for every Run's sessions (`harness.default`), listing only those installed on `PATH`. An unavailable Harness or unknown name is refused and asked again. The default is the configured Harness when installed, else the first installed in `claude`, `codex`, `agy`, `grok`, `muse`, `opencode` order. Then the Model (`harness.<name>.model`) and the Effort (`harness.<name>.effort`) for the Harness chosen only, each defaulting to the User config's, where Enter on none keeps the Harness's own default and `-` clears one. They're checked as a Run checks them (see [Harness, Model and Effort](#harness-model-and-effort)). For Claude, a named Model gets the test call; if Claude refuses it, Setup shows what Claude said and asks the Model and Effort again. For Codex, Setup lists the Models `codex debug models` offers, by slug and display name, then the Efforts the Model chosen supports; each answer is matched regardless of case and written as Codex names it, so `GPT-6.1-Sol` and `Max` are written as `gpt-6.1-sol` and `max`, and a Model or Effort the catalog doesn't have is asked again, with the valid choices. For agy, Setup lists `agy models` and its supported Efforts, checks each answer without a model turn, and writes the canonical names regardless of case. For Grok Build, Setup lists `grok models` and the chosen Model's Efforts from its refreshed cache, and checks each answer against them. Muse proposes `muse-spark-1.3` and checks the answers against its cache, or with a minimal call; a refusal asks for the Model and Effort again. OpenCode proposes no Model and checks the Model and Effort with a minimal standalone call, asking again on a refusal. The other Harnesses' sections are left as they are. With no Harness installed here, Setup says so, asks none of this, and leaves the harness settings as they are.
2. Every Run a Merge run? (`merge.always`)
3. With that on, every Run may start a Base fix when the Base branch's CI is red? (`base.fix`). With it off, this is not asked, and `base.fix` is written at its default, `false`.
4. Every Run first fast-forwards your checkout of the Base branch? (`launch.pull`)
5. Security runs may fix reproduced findings? (`security.fix`), off by default.
6. Runs review their change for Security findings? (`security.review`), off by default.
7. Run notifications? (`email.always`). If yes, the address (`email.to`), asked again until it has an `@`, then the sender (`email.from`).
8. With notifications on, the Resend API key, with input hidden. With none saved it asks `Resend API key (input hidden, Enter to skip):`; with one in the [Credentials](#email), `Resend API key (input hidden, Enter keeps the saved one):`, and a new one replaces it. Surrounding spaces are trimmed, and anything that doesn't start with `re_` is asked again. A key you give is saved in the Credentials, `~/.thirdshift/credentials.toml`, created with mode 600 (and `~/.thirdshift` with it) or edited in place, keeping its comments and anything else in it and changing only `resend.key`; `setup` then prints `wrote the Credentials <path>`. The key is never printed, nor written to the User config. Skipping writes no Credentials and says how to add a key later: rerun `thirdshift setup`, or set `RESEND_API_KEY`. With `RESEND_API_KEY` set and not empty, which wins over the Credentials, nothing is asked, and it says the key comes from `RESEND_API_KEY`. With a key found or given, it offers to send a test email (default No), as `thirdshift email-test` does, once the files are written.

Pressing Enter takes the default shown, which is the file's current value, or else the setting's default, and for the address the suggested email above. `logs.dir`, `activity.quiet_skips`, `spec.parallel`, `pickup.limit`, `pickup.wait_minutes` and `security.harness` are not asked about. Setup adds a missing `security.harness` at its default, blank, with a comment explaining it; a configured Harness is kept. The answers are written like everything else below: in place, keeping your comments. Ctrl-C, SIGTERM or SIGHUP during the questions or a Model/Effort check exits `1` with `interrupted` and writes nothing: neither the User config nor the Credentials. Ordinary and hidden questions stop without another line of input, and hidden input restores the saved terminal settings, including echo. With notifications off, nothing about a key is asked, and saved Credentials stay as they were, so `--email` on a single Run still works. Credentials a Run would refuse (see [Email](#email)) are refused before any question, exit `1`, and not touched. With no terminal, as from cron or `thirdshift setup </dev/null`, `setup` asks nothing and never writes the Credentials. Either way it prints the file's path on stderr and exits `0` with stdout empty. Over a User config that is already there, `setup` edits it in place: its comments and key order stay, as do the values it didn't ask about, and each key it lacks is added at its default with its comment, so afterwards the file lists every setting this version knows. If `[security]` falls between Harness sections, Setup moves it after them, keeping its values and comments. One that already lists every setting, down to the commented-out `email.to` line, with Security outside the Harness area is left byte for byte as it was. A key added to an inline table, such as `launch = { pull = true }`, gets no comment, since TOML has no place for one there. One a Run would refuse is refused the same way, exit `1`, and not touched. Any argument after `setup` is an argument error (exit `2`).

The first Run on a machine with no User config, started from a terminal, offers Setup before any work, on stderr: `No User config at <path>. Set your defaults now? [Y/n]`. Yes (or Enter) asks the questions above, writes the file, and the Run carries on using your answers; a flag in the command, such as `--no-merge`, `--email` or `--no-email`, still wins over them. No writes every setting at its default, as `setup` with no terminal does, asks nothing about a key, writes no Credentials, says that `thirdshift setup` changes it, and the Run carries on; later Runs find the file and don't offer again. If the file can't be written, stderr gets a `warning:` line and the Run carries on with the defaults. Ctrl-C, SIGTERM or SIGHUP during the offer, the questions or a Model/Effort check exits `1` with `interrupted`, writes nothing and ends the command before any work, with no Run notification. A command thirdshift can't parse exits `2` before any offer. A Run with no terminal, as from cron, CI, `nohup` or an agent's shell, offers nothing, writes nothing, and runs on the defaults, so a later Run from a terminal still gets the offer.

With `merge.always = true`, `thirdshift <Issue URL>` is a Merge run, and `thirdshift --no-merge <Issue URL>` (or `no-merge`, before or after the URL) leaves that one Run's pull request ready for review.

With `base.fix = true`, every Run may start a [Base fix](runs.md#base-fix), as if given `base-fix`, and `thirdshift --no-base-fix <Issue URL>` (or `no-base-fix`, before or after the URL) forbids it for that one Run.

With `launch.pull = true`, every Run brings the Base branch checked out in the directory you start it from (the **Launch directory**) up to date with `origin`, so you no longer `git pull` by hand before each Run. It happens after the pre-flight checks pass and before the worktree is created, as `git merge --ff-only origin/<Base branch>`: fast-forward only, never a merge commit or a rebase, always from `origin`, whatever the branch's upstream or your `pull.*` settings. A progress line on stderr says when it updates the branch; an already up-to-date branch is left quietly as it is. It is skipped when the checked-out branch isn't the Base branch, as in a Continuation whose open pull request targets another base, or on a detached HEAD. If the update can't happen, for example because uncommitted changes are in the way, stderr gets a `warning:` line with git's error and the command to run by hand, your changes are left as they were, and the Run carries on with the same outcome and exit code. The setting only affects your checkout: the Run's worktree starts from `origin/<Base branch>` either way.

With `email.always = true`, every Run sends a [Run notification](#email) to `email.to`, as if given `--email`, and `thirdshift --no-email <Issue URL>` (or `no-email`, before or after the URL) sends none for that one Run.

`spec.parallel` sets how many Tickets a [Spec run](runs.md#spec-runs) runs at once, by default 3. It must be a whole number from 1 up; `parallel <n>` on the command line wins over it for one Spec run.

`pickup.limit` sets the [Claim limit](passes.md#the-claim-limit): how many open issues labelled `in-progress` stop a [Pickup run](passes.md#pickup-runs) from taking another, by default 3. It must be a whole number from 1 up. There is no command-line flag for it.

`logs.dir` sets the root of the [logs](observing.md#logs), with each repository's in its `<owner>/<repo>/` folder, created if missing. It must be an absolute path, `~` or a path starting with `~/`, where `~` stands for `$HOME`. A relative path stops the Run before any work, since the directory a Run is launched from is no base for a setting that holds for every Run.

`[harness]` chooses the **Harness**, the agent CLI every session of a command runs on, and for each Harness the **Model** its sessions run on and their **Effort**, how hard that Model reasons. See [Harness, Model and Effort](#harness-model-and-effort).

A Run reads the file before any work. One that isn't valid TOML, or that has a key or section thirdshift doesn't know, such as `alway` for `always`, or a value of the wrong type, such as anything but `true` or `false` for `always`, or `0` for `spec.parallel` or `pickup.limit`, stops the Run with an error naming the file and the offending key, so a typo can't silently leave a setting off. `email-test` and `setup` read it the same way. `update`, `version` and `help` never read it, so a broken User config can't block them, and they never offer Setup; nor does `email-test`.

## Harness, Model and Effort

The available Harnesses are Claude Code (`claude`), Codex (`codex`), Antigravity CLI (`agy`), Grok Build (`grok`), Muse Code (`muse`) and OpenCode (`opencode`).

Every session of a command, its implement session, each Repair and Resume, a Spec review, an Architecture review, each Ticket's Run and a Base fix, runs on one **Harness**, with one **Model** and one **Effort**. `harness <name>`, `model <name>` and `effort <level>` (or `--harness`, `--model` and `--effort`) choose them for one command, before or after the Issue URL, and on `architect` (but not with `--plan-only`) and `pickup` too:

```sh
thirdshift https://github.com/<owner>/<repo>/issues/<n> model claude-opus-5-5 effort high
```

For each of the three, the command wins, then the [User config](#user-config), then the default. The Harness's default is `claude`; the Model's and the Effort's are nothing, which leaves them to the Harness, so with no flag and no `[harness]` section sessions run exactly as `claude -p` would. A Model and Effort set in the User config come from the chosen Harness's own section, so `harness claude` over a `codex` default takes `[harness.claude]`, while `model` and `effort` on the command apply to whichever Harness is chosen. Names are passed to Claude Code as given: `--model` and `--effort` when set. Grok Build takes `-m <model>` and `--reasoning-effort <effort>`. Codex takes them as it names them, which the check below settles: `-m <model>` and `-c model_reasoning_effort="<effort>"` when set.

With `harness codex`, every session runs `codex exec --json --dangerously-bypass-approvals-and-sandbox` in the Run's worktree, with stdin set to null, and with `-c project_doc_fallback_filenames=["CLAUDE.md"]`, so Codex reads `CLAUDE.md` where a repository has no `AGENTS.md`. Codex's sandbox is off: on a stock Ubuntu machine it can't start, and commits and pushes from a worktree write outside it, so the worktree is the only boundary, as with Claude's auto mode ([ADR-0012](../../docs/adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)). The Factory skills are linked into the worktree's `.agents/skills/`, kept out of git with `/.agents/skills/thirdshift-*` in `.git/info/exclude`, and each prompt's first line loads its skill as `$thirdshift-<skill>`. `codex exec` kills what the agent left running as it exits, so a session whose turn ends with a command or a sub-agent call still running (started, never completed) gets a Resume, as on Claude: `codex exec … resume <session id>`, with the Model, Effort, bypass flag and `CLAUDE.md` fallback given again, as Codex keeps none of them. An interrupt sends a Codex session SIGINT, the only signal Codex stops cleanly on, before the SIGTERM and SIGKILL every session gets, each 10 seconds apart. Progress lines come from Codex's `item.*` events, the session-ended line gives its token totals, as Codex reports no turns or cost, and the final message is the last `agent_message`. A session whose turn fails (`turn.failed`), or that exits non-zero, fails the Run with Codex's error in the cause; an `error` event Codex retries does not. Each Codex session adds a `trust_level = "trusted"` entry for the repository to `~/.codex/config.toml`, and your `~/.claude/` instructions and hooks don't reach it.

With `harness agy`, every session runs Google **Antigravity CLI** as `agy -p --dangerously-skip-permissions --output-format stream-json`, with `--model` and `--effort` when set, stdin null, and `AGY_CLI_DISABLE_AUTO_UPDATE=true` (agy ignores `=1`). The sandbox is opt-in and is never enabled, so unattended sessions can write the shared `.git` ([ADR-0013](../../docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). Factory skills are linked into `.agents/skills/`, excluded from git, and loaded by the `/thirdshift-<skill>` first line. agy reads project `AGENTS.md`, `GEMINI.md` and `.agents/rules/`, and user instructions under `~/.gemini/`; it does not read `~/.claude/`. When root `CLAUDE.md` exists and neither `AGENTS.md` nor `GEMINI.md` does, thirdshift adds a `GEMINI.md` link to `CLAUDE.md` and excludes it through `.git/info/exclude`; existing instruction files always win.

agy's `init` supplies the conversation id and `step_update` supplies tool progress. The final `result` supplies the final message, success/error status, error and cumulative usage (including on Resume). `ERROR` or a nonzero exit fails the session with agy's error. A Resume uses `--conversation <id>` with the same flags, Model, Effort and environment; an interrupt sends SIGTERM, then SIGKILL after the grace period. Its start-up can be slow and variable: the research measured 8–95 seconds. No speed assumption or backend retry is made.

Before any work on agy, a named Model or Effort is checked against the free `agy models` catalog, which lists IDs, labels and effort-suffixed variants. IDs, display labels and bare aliases are matched regardless of case, as are Efforts. `model Gemini-3.8-Flash effort Medium` becomes `--model gemini-3.8-flash --effort medium`; `model gemini-3.8-flash-medium` already carries its Effort. A bare alias requires an Effort; a Model or Effort the catalog lacks fails naming valid choices. This check uses null stdin and disables self-update too, without a model turn.

With `harness grok`, every session runs `grok -p <prompt> --always-approve --sandbox off --output-format streaming-messages-json` with stdin set to null and with `GROK_DISABLE_AUTOUPDATER=1` and `GROK_FOLDER_TRUST=0`. The approval flag and disabled sandbox are always explicit, overriding the CLI’s own config and `GROK_SANDBOX`. No repository needs a one-time `grok --trust`: the trust environment override allows its project instructions and skills to load in the Run's worktree. The Factory skills are linked into `.agents/skills/`, excluded from git, and the prompt loads its skill as `/thirdshift-<skill>` on its first line. A Resume repeats the flags and environment with `-r <session id>`. Grok documents no stream notification proving that background work was killed at exit, so thirdshift does not infer an automatic Resume from its tool activity. Grok's Messages stream shares Claude's event shapes, with Grok's tool names and errors; it supplies progress, the session id, final message, turns and cost, or token usage when cost is unavailable. Grok starts shell commands in their own process groups, which thirdshift's tree-stop also stops; the CLI gets SIGTERM before SIGKILL.

Grok reads both `AGENTS.md` and `CLAUDE.md` when both exist, along with `AGENT.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md` and project rules under `.grok`, `.claude` and `.cursor`. It also reads `~/.grok/rules`, `~/.claude/CLAUDE.md` and `~/.claude/rules`; thirdshift writes none of these user-level files. Grok Build uses `~/.grok` for sign-in and sessions (or `GROK_HOME` when set).

Interrupting a Command stops every session's process tree on every Harness, including commands that started their own process group or session. The same owner supervises all Model/Effort checks, from Command startup and terminal Setup: interruption stops the check and its owned commands, even after the CLI exits while its captured output pipes are still open. A cancelled Command check ends before any Claim, worktree or Command log. thirdshift snapshots the descendants with `ps` on Linux and macOS, sends each process group the Harness's stop signals with a 10-second grace period each, then takes another snapshot before SIGKILL to catch commands started in the meantime. If the CLI exits on an earlier signal, thirdshift kills the remaining commands immediately without another grace period.

Before any work, where an Origin match failure would stop it, and so before the Claim, the worktree and any Command log, a command checks what it chose: the Harness's CLI must be on `PATH`, else it fails naming the CLI and the setting that chose it; and on Claude a named Model gets a minimal test call, `claude -p --model <model>` with the Effort, if any, which must succeed, else the command fails with what Claude said. No test call is made when no Model is named. On Codex a named Model and Effort are checked against `codex debug models`, which costs no tokens: each matches regardless of case, a Model by its slug or its display name, and becomes the name Codex takes, so `model GPT-6.1-Sol effort Max` reaches Codex as `gpt-6.1-sol` and `max`. An unknown Model, or an Effort that Model doesn't support (with no Model, one no Model supports), fails naming the valid choices. On Grok Build, `grok models` lists the available Models without a model call and refreshes `models_cache.json` under `GROK_HOME` or `~/.grok`; thirdshift reads that cache for each Model's supported Efforts. Both are matched regardless of case and passed as Grok names them. With no Model named, a named Effort is checked against Grok's current default Model. Unknown Models and unsupported Efforts fail with the valid choices, and an unreadable catalog fails before work. The check has the same null stdin and fixed environment as sessions. What the command records is what the check settled on. A Pass makes these checks only once it has decided not to skip, and a Spec run makes them once for all its Tickets.

With `harness muse`, every session and Resume runs `muse exec --json --yolo`, with stdin null and `MUSE_NO_AUTO_UPDATE=1`. `--yolo` disables approvals and the sandbox and trusts the worktree ([ADR-0013](../../docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). Muse reads the project's `AGENTS.md`, falling back to `CLAUDE.md`, and also reads `~/.claude/CLAUDE.md`. The Factory skills are linked into `.agents/skills/` and kept out of git. The prompt's first line tells the model to load its `thirdshift-<skill>` with its skill tool. If the stream shows no `read_skill` load of that skill, thirdshift writes a warning to the progress lines and Command log; it neither fails nor retries the session.

Muse's named Model is checked against its cached catalog under `~/.local/share/muse/model-catalog/` when present, matching its id or display label regardless of case. Without that cache, thirdshift makes a minimal test call with the same flags and environment. Effort is checked locally before any call: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultra`. Model and Effort reach Muse as `--model` and `--reasoning-effort`. Setup proposes `muse-spark-1.3`; leaving the Model unset uses Muse's own default, `muse-spark-1.3-contributor`, which allows content to be used for product improvement.

Muse's session id comes from `stream.id`, and a Resume uses `--session-id <id>` with every flag and environment override again. Progress lines name session starts, tools and errors. After exit, thirdshift reads the last reply and token totals from Muse's own `sessions/<year>/<month>/<day>/<id>/session.jsonl`. Architecture reviews see that last reply alone, rather than Muse's terminal event's joined replies. A missing or unreadable log falls back to the stream text with no usage. `run.terminal.failed` or a non-zero exit fails the Run with Muse's error. An interrupt sends SIGTERM, then SIGKILL after ten seconds if needed.

With `harness opencode`, every session, Resume and preflight check runs `opencode run --standalone --format json --auto`, with the prompt on stdin and `OPENCODE_DISABLE_AUTOUPDATE=1`. Standalone starts a private server in the session's process tree and adds about 2–3 seconds per session; it keeps the work and its commands under thirdshift's interruption handling rather than in the shared service ([ADR-0013](../../docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). OpenCode reads project `AGENTS.md` and `~/.config/opencode/AGENTS.md`; it never reads `CLAUDE.md` or `~/.claude/CLAUDE.md` itself. When root `CLAUDE.md` exists and `AGENTS.md` does not, thirdshift links `AGENTS.md` to it and excludes that link from git. Factory skills are linked into `.agents/skills/`, and the prompt tells the model to load its skill with the skill tool. A missing successful `skill` load produces a warning without failing or retrying the session.

OpenCode takes the Model id from your provider config, as `<provider>/<model>`, including MiMo-V2.6-Pro when your provider routes it. Model and Effort are passed together as `-m <provider>/<model>#<effort>`; an explicit Effort replaces a variant already in the Model id, and needs a Model set too. Before any work, thirdshift makes a minimal standalone test call with the same stdin, flags and environment. `provider.no-route` identifies a bad Model or Effort without a model turn; a successful check uses a model call because OpenCode has no free catalog without its service. Setup checks its answers the same way, proposing no default Model.

The session id comes from `sessionID` in the stream, and a Resume uses `-s <id>` with every flag and environment override again. OpenCode creates a new session for an unknown id, so thirdshift only resumes an id it read from the stream. Progress comes from `step_start`, `tool_use`, `step_finish` and `error`. After exit, thirdshift reads `opencode session export --standalone <id>`: an exported `failed` outcome fails the Run even with exit 0, the final message is the last assistant message's last text part, and usage and cost are summed over assistant messages to exclude the automatic title call. Input totals include cache reads and writes, output totals include reasoning, and each subtotal is shown. If export fails or is unreadable, the last stream `text` supplies the final message and no usage is shown. A stream error or nonzero exit also fails the Run with OpenCode's error. An interrupt sends SIGTERM through the whole process tree, then SIGKILL after the grace period if needed.

The Command log's opening lines (`sessions run on claude · claude-opus-5-5 · high`), the Activity log's start line, the pull request's body and the Run notification each name the Harness, Model and Effort, `default model` or `default effort` where the Harness's own is used. In the pull request's body, the Spec PR's included, thirdshift writes the line itself once the opening session has ended, such as `Built with claude · claude-opus-5-5 · high`, replacing any it wrote there before.

## Email

thirdshift sends email itself, with one HTTPS request to [Resend](https://resend.com)'s API, so it needs no mail server on the machine and works where SMTP ports are blocked ([ADR 0005](../../docs/adr/0005-run-notifications-through-resend.md)). It needs a Resend account and an API key:

- **The API key** comes from the **`RESEND_API_KEY`** environment variable when it is set and not empty, and otherwise from the **Credentials**, `~/.thirdshift/credentials.toml`, a file only you should be able to read (mode 600):

  ```toml
  [resend]
  key = "re_..."
  ```

  `thirdshift setup` asks for the key, hidden, and saves it there for you. The environment variable wins, so a CI secret or an `export` overrides the saved key for one command. The Credentials are what let a Run started from cron, `nohup` or an agent's shell find the key, since those read no shell profile. thirdshift never reads the key from the User config, so that file holds no secret.
- **`email.to`** in the [User config](#user-config) is the address email goes to when the command gives none.
- **`email.from`** is the sender. Without it, email comes from **`onboarding@resend.dev`**, Resend's shared sender, which only delivers to the address of your own Resend account. To send to any other address, set `email.from` to an address on a domain you have verified with Resend.

To check the setup without starting a Run:

```sh
thirdshift email-test you@example.com   # or just `thirdshift email-test`, to send to email.to
```

It sends one test email, whose subject marks it as a test and whose body names the host, the time and the sender. Before sending, it checks that it has an address (the argument, else `email.to`) and a key (`RESEND_API_KEY`, else the Credentials); if either is missing, it exits `1` naming what's missing and sends nothing. With no key anywhere, it lists every way to give one:

```
no Resend API key. Either:
  - run `thirdshift setup`, or
  - add it to /home/you/.thirdshift/credentials.toml (mode 600):
        [resend]
        key = "re_..."
  - or set RESEND_API_KEY in the environment the Run starts from
    (a crontab line, CI secret, or a shell profile the Run's shell reads)
```

The Credentials are read only when `RESEND_API_KEY` is unset or empty, and only by `email-test`, `setup`, and a Run or a Pass that asks for a notification. A missing file just means no key from it. One that isn't valid TOML, or holds anything but a quoted `resend.key`, such as a typo like `kye`, stops the command with exit `1`, naming the file and the offending key, and nothing is sent. One that others can read is still used, with a `warning:` line on stderr saying to `chmod 600` it. Nothing is sent to check the key itself. When Resend accepts the email, it prints `accepted by Resend; check your inbox` and exits `0`; that is all it can verify, so check that the email arrives. When Resend refuses it, for example for a bad key or a sender it won't send from, it prints Resend's error text word for word, with where the key came from, and exits `1`. It gives up after 30 seconds without an answer.

#### Run notifications

`--email` (or `email`) asks a Run for a **Run notification**: one email, sent when the Run ends, whatever the outcome. The word after the flag is the address only if it contains `@` and doesn't start with `https://`, so the Issue URL is never taken for it; otherwise the email goes to `email.to`. With `email.always = true` in the [User config](#user-config), a Run asks for one without the flag, and `--no-email` (or `no-email`) skips it for that Run; an address after `--email` still wins over `email.to`. Giving a flag twice, or `--email` together with `--no-email`, is an argument error.

A Run that asks for a notification, by the flag or by `email.always`, makes the same checks as `email-test` before any other work: an address is known, and a key is found, in `RESEND_API_KEY` or else the Credentials. If either fails, or the Credentials are broken, the Run stops, exits `1` naming what's wrong (with no key, the message above listing every way to give one), and sends nothing. A Run that asks for no notification never reads the Credentials, so a broken file can't stop it, and `update`, `version` and `help` never read it either. Once they pass, every way the Run ends sends exactly one notification, after its outcome is final and its cleanup done: ready for review, merged, a [Failed run](runs.md#failed-runs) (including a later preflight failure such as an origin mismatch), or interrupted by Ctrl-C, SIGTERM or a closed terminal.

- **Subject**: `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, where the outcome is `ready for review`, `merged`, `failed` or `interrupted`. The title is left out if it can't be read from GitHub.
- **Body**, plain text: a `Result:` line with the same outcome as the subject, so it is readable even when a long issue title hides the end of the subject; immediately followed by one `Issue:` line with the full original Issue URL, including after failure or interruption with no pull request; then the pull request URL (if any), the failure cause (if failed), after an Inherited failure with no Base fix taken its [`Base check:`, `Retry with:` and `Or set:` lines](runs.md#an-inherited-failure-links-the-base-branchs-checks), the session log path (if any), the [Command log](observing.md#logs) path as a `Command log:` line, the hostname and how long the Run took.

The email's `Built with` line names the command's configured Harness, Model and Effort and is marked `(configured)`. A `Session models` section lists sessions whose Harness was launched, including failed or interrupted sessions, Resumes and child Runs belonging to that Command. Models evidenced by the Harness's output or retained records are marked `(reported)`, including model switches and delegated Claude answers. When the Harness supplies no model evidence, the requested Model is labelled `(requested; actual model not reported)`, or its default is labelled `Harness default (actual model not reported)`. A listed session proves the Harness was launched; an unconfirmed request does not prove that Model executed. Effort is the session's configured Effort; it does not describe a delegated answer's Effort. Attribution records (`*.models.json`) are kept beside the Session logs; a failure to write or read them is a warning and never changes the Run's outcome.

A [Spec run](runs.md#spec-runs) sends at most one notification for the whole Spec, under the same rules, with its checks made once before any Ticket starts. Its subject names the Spec, its `Issue:` line links the parent Spec rather than a Ticket or pull request, its outcome is the Spec run's, and its body, after what a Run's holds (the Spec PR, if any), lists each Ticket's outcome, one line per Ticket as in the summary on stderr, such as `#21 landed with https://github.com/acme/widgets/pull/1` or `#22 blocked by #21`. The Ticket Runs inside it never send a notification of their own, whatever the User config says.

An [Architect run](passes.md#architect-runs) takes the same flags and the same `email.always`, and sends [one notification](passes.md#one-run-notification) covering its review and the run it dispatched.

A [Pickup run](passes.md#pickup-runs) takes the same flags and the same `email.always` too. One that took an issue sends [one notification](passes.md#one-run-notification-for-the-issue-taken), the one the run it dispatched would have sent by hand, and a skipped one sends none.

A notification that can't be sent is a `warning:` line on stderr with Resend's error. It never changes the Run's outcome, stdout or exit code.
