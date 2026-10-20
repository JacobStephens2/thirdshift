# The "How to use thirdshift" tutorial video: commands and outline

Research date: 2026-10-19.

The general findings — van der Meij's guidelines for software-training video, real versus rendered terminals, legibility, length and chapters, and accessibility — are in the explainer-video skill's [`research/tutorial.md`](https://github.com/JacobStephens2/skills/blob/main/skills/explainer-video/research/tutorial.md). This note is its §5 for thirdshift: what the video must show, command by command, with the output a viewer should expect at each step.

**Method.** Cites this repo with `file:line` — `README.md`, `docs/guide.md`, `site/index.html`, and `src/` only to confirm a command or flag exists.

The repo has a complete real transcript of a *successful* Run and **nothing** for `status`, `show`, `logs`, `setup`, `update` or the installer (§5.9). The project publishes real stderr as proof (`site/index.html:105`, `:316`), so the video records a real terminal.

## 5. What the video must show, from the repo

**5.1 Prerequisites.** The account needs (`docs/guide.md:240-246`) the **Harness** "installed and authenticated: **`claude`** (Claude Code, the default), **`codex`**… or **`opencode`**"; "**`gh`** (GitHub CLI), logged in"; "**`git`** with a global `user.name` and `user.email`, and credentials that can push to the repository (`gh auth setup-git` makes git use `gh`'s login)"; "**`node`**… for a Security run only". Platforms: "x86_64 Linux (Rocky, Ubuntu and WSL2) and Apple Silicon macOS" (`README.md:19`). Two lines belong on screen: sessions "run headless with the full permissions of that user account, including `sudo` if the user has it, and nobody is there to approve anything" (`:249`), and "**Install the target repository's toolchains up front**" (`:257`). Show `type thirdshift claude gh git` — the guide's own idiom (`:228`, `:905`) — and `gh auth status`. **Output: not documented; record it.**

**5.2 Install.** Verbatim (`README.md:13`):

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
```

The short redirect `curl -fsSL https://thirdshift.app/install.sh | sh` (`site/index.html:71`) "redirects to the same installer" (`docs/guide.md:209`). It "puts a single binary in `~/.local/bin`; no Rust toolchain or checkout of this repo is needed" (`README.md:15`) and "ends by pointing you to Setup" (`:211`). Alternative: `cargo install thirdshift` (`:215`). **Output: not documented.**

**5.3 `thirdshift setup`.** Writes `~/.thirdshift/config.toml` (`:347`) "listing every setting at its default so the file itself shows what can be changed" (`:397`), printed in full at `:399-458`. From a terminal it asks eight things in order (`:465-472`): Harness, then its Model and Effort; `merge.always`; `base.fix`; `launch.pull`; `security.fix`; `security.review`; then notifications, address, sender and the Resend key "with input hidden". "Pressing Enter takes the default shown" (`:474`); it "prints the file's path on stderr and exits `0` with stdout empty". Only two prompt strings are quotable — `Resend API key (input hidden, Enter to skip):` (`:472`) and `No User config at <path>. Set your defaults now? [Y/n]` (`:476`). **The rest is described, not quoted: record the transcript.**

**5.4 First Run.** Verbatim (`:268-270`):

```sh
cd ~/repos/widgets          # a clone of the issue's repository, on the Base branch
thirdshift https://github.com/acme/widgets/issues/7
```

Shorthand is real (`:274-278`): `thirdshift 7`, `thirdshift merge 7`, `thirdshift --no-merge 7 harness codex`. "A Run takes minutes to tens of minutes" (`:288`). `no-merge`/`--no-merge` goes "before or after the URL" (`:296`); confirmed at `src/args.rs:459-460`.

**Output — real and quotable.** `site/index.html:105` states the lines come from a real Pickup run on 2026-10-12; `:316` that they are "copied, timestamps and all, from the Pickup run's Command log… there are 139 lines on stderr in all." Use them in order from `:208-305`: `sessions run on codex · gpt-6.1-sol · high`, `labelling #663 in-progress, in place of ready-for-agent`, `creating worktree /home/jacob/repos/thirdshift-issue-663 on issue-663 from origin/main`; then session lines as `implement: $ <command>`, closing `implement: session ended after 26m 2s: 7416316 input tokens (7285760 cached), 20664 output tokens` (`:263`); then `pushing issue-663`, `checking the PR`, `merging origin/main into issue-663`, `waiting up to 60s for CI on c16e780`, `CI on c16e780: 6 of 8 checks still running`, `CI passed on c16e780`, `origin/main moved while CI ran; merging it again`, and the cleanup and merge lines ending `PR …/pull/665 is merged`.

The format is fixed (`docs/guide.md:336`): every line "starts with the local time it was printed, as in `thirdshift: 12:14:49 pushing issue-7`"; the first names date and offset; a top-level Run prints `Took: 1h 2m 3s`; the last line is `PR <url> is ready for review` or `PR <url> is merged`. **stdout carries only the PR URL** — `url=$(thirdshift "$issue") && echo "ready: $url"` (`:334`). Two warnings: "Your checkout's uncommitted changes stay out of the Run. Push any Base branch commits first" (`README.md:37`), and blunter — "Unpushed commits on the Base branch are not: push them first, or the Run stops" (`docs/guide.md:343`).

**5.5 Watching.** `thirdshift status` "names this host and lists live Commands across every repository in `logs.dir`, with their issue and title when present, Harness, Model, Effort, current stage and its age" (`:76`). Verbatim (`:166-172`):

```sh
thirdshift show 'widgets#7'
thirdshift logs 'widgets#7' --session implement
thirdshift logs 'widgets#7' --session repair --follow
```

`show` gives "the Runs, stage history with start times, outcomes, and Command and Session log paths" plus "its latest event, such as `Bash: cargo test`" (`:174`). `logs` "writes the Command log verbatim"; `--follow` "first prints the selected log, then flushes appended bytes until the owning Command ends or dies" (`:199`). Confirmed at `src/args.rs:254,269,280,299`. **Flag: the guide shows no human-readable sample output for `status`, `show` or `logs`.** The only rendered example is the idle line (`:82`): `Architect run idle on acme/widgets: Ready issue #7 "Make widgets" goes first: …`. Everything else is JSON (`:88`, `:138`, `:178`). Stage names are quotable (`:120`): `implement`, `Resume`, `Security review`, `pull request`, `CI watch`, `Repair`, `Self-merge`, `cleanup`.

**5.6 What comes back, and a Failed run.** "Ready" means (`site/index.html:339`): "An open pull request from the Issue branch to the Base branch. It is mergeable, its CI passes, and it has been reviewed against your repository's documented coding standards and against the issue. Otherwise thirdshift doesn't call it ready." Findings the agent declined are "listed as an Unaddressed finding in the pull request body, with the code, plan decision, ADR or focused run that refutes it" (`:340`). A Failed run "ends… without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI" (`docs/guide.md:724`); it "commits any uncommitted work… and pushes the Issue branch, so nothing is lost" and converts an open PR "back to a draft" (`:728-729`). Rerunning is the same command — it "picks up where the last Run stopped" as a "**Continuation**" (`:712-721`) — with the caveat on screen: "A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit" (`:722`).

The site's Failed-run block is explicitly hypothetical — "**Illustrative** … Their order is inferred from the code, not seen in a real Run" (`site/index.html:321-329`). **Prefer the one real failure transcript** (`docs/guide.md:745-752`), which prints the retry command:

```
thirdshift: 03:12:40 CI red on test, which also fails on main at 362b9ca; fix main first
thirdshift: 03:12:40 Retry with: thirdshift https://github.com/acme/widgets/issues/7 base-fix
```

**5.7 Scheduling a Pickup run.** "thirdshift has no scheduler of its own… the operating system's scheduler runs the ordinary command" (`:1107`). Verbatim (`:1112-1115`):

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

Narrate three things (`:905`, `:1118`): `PATH` is set "because cron doesn't read your shell profile", and "cron expands neither `~` nor `$HOME` on that line"; `cd` "makes a clone of the repository the **Launch directory**" and `base main` names the Base branch "so the branch checked out in the clone doesn't matter"; and the throttles — "a pass takes one issue, so `*/30` starts at most one every half hour", "at 3 of them, or the `pickup.limit` you set, the passes take nothing until you have dealt with one", and a newly labelled issue "waits thirty minutes by default". Cautious variant: `… pickup base main no-merge` (`:1140`). Add with `crontab -e`, and "end each with a newline" (`:1136`). **Output is real** (`site/index.html:206-207`): `Pickup run starting, 2026-10-12 -0400`, then `taking Ready issue #663 "…", as thirdshift …/issues/663 would`.

**5.8 `help` and `update`.** `help` prints "to stdout and exit `0`" (`docs/guide.md:339-341`); **the real text is `src/main.rs:60-103`**, opening "thirdshift turns a GitHub issue into a ready-for-review pull request, unattended." (`:61`), with `usage:` at `:63-84` and the shared-options block at `:92`. `update` "replaces the installed binary with the latest stable GitHub Release, or says it is already on it. Messages go to stderr and stdout stays empty" (`docs/guide.md:221-223`), and "only replaces a copy put in place by the shell installer", refusing "to touch any other copy" (`:227`). Confirmed at `src/args.rs:236,238`. **Output: not documented for either.**

### 5.9 Output the docs never show — record it or drop the beat

| Beat | Documented output? |
| --- | --- |
| The `curl` installer's own output | **No** (`README.md:13` gives the command only) |
| The `setup` questions | Described, not quoted (`docs/guide.md:465-474`); two strings exist (`:472`, `:476`) |
| `status` / `show` / `logs --follow` human output | **No sample anywhere** — only the idle line (`:82`) and JSON (`:88`, `:138`, `:178`) |
| `thirdshift update` | **No sample** (`:221`) |
| Prerequisite checks | **No sample** |
| A Failed run's stderr | Only the "illustrative" block (`site/index.html:321`) and the real Inherited-failure block (`docs/guide.md:745`) |
| A successful Run's stderr | **Real and complete** (`site/index.html:206-305`; 139 lines, `:316`) |

## Tutorial cut: thirdshift rules and outline

These add to the general rules in the skill's note.

1. **Never show a line the binary does not print.** Quote `site/index.html:206-305`, `docs/guide.md:336`, `:745-752` and `src/main.rs:60-103` character for character; record everything else from life.
2. **Caption any placeholder or reconstructed output as illustrative**, as the site does itself (`site/index.html:321`).
3. **One chapter per §5 heading**, 45–90 s each, 7–10 min total.
4. **Open on the finished pull request.**
5. **Demonstrate thirdshift's decisions, not git, `gh` or cron syntax.**

| # | Chapter | Beat: command | Expected output to show | Narration gist | Source |
|---|---|---|---|---|---|
| 0 | Where we end up | — | The merged PR and `PR …/pull/665 is merged` | "One issue URL in, one reviewed pull request out." | `site/index.html:305`, `:339`; G4 |
| 1 | Prerequisites | `type thirdshift claude gh git`; `gh auth status` | **Record live — undocumented** | Harness + `gh` + git identity; sessions run as you, unattended, with no approver | `docs/guide.md:240-246`, `:249` |
| 2 | Install | `curl -LsSf …/thirdshift-installer.sh \| sh` | **Record live**; end on the Setup pointer | One binary into `~/.local/bin`; no Rust, no checkout | `README.md:13-15`, `docs/guide.md:211` |
| 3 | Setup | `thirdshift setup` | **Record the transcript**, then the written `config.toml` | Eight questions, Enter takes the default; the file lists every setting | `docs/guide.md:465-474`, `:399-458` |
| 4 | Push first | `git push` before the Run | The rule as on-screen text | "Unpushed commits on the Base branch… push them first, or the Run stops" | `docs/guide.md:343`, `README.md:37` |
| 5 | First Run | `cd ~/repos/widgets`; `thirdshift <Issue URL>` | `labelling #663 in-progress…`, `creating worktree …`, `implement: $ …`, `pushing issue-663`, `CI passed on …`, `PR <url> is ready for review` | Claim, worktree, one session implements *and* reviews, then thirdshift takes over deterministically | `docs/guide.md:268-270`, `:336`; `site/index.html:208-305` |
| 6 | Watching | `thirdshift status`; `thirdshift show 'widgets#7'`; `thirdshift logs … --follow` | **Record live — no sample in docs**; name the stage on screen | Host, live Commands, stage and age; `show` adds the latest event; `logs --follow` tails | `docs/guide.md:76`, `:174`, `:199`; `src/args.rs:254,269,280` |
| 7 | What comes back | The PR body on GitHub | The ready-for-review definition; one Unaddressed finding | "Otherwise thirdshift doesn't call it ready" | `site/index.html:339-340` |
| 8 | When it fails | re-run the same command | `CI red on test, which also fails on main…`; `Retry with: … base-fix` | Work is committed and pushed, the PR drops to draft, the same command continues it — and it is not idempotent | `docs/guide.md:724-729`, `:745-752`, `:712-722` |
| 9 | Merging | `thirdshift merge <Issue URL>` / `--no-merge` | `merging the PR into main`; `PR <url> is merged` | Self-merge only once open, ready, mergeable and green; never GitHub auto-merge | `docs/guide.md:293-296`; `site/index.html:268`, `:304-305`; `src/args.rs:459` |
| 10 | On a schedule | `crontab -e`, paste the `PATH` + `*/30` lines | `Pickup run starting, 2026-10-12 -0400`; `taking Ready issue #663 …` | thirdshift schedules nothing; `PATH` because cron reads no profile; one issue per pass, 3 Claims and it stops | `docs/guide.md:1107-1122`; `site/index.html:206-207` |
| 11 | Help and update | `thirdshift help`; `thirdshift update` | The real help text; **record `update` live** | Where to look next; `update` only touches a shell-installer copy | `src/main.rs:60-103`; `docs/guide.md:221-227` |
