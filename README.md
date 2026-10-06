# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it. The humans are the day shift; the agents work the third shift, overnight, on any server, while you do something else.

Quality over quantity: a pull request marked ready for review is one the factory stands behind. When it can't stand behind the work, the **Run** is a **Failed run**: the work is pushed so nothing is lost, and the pull request goes back to a draft.

Terms in **bold** are defined in [`CONTEXT.md`](CONTEXT.md).

## What a Run does

From a clone of the issue's repository, `thirdshift <Issue URL>`:

1. Checks that the Run makes sense, before creating anything:
   - the **Origin match**: the **Issue URL** must belong to the same GitHub repository as the `origin` remote of the directory you run it from;
   - the issue is open;
   - the **Base branch** is well defined: HEAD is not detached, the branch exists on `origin`, and your local copy is not ahead of it, so unpushed commits can't leak into the pull request;
   - a local Issue branch, if you have one, points at the same commit as its copy on `origin`, so local-only commits are never destroyed.

   Any failed check stops the Run before any work happens.
2. Picks the **Issue branch** (`issue-<n>`, or `issue-<n>-branch-<k>` once earlier ones are finished) and the **Base branch**, normally the branch you have checked out. Then it makes the [Claim](#the-claim): it labels the issue `in-progress`, in place of `ready-for-agent` if it has that.
3. Creates a git worktree next to your clone, named `<repo>-<Issue branch>`, so your own checkout is never touched.
4. Runs a headless Claude Code session in that worktree with the **Factory skills** in [`skills/`](skills/) linked into its `.claude/skills/` as `thirdshift-<skill>` and kept out of git by a `.git/info/exclude` entry ([ADR-0012](docs/adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)), started with one of the **Session prompts** in [`prompts/`](prompts/). The agent implements the issue, reviews its work against the Base branch, addresses the **Standards findings** and **Spec findings** it agrees with, and opens a ready-for-review pull request that lists every **Unaddressed finding** with a reason and closes the issue.
5. Takes over deterministically: pushes the Issue branch, skipping your repo's git hooks since the session runs the tests and CI gates the pull request, checks through `gh` that the pull request exists, is open and targets the Base branch, and marks it ready for review (`gh pr ready`) if the agent left it as a draft.
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 5 per Run. A failed check that also failed, under the same name, on the Base branch commit the head last merged in is an **Inherited failure**, not the branch's to fix: the CI-fix Repair is given only the branch's own failures, with the Inherited failures listed as not to fix. If every failed check is an Inherited failure, no Repair starts: the Run merges the Base branch again if it has moved since, and otherwise ends as a [Failed run](#failed-runs) that says to fix the Base branch first, unless it was given `base-fix` or the [User config](#user-config) sets `base.fix`, when it starts a [Base fix](#base-fix) first. A check that is pending, passed or missing on that Base branch commit is the branch's own, and thirdshift never triggers or waits for the Base branch's CI. If a CI-fix Repair leaves the head unchanged, with no commit of its own and no Base branch move to merge, thirdshift does a **Check re-run**, once for that head: it asks GitHub to re-run the branch's own failed checks (`gh run rerun <run-id> --failed`, once per workflow run), says so on stderr as `re-running the failed checks on <short sha>: <check>[, <check>…]`, and watches CI on that head again, from the new attempt on. A check that failed for a reason that does not repeat, such as a flaky test or a runner fault, then passes, and the Run goes on as for any green head. If CI is red again, the Run ends as a Failed run, with no second Repair or re-run for that head. It ends the same way, with nothing re-run, if one of those checks can't be re-run (a commit status, or a check run that is not a GitHub Actions job), since the head could not go green, or if GitHub refuses the re-run, which stderr shows. What the Repair concluded is never read, Inherited failures are never re-run on their own account, and a Check re-run is no Repair, so it counts against no budget ([ADR-0010](docs/adr/0010-one-check-re-run-before-a-declined-ci-fix.md)). If the Base branch moves while CI runs, it merges it again and goes round, at most 5 times per Run. In a Merge run, each round also takes in **Foreign commits** first: see [Foreign commits in a Merge run](#foreign-commits-in-a-merge-run).
7. In a **Merge run** (`thirdshift merge <Issue URL>`), does the **Self-merge**: once the pull request is open, ready for review, mergeable and green, thirdshift merges it into the Base branch with a merge commit, on exactly the head commit whose CI it watched (`gh pr merge --merge --match-head-commit <sha>`). It never uses GitHub's auto-merge ([ADR-0004](docs/adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)). A merge that fails goes back round step 6, within the same budgets, and is tried again on the new green head. If that round finds nothing to fix, the refusal is a **policy refusal**, such as merge commits being disallowed or a review being required. After the merge, thirdshift deletes the Issue branch on `origin`, and closes the issue if it is still open, with the comment `Closed by #<pr>, merged into <base> by a thirdshift Merge run.` GitHub closes it on its own only for a merge into the repository's default branch, and then thirdshift leaves it alone. Once the issue is closed, either way, thirdshift takes `in-progress` off it, [removing the Claim](#when-the-claim-ends).
8. Cleans up: removes the worktree, the local Issue branch and the temporary directory the Factory skills were written out to, whether the Run succeeded or not. The one exception is a **Failed run** whose work could not be pushed: see below.

### The Claim

The **Claim** is the mark that the factory has taken an issue, so the issue list shows what it is working on. When a Run or a [Spec run](#spec-runs) starts on an issue, thirdshift labels the issue `in-progress`, in place of `ready-for-agent` if it has that. An issue with no `ready-for-agent` label still gets `in-progress`, so the label means the same thing however the Run was started.

- It is made once every check of step 1 has passed and the branches are picked, and before the worktree is created. A Run stopped by a check changes no label.
- It is one request, which adds `in-progress`, removes `ready-for-agent` and keeps the issue's other labels, so the swap can't stop halfway. stderr says `labelling #<n> in-progress, in place of ready-for-agent`, or `labelling #<n> in-progress`.
- thirdshift first creates the `in-progress` label if the repository has none.
- It is made however the Run or Spec run was started: by you on an Issue URL, by an [Architect run](#architect-runs) for the plan it dispatches, or by a [Pickup run](#pickup-runs) for the Ready issue it took.
- A Ticket's Run in a Spec run makes none, since the Spec carries the Claim, and nor does a [Base fix](#base-fix): their issues' labels are left as they are.
- A [Continuation](#continuation) on an issue that is already `in-progress` changes nothing and makes no request. One on an issue that isn't Claimed yet Claims it.
- If the Claim can't be made, the Run stops there, before any work, with exit `1` and the cause: `could not make the Claim on #<n>: <gh's error>`.

#### When the Claim ends

How the Run or the Spec run ends decides what becomes of its Claim:

- **Released**, when it fails with nothing on `origin` to take over: no Issue branch for the issue there and no pull request from one, open, merged or closed, or for a Spec run, no Spec branch and no Spec PR. That is a session that fails before it changes anything, as on a usage limit or an expired login, a Ctrl-C before any work, or a worktree that can't be created, on an issue that was never started before. A retry that fails that way on an issue an earlier Run left an Issue branch or a pull request for keeps its Claim. The issue's labels go back as they were before the Claim: `in-progress` comes off, and `ready-for-agent` goes back if the issue had it, and only then. So an outage costs a retry and doesn't strand the issue. stderr says `releasing the Claim on #<n>: labelling it ready-for-agent, in place of in-progress`, or `releasing the Claim on #<n>: removing in-progress`. A Failed run whose push failed is released too, since nothing of it reached `origin`: its work is only in the worktree it kept, which stops the next Run on that machine at the pre-flight checks until you push or delete the local Issue branch.
- **Kept**, on any other ending that leaves the issue open: a pull request ready for review, or a [Failed run](#failed-runs) that pushed its Issue branch or left a pull request. The issue stays `in-progress` until you relabel it, so a failure waits for you.
- **Removed**, in a Merge run, once the Self-merge has left the issue closed, whether thirdshift closed it or GitHub did: `in-progress` comes off, after `removing in-progress from issue #<n>` on stderr.

A release undoes only what the Claim changed, in one request that keeps the issue's other labels, any added during the Run included. An issue that was already `in-progress` when the Run started stays `in-progress`, with `ready-for-agent` back if the Claim took it off (`releasing the Claim on #<n>: labelling it ready-for-agent again`), and one whose `in-progress` someone took off during the Run is left as it is. A Claim that can't be released, or can't be removed after a merge, doesn't change how the Run ends: stderr gets a `warning:` line naming the command to run by hand.

A process killed outright, with `kill -9` or by a power cut, releases nothing: the issue stays `in-progress` for you to relabel.

An issue closed some other way, as when you merge its pull request by hand, keeps `in-progress` until a [Pickup run](#pickup-runs) takes it off, in the [Sweep](#the-sweep).

### Status

The designed behaviour described in this README is implemented. Known gaps and planned work are tracked in the [open issues](https://github.com/JacobStephens2/thirdshift/issues).

## Install

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
```

The shell installer and the binary both come from this repository's [GitHub Releases](https://github.com/JacobStephens2/thirdshift/releases). It puts `thirdshift` in `~/.local/bin`, where Claude Code's installer puts `claude`, and needs no Rust toolchain. thirdshift is a single binary with the Factory skills compiled in, so it runs without a checkout of this repository ([ADR-0001](docs/adr/0001-rust-binary-with-embedded-skills.md), [ADR-0003](docs/adr/0003-static-binaries-through-github-releases.md)).

The installer ends by pointing you to Setup. Run `thirdshift setup` to choose your defaults, or just start a Run and thirdshift will offer it (see [User config](#user-config)).

If you use Rust, you can install it from crates.io instead:

```sh
cargo install thirdshift
```

### Updating

```sh
thirdshift update
```

It replaces the installed binary with the latest stable GitHub Release, or says it is already on it. Messages go to stderr and stdout stays empty. It exits `0` when it updated or was already up to date, and `1` on any failure, such as no network. Updating is safe while a Run or a Spec run is using the old binary: it keeps running it. On Linux so do the child Runs it starts afterwards, a Ticket's Run or a Base fix, which it starts from the binary it is running rather than from the install path, so they run its version even once that path holds the new one or nothing at all. On macOS a child Run is started from the install path, so one started after the update runs the new binary. A Run never checks for updates or updates itself.

`thirdshift update` only replaces a copy put in place by the shell installer (the `curl` command above), which leaves an install receipt in `~/.config/thirdshift/`. It refuses to touch any other copy and lists the command that updates each other kind of install: `cargo install thirdshift` for a crates.io install, pulling and reinstalling for a build from source (see [Building from source](#building-from-source)), and the `curl` command for a copy placed by hand.

Keep a single copy: with copies in more than one directory on your `PATH` (say `~/.local/bin` and `~/.cargo/bin`), you can end up running a stale one without noticing. `type -a thirdshift` lists every copy on your `PATH`.

## Supported platforms

- **Rocky Linux** 9.8 and later, x86_64.
- **Ubuntu** 24.04.3 and later, x86_64.
- **WSL2**, latest release, with Ubuntu 24.04 or later, x86_64. Keep your repositories on the Linux filesystem (e.g. `~/repos`), not under `/mnt/c`, where git is slow and file permissions misbehave. WSL1 is not supported.
- **macOS** Tahoe 26.5.2 and later, Apple Silicon.

One statically linked Linux binary covers the first three, so it doesn't depend on the host's glibc; macOS gets a native build.

## Prerequisites

The user account that runs thirdshift needs:

- **`claude`** (Claude Code), logged in.
- **`codex`** (Codex CLI), logged in, only for `harness codex`.
- **`gh`** (GitHub CLI), logged in.
- **`git`** with a global `user.name` and `user.email`, and credentials that can push to the repository (`gh auth setup-git` makes git use `gh`'s login). The agents commit as this identity; without it, an agent may borrow the author of the last commit.

### Auto mode

Sessions run headless in Claude Code's auto mode (`claude -p --permission-mode auto`), with the full permissions of that user account, including `sudo` if the user has it. Nobody is there to approve anything; instead, auto mode's classifier checks each action and may block ones it judges risky, such as destructive commands or actions outside the task. A blocked action the agent can't work around can end the Run as a Failed run.

`claude -p` exits as soon as the agent ends its turn, killing any background task it started. So every prompt tells the agent to run long commands, such as tests, in the foreground, and to stop any background task it no longer needs, with the `TaskStop` tool, before it ends its turn. If a session still ends with background work running, thirdshift takes it as work the session was waiting on and gives it a **Resume**: it continues that same session once (`claude -p --resume`), asking the agent to re-run the work in the foreground and finish. If the Resume ends the same way, the work may have been abandoned, such as a hung test run the agent could not stop: thirdshift says which task was killed and carries on, so the pull request check, CI and the rest decide how the Run ends. If the Run then fails, its cause names the killed task ahead of what failed.

### What sessions leave behind

Cleanup removes only thirdshift's own worktree, local Issue branch and the temporary directory it wrote the Factory skills out to. The `.git/info/exclude` entry that keeps the linked skills out of git, `/.claude/skills/thirdshift-*`, or `/.agents/skills/thirdshift-*` on Codex, is added once and left in place, as every worktree of the repository shares the file. Anything else an agent does as your user stays. For example, one Run downloaded a JDK to `~/.local/jdk/` because the server had no Java, installed Playwright in `/tmp/pw`, and left a pull request body draft in `/tmp/`. This is by design, but worth knowing:

- **Install the target repository's toolchains up front** (language runtimes, package managers, test tools), so the agents don't download their own on every Run.
- **For UI work, install the headless browser dependencies**, e.g. `libgbm1` on Ubuntu server, which headless Chromium needs. Without it, agents can't take screenshots to check a UI change and fall back to reading the code.

### Usage and cost

When `claude` is logged in with a claude.ai subscription, Runs use the plan's usage allowance, not API billing. The `total_cost_usd` in the session logs is the API-price equivalent, not a charge. An unattended Run that hits the plan's usage limit stalls or fails partway through.

## Usage

```sh
cd ~/repos/widgets          # a clone of the issue's repository, on the Base branch
thirdshift https://github.com/acme/widgets/issues/7
```

A Run takes minutes to tens of minutes.

To have the Run merge its pull request instead of leaving it for your review, start a **Merge run**:

```sh
thirdshift merge https://github.com/acme/widgets/issues/7
```

`thirdshift --merge <Issue URL>` does the same, and either goes before or after the URL. The agent gets the same implement prompt either way; the differences are the Self-merge at the end and the review of [Foreign commits](#foreign-commits-in-a-merge-run). To make every Run on a machine a Merge run, set `merge.always` in the [User config](#user-config); `no-merge` (or `--no-merge`), before or after the URL, then makes one Run end ready for review instead. Giving a flag twice, or `merge` together with `no-merge`, is an argument error.

A Merge run ends in one of three ways:

- **Merged**: exit `0`, the pull request's URL on stdout, and `PR <url> is merged` as stderr's last line.
- **Policy refusal**: GitHub refused the merge and a round of merging the Base branch and watching CI found nothing left to fix. Exit `1`, the pull request's URL on stdout, and GitHub's error on stderr. The pull request stays open and ready for review, not a draft, for you to merge by hand or to change the repository's settings. thirdshift never reads GitHub's error text to decide this.
- **Any other failure**: a [Failed run](#failed-runs), as for a Run without `merge`.

To be emailed when the Run ends, add `--email` (or `email`), optionally followed by the address, before or after the URL and alongside `merge`:

```sh
thirdshift --email you@example.com https://github.com/acme/widgets/issues/7
```

It sends one [Run notification](#run-notifications), whatever the outcome. To have every Run on a machine send one, set `email.always` in the [User config](#user-config); `no-email` (or `--no-email`) then skips it for one Run.

- **stdout** carries only the pull request's URL: on success, and on a Failed run that leaves an open pull request, a draft or, after a policy refusal, one ready for review. The exit code tells the two apart, so script it as `url=$(thirdshift "$issue") && echo "ready: $url"`.
- **stderr** carries everything else: errors, cleanup problems, and progress lines while sessions run. Each line starts with the local time it was printed, as in `thirdshift: 12:14:49 pushing issue-7`, so a quiet terminal shows how long the Run has been on its last step. The first line of a Run, a Spec run, an [Architect run](#architect-runs) or a [Pickup run](#pickup-runs) also names the date and the UTC offset, as in `thirdshift: 12:00:01 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400`, so a log that collects many commands, such as a crontab line's, reads as a dated list. Everything on stderr and stdout is kept in the command's [Command log](#logs) too. A successful Run's last line names the pull request too: `PR <url> is ready for review`, or `PR <url> is merged` after a Merge run, followed only by a `warning:` line if a [Run notification](#run-notifications) can't be sent.
- **Exit code** `0` means the Run ended with a pull request the factory stands behind, merged in a Merge run. Once the Self-merge has merged, the Run succeeds even if deleting the Issue branch on `origin`, closing the issue or removing `in-progress` from it then fails: the merge can't be undone, so each failed step is a `warning:` line on stderr naming the command to run by hand, and the Run still exits `0` with the URL on stdout. Ctrl-C likewise: before the merge it makes a Failed run, after it thirdshift finishes these steps and exits as merged. `2` means the command is one thirdshift can't use: the Issue URL is missing or isn't a GitHub Issue URL, there is an argument other than the URL and the Run flags, a Run flag is repeated or contradicts another, or `parallel` isn't followed by a whole number from 1 up; the error and the help text go to stderr, before any work. A [User config](#user-config) thirdshift can't use exits `1`, also before any work. Any other failure exits `1`.

The other commands:

```sh
thirdshift architect [<focus>]      # review the Base branch's architecture, publish a plan and run it (see Architect runs)
thirdshift architect [<focus>] --plan-only   # publish the plan, mark it ready and stop there
thirdshift architect base <branch> [<focus>]   # do either with <branch> as the Base branch, from a clone on any branch
thirdshift pickup                   # take the lowest-numbered Ready issue in the repository and run it (see Pickup runs)
thirdshift pickup base <branch>     # do that with <branch> as the Base branch, from a clone on any branch
thirdshift email-test [<address>]   # send a test email through Resend (see Email)
thirdshift setup                    # choose your defaults and write the User config with every setting (see User config)
thirdshift update                   # update to the latest release (see Updating)
thirdshift version                  # print thirdshift <version>
thirdshift help                     # print every form of the command, each with a one-line description
```

`version` and `help` print to stdout and exit `0`. `update`, `setup` and `email-test` follow the Run's rule: stdout stays empty, messages go to stderr. `architect` follows it too: stdout carries the pull request's URL of the run it dispatches, or, in its place, the URL of the issue it ended on: its plan with `--plan-only`, or its idea when the review published no plan. A skipped Architect run prints nothing there when [another is still running](#one-at-a-time), the URL of each Architect plan that is [still open](#one-architect-plan-at-a-time), the URL of each [Architect idea waiting for triage](#an-architect-idea-waits-for-triage), and the URL of the [Ready issue that goes first](#a-ready-issue-goes-first). `pickup` follows it as well: stdout carries the pull request's URL of the run it dispatches, and nothing when the [Pickup run](#pickup-runs) is skipped.

Uncommitted changes in your clone are fine: the Run works in its own worktree from `origin`, so they are simply left out. Unpushed commits on the Base branch are not: push them first, or the Run stops.

### User config

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
quiet_skips = true   # a skipped Architect run or Pickup run prints nothing

[spec]
parallel = 2   # how many Tickets a Spec run runs at once, instead of 3

[pickup]
limit = 5   # how many open issues labelled in-progress stop a Pickup run taking another, instead of 3

[harness]
default = "claude"   # the Harness every session runs on; claude unless set

[harness.claude]
model = "opus"   # the Model Claude Code's sessions run on; blank for its own default
effort = "high"  # how hard that Model reasons; blank for Claude Code's own default

[harness.codex]
model = ""
effort = ""
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
quiet_skips = false   # a skipped Architect run or Pickup run prints nothing, leaving only its Activity log line; default false

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3

[pickup]
limit = 3   # how many open issues labelled in-progress stop a Pickup run taking another; default 3

[harness]
default = "claude"   # the Harness every Run's sessions run on, claude, codex or muse; default claude

[harness.claude]
model = ""    # the Model Claude Code's sessions run on; default blank, for Claude Code's own
effort = ""   # how hard that Model reasons; default blank, for Claude Code's own

[harness.codex]
model = ""    # the Model Codex's sessions run on; default blank, for Codex's own
effort = ""   # how hard that Model reasons; default blank, for Codex's own

[harness.muse]
model = ""    # the Model Muse Code's sessions run on; default blank, for Muse Code's own
effort = ""   # how hard that Model reasons; default blank, for Muse Code's own
```

Every key holds its real value, so a Run reading it does exactly what it does with no file. `email.to` has no default, so `setup` suggests one: the public email of your GitHub profile (from `gh api user`), else your global git `user.email`, unless that is a `@users.noreply.github.com` address, which can't receive mail. With neither, `email.to` is the only line written commented out, as above. `setup` never asks `gh` for more scopes, so a private GitHub email is not read, and a Run never looks the suggestion up: `--email` with no address and no `email.to` still stops the Run. From a terminal (stdin and stderr both terminals), `setup` first asks, on stderr:

1. The Harness for every Run's sessions (`harness.default`), listing only those installed on `PATH`. An unavailable Harness or unknown name is refused and asked again. The default is the configured Harness when installed, else the first installed, starting with `claude`. Then the Model (`harness.<name>.model`) and the Effort (`harness.<name>.effort`) for the Harness chosen only, each defaulting to the User config's, where Enter on none keeps the Harness's own default and `-` clears one. They're checked as a Run checks them (see [Harness, Model and Effort](#harness-model-and-effort)). For Claude, a named Model gets the test call; if Claude refuses it, Setup shows what Claude said and asks the Model and Effort again. For Codex, Setup lists the Models `codex debug models` offers, by slug and display name, then the Efforts the Model chosen supports; each answer is matched regardless of case and written as Codex names it, so `GPT-6.1-Sol` and `Max` are written as `gpt-6.1-sol` and `max`, and a Model or Effort the catalog doesn't have is asked again, with the valid choices. Muse proposes `muse-spark-1.3` and checks the answers against its cache, or with a minimal call; a refusal asks for the Model and Effort again. The other Harnesses' sections are left as they are. With no Harness installed here, Setup says so, asks none of this, and leaves the harness settings as they are.
2. Every Run a Merge run? (`merge.always`)
3. With that on, every Run may start a Base fix when the Base branch's CI is red? (`base.fix`). With it off, this is not asked, and `base.fix` is written at its default, `false`.
4. Every Run first fast-forwards your checkout of the Base branch? (`launch.pull`)
5. Run notifications? (`email.always`). If yes, the address (`email.to`), asked again until it has an `@`, then the sender (`email.from`).
6. With notifications on, the Resend API key, with input hidden. With none saved it asks `Resend API key (input hidden, Enter to skip):`; with one in the [Credentials](#email), `Resend API key (input hidden, Enter keeps the saved one):`, and a new one replaces it. Surrounding spaces are trimmed, and anything that doesn't start with `re_` is asked again. A key you give is saved in the Credentials, `~/.thirdshift/credentials.toml`, created with mode 600 (and `~/.thirdshift` with it) or edited in place, keeping its comments and anything else in it and changing only `resend.key`; `setup` then prints `wrote the Credentials <path>`. The key is never printed, nor written to the User config. Skipping writes no Credentials and says how to add a key later: rerun `thirdshift setup`, or set `RESEND_API_KEY`. With `RESEND_API_KEY` set and not empty, which wins over the Credentials, nothing is asked, and it says the key comes from `RESEND_API_KEY`. With a key found or given, it offers to send a test email (default No), as `thirdshift email-test` does, once the files are written.

Pressing Enter takes the default shown, which is the file's current value, or else the setting's default, and for the address the suggested email above. `logs.dir`, `activity.quiet_skips`, `spec.parallel` and `pickup.limit` are not asked about. The answers are written like everything else below: in place, keeping your comments. Ctrl-C during the questions, the key included, writes nothing: neither the User config nor the Credentials. With notifications off, nothing about a key is asked, and saved Credentials stay as they were, so `--email` on a single Run still works. Credentials a Run would refuse (see [Email](#email)) are refused before any question, exit `1`, and not touched. With no terminal, as from cron or `thirdshift setup </dev/null`, `setup` asks nothing and never writes the Credentials. Either way it prints the file's path on stderr and exits `0` with stdout empty. Over a User config that is already there, `setup` edits it in place: its comments and key order stay, as do the values it didn't ask about, and each key it lacks is added at its default with its comment, so afterwards the file lists every setting this version knows. One that already does, down to the commented-out `email.to` line, is left byte for byte as it was. A key added to an inline table, such as `launch = { pull = true }`, gets no comment, since TOML has no place for one there. One a Run would refuse is refused the same way, exit `1`, and not touched. Any argument after `setup` is an argument error (exit `2`).

The first Run on a machine with no User config, started from a terminal, offers Setup before any work, on stderr: `No User config at <path>. Set your defaults now? [Y/n]`. Yes (or Enter) asks the questions above, writes the file, and the Run carries on using your answers; a flag in the command, such as `--no-merge`, `--email` or `--no-email`, still wins over them. No writes every setting at its default, as `setup` with no terminal does, asks nothing about a key, writes no Credentials, says that `thirdshift setup` changes it, and the Run carries on; later Runs find the file and don't offer again. If the file can't be written, stderr gets a `warning:` line and the Run carries on with the defaults. Ctrl-C during the offer or the questions writes nothing and ends the command before any work, with no Run notification. A command thirdshift can't parse exits `2` before any offer. A Run with no terminal, as from cron, CI, `nohup` or an agent's shell, offers nothing, writes nothing, and runs on the defaults, so a later Run from a terminal still gets the offer.

With `merge.always = true`, `thirdshift <Issue URL>` is a Merge run, and `thirdshift --no-merge <Issue URL>` (or `no-merge`, before or after the URL) leaves that one Run's pull request ready for review.

With `base.fix = true`, every Run may start a [Base fix](#base-fix), as if given `base-fix`, and `thirdshift --no-base-fix <Issue URL>` (or `no-base-fix`, before or after the URL) forbids it for that one Run.

With `launch.pull = true`, every Run brings the Base branch checked out in the directory you start it from (the **Launch directory**) up to date with `origin`, so you no longer `git pull` by hand before each Run. It happens after the pre-flight checks pass and before the worktree is created, as `git merge --ff-only origin/<Base branch>`: fast-forward only, never a merge commit or a rebase, always from `origin`, whatever the branch's upstream or your `pull.*` settings. A progress line on stderr says when it updates the branch; an already up-to-date branch is left quietly as it is. It is skipped when the checked-out branch isn't the Base branch, as in a Continuation whose open pull request targets another base, or on a detached HEAD. If the update can't happen, for example because uncommitted changes are in the way, stderr gets a `warning:` line with git's error and the command to run by hand, your changes are left as they were, and the Run carries on with the same outcome and exit code. The setting only affects your checkout: the Run's worktree starts from `origin/<Base branch>` either way.

With `email.always = true`, every Run sends a [Run notification](#run-notifications) to `email.to`, as if given `--email`, and `thirdshift --no-email <Issue URL>` (or `no-email`, before or after the URL) sends none for that one Run.

`spec.parallel` sets how many Tickets a [Spec run](#spec-runs) runs at once, by default 3. It must be a whole number from 1 up; `parallel <n>` on the command line wins over it for one Spec run.

`pickup.limit` sets the [Claim limit](#the-claim-limit): how many open issues labelled `in-progress` stop a [Pickup run](#pickup-runs) from taking another, by default 3. It must be a whole number from 1 up. There is no command-line flag for it.

`logs.dir` sets the root of the [logs](#logs), with each repository's in its `<owner>/<repo>/` folder, created if missing. It must be an absolute path, `~` or a path starting with `~/`, where `~` stands for `$HOME`. A relative path stops the Run before any work, since the directory a Run is launched from is no base for a setting that holds for every Run.

`[harness]` chooses the **Harness**, the agent CLI every session of a command runs on, and for each Harness the **Model** its sessions run on and their **Effort**, how hard that Model reasons. See [Harness, Model and Effort](#harness-model-and-effort).

A Run reads the file before any work. One that isn't valid TOML, or that has a key or section thirdshift doesn't know, such as `alway` for `always`, or a value of the wrong type, such as anything but `true` or `false` for `always`, or `0` for `spec.parallel` or `pickup.limit`, stops the Run with an error naming the file and the offending key, so a typo can't silently leave a setting off. `email-test` and `setup` read it the same way. `update`, `version` and `help` never read it, so a broken User config can't block them, and they never offer Setup; nor does `email-test`.

### Harness, Model and Effort

The Harnesses are Claude Code (`claude`), Codex (`codex`) and Muse Code (`muse`).

Every session of a command, its implement session, each Repair and Resume, a Spec review, an Architecture review, each Ticket's Run and a Base fix, runs on one **Harness**, with one **Model** and one **Effort**. `harness <name>`, `model <name>` and `effort <level>` (or `--harness`, `--model` and `--effort`) choose them for one command, before or after the Issue URL, and on `architect` (but not with `--plan-only`) and `pickup` too:

```sh
thirdshift https://github.com/<owner>/<repo>/issues/<n> model claude-opus-5-5 effort high
```

For each of the three, the command wins, then the [User config](#user-config), then the default. The Harness's default is `claude`; the Model's and the Effort's are nothing, which leaves them to the Harness, so with no flag and no `[harness]` section sessions run exactly as `claude -p` would. A Model and Effort set in the User config come from the chosen Harness's own section, so `harness claude` over a `codex` default takes `[harness.claude]`, while `model` and `effort` on the command apply to whichever Harness is chosen. Names are passed to Claude Code as given: `--model` and `--effort` when set. Codex takes them as it names them, which the check below settles: `-m <model>` and `-c model_reasoning_effort="<effort>"` when set.

With `harness codex`, every session runs `codex exec --json --dangerously-bypass-approvals-and-sandbox` in the Run's worktree, with stdin set to null, and with `-c project_doc_fallback_filenames=["CLAUDE.md"]`, so Codex reads `CLAUDE.md` where a repository has no `AGENTS.md`. Codex's sandbox is off: on a stock Ubuntu machine it can't start, and commits and pushes from a worktree write outside it, so the worktree is the only boundary, as with Claude's auto mode ([ADR-0012](docs/adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)). The Factory skills are linked into the worktree's `.agents/skills/`, kept out of git with `/.agents/skills/thirdshift-*` in `.git/info/exclude`, and each prompt's first line loads its skill as `$thirdshift-<skill>`. `codex exec` kills what the agent left running as it exits, so a session whose turn ends with a command or a sub-agent call still running (started, never completed) gets a Resume, as on Claude: `codex exec … resume <session id>`, with the Model, Effort, bypass flag and `CLAUDE.md` fallback given again, as Codex keeps none of them. An interrupt sends a Codex session SIGINT, the only signal Codex stops cleanly on, before the SIGTERM and SIGKILL every session gets, each 10 seconds apart. Progress lines come from Codex's `item.*` events, the session-ended line gives its token totals, as Codex reports no turns or cost, and the final message is the last `agent_message`. A session whose turn fails (`turn.failed`), or that exits non-zero, fails the Run with Codex's error in the cause; an `error` event Codex retries does not. Each Codex session adds a `trust_level = "trusted"` entry for the repository to `~/.codex/config.toml`, and your `~/.claude/` instructions and hooks don't reach it.

Interrupting a Command stops every session's process tree on every Harness, including commands that started their own process group or session. thirdshift snapshots the descendants with `ps` on Linux and macOS, sends each process group the Harness's stop signals with a 10-second grace period each, then takes another snapshot before SIGKILL to catch commands started in the meantime. If the CLI exits on an earlier signal, thirdshift kills the remaining commands immediately without another grace period.

Before any work, where an Origin match failure would stop it, and so before the Claim, the worktree and any Command log, a command checks what it chose: the Harness's CLI must be on `PATH`, else it fails naming the CLI and the setting that chose it; and on Claude a named Model gets a minimal test call, `claude -p --model <model>` with the Effort, if any, which must succeed, else the command fails with what Claude said. No test call is made when no Model is named. On Codex a named Model and Effort are checked against `codex debug models`, which costs no tokens: each matches regardless of case, a Model by its slug or its display name, and becomes the name Codex takes, so `model GPT-6.1-Sol effort Max` reaches Codex as `gpt-6.1-sol` and `max`. An unknown Model, or an Effort that Model doesn't support (with no Model, one no Model supports), fails naming the valid choices. What the command records is what the check settled on. A Pickup run or an Architect run makes these checks only once it has decided not to skip, and a Spec run makes them once for all its Tickets.

With `harness muse`, every session and Resume runs `muse exec --json --yolo`, with stdin null and `MUSE_NO_AUTO_UPDATE=1`. `--yolo` disables approvals and the sandbox and trusts the worktree ([ADR-0013](docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). Muse reads the project's `AGENTS.md`, falling back to `CLAUDE.md`, and also reads `~/.claude/CLAUDE.md`. The Factory skills are linked into `.agents/skills/` and kept out of git. The prompt's first line tells the model to load its `thirdshift-<skill>` with its skill tool. If the stream shows no `read_skill` load of that skill, thirdshift writes a warning to the progress lines and Command log; it neither fails nor retries the session.

Muse's named Model is checked against its cached catalog under `~/.local/share/muse/model-catalog/` when present, matching its id or display label regardless of case. Without that cache, thirdshift makes a minimal test call with the same flags and environment. Effort is checked locally before any call: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultra`. Model and Effort reach Muse as `--model` and `--reasoning-effort`. Setup proposes `muse-spark-1.3`; leaving the Model unset uses Muse's own default, `muse-spark-1.3-contributor`, which allows content to be used for product improvement.

Muse's session id comes from `stream.id`, and a Resume uses `--session-id <id>` with every flag and environment override again. Progress lines name session starts, tools and errors. After exit, thirdshift reads the last reply and token totals from Muse's own `sessions/<year>/<month>/<day>/<id>/session.jsonl`. Architecture reviews see that last reply alone, rather than Muse's terminal event's joined replies. A missing or unreadable log falls back to the stream text with no usage. `run.terminal.failed` or a non-zero exit fails the Run with Muse's error. An interrupt sends SIGTERM, then SIGKILL after ten seconds if needed.

The Command log's opening lines (`sessions run on claude · claude-opus-5-5 · high`), the Activity log's start line, the pull request's body and the Run notification each name the Harness, Model and Effort, `default model` or `default effort` where the Harness's own is used. In the pull request's body, the Spec PR's included, thirdshift writes the line itself once the opening session has ended, such as `Built with claude · claude-opus-5-5 · high`, replacing any it wrote there before.

### Email

thirdshift sends email itself, with one HTTPS request to [Resend](https://resend.com)'s API, so it needs no mail server on the machine and works where SMTP ports are blocked ([ADR 0005](docs/adr/0005-run-notifications-through-resend.md)). It needs a Resend account and an API key:

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

The Credentials are read only when `RESEND_API_KEY` is unset or empty, and only by `email-test`, `setup`, and a Run, an Architect run or a Pickup run that asks for a notification. A missing file just means no key from it. One that isn't valid TOML, or holds anything but a quoted `resend.key`, such as a typo like `kye`, stops the command with exit `1`, naming the file and the offending key, and nothing is sent. One that others can read is still used, with a `warning:` line on stderr saying to `chmod 600` it. Nothing is sent to check the key itself. When Resend accepts the email, it prints `accepted by Resend; check your inbox` and exits `0`; that is all it can verify, so check that the email arrives. When Resend refuses it, for example for a bad key or a sender it won't send from, it prints Resend's error text word for word, with where the key came from, and exits `1`. It gives up after 30 seconds without an answer.

#### Run notifications

`--email` (or `email`) asks a Run for a **Run notification**: one email, sent when the Run ends, whatever the outcome. The word after the flag is the address only if it contains `@` and doesn't start with `https://`, so the Issue URL is never taken for it; otherwise the email goes to `email.to`. With `email.always = true` in the [User config](#user-config), a Run asks for one without the flag, and `--no-email` (or `no-email`) skips it for that Run; an address after `--email` still wins over `email.to`. Giving a flag twice, or `--email` together with `--no-email`, is an argument error.

A Run that asks for a notification, by the flag or by `email.always`, makes the same checks as `email-test` before any other work: an address is known, and a key is found, in `RESEND_API_KEY` or else the Credentials. If either fails, or the Credentials are broken, the Run stops, exits `1` naming what's wrong (with no key, the message above listing every way to give one), and sends nothing. A Run that asks for no notification never reads the Credentials, so a broken file can't stop it, and `update`, `version` and `help` never read it either. Once they pass, every way the Run ends sends exactly one notification, after its outcome is final and its cleanup done: ready for review, merged, a [Failed run](#failed-runs) (including a later preflight failure such as an origin mismatch), or interrupted by Ctrl-C, SIGTERM or a closed terminal.

- **Subject**: `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, where the outcome is `ready for review`, `merged`, `failed` or `interrupted`. The title is left out if it can't be read from GitHub.
- **Body**, plain text: the pull request URL (if any), the failure cause (if failed), after an Inherited failure with no Base fix taken its [`Base check:`, `Retry with:` and `Or set:` lines](#an-inherited-failure-links-the-base-branchs-checks), the session log path (if any), the [Command log](#logs) path as a `Command log:` line, the hostname and how long the Run took.

A [Spec run](#spec-runs) sends at most one notification for the whole Spec, under the same rules, with its checks made once before any Ticket starts. Its subject names the Spec, its outcome is the Spec run's, and its body, after what a Run's holds (the Spec PR, if any), lists each Ticket's outcome, one line per Ticket as in the summary on stderr, such as `#21 landed with https://github.com/acme/widgets/pull/1` or `#22 blocked by #21`. The Ticket Runs inside it never send a notification of their own, whatever the User config says.

An [Architect run](#architect-runs) takes the same flags and the same `email.always`, and sends [one notification](#one-run-notification) covering its review and the run it dispatched.

A [Pickup run](#pickup-runs) takes the same flags and the same `email.always` too. One that took an issue sends [one notification](#one-run-notification-for-the-issue-taken), the one the run it dispatched would have sent by hand, and a skipped one sends none.

A notification that can't be sent is a `warning:` line on stderr with Resend's error. It never changes the Run's outcome, stdout or exit code.

### Logs

Everything thirdshift logs goes under `~/.thirdshift/logs/`, or the `logs.dir` set in the [User config](#user-config), in a folder per repository, `<owner>/<repo>/`, named for the GitHub repository rather than your checkout, so a fork and its upstream never share one and every checkout or worktree of a repository logs to the same place. thirdshift creates each folder as it needs it, so a repository's first pass needs nothing made by hand:

```
~/.thirdshift/logs/<owner>/<repo>/
├── activity.log              the Activity log
├── sessions/                 Session logs
└── commands/
    ├── architect/            <stamp>.log
    ├── pickup/               <n>-<stamp>.log
    └── issue/                <n>-<stamp>.log
```

The folder already names the repository, so no file name repeats it. A **Session log** is one session's full transcript, as Claude Code's `stream-json` output:

```
~/.thirdshift/logs/<owner>/<repo>/sessions/<n>-<stamp>-implement.jsonl
~/.thirdshift/logs/<owner>/<repo>/sessions/<n>-<stamp>-repair-<i>.jsonl
~/.thirdshift/logs/<owner>/<repo>/sessions/architect-<stamp>-architecture-review.jsonl
```

A Resume is logged as its session's kind plus `-resume`, e.g. `implement-resume.jsonl`.

A **Command log** is everything one command printed, stderr and stdout in the order printed, while the terminal still shows all of it. Its folder is the command typed: `thirdshift <Issue URL>`, a Run or a Spec run, in `issue/`, `thirdshift pickup` in `pickup/`, named for the issue it took, and `thirdshift architect` in `architect/`. A Spec run's covers its Tickets' Runs, a Run's covers its [Base fix](#base-fix), and a Pickup run's or an Architect run's covers the Spec run or Run it dispatched: none of those keeps one of its own. So `ls -t ~/.thirdshift/logs/acme/widgets/commands/pickup | head` lists the recent passes on acme/widgets that took an issue. A command keeps its Command log once it starts work: a Pickup run or an Architect run skipped before doing any work keeps none, nor does a Run that fails before it starts work, as on the [Origin match](#what-a-run-does), and `setup`, `email-test`, `update`, `version` and `help` never keep one. Lines printed before the Command log's name is known, such as a Pickup run's lines on the issues it [passed over](#why-an-issue-was-passed-over) before it took one, are written first, and once it is created a progress line says `logging this command to <path>`. A Command log that can't be written, as when its folder can't be created or the disk is full, is one `warning:` line on stderr, and the command carries on with the same outcome, stdout and exit code.

The stamp is the local time the command started, with its UTC offset, as in `20261003T120000-0400`, in the machine's time zone, or `TZ`'s if set, so it agrees with `date` and `ls -l`. A command's Command log and all of its Session logs, its Tickets' Runs' and its Base fix's included, share that one stamp, so they sort together and each can be found from the other. When a Run fails, stderr ends with the path of its most recent Session log, the place to start looking, then that of its Command log.

The **Activity log**, `activity.log`, is a short running record of what the factory did on the repository ([ADR-0011](docs/adr/0011-thirdshift-keeps-a-per-repository-activity-log.md)). A Run, a Spec run, an Architect run or a Pickup run writes a line when it starts work, naming its Command log, and one when it ends, with its outcome; a Run you type by hand writes them too, and the run an Architect run or a Pickup run dispatched, a Spec run's Tickets and a Base fix write none of their own. A skipped Architect run or Pickup run writes a line only when its reason differs from the last line of its own kind, so a repository that sits idle shows one line, not one per pass, and the first skip after a pass that did work always shows: that is when the repository went idle. Every line starts with the local date and time:

```
2026-10-03 02:00:01 Pickup run skipped: no Ready issue on acme/widgets
2026-10-03 09:30:01 Pickup run #41 started: commands/pickup/41-20261003T093001-0400.log
2026-10-03 09:52:17 Pickup run #41 ended: PR https://github.com/acme/widgets/pull/42 is merged
2026-10-03 10:00:01 Pickup run skipped: no Ready issue on acme/widgets
```

Each line is appended whole, so passes that run at once on one repository never garble it. One that can't be written is one `warning:` line on stderr, and the command carries on as for a Command log. thirdshift never rotates it: collapsed skips keep it small. Logs written before this layout, under `sessions/` and `commands/` at the root of the logs, are not moved.

With `quiet_skips = true` in the `[activity]` section of the [User config](#user-config), a skipped Architect run or Pickup run prints nothing on stdout or stderr, its dated first line included, and leaves only its Activity log line. A pass that does work, or fails, prints as ever, so a scheduler's log file catches only what went wrong. Without it, a skipped pass prints as it always has, for a pass you type by hand.

## Foreign commits in a Merge run

A Merge run merges only code an agent wrote or reviewed. If someone else pushes to the Issue branch during the Run, their **Foreign commits** are reviewed before they can be merged:

1. Each round of step 6 starts by fetching the Issue branch from `origin`. Any new commits there are merged into the Run's branch, by fast-forward or a merge commit, never a rebase, each logged on stderr as `merging new commit <sha> from origin/<branch>`. Foreign commits that arrive while CI runs on a green head send the Run round again rather than on to the merge. A merge that conflicts with the Run's own work gets a conflict Repair.
2. A **review Repair** then runs `/thirdshift-code-review` with the head thirdshift last knew as the Run's own as the fixed point. The agent fixes the findings it agrees with, adds the rest to the pull request body's "Unaddressed findings" section marked as coming from the Foreign commits, and pushes.
3. The round goes on as usual: the Base branch is merged in, CI watched, and the merge tried once the head is green.

Review Repairs are logged as `repair-<i>`, numbered with the other Repairs, count against the cap of 5 Repairs, and get a Resume like any session. A round that picks up Foreign commits counts against the same 5, like a Base branch move. A Run that spends either budget fails.

A Run without `merge` does none of this: it never fetches someone else's commits into its branch.

## Base fix

A Base branch whose CI is broken fails every Run on it with an Inherited failure. To have a Run fix the Base branch itself and carry on, give it `base-fix` (or `--base-fix`), before or after the URL:

```sh
thirdshift base-fix https://github.com/acme/widgets/issues/7
```

To allow it for every Run on a machine, set `base.fix` in the [User config](#user-config); `no-base-fix` (or `--no-base-fix`), before or after the URL, then forbids it for one Run. Giving either flag twice, or `base-fix` together with `no-base-fix`, is an argument error.

When the Run's only red checks are Inherited failures and the Base branch has not moved since, a Run allowed to then starts a **Base fix**, at most one per Run ([ADR-0008](docs/adr/0008-inherited-failures-fail-the-run.md)):

1. thirdshift looks for an open issue labelled `base-fix`, other than the Run's own, whose title names the same Base branch and every one of those checks. If there is one, another Run's Base fix is already under way, and the Run waits on that one instead: see [One Base fix per Base branch](#one-base-fix-per-base-branch).
2. Otherwise, thirdshift opens an issue from a fixed template, with no agent session: titled `CI red on <base>: <check>[, <check>…]`, its body naming the checks with their URLs on the Base branch, the Base branch and its short sha, and the Run's pull request, labelled `base-fix` and `ready-for-agent`. A label the repository lacks is added to it first.
3. It starts a child `thirdshift` on that issue from the same Launch directory, as a Merge run into the Run's Base branch, whatever the Run's own goal. Its progress lines are relayed with a `#<n>: ` prefix, after `starting Base fix #<n> into <base>: <issue URL>` and `waiting on Base fix #<n>`. The Base fix sends no Run notification, makes no [Claim](#the-claim) on its issue, treats every red check as its own to fix rather than as an Inherited failure, and never starts a Base fix of its own.
4. Once the Base fix has merged, and its Self-merge has closed its issue, the Run merges the Base branch in again and watches CI, and goes on as usual: ready for review, or merged in a Merge run.

If the Base fix fails, the Run is a [Failed run](#failed-runs) with the cause `Base fix <issue URL> failed: <its cause>`. If the checks are still Inherited failures once the Base fix has merged, there is no second one, and the cause is `CI red on <check>, which also fails on <base> at <short sha>, even after Base fix <issue URL> merged; fix <base> first`. The Run's last progress lines include `Base fix: <issue URL> merged`, or `not merged`, and its own [Run notification](#run-notifications) has the same `Base fix:` line.

On a Spec, `base-fix`, `no-base-fix` or the User config's `base.fix` holds for each Ticket's Run, whose Base fix goes into the Spec branch, its Base branch, and for the Spec PR, whose Base fix goes into the Base branch. Tickets running at once that meet the same Inherited failure share one Base fix: the first starts it, and the others wait on it. A Ticket's Run sends no Run notification, so the Spec run reports its Base fix: a Ticket that landed after one has `#<n> landed with <PR URL>, after Base fix <issue URL> merged`, or `closed` for one it waited on, as its line in the Tickets checklist and in the Spec run's Run notification, and one whose Base fix failed has the cause naming the issue.

### One Base fix per Base branch

A Run that finds an open Base fix issue for its Base branch and checks starts none of its own. After `waiting on Base fix #<n>, already open: <issue URL>`, it waits for that issue to close, as the Base fix's Self-merge leaves it, then merges the Base branch in again and watches CI. Waiting is the Run's one Base fix: if the checks are still Inherited failures once the issue has closed, the cause is `CI red on <check>, which also fails on <base> at <short sha>, even after Base fix <issue URL> closed; fix <base> first`, and its Run notification's `Base fix:` line says `closed`, or `not closed` if the Run ended first.

An issue that covers more checks than the Run's counts; one for another Base branch, or that leaves out one of the Run's checks, does not. The wait has no time limit: a Base fix issue from another clone or machine that nobody is working on has to be closed, or the Run interrupted, by hand.

Runs from one Launch directory, as a Spec run's Tickets are, look for the issue and write it one at a time, under a lock in the clone's git directory, so those that meet the same Inherited failure at once get one Base fix issue and one Base fix. There, a Run also knows whether a Base fix started from that Launch directory is still running:

- A Run waiting on one that fails, leaving its issue open, fails too, with the cause `Base fix <issue URL> ended with its issue still open`.
- A Run that finds the issue open after that Base fix has ended starts it again on the same issue, as its one Base fix, after `Base fix #<n> is open but no longer running; starting it again into <base>: <issue URL>`. It continues the Base fix's Issue branch, as any Run on an issue with one does: see [Continuation](#continuation).

Across clones and machines the look is only a best-effort lock, and two Runs that look at the same moment may each start a Base fix.

Without `base-fix` or `base.fix`, or with `no-base-fix`, an Inherited failure fails the Run as described in [What a Run does](#what-a-run-does), and the Run says [where the checks fail on the Base branch](#an-inherited-failure-links-the-base-branchs-checks), offering a Base fix if nobody decided against one.

## Continuation

Running thirdshift again on an issue picks up where the last Run stopped ([ADR-0002](docs/adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed run with the same command, or start an issue on one server and continue it on another.

A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit.

## Failed runs

A **Failed run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI, or, for a Merge run, without that pull request merged. Causes include the session exiting non-zero, no pull request or one with the wrong base, running out of Repairs, CI red only on **Inherited failures** (`CI red on <check>[, <check>…], which also fails on <base> at <short sha>; fix <base> first`), a CI-fix Repair that finds nothing on the branch to fix (a **declined CI fix**: the head is unchanged after it, and its one Check re-run left CI red or could not happen), and a Base branch that keeps moving while CI runs, or in a Merge run, an Issue branch that keeps getting Foreign commits.

A Failed run:

1. Commits any uncommitted work as `thirdshift: failed run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again. The exception is a Merge run's policy refusal: the pull request is ready, mergeable and green and only the Self-merge could not happen, so it stays ready for review, and no failure commit is pushed onto the head whose CI was watched.
3. Cleans up as usual, prints the reason to stderr and exits non-zero. If the push failed, the worktree and local Issue branch are kept instead, and stderr names the branch, its head commit and the worktree path, so you can recover the work or push it by hand.
4. Releases its [Claim](#when-the-claim-ends) if it left nothing on `origin`: no Issue branch and no pull request. Otherwise the issue stays `in-progress`.

Merges, never rebases or force-pushes: a branch worked on from several servers never loses history.

### An Inherited failure links the Base branch's checks

A Run that fails on Inherited failures with no Base fix taken says more after its cause, on stderr and in its [Run notification](#run-notifications): a `Base check:` line for each of those checks, with its URL on the Base branch commit, as a [Base fix](#base-fix) issue lists them, or the name alone for a check with no URL. If nobody decided against a Base fix, with neither `base-fix` nor `no-base-fix` given and no `base.fix = true` in the [User config](#user-config), it also offers one: a `Retry with:` line with the command that starts the Run again with `base-fix` added, keeping the `merge`, `email` and `parallel` flags it was given, and an `Or set:` line naming `base.fix`:

```
thirdshift: 03:12:40 CI red on test, which also fails on main at 362b9ca; fix main first
thirdshift: 03:12:40 Base check: test: https://github.com/acme/widgets/actions/runs/1/job/2
thirdshift: 03:12:40 Retry with: thirdshift https://github.com/acme/widgets/issues/7 base-fix
thirdshift: 03:12:40 Or set: base.fix = true in ~/.thirdshift/config.toml, to allow a Base fix for every Run on this machine
thirdshift: 03:12:40 session log: ~/.thirdshift/logs/acme/widgets/sessions/7-….jsonl
thirdshift: 03:12:40 command log: ~/.thirdshift/logs/acme/widgets/commands/issue/7-….log
```

A Run given `no-base-fix` gets the `Base check:` lines and no offer. A Run that had its one Base fix gets neither: its `Base fix:` line already says what happened. The cause itself, and so the failure commit's message, is the same in every case, and no other cause adds a line.

In a [Spec run](#spec-runs), a Ticket's Run says this on its own stderr, relayed with its `#<n>: ` prefix, and the command it offers is the Spec run's, with `base-fix` added. The Spec run's summary and its Run notification show the Ticket's cause alone. After an [Architect run](#architect-runs), the command offered is `thirdshift <plan URL>` with the dispatched run's flags and `base-fix`, since another Architect run would start a new review rather than take that plan up again.

## Spec runs

A **Spec** is an issue with sub-issues, its **Tickets**. `thirdshift <Issue URL>` on a Spec is a **Spec run**: it works through the Tickets in the order their GitHub "blocked by" links allow, each Ticket's **Run** a **Merge run** into the **Spec branch**, then leaves one **Spec PR** from the Spec branch into the Base branch ready for review ([ADR-0006](docs/adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md)). An issue with no sub-issues is an ordinary Run.

A Spec run makes the [Claim](#the-claim) on the Spec, as a Run does on its issue: the Spec is labelled `in-progress`, in place of `ready-for-agent`. Its Tickets' Runs change no label on their Tickets. The Claim [ends](#when-the-claim-ends) as a Run's does: it is released if the Spec run fails with no Spec branch on `origin` and no Spec PR, removed once a Self-merge of the Spec PR has left the Spec closed, and kept otherwise. The Spec branch is pushed before the first Ticket starts, so a Spec run that got as far as its Tickets keeps its Claim.

The Spec PR opens as a draft as soon as the first Ticket lands, titled from the Spec, with `Closes #<spec>` and a Tickets checklist: one line per Ticket, ticked once it is done, with its pull request, or saying it is running, failed, blocked or unready. thirdshift rewrites the checklist between its `<!-- thirdshift:tickets -->` markers as each Ticket starts and ends, leaving the rest of the body as it is. Once every Ticket is done, the **Spec review** rewrites the body, and thirdshift puts the checklist back, appending it if the markers are gone, before marking the Spec PR ready.

The Spec review session reviews the whole Spec branch against the Base branch and the Spec. Once it is marked ready, the Spec PR goes through the same step 6 as a Run's pull request, with the Spec as the issue and the Spec branch as the Issue branch: the Base branch is merged in (never rebased), CI watched, and a conflict or red CI handed to a Repair, within the same budgets, with the same Check re-run after a CI-fix Repair that leaves the head unchanged. Inherited failures are told apart the same way everywhere: the Spec PR's against the Base branch, and a Ticket's against the Spec branch, its Base branch, with the cause shown on the Ticket's line of the checklist.

Tickets always merge into the Spec branch, whatever the command or the User config says. `merge` (or `--merge`, or `merge.always = true` without `no-merge`) applies to the Spec PR alone: the Spec run ends with the Self-merge of the Spec PR into the Base branch, then deletes the Spec branch on `origin` and closes the Spec if the merge did not. Without it, the Spec run ends with the Spec PR ready for review. A Spec PR that fails from the Spec review on follows the [Failed run](#failed-runs) rules for its pull request, back to draft unless only the Self-merge could not happen (a policy refusal), and the Spec run exits `1`.

A Spec run takes every Ticket it can reach. A Ticket runs once it is open, has every blocker closed, and is not an **Unready Ticket**: an open Ticket labelled `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. An open Ticket with no triage label is taken. A Ticket with sub-issues of its own is never run either, and is reported as unready. A blocker outside the Spec counts once it is closed. The graph is read again from GitHub whenever a Ticket's Run ends, so removing a label, adding a Ticket or closing one by hand takes effect in the same Spec run.

A Ticket whose Run fails is not tried again in that Spec run, and stops only the Tickets it blocks; every other Ticket it can reach still runs. Nor do Unready Tickets, Tickets blocked by an open issue outside the Spec, or Tickets in a cycle of "blocked by" links run, nor any Ticket downstream of them.

Independent Tickets run at once, up to 3 by default. Whenever a Ticket's Run ends, the Spec run reads the graph from GitHub again and starts ready Tickets until the limit is reached. To change the limit for one Spec run, add `parallel <n>` (or `--parallel <n>`) before or after the URL; `spec.parallel` in the [User config](#user-config) sets it for every Spec run on the machine:

```sh
thirdshift parallel 5 https://github.com/acme/widgets/issues/20   # up to 5 Tickets at once
thirdshift --parallel 1 https://github.com/acme/widgets/issues/20 # one at a time
```

`parallel` followed by `0`, a negative number or anything but a whole number, or given twice, is an argument error (exit `2`). `parallel` on an issue with no sub-issues stops the Run before any work, since there are no Tickets to run at once. The Tickets' Runs share the Launch directory: they create their worktrees there one at a time, and a git command that finds a lock file held by another waits and tries again.

When nothing is left to run and any Ticket is not done, the Spec run is a **Failed spec run**: it leaves the Spec PR a draft, its checklist showing what's missing, prints its URL on stdout (if any Ticket has landed, so there is one), exits `1`, and lists on stderr each Ticket that landed, with its pull request, and each one not done, with why:

```
thirdshift: 03:12:40 #21 failed: claude exited 1 (session log: ~/.thirdshift/logs/acme/widgets/sessions/21-….jsonl)
thirdshift: 03:12:40 #22 blocked by #21
thirdshift: 03:12:40 #23 landed with https://github.com/acme/widgets/pull/1
thirdshift: 03:12:40 #24 unready: labelled needs-info
thirdshift: 03:12:40 #25 blocked by #99 (outside the Spec)
thirdshift: 03:12:40 #26 in a cycle: #26 blocked by #27 blocked by #26
thirdshift: 03:12:40 #27 in a cycle: #27 blocked by #26 blocked by #27
thirdshift: 03:12:40 Tickets not done: #21, #22, #24, #25, #26, #27
```

Running the Spec again picks up where the last Spec run stopped. The Spec branch is on `origin`, so the Spec run continues it ([ADR-0002](docs/adr/0002-existing-issue-branch-means-continue.md)) and updates its draft Spec PR rather than opening another. Closed Tickets are done and not run again, and a Ticket that failed still has its Issue branch and draft pull request, so its Run is a [Continuation](#continuation) into the Spec branch. With every Ticket closed, the Spec run goes straight to the Spec review and the Spec PR. With every Ticket closed and no Spec branch, as when the Spec was done some other way, it stops with `every Ticket is closed and there is no Spec branch; nothing to do` and exits `1`, creating no branch or pull request and leaving the Spec open.

## Architect runs

`thirdshift architect` starts an **Architect run**: the factory looks for architecture work itself, with no **Issue URL**, publishes a plan for the top opportunity, and implements it. Start it from a clone of the repository, on the **Base branch**, or name the Base branch with `base <branch>`:

```sh
cd ~/repos/widgets
thirdshift architect                              # review the whole codebase, then run the plan
thirdshift architect "the Spec run"               # point the review at an area
thirdshift architect merge parallel 2             # merge the plan's pull request, two Tickets at once
thirdshift architect base-fix                     # the run the plan is dispatched as may start a Base fix
thirdshift architect "the Spec run" --plan-only   # publish the plan, mark it ready and stop
thirdshift architect base main                    # review main and run the plan on it, whatever branch the clone is on
thirdshift architect --email you@example.com      # email how the Architect run ended
```

The focus is optional free text, given as one argument, anywhere among the flags; it goes into the Session prompt. `architect` is a command only as the first argument.

`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel <n>`, `harness <name>`, `model <name>` and `effort <level>` (or the same with dashes) are for the run the plan is dispatched as, and mean what they do for `thirdshift <Issue URL>`, so none of them is ever read as the focus. The [Harness, Model and Effort](#harness-model-and-effort) are the Architecture review's too. `--plan-only` stops the Architect run once the plan is marked ready, and dispatches nothing. `email`, optionally followed by an address, and `no-email` (or `--email` and `--no-email`) are for the Architect run's own [Run notification](#one-run-notification), with or without `--plan-only`. The word after `email` is the address only if it contains `@`, so a focus without one is never taken for it.

`base <branch>` (or `--base <branch>`) names the Architect run's **Base branch**, so the branch your clone has checked out stops mattering: start it from a clone on another branch, on a detached HEAD, or with uncommitted changes. The Architecture review's worktree starts at the head of `<branch>` on `origin`, and the run the plan is dispatched as takes `<branch>` as its Base branch too: a Run's **Issue branch**, or a Spec run's **Spec branch**, is branched off it, and the pull request or **Spec PR** targets it. It goes before or after the focus and the other flags, `--plan-only` included, and the word after it is always the branch, never the focus. Without `base`, the Base branch is the branch checked out. `base` is a flag of `architect` and [`pickup`](#pickup-runs) only: `thirdshift <Issue URL>` doesn't take it. It is what lets an Architect run be started [on a schedule](#on-a-schedule).

A second focus, a repeated flag, `merge` with `no-merge`, `base-fix` with `no-base-fix`, `email` with `no-email`, `parallel` without a whole number from 1 up, `base` with no branch after it, any other argument starting with a dash, or an empty focus is an argument error (exit `2`). So is `merge`, `no-merge`, `base-fix`, `no-base-fix` or `parallel` with `--plan-only`, since nothing is dispatched for them to apply to.

An Architect run:

1. Makes the checks a Run makes that don't need an issue, before creating anything: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached, and the Base branch exists on `origin` with your local copy not ahead of it. With `base <branch>`, a detached HEAD is fine, since the checkout no longer picks the Base branch, and the other two checks are made on `<branch>`: `base branch <branch> does not exist on origin; push it first`, or `local <branch> is <n> commit(s) ahead of origin/<branch>; push them first`. There is no **Origin match**, since there is no Issue URL: the repository is the one `origin` names. If another Architect run on the repository, or a [Pickup run](#pickup-runs), is still running, it is [skipped](#one-at-a-time) here, and if not, it is skipped when an earlier [Architect plan is still open](#one-architect-plan-at-a-time), and if not, when an [Architect idea waits for triage](#an-architect-idea-waits-for-triage), and if not, when the repository has a [Ready issue](#a-ready-issue-goes-first). With `launch.pull = true` in the [User config](#user-config), it then fast-forwards your checkout of the Base branch, as a Run does, and so only when the Base branch is the branch checked out: with `base <branch>` from a clone on another branch, your checkout is left alone.
2. Creates a git worktree next to your clone, named `<repo>-architect`, detached at the head of the Base branch on `origin`, with no **Issue branch**. Your checkout, its uncommitted changes and its untracked files are never touched or scanned.
3. Runs the **Architecture review** there: a headless Claude Code session with the **Factory skills** loaded, started with the [Architecture review prompt](prompts/architecture-review.md). It looks for deepening opportunities, skips any an open issue already covers, and takes the top recommendation. If it is Strong, the review publishes it as the plan, labelled `needs-triage`: a **Spec** with **Tickets**, or a single Ticket when one session is enough. It may edit files in the worktree to check an idea, but commits and pushes nothing. It ends its final message with one line naming the plan: `Architecture review plan: <Issue URL>`. thirdshift reads only that line. With [no Strong candidate](#no-strong-candidate) the line names another issue, which thirdshift labels an **Architect idea**, and steps 5 to 7 are skipped.
4. Removes the worktree once the session ends, whatever the outcome. The temporary directory the Factory skills were written out to goes as the Architect run ends.
5. Checks the plan: the issue is in this repository, is open, was created after the Architect run started, and carries no other label that says it is not agent work (`ready-for-human`, `needs-info` or `wontfix`).
6. Marks the plan ready, in one request: `needs-triage` is swapped for `ready-for-agent`, the plan is labelled `architect-plan`, the mark a later Architect run finds an open **Architect plan** by, and its other labels are kept. thirdshift adds that label, never the agent, and first creates it if the repository has none. A Spec's Tickets are left as the review labelled them, without `architect-plan`.
7. Dispatches the plan, unless given `--plan-only`, exactly as `thirdshift <plan URL>` would from the same clone on the Architect run's Base branch: a [Spec run](#spec-runs) when the plan has sub-issues, a Run otherwise. Its Base branch is the Architect run's, so with `base <branch>` it is `<branch>`, not the branch checked out. `merge` and `no-merge` apply to the Spec PR or the Run's pull request, `parallel <n>` to the Spec run, and `base-fix` and `no-base-fix` to whether that run may start a [Base fix](#base-fix), as if given to that command, and the [User config](#user-config) sets what they leave unsaid: `merge.always`, `spec.parallel`, `base.fix`, `launch.pull` and `logs.dir`. The Architecture review itself never watches CI, so a Base fix can only happen in the dispatched run. It makes the [Claim](#the-claim) as that command would too: once its checks pass, the plan's `ready-for-agent` is swapped for `in-progress`, and the plan keeps `architect-plan`. The one difference is that it sends no [Run notification](#run-notifications) of its own, whatever `email.always` says: the Architect run sends [the one](#one-run-notification).

The dispatched run's ending is the Architect run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Architect run fails as that Failed run or Failed spec run does, with the cause, the session log and the Command log on stderr and the pull request's URL on stdout if it left one. The plan stays open and `architect-plan`, and `in-progress` unless the run left nothing on `origin` and so [released its Claim](#when-the-claim-ends), for `thirdshift <plan URL>` to take up again: no later Architect run [retries it](#one-architect-plan-at-a-time). `parallel <n>` on a plan that is a single Ticket fails the same way as it does for `thirdshift <Issue URL>` on an issue that isn't a Spec: `parallel is only for a Spec, and #<n> has no sub-issues`, exit `1`, before any implementing and before the Claim, with the plan left `ready-for-agent`, to run without it.

With `--plan-only`, it instead exits `0` with the plan's URL alone on stdout, and `plan <url> is ready for an agent` as stderr's last line. Read or edit the plan, then run it with `thirdshift <Issue URL>`. The plan is labelled `architect-plan` here too, so until it is finished or closed the next Architect run is [skipped](#one-architect-plan-at-a-time).

Progress lines on stderr say when the review starts, which plan it reported, when the labels are swapped, and when the plan is dispatched: `dispatching the plan <url>, as thirdshift <url> would`. The dispatched run's own progress lines follow. With no Strong candidate, the last line says which issue the Architect run ended on instead.

A review session that fails or is interrupted, a final message without one of the lines the prompt asks for, or a plan that fails a check ends the Architect run as a failure: exit `1`, nothing on stdout, the cause on stderr and then the paths of the session log and the Command log. Nothing is dispatched and no label is changed, so a plan the review did publish stays `needs-triage`, without `architect-plan`, for you to finish or close, and doesn't skip the next Architect run. A review that finds no deepening opportunity at all has no issue to name, so it ends without one of those lines and the Architect run fails this way too; its session log says what it looked at.

### One Run notification

An Architect run asked for a [Run notification](#run-notifications), by `email` or by `email.always = true` in the [User config](#user-config) without `no-email`, sends exactly one, whatever its outcome, with or without `--plan-only`, unless it is skipped. A skipped Architect run, [however](#one-at-a-time) [it](#one-architect-plan-at-a-time) [is](#an-architect-idea-waits-for-triage) [skipped](#a-ready-issue-goes-first), sends none, even when asked, so a schedule can start one every few minutes without flooding your inbox: its line is in the log its scheduler keeps. It makes a Run's checks before any other work all the same, an address and a Resend API key, and stops with exit `1` if either is missing, so a run that would have been skipped fails on them too. The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout.

- **Subject**: `[thirdshift] <owner>/<repo> Architect run: <outcome>`. With a dispatched run, the outcome is that run's: `ready for review`, `merged`, `failed` or `interrupted`. Without one, it is the review's: `plan published` (with `--plan-only`), `idea filed`, `idea already filed`, `review failed` (also for a plan that fails a check) or `interrupted`. The repository is left out if `origin` doesn't name one on GitHub.
- **Body**, plain text: a `Review:` line saying how the Architecture review ended, with the URL of the plan or idea issue it named (`plan published: <url>`, `idea filed: <url>`, `idea already filed: <url>`, `failed` or `interrupted`); when the plan was dispatched, a `Dispatched:` line with that run's outcome; then what a Run's notification holds, for the dispatched run or else the failed review: the pull request URL (if any), the failure cause (if failed), with the [lines after it](#an-inherited-failure-links-the-base-branchs-checks) of a dispatched run that failed on an Inherited failure, a `Base fix:` line for a dispatched run that started or waited on a [Base fix](#base-fix), the session log path (if any), the `Command log:` path (if any), the hostname and how long the Architect run took. After a dispatched Spec run, it ends with a line per Ticket, as a Spec run's notification does.

### No Strong candidate

Only a Strong top recommendation becomes a plan. When the review's top recommendation is Worth exploring or Speculative, it publishes no plan, and the Architect run ends in one of two ways, both a success, with or without `--plan-only`: exit `0`, with one issue's URL alone on stdout. thirdshift labels that issue an **Architect idea** and dispatches nothing: there is nothing to run.

- **An idea issue.** The review files its top recommendation as one issue labelled `needs-triage`, for the **Day shift** to flesh out, and ends its final message with `Architecture review idea: <Issue URL>`. stdout carries the idea issue's URL, and stderr's last line is `no Strong candidate: the Architecture review filed the idea <url>`.
- **Already filed.** An open issue already covers that recommendation, so the review files nothing and ends its final message with `Architecture review already filed: <Issue URL>`. stdout carries that issue's URL, and stderr's last line is `no Strong candidate: <url> already covers the Architecture review's top recommendation, so it filed nothing`.

Either way, thirdshift then puts `architect-idea` and `needs-triage` on that issue, in one request that keeps its other labels, with a progress line saying so: `labelling #<n> an Architect idea: adding needs-triage and architect-idea`. thirdshift adds `architect-idea`, never the agent, and first creates it if the repository has none, as it does `architect-plan`. `needs-triage` goes back on an issue that already covered the idea even if it had been triaged, `ready-for-human` say, since the factory again takes it for the best next move. If thirdshift can't label the issue, the Architect run fails as a failed review does, with the cause on stderr, `could not label the Architect idea #<n>: <why>`, and in its [Run notification](#one-run-notification).

### An Architect idea waits for triage

An Architect idea means the factory has run out of Strong ideas, so while any Architect idea is open and still labelled `needs-triage`, every later Architect run on the repository is skipped, before any review, instead of filing the next weaker idea or landing on the same issue again. Both labels count in any case, as every label rule does. The check comes after the [lock](#one-at-a-time) and the [open Architect plan](#one-architect-plan-at-a-time) check, so with both an open plan and a waiting idea, the skip reports the plan:

- **stdout** carries each waiting Architect idea's URL, one a line, newest first.
- **stderr** carries one progress line naming each of them as waiting for triage, several joined by `; `: `Architect idea #<n> "<title>" is waiting for triage: <Issue URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

Any triage decision releases the pause: take `needs-triage` off, whatever replaces it (`ready-for-agent`, `ready-for-human`, `needs-info`, `wontfix` or nothing), or close the issue. The issue keeps `architect-idea`, which only counts alongside `needs-triage`. An idea triaged `ready-for-agent` is then a [Ready issue](#a-ready-issue-goes-first), once it has settled, so the next Architect run is skipped for it in turn, until a Pickup run, or you, takes it.

### A Ready issue goes first

Work a human shaped goes ahead of the factory's own: while the repository has a **Ready issue**, by the very rule a [Pickup run](#pickup-runs) takes one by, every Architect run on it is skipped, before any review. The check comes last, after the [lock](#one-at-a-time), the [open Architect plan](#one-architect-plan-at-a-time) check and the [waiting Architect idea](#an-architect-idea-waits-for-triage) check, so with an open plan or a waiting idea as well, the skip reports that instead. It is only the search: an Architect run makes no [Sweep](#the-sweep) and holds no [Claim limit](#the-claim-limit), which are the Pickup run's.

- **stdout** carries the URL of the lowest-numbered Ready issue, the one a Pickup run would take.
- **stderr** carries, as a Pickup run's does, [a line on each issue labelled `ready-for-agent`](#why-an-issue-was-passed-over) passed over before it, then one progress line naming it: `Ready issue #<n> "<title>" goes first: <Issue URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

An issue labelled `ready-for-agent` that is not a Ready issue never skips an Architect run: one that carries a Claim, was labelled or shaped less than ten minutes ago, has an open blocker, is a Ticket inside a Spec that is not itself a Ready issue, or is a Base fix's issue, among the rest of the rule. Those get their lines on stderr all the same, and the Architect run goes on.

The rule doesn't ask whether anything will take the Ready issue. A `ready-for-agent` issue that you mean to run by hand, on a repository with no Pickup line in its crontab, keeps every Architect run on it from starting until you run it, which makes the Claim, or take the label off.

### One at a time

Only one Architect run or [Pickup run](#pickup-runs) per repository runs at a time on a machine. A `thirdshift architect` started while another on the same repository, or a `thirdshift pickup`, is still running, the Spec run or Run it dispatched included, is **skipped**: it prints `an Architect run or a Pickup run is already running on <owner>/<repo>` as a progress line on stderr, prints nothing on stdout, and exits `0`. Skipped is not a failure. A skipped run does nothing else: no launch pull, no worktree, no agent session, and no issue filed or labelled. This holds for every Architect run, however it was started, so two can never plan the same refactor. A Pickup run is skipped the same way while an Architect run or another Pickup run is still running, so the two never build one repository at once.

The repository is the one `origin` names, so two clones of one repository count as the same, and Architect runs on different repositories run at the same time. A Run or a Spec run started on an **Issue URL** is never skipped this way, and never makes an Architect run or a Pickup run skip.

thirdshift holds the rule with an operating-system lock on a file under `~/.thirdshift/architect-locks/`, one for both kinds of run, which the run's process takes without waiting, after its checks in step 1, and holds until it exits. The operating system releases the lock when that process ends for any reason, a crash, a kill or a reboot included, so there is never a stale lock to clear: the next Architect run or Pickup run on the repository runs normally. The file itself stays, and means nothing on its own. A review worktree that a killed Architect run left behind is removed by the next one.

A skipped Architect run sends no [Run notification](#one-run-notification), even when asked, and neither does a skipped Pickup run ([sends none](#one-run-notification-for-the-issue-taken)). Both make the notification's checks before the lock is tried all the same, so a broken setup fails the pass with exit `1` rather than being skipped.

### One Architect plan at a time

An Architect run is also skipped while an earlier **Architect plan** is still open, so a new refactor is never planned while the last one is unfinished: its pull request waiting for review, its run failed, or its plan published with `--plan-only` and not started.

thirdshift marks every Architect plan with the label `architect-plan`, in step 6: the Spec, or the standalone Ticket, never a Spec's Tickets. Once the one-at-a-time lock is its own, and before the launch pull, the worktree and the Architecture review, an Architect run lists the repository's open issues labelled `architect-plan`. If there are any, it is skipped:

- **stdout** carries each open Architect plan's URL, one a line, newest first.
- **stderr** carries one progress line naming each of them with the command that picks it up, several joined by `; `: `Architect plan #<n> "<title>" is still open: pick it up with thirdshift <plan URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

It applies to every Architect run, with or without `--plan-only`, and no flag overrides it. The rule is released by finishing the Architect plan, since merging its work closes the issue, by closing the issue, or by removing its `architect-plan` label. The check reads GitHub, so unlike the lock it holds across machines.

An Architect run never retries or dispatches an existing Architect plan. One whose dispatched run failed stays open, and picking it up is yours to do, with `thirdshift <plan URL>`, which continues whatever branch and pull request the failed run left. What isn't labelled `architect-plan` never counts here: a plan that a failed or interrupted review left `needs-triage`, or an [Architect idea](#no-strong-candidate), which [pauses Architect runs](#an-architect-idea-waits-for-triage) by its own rule, only while it waits for triage.

The check comes after the lock so that the plan of an Architect run that is still running, labelled already while its Spec run or Run goes on, is reported as [already running](#one-at-a-time), not as an unfinished plan. A skipped run sends no [Run notification](#one-run-notification), even when asked: its stdout carries the open Architect plans' URLs, and its stderr line their commands.

### On a schedule

thirdshift has no scheduler of its own: the operating system's scheduler runs the ordinary command. This crontab entry, written for a Linux machine with cron, starts an Architect run every night at 02:00. Add it with `crontab -e`, with your own paths in place of `/home/you` and `~/repos/thirdshift`:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main >> ~/.thirdshift/logs/cron.log 2>&1
```

- **`PATH`** is set because cron doesn't read your shell profile, and `thirdshift`, `claude`, `gh`, `git` and the repository's build tools must all be found. List every directory that holds one, written out in full: cron expands neither `~` nor `$HOME` on that line. `type thirdshift claude gh git` in your own shell shows where they are.
- **`cd`** makes a clone of the repository the **Launch directory**, as for a hand-typed Architect run, and **`base main`** names the [Base branch](#architect-runs), so the branch checked out in the clone doesn't matter: it runs from the clone you work in, whatever you left it on.
- **With a Pickup line on the same minutes**, add `&& sleep 20` between `cd` and `thirdshift architect`, as in the [whole crontab](#a-pickup-run-on-a-schedule) below. This gives the Pickup run a 20-second head start to take the [shared lock](#one-at-a-time) before the Architect run tries it.
- **One line per repository, at different hours**, so the Architect runs don't compete for the machine. thirdshift keeps no list of repositories.
- **The log file** takes everything the command prints. A failure before the [Run notification](#one-run-notification)'s checks have passed, such as a broken [User config](#user-config), a missing Resend key or a bad argument, shows up only there: no email is sent for it. Use one fixed file for every line, in a folder that already exists, such as `~/.thirdshift/logs/`: the shell opens it before thirdshift starts, so a line whose folder is missing never starts the command, and nothing reports it. What each pass did on its repository is in that repository's [Activity log](#logs), in folders thirdshift creates itself, and with `activity.quiet_skips = true` the log file holds no skips at all.
- **`claude` and `gh` must already be logged in** for the user the schedule runs as, with git able to push, as the [Prerequisites](#prerequisites) say. Without a terminal an Architect run behaves as it does from one, except that it never offers Setup.

The command's flags and the User config decide what a scheduled Architect run does, as for a hand-typed one: with `merge.always` it merges the Architect plan's pull request, and without it the pull request is left for review. These are the cautious variants, whatever the User config says:

```
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main no-merge >> ~/.thirdshift/logs/cron.log 2>&1
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main --plan-only >> ~/.thirdshift/logs/cron.log 2>&1
```

The first leaves the pull request for you to review, and the second stops at the Architect plan, for you to read and run with `thirdshift <Issue URL>`.

A scheduled Architect run is skipped when another on the repository, or a Pickup run, is [still running](#one-at-a-time), the Spec run or Run it dispatched included, when an [Architect plan is still open](#one-architect-plan-at-a-time), when an [Architect idea waits for triage](#an-architect-idea-waits-for-triage), or when the repository has a [Ready issue](#a-ready-issue-goes-first), so a `ready-for-agent` issue with no Pickup line to take it holds off every night until you run it: no new refactor is planned until last night's Architect plan is closed or loses its `architect-plan` label. Merging its pull request closes it; if its run failed, pick it up with `thirdshift <plan URL>` first. Closing the pull request alone leaves the Architect plan open. A skip exits `0`, so the scheduler sees no failure, and the repository's [Activity log](#logs) has the line that says why, as does the log file unless `activity.quiet_skips` is set. With Run notifications on, by `email.always = true` in the User config or `email` on the line, an Architect run that did work sends one email, and a skipped one sends none, so a crontab line can fire every few minutes, as for [Weeding](#weeding), without flooding your inbox. A night without an email means it was skipped, or failed before the notification's checks passed: look in the log file.

Any scheduler that runs the command works; a systemd user timer needs lingering on (`loginctl enable-linger`) to fire while you are logged out.

### Weeding

**Weeding** is the factory clearing what features leave behind in a codebase as they land, with nobody starting it: Architect runs started often enough that each one begins as soon as the last one's Architect plan is merged. thirdshift still never schedules itself ([ADR-0009](docs/adr/0009-the-operating-system-schedules-thirdshift.md)): it is the line from [On a schedule](#on-a-schedule), fired every five minutes instead of nightly, with `merge.always = true` in the [User config](#user-config) so each Architect plan's pull request is merged without waiting for you:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/5 * * * * cd ~/repos/thirdshift && thirdshift architect base main >> ~/.thirdshift/logs/cron.log 2>&1
```

The `PATH` line, the `cd` and `base main` and the logins are as in [On a schedule](#on-a-schedule). Set `activity.quiet_skips = true` in the User config too, so the skips leave the log file alone: each is in the repository's [Activity log](#logs), once per change of reason.

Most passes are skipped, and that is what paces Weeding. A pass is skipped while:

- **a run is still going**: an Architect run or a Pickup run on the repository, the Spec run or Run it dispatched included ([One at a time](#one-at-a-time));
- **an Architect plan is open**: one whose pull request waits for review, one whose run failed, or one published with `--plan-only` and not yet built ([One Architect plan at a time](#one-architect-plan-at-a-time));
- **an Architect idea waits for triage**, since the factory has run out of Strong ideas ([An Architect idea waits for triage](#an-architect-idea-waits-for-triage));
- **the repository has a Ready issue**, so work a human shaped goes first ([A Ready issue goes first](#a-ready-issue-goes-first)).

A skip exits `0` and sends no email, even with Run notifications on, so the scheduler sees no failure and your inbox only hears of passes that did work. The repository's [Activity log](#logs) has the line that says why, once for each change of reason. An Architect plan whose run failed stays open, so Weeding pauses for the **Day shift** until you pick it up with `thirdshift <plan URL>`, close it, or take its `architect-plan` label off; an Architect idea pauses it until you triage the idea.

On a repository that also has a [Pickup line](#a-pickup-run-on-a-schedule), put `--plan-only` on the Weeding line instead: the Architect run publishes the Architect plan and marks it ready, and a Pickup run builds it as an ordinary Ready issue, under the [Claim limit](#the-claim-limit), and merges it with `merge.always`, as in the [whole crontab](#a-pickup-run-on-a-schedule) below.

## Pickup runs

`thirdshift pickup` starts a **Pickup run**: one pass, with no **Issue URL**, that takes the lowest-numbered **Ready issue** in the repository and runs it, so that an issue you have already marked ready needs no command typed for it. Start it from a clone of the repository, on the **Base branch**, or name the Base branch with `base <branch>`:

```sh
cd ~/repos/widgets
thirdshift pickup                      # take the lowest-numbered Ready issue and run it
thirdshift pickup merge parallel 2     # merge its pull request; two Tickets at once if it is a Spec
thirdshift pickup base main no-merge   # run it on main, whatever branch the clone is on, and leave its pull request for review
```

A Ready issue is an open issue that:

- is labelled `ready-for-agent`: an issue without the label is never taken;
- has none of the labels that make an **Unready Ticket**, `ready-for-human`, `needs-info`, `wontfix` and `needs-triage`, so a contradictory label errs on the side of not running;
- carries no [Claim](#the-claim): it is not labelled `in-progress`;
- is not a sub-issue. A sub-issue is a **Ticket** of a **Spec**, and is never run on its own, whatever the Spec's labels: it is reached through its Spec, when the Spec is itself a Ready issue, so its work always goes through the **Spec branch**. Labelling one Ticket never promotes its Spec, either;
- is not labelled `base-fix`: the Run that opened a [Base fix](#base-fix)'s issue owns it;
- has no open blocker, by GitHub's "blocked by" links, never the text of its body. Once every blocker is closed, it can be taken;
- was never started: no **Issue branch** for it is on `origin`, and no pull request from one exists, open, merged or closed. So a Pickup run never does a [Continuation](#continuation), and an issue whose Run failed waits for you;
- is not a **Spec** whose **Tickets** are all closed. With nothing started on it, such a Spec was done some other way, and the [Spec run](#spec-runs) it would be dispatched as stops with nothing to do, so a Pickup run passes it over rather than take it on every pass. Waiting never makes it a Ready issue, so this comes before the next condition, however lately the Spec was labelled: close it, or take its `ready-for-agent` off. A Spec with at least one open Ticket is taken;
- is settled: ten minutes have passed since the latest of `ready-for-agent` being applied to it, a sub-issue being added to it or removed, and a "blocked by" link being added to it or removed. So a Spec is not taken while its Tickets are still being attached. These are read from the issue's timeline, since adding a sub-issue does not change the issue's update time, and the ten minutes is fixed, with no setting. A later pass takes the issue once it has settled.

A Pickup run:

1. Makes the checks an [Architect run](#architect-runs) makes, before anything else but the [Run notification](#one-run-notification-for-the-issue-taken)'s own: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached unless `base <branch>` names the Base branch, and the Base branch exists on `origin` with your local copy not ahead of it. A check that fails stops the pass with exit `1`, before any label is read or changed.
2. Tries the lock an Architect run takes, and is [skipped](#one-at-a-time) if an Architect run or another Pickup run on the repository is still running on the machine.
3. Takes `in-progress` off the repository's closed issues: the [Sweep](#the-sweep).
4. Counts the repository's open issues labelled `in-progress`, and is skipped if the repository is at its [Claim limit](#the-claim-limit).
5. Lists the repository's open issues labelled `ready-for-agent`, lowest number first, and takes the first that is a Ready issue, saying so on stderr: `taking Ready issue #<n> "<title>", as thirdshift <Issue URL> would`. Each one it passes over on the way gets [a line saying why](#why-an-issue-was-passed-over). One issue a pass: the rest wait for the next.
6. Dispatches it exactly as `thirdshift <Issue URL>` would from the same clone on the Pickup run's Base branch: a [Spec run](#spec-runs) when the issue has sub-issues, a Run otherwise. That run makes the Claim, so the issue's `ready-for-agent` is swapped for `in-progress` and no later pass takes it again.

`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel <n>`, `harness <name>`, `model <name>` and `effort <level>`, with or without dashes, are for the dispatched run, and mean what they do for `thirdshift <Issue URL>`, as do `email`, optionally followed by an address, and `no-email` for the Pickup run's [Run notification](#one-run-notification-for-the-issue-taken). The [User config](#user-config) sets what they leave unsaid: `merge.always`, `base.fix`, `spec.parallel`, `email.always`, `launch.pull` and `logs.dir`. A User config a Run would refuse stops the pass the same way, before any check. `parallel <n>` applies when the Ready issue is a Spec and is ignored, with no error, when it isn't, unlike on an Issue URL: the command can't know which it will take. `base <branch>` (or `--base <branch>`) names the Base branch as it does for [`architect`](#architect-runs), and the dispatched run takes it: its Issue branch or Spec branch is branched off `<branch>`, and its pull request targets it. With `base <branch>`, a Pickup run started [on a schedule](#a-pickup-run-on-a-schedule) runs from the clone you work in, whatever branch you left it on.

`pickup` is a command only as the first argument, and takes nothing but those flags, each at most once: a focus, `--plan-only`, an Issue URL, a repeated or contradictory flag, or any other argument is an argument error (exit `2`).

The dispatched run's ending is the Pickup run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Pickup run fails as that [Failed run](#failed-runs) or Failed spec run does. After an [Inherited failure](#an-inherited-failure-links-the-base-branchs-checks), the command it offers is `thirdshift <Issue URL>` with the dispatched run's flags and `base-fix`, since another Pickup run would not take a started issue again.

A pass is **skipped** when the lock is held, when the repository is at its Claim limit, or when the repository has no Ready issue: it exits `0` with stdout empty and one line of reason on stderr, `an Architect run or a Pickup run is already running on <owner>/<repo>`, `at the Claim limit on <owner>/<repo>: <count> open issue(s) labelled in-progress, pickup.limit is <limit>` or `no Ready issue on <owner>/<repo>`, the last after the lines on the issues it passed over. Skipped is not a failure. A skipped pass starts no agent session, makes no Claim, and sends no Run notification. One skipped for the lock changes no label at all; the others have made the Sweep, whose lines come before the reason.

### The Claim limit

A Pickup run takes nothing while the repository is at its **Claim limit**: as many open issues carry a [Claim](#the-claim), that is, are labelled `in-progress`, as the limit, which is 3 unless you set it. It keeps a broken Base branch from failing every Ready issue in turn, one a pass, and pull requests from piling up unreviewed: once that many issues wait on you, the factory waits too.

- The count is every open issue labelled `in-progress` in the repository, whoever started it: a Run you started by hand counts as one a Pickup run dispatched does. Closed issues don't count.
- `pickup.limit` in the [User config](#user-config) sets the limit, a whole number from 1 up, by default 3. Raise it on a machine where you review quickly; `limit = 1` takes one issue at a time. There is no command-line flag for it.
- At or over the limit, the pass is skipped: exit `0`, stdout empty, and the reason on stderr with the count and the limit, such as `at the Claim limit on acme/widgets: 3 open issue(s) labelled in-progress, pickup.limit is 3`. No Ready issue is looked for, and no session is started.
- An issue leaves the count when it is closed, as merging its pull request does, or when you take its `in-progress` label off.
- If the issues can't be counted, the pass stops with exit `1` and `gh`'s error.

### The Sweep

In the **Sweep**, each Pickup run, once it holds the lock and before it counts, lists the repository's closed issues labelled `in-progress` and takes the label off each, so an issue whose pull request you merged by hand doesn't look taken for ever.

- Only `in-progress` is removed, in one request per issue: the closed issue's other labels stay. stderr says `taking in-progress off #<n>, which is closed`.
- It runs on every pass that gets the lock, one that is then skipped for the Claim limit or for having no Ready issue included, and not on one skipped because the lock is held.
- A failure, to list the closed issues or to take the label off one, is a `warning:` line on stderr, such as `warning: could not take in-progress off #<n>: <gh's error>`, and the pass carries on: the next pass tries again.

### Why an issue was passed over

Each open issue labelled `ready-for-agent` that a pass looks at and does not take gets one line on stderr, naming the issue and the first reason that applies, in the order of the Ready issue rule above. The lines come before the line that says what the pass did, so the cron log explains why nothing started:

```
thirdshift: 03:00:01 Pickup run starting, 2026-10-03 -0400
thirdshift: 03:00:02 #18 labelled needs-info
thirdshift: 03:00:02 #19 labelled in-progress
thirdshift: 03:00:03 #21 is a Ticket of #20, which is not ready
thirdshift: 03:00:03 #26 labelled base-fix
thirdshift: 03:00:04 #30 blocked by #29
thirdshift: 03:00:05 #31 already started: issue-31 is on origin
thirdshift: 03:00:06 #32 already started: PR https://github.com/acme/widgets/pull/37
thirdshift: 03:00:06 #33 every Ticket is closed
thirdshift: 03:00:07 #34 not settled: labelled ready-for-agent less than 10 minutes ago
thirdshift: 03:00:07 #35 not settled: a sub-issue added or removed less than 10 minutes ago
thirdshift: 03:00:08 #36 not settled: a "blocked by" link added or removed less than 10 minutes ago
thirdshift: 03:00:08 no Ready issue on acme/widgets
```

A line with `blocked by` names every open blocker. A Ticket's line names its Spec: the Ticket runs when its Spec does. A Ticket whose Spec is a Ready issue gets no line: its Spec is taken, by this pass or a later one. A Spec with every Ticket closed that was started, with its Spec branch on `origin` or a pull request from it, gets the `already started` line, so `every Ticket is closed` is only ever said of one with nothing on `origin`. When a later issue is a Ready issue, the lines on the earlier ones are printed and it is taken:

```
thirdshift: 03:00:01 Pickup run starting, 2026-10-03 -0400
thirdshift: 03:00:02 #18 labelled needs-info
thirdshift: 03:00:03 #20 blocked by #17
thirdshift: 03:00:04 taking Ready issue #22 "Sharpen the widgets", as thirdshift https://github.com/acme/widgets/issues/22 would
thirdshift: 03:00:04 logging this command to ~/.thirdshift/logs/acme/widgets/commands/pickup/22-20261003T030001-0400.log
```

### One Run notification for the issue taken

A Pickup run asked for a [Run notification](#run-notifications), by `email` or by `email.always = true` in the [User config](#user-config) without `no-email`, sends exactly one when it took a Ready issue: the notification the Spec run or Run it dispatched would have sent started by hand. An address after `email` wins over `email.to`, as for a Run.

- **Subject**: that run's, `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, naming the issue taken, with the outcome `ready for review`, `merged`, `failed` or `interrupted`.
- **Body**: that run's too: what a Run's notification holds, and after a Spec run a line per Ticket. The time it gives is the whole pass's, the search for the Ready issue included.

The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout. The dispatched run sends no notification of its own, whatever the User config says, as with an [Architect run](#one-run-notification), nor do a Spec run's Ticket Runs.

A skipped pass sends no notification, even when one was asked for: a pass every half hour would otherwise send dozens a day. A skipped Architect run sends none either.

The notification's checks, an address and a Resend API key, are made before any other work on every pass, before the checks in step 1, the lock and any label read or changed. A broken setup therefore stops the pass with exit `1`, naming what is missing, even a pass that would have been skipped, so it shows in the scheduler's log on the first pass, not only once an issue is ready. A pass that asks for no notification makes none of these checks.

### A Pickup run on a schedule

thirdshift has no scheduler of its own ([ADR-0009](docs/adr/0009-the-operating-system-schedules-thirdshift.md)): the operating system's scheduler runs the ordinary command, one pass each time. This crontab entry, written for a Linux machine with cron, starts a pass every half hour. Add it with `crontab -e`, with your own paths in place of `/home/you` and `~/repos/widgets`:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

The `PATH` line, the `cd` and `base main`, the log file's directory, the logins `claude` and `gh` need, and schedulers other than cron are as for an Architect run: see its [On a schedule](#on-a-schedule). What differs for a Pickup run:

- **The interval** sets how soon a Ready issue is taken and how much work is started: a pass takes one issue, so `*/30` starts at most one every half hour. A pass lasts as long as the run it dispatched, and the passes that fire meanwhile are skipped, so the schedule builds one issue of a repository at a time on a machine, and the first pass after it ends takes the next, unless the repository is by then at its [Claim limit](#the-claim-limit): each pull request left for review, and each failure left for you, holds a Claim, and at 3 of them, or the `pickup.limit` you set, the passes take nothing until you have dealt with one. An issue you have just labelled also waits ten minutes, until it is settled. A Run you start by hand on an Issue URL is outside all this: it neither waits for a pass nor makes one skip.
- **One line per repository.** thirdshift keeps no list of repositories. A Pickup run and an Architect run on one repository each skip while the other is still running, the Spec run or Run it dispatched included, so both lines can go in one crontab and the two never build that repository at once. Passes on different repositories do run at the same time: give their lines different minutes, such as `15,45`, if they would compete for the machine.
- **A pass is skipped** when the [lock is held](#one-at-a-time), when the repository has no Ready issue, and when it is at its Claim limit. A skip exits `0`, so the scheduler sees no failure, and sends no email, even with Run notifications on. The repository's [Activity log](#logs) records a skip when its reason changes. Unless `activity.quiet_skips` is set, the log file holds each skip's line too, and before the line of a pass that found no Ready issue, as before the line of one that took an issue, [why each issue was passed over](#why-an-issue-was-passed-over). A pass skipped for the lock or the Claim limit looks at no issue, so it has no such lines.
- **A Run notification**, by `email.always = true` in the [User config](#user-config) or `email` on the line, is sent [only for an issue taken](#one-run-notification-for-the-issue-taken). So a day without email doesn't say the passes are running: a pass that fails before it takes an issue, on a broken User config, a missing Resend key or a failed check, shows up only in the log file.

A whole crontab with both Weeding and a Pickup run on one repository looks like this, with `activity.quiet_skips = true` in the User config, every line sending its output to the one log file, which catches only what went wrong, so `tail ~/.thirdshift/logs/acme/widgets/activity.log` shows what the factory has been doing on the repository lately:

```
# cron reads no shell profile: list every directory that holds thirdshift, claude, gh, git and the build tools
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin

# Weeding, every five minutes: an Architect run publishes each Architect plan, and a Pickup run builds it
*/5 * * * * cd ~/repos/widgets && sleep 20 && thirdshift architect base main --plan-only >> ~/.thirdshift/logs/cron.log 2>&1

# Pickup runs, every half hour
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

For one Architect run a night instead of Weeding, put the nightly line from [On a schedule](#on-a-schedule) in place of the Weeding line: a repository needs one or the other, since Weeding is Architect runs started more often. Adding a repository is adding its lines. Add the lines with `crontab -e` rather than by joining files, and end each with a newline: a line run together with the comment after it, such as `... 2>&1# Edit this file`, is a shell syntax error, so cron fires it but the pass never starts and nothing reaches its log.

The command's flags and the User config decide what a pass does with the issue it takes, as for a hand-typed `thirdshift <Issue URL>`: with `merge.always` it merges the pull request, and without it the pull request is left for review. This is the cautious variant, whatever the User config says:

```
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main no-merge >> ~/.thirdshift/logs/cron.log 2>&1
```

An issue whose run failed [keeps its Claim](#when-the-claim-ends) and waits for the **Day shift**: it stays `in-progress`, with the Issue branch and any draft pull request the run left, no later pass takes it, and it counts towards the Claim limit until it is closed or you take the label off. To send it round again by hand, run `thirdshift <Issue URL>` from the clone, with the Base branch checked out, since that command takes no `base <branch>`: it picks up where the failed run stopped, as a [Continuation](#continuation) does. Labelling it `ready-for-agent` again doesn't do it: a Pickup run never takes an issue that was started. The one failure a later pass does retry is one that left nothing on `origin`, such as a usage limit or an expired login: its Claim is released, so the issue is a Ready issue again once it has settled.

If you shape your issues with the upstream `to-spec` and `to-tickets`, from [mattpocock/skills](https://github.com/mattpocock/skills), unchanged: `ready-for-agent` on a **Spec** must mean its **Tickets** are published, every Ticket attached as a sub-issue and every "blocked by" link in place. Upstream, `to-spec` labels the Spec `ready-for-agent` when it publishes it, minutes or days before `to-tickets` attaches the Tickets, and a Spec with no Tickets yet looks exactly like a standalone Ticket, so a pass would start a plain Run on it. Label such a Spec `needs-triage` until its Tickets are attached, then swap that for `ready-for-agent`, as the copies of the two this repository's own Day shift uses, under `.agents/skills/`, do ([`docs/agents/triage-labels.md`](docs/agents/triage-labels.md)). The ten minutes an issue must be settled for are only a second line of defence: they cover Tickets attached within minutes of the label, not a Spec left labelled and without Tickets for longer.

## Building from source

Building needs the Rust toolchain. From a clone of this repository:

```sh
cargo install --path .
```

`cargo install` puts the binary in `~/.cargo/bin`. To update it, pull and reinstall:

```sh
git checkout main && git pull
cargo install --path .
```

If the shell installer also put a copy in `~/.local/bin`, typing `thirdshift` runs whichever copy comes first on your `PATH`, and the installer's copy often does. To see every copy, in the order the shell tries them:

```sh
type -a thirdshift
```

To run the build from source, give its path, as in `~/.cargo/bin/thirdshift <Issue URL>`, or put `~/.cargo/bin` first on your `PATH`, for example in `~/.bashrc`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

`thirdshift version` can't tell the two apart: a build from `main` reports the last released version until the next release bumps it.

Editing a skill in `skills/` has no effect until you rebuild and reinstall ([ADR-0001](docs/adr/0001-rust-binary-with-embedded-skills.md)).

The [Prompts and skills page](https://thirdshift.app/prompts/) is generated from the prompts, the `claude` arguments and the skills, and each Session prompt is also generated as a Markdown file in [`prompts/`](prompts/), with a [`README.md`](prompts/README.md) that lists them with the `claude` command lines. `cargo test` fails until they are regenerated after a change to any of them. Regenerate the page and `prompts/` with `UPDATE_PROMPTS=1 cargo test prompts_page`, which also deletes any file in `prompts/` that it doesn't generate.

The integration tests swap in a fake `gh` and `claude`, from [`tests/fakes.rs`](tests/fakes.rs). The first test that needs them builds them with `rustc` (the one on `PATH`, or `$RUSTC`), so the suite needs nothing else on `PATH` beyond `git` and `bash`.

### The Rust version

[`rust-toolchain.toml`](rust-toolchain.toml) pins the exact Rust version this repository builds with, and the `rustfmt` and `clippy` components CI runs. `rustup` reads it, so `cargo` in a checkout, CI, the release builds and the crates.io publish all use that version, and `rustup` installs it the first time it is needed. No workflow names a version of its own. The file is not in the crates.io package, so `cargo install thirdshift` builds with whatever Rust the machine has.

A newer Rust arrives as a pull request that changes `channel` in that file, so it runs through CI before `main` builds with it. Dependabot opens that pull request within a week of a stable release ([`.github/dependabot.yml`](.github/dependabot.yml)). To move the pin by hand, set `channel` to the new version in full, patch number included, run the three checks CI runs, and open a pull request with the change and whatever the new compiler's lints asked for:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Releasing

From a clone of this repo, signed in to `gh` and with `claude` logged in, run the release script with the new version:

```sh
scripts/release.sh 0.4.0
```

It works from `origin/main` in a temporary worktree, so your checked-out branch and uncommitted changes don't matter and aren't touched. It opens a pull request from `release-<version>` that bumps the version in `Cargo.toml` and `Cargo.lock` and nothing else. Its body is a summary of the release, then the version diff. `claude -p`, with no tools, writes the summary from the version diff and the title, number and body of each pull request merged since the last tag, following the prompt in [`scripts/release-summary.md`](scripts/release-summary.md). If `claude` fails or prints nothing, the script warns and uses GitHub's generated notes as the summary instead, with a note saying so. It writes the summary before it pushes anything. With `--review` (`scripts/release.sh --review 0.4.0`), it then prints the summary and asks `[y]es / [e]dit / [n]o`: `y` carries on, `e` opens the summary in `$EDITOR` and carries on with what you save, and `n` stops with no branch, pull request or tag pushed. Without `--review` it never prompts. It waits for the pull request's checks, merges it with a merge commit, then tags that merge commit `v<version>` and pushes the tag. Pushing the tag starts the release workflow (below), and the script then waits for that workflow's run on the merge commit, printing the run's URL so you can watch it. CI's run on `main` for the same commit isn't waited for. Once the release workflow run succeeds, the GitHub Release is published with its final body and the crate is on crates.io, and the script prints the GitHub Release's URL and exits `0`. It prints a line on stderr for each step. If the checks fail, it stops before merging and leaves the pull request open. If the release workflow run fails, it exits `1`, naming the failed jobs and the run's URL, and leaves the tag in place: the tag is already public, so rerun the failed jobs with `gh run rerun --failed <run id>` rather than cutting a new version. Neither wait has an overall timeout.

Before it pushes anything, it refuses a release that can't be cut and says why: a version that isn't plain `X.Y.Z` (no leading `v`), a version that isn't higher than the one in `Cargo.toml` on `origin/main`, a `v<version>` tag that already exists locally or on origin, or a latest CI run on `origin/main` that failed or hasn't finished.

If it's interrupted, run it again with the same version: it picks up the existing branch or pull request and carries on from the first step not yet done, without the checks above, which the first run passed. With `--review`, it asks about a new summary before opening a pull request for a branch already pushed. Once the tag is on the merge of the pull request, it says the release is already tagged and picks up from the release workflow run: it waits for a run still going, prints the GitHub Release's URL and exits `0` for one that succeeded, and exits `1` as above for one that failed, reporting the run's latest attempt, so a rerun of its failed jobs counts. So you can stop it with Ctrl-C while it waits and run it again to carry on waiting. A `v<version>` tag anywhere else on origin is still refused.

The tag starts the release workflow, generated by [`dist`](https://github.com/axodotdev/cargo-dist) from `dist-workspace.toml`. It builds the Linux and macOS binaries and the installer, checks that the Linux binary starts on Rocky Linux 9 and in WSL2 with Ubuntu 24.04, and only then publishes the GitHub Release. It then publishes the same version to crates.io, skipping it if it is already there, and finally replaces the Release's body with the bump pull request's summary, then notes generated from the merged pull requests, then the install instructions. [`scripts/release-notes.sh`](scripts/release-notes.sh) builds that body; for a tag pushed by hand, with no bump pull request summary, it is the generated notes and the install instructions alone.

## Credits and license

thirdshift is released under the [MIT License](LICENSE).

The Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](skills/LICENSE) and is embedded in the binary and written out with the skills on every Run. The `thirdshift-pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/thirdshift-pr/CREDITS.md`](skills/thirdshift-pr/CREDITS.md).
