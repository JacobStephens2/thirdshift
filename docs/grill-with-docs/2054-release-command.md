# Grilling: #2054, `thirdshift release` command for scheduled and on-demand releases

Issue: https://github.com/JacobStephens2/thirdshift/issues/2054
Research: see "Research results" below

## Round 1 (answered)

**Q1 - Domain term and Command / Pass status.** What should `thirdshift release` be in the domain model?
- (a) **A `Release run` Command — and a `Pass` when started with no `<version>`** (while `thirdshift release <version>` is a direct **Command**, just as `thirdshift fix <pr>` is direct and `thirdshift fix` is a **Pass**). _Avoid_: release job, cut, publish run.
- (b) A `Release run` Command, never a `Pass`.
- (c) An administrative CLI subcommand like `thirdshift update`.

*Recommended:* (a). Scheduled runs get quiet deduplicated skips when idle, **Cron jobs** last-fired tracking, and **Needs you** / **Run notifications** when a release fails.

*Answer:* agreed

**Q2 - Scope and fate of the former release shell script.** Should `thirdshift release` be native Rust scoped to `JacobStephens2/thirdshift` and replace the former release shell script?
- (a) **Native Rust in `thirdshift`, scoped to `JacobStephens2/thirdshift`, retiring the former release shell script**: port the release state machine into Rust, verify the **Launch directory**'s `origin` is `JacobStephens2/thirdshift`, move the former shell summary prompt into `src/prompt.rs`, and delete the former release shell script, the former release Experiment helper and the former summary shim.
- (b) Native Rust replacing the former release shell script, generalized to any Cargo + `cargo-dist` GitHub repository.
- (c) Keep the former release shell script and have `thirdshift release` shell out to it.

*Recommended:* (a).

*Answer:* agreed

**Q3 - When an unscoped `thirdshift release` cuts a release vs skips.** `origin/main` always has at least the previous release's bump merge commit ahead of the last `v*` tag, and later commits may touch only non-shipped files (`docs/`, `site/`, `.agents/`, `tests/`). What changes since the last `v*` tag on `main` make a candidate commit releasable in Pass mode?
- (a) **Shipped-artifact or release-pipeline changes since the last release tag**: skip when the candidate `main` commit has no diff from the last `v*` tag in paths that affect the shipped binary, crate or release pipeline (`src/`, `skills/`, `processes/`, `Cargo.toml`, `Cargo.lock`, `dist-workspace.toml`, `rust-toolchain.toml`, `README.md`, `LICENSE`, `.github/workflows/release*`, `.github/workflows/publish-crates.yml`, `scripts/release*`). An explicit `thirdshift release <version>` still cuts on demand regardless.
- (b) Any non-bump commit on `main` since the last release tag.

*Recommended:* (a).

*Answer:* agreed

**Q4 - Automatic version selection when `<version>` is omitted.** How should `thirdshift release` choose the version when `<version>` is omitted?
- (a) Deterministic bump with CLI override — patch by default (`X.Y.(Z+1)`), with `--minor` and optional `<version>`.
- (b) Deterministic bump — minor by default (`X.(Y+1).0`), with `--patch` and optional `<version>`.
- (c) Infer `minor` vs `patch` from merged PRs' linked issue labels since the last release tag.
- (d) **Ask the summary Harness session to pick `minor` vs `patch`** before committing the bump, while still allowing an explicit `<version>` or `--major` / `--minor` / `--patch` flag on the CLI.

*Recommended:* (a).

*Answer:* (d) as default behavior, while allowing an argument for either a specific `<version>` (higher than the current version on `main`) or `--major` / `--minor` / `--patch`.

**Q5 - Interactive flags (`--review` and `--with-experiment`) in unattended runs.** How should `thirdshift release` handle `--review` and `--with-experiment`?
- (a) **Keep both flags off by default; make prompts strictly terminal-only**: `--review` refuses up front if stdin is not a terminal; `--with-experiment` runs the release **Experiment** when requested, prompts `release anyway? [y/N]` only on an interactive terminal, and fails cleanly on `fail` or `inconclusive` when unattended.
- (b) Run `--with-experiment` by default, failing unattended on `fail` or `inconclusive`.
- (c) Drop `--review` and/or `--with-experiment` interactive overrides.

*Recommended:* (a).

*Answer:* agreed

**Q6 - Boundary between `thirdshift release` and `thirdshift update`.** Should `thirdshift release` stop once the release is published and the bump PR is merged, leaving `thirdshift update` separate?
- (a) **Keep `thirdshift release` and `thirdshift update` separate**: `thirdshift release` only cuts, publishes and merges the release; `thirdshift update` remains its own command / cron entry.
- (b) Have `thirdshift release` (or `--update`) update local or **Fleet** binaries after publishing.

