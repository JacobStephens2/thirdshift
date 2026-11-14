# Grilling: #1468, Custom-runner Arm for processes thirdshift doesn't run itself

Issue: https://github.com/JacobStephens2/thirdshift/issues/1468
Research: [eval-process-arms-and-human-review.md](../../research/eval-process-arms-and-human-review.md), [process-comparison-cost-accounting.md](../../research/process-comparison-cost-accounting.md), [process-comparison-grading-and-evidence.md](../../research/process-comparison-grading-and-evidence.md), [1484-implement-suite.md](1484-implement-suite.md), [ADR 0027](../../adr/0027-a-trial-carries-out-a-process-but-is-not-a-command.md), [ADR 0028](../../adr/0028-who-decides-a-verdict-is-fixed-in-the-bar.md), [ADR 0032](../../adr/0032-custom-runners-run-inside-one-trial-sandbox.md)

## Round 1 (answered)

**Q1.1 - The term, and which Arm may be one (`GLOSSARY.md`).** Today `GLOSSARY.md` defines **Arm** as requiring *"a build of thirdshift, with its Factory skills and Session prompts, the Process its Trials carry out..."*, and **Trial** as *"an agent session of its own, or every session of the Process the Arm carries out, up to its first publishing Step"*. Neither admits a setup with no thirdshift build or **Process** of its own. Meanwhile, an **Experiment** might put thirdshift as Baseline against an outside orchestrator as Candidate, test an edit to an outside orchestrator against an earlier version of itself, or put the outside orchestrator as Baseline. The options:
- (a) **Add `External runner` to `GLOSSARY.md`, and broaden `Arm` and `Trial` symmetrically** so that either the **Baseline arm**, the **Candidate arm**, or both may run an External runner in place of a thirdshift build and **Process** (_Avoid_: custom runner, agent adapter, scaffold).
- (b) **Candidate arm only**, keeping the Baseline arm required to be a thirdshift build, and broaden **Arm** without a separate **External runner** entry.
- (c) **Another term** (such as **Outside process** or **Custom runner**).

*Recommended:* (a). "Custom runner" is easy to confuse with **Custom process**, "agent" is already on **Harness**'s `_Avoid_` list, and `arms::Prepared` already treats `baseline` and `candidate` symmetrically (`src/evals/arms.rs`), which lets you compare two versions of an External runner as well as pit one against thirdshift.

*Answer:* I like runner, but I'm wary about its collision with GitHub's runners, so what else could work? I could actually work with Custom runner, to tie it into Custom process, but still thinking of options here.

*Checked:* `GLOSSARY.md` uses `runner` once, under **Abandoned job** (`GLOSSARY.md:259`: *"A GitHub Actions job … as when its runner is lost"*, `_Avoid_: lost runner`), while `docs/eval-tasks.md:493` already calls Trial execution *"the runner"* (*"This command prepares the runner's inputs"*). In #941 (`941-metaphors-and-names.md`), collisions with GitHub terms (`run` vs check runs, `PR review` vs **Review run**) were solved by two-word compound terms rather than coined single words. Q2.1 puts **Custom runner** beside three non-`runner` options.

**Q1.2 - Which Eval suites an External runner can enter.** thirdshift has three **Eval suite** session kinds (`docs/trial.md`): `code-review` and `spec-review` (which expect `.standards` and `.spec` axis report files with the repository left untouched), and `process` (which hands the **Trial** a prepared repository and **Ticket** and grades the committed branch head, **Base-tests**, **Cross-tests**, the **Grader panel** and the blind pairwise page). Also, when `--suite` is omitted, `thirdshift experiment` chooses suites from `git diff --name-only <baseline> <candidate>` in the thirdshift repository (`src/evals/experiment.rs:246`), which does not exist when an Arm has no thirdshift commit. The options:
- (a) **`process` Eval suites only, with explicit `--suite` required whenever either Arm uses an External runner.** Selecting a `code-review` or `spec-review` suite, or passing `--release`, with an External runner is refused before dispatch.
- (b) **Every Eval suite kind (`process`, `code-review`, `spec-review`)**, extending the External-runner contract to produce review axis reports and enforce unchanged-repository checks too.

*Recommended:* (a). #1468 is for multi-session workflows whose orchestrator thirdshift cannot express as a **Process** file; review suites test single review sessions that thirdshift already runs on any **Harness**, and requiring `--suite` avoids inventing synthetic file-touch diffs when an Arm has no thirdshift commit.

*Answer:* agree

**Q1.3 - How an External runner is defined and frozen.** #1468's hand-run comparison showed that pinning a command string alone lets an Arm drift mid-Experiment (the other Arm's skills moved dozens of commits on unmerged branches in days). Before the first **Trial**, thirdshift also needs to know:
- every local script, skill directory and helper file to freeze by SHA-256 (for Trial integrity checks, Experiment resume identity, and per-Arm cost-estimate history via a content digest like `Setup.process_digest`);
- every **Harness** / provider the runner may call (to pin real binary paths and `--version` in `prepare-arms`, populate logins, open proxy hosts, and supply the Arm's model families to the **Grader panel**'s `arm_family_rule`).

The options:
- (a) **A runner manifest file (`~/.thirdshift/runners/<name>.toml` or an explicit `<file>.toml`)**, selected with `--baseline-runner <name-or-file.toml>` / `--candidate-runner <name-or-file.toml>` (mutually exclusive with `--*-process`, and needing no thirdshift commit on that Arm). `prepare-arms` freezes the manifest and every file or folder it lists by SHA-256, pins each declared Harness's real path and `--version`, and verifies every hash before each Trial starts.
- (b) **CLI flags alone** (`--candidate-runner-cmd`, `--candidate-runner-files`, `--candidate-runner-harnesses`, …), with no `.toml` manifest.
- (c) **A container image per External runner** (Harbor `BaseInstalledAgent` style), rather than a TOML manifest and frozen directory.

*Recommended:* (a). It mirrors `~/.thirdshift/processes/<name>.toml` and `arms::frozen::Process` (`src/evals/arms/frozen.rs`), keeps multi-provider runner definitions readable and repeatable, gives `Setup` one content digest for cost history and resume identity, and runs on existing `host` (`bwrap`) tasks today without precluding task container images later.

