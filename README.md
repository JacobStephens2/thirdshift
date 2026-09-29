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
2. Picks the **Issue branch** (`issue-<n>`, or `issue-<n>-branch-<k>` once earlier ones are finished) and the **Base branch**, normally the branch you have checked out.
3. Creates a git worktree next to your clone, named `<repo>-<Issue branch>`, so your own checkout is never touched.
4. Runs a headless Claude Code session in that worktree with the **Factory skills** loaded. The agent implements the issue, reviews its work against the Base branch, addresses the **Standards findings** and **Spec findings** it agrees with, and opens a ready-for-review pull request that lists every **Unaddressed finding** with a reason and closes the issue.
5. Takes over deterministically: pushes the Issue branch, skipping your repo's git hooks since the session runs the tests and CI gates the pull request, checks through `gh` that the pull request exists, is open and targets the Base branch, and marks it ready for review (`gh pr ready`) if the agent left it as a draft.
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 5 per Run. If a CI-fix Repair leaves the head unchanged, with no commit of its own and no Base branch move to merge, the Run ends as a [Failed run](#failed-runs), since CI would stay red on the same head. If the Base branch moves while CI runs, it merges it again and goes round, at most 5 times per Run. In a Merge run, each round also takes in **Foreign commits** first: see [Foreign commits in a Merge run](#foreign-commits-in-a-merge-run).
7. In a **Merge run** (`thirdshift merge <Issue URL>`), does the **Self-merge**: once the pull request is open, ready for review, mergeable and green, thirdshift merges it into the Base branch with a merge commit, on exactly the head commit whose CI it watched (`gh pr merge --merge --match-head-commit <sha>`). It never uses GitHub's auto-merge ([ADR-0004](docs/adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)). A merge that fails goes back round step 6, within the same budgets, and is tried again on the new green head. If that round finds nothing to fix, the refusal is a **policy refusal**, such as merge commits being disallowed or a review being required. After the merge, thirdshift deletes the Issue branch on `origin`, and closes the issue if it is still open, with the comment `Closed by #<pr>, merged into <base> by a thirdshift Merge run.` GitHub closes it on its own only for a merge into the repository's default branch, and then thirdshift leaves it alone.
8. Cleans up: removes the worktree, the local Issue branch and the temporary plugin directory, whether the Run succeeded or not. The one exception is a **Failed run** whose work could not be pushed: see below.

### Status

The designed behaviour described in this README is implemented. Known gaps and planned work are tracked in the [open issues](https://github.com/JacobStephens2/thirdshift/issues).

## Install

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
```

The shell installer and the binary both come from this repository's [GitHub Releases](https://github.com/JacobStephens2/thirdshift/releases). It puts `thirdshift` in `~/.local/bin`, where Claude Code's installer puts `claude`, and needs no Rust toolchain. thirdshift is a single binary with the Factory skills compiled in, so it runs without a checkout of this repository ([ADR-0001](docs/adr/0001-rust-binary-with-embedded-skills.md), [ADR-0003](docs/adr/0003-static-binaries-through-github-releases.md)).

If you use Rust, you can install it from crates.io instead:

```sh
cargo install thirdshift
```

### Updating

```sh
thirdshift update
```

It replaces the installed binary with the latest stable GitHub Release, or says it is already on it. Messages go to stderr and stdout stays empty. It exits `0` when it updated or was already up to date, and `1` on any failure, such as no network. Updating is safe while a Run is using the old binary, and a Run never checks for updates or updates itself.

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
- **`gh`** (GitHub CLI), logged in.
- **`git`** with a global `user.name` and `user.email`, and credentials that can push to the repository (`gh auth setup-git` makes git use `gh`'s login). The agents commit as this identity; without it, an agent may borrow the author of the last commit.

### Auto mode

Sessions run headless in Claude Code's auto mode (`claude -p --permission-mode auto`), with the full permissions of that user account, including `sudo` if the user has it. Nobody is there to approve anything; instead, auto mode's classifier checks each action and may block ones it judges risky, such as destructive commands or actions outside the task. A blocked action the agent can't work around can end the Run as a Failed run.

`claude -p` exits as soon as the agent ends its turn, killing any background task it started. So every prompt tells the agent to run long commands, such as tests, in the foreground. If a session still ends while waiting on background work, thirdshift gives it a **Resume**: it continues that same session once (`claude -p --resume`), asking the agent to re-run the work in the foreground and finish. If the Resume ends the same way, the Run fails and says which task was killed.

### What sessions leave behind

Cleanup removes only thirdshift's own worktree, local Issue branch and temporary plugin directory. Anything else an agent does as your user stays. For example, one Run downloaded a JDK to `~/.local/jdk/` because the server had no Java, installed Playwright in `/tmp/pw`, and left a pull request body draft in `/tmp/`. This is by design, but worth knowing:

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
- **stderr** carries everything else: errors, cleanup problems, and progress lines while sessions run. A successful Run's last line names the pull request too: `PR <url> is ready for review`, or `PR <url> is merged` after a Merge run, followed only by a `warning:` line if a [Run notification](#run-notifications) can't be sent.
- **Exit code** `0` means the Run ended with a pull request the factory stands behind, merged in a Merge run. Once the Self-merge has merged, the Run succeeds even if deleting the Issue branch on `origin` or closing the issue then fails: the merge can't be undone, so each failed step is a `warning:` line on stderr naming the command to run by hand, and the Run still exits `0` with the URL on stdout. Ctrl-C likewise: before the merge it makes a Failed run, after it thirdshift finishes these steps and exits as merged. `2` means the Issue URL is missing or isn't a GitHub Issue URL, there is an argument other than the URL and the Run flags, or a Run flag is repeated or contradicts another; the error and the help text go to stderr, before any work. A [User config](#user-config) thirdshift can't use exits `1`, also before any work. Any other failure exits `1`.

The other commands:

```sh
thirdshift email-test [<address>]   # send a test email through Resend (see Email)
thirdshift setup                    # write the User config with every setting at its default (see User config)
thirdshift update                   # update to the latest release (see Updating)
thirdshift version                  # print thirdshift <version>
thirdshift help                     # print every form of the command, each with a one-line description
```

`version` and `help` print to stdout and exit `0`. `update`, `setup` and `email-test` follow the Run's rule: stdout stays empty, messages go to stderr.

Uncommitted changes in your clone are fine: the Run works in its own worktree from `origin`, so they are simply left out. Unpushed commits on the Base branch are not: push them first, or the Run stops.

### User config

A **User config** at `~/.thirdshift/config.toml` sets this machine's defaults for every Run. It is optional: with no file, or one that says nothing about a setting, a Run does only what its command asks for.

```toml
[merge]
always = true   # every Run is a Merge run, without the merge word

[launch]
pull = true     # every Run first fast-forwards your checkout of the Base branch

[email]
always = true                                 # every Run sends a Run notification, without the email word
to = "you@example.com"                        # where email goes when the command names no address
from = "thirdshift@your-verified-domain.com"  # the sender; onboarding@resend.dev if unset

[logs]
dir = "~/elsewhere/logs"   # where session logs go, instead of ~/.thirdshift/logs
```

`thirdshift setup` writes this file for you, listing every setting at its default so the file itself shows what can be changed:

```toml
[merge]
always = false   # every Run is a Merge run, without the merge word; default false

[launch]
pull = false     # every Run first fast-forwards your checkout of the Base branch; default false

[email]
always = false                  # every Run sends a Run notification, without the email word; default false
# to = "you@example.com"        # where email goes when the command names no address; no default
from = "onboarding@resend.dev"  # the sender; default onboarding@resend.dev, which only delivers to your Resend account's address

[logs]
dir = "~/.thirdshift/logs"   # where session logs go; default ~/.thirdshift/logs
```

Every key holds its real value, so a Run reading it does exactly what it does with no file. `email.to` has no default, so it is the only line written commented out. `setup` asks nothing, prints the file's path on stderr and exits `0` with stdout empty. A User config that is already there keeps its values; one a Run would refuse is refused the same way, exit `1`, and not touched. Any argument after `setup` is an argument error (exit `2`).

With `merge.always = true`, `thirdshift <Issue URL>` is a Merge run, and `thirdshift --no-merge <Issue URL>` (or `no-merge`, before or after the URL) leaves that one Run's pull request ready for review.

With `launch.pull = true`, every Run brings the Base branch checked out in the directory you start it from (the **Launch directory**) up to date with `origin`, so you no longer `git pull` by hand before each Run. It happens after the pre-flight checks pass and before the worktree is created, as `git merge --ff-only origin/<Base branch>`: fast-forward only, never a merge commit or a rebase, always from `origin`, whatever the branch's upstream or your `pull.*` settings. A progress line on stderr says when it updates the branch; an already up-to-date branch is left quietly as it is. It is skipped when the checked-out branch isn't the Base branch, as in a Continuation whose open pull request targets another base, or on a detached HEAD. If the update can't happen, for example because uncommitted changes are in the way, stderr gets a `warning:` line with git's error and the command to run by hand, your changes are left as they were, and the Run carries on with the same outcome and exit code. The setting only affects your checkout: the Run's worktree starts from `origin/<Base branch>` either way.

With `email.always = true`, every Run sends a [Run notification](#run-notifications) to `email.to`, as if given `--email`, and `thirdshift --no-email <Issue URL>` (or `no-email`, before or after the URL) sends none for that one Run.

`logs.dir` sets the directory [session logs](#logs) are written to, created if missing. It must be an absolute path, `~` or a path starting with `~/`, where `~` stands for `$HOME`. A relative path stops the Run before any work, since the directory a Run is launched from is no base for a setting that holds for every Run.

A Run reads the file before any work. One that isn't valid TOML, or that has a key or section thirdshift doesn't know, such as `alway` for `always`, or a value of the wrong type, such as anything but `true` or `false` for `always`, stops the Run with an error naming the file and the offending key, so a typo can't silently leave a setting off. `email-test` and `setup` read it the same way. `update`, `version` and `help` never read it, so a broken User config can't block them.

### Email

thirdshift sends email itself, with one HTTPS request to [Resend](https://resend.com)'s API, so it needs no mail server on the machine and works where SMTP ports are blocked ([ADR 0005](docs/adr/0005-run-notifications-through-resend.md)). It needs a Resend account and an API key:

- **`RESEND_API_KEY`**, an environment variable, holds the API key. thirdshift reads the key only from there, never from the User config, so the config file holds no secret.
- **`email.to`** in the [User config](#user-config) is the address email goes to when the command gives none.
- **`email.from`** is the sender. Without it, email comes from **`onboarding@resend.dev`**, Resend's shared sender, which only delivers to the address of your own Resend account. To send to any other address, set `email.from` to an address on a domain you have verified with Resend.

To check the setup without starting a Run:

```sh
export RESEND_API_KEY=re_...
thirdshift email-test you@example.com   # or just `thirdshift email-test`, to send to email.to
```

It sends one test email, whose subject marks it as a test and whose body names the host, the time and the sender. Before sending, it checks that it has an address (the argument, else `email.to`) and a non-empty `RESEND_API_KEY`; if either is missing, it exits `1` naming what's missing and sends nothing. Nothing is sent to check the key itself. When Resend accepts the email, it prints `accepted by Resend; check your inbox` and exits `0`; that is all it can verify, so check that the email arrives. When Resend refuses it, for example for a bad key or a sender it won't send from, it prints Resend's error text word for word and exits `1`. It gives up after 30 seconds without an answer.

#### Run notifications

`--email` (or `email`) asks a Run for a **Run notification**: one email, sent when the Run ends, whatever the outcome. The word after the flag is the address only if it contains `@` and doesn't start with `https://`, so the Issue URL is never taken for it; otherwise the email goes to `email.to`. With `email.always = true` in the [User config](#user-config), a Run asks for one without the flag, and `--no-email` (or `no-email`) skips it for that Run; an address after `--email` still wins over `email.to`. Giving a flag twice, or `--email` together with `--no-email`, is an argument error.

A Run that asks for a notification, by the flag or by `email.always`, makes the same checks as `email-test` before any other work: an address is known, and `RESEND_API_KEY` is set and not empty. If either fails, the Run stops, exits `1` naming what's missing, and sends nothing. Once they pass, every way the Run ends sends exactly one notification, after its outcome is final and its cleanup done: ready for review, merged, a [Failed run](#failed-runs) (including a later preflight failure such as an origin mismatch), or interrupted by Ctrl-C, SIGTERM or a closed terminal.

- **Subject**: `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, where the outcome is `ready for review`, `merged`, `failed` or `interrupted`. The title is left out if it can't be read from GitHub.
- **Body**, plain text: the pull request URL (if any), the failure cause (if failed), the session log path (if any), the hostname and how long the Run took.

A notification that can't be sent is a `warning:` line on stderr with Resend's error. It never changes the Run's outcome, stdout or exit code.

### Logs

Each session's full transcript, as Claude Code's `stream-json` output, is written to its own file under `~/.thirdshift/logs/`, or the `logs.dir` set in the [User config](#user-config) (created if missing):

```
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-implement.jsonl
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-repair-<i>.jsonl
```

A Resume is logged as its session's kind plus `-resume`, e.g. `implement-resume.jsonl`.

All sessions in a Run share the Run's UTC timestamp, so a Run's logs sort together. When a Run fails, stderr ends with the path of its most recent session log, the place to start looking.

## Foreign commits in a Merge run

A Merge run merges only code an agent wrote or reviewed. If someone else pushes to the Issue branch during the Run, their **Foreign commits** are reviewed before they can be merged:

1. Each round of step 6 starts by fetching the Issue branch from `origin`. Any new commits there are merged into the Run's branch, by fast-forward or a merge commit, never a rebase, each logged on stderr as `merging new commit <sha> from origin/<branch>`. Foreign commits that arrive while CI runs on a green head send the Run round again rather than on to the merge. A merge that conflicts with the Run's own work gets a conflict Repair.
2. A **review Repair** then runs `/thirdshift:code-review` with the head thirdshift last knew as the Run's own as the fixed point. The agent fixes the findings it agrees with, adds the rest to the pull request body's "Unaddressed findings" section marked as coming from the Foreign commits, and pushes.
3. The round goes on as usual: the Base branch is merged in, CI watched, and the merge tried once the head is green.

Review Repairs are logged as `repair-<i>`, numbered with the other Repairs, count against the cap of 5 Repairs, and get a Resume like any session. A round that picks up Foreign commits counts against the same 5, like a Base branch move. A Run that spends either budget fails.

A Run without `merge` does none of this: it never fetches someone else's commits into its branch.

## Continuation

Running thirdshift again on an issue picks up where the last Run stopped ([ADR-0002](docs/adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed run with the same command, or start an issue on one server and continue it on another.

A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit.

## Failed runs

A **Failed run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI, or, for a Merge run, without that pull request merged. Causes include the session exiting non-zero, no pull request or one with the wrong base, running out of Repairs, a CI-fix Repair that finds nothing on the branch to fix (a **declined CI fix**: the head is unchanged after it, so CI would stay red), a session that still ends while waiting on background work after its Resume, and a Base branch that keeps moving while CI runs, or in a Merge run, an Issue branch that keeps getting Foreign commits.

A Failed run:

1. Commits any uncommitted work as `thirdshift: failed run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again. The exception is a Merge run's policy refusal: the pull request is ready, mergeable and green and only the Self-merge could not happen, so it stays ready for review, and no failure commit is pushed onto the head whose CI was watched.
3. Cleans up as usual, prints the reason to stderr and exits non-zero. If the push failed, the worktree and local Issue branch are kept instead, and stderr names the branch, its head commit and the worktree path, so you can recover the work or push it by hand.

Merges, never rebases or force-pushes: a branch worked on from several servers never loses history.

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

The [Prompts and skills page](https://thirdshift.app/prompts/) is generated from the prompts, the `claude` arguments and the skills, and `cargo test` fails until it is regenerated after a change to any of them. Regenerate it with `UPDATE_PROMPTS_PAGE=1 cargo test prompts_page`.

Running the test suite (`cargo test`) also needs **`python3`** on `PATH`: the integration tests swap in fake `gh` and `claude`, which are Python scripts in `tests/fakes/`.

## Releasing

1. Bump `version` in `Cargo.toml` (and `Cargo.lock`) in a pull request.
2. Merge it.
3. Push a `v<version>` tag on the merge commit, e.g. `git tag v0.2.0 && git push origin v0.2.0`.

The tag starts the release workflow, generated by [`dist`](https://github.com/axodotdev/cargo-dist) from `dist-workspace.toml`. It builds the Linux and macOS binaries and the installer, checks that the Linux binary starts on Rocky Linux 9 and in WSL2 with Ubuntu 24.04, and only then publishes the GitHub Release. It then publishes the same version to crates.io, skipping it if it is already there, and finally replaces the Release's body with notes generated from the merged pull requests.

## Credits and license

thirdshift is released under the [MIT License](LICENSE).

The Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](skills/LICENSE) and is embedded in the binary and written out with the skills on every Run. The `pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/pr/CREDITS.md`](skills/pr/CREDITS.md).