*Recommended:* (a).

*Answer:* agreed

## Round 2 (answered)

**Q7 - Guardrails and single-session flow for the Harness bump choice.**
- (a) **Cap automatic choice to `minor` | `patch`** (leaving `major` strictly to explicit `--major` or `<version>`), decide bump + summary in a **single** session (after any `--with-experiment` check, supplying merged PRs and both candidate versions `X.(Y+1).0` and `X.Y.(Z+1)`), and fall back to `patch` + GitHub's generated notes if the **Harness** fails or gives malformed output.
- (b) Allow the Harness to pick `major` | `minor` | `patch`.
- (c) Fail the Release run instead of falling back when the Harness fails.

*Recommended:* (a).

*Answer:* agree

**Q8 - CLI flags and whether `--patch` / `--minor` / `--major` are Passes.**
- (a) **Any invocation without an explicit `<version>` (plain `thirdshift release`, `--patch`, `--minor`, `--major`) is a `Pass`** that skips when nothing shipped changed; `thirdshift release <version>` is a direct **Command**. Standard `--harness`/`--model`/`--effort` and `--email` / `--email-unless-merged` / `--no-email` flags apply, with `<version>` and bump flags mutually exclusive.
- (b) Only plain `thirdshift release` is a **Pass**; `--patch`, `--minor`, `--major` and `<version>` are all direct **Commands**.

*Recommended:* (a).

*Answer:* agreed

**Q9 - Concurrency, `Command limit`, and protecting open bump PRs from `Review` / `Fix` Passes.**
- (a) **Dedicated lock + cross-machine origin reservation, outside `Command limit`, and exclude `release-*` PRs from unattended `Review` / `Fix` Passes**: honor live **Experiment** and **Quota wait** Pass gates; hold `~/.thirdshift/release.lock` locally plus an atomic `refs/thirdshift/reservations/release` lease on `origin` and `in-progress` on the bump PR while live (skipping in Pass mode and failing in direct mode if held); and skip `release-*` branches targeting `main` in unattended `Review` (`--author`) and `Fix` Passes.
- (b) Share `JacobStephens2/thirdshift`'s **Command limit** as `Sharing::OneOfAKind(PassKind::Release)`.

*Recommended:* (a).

*Answer:* agree

**Q10 - What an unscoped `thirdshift release` Pass does with an unfinished earlier `release-<X.Y.Z>`.**
- (a) **Auto-resume recoverable states; skip human-blocked states**: adopt and finish/recover an unfinished `release-<X.Y.Z>` when recoverable (pushed branch with no PR, open non-draft exact-bump PR waiting on CI, or unpublished failed `release.yml` workflow); **skip** with reason deduplication when human-blocked (draft PR after a failed automatic Fix run, non-bump commits on `release-<X.Y.Z>`, or unrecoverable post-merge failure). Explicit `thirdshift release <version>` still reports refusal/recovery instructions directly.
- (b) Never auto-resume in Pass mode.
- (c) Always attempt resume and fail on every cron tick.

*Recommended:* (a).

*Answer:* agree

**Q11 - Waiting vs skipping when `main` push CI is pending or red.**
- (a) **Release immediately from the newest already-green `main` commit with shipped changes since the last tag; if none is green yet, wait while a candidate's `ci.yml` is queued/running; when all candidates with shipped changes have finished non-green, skip in Pass mode and fail in direct mode** instead of looping forever.
- (b) Wait for `origin/main`'s tip `ci.yml` if pending before falling back to an older green commit.
- (c) Fail into **Needs you** even in Pass mode when all candidates with shipped changes finished red.

*Recommended:* (a).

*Answer:* agree