*Answer:* agreed

**Q1.4 - Sandbox, logins, and network boundary.** In a thirdshift Process Trial today (`src/sandbox.rs`), `thirdshift --trial` runs outside `bwrap` and wraps each individual **Step** session in its own single-Harness `bwrap` sandbox. An External runner is a black box that spawns its own child CLIs or SDK calls across multiple providers during one Trial. The options:
- (a) **One Trial-wide `bwrap` sandbox (inside the Trial's `systemd-run` memory scope) enclosing the External runner and every child process it starts.** thirdshift builds one fresh `HOME`, binds the pinned binaries and logins for only the Harnesses/providers the runner manifest declares, and runs one allowlisting `HTTPS_PROXY` permitting the union of those providers' API hosts (with GitHub, general web hosts, and unbound host paths unreachable, and `THIRDSHIFT_TRIAL_NO_SANDBOX` remaining the test/emergency escape).
- (b) **Run the External runner outside `bwrap`**, leaving isolation to the runner itself or to separate OS accounts as in #1628's hand-run setup.

*Recommended:* (a). Running a black-box orchestrator unsandboxed on an Experiment host would expose the source repository's reference fix, `~/.thirdshift/evals`, GitHub, and live factory state without separate OS accounts. Enclosing the whole runner in one multi-provider `bwrap` and `systemd-run` scope gives every child session the same time-safety, `oom_score_adj`, and `MemoryMax` boundary as native Process Trials.

*Answer:* agree

**Q1.5 - How the runner receives the Eval task and returns its code change and outcome.** For `process` tasks today, `prepare_trial` checks out `inputs.branch` (`issue-<n>`) at `inputs.base`, sets `origin` to a local bare repo (`.git/eval-origin.git`) holding only `base`, and writes `.thirdshift-issue.md` (`src/evals/repository.rs`). Downstream code **Graders** (`grading/process.rs`) and Pair evidence (`pairs.rs`) read `HEAD^{commit}` from that repository, and process Trials end `delivered`, `stopped_to_ask`, `failed`, or `void` (`docs/trial.md`). The options:
- (a) **Prepared repository + `request.json` in; committed `HEAD` + `result.json` out.** thirdshift starts the runner in the prepared repository with the path to a versioned `request.json` (`repository`, `issue_file`, `base`, `branch`, `output_dir`). The runner commits its final change on `HEAD` and writes `output_dir/result.json` reporting `outcome` (`delivered`, `stopped_to_ask`, `failed`, or a known `void` cause), optional `cause`, and optional `steps` timings; uncommitted working-tree files are ignored by grading.
- (b) **Patch file out (SWE-bench style)**: the runner writes a `git diff` patch into `output_dir`, and thirdshift applies and commits it before grading.
- (c) **Exit-code and marker-file conventions only (no JSON request or result)**: exit `0` means `delivered`, specific exit codes or marker files mean `stopped_to_ask` / `void`, and `HEAD` holds the code.

*Recommended:* (a). Grading `HEAD^{commit}` reuses `grading/process.rs` and `pairs.rs` unchanged and avoids `git apply` edge cases on binary files, renames, or multi-commit branches, while a small versioned `request.json` / `result.json` matches `docs/trial.md` without overloading Unix exit codes.

*Answer:* Agree, though the standardization of b is attractive, I think a sounds better.

**Q1.6 - Session traces, per-role usage accounting, and leak-check evidence.** Four parts of an **Experiment** read a Trial's sessions today:
1. `accounting.rs` prices each session's tokens per observed **Model** from the Experiment's frozen `prices.json`, groups them by `role`, and marks missing or unpriceable usage as `Void` (`accounting_gap`);
2. `leaks.rs` scans every session transcript line for the shipped fix's merge commit, pull request, issue, branch, and title;
3. `monitor.rs` watches live Claude and Codex streams for the 85% quota stop (`usage_limit`);
4. the **Console**'s Trial Peek / conversation view and OTLP **Trace** export show the sessions.

In #1628's hand-run comparison, the contractor's orchestrator ran `claude --output-format stream-json` and `codex exec --json`, whereas non-CLI outside frameworks (Python/SDK agents) call provider APIs directly without emitting Harness CLI streams. The options:
- (a) **Accept both raw Harness streams (converted by thirdshift) and normalized session records (transcript + per-model token counts), listed in `output_dir` with each session's `role`.** When a runner spawns supported Harnesses (`claude`, `codex`, `opencode`, `grok`, `kimi`, `agy`, `muse`), it leaves their raw JSONL streams in `output_dir` (or runs them through a small recording helper thirdshift provides in the sandbox) and thirdshift's own adapters produce `.logs.jsonl` / `.spans.jsonl`, giving identical cache-inclusive token math, live quota stops, leak scanning, and Console rendering. When a runner calls model APIs directly without a supported Harness CLI, it writes a session transcript plus raw per-model token counts (`input`, `output`, `cache_read`, `cache_write`, `role`, start/end times) that `leaks.rs` scans and `accounting.rs` prices from the frozen `prices.json` (never trusting runner-computed USD).
- (b) **Raw Harness CLI streams only**: every session inside an External runner must come from one of thirdshift's seven Harnesses; direct-SDK runners wait until needed.
- (c) **Normalized runner-parsed format only**: require every External runner, even one calling `claude` and `codex`, to parse its own CLI streams into a custom trajectory and usage JSON.

*Recommended:* (a). Supporting raw Harness streams avoids the exact cache-inclusivity traps #1628 hit when external skills parsed `claude` and `codex` differently (`process-comparison-cost-accounting.md` §1–2) and preserves live quota stops and Console rendering, while also accepting `{role, transcript, models}` records keeps direct-SDK scaffolds (Harbor, OpenHands, custom Python runners) from having to fake Claude CLI JSONL.

*Answer:* agreed

**Q1.7 - Questions to a human: unattended boundary vs simulated user.** #1468's triage notes and `eval-process-arms-and-human-review.md` §2 noted three ways to treat an orchestrator that stops to ask a person a question: record `stopped_to_ask` as no delivery, reply with a scripted refusal (`"YOU SHOULD NEVER ASK FOR HUMAN HELP"`), or answer from a brief / simulated user. For Process Arms, #1484 Q3.4 settled on unattended execution where `stopped_to_ask` counts as a failed **Score** and is reported beside the **Verdict**. The options:
- (a) **Strictly unattended at thirdshift's boundary, matching Process Arms (#1484 Q3.4).** thirdshift invokes the External runner once with `.thirdshift-issue.md` and never answers questions; if the runner stops to ask a human, it ends `stopped_to_ask`, counted as a failed Score in the Verdict and tallied under `outcomes.stopped_to_ask`. Any internal "never ask for human help" prompt belongs inside the runner's own scaffold, and thirdshift runs no simulated user.
- (b) **Per-Arm question policy run by thirdshift** (choosing in the Arm definition between `stopped_to_ask`, a scripted refusal turn, or a brief/simulated-user session).

*Recommended:* (a). Giving an External runner a simulated-user oracle that resolves Ticket ambiguities while a thirdshift Process Arm sees only `.thirdshift-issue.md` would tilt the comparison, whereas an internal refusal prompt inside a multi-step orchestrator is already part of the runner itself if its author wants one.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - Choosing the term (`GLOSSARY.md`, folder name, and CLI flags).** Following your Q1.1 note: `GLOSSARY.md` mentions GitHub Actions' `runner` once under **Abandoned job** (`GLOSSARY.md:259`), while `docs/eval-tasks.md:493` already calls Trial execution *"the runner"*. Across all options below, **Arm** and **Trial** broaden symmetrically so either the **Baseline arm**, the **Candidate arm**, or both may use one. The options:
- (a) **`Custom runner`** (`~/.thirdshift/runners/<name>.toml`, `--baseline-runner` / `--candidate-runner`; `_Avoid_`: bare `runner`, external runner, agent adapter, scaffold). Ties directly to **Custom process** (`~/.thirdshift/processes/`, `--*-process`), pairs naturally with thirdshift's own built-in Trial runner (`thirdshift --trial`), fits single-command tools as well as multi-agent orchestrators, and—like **Ticket run** and **Review run** in #941—uses the two-word compound everywhere in prose so it never collides with a GitHub Actions job's runner.
- (b) **`Trial driver`** (`~/.thirdshift/drivers/<name>.toml`, `--baseline-driver` / `--candidate-driver`). Drops the word `runner` altogether, stays plain English (*"thirdshift drives coding agents"*), and fits single-session and multi-session setups alike.
- (c) **`Custom orchestrator`** (`~/.thirdshift/orchestrators/<name>.toml`, `--baseline-orchestrator` / `--candidate-orchestrator`). No collision with GitHub Actions, **Process** or **Harness**, though awkward when an outside setup runs a single session rather than orchestrating sub-agents.
- (d) **`External process`** (`~/.thirdshift/external-processes/<name>.toml`, `--baseline-external-process` / `--candidate-external-process`). Completes **Built-in process** / **Custom process** / **External process**, at the cost of splitting **Process** so only Built-in and Custom processes run factory **Commands** while External processes run only in **Trials**.

*Recommended:* (a). Pairing **Custom runner** with **Custom process** (`~/.thirdshift/runners/` beside `~/.thirdshift/processes/`, `--*-runner` beside `--*-process`) makes the CLI and folder layout immediately predictable, and always writing the two-word term **Custom runner** keeps it clear of GitHub Actions' runner.

*Answer:* agree

*Checked:* Added **Custom runner** to `GLOSSARY.md` (`_Avoid_`: bare `runner`, `external runner`, `agent adapter`, `scaffold`), and updated **Arm**, **Trial**, **Experiment**, and **Credentials** inline to match `Q2.1`–`Q2.4`.

**Q2.2 - CLI invocation and `Setup` identity when an Arm has no thirdshift commit.** Today `thirdshift experiment <candidate> [--baseline <ref>]` requires a positional `<candidate>` Git reference (`src/evals/experiment/options.rs:273`), defaults `--baseline` to `git merge-base main <candidate>` (`src/evals/experiment.rs:241`), and `Setup` (`experiment.rs:161`, `docs/experiment-results.md` format 12, Console `render.rs:1209`) has non-null `commit` and `harness`. Building on Q1.2 and Q1.3, when one or both Arms use a manifest file:
- (a) **An Arm is either a thirdshift build or a manifest, never both.**
  - When `--candidate-runner` (or Q2.1's flag) is given, the positional `<candidate>` commit is omitted; supplying both, or combining the manifest flag with `--*-process`, `--*-harness`, `--*-model` or `--*-effort` on the same Arm, is refused.
  - When Baseline is a thirdshift build and Candidate is a manifest, `--baseline` defaults to `HEAD` in the current thirdshift checkout (since there is no Candidate commit to take `merge-base main` with). When *both* Arms are manifests, no thirdshift checkout or Cargo build is needed at all.
  - In `Setup` (bumping `docs/experiment-results.md` to format 13), `commit` and `harness` become nullable (`null` on a manifest Arm), and `Setup` gains the manifest's `name` and SHA-256 `digest`—used for Experiment identity, resume, per-Arm cost history (`cost.rs:127`), Console Trial Peek, and `PairPage.revealed`.
- (b) **Keep `<candidate>` positional commit required** even for a manifest Arm, recording `HEAD` on `Setup.commit` for reference without building it.

*Recommended:* (a). `cost.rs:127` keys per-Arm cost estimates by serialized `Setup`; putting an unused thirdshift commit into a manifest Arm's `Setup` would throw away its cost history every time `HEAD` moves in thirdshift and misstate what `PairPage.revealed` ran.

*Answer:* agree

**Q2.3 - Manifest contents, home-file mapping, `arm_families`, and Git provenance.** In #1628, the other Arm's skills lived in another checkout on fast-moving branches and expected to be found in Harness skill folders (`~/.claude/skills/`, `~/.agents/skills/`, etc.) or beside its scripts. Inside `bwrap` (Q1.4), unbound host paths do not exist. Meanwhile, `panel::arm_families` (`src/evals/experiment/panel.rs:270`), `sandbox`, and `proxy` must know every Harness and provider family before the first Trial starts. Building on Q1.3 and Q1.4, the options:
- (a) **Declarative `.toml` with `command`, mapped `files`, `harnesses`, `providers`, and optional `roles`; exact bytes frozen by SHA-256, with enclosing Git `HEAD` + `dirty` recorded as provenance.**
  - `command = ["...", ...]`: executable and arguments (resolved in the frozen bundle or `PATH`, with no shell word-splitting).
  - `files`: host files or folders frozen by SHA-256 into the Arm's bundle, each placed either in the bundle (`runner_dir` in `request.json`) or at a declared relative path inside fresh `HOME` (such as `.claude/skills/foo` or `.agents/skills/foo`), rejecting `..` and absolute targets. When a source path sits inside a Git worktree at `prepare-arms` time, the Arm record also records its repository origin/path, `HEAD` commit, and `dirty` flag as provenance, without refusing uncommitted local edits.
  - `harnesses` (any of the seven Harnesses, plus required `<provider>/<model>` or alias entries when OpenCode or Kimi is listed so `proxy::allowed_hosts` knows which provider hosts to open) and `providers` (direct-SDK providers from `proxy.rs`'s host lists: `anthropic`, `openai`, `xai`, `meta`, `google`, `fireworks-ai`, `baseten`, `kimi`); their families are unioned into `arm_families` for the Grader panel's `arm_family_rule`.
  - Optional `roles` map (`role -> { model, effort }`) saved in the Arm record for human provenance (#1468 comment 3), while pricing still uses observed per-session Models from Q1.6.
- (b) **As (a), but refuse `prepare-arms` whenever any source path is uncommitted (`dirty`) or outside a Git repository.**
- (c) **Freeze only one bundle folder** with no mapping into fresh `HOME`, leaving the runner to copy skills into `$HOME` on every Trial.

*Recommended:* (a). Mapping frozen skill directories into fresh `HOME` avoids copy boilerplate in every wrapper script, and SHA-256 hashing of the exact file bytes (as `arms/frozen.rs` already does for `~/.thirdshift/skills/`) guarantees Trial-to-Trial reproducibility while letting you test local edits without throwaway commits—with Git `HEAD` and `dirty` still captured when present.

*Answer:* agree

**Q2.4 - Direct-SDK credentials and blocking provider-hosted web search inside the shared sandbox.** Checking `src/sandbox.rs`, `src/resend_key.rs`, and `src/harness/*.rs` surfaced two concrete constraints for Q1.4's shared sandbox:
1. **Credentials**: `sandbox.rs` clears ambient environment variables, and Experiments can be started from the **Console** (`src/console/start/experiment.rs`), where the Console daemon has no interactive shell exports. CLI Harnesses already copy their logins from `~/.thirdshift/credentials.toml` or `$HOME` (with `GrokGuard` serializing Grok Trials), whereas direct-SDK runners (Q1.6) have no CLI login file.
2. **Server-side web search**: `claude` (`WebSearch`, `WebFetch`), `codex` (`web_search`), `grok`, and `muse` execute web search **on the provider's own API servers** (`api.anthropic.com`, `api.openai.com`, etc.), so `HTTPS_PROXY` host allowlisting alone cannot block a sub-agent from searching the web for the public fix—native Trials pass per-Harness CLI switches (`src/harness/claude.rs:60`, `codex.rs:68`, `grok.rs:76`, `muse.rs:68`).

The options:
- (a) **Store direct-SDK Trial keys in `~/.thirdshift/credentials.toml` (`[providers]`, Trials-only) and place thin `PATH` wrappers over pinned Harness binaries to enforce Trial web-search switches.**
  - Declared `harnesses` get only their logins in fresh `HOME` (acquiring `GrokGuard` when `grok` is declared, and building a multi-provider `OPENCODE_CONFIG_CONTENT` with `webfetch`/`websearch` denied when `opencode` is declared), and thin `PATH` wrappers ahead of the pinned real binaries prepend each Harness's no-web-search switch without altering stdin, stdout, stderr, or exit codes.
  - Declared direct-SDK `providers` receive only their corresponding API key from `[providers]` in `credentials.toml` (missing login or key -> `Void` with `session_unavailable`, matching `docs/trial.md:155`), and any web-search tool call recorded in a Trial's session transcripts voids the Trial.
- (b) **Read direct-SDK keys from thirdshift's ambient process environment (`api_key_env` style)** instead of `credentials.toml`, and leave disabling web search to the runner's own flags.

*Recommended:* (a). Keeping Trial keys in `credentials.toml` works identically from the CLI and from Console Start without exposing ambient host secrets (ADR 0007), and `PATH` wrappers close the provider-hosted web-search leak automatically even when an outside skill invokes bare `claude` or `codex exec`.

*Answer:* agree

**Q2.5 - `request.json` / `result.json` edge cases: crashes, allowed `void_cause`s, and uncommitted files.** Q1.5 settled on `request.json` in and committed `HEAD` + `output_dir/result.json` out. Three edge cases arise at that boundary:
1. In `execution.rs:273` today, if `thirdshift --trial` exits nonzero without JSON, thirdshift aborts the whole **Experiment** (`Arm Trial protocol failed`), because that indicates an internal thirdshift bug. An outside runner script, by contrast, may crash with an unhandled exception on one tricky **Eval task** out of 20.
2. In `docs/trial.md:207-213`, `memory_cap` and `host_killed` require outer-launcher evidence so an Arm cannot excuse its own crashes as **Void Trials**.
3. A runner might exit `delivered` while leaving uncommitted working-tree files or `HEAD` still at `base`.

The options:
- (a) **Treat runner crashes without outer Void evidence as a `failed` Trial (not an Experiment abort), restrict self-reported `void_cause`s to provider availability/limits, and grade committed `HEAD` only.**
  - `request.json` (version 1) passes `version`, `repository`, `issue_file`, `issue_url`, `base`, `branch`, `runner_dir`, and `output_dir`.
  - Outer supervision keeps precedence (`interrupted`, `sandbox_unavailable`, `memory_cap`, `host_killed`, live `usage_limit`, and timeout -> `failed`).
  - Otherwise, if `result.json` is missing, unreadable, or the runner exited nonzero without writing a valid version-1 `result.json`, that Trial is recorded as **`failed`** with its exit status and stderr in `cause` (matching `docs/trial.md:206`: *"An unexplained nonzero exit is a failure"*), so one task crash doesn't kill a 60-Trial Experiment.
  - In `result.json`, the runner may report `outcome` = `delivered`, `stopped_to_ask`, `failed`, or `void` with `void_cause` limited to `session_unavailable`, `usage_limit`, or `rate_limit`; claiming outer causes (`memory_cap`, `host_killed`, `sandbox_unavailable`, `interrupted`, `accounting_gap`, `fix_leaked`, `repository_changed`) is rejected as `failed`.
  - Uncommitted working-tree edits are ignored (with a note in Trial diagnostics when `delivered` had dirty tracked/untracked files), and `delivered` with `HEAD == base` grades `HEAD` normally (failing `fail_to_pass` checks).
- (b) **Abort the whole Experiment** on a missing/invalid `result.json` or nonzero runner exit (matching `execution.rs:273`), and fail before grading if the working tree is dirty.

*Recommended:* (a). Outside orchestrators are code under test: a crash on task #7 is a genuine failure of that Arm on task #7, not a reason to lose the rest of an overnight Experiment, and restricting self-reported `void_cause`s stops a buggy runner from turning its own crashes into Void retries.

*Answer:* agree

**Q2.6 - Session manifest, live recording helper, token-field convention, and `accounting_gap` detection.** Building on Q1.6, checking `src/trial/accounting.rs:84-87` and `src/evals/experiment/accounting.rs:112-129` exposed two sharp traps:
1. **Cache token convention**: `Prices::cost` expects `Measurement.input_tokens` to *include* `cache_read + cache_write` (OpenAI's convention), whereas Anthropic's API reports `input_tokens` *excluding* cache—the exact discrepancy noted in `process-comparison-cost-accounting.md` §1–2.
2. **Silent $0.00 Trials**: `experiment/accounting.rs:118` currently exempts `role == "process"` from the empty-sessions `accounting_gap` check so command-only Custom processes can pass with zero sessions. If an outside runner forgets to save its sub-agent streams (or deletes its run directory on exit, as #1628's candidate did), exempting it would silently price a 72-session Trial at `$0.00` and skip every session in `leaks.rs`.

The options:
- (a) **Unify everything into OTLP `.logs.jsonl` + `.spans.jsonl` in `attempt/session/`, provide an optional live recorder helper, use explicit `uncached_input_tokens` on direct-SDK records, and trigger `accounting_gap` whenever provider traffic or Harness sessions go unrecorded.**
  - **Optional live helper**: thirdshift exposes a small recorder command in the sandbox (passed in `request.json`, e.g. `record_command`) that wraps any supported Harness CLI call with `--role <role>`, streaming live into `attempt/session/` so `monitor.rs` catches the 85% quota window mid-Trial and the runner cannot accidentally delete completed streams.
  - **Post-run manifest (`output_dir/sessions.json`)**: runners that manage their own files list each top-level session (either `{ role, harness, stream_file, started_at, ended_at }` for raw Harness JSONL, or `{ role, transcript_file, started_at, ended_at, models: { <model>: { uncached_input_tokens, cache_read_tokens, cache_write_tokens, output_tokens } } }` for direct-SDK runs—stating `uncached_input_tokens` explicitly so Anthropic and OpenAI callers cannot confuse inclusive vs exclusive input). thirdshift converts both kinds into standard OTLP `.logs.jsonl` + `.spans.jsonl` (and checks converted Claude/Codex streams for quota rejections), so `accounting`, `leaks`, Waterfall, Console Trial Peek, and OTLP export need no second read path. In-process Harness subagents (such as Claude `Task`) stay rolled into their parent stream rather than listed twice.
  - **Completeness check (`accounting_gap`)**: the sandbox's `HTTPS_PROXY` and `PATH` Harness wrappers count outbound provider connections and non-probe Harness runs; if provider traffic occurred (or a runner with declared `harnesses`/`providers` ended `delivered` or `stopped_to_ask`) with zero recorded sessions—or fewer Harness session entries than Harness runs that crossed the proxy—the Trial becomes `Void` (`accounting_gap`).
- (b) **Post-run `sessions.json` conversion only (no live recorder helper and no proxy/wrapper completeness check)**, requiring at least one session only when `outcome` is `delivered` or `stopped_to_ask`.

*Recommended:* (a). Converting both raw Harness streams and direct-SDK records into `.logs.jsonl` + `.spans.jsonl` keeps one representation across every Experiment reader, `uncached_input_tokens` eliminates the inclusive/exclusive cache trap, the live helper gives CLI orchestrators mid-Trial quota pauses for free, and checking against proxy traffic catches deleted or forgotten round streams (the central fairness hazard in `process-comparison-cost-accounting.md` §4).

*Answer:* agree

**Q2.7 - Sub-issue #1628.** #1628 (*"Hand-run comparison of the Built-in Ticket run against a Custom process, by replaying closed issues"*) is still open (`ready-for-human`) as a sub-issue of #1468, though #1468's third comment records that its 10 paired live runs were completed by hand and summarizes their lessons. Once this grill finishes, what should happen to #1628?
- (a) **Close #1628 as completed**, with a short comment pointing to #1468's triage summary and the resulting Spec.
- (b) **Leave #1628 open** (for example, if another hand-run replay is planned on it).

*Recommended:* (a). The hand-run comparison it specified has already been carried out and its findings folded into #1484, #1872, and this issue.

*Answer:* agree

## Round 3 (answered)

**Q3.1 - Validating and smoke-testing a Custom runner (`evals` CLI and Console parity).** Custom processes have `thirdshift process check <name>` (`src/console/parity.rs:72`), and under [ADR 0026](../../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md) every CLI command and Experiment start option has a Console API twin (`src/console/start/experiment.rs`). Before starting a 60-Trial **Experiment**, an author wiring up a **Custom runner** needs a cheap way to catch a missing file, missing `[providers]` key, unpriced **Model**, or malformed `sessions.json`. The options:
- (a) **Add `thirdshift evals check-runner <name-or-file.toml> [--suite <suite> --task <task>]` (with its Console API twin, plus `baseline_runner` and `candidate_runner` in `start/experiment.rs`).** Without `--task`, it validates the `.toml` manifest, source `files`, pinned `harnesses`, logins and `[providers]` keys, and any declared `roles` Models against `prices.json`. With `--suite` and `--task`, it runs one throwaway Trial inside the real Trial sandbox and exercises `result.json`, session conversion, pricing, `accounting_gap`, and the task's code checks outside Experiment storage.
- (b) **Static check only (`evals check-runner <name-or-file.toml>`)**, with no single-task Trial dry run.
- (c) **A new top-level `thirdshift runner` command (`list`, `show`, `check`)** alongside `thirdshift process` (and mentioned on the hand-written site pages per `AGENTS.md`).

*Recommended:* (a). Unlike **Custom process**, a **Custom runner** is strictly an **Evals** concept rather than a top-level factory **Command**, so housing it under `thirdshift evals` avoids adding a top-level workflow command while letting you smoke-test a new wrapper end to end on one task before spending 60 Trials.

*Answer:* agree

**Q3.2 - Preventing in-worktree runner scratch files from breaking blinding (`git_exclude`).** In #1628 (`process-comparison-cost-accounting.md:76-78`), the other Arm wrote `.review-runs/<round>/` inside the repository worktree and relied on `.gitignore`. On a task repository whose `.gitignore` does not list `.review-runs/` (or `.openhands/`, `.aider*`, etc.), an agent running `git add .` or `git commit -a` commits those scratch files onto `HEAD`, where `pairs::diff` (`src/evals/experiment/pairs.rs:239`) would render them on the blind pairwise page and reveal which Arm produced the change. `prepare_trial` (`src/evals/repository.rs:233-236`) already writes `.git/info/exclude` for `.thirdshift-issue.md` without modifying tracked `.gitignore`. The options:
- (a) **Allow optional `git_exclude = [...]` patterns in `<name>.toml`, appended to `.git/info/exclude` before the Custom runner starts**, and record the Trial as `failed` if `HEAD` still commits a path matching `git_exclude` (for instance via `git add -f`), so neither the code Grader nor the blind pairwise page ever sees runner scratch files.
- (b) **Leave scratch exclusion solely to the Custom runner's own wrapper script.**

*Recommended:* (a). Appending `git_exclude` to `.git/info/exclude` extends `repository.rs:233` by a few lines, keeps tracked `.gitignore` clean, and prevents an unignored scratch directory from spoiling blind human review across 60 pairs.

*Answer:* agree

**Q3.3 - Record an ADR (`0031`)?** Native `process` Trials enter `bwrap` *per Step* with one Harness and no `PATH` shims, whereas a **Custom runner** Trial enters `bwrap` *once for the whole Trial* with multiple logins, `[providers]` keys from `credentials.toml`, `PATH` wrappers disabling provider-hosted web search, and unified OTLP session conversion verified against `HTTPS_PROXY` traffic—deliberately rejecting Harbor's container-per-agent, SWE-bench `git diff` patches, runner-computed costs, and simulated-user Q&A. That meets all three ADR criteria (hard to reverse once wrappers target the contract, surprising next to per-Step sandboxes without context, and chosen over real alternatives). The options:
- (a) **Write `docs/adr/0031-custom-runners-run-inside-one-trial-sandbox.md`** recording the boundary and rejected options.
- (b) **Skip the ADR**, keeping the rationale in the Spec and `docs/trial.md` alone.

*Recommended:* (a).

*Answer:* agree

*Checked:* Numbered as [ADR 0032](../../adr/0032-custom-runners-run-inside-one-trial-sandbox.md) after ADR 0031 (#583) merged on `main`.

**Q3.4 - Shared understanding?** Every other branch of the design tree is settled (listed below). If `Q3.1`–`Q3.3` are agreed, is the design understood well enough to:
1. write ADR `0031` (if Q3.3 is (a)) and move this file to `docs/grill-with-docs/done/1468-custom-runner-arm.md`,
2. publish `#1468` as the **Spec** (retitled to **Custom runner**) with its **Tickets** and blocking links following `docs/agents/triage-labels.md`'s Day shift publication procedure, and
3. close sub-issue `#1628` as completed (Q2.7)?

*Recommended:* Yes.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- **Spec #1484 and Spec #1872 are shipped.** thirdshift already runs `process` Trials on time-safe repositories, grades `HEAD^{commit}` with offline code **Graders**, runs **Base-tests** and **Cross-tests** (`src/evals/experiment/pairs.rs`), runs the multi-family **Grader panel** in both orders (`src/evals/experiment/panel.rs`) and serves the blind human pairwise page (`src/evals/experiment/pairwise.rs`), scans Trial transcripts for fix leaks (`src/evals/experiment/leaks.rs`), prices per-session and per-role usage from a frozen price table with `accounting_gap` Void checks (`src/evals/experiment/accounting.rs`), and shows per-Arm cost estimates from each Arm's own history (`src/evals/experiment/cost.rs`).
- **A Trial is part of its Experiment Command, not a Command of its own** ([ADR 0027](../../adr/0027-a-trial-carries-out-a-process-but-is-not-a-command.md), #1484 Q3.2 and Q8.1). A **Custom runner** Trial likewise keeps its evidence under the Experiment and writes no factory **Command record**, **Claim**, pull request, **Activity log** line or **Run metrics**.
- **Who decides a Verdict is fixed in the Bar** ([ADR 0028](../../adr/0028-who-decides-a-verdict-is-fixed-in-the-bar.md), #1484 Q9.1). Code Graders, Pair evidence, the Grader panel, and people's blind merge calls treat both Arms identically once each Trial produces a final commit and retained session evidence.
- **Harbor is a design reference, not a dependency** (#1484 Q2.2).
- **The term is `Custom runner`** (`~/.thirdshift/runners/<name>.toml`, `--baseline-runner` / `--candidate-runner`, `_Avoid_`: bare `runner`, `external runner`, `agent adapter`, `scaffold`), added to `GLOSSARY.md` alongside symmetric **Arm** and **Trial** updates so either the **Baseline arm**, the **Candidate arm**, or both may use one (Q1.1, Q2.1).
- **`process` Eval suites only, with `--suite` required** whenever either Arm is a Custom runner; `code-review` suites, `spec-review` suites, and `--release` are refused before dispatch (Q1.2).
- **An Arm is either a thirdshift build or a Custom runner manifest, never both.** `--candidate-runner` omits the positional `<candidate>` commit (combining `--*-runner` with a commit, `--*-process`, `--*-harness`, `--*-model` or `--*-effort` on the same Arm is refused); when Baseline is a thirdshift build and Candidate is a Custom runner, `--baseline` defaults to `HEAD`, and when both Arms are Custom runners no thirdshift checkout or Cargo build is needed. `Setup` (format 13 in `docs/experiment-results.md`) makes `commit` and `harness` nullable and adds `runner` and `runner_digest` for identity, resume, cost history, Console Trial Peek, and `PairPage.revealed` (Q1.3, Q2.2).
- **Declarative `.toml` manifest with SHA-256 freezing, Git provenance, and `git_exclude`.** `<name>.toml` declares `command = [...]` (argv), `files` (frozen by SHA-256 into `runner_dir` or mapped to relative paths inside fresh `HOME`, rejecting `..` and absolute targets, with enclosing Git repo path/origin, `HEAD` commit, and `dirty` flag recorded in the Arm record without refusing uncommitted local edits), `harnesses` (pinned by real path and `--version`, with model/provider aliases where OpenCode or Kimi needs host resolution), `providers` (direct-SDK providers from `proxy.rs`), optional `roles` (`role -> { model, effort }`), and optional `git_exclude = [...]` patterns appended to `.git/info/exclude` before the Custom runner starts (recording the Trial as `failed` if `HEAD` still commits a `git_exclude` path). Declared Harnesses and providers are unioned into `panel::arm_families`, and frozen file hashes are verified before every Trial (Q1.3, Q2.3, Q3.2).
- **One Trial-wide `bwrap` sandbox (inside the Trial's `systemd-run` memory scope)** encloses the Custom runner and every child process it starts, holding one fresh `HOME`, logins only for declared `harnesses` (with `GrokGuard` serialization for Grok and multi-provider `OPENCODE_CONFIG_CONTENT` for OpenCode), Trials-only direct-SDK API keys from `[providers]` in `~/.thirdshift/credentials.toml` (missing login or key -> `Void` with `session_unavailable`), one allowlisting `HTTPS_PROXY` for the union of declared API hosts, and thin `PATH` wrappers ahead of pinned Harness binaries that prepend each Harness's no-web-search switch without changing stdio/exit codes (while any web-search tool call in normalized transcripts voids the Trial). `THIRDSHIFT_TRIAL_NO_SANDBOX` remains the test/emergency escape (Q1.4, Q2.4).
- **Version-1 `request.json` in; committed `HEAD` + `output_dir/result.json` out.** `request.json` passes `version`, `repository`, `issue_file`, `issue_url`, `base`, `branch`, `runner_dir`, `output_dir`, and `record_command`. Outer supervision (`interrupted`, `sandbox_unavailable`, `memory_cap`, `host_killed`, live `usage_limit`, timeout -> `failed`) keeps precedence. Missing/unreadable `result.json` or nonzero exit without outer Void evidence marks that Trial `failed` (with exit status and stderr in `cause`) rather than aborting the Experiment. Self-reported `void_cause`s in `result.json` are restricted to `session_unavailable`, `usage_limit`, and `rate_limit`; outer causes claimed by a runner become `failed`. Uncommitted working-tree edits are ignored (noted in diagnostics), and `delivered` with `HEAD == base` grades `HEAD` normally (Q1.5, Q2.5).
- **Unified OTLP `.logs.jsonl` + `.spans.jsonl` session conversion, live helper, `uncached_input_tokens`, and `accounting_gap` checks.** Runners either wrap CLI calls with the in-sandbox `record_command` (`--role <role>`, giving live streaming and mid-Trial 85% quota stops) or write `output_dir/sessions.json` listing raw Harness streams (`{ role, harness, stream_file, started_at, ended_at }`) and/or direct-SDK records (`{ role, transcript_file, started_at, ended_at, models: { <model>: { uncached_input_tokens, cache_read_tokens, cache_write_tokens, output_tokens } } }`), priced solely from the Experiment's frozen `prices.json` and converted into `.logs.jsonl` + `.spans.jsonl` for `accounting`, `leaks`, Waterfall, Console Trial Peek, and OTLP export. If outbound provider traffic crossed `HTTPS_PROXY` (or a runner with declared `harnesses`/`providers` ended `delivered` or `stopped_to_ask`) with zero recorded sessions—or fewer Harness session entries than Harness runs that crossed the proxy—the Trial is `Void` (`accounting_gap`) (Q1.6, Q2.6).
- **Strictly unattended at thirdshift's boundary**, matching Process Arms (#1484 Q3.4): no simulated user, and `stopped_to_ask` counts as a failed **Score** in the **Verdict** and is tallied under `outcomes.stopped_to_ask` (Q1.7).
- **`thirdshift evals check-runner <name-or-file.toml> [--suite <suite> --task <task>]` and Console parity.** Validates the manifest, frozen files, pinned Harnesses, logins/`[providers]` keys, and `roles` pricing statically, or runs a single-task sandboxed Trial dry run outside Experiment storage when `--suite` and `--task` are passed; paired with its Console API twin (`src/console/parity.rs`) and `baseline_runner` / `candidate_runner` in `src/console/start/experiment.rs`, with no top-level `thirdshift runner` command (Q3.1).
- **[ADR 0032](../../adr/0032-custom-runners-run-inside-one-trial-sandbox.md) is written** (renumbered from `0031` after #583's ADR 0031 landed on `main`), recording the single-sandbox Custom runner boundary and rejected alternatives (Q3.3).
- **Shared understanding is confirmed** (Q3.4). Sub-issue #1628 closes as completed, and `#1468` publishes as a Spec with its Tickets following `to-spec`, `to-tickets`, and `docs/agents/triage-labels.md` (Q2.7, Q3.4).
- **No release-split Spec needed (`docs/agents/triage-labels.md`).** Every piece of this design is repository code and documentation verifiable in tests prior to merge, so one Spec can land it without a mid-Spec host release.
- **The frontier is empty: nothing is left open.**

## Research results (2026-11-13)

- `arms::Prepared` (`src/evals/arms.rs:88-102`) stores `baseline: Arm` and `candidate: Arm` with `commit`, `binary`, `harness: FixedHarness`, and `process: Option<frozen::Process>`. `frozen::Process` (`src/evals/arms/frozen.rs:14-28`) already freezes arbitrary home-relative file trees into `.arm-inputs-*` with SHA-256 verification before every Trial (`Execution::run`, `src/evals/experiment/execution.rs:97-101`). Confirmed in source.
- `sandbox::Request` (`src/sandbox.rs:116-126`) and `proxy::allowed_hosts` (`src/proxy.rs:70-91`) currently prepare one `Harness` per `bwrap` invocation. OpenCode passes provider config via `OPENCODE_CONFIG_CONTENT` (`src/sandbox.rs:56`) and Grok holds a single-Trial `GrokGuard` lock on `~/.thirdshift/trials/grok` (`src/sandbox.rs:137`). Confirmed in source.
- `grading::process::execute` (`src/evals/experiment/grading/process.rs:42,67-74`) and `pairs.rs` read the Trial's work solely through `git rev-parse --verify HEAD^{commit}` in `attempt/repository`, independent of how `HEAD` was produced. Confirmed in source.
- `trial_sessions::attempt` (`src/evals/experiment/trial_sessions.rs:53-79`) discovers every `.jsonl` and `.spans.jsonl` file in `attempt/session/`; `leaks::scan` (`src/evals/experiment/leaks.rs:183-220`) decodes JSON strings recursively across every selected session file; `accounting::capture` (`src/evals/experiment/accounting.rs:16-131`) reads `.spans.jsonl` (`thirdshift.trial.role` attribute or `SessionStart` sidecar) and voids any non-Void Trial whose `accounting.gap()` is `Some` (`execution.rs:355-360`). Confirmed in source.
- `panel::arm_families` (`src/evals/experiment/panel.rs:270-288`) collects every provider family used by either Arm (`Setup` plus `prepared.*.process.steps`) so `arm_family_rule` can withhold votes from same-family panel seats and `check_stand_in` can verify enough independent families before Trials start. An outside runner must therefore declare its Harnesses/families upfront, not only after Trials run. Confirmed in source.
- Sub-issue #1628 ("Hand-run comparison of the Built-in Ticket run against a Custom process, by replaying closed issues") is still open on GitHub (`ready-for-human`), though #1468's third comment records that its 10 paired live runs were completed by hand. Confirmed via `gh issue view 1628`.
- `GLOSSARY.md` contains `runner` once (`GLOSSARY.md:259`, under **Abandoned job**: *"A GitHub Actions job … as when its runner is lost"*, `_Avoid_: lost runner`), while `docs/eval-tasks.md:493` calls Trial execution *"the runner"*. Confirmed via `rg -n -i runner GLOSSARY.md docs/`.
- `thirdshift experiment` (`src/evals/experiment/options.rs:273`, `src/console/start/experiment.rs:83-90`) currently requires a positional `<candidate>` Git reference, and `Setup` (`src/evals/experiment.rs:161-170`, `src/evals/experiment/cost.rs:127`) serializes non-optional `commit` and `harness` as the exact lookup key for per-Arm cost-estimate history. Confirmed in source.
- `Credentials` (`src/resend_key.rs:95-111`) currently stores `resend.key`, `claude.setup_token`, `opencode.api_key` / `[opencode.keys]`, `[fleet]`, and `[report]`. Provider-hosted web search on `claude` (`--disallowedTools=WebSearch,WebFetch`), `codex` (`-c web_search="disabled"`), `grok` (`--disable-web-search`), and `muse` (`--disable-web-tools`) runs over the allowed provider API hosts and is disabled by CLI flags in `src/harness/*.rs`. Confirmed in source.
- `Prices::cost` (`src/trial/accounting.rs:84-87`) subtracts `cache_read + cache_write` from `Measurement.input_tokens` (expecting cache-inclusive input), and `experiment/accounting.rs:118` skips the empty-sessions `accounting_gap` check when `role == "process"`. Confirmed in source.
- `src/console/parity.rs:69-91` inventories `PROCESSES` and `EVALS` CLI subcommands against Console API routes (`ADR 0026`). `pairs::diff` (`src/evals/experiment/pairs.rs:239-250`) exports file diffs (`base..commit`) without commit messages, `blind_diff` (`src/trial/reports.rs:37`) strips metadata tables naming `harness`/`model`/`effort`/`arm`, and `prepare_trial` (`src/evals/repository.rs:233-236`) writes `.git/info/exclude` for `.thirdshift-issue.md`. Confirmed in source.

## Waiting on research

- None.

## Published (2026-11-13)

- Spec #1468 (rewritten): Custom-runner Arms for process Experiments (`ready-for-agent`).
- Tickets:
  - #2188: Freeze Custom runner manifests and run Custom-runner Trials on process suites.
  - #2189: Enclose Custom runner Trials in a multi-provider sandbox with Trial provider keys, web-search wrappers, and panel family rules (blocked by #2188).
  - #2190: Support Custom-runner Arms in Console Experiment start, Trial Peek, and blind pairwise review (blocked by #2188).
  - #2191: Record Custom runner sessions into OTLP traces and enforce accounting_gap and web-search Void checks (blocked by #2189).
  - #2192: Add thirdshift evals check-runner, its Console API twin, and Custom-runner documentation (blocked by #2190, #2191).
- [ADR 0032](../../adr/0032-custom-runners-run-inside-one-trial-sandbox.md): A Custom runner runs inside one Trial sandbox and returns a commit and session traces.
- Sub-issue #1628: closed as completed.