**Q12 - Installed binary execution (#336) and sandboxed/traced summary session (#653).**
- (a) **Run everything from the executing `thirdshift` binary against fetched `origin/main` (closing #336), and run the summary/bump session as a recorded `Session` inside the fresh-home `bwrap` sandbox (closing #653)**.
- (b) Run from the executing binary (#336), keeping the former unsandboxed Harness summary driver (#653 separate).

*Recommended:* (a).

*Answer:* agree (plus investigate what prevents unauthorized people or sessions from triggering a release; addressed in Round 3).

**Q13 - Inbox lifecycle, `email-unless-merged`, and Console API parity.**
- (a) **Expose `release` on `/api/v1/start` (excluding only `release --review` as terminal-only), show live runs in `Working` (with periodic progress lines during CI / `release.yml` waits), place clean publish+merge runs in `Done` (quiet under `email-unless-merged`), and send failed or unmerged-after-publish runs to `Needs you` + email** (settling once the bump PR is merged/closed, a later **Release run** starts/succeeds, or dismissed).
- (b) Always email on a published release even under `email-unless-merged`.

*Recommended:* (a).

*Answer:* agreed

## Round 3 (answered)

**Q14 - GitHub tag & workflow authorization (who can publish a release).**
- (a) **Enforce both in `release.yml` (via `scripts/generate-release.py`) and with a GitHub Tag Ruleset**:
  - In `release.yml`: before publishing on a tag push, verify `github.actor == 'JacobStephens2'` *and* that the tagged commit's tree equals a commit on `origin/main` plus only the `Cargo.toml`/`Cargo.lock` version bump for that tag.
  - On GitHub: add a `refs/tags/v*` ruleset allowing tag creation/updates only by **Repository admin** (`JacobStephens2`) and blocking tag deletion, so `write` collaborators cannot push release tags while unattended cron and unpublished-tag recovery still work.
- (b) Enforce the `github.actor` + exact `main` bump-tree gate in `release.yml` only.
- (c) Leave `release.yml` and GitHub rulesets unchanged.

*Recommended:* (a).

*Answer:* agreed

**Q15 - Blocking in-session agent calls and Console API scope.**
- (a) **Refuse `thirdshift release` when `THIRDSHIFT_SESSION` or `THIRDSHIFT_PARENT_RECORD` is set, add tag-push and `thirdshift release` detection to `destructive.rs`, and require `admin` scope (like `update`) to start `release` via the Console / `/api/v1/start` (with `release --review` CLI-only).**
- (b) Same session refusal + `destructive.rs` detection, and make `thirdshift release` completely CLI-only.
- (c) Same session refusal + `destructive.rs` detection, keeping `run` scope on `/api/v1/start`.

*Recommended:* (a).

*Answer:* agreed

**Q16 - Prompt criteria for `minor` vs `patch` (and fixing duplicate fallback notes).**
- (a) **Instruct the Harness in `src/prompt.rs` to choose `minor` when merged PRs add a new command, flag, Console page/action, config key or user-visible contract change, and `patch` when they only fix bugs, harden edge cases or refine internals — and keep `generate-notes` out of `<!-- release-summary:start -->` on fallback so `scripts/release-notes.sh` never duplicates `## What's Changed` on the GitHub Release page.**
- (b) Use a different rule for `minor` vs `patch`.

*Recommended:* (a).

*Answer:* agreed

## Settled by earlier answers (shout if any is wrong)

- **Release run** (`thirdshift release`) is a native Rust **Command** scoped to `JacobStephens2/thirdshift`, replacing the former release shell script, the former release Experiment helper and the former summary shim while keeping `scripts/release-notes.sh` for `.github/workflows/release-notes.yml`. Added to `GLOSSARY.md` and `ADR 0026`. (Q1, Q2, Q16)
- CLI shape: `thirdshift release [<version> | --major | --minor | --patch] [--with-experiment] [--review] [--harness <name>] [--model <name>] [--effort <level>] [--email [<addr>] | --email-unless-merged [<addr>] | --no-email]`, with `<version>` and the bump flags mutually exclusive. (Q4, Q5, Q8)
- Any versionless invocation (`thirdshift release`, `--patch`, `--minor`, `--major`) is a **Pass**; `thirdshift release <version>` is a direct **Command** that cuts or resumes `<version>` even without shipped changes. (Q1, Q3, Q8)
- In Pass mode, a candidate `main` commit needs at least one diff since the last `v*` tag in shipped-artifact or release-pipeline paths (`src/`, `skills/`, `processes/`, `Cargo.toml`, `Cargo.lock`, `dist-workspace.toml`, `rust-toolchain.toml`, `README.md`, `LICENSE`, `.github/workflows/release*`, `.github/workflows/publish-crates.yml`, `scripts/release*`). Docs-only, `site/`-only, `.agents/`-only and test-only days skip quietly. (Q3)
- When neither `<version>` nor a bump flag is given, one `bwrap`-sandboxed, traced **Session** (after any `--with-experiment` check) reads the merged PRs since the last tag, picks `minor` or `patch` (never `major`), and writes the release summary; if the **Harness** fails or returns malformed output, it falls back to `patch` and GitHub's generated notes without duplicating `## What's Changed` inside `<!-- release-summary:start -->`. (Q4, Q7, Q12, Q16)
- `--review` refuses up front without a terminal; `--with-experiment` prompts `release anyway? [y/N]` only on a terminal and fails cleanly on `fail` or `inconclusive` when unattended. (Q5)
- `thirdshift release` executes entirely from the running `thirdshift` binary against fetched `origin/main` in temporary worktrees (closing #336), sandboxes the summary session with `bwrap` (closing #653), and stops after publishing + merging the bump PR, leaving `thirdshift update` separate. (Q6, Q12)
- Concurrency and reservations: honors live **Experiment** and **Quota wait** Pass gates; holds `~/.thirdshift/release.lock` locally plus an atomic `refs/thirdshift/reservations/release` lease on `origin` and `in-progress` on the bump PR while live, outside **Command limit**. Unattended `Review` (`--author`) and `Fix` Passes skip `release-*` branches targeting `main`. (Q9)
- Unfinished `release-<X.Y.Z>` state on `origin`: Pass mode auto-resumes/recovers recoverable states (pushed branch, open non-draft exact-bump PR, unpublished failed `release.yml`) and skips human-blocked states (such as a draft bump PR after a failed automatic Fix run). (Q10)
- `main` push CI: selects the newest already-green `main` commit with shipped changes since the last tag; waits if no candidate is green yet while a candidate's `ci.yml` is queued/running; skips (Pass) or fails (direct) if all candidates with shipped changes finished non-green. (Q11)
- Authorization and guardrails:
  - `release.yml` (generated via `scripts/generate-release.py`) verifies on tag push before publishing that `github.actor == 'JacobStephens2'` and that the tagged commit's tree is an exact `Cargo.toml`/`Cargo.lock` bump off a commit on `origin/main`. (Q14)
  - A GitHub `refs/tags/v*` ruleset (provisioned via setup/wizard or `gh api`) restricts tag creation/updates to **Repository admin** (`JacobStephens2`) and blocks tag deletion. (Q14)
  - `thirdshift release` refuses when `THIRDSHIFT_SESSION` or `THIRDSHIFT_PARENT_RECORD` is set; `destructive.rs` flags tag pushes and `thirdshift release`; `/api/v1/start` and Console Start controls require `admin` scope for `release`, with `release --review` CLI-only in `ADR 0026`. (Q13, Q15)
- **Inbox** & **Run notifications**: live runs show in **Working**; clean publish+merge lands in **Done** (saved, not emailed under `email-unless-merged`); failed runs or unmerged-after-publish runs land in **Needs you** and email even under `email-unless-merged`. **Cron jobs** recognizes `thirdshift release` and records Pass last-fired time. (Q1, Q13)
- Nothing is left open: the design tree is complete.

## Research results (2026-11-13)

- Live GitHub settings (`gh api repos/JacobStephens2/thirdshift`): public repo, `rulesets: []`, `main` unprotected (404), tag protection absent (404), `environments: []`. Collaborators with `push`: `JacobStephens2` (`admin`) and `vsto-eta` (`write`). Authenticated host user: `JacobStephens2`.
- Outsiders and non-collaborator `CONTRIBUTOR`s cannot push branches/tags, merge PRs, trigger `JacobStephens2/thirdshift` tag workflows from forks, pass crates.io OIDC trusted publishing (`.github/workflows/publish-crates.yml`), or pass `src/trust.rs` (`OWNER`, `MEMBER`, `COLLABORATOR` only).
- `processes/` is compiled into the binary (`src/processes/catalog.rs`, `include_dir!("$CARGO_MANIFEST_DIR/processes")`), alongside `skills/` (`src/skills.rs`), though `RELEASE_PATHS` in `scripts/generate-release.py` previously omitted `processes/**`.
- Because the retired release shell script tags `vX.Y.Z` on `release-X.Y.Z` before merging `release-X.Y.Z` into `main`, `origin/main` always has at least that merge commit ahead of `vX.Y.Z` even when nothing landed after the release (`git log --graph origin/main`).
- `v0.29.2`'s GitHub Release duplicated `## What's Changed` because the former release shell script wrote `generate-notes` inside `<!-- release-summary:start -->` on agent failure and `scripts/release-notes.sh` appended `generate-notes` again.
- Every factory Harness session carries `THIRDSHIFT_SESSION` (`src/process/session.rs:13`).
- Open issues directly subsumed by this design: [#336](https://github.com/JacobStephens2/thirdshift/issues/336) (release without `git pull` / local working-tree `cargo run`) and [#653](https://github.com/JacobStephens2/thirdshift/issues/653) (sandbox release summary with `bwrap`).

## Waiting on research

- None.
